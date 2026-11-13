//! The editor Worker as a window drives it: commands in, one `EditorView` out on a `watch`.
use std::time::Duration;
use td_companion::{checkpoints::Checkpoint, devices::DeviceId};
use td_companion::{
    editor::{
        AddKind, AddTarget, Command, CommandKind, ConflictView, Decision, Done, EditorError,
        EditorView, NewLayout, SaveState, SessionError, Worker,
    },
    testing::TestCompanion,
};
use td_core::{Layout, Orientation, editing::Edit, layout::Page};
use tokio::sync::watch;

async fn worker() -> (TestCompanion, Worker) {
    let td = TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    let worker = td.handle().open_editor_worker(&device).unwrap();
    (td, worker)
}
/// The first published view that satisfies `done`.
async fn until(
    view: &mut watch::Receiver<EditorView>,
    done: impl FnMut(&EditorView) -> bool,
) -> EditorView {
    tokio::time::timeout(Duration::from_secs(10), view.wait_for(done))
        .await
        .expect("the view never got there")
        .expect("the worker stopped")
        .clone()
}
/// The view after the outcome numbered `number`.
async fn after(view: &mut watch::Receiver<EditorView>, number: u64) -> EditorView {
    until(view, |v| {
        v.outcome.as_ref().is_some_and(|o| o.number >= number)
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn two_quick_edits_apply_in_order() {
    let (_td, worker) = worker().await;
    let mut view = worker.subscribe();
    worker.dispatch(Command::NewLayout(NewLayout::Blank));
    worker.dispatch(Command::Edit(Edit::Name {
        name: "First".into(),
    }));
    worker.dispatch(Command::Edit(Edit::Name {
        name: "Second".into(),
    }));
    let shown = after(&mut view, 3).await;
    assert_eq!(shown.outcome.unwrap().number, 3);
    assert_eq!(shown.layout.unwrap().name, "Second");
}

/// The Tile id an Add Tile outcome created.
fn created(view: &EditorView) -> String {
    match &view.outcome.as_ref().unwrap().result {
        Ok(Done::Tile(id)) => id.clone(),
        other => panic!("no Tile was created: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_label_shows_and_blocks_a_send_until_discarded() {
    let (_td, worker) = worker().await;
    let mut view = worker.subscribe();
    worker.dispatch(Command::NewLayout(NewLayout::Blank));
    worker.dispatch(Command::Add {
        kind: AddKind::Key,
        target: AddTarget::Cell { row: 0, col: 1 },
    });
    let id = created(&after(&mut view, 2).await);
    worker.dispatch(Command::Edit(Edit::Label {
        tile: id.clone(),
        label: "Seventeen letters".into(),
    }));
    worker.dispatch(Command::Select {
        page: "page1".into(),
        tile: None,
    });
    worker.dispatch(Command::Send);
    let shown = after(&mut view, 5).await;
    let tile = &shown.layout.as_ref().unwrap().pages[0].tiles[0];
    assert_eq!(tile.tile.label, "Seventeen letters");
    assert_eq!(tile.problems.len(), 1);
    let outcome = shown.outcome.unwrap();
    assert_eq!(outcome.command, CommandKind::Send);
    let failure = outcome.result.unwrap_err();
    assert!(matches!(
        *failure.error,
        SessionError::Editor(EditorError::Rejected)
    ));
    // Nothing reached the Device, so the copy doesn't blame the connection.
    assert_eq!(failure.notice.text, "unresolved rejected fields");
    // Send to Device selects the first problem Tile.
    assert_eq!(shown.tile.as_deref(), Some(id.as_str()));
    assert_eq!(shown.save_state, SaveState::Unsent);

    worker.dispatch(Command::DiscardRejected);
    worker.dispatch(Command::Send);
    let shown = after(&mut view, 7).await;
    assert_eq!(shown.outcome.unwrap().result, Ok(Done::Changed));
    let tile = &shown.layout.unwrap().pages[0].tiles[0];
    assert_eq!(tile.tile.label, "Tile");
    assert!(tile.problems.is_empty());
    assert_eq!(shown.save_state, SaveState::Saved);
}

fn named(name: &str) -> Layout {
    Layout {
        schema: 1,
        name: name.into(),
        orientation: Orientation::Landscape,
        full_screen: false,
        pages: vec![Page::Grid {
            id: "page1".into(),
            title: "Page 1".into(),
            rows: 3,
            cols: 4,
            tiles: vec![],
        }],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conflict_publishes_and_resolves() {
    let (td, worker) = worker().await;
    let device = DeviceId::parse("mock0001").unwrap();
    let mut view = worker.subscribe();
    worker.dispatch(Command::NewLayout(NewLayout::Blank));
    worker.dispatch(Command::Send);
    assert_eq!(after(&mut view, 2).await.save_state, SaveState::Saved);

    // A Load that finds another Layout asks which one stays; Keep mine keeps the draft.
    td.write_layout_unseen(&device, &named("Elsewhere")).await;
    worker.dispatch(Command::Edit(Edit::Name {
        name: "Mine".into(),
    }));
    worker.dispatch(Command::Load);
    let shown = after(&mut view, 4).await;
    assert_eq!(
        shown.conflict,
        Some(ConflictView {
            name: "Elsewhere".into()
        })
    );
    assert_eq!(shown.layout.unwrap().name, "Mine");
    worker.dispatch(Command::Resolve(Decision::KeepMine));
    worker.dispatch(Command::Send);
    let shown = after(&mut view, 6).await;
    assert_eq!(shown.conflict, None);
    assert_eq!(shown.layout.unwrap().name, "Mine");
    assert_eq!(shown.save_state, SaveState::Saved);

    // A send the Device refuses because its Layout changed publishes the conflict too.
    td.write_layout_unseen(&device, &named("Other")).await;
    worker.dispatch(Command::Edit(Edit::Name {
        name: "Local".into(),
    }));
    worker.dispatch(Command::Send);
    let shown = after(&mut view, 8).await;
    let failure = shown.outcome.unwrap().result.unwrap_err();
    assert_eq!(failure.refusal().unwrap().code, "layout.changed");
    assert_eq!(
        shown.conflict,
        Some(ConflictView {
            name: "Other".into()
        })
    );
    worker.dispatch(Command::Resolve(Decision::UseDevice));
    let shown = after(&mut view, 9).await;
    assert_eq!(shown.conflict, None);
    assert_eq!(shown.layout.unwrap().name, "Other");
    assert_eq!(shown.save_state, SaveState::Saved);
}

/// A failed assertion releases the held send, so the test fails instead of hanging.
struct ReleaseOnPanic(td_companion::checkpoints::Checkpoints, Checkpoint);
impl Drop for ReleaseOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.release(self.1, true);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_held_at_a_checkpoint_leaves_edits_live_then_its_receipt_lands() {
    let (td, worker) = worker().await;
    let mut view = worker.subscribe();
    worker.dispatch(Command::NewLayout(NewLayout::Starter));
    after(&mut view, 1).await;
    td.checkpoints().hold(Checkpoint::BeforeEditorReadback);
    let _release = ReleaseOnPanic(td.checkpoints().clone(), Checkpoint::BeforeEditorReadback);
    worker.dispatch(Command::Send);
    td.until(|td| td.checkpoints().waiting(Checkpoint::BeforeEditorReadback))
        .await;
    assert_eq!(worker.view().save_state, SaveState::Sending);

    // The send waits; an edit still lands, without an outcome for the send yet.
    worker.dispatch(Command::Edit(Edit::Label {
        tile: "prev".into(),
        label: "Newer".into(),
    }));
    let shown = after(&mut view, 2).await;
    assert_eq!(shown.outcome.as_ref().unwrap().command, CommandKind::Edit);
    assert_eq!(shown.save_state, SaveState::Sending);
    let pages = shown.layout.unwrap().pages;
    assert_eq!(pages[0].tiles[0].tile.label, "Newer");

    td.checkpoints()
        .release(Checkpoint::BeforeEditorReadback, false);
    let shown = after(&mut view, 3).await;
    let outcome = shown.outcome.unwrap();
    assert_eq!(outcome.command, CommandKind::Send);
    assert_eq!(outcome.result, Ok(Done::Changed));
    // The newer edit wasn't in the send, so it stays unsent.
    assert_eq!(shown.save_state, SaveState::Unsent);
    assert!(shown.push_progress.is_some());
    assert_eq!(shown.layout.unwrap().pages[0].tiles[0].tile.label, "Newer");
}

#[tokio::test(flavor = "multi_thread")]
async fn equal_failures_get_new_outcome_numbers() {
    let (_td, worker) = worker().await;
    let mut view = worker.subscribe();
    worker.dispatch(Command::NewLayout(NewLayout::Blank));
    let missing = || {
        Command::Edit(Edit::RemoveTile {
            tile: "missing".into(),
        })
    };
    worker.dispatch(missing());
    let first = after(&mut view, 2).await.outcome.unwrap();
    worker.dispatch(missing());
    // Each one is published, so the window shows the second failure too.
    let second = after(&mut view, 3).await.outcome.unwrap();
    assert_eq!(first.number, 2);
    assert_eq!(second.number, 3);
    let (first, second) = (first.result.unwrap_err(), second.result.unwrap_err());
    assert_eq!(first.notice, second.notice);
    assert_ne!(first, second);
}

fn gif_bytes() -> Vec<u8> {
    let mut out = vec![];
    {
        let mut encoder = gif::Encoder::new(&mut out, 8, 4, &[0, 0, 0, 255, 0, 0]).unwrap();
        for value in [0, 1] {
            let frame = gif::Frame {
                width: 8,
                height: 4,
                delay: 20,
                buffer: std::borrow::Cow::Owned(vec![value; 32]),
                ..Default::default()
            };
            encoder.write_frame(&frame).unwrap();
        }
    }
    out
}
/// Makes the mock Device take animated Tiles.
fn offer_gifs(td: &TestCompanion) {
    let id = DeviceId::parse("mock0001").unwrap();
    let mut caps = td
        .handle()
        .devices()
        .get(&id)
        .unwrap()
        .capabilities
        .unwrap();
    caps.anim = Some(td_companion::devices::AnimationCapabilities {
        image: [128, 128],
        min_delay_ms: 100,
        page_tiles: 4,
        page_pixels: 65536,
    });
    caps.formats.push("gif".into());
    caps.tile_kinds.push("gif".into());
    caps.gif_frames = Some(60);
    caps.gif_pixels = Some(128 * 128 * 60);
    caps.asset_bytes = 1024 * 1024;
    td.deck("mock0001").set_gif_capabilities(true);
    td.handle()
        .devices()
        .set_capability_fixture(&id, Some(caps));
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_handle_cancels_gif_work() {
    use td_companion::{editor::GifWork, jobs::JobState};
    let (td, worker) = worker().await;
    offer_gifs(&td);
    let mut view = worker.subscribe();
    worker.dispatch(Command::NewLayout(NewLayout::Blank));
    td.hold_jobs();
    worker.dispatch(Command::AddGif {
        file: "cat.gif".into(),
        bytes: gif_bytes(),
        target: AddTarget::Auto,
    });
    let shown = after(&mut view, 2).await;
    assert!(shown.offers_gif);
    let id = created(&shown);
    let tile = &shown.layout.unwrap().pages[0].tiles[0];
    assert_eq!(tile.tile.id, id);
    assert!(matches!(
        tile.gif,
        Some(GifWork::Queued { .. } | GifWork::Preparing { .. })
    ));
    let job = td
        .handle()
        .snapshot()
        .jobs
        .into_iter()
        .find(|j| j.kind == "assets.render")
        .unwrap();

    drop(worker);
    // The view closes once the worker has stopped and cleaned up.
    tokio::time::timeout(Duration::from_secs(10), async {
        while view.changed().await.is_ok() {}
    })
    .await
    .expect("the worker stops when its last handle drops");
    td.release_jobs();
    assert_eq!(td.wait(&job).await.state, JobState::Cancelled);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_queued_behind_a_load_is_saved_when_the_companion_stops() {
    let (td, worker) = worker().await;
    let mut view = worker.subscribe();
    worker.dispatch(Command::NewLayout(NewLayout::Blank));
    worker.dispatch(Command::Add {
        kind: AddKind::Key,
        target: AddTarget::Cell { row: 0, col: 0 },
    });
    let id = created(&after(&mut view, 2).await);
    worker.dispatch(Command::Send);
    assert_eq!(after(&mut view, 3).await.save_state, SaveState::Saved);

    // A slow Device read holds the queue; a rename waits behind it.
    td.checkpoints().hold(Checkpoint::BeforeEditorLoad);
    let _release = ReleaseOnPanic(td.checkpoints().clone(), Checkpoint::BeforeEditorLoad);
    worker.dispatch(Command::Load);
    td.until(|td| td.checkpoints().waiting(Checkpoint::BeforeEditorLoad))
        .await;
    worker.dispatch(Command::Edit(Edit::Label {
        tile: id.clone(),
        label: "Renamed".into(),
    }));
    worker.dispatch(Command::Load);
    // Quit from the tray while the read is still out.
    td.handle().request_shutdown();
    td.checkpoints()
        .release(Checkpoint::BeforeEditorLoad, false);
    tokio::time::timeout(Duration::from_secs(10), worker.close())
        .await
        .expect("close returns once the Session has dropped");

    let reopened = td.handle().open_editor("mock0001").unwrap();
    let tile = reopened.draft().unwrap().effective_tile(&id).unwrap();
    assert_eq!(tile.label, "Renamed");
    // The second Load needed the Device, so it was skipped and says so.
    let stopped = worker.view().outcome.unwrap();
    assert_eq!(stopped.command, CommandKind::Load);
    assert!(matches!(
        *stopped.result.unwrap_err().error,
        SessionError::Editor(EditorError::Stopped)
    ));
}
