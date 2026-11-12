//! The apps a radio-build Device keeps Pairings with, read and forgotten over USB (net-v2's `net.list` and
//! `net.forget`).
//!
//! A forget whose answer never came may or may not have reached the Device. Until a later answer settles it, the app
//! stays marked as being removed: the next list over USB sends the forget again first, and a USB link that comes up
//! sends it too. Once the Device confirms the forget, or `net.list` no longer shows the app, the mark goes. When the
//! app was this Companion, its own key for that Device goes with it.
use super::{Carrier, DeviceError, DeviceId, DeviceManager, Inner, Reach, actor};
use crate::api::CommonError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use td_link::{
    Session,
    net::CompanionId,
    pairings::{self, Forget, NetPairings, PairingsError},
};
use ts_rs::TS;

impl Inner {
    fn marked(&self, device: &DeviceId) -> Vec<CompanionId> {
        self.wifi
            .forgetting
            .lock()
            .get(device)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }

    fn mark(&self, device: &DeviceId, ids: impl IntoIterator<Item = CompanionId>) {
        self.wifi
            .forgetting
            .lock()
            .entry(device.clone())
            .or_default()
            .extend(ids);
    }

    /// The Device no longer keeps `app`: the mark goes, and so does this Companion's key when `app` was this
    /// Companion.
    fn settled(self: &Arc<Self>, device: &DeviceId, app: &CompanionId) {
        // The Device has already erased the Pairing, so a failure here is this Companion's own. Keep the mark until
        // the local key is gone too, so the next list tries again; the snapshot shows it as still being removed.
        if *app == self.companion_id() && self.reg.lock().wifi.contains_key(device) {
            let manager = DeviceManager {
                inner: self.clone(),
            };
            if let Err(error) = manager.wifi_forget(device) {
                eprintln!("Companion: couldn't forget the Pairing with {device} here: {error}");
                return;
            }
        }
        let mut forgetting = self.wifi.forgetting.lock();
        if let Some(ids) = forgetting.get_mut(device) {
            ids.remove(app);
            if ids.is_empty() {
                forgetting.remove(device);
            }
        }
    }

    /// Whether this Companion's own Pairing with `device` is being removed.
    pub(super) fn removing_here(&self, device: &DeviceId) -> bool {
        let own = self.companion_id();
        self.wifi
            .forgetting
            .lock()
            .get(device)
            .is_some_and(|ids| ids.contains(&own))
    }

    /// Sends one forget and settles the mark only on success. Any failure keeps it for a retry.
    fn forget_marked(
        self: &Arc<Self>,
        device: &DeviceId,
        session: &Session,
        forget: Forget,
        apps: &[CompanionId],
    ) -> Result<(), PairingsError> {
        self.mark(device, apps.iter().copied());
        self.pairing_progress(device, PairingPhase::Removing, None, None);
        let result = pairings::net_forget(session, forget);
        if result.is_ok() {
            for app in apps {
                self.settled(device, app);
            }
        }
        result.map(|_| ())
    }

    /// Sends every forget still marked for `device` again. Stops at the first that goes unanswered.
    fn resend(self: &Arc<Self>, device: &DeviceId, session: &Session) -> Result<(), PairingsError> {
        for app in self.marked(device) {
            // Each marked app is attempted once per call; refusals keep the mark for the next call.
            if let Err(PairingsError::Link(error)) =
                self.forget_marked(device, session, Forget::One(app), &[app])
            {
                return Err(PairingsError::Link(error));
            }
        }
        Ok(())
    }

    /// Lists the Pairings after sending the marked forgets again, and drops the marks of apps the list no longer
    /// shows.
    fn list(
        self: &Arc<Self>,
        device: &DeviceId,
        session: &Session,
    ) -> Result<NetPairings, PairingsError> {
        self.resend(device, session)?;
        let listed = pairings::net_list(session)?;
        for app in self.marked(device) {
            if !listed.pairings.iter().any(|p| p.app_id == app) {
                self.settled(device, &app);
            }
        }
        Ok(listed)
    }

    /// Sends the marked forgets once a USB link to `device` is up, on its actor.
    pub(super) fn resend_on_link(self: &Arc<Self>, device: &DeviceId, handle: &actor::Handle) {
        if self.marked(device).is_empty() && !self.pairing_progress.borrow().contains_key(device) {
            return;
        }
        let inner = self.clone();
        let id = device.clone();
        let _ = handle.sender().send(Box::new(move |session: &Session| {
            inner.pairing_progress(&id, PairingPhase::Reconciling, None, None);
            let result = inner.list(&id, session);
            inner.pairing_finished(&id, &result);
            if let Err(error) = result {
                eprintln!("Companion: couldn't finish forgetting a Pairing on {id}: {error}");
            }
        }));
    }
}

impl DeviceManager {
    /// The id this Companion pairs with, which a Device lists as the app_id.
    pub fn app_id(&self) -> CompanionId {
        self.inner.companion_id()
    }

    /// Lists the apps `device` keeps Pairings with, over USB. Blocking: run it on the Device's actor.
    fn device_pairings(
        &self,
        device: &DeviceId,
        session: &Session,
    ) -> Result<NetPairings, PairingsError> {
        self.inner.list(device, session)
    }

    /// Forgets one app on `device`, or every one, over USB, and lists what is left. Blocking: run it on the Device's
    /// actor.
    fn device_forget(
        &self,
        device: &DeviceId,
        session: &Session,
        forget: Forget,
    ) -> Result<NetPairings, PairingsError> {
        let inner = &self.inner;
        inner.resend(device, session)?;
        match forget {
            Forget::One(app) => inner.forget_marked(device, session, forget, &[app])?,
            Forget::All => {
                // Every app the Device lists now is marked, so an unanswered request is sent again app by app: a
                // second `all` could remove an app paired in between.
                let apps: Vec<CompanionId> = pairings::net_list(session)?
                    .pairings
                    .into_iter()
                    .map(|p| p.app_id)
                    .collect();
                inner.forget_marked(device, session, forget, &apps)?;
            }
        }
        inner.list(device, session)
    }
}

/// One app the Device keeps a Pairing with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DevicePairing {
    /// The app's id: 32 lower-case hex characters.
    pub app: String,
    /// The name the app gave when it paired.
    pub name: String,
    /// The app is this Companion.
    pub this_app: bool,
}
/// The apps a Device keeps Pairings with, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DevicePairings {
    pub pairings: Vec<DevicePairing>,
    /// The most Pairings a Device keeps, 4.
    pub max: u32,
    /// Whether the Device's pairing window is open.
    pub pairing: bool,
    /// Milliseconds left in a lockout after wrong codes, 0 when none.
    pub locked_ms: u64,
}
#[derive(Debug, thiserror::Error)]
pub enum DevicePairingsError {
    #[error("The Device couldn't erase the Pairing from its storage. Try again.")]
    Storage,
    #[error(transparent)]
    Device(#[from] DeviceError),
}
impl From<CommonError> for DevicePairingsError {
    fn from(error: CommonError) -> Self {
        Self::Device(error.into())
    }
}
impl From<Reach> for DevicePairingsError {
    fn from(reach: Reach) -> Self {
        Self::Device(reach.into())
    }
}
impl From<PairingsError> for DevicePairingsError {
    fn from(error: PairingsError) -> Self {
        match error {
            PairingsError::Link(error) => DeviceError::from(error).into(),
            PairingsError::Refused { reason, error } => match reason.as_str() {
                "storage" => Self::Storage,
                "usb-only" => DeviceError::UsbOnly.into(),
                "args" => CommonError::invalid("path", error).into(),
                // A Device without `net.*` answers the unknown-message error, with no reason.
                "unsupported" | "" => DeviceError::NeedsFirmware { field: "pairings" }.into(),
                _ => DeviceError::Link {
                    reason: format!("the Device refused: {error} ({reason})"),
                }
                .into(),
            },
        }
    }
}

fn view(listed: NetPairings, own: &CompanionId) -> DevicePairings {
    DevicePairings {
        pairings: listed
            .pairings
            .into_iter()
            .map(|p| DevicePairing {
                app: pairings::app_id_hex(&p.app_id),
                name: p.app_name,
                this_app: p.app_id == *own,
            })
            .collect(),
        max: pairings::MAX_PAIRINGS,
        pairing: listed.pairing,
        locked_ms: listed.locked_ms,
    }
}

/// Progress is owned by the Companion, including retries after the requesting window closes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PairingPhase {
    #[default]
    Idle,
    Reading,
    Removing,
    Reconciling,
    Ready,
    Failed,
}

/// One view of the Device's list, pending removals and this Companion's local Pairing.
/// `stale` means the cached list must not be presented as a current Device read.
#[derive(Clone, Debug, Default)]
pub struct PairingSnapshot {
    pub phase: PairingPhase,
    pub listed: Option<DevicePairings>,
    pub removing: Vec<String>,
    pub local_pairing: Option<super::WifiPairing>,
    pub stale: bool,
    pub error: Option<String>,
}
impl Inner {
    fn pairing_progress(
        self: &Arc<Self>,
        device: &DeviceId,
        phase: PairingPhase,
        listed: Option<DevicePairings>,
        error: Option<String>,
    ) {
        let removing = self
            .marked(device)
            .iter()
            .map(pairings::app_id_hex)
            .collect();
        let local_pairing = DeviceManager {
            inner: self.clone(),
        }
        .wifi_pairings()
        .into_iter()
        .find(|p| &p.device == device);
        self.pairing_progress.send_modify(|all| {
            let state = all.entry(device.clone()).or_default();
            state.phase = phase;
            if let Some(listed) = listed {
                state.listed = Some(listed);
            }
            state.removing = removing;
            state.local_pairing = local_pairing;
            state.stale = phase != PairingPhase::Ready;
            state.error = error;
        });
    }
    fn pairing_finished(
        self: &Arc<Self>,
        device: &DeviceId,
        result: &Result<NetPairings, PairingsError>,
    ) {
        match result {
            Ok(listed) => self.pairing_progress(
                device,
                PairingPhase::Ready,
                Some(view(listed.clone(), &self.companion_id())),
                None,
            ),
            Err(error) => {
                self.pairing_progress(device, PairingPhase::Failed, None, Some(error.to_string()))
            }
        }
    }
}
impl super::Operations {
    /// Subscribe before reading `pairing_snapshot`. Refreshes on USB reconnect are automatic
    /// once this Device has been read, even if no removal remains pending.
    pub fn observe_pairings(
        &self,
    ) -> tokio::sync::watch::Receiver<std::collections::BTreeMap<DeviceId, PairingSnapshot>> {
        self.devices.inner.pairing_progress.subscribe()
    }
    pub fn pairing_snapshot(&self, id: &DeviceId) -> PairingSnapshot {
        self.devices
            .inner
            .pairing_progress
            .borrow()
            .get(id)
            .cloned()
            .unwrap_or_default()
    }
    pub async fn pairings_get(&self, id: &DeviceId) -> Result<DevicePairings, DevicePairingsError> {
        self.pairings_run(id, None).await
    }
    pub async fn pairings_forget(
        &self,
        id: &DeviceId,
        app: &str,
    ) -> Result<DevicePairings, DevicePairingsError> {
        let app = pairings::parse_app_id(app)
            .ok_or_else(|| CommonError::invalid("path", "app is 32 lower-case hex characters"))?;
        self.pairings_run(id, Some(Forget::One(app))).await
    }
    pub async fn pairings_forget_all(
        &self,
        id: &DeviceId,
    ) -> Result<DevicePairings, DevicePairingsError> {
        self.pairings_run(id, Some(Forget::All)).await
    }
    async fn pairings_run(
        &self,
        id: &DeviceId,
        forget: Option<Forget>,
    ) -> Result<DevicePairings, DevicePairingsError> {
        let summary = self.devices.get(id).ok_or(DeviceError::NotFound)?;
        if summary.capabilities.as_ref().and_then(|c| c.wifi) != Some(true) {
            return Err(DeviceError::NeedsFirmware { field: "pairings" }.into());
        }
        if summary.carrier == Some(Carrier::Wifi) {
            return Err(DeviceError::UsbOnly.into());
        }
        let inner = self.devices.inner.clone();
        let device = id.clone();
        let result = self
            .devices
            .call(id, move |session| {
                inner.pairing_progress(
                    &device,
                    if forget.is_some() {
                        PairingPhase::Removing
                    } else {
                        PairingPhase::Reading
                    },
                    None,
                    None,
                );
                let manager = DeviceManager {
                    inner: inner.clone(),
                };
                let result = match forget {
                    Some(forget) => manager.device_forget(&device, session, forget),
                    None => manager.device_pairings(&device, session),
                };
                inner.pairing_finished(&device, &result);
                result
            })
            .await;
        let listed = match result {
            Ok(listed) => listed?,
            Err(error) => {
                self.devices.inner.pairing_progress(
                    id,
                    PairingPhase::Failed,
                    None,
                    Some(format!("{error:?}")),
                );
                return Err(error.into());
            }
        };
        Ok(view(listed, &self.devices.app_id()))
    }
}
