//! The apps a Device keeps Pairings with, as its settings screen shows them: read and forgotten
//! over USB only. Ported from the 1.x app's `lib/device/pairings.ts`.
//!
//! A forget that goes unanswered may or may not have reached the Device. The Companion sends it
//! again before the next list (`pairings.rs`), so the session reads the list once, quietly: when
//! that read works, the forget is settled either way and the error goes. Otherwise the error
//! stays, and forgetting again retries.
use super::{
    Carrier, DeviceError, DeviceId, DevicePairings, DevicePairingsError, DeviceSummary, LinkState,
    Operations,
};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::watch;

pub use super::settings_session::Load;

/// `true` for a Device whose Firmware keeps Pairings: one with a Wi-Fi station.
pub fn offers_pairings(device: &DeviceSummary) -> bool {
    device
        .capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.wifi == Some(true))
}

/// Pairings are read and changed over USB only.
pub fn over_usb(device: &DeviceSummary) -> bool {
    matches!(device.link, LinkState::Connected { .. }) && device.carrier != Some(Carrier::Wifi)
}

/// What is being forgotten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Forgetting {
    App(String),
    All,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PairingsView {
    pub load: Load,
    pub pairings: Option<DevicePairings>,
    pub forgetting: Option<Forgetting>,
    /// The Firmware answered that it keeps no Pairings.
    pub needs: bool,
    pub error: Option<String>,
}
impl PairingsView {
    /// The count line, as "2 of 4".
    pub fn count(&self) -> Option<String> {
        self.pairings
            .as_ref()
            .map(|p| format!("{} of {}", p.pairings.len(), p.max))
    }

    /// The status line under "Paired apps", if any.
    pub fn status(&self, name: &str, live: bool, usb: bool) -> Option<String> {
        if !live {
            return Some(format!("Plug in {name} to see its paired apps."));
        }
        if !usb {
            return Some(format!(
                "Plug in {name} with a USB cable to see its paired apps."
            ));
        }
        let pairings = self.pairings.as_ref()?;
        if pairings.pairings.is_empty() {
            Some(format!(
                "No apps are paired. Pair one over Wi-Fi to use {name} without the cable."
            ))
        } else if pairings.pairings.len() as u32 >= pairings.max {
            Some("Four apps are paired. Forget one to pair another.".into())
        } else {
            None
        }
    }

    /// Buttons may act: the list is read and nothing is being forgotten.
    pub fn editable(&self, usb: bool) -> bool {
        usb && self.pairings.is_some() && self.forgetting.is_none()
    }
}

fn unknown_outcome(error: &DevicePairingsError) -> bool {
    matches!(
        error,
        DevicePairingsError::Device(DeviceError::Timeout | DeviceError::UnknownOutcome { .. })
    )
}

struct Inner {
    operations: Operations,
    id: DeviceId,
    view: watch::Sender<PairingsView>,
    /// Bumped by each read and forget; an older answer doesn't replace a newer one.
    seq: Mutex<u64>,
    /// The Device was over USB when last observed.
    was_usb: Mutex<bool>,
}

/// One Device's Pairings for its settings screen. Clones share their state.
#[derive(Clone)]
pub struct PairingsSession {
    inner: Arc<Inner>,
}

impl PairingsSession {
    pub fn new(operations: Operations, id: DeviceId) -> Self {
        let usb = operations.devices.get(&id).is_some_and(|d| over_usb(&d));
        Self {
            inner: Arc::new(Inner {
                operations,
                id,
                view: watch::channel(PairingsView::default()).0,
                seq: Mutex::new(0),
                was_usb: Mutex::new(usb),
            }),
        }
    }
    pub fn view(&self) -> PairingsView {
        self.inner.view.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<PairingsView> {
        self.inner.view.subscribe()
    }
    fn set(&self, change: impl FnOnce(&mut PairingsView)) {
        self.inner.view.send_modify(change);
    }
    fn device(&self) -> Option<DeviceSummary> {
        self.inner.operations.devices.get(&self.inner.id)
    }
    fn called(&self) -> String {
        self.device()
            .and_then(|d| d.name)
            .unwrap_or_else(|| "the Device".into())
    }
    fn ready(&self) -> bool {
        self.device()
            .is_some_and(|d| over_usb(&d) && offers_pairings(&d))
    }
    fn next(&self) -> u64 {
        let mut seq = self.inner.seq.lock();
        *seq += 1;
        *seq
    }
    fn current(&self, mine: u64) -> bool {
        *self.inner.seq.lock() == mine
    }

    fn failed(&self, error: &DevicePairingsError) {
        let name = self.called();
        let text = match error {
            DevicePairingsError::Device(DeviceError::NeedsFirmware { .. }) => {
                self.set(|view| {
                    view.needs = true;
                    view.error = None;
                });
                return;
            }
            DevicePairingsError::Device(DeviceError::UsbOnly) => {
                format!("Plug in {name} with a USB cable to change its paired apps.")
            }
            DevicePairingsError::Storage => format!("{name} couldn't erase that. Try again."),
            error if unknown_outcome(error) => format!(
                "{name} didn't answer, so the app may still be paired. Plug {name} in again to finish."
            ),
            _ => format!("Couldn't reach {name}. Check that it's plugged in, then retry."),
        };
        self.set(|view| view.error = Some(text));
    }

    /// Reads the list. `report` false keeps a failure quiet, so the error already shown stays.
    async fn read(&self, report: bool) -> bool {
        let mine = self.next();
        match self.inner.operations.pairings_get(&self.inner.id).await {
            Ok(pairings) => {
                if self.current(mine) {
                    self.set(|view| {
                        view.pairings = Some(pairings);
                        view.load = Load::Ready;
                        view.needs = false;
                    });
                }
                true
            }
            Err(error) => {
                if !self.current(mine) {
                    return false;
                }
                self.set(|view| {
                    if view.load == Load::Loading {
                        view.load = Load::Failed;
                    }
                });
                if report {
                    self.failed(&error);
                }
                false
            }
        }
    }

    /// Reads the list when the Device is over USB and keeps Pairings.
    pub async fn load(&self) {
        if !self.ready() {
            return;
        }
        self.set(|view| {
            if view.load != Load::Ready {
                view.load = Load::Loading;
            }
        });
        if self.read(true).await {
            self.set(|view| view.error = None);
        }
    }

    async fn run(&self, forgetting: Forgetting) -> bool {
        if self.view().forgetting.is_some() || !self.ready() {
            return false;
        }
        let mine = self.next();
        self.set(|view| {
            view.forgetting = Some(forgetting.clone());
            view.error = None;
        });
        let operations = &self.inner.operations;
        let result = match &forgetting {
            Forgetting::App(app) => operations.pairings_forget(&self.inner.id, app).await,
            Forgetting::All => operations.pairings_forget_all(&self.inner.id).await,
        };
        self.set(|view| view.forgetting = None);
        match result {
            Ok(pairings) => {
                if self.current(mine) {
                    self.set(|view| {
                        view.pairings = Some(pairings);
                        view.load = Load::Ready;
                    });
                }
                true
            }
            Err(error) => {
                self.failed(&error);
                // The Companion sends an unanswered forget again before the next list, so one
                // read may settle it.
                if unknown_outcome(&error) && self.read(false).await {
                    self.set(|view| view.error = None);
                }
                false
            }
        }
    }

    /// Forgets one app by its id. Pressing it again after a failure retries.
    pub async fn forget(&self, app: &str) -> bool {
        self.run(Forgetting::App(app.into())).await
    }

    /// Forgets every app. The screen asks first.
    pub async fn forget_all(&self) -> bool {
        self.run(Forgetting::All).await
    }

    /// Follows the Device. `true` when it just came back over USB, so the list should be read.
    pub fn observe(&self, device: &DeviceSummary) -> bool {
        let usb = over_usb(device);
        let was = std::mem::replace(&mut *self.inner.was_usb.lock(), usb);
        !was && usb && offers_pairings(device)
    }
}
