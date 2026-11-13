//! Coordinated Device operations used by native screens, tests and the local web adapter.
//! Clone this interface from `Handle::device_operations`. Blocking link work runs on actors.
//! Settings operations on a Device are serialized through PC persistence as well as link writes.
use super::{DeviceId, DeviceManager};
use std::{collections::BTreeMap, sync::Arc};

type SettingsGates = BTreeMap<DeviceId, Arc<tokio::sync::Mutex<()>>>;
type Sessions = BTreeMap<DeviceId, (super::SettingsSession, super::PairingsSession)>;

#[derive(Clone)]
pub struct Operations {
    pub(super) devices: DeviceManager,
    pub(super) authority: crate::layouts::LayoutAuthority,
    pub(super) photos: crate::photos::PhotoFrames,
    gates: Arc<parking_lot::Mutex<SettingsGates>>,
    sessions: Arc<parking_lot::Mutex<Sessions>>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) checkpoints: crate::checkpoints::Checkpoints,
}
impl Operations {
    pub(crate) fn new(
        devices: DeviceManager,
        authority: crate::layouts::LayoutAuthority,
        photos: crate::photos::PhotoFrames,
        #[cfg(any(test, feature = "test-hooks"))] checkpoints: crate::checkpoints::Checkpoints,
    ) -> Self {
        Self {
            devices,
            authority,
            photos,
            gates: Default::default(),
            sessions: Default::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            checkpoints,
        }
    }
    pub(super) async fn settings_guard(&self, id: &DeviceId) -> tokio::sync::OwnedMutexGuard<()> {
        let gate = self.gates.lock().entry(id.clone()).or_default().clone();
        gate.lock_owned().await
    }
    /// The settings every screen of this Device shares, so setup and Settings show one state.
    pub fn settings_session(&self, id: &DeviceId) -> super::SettingsSession {
        self.sessions(id).0
    }
    /// The Pairings every screen of this Device shares.
    pub fn pairings_session(&self, id: &DeviceId) -> super::PairingsSession {
        self.sessions(id).1
    }
    fn sessions(&self, id: &DeviceId) -> (super::SettingsSession, super::PairingsSession) {
        self.sessions
            .lock()
            .entry(id.clone())
            .or_insert_with(|| {
                // Sessions get a copy without this cache, so nothing keeps itself alive.
                let operations = Self {
                    sessions: Default::default(),
                    ..self.clone()
                };
                (
                    super::SettingsSession::new(operations.clone(), id.clone()),
                    super::PairingsSession::new(operations, id.clone()),
                )
            })
            .clone()
    }
}
