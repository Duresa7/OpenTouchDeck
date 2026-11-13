use crate::{
    companion::{Companion, Config, Handle},
    devices::{BoardKind, DeviceId, DeviceSummary, LinkState, MockPorts, NewBoard},
};
use std::time::{Duration, Instant};

/// The production in-process seam with mock ports and temporary storage. No web adapter,
/// listener or session is created. `ports` and `deck` change the world around the Companion;
/// behaviour is driven and observed through `handle`.
pub struct InProcessCompanion {
    pub handle: Handle,
    pub ports: MockPorts,
    input: td_host::FakeSink,
    companion: Option<Companion>,
    _data: tempfile::TempDir,
}
impl InProcessCompanion {
    pub async fn start() -> Self {
        let data = tempfile::tempdir().unwrap();
        let config = Config::mock(data.path().to_owned());
        let (ports, input) = config.devices.mock.clone().unwrap();
        let companion = Companion::start(config).await.unwrap();
        Self {
            handle: companion.handle(),
            ports,
            input,
            companion: Some(companion),
            _data: data,
        }
    }
    pub async fn shutdown(&mut self) {
        if let Some(companion) = self.companion.take() {
            companion.shutdown().await;
        }
    }
    pub fn drop_owner(&mut self) {
        self.companion.take();
    }
    pub fn device(&self, id: &DeviceId) -> Option<DeviceSummary> {
        self.handle.devices().get(id)
    }
    /// Points where a test holds the Companion's work.
    pub fn checkpoints(&self) -> &crate::checkpoints::Checkpoints {
        &self.handle.checkpoints
    }
    pub fn deck(&self, id: &str) -> td_link::MockDeck {
        self.ports.deck(id).unwrap()
    }
    /// Moves the same monotonic clock used by selections, consent and Jobs.
    pub fn advance_clock(&self, by: Duration) {
        self.handle.clock.advance(by);
    }
    pub fn input_log(&self) -> Vec<String> {
        crate::features::test_hooks::input_log(&self.input)
    }
    pub async fn until(&self, mut done: impl FnMut(&Self) -> bool) {
        let started = Instant::now();
        while !done(self) {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the Companion didn't settle in time"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    pub async fn plug_device(&self, id: &str) -> DeviceId {
        self.ports.plug(id, BoardKind::Device);
        let id = DeviceId::parse(id).unwrap();
        self.until(|td| {
            td.device(&id)
                .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
        })
        .await;
        id
    }
    pub async fn plug_board(&self, id: &str, kind: BoardKind) -> NewBoard {
        self.ports.plug(id, kind);
        self.until(|td| td.handle.snapshot().new_boards.iter().any(|b| b.id == id))
            .await;
        self.handle
            .snapshot()
            .new_boards
            .into_iter()
            .find(|b| b.id == id)
            .unwrap()
    }
    pub async fn unplug(&self, id: &str) {
        self.ports.unplug(id);
        self.until(|td| {
            !td.handle
                .devices()
                .ports()
                .iter()
                .any(|p| p.usb_serial.as_deref() == Some(id))
        })
        .await;
    }
}
