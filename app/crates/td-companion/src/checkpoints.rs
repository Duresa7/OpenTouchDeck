//! Points in Device writes where a test stops the work, acts, then lets it go on or fail there.

use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Checkpoint {
    /// A Layout push wrote the Device and has not yet written the pushed copy.
    LayoutPut,
    /// An orientation change is about to send `SettingsPut`.
    BeforeOrientationPut,
    /// An orientation change sent `SettingsPut` and has more steps to run.
    AfterOrientationPut,
    /// The Integrations session rendered a view and has not yet sent it to the window.
    IntegrationsPublish,
}

#[derive(Default)]
struct Point {
    held: bool,
    waiting: bool,
    fail: bool,
    /// Only the first arrival waits; later ones pass while it does.
    once: bool,
}

#[derive(Clone, Default)]
pub struct Checkpoints(Arc<(Mutex<HashMap<Checkpoint, Point>>, Condvar)>);

impl Checkpoints {
    /// Work that reaches `at` from now on waits there until `release`.
    pub fn hold(&self, at: Checkpoint) {
        self.0.0.lock().entry(at).or_default().held = true;
    }

    /// Like `hold`, but only the next arrival waits; later work passes while it does.
    pub fn hold_once(&self, at: Checkpoint) {
        let mut points = self.0.0.lock();
        let point = points.entry(at).or_default();
        point.held = true;
        point.once = true;
    }

    /// Lets the work at `at` go on. With `fail`, its next step fails as a Device request that timed out would.
    pub fn release(&self, at: Checkpoint, fail: bool) {
        let mut points = self.0.0.lock();
        let point = points.entry(at).or_default();
        point.held = false;
        point.once = false;
        point.fail = fail;
        self.0.1.notify_all();
    }

    /// Whether work waits at `at` now.
    pub fn waiting(&self, at: Checkpoint) -> bool {
        self.0.0.lock().get(&at).is_some_and(|point| point.waiting)
    }

    pub(crate) fn pass(&self, at: Checkpoint) -> Result<(), ()> {
        let mut points = self.0.0.lock();
        let point = points.entry(at).or_default();
        if point.held && point.once && point.waiting {
            return Ok(());
        }
        loop {
            let point = points.entry(at).or_default();
            if !point.held {
                point.waiting = false;
                return if std::mem::take(&mut point.fail) {
                    Err(())
                } else {
                    Ok(())
                };
            }
            point.waiting = true;
            self.0.1.wait(&mut points);
        }
    }
}
