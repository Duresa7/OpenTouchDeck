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
    assert!(matches!(
        *outcome.result.unwrap_err().error,
        SessionError::Editor(EditorError::Rejected)
    ));
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

#[tokio::test(flavor = "multi_thread")]
async fn a_send_held_at_a_checkpoint_leaves_edits_live_then_its_receipt_lands() {
    let (td, worker) = worker().await;
    let mut view = worker.subscribe();
    worker.dispatch(Command::NewLayout(NewLayout::Starter));
    after(&mut view, 1).await;
    td.checkpoints().hold(Checkpoint::BeforeEditorReadback);
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
