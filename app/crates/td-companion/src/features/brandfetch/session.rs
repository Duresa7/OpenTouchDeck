//! Native Brandfetch settings. Construct without I/O, then `load` off the UI thread.
//! Each operation runs on a blocking worker and owns the shared gate through publication.
//! Cancelling its caller does not abandon persistence or leave a busy view behind.
use super::{ApiKey, BrandfetchError};
use crate::{api::CommonError, companion::Handle};
use std::sync::Arc;
use td_core::{app_dirs::Dirs, settings::AppSettings};
use tokio::sync::watch;

/// Safe, typed reasons for the page. No transport, keychain or server text reaches a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    NoKey,
    KeyRefused,
    Quota,
    Unavailable,
    NoKeychain,
    Storage,
    InvalidInput,
    Worker,
}
impl From<&BrandfetchError> for Problem {
    fn from(error: &BrandfetchError) -> Self {
        match error {
            BrandfetchError::NoKey => Self::NoKey,
            BrandfetchError::KeyRefused => Self::KeyRefused,
            BrandfetchError::Quota => Self::Quota,
            BrandfetchError::NoKeychain => Self::NoKeychain,
            BrandfetchError::Store(_) => Self::Storage,
            BrandfetchError::Common(CommonError::Internal) => Self::Worker,
            BrandfetchError::Common(_) => Self::InvalidInput,
            _ => Self::Unavailable,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyTestResult {
    #[default]
    NotTested,
    Testing,
    Passed,
    Failed(Problem),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Load,
    SaveClientId,
    SaveKey,
    ForgetClientId,
    ForgetKey,
    TestKey,
    SetEnabled,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrandfetchView {
    pub loaded: bool,
    pub enabled: bool,
    /// Public search client ID. Never the API key.
    pub client_id: Option<String>,
    pub key_set: bool,
    pub test_result: KeyTestResult,
    pub logos: u32,
    pub need_refresh: u32,
    /// Asset IDs whose cached originals have expired.
    pub expired: Vec<String>,
    pub busy: Option<Operation>,
    pub problem: Option<Problem>,
}
struct Inner {
    handle: Handle,
    dirs: Dirs,
    view: watch::Sender<BrandfetchView>,
}
/// One window's Brandfetch settings. Clones share the watch view. Call `load` when opening
/// the page or refreshing counts after Asset changes; it rereads saved state from other windows.
#[derive(Clone)]
pub struct BrandfetchSession {
    inner: Arc<Inner>,
}
impl BrandfetchSession {
    /// `dirs` is the same preference location used by the native host's Settings page.
    /// This constructor performs no disk, keychain or network I/O.
    pub fn new(handle: Handle, dirs: Dirs) -> Self {
        Self {
            inner: Arc::new(Inner {
                handle,
                dirs,
                view: watch::channel(BrandfetchView::default()).0,
            }),
        }
    }
    pub fn view(&self) -> BrandfetchView {
        self.inner.view.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<BrandfetchView> {
        self.inner.view.subscribe()
    }
    pub async fn load(&self) -> Result<(), BrandfetchError> {
        self.run(Operation::Load, |_| Ok(())).await
    }
    pub async fn save_client_id(&self, id: String) -> Result<(), BrandfetchError> {
        self.run(Operation::SaveClientId, move |inner| {
            inner.handle.brandfetch().save_client_id(id)
        })
        .await
    }
    pub async fn save_key(&self, key: ApiKey) -> Result<(), BrandfetchError> {
        self.run(Operation::SaveKey, move |inner| {
            inner.handle.brandfetch().save_key(key)
        })
        .await
    }
    pub async fn forget_client_id(&self) -> Result<(), BrandfetchError> {
        self.run(Operation::ForgetClientId, |inner| {
            inner.handle.brandfetch().forget_client_id()
        })
        .await
    }
    pub async fn forget_key(&self) -> Result<(), BrandfetchError> {
        self.run(Operation::ForgetKey, |inner| {
            inner.handle.brandfetch().forget_key()
        })
        .await
    }
    /// One Brand API request with a 20-second HTTP timeout, on a blocking worker.
    /// Saving never tests the key. Cancellation lets this bounded request finish and publish.
    pub async fn test_key(&self) -> Result<(), BrandfetchError> {
        self.run(Operation::TestKey, |inner| {
            inner.handle.brandfetch().test_key()
        })
        .await
    }
    /// The switch never deletes credentials or changes the existing cached-logo expiry policy.
    pub async fn set_enabled(&self, on: bool) -> Result<(), BrandfetchError> {
        self.run(Operation::SetEnabled, move |inner| {
            AppSettings::set_brandfetch_enabled(&inner.dirs, on)
                .map(|_| ())
                .map_err(|_| BrandfetchError::Store("Couldn't write app preferences.".into()))
        })
        .await
    }
    async fn run(
        &self,
        operation: Operation,
        work: impl FnOnce(&Inner) -> Result<(), BrandfetchError> + Send + 'static,
    ) -> Result<(), BrandfetchError> {
        let gate = self
            .inner
            .handle
            .brandfetch()
            .0
            .operations
            .clone()
            .lock_owned()
            .await;
        let inner = self.inner.clone();
        // The guard moves into the worker. Even when this future is dropped, completion and
        // publication stay serialized with the next operation from any window.
        tokio::task::spawn_blocking(move || {
            let _gate = gate;
            inner.view.send_modify(|view| {
                view.busy = Some(operation);
                view.problem = None;
                if operation == Operation::TestKey {
                    view.test_result = KeyTestResult::Testing;
                }
            });
            let result = work(&inner);
            let snapshot = inner
                .handle
                .brandfetch()
                .status(inner.handle.clock.unix_s())
                .and_then(|status| {
                    AppSettings::load(&inner.dirs)
                        .map(|settings| (status, settings))
                        .map_err(|_| {
                            BrandfetchError::Store("Couldn't read app preferences.".into())
                        })
                });
            let result =
                result.and(snapshot.as_ref().map(|_| ()).map_err(|_| {
                    BrandfetchError::Store("Couldn't read Brandfetch settings.".into())
                }));
            inner.view.send_modify(|view| {
                if let Ok((status, settings)) = snapshot {
                    view.loaded = true;
                    view.enabled = settings.brandfetch_enabled;
                    view.client_id = status.client_id;
                    view.key_set = status.key_set;
                    view.logos = status.logos;
                    view.need_refresh = status.need_refresh;
                    view.expired = status.expired;
                }
                view.busy = None;
                view.problem = result.as_ref().err().map(Problem::from);
                if operation == Operation::TestKey {
                    view.test_result = match &result {
                        Ok(()) => KeyTestResult::Passed,
                        Err(error) => KeyTestResult::Failed(Problem::from(error)),
                    };
                } else if result.is_ok()
                    && matches!(operation, Operation::SaveKey | Operation::ForgetKey)
                {
                    view.test_result = KeyTestResult::NotTested;
                }
            });
            result
        })
        .await
        .map_err(|_| BrandfetchError::Common(CommonError::Internal))?
    }
}
