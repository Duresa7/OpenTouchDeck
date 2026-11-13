use super::*;
use std::time::Duration;
use td_host::{InputSink, KeyEvent, keymap::Code};

struct BlockingSink {
    events: td_host::FakeSink,
    entered: mpsc::Sender<()>,
    resume: mpsc::Receiver<()>,
    first: bool,
}
impl InputSink for BlockingSink {
    fn send(&mut self, event: KeyEvent) -> Result<(), String> {
        self.events.send(event)?;
        if self.first {
            self.first = false;
            self.entered.send(()).unwrap();
            self.resume.recv().unwrap();
        }
        Ok(())
    }
}

#[test]
fn resumed_report_only_releases_after_shutdown() {
    let devices = super::super::DeviceManager::new(crate::api::Events::default());
    let events = td_host::FakeSink::default();
    let (entered, waiting) = mpsc::channel();
    let (resume, blocked) = mpsc::channel();
    let sink = std::sync::Mutex::new(Some(BlockingSink {
        events: events.clone(),
        entered,
        resume: blocked,
        first: true,
    }));
    let input: InputFactory = Arc::new(move || Ok(Box::new(sink.lock().unwrap().take().unwrap())));
    let hello: td_core::Hello = serde_json::from_value(serde_json::json!({"product":"touchdeck","proto":1,"elf_sha256":"","version":"","running":"","sinks":["link"],"recovery":false,"layout":""})).unwrap();
    let (mirror, _) = mirror(&devices.inner, &input, &hello);
    let inner = devices.inner.clone();
    let (done, finished) = mpsc::channel();
    let worker = thread::spawn(move || {
        forward(
            &inner,
            &DeviceId::parse("mock0001").unwrap(),
            0,
            0,
            [Incoming::Event(
                serde_json::json!({"m":"key.report","seq":1,"type":"kbd","mods":1,"keys":[4,5]}),
            )],
            mirror,
            Touches {
                presses: mpsc::sync_channel(16).0,
            },
        );
        done.send(()).unwrap();
    });
    devices.inner.track(worker);
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    devices.stop();
    assert!(!devices.wait_stopped(Duration::from_millis(20)));
    resume.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(2)).unwrap();
    let typed = events.take();
    assert_eq!(
        typed
            .iter()
            .filter(|e| matches!(e, KeyEvent::Down(_)))
            .count(),
        1
    );
    assert!(typed.contains(&KeyEvent::Up(Code::Keyboard(0xe0))));
}
