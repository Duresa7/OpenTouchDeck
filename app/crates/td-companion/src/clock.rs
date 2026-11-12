use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// The Companion's clock. Tests move it forward.
#[derive(Clone, Default)]
pub(crate) struct Clock(Arc<AtomicU64>);
impl Clock {
    pub(crate) fn now(&self) -> Instant {
        Instant::now() + Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
    /// Wall-clock seconds since the Unix epoch, moved forward as far as tests moved [`Self::now`].
    /// For deadlines that must hold across sleep, such as Brandfetch's 30 days.
    pub(crate) fn unix_s(&self) -> u64 {
        crate::photos::clock::unix_s() + self.0.load(Ordering::SeqCst) / 1000
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}
