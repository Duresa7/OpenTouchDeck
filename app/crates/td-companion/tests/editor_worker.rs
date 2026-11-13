//! The editor Worker as a window drives it: commands in, one `EditorView` out on a `watch`.
use std::time::Duration;
use td_companion::{
    editor::{Command, EditorView, NewLayout, Worker},
    testing::TestCompanion,
};
use td_core::editing::Edit;
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
