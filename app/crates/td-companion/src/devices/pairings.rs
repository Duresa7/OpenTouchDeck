//! The apps a radio-build Device keeps Pairings with, read and forgotten over USB (net-v2's `net.list` and
//! `net.forget`).
//!
//! A forget whose answer never came may or may not have reached the Device. Until a later answer settles it, the app
//! stays marked as being removed: the next list over USB sends the forget again first, and a USB link that comes up
//! sends it too. Once the Device confirms the forget, or `net.list` no longer shows the app, the mark goes. When the
//! app was this Companion, its own key for that Device goes with it.
use super::{DeviceId, DeviceManager, Inner, actor};
use std::sync::Arc;
use td_link::{
    Session,
    net::CompanionId,
    pairings::{self, Forget, NetPairings, PairingsError},
};

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
        {
            let mut forgetting = self.wifi.forgetting.lock();
            if let Some(ids) = forgetting.get_mut(device) {
                ids.remove(app);
                if ids.is_empty() {
                    forgetting.remove(device);
                }
            }
        }
        if *app == self.companion_id() && self.reg.lock().wifi.contains_key(device) {
            let manager = DeviceManager {
                inner: self.clone(),
            };
            if let Err(error) = manager.wifi_forget(device) {
                eprintln!("Companion: couldn't forget the Pairing with {device} here: {error}");
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
        if self.marked(device).is_empty() {
            return;
        }
        let inner = self.clone();
        let id = device.clone();
        let _ = handle.sender().send(Box::new(move |session: &Session| {
            if let Err(error) = inner.list(&id, session) {
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
    pub fn device_pairings(
        &self,
        device: &DeviceId,
        session: &Session,
    ) -> Result<NetPairings, PairingsError> {
        self.inner.list(device, session)
    }

    /// Forgets one app on `device`, or every one, over USB, and lists what is left. Blocking: run it on the Device's
    /// actor.
    pub fn device_forget(
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
