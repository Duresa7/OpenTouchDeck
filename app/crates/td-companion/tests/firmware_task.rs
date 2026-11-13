//! A screen's Firmware task through the in-process Companion: the bundled Firmware, a picked file,
//! the native consent (scripted here), a decline, a cancel while it waits and the build switch.
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use td_companion::{
    devices::{BoardKind, FirmwareChannel, LinkState},
    features::firmware::{
        Confirmation, ConsentAnswer, FirmwareTask, JobView, Prompter, Source, Target, TaskView,
    },
    testing::InProcessCompanion,
};

const NORMAL: &[u8] = include_bytes!("../../td-link/tests/fixtures/touchdeck-test.tdfw");
const RADIO: &[u8] = include_bytes!("../../td-link/tests/fixtures/touchdeck-test-radio.tdfw");

async fn until(task: &FirmwareTask, done: impl Fn(&TaskView) -> bool) -> TaskView {
    let mut views = task.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            {
                let view = views.borrow_and_update().clone();
                if done(&view) {
                    return view;
                }
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the task settled in time: {:?}", task.view()))
}

fn updates(td: &InProcessCompanion, serial: &str) -> usize {
    td.deck(serial)
        .received()
        .iter()
        .filter(|v| v["m"] == "update.begin")
        .count()
}

/// Hands out a file only when the test says so, and remembers each consent question.
#[derive(Default)]
struct Picker {
    file: Mutex<Option<mpsc::Receiver<Option<&'static [u8]>>>>,
    asked: Mutex<Vec<Confirmation>>,
}
impl Picker {
    fn hold(&self) -> mpsc::Sender<Option<&'static [u8]>> {
        let (send, receive) = mpsc::channel();
        *self.file.lock().unwrap() = Some(receive);
        send
    }
}
impl Prompter for Picker {
    fn pick(&self) -> Result<Option<Vec<u8>>, String> {
        let file = self.file.lock().unwrap().take().expect("a file was held");
        Ok(file.recv().unwrap().map(<[u8]>::to_vec))
    }
    fn confirm(&self, confirmation: &Confirmation, closed: &(dyn Fn() -> bool + Sync)) -> bool {
        self.asked.lock().unwrap().push(confirmation.clone());
        while !closed() {
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }
}

#[tokio::test]
async fn an_update_from_a_file_waits_for_consent_on_this_pc_then_installs_it() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let task = FirmwareTask::new(td.handle.clone(), Target::Device(id.clone()));
    // No answer yet: the dialog stays open.
    td.handle.firmware().script(vec![]);

    let started = tokio::spawn({
        let task = task.clone();
        async move { task.start_from(Source::File, None).await }
    });
    let view = until(&task, |v| matches!(v.view, JobView::Waiting { .. })).await;
    assert_eq!(view.view.text(), "Waiting for confirmation on this PC.");
    assert!(view.view.in_progress());
    assert_eq!(
        updates(&td, "mock0001"),
        0,
        "nothing is written before consent"
    );

    td.handle.firmware().script(vec![ConsentAnswer::Accept]);
    started.await.unwrap();
    let view = task.view();
    assert_eq!(view.view.text(), "Updated to test-only", "{view:?}");
    assert!(view.log.iter().any(|l| l == "Waiting for confirmation"));
    assert_eq!(updates(&td, "mock0001"), 1);
    td.shutdown().await;
}

#[tokio::test]
async fn a_declined_file_changes_nothing_and_says_so() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let task = FirmwareTask::new(td.handle.clone(), Target::Device(id));
    td.handle.firmware().script(vec![ConsentAnswer::Decline]);

    task.start_from(Source::File, None).await;
    let view = task.view();
    assert!(
        matches!(&view.view, JobView::Failed { code, .. } if code == "firmware.declined"),
        "{view:?}"
    );
    assert_eq!(
        view.view.text(),
        "Declined on this PC. Mock Device mock0001 still runs mock."
    );
    assert_eq!(updates(&td, "mock0001"), 0);
    td.shutdown().await;
}

#[tokio::test]
async fn the_file_dialog_holds_the_task_and_closing_it_changes_nothing() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let task = FirmwareTask::new(td.handle.clone(), Target::Device(id));
    let picker = Arc::new(Picker::default());
    td.handle.firmware().set_prompter(picker.clone());

    let file = picker.hold();
    let started = tokio::spawn({
        let task = task.clone();
        async move { task.start_from(Source::File, None).await }
    });
    let view = until(&task, |v| matches!(v.view, JobView::Choosing { .. })).await;
    assert_eq!(view.view.text(), "Choose a Firmware file on this PC…");
    assert!(view.view.in_progress());
    // A second start while the dialog is open does nothing.
    task.start().await;
    assert!(matches!(task.view().view, JobView::Choosing { .. }));

    // Closed without a file: back to where it was.
    file.send(None).unwrap();
    started.await.unwrap();
    assert_eq!(task.view(), TaskView::default());
    assert_eq!(updates(&td, "mock0001"), 0);
    td.shutdown().await;
}

#[tokio::test]
async fn a_file_that_is_not_firmware_is_refused_in_words() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let task = FirmwareTask::new(td.handle.clone(), Target::Device(id));
    let picker = Arc::new(Picker::default());
    td.handle.firmware().set_prompter(picker.clone());

    picker.hold().send(Some(b"not a bundle")).unwrap();
    task.start_from(Source::File, None).await;
    assert_eq!(
        task.view().view.text(),
        "That file isn't a Firmware file for this Device."
    );

    // A file of the other build is refused before any dialog asks.
    picker.hold().send(Some(RADIO)).unwrap();
    task.start_from(Source::File, None).await;
    assert_eq!(
        task.view().view.text(),
        "That file is the radio build, and Mock Device mock0001 runs the normal build. Nothing changed."
    );
    assert!(picker.asked.lock().unwrap().is_empty());
    assert_eq!(updates(&td, "mock0001"), 0);
    td.shutdown().await;
}

#[tokio::test]
async fn cancel_while_waiting_closes_the_dialog_and_changes_nothing() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let task = FirmwareTask::new(td.handle.clone(), Target::Device(id));
    let picker = Arc::new(Picker::default());
    td.handle.firmware().set_prompter(picker.clone());

    picker.hold().send(Some(NORMAL)).unwrap();
    let started = tokio::spawn({
        let task = task.clone();
        async move { task.start_from(Source::File, None).await }
    });
    until(&task, |v| matches!(v.view, JobView::Waiting { .. })).await;
    task.cancel();
    started.await.unwrap();
    let view = task.view();
    assert!(matches!(view.view, JobView::Cancelled { .. }), "{view:?}");
    assert_eq!(
        view.view.text(),
        "Update stopped. Mock Device mock0001 still runs mock."
    );
    assert_eq!(picker.asked.lock().unwrap().len(), 1);
    assert_eq!(updates(&td, "mock0001"), 0);
    td.shutdown().await;
}

#[tokio::test]
async fn the_build_switch_asks_on_this_pc_then_runs_the_other_build() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let task = FirmwareTask::new(td.handle.clone(), Target::Device(id.clone()));
    td.handle.firmware().script(vec![ConsentAnswer::Accept]);

    task.start_from(Source::Bundled, Some(FirmwareChannel::Radio))
        .await;
    let view = task.view();
    assert_eq!(
        view.view.text(),
        "Now on the radio build, test-only-radio",
        "{view:?}"
    );
    assert!(view.log.iter().any(|l| l == "Waiting for confirmation"));
    td.until(|td| {
        td.device(&id).is_some_and(|d| {
            matches!(d.link, LinkState::Connected { .. })
                && d.firmware
                    .is_some_and(|f| f.channel == FirmwareChannel::Radio)
        })
    })
    .await;
    td.shutdown().await;
}

#[tokio::test]
async fn without_bundled_firmware_only_a_file_can_update() {
    let mut td = InProcessCompanion::start().await;
    // A build made without the bundle variable, like 2.0.0-preview.1.
    td.handle.firmware().set_bundles(None, None);
    assert!(td.handle.firmware().builds().is_err());
    let id = td.plug_device("mock0001").await;
    let task = FirmwareTask::new(td.handle.clone(), Target::Device(id.clone()));
    td.handle.firmware().script(vec![ConsentAnswer::Accept]);

    task.start().await;
    let view = task.view();
    assert!(
        matches!(&view.view, JobView::Failed { code, .. } if code == "firmware.not_bundled"),
        "{view:?}"
    );
    assert_eq!(
        view.view.text(),
        "This copy of Open TouchDeck doesn't include Firmware. Update from a file instead."
    );
    task.start_from(Source::Bundled, Some(FirmwareChannel::Radio))
        .await;
    assert_eq!(
        task.view().view.text(),
        "This copy of Open TouchDeck doesn't include the radio build."
    );
    assert_eq!(updates(&td, "mock0001"), 0);

    // A New board can't be installed either, and says why.
    let board = td.plug_board("mock0009", BoardKind::Silent).await;
    let install = FirmwareTask::new(td.handle.clone(), Target::NewBoard(board.id));
    install.start().await;
    assert_eq!(
        install.view().view.text(),
        "This copy of Open TouchDeck doesn't include Firmware to install."
    );

    task.start_from(Source::File, None).await;
    assert_eq!(task.view().view.text(), "Updated to test-only");
    td.shutdown().await;
}

#[tokio::test]
async fn the_build_row_shows_only_when_this_companion_carries_the_other_build() {
    use td_companion::features::firmware::{FirmwareBuild, RadioBuild, build_choice};
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let mut device = td.device(&id).unwrap();
    let both = FirmwareBuild {
        version: "1.3.0".into(),
        build: String::new(),
        radio: Some(RadioBuild {
            version: "1.3.0-radio".into(),
            build: String::new(),
        }),
    };
    let normal_only = FirmwareBuild {
        radio: None,
        ..both.clone()
    };

    let choice = build_choice(&device, Some(&both), "Desk").unwrap();
    assert_eq!(choice.current, FirmwareChannel::Normal);
    assert_eq!(choice.target, FirmwareChannel::Radio);
    assert_eq!(choice.title, "Normal build");
    assert_eq!(choice.action, "Install the radio build");
    assert_eq!(choice.confirm_title, "Install the radio build on Desk?");
    assert_eq!(choice.confirm_action, "Install");
    assert!(!choice.warn);
    assert_eq!(build_choice(&device, Some(&normal_only), "Desk"), None);
    assert_eq!(
        build_choice(&device, None, "Desk"),
        None,
        "no bundle at all"
    );

    device.firmware.as_mut().unwrap().channel = FirmwareChannel::Radio;
    let choice = build_choice(&device, Some(&normal_only), "Desk").unwrap();
    assert_eq!(choice.target, FirmwareChannel::Normal);
    assert_eq!(choice.title, "Radio build");
    assert_eq!(choice.action, "Switch to the normal build");
    assert_eq!(
        choice.confirm_note,
        "The Wi-Fi and Bluetooth setup stays stored on Desk but unused. Desk restarts once."
    );
    assert!(choice.warn);
    assert_eq!(build_choice(&device, None, "Desk"), None);

    device.firmware = None;
    assert_eq!(build_choice(&device, Some(&both), "Desk"), None);
    td.shutdown().await;
}
