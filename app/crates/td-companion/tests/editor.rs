use std::collections::BTreeMap;
use td_companion::editor::{Decision, Editor, EditorError};
use td_core::{
    Layout,
    Orientation::{self, Landscape, Portrait},
    editing::{Appearance, Capabilities, Edit},
    layout::{Action, ActionSink, Page, Tile},
};

fn layout(orientation: Orientation) -> Layout {
    let mut layout = Layout::parse(br#"{"schema":1,"name":"Draft","orientation":"landscape","pages":[{"kind":"grid","id":"main","title":"Main","rows":2,"cols":3,"tiles":[{"id":"t","label":"Tile","row":0,"col":0,"rowSpan":1,"colSpan":1,"icon":null,"action":{"type":"none"}}]}]}"#).unwrap();
    layout.orientation = orientation;
    layout
}
fn editor(dir: &tempfile::TempDir) -> Editor {
    Editor::open(dir.path().into(), "mock0001").unwrap()
}
fn tile(layout: &Layout, id: &str) -> Tile {
    layout
        .pages
        .iter()
        .find_map(|page| match page {
            Page::Grid { tiles, .. } => tiles.iter().find(|tile| tile.id == id).cloned(),
            _ => None,
        })
        .unwrap()
}
fn rename(label: &str) -> Edit {
    Edit::Label {
        tile: "t".into(),
        label: label.into(),
    }
}
fn invalid_action(id: &str) -> Edit {
    Edit::Action {
        tile: id.into(),
        action: Action::Page {
            page: "missing".into(),
        },
    }
}

#[test]
fn rejected_fields_duplicate_persist_reload_and_conflicting_push_form_one_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    editor.receive(layout(Landscape), "r1".into()).unwrap();
    assert!(editor.draft(Landscape).unwrap().is_sent());
    editor.edit(Landscape, rename("Mine")).unwrap();
    assert!(editor.edit(Landscape, invalid_action("t")).is_err());
    let copy = editor
        .edit(Landscape, Edit::Duplicate { tile: "t".into() })
        .unwrap()
        .unwrap();
    let draft = editor.draft(Landscape).unwrap();
    assert_eq!(draft.rejected().len(), 2);
    assert_eq!(
        draft.effective_tile(&copy).unwrap().action,
        Action::Page {
            page: "missing".into()
        }
    );
    assert_eq!(tile(draft.layout(), &copy).action, Action::None);
    assert!(matches!(
        editor.prepare_push(Landscape, Capabilities::default()),
        Err(EditorError::Rejected)
    ));
    editor.persist().unwrap();
    let mut editor = Editor::open(dir.path().into(), "mock0001").unwrap();
    assert_eq!(editor.draft(Landscape).unwrap().rejected().len(), 2);
    assert_eq!(
        editor
            .draft(Landscape)
            .unwrap()
            .effective_tile(&copy)
            .unwrap()
            .action,
        Action::Page {
            page: "missing".into()
        }
    );
    for id in ["t", copy.as_str()] {
        editor
            .edit(
                Landscape,
                Edit::Action {
                    tile: id.into(),
                    action: Action::None,
                },
            )
            .unwrap();
    }
    let plan = editor
        .prepare_push(Landscape, Capabilities::default())
        .unwrap();
    assert_eq!(plan.expected_revision(), Some("r1"));
    // The adapter receives a conditional-send conflict and reads the current Device Layout.
    let mut remote = layout(Landscape);
    remote.name = "On Device".into();
    editor.receive(remote.clone(), "r2".into()).unwrap();
    assert!(matches!(
        editor.prepare_push(Landscape, Capabilities::default()),
        Err(EditorError::Conflict)
    ));
    editor.persist().unwrap();
    let mut editor = Editor::open(dir.path().into(), "mock0001").unwrap();
    assert_eq!(
        editor
            .draft(Landscape)
            .unwrap()
            .conflict()
            .unwrap()
            .revision(),
        "r2"
    );
    assert!(matches!(
        editor.finish_push(plan, remote, "r2".into()),
        Err(EditorError::StalePush)
    ));
    editor.resolve(Landscape, Decision::KeepMine).unwrap();
    let plan = editor
        .prepare_push(Landscape, Capabilities::default())
        .unwrap();
    assert_eq!(plan.expected_revision(), Some("r2"));
    assert_eq!(tile(plan.layout(), "t").label, "Mine");
    let displayed = plan.layout().clone();
    editor.finish_push(plan, displayed, "r3".into()).unwrap();
    assert!(editor.draft(Landscape).unwrap().is_sent());
    editor.persist().unwrap();
    assert!(
        Editor::open(dir.path().into(), "mock0001")
            .unwrap()
            .draft(Landscape)
            .unwrap()
            .is_sent()
    );
}

#[test]
fn drafts_and_asset_replacements_are_isolated_by_device_and_orientation() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    let old = format!("sha256:{}", "a".repeat(64));
    let new = format!("sha256:{}", "b".repeat(64));
    for orientation in [Landscape, Portrait] {
        editor.start(layout(orientation)).unwrap();
        editor
            .edit(
                orientation,
                Edit::Renderer {
                    tile: "t".into(),
                    appearance: Appearance::Gif { asset: old.clone() },
                },
            )
            .unwrap();
        let plan = editor
            .prepare_push(orientation, Capabilities::default())
            .unwrap();
        let displayed = plan.layout().clone();
        editor.finish_push(plan, displayed, "r1".into()).unwrap();
    }
    editor.edit(Portrait, rename("Portrait only")).unwrap();
    editor
        .replace_assets(BTreeMap::from([(old.clone(), new.clone())]))
        .unwrap();
    for orientation in [Landscape, Portrait] {
        let draft = editor.draft(orientation).unwrap();
        assert_eq!(
            tile(draft.layout(), "t").asset.as_deref(),
            Some(new.as_str())
        );
        assert!(!draft.is_sent());
    }
    assert_eq!(
        tile(editor.draft(Landscape).unwrap().layout(), "t").label,
        "Tile"
    );
    assert!(
        editor
            .replace_assets(BTreeMap::from([(new.clone(), "bad".into())]))
            .is_err()
    );
    assert_eq!(
        tile(editor.draft(Landscape).unwrap().layout(), "t")
            .asset
            .as_deref(),
        Some(new.as_str())
    );
    editor.persist().unwrap();
    let editor = Editor::open(dir.path().into(), "mock0001").unwrap();
    assert_eq!(
        tile(editor.draft(Portrait).unwrap().layout(), "t").label,
        "Portrait only"
    );
    assert_eq!(
        editor
            .prepare_push(Landscape, Capabilities::default())
            .unwrap()
            .expected_revision(),
        Some("r1")
    );
    assert!(
        Editor::open(dir.path().into(), "mock0002")
            .unwrap()
            .draft(Landscape)
            .is_none()
    );
}

#[test]
fn choosing_device_discards_rejections_while_keep_mine_preserves_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    editor.start(layout(Landscape)).unwrap();
    assert!(editor.edit(Landscape, rename("")).is_err());
    let mut remote = layout(Landscape);
    remote.name = "Remote".into();
    editor.receive(remote.clone(), "r1".into()).unwrap();
    assert_eq!(
        editor
            .draft(Landscape)
            .unwrap()
            .conflict()
            .unwrap()
            .layout(),
        &remote
    );
    editor.resolve(Landscape, Decision::KeepMine).unwrap();
    assert_eq!(editor.draft(Landscape).unwrap().rejected().len(), 1);
    editor.receive(remote.clone(), "r2".into()).unwrap();
    editor.resolve(Landscape, Decision::UseDevice).unwrap();
    assert_eq!(editor.draft(Landscape).unwrap().layout(), &remote);
    assert!(editor.draft(Landscape).unwrap().rejected().is_empty());
    assert!(editor.draft(Landscape).unwrap().is_sent());
}

#[test]
fn send_snapshots_require_matching_readback_and_preserve_newer_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    editor.start(layout(Landscape)).unwrap();
    let plan = editor
        .prepare_push(Landscape, Capabilities::default())
        .unwrap();
    let mut mismatch = plan.layout().clone();
    mismatch.name = "Something else".into();
    assert!(matches!(
        editor.finish_push(plan.clone(), mismatch, "bad".into()),
        Err(EditorError::PushMismatch)
    ));
    assert!(!editor.draft(Landscape).unwrap().is_sent());
    editor.edit(Landscape, rename("Newer edit")).unwrap();
    let displayed = plan.layout().clone();
    editor
        .finish_push(plan.clone(), displayed.clone(), "r1".into())
        .unwrap();
    assert!(!editor.draft(Landscape).unwrap().is_sent());
    assert_eq!(
        tile(editor.draft(Landscape).unwrap().layout(), "t").label,
        "Newer edit"
    );
    assert!(matches!(
        editor.finish_push(plan, displayed, "stale".into()),
        Err(EditorError::StalePush)
    ));
}

#[test]
fn structural_changes_recheck_rejected_fields_and_remove_stale_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    editor.start(layout(Landscape)).unwrap();
    assert!(editor.edit(Landscape, invalid_action("t")).is_err());
    editor
        .edit(
            Landscape,
            Edit::AddPage {
                page: Page::Grid {
                    id: "missing".into(),
                    title: "Target".into(),
                    rows: 1,
                    cols: 1,
                    tiles: vec![],
                },
            },
        )
        .unwrap();
    assert!(editor.draft(Landscape).unwrap().rejected().is_empty());
    assert_eq!(
        tile(editor.draft(Landscape).unwrap().layout(), "t").action,
        Action::Page {
            page: "missing".into()
        }
    );
    assert!(editor.edit(Landscape, rename("")).is_err());
    editor
        .edit(Landscape, Edit::RemoveTile { tile: "t".into() })
        .unwrap();
    assert!(editor.draft(Landscape).unwrap().rejected().is_empty());
    editor.persist().unwrap();
    assert!(Editor::open(dir.path().into(), "mock0001").is_ok());
}

#[test]
fn sink_changes_use_the_effective_rejected_action_and_core_wire_policy() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    editor.start(layout(Landscape)).unwrap();
    assert!(
        editor
            .edit(
                Landscape,
                Edit::Action {
                    tile: "t".into(),
                    action: Action::Key {
                        sink: Some(ActionSink::Bluetooth),
                        usage: 0,
                        mods: vec![]
                    }
                }
            )
            .is_err()
    );
    assert!(
        editor
            .edit(
                Landscape,
                Edit::Sink {
                    tile: "t".into(),
                    sink: ActionSink::Link
                }
            )
            .is_err()
    );
    let draft = editor.draft(Landscape).unwrap();
    assert_eq!(
        draft.effective_tile("t").unwrap().action,
        Action::Key {
            sink: None,
            usage: 0,
            mods: vec![]
        }
    );
    assert!(matches!(
        editor.prepare_push(Landscape, Capabilities::default()),
        Err(EditorError::Rejected)
    ));
    editor.discard_rejected(Landscape).unwrap();
    assert!(
        editor
            .prepare_push(Landscape, Capabilities::default())
            .is_ok()
    );
}

#[derive(serde::Deserialize)]
struct Case {
    name: String,
    sinks: Option<Vec<String>>,
    action: Action,
    compatibility: String,
}
#[test]
fn push_admission_uses_the_shared_compatibility_cases() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../../../../schema/editor-compatibility.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    for case in cases {
        editor.start(layout(Landscape)).unwrap();
        editor
            .edit(
                Landscape,
                Edit::Action {
                    tile: "t".into(),
                    action: case.action,
                },
            )
            .unwrap();
        let result = editor.prepare_push(
            Landscape,
            Capabilities {
                sinks: case.sinks.as_deref(),
            },
        );
        let got = match result {
            Ok(_) => "supported",
            Err(EditorError::UnknownCapabilities) => "unknown",
            Err(EditorError::Incompatible(_)) => "unsupported",
            other => panic!("{other:?}"),
        };
        assert_eq!(got, case.compatibility, "{}", case.name);
        editor
            .edit(
                Landscape,
                Edit::Sink {
                    tile: "t".into(),
                    sink: ActionSink::Link,
                },
            )
            .unwrap();
        let plan = editor
            .prepare_push(Landscape, Capabilities { sinks: Some(&[]) })
            .unwrap();
        assert!(
            !String::from_utf8(plan.layout().to_canonical_json())
                .unwrap()
                .contains("\"sink\"")
        );
    }
}

#[test]
fn invalid_device_ids_and_corrupt_storage_are_reported_without_overwriting_files() {
    let dir = tempfile::tempdir().unwrap();
    for device in ["", "../escape", "UPPER", "/absolute"] {
        assert!(matches!(
            Editor::open(dir.path().into(), device),
            Err(EditorError::InvalidDevice)
        ));
    }
    let folder = dir.path().join("editor-drafts-v1");
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("mock0001.json");
    std::fs::write(&path, b"broken").unwrap();
    assert!(Editor::open(dir.path().into(), "mock0001").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken");
}

#[test]
fn a_conflicting_send_to_a_now_empty_slot_keeps_the_draft_and_drops_the_stale_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    editor.receive(layout(Landscape), "r1".into()).unwrap();
    editor.edit(Landscape, rename("Mine")).unwrap();
    let stale = editor
        .prepare_push(Landscape, Capabilities::default())
        .unwrap();
    assert_eq!(stale.expected_revision(), Some("r1"));
    editor.receive_missing(Landscape).unwrap();
    assert!(!editor.draft(Landscape).unwrap().is_sent());
    let displayed = stale.layout().clone();
    assert!(matches!(
        editor.finish_push(stale, displayed, "stale".into()),
        Err(EditorError::StalePush)
    ));
    editor.persist().unwrap();
    let editor = Editor::open(dir.path().into(), "mock0001").unwrap();
    let retry = editor
        .prepare_push(Landscape, Capabilities::default())
        .unwrap();
    assert_eq!(retry.expected_revision(), None);
    assert_eq!(tile(retry.layout(), "t").label, "Mine");
}

#[test]
fn a_saved_rejection_that_is_now_valid_is_applied_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    editor.start(layout(Landscape)).unwrap();
    editor.persist().unwrap();
    let path = dir.path().join("editor-drafts-v1/mock0001.json");
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stored["drafts"][0]["rejected"] = serde_json::json!([rename("Recovered")]);
    std::fs::write(path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let editor = Editor::open(dir.path().into(), "mock0001").unwrap();
    let draft = editor.draft(Landscape).unwrap();
    assert!(draft.rejected().is_empty());
    assert_eq!(draft.effective_tile("t").unwrap().label, "Recovered");
}

#[test]
fn identical_receive_during_send_keeps_the_newer_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&dir);
    editor.receive(layout(Landscape), "before".into()).unwrap();
    editor.edit(Landscape, rename("Sending")).unwrap();
    let plan = editor
        .prepare_push(Landscape, Capabilities::default())
        .unwrap();
    let displayed = plan.layout().clone();
    editor.receive(displayed.clone(), "newer".into()).unwrap();
    editor.finish_push(plan, displayed, "older".into()).unwrap();
    assert!(editor.draft(Landscape).unwrap().is_sent());
    assert_eq!(
        editor
            .prepare_push(Landscape, Capabilities::default())
            .unwrap()
            .expected_revision(),
        Some("newer")
    );
}

#[tokio::test]
async fn native_session_sends_reads_back_and_restores_per_device_drafts() {
    let td = td_companion::testing::TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    let handle = td.handle();
    let mut session = handle.open_editor(&device).unwrap();
    session.start(layout(Landscape)).unwrap();
    assert_eq!(session.selection(), (Some("main"), None));
    session.select("main", Some("t"));
    session.edit(Edit::Duplicate { tile: "t".into() }).unwrap();
    assert!(session.selection().1.is_some());
    session.send(Capabilities::default()).await.unwrap();
    assert!(session.draft().unwrap().is_sent());
    session.edit(rename("Unsent")).unwrap();
    drop(session);
    let mut restored = handle.open_editor(&device).unwrap();
    assert!(!restored.draft().unwrap().is_sent());
    restored.load().await.unwrap();
    assert!(restored.draft().unwrap().conflict().is_some());
    restored.resolve(Decision::UseDevice).unwrap();
    assert!(restored.draft().unwrap().is_sent());
    assert!(handle.open_editor("mock0002").unwrap().draft().is_none());
}

#[test]
fn preset_fallback_preserves_interactive_renderers_and_refuses_bad_bindings() {
    use td_companion::editor::placed_appearance;
    use td_core::layout::{StreamType, TileRender};
    use td_host::integrations::{DataStream, PresetTile};
    let mut preset = PresetTile {
        id: "level".into(),
        name: "Level".into(),
        action: serde_json::json!({"type":"widget","source":"volume","field":"level","account":""}),
        render: Some(TileRender::Graph),
        streams: None,
        input: false,
    };
    let streams = [DataStream {
        id: "level".into(),
        kind: "numeric".into(),
        unit: None,
        settable: true,
    }];
    assert!(placed_appearance(&preset, "volume", &streams, true).is_some());
    assert!(placed_appearance(&preset, "volume", &streams, false).is_none());
    assert!(placed_appearance(&preset, "other", &streams, true).is_none());
    assert!(placed_appearance(&preset, "volume", &[], true).is_none());
    preset.render = Some(TileRender::Bar);
    preset.input = true;
    assert!(placed_appearance(&preset, "volume", &streams, false).is_some());
    preset.streams = Some(BTreeMap::from([("value".into(), StreamType::Series)]));
    assert!(placed_appearance(&preset, "volume", &streams, true).is_none());
    preset.render = Some(TileRender::NowPlaying);
    preset.streams = Some(BTreeMap::from([
        ("title".into(), StreamType::Text),
        ("artist".into(), StreamType::Text),
        ("cover".into(), StreamType::Image),
        ("state".into(), StreamType::Number),
    ]));
    assert!(placed_appearance(&preset, "volume", &streams, false).is_none());
    preset.input = false;
    assert!(placed_appearance(&preset, "volume", &streams, false).is_some());
}

#[tokio::test]
async fn session_conditional_send_captures_conflict_and_keep_mine_retries() {
    let td = td_companion::testing::TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    let handle = td.handle();
    let mut mine = handle.open_editor(&device).unwrap();
    mine.start(layout(Landscape)).unwrap();
    mine.send(Capabilities::default()).await.unwrap();
    let mut other = handle.open_editor(&device).unwrap();
    other.edit(rename("Other")).unwrap();
    other.send(Capabilities::default()).await.unwrap();
    mine.edit(rename("Mine")).unwrap();
    assert!(mine.send(Capabilities::default()).await.is_err());
    assert!(mine.draft().unwrap().conflict().is_some());
    mine.resolve(Decision::KeepMine).unwrap();
    mine.send(Capabilities::default()).await.unwrap();
    assert!(mine.draft().unwrap().is_sent());
    assert_eq!(
        mine.draft().unwrap().effective_tile("t").unwrap().label,
        "Mine"
    );
}
