//! Coordinated Device operations used by native screens, tests and the local web adapter.
//! Clone this interface from `Handle::device_operations`. Blocking link work runs on actors.
//! Settings operations on a Device are serialized through PC persistence as well as link writes.
use super::{DeviceId, DeviceManager};
use std::{collections::BTreeMap, sync::Arc};

type SettingsGates = BTreeMap<DeviceId, Arc<tokio::sync::Mutex<()>>>;

#[derive(Clone)]
pub struct Operations {
    pub(super) devices: DeviceManager,
    pub(super) authority: crate::layouts::LayoutAuthority,
    pub(super) photos: crate::photos::PhotoFrames,
    gates: Arc<parking_lot::Mutex<SettingsGates>>,
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
            #[cfg(any(test, feature = "test-hooks"))]
            checkpoints,
        }
    }
    pub(super) async fn settings_guard(&self, id: &DeviceId) -> tokio::sync::OwnedMutexGuard<()> {
        let gate = self.gates.lock().entry(id.clone()).or_default().clone();
        gate.lock_owned().await
    }
}
