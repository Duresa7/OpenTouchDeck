//! Behavioral ports of the 1.x editor tests, exercised through the native Session with a mock Device.
use std::collections::BTreeMap;
use td_companion::{
    editor::{AddKind, AddTarget, Session},
    testing::TestCompanion,
};
use td_core::{
    Layout, Orientation,
    editing::{Appearance, Edit},
    layout::{Action, Page, StreamType, TileRender},
};
use td_host::integrations::{DataStream, PresetTile};

async fn session() -> (TestCompanion, Session) {
    let td = TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    let session = td.handle().open_editor(&device).unwrap();
    (td, session)
}
fn blank() -> Layout {
    Layout {
        schema: 1,
        name: "New Layout".into(),
        orientation: Orientation::Landscape,
        full_screen: false,
        pages: vec![Page::Grid {
            id: "page1".into(),
            title: "Page 1".into(),
            rows: 2,
            cols: 3,
            tiles: vec![],
        }],
    }
}
fn tiles(session: &Session) -> Vec<td_core::layout::Tile> {
    session
        .draft()
        .unwrap()
        .layout()
        .pages
        .iter()
        .filter_map(|p| match p {
            Page::Grid { tiles, .. } => Some(tiles.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}
fn preset(render: Option<TileRender>) -> PresetTile {
    PresetTile {
        id: "level".into(),
        name: "five hour pct".into(),
        action: serde_json::json!({"type":"widget","source":"volume","field":"level","account":""}),
        render,
        streams: None,
        input: false,
    }
}
fn streams() -> Vec<DataStream> {
    vec![DataStream {
        id: "level".into(),
        kind: "numeric".into(),
        unit: Some("%".into()),
        settable: true,
    }]
}

#[tokio::test]
async fn add_targets_clicked_cell_selects_default_action_and_refuses_overlap_atomically() {
    let (_td, mut s) = session().await;
    s.start(blank()).unwrap();
    assert_eq!(s.selection(), (Some("page1"), None));
    let id = s
        .add(AddKind::Consumer, AddTarget::Cell { row: 1, col: 2 })
        .unwrap();
    assert_eq!(s.selection(), (Some("page1"), Some(id.as_str())));
    assert_eq!((tiles(&s)[0].row, tiles(&s)[0].col), (1, 2));
    assert_eq!(
        tiles(&s)[0].action,
        Action::Consumer {
            usage: 205,
            sink: None
        }
    );
    let before = s.draft().unwrap().layout().clone();
    assert!(
        s.add(AddKind::Key, AddTarget::Cell { row: 1, col: 2 })
            .is_err()
    );
    assert_eq!(s.draft().unwrap().layout(), &before);
}
#[tokio::test]
async fn add_picks_first_free_cell_and_creates_page_when_full() {
    let (_td, mut s) = session().await;
    s.start(blank()).unwrap();
    for col in 0..3 {
        s.add(AddKind::None, AddTarget::Cell { row: 0, col })
            .unwrap();
    }
    assert_eq!(s.add_target().unwrap(), AddTarget::Cell { row: 1, col: 0 });
    for _ in 0..3 {
        s.add(AddKind::None, AddTarget::Auto).unwrap();
    }
    assert_eq!(s.add_target().unwrap(), AddTarget::NewPage);
    let id = s.add(AddKind::Key, AddTarget::Auto).unwrap();
    assert_eq!(s.selection(), (Some("page2"), Some(id.as_str())));
    assert_eq!(s.draft().unwrap().layout().pages.len(), 2);
    assert!(
        matches!(&s.draft().unwrap().layout().pages[1], Page::Grid { title, tiles, .. } if title == "Page 2" && tiles.len() == 1 && tiles[0].row == 0 && tiles[0].col == 0)
    );
}
#[tokio::test]
async fn incomplete_add_forms_start_as_nothing_keep_rejections_and_clear_removes_them() {
    let (_td, mut s) = session().await;
    s.start(blank()).unwrap();
    for kind in [AddKind::Launch, AddKind::Open, AddKind::Command] {
        let id = s.add(kind, AddTarget::Auto).unwrap();
        assert_eq!(tiles(&s).last().unwrap().action, Action::None);
        assert_ne!(
            s.draft().unwrap().effective_tile(&id).unwrap().action,
            Action::None
        );
        assert_eq!(s.draft().unwrap().rejected().len(), 1);
        s.edit(Edit::RemoveTile { tile: id }).unwrap();
        assert!(s.draft().unwrap().rejected().is_empty());
        assert_eq!(s.selection().1, None);
    }
    for kind in [
        AddKind::Key,
        AddKind::Mod,
        AddKind::Consumer,
        AddKind::Page,
        AddKind::None,
    ] {
        s.add(kind, AddTarget::Auto).unwrap();
        assert!(s.draft().unwrap().rejected().is_empty());
    }
}
#[tokio::test]
async fn preset_preview_matches_one_by_one_placement_with_sentence_case_label_and_plain_fallback() {
    let (_td, mut s) = session().await;
    s.start(blank()).unwrap();
    let p = preset(Some(TileRender::Graph));
    let preview = s.preset_tile(&p, "volume", &streams(), true).unwrap();
    let id = s
        .place_preset(
            &p,
            "volume",
            &streams(),
            true,
            AddTarget::Cell { row: 0, col: 0 },
        )
        .unwrap();
    let mut placed = tiles(&s)[0].clone();
    placed.id = "preset".into();
    assert_eq!(preview, placed);
    assert_eq!(preview.label, "Five hour pct");
    assert_eq!((preview.row_span, preview.col_span), (1, 1));
    assert_eq!(s.selection().1, Some(id.as_str()));
    assert_eq!(s.draft().unwrap().layout().schema, 2);
    let plain = s.preset_tile(&p, "volume", &streams(), false).unwrap();
    assert_eq!(plain.render, None);
    assert_eq!(plain.action, preview.action);
    for (name, expected) in [
        ("a very long preset tile name", "A very long pres"),
        ("   ", "Tile"),
        ("Kitchen tv 👩‍👩‍👧‍👦", "Kitchen tv"),
    ] {
        let mut p = preset(None);
        p.name = name.into();
        assert_eq!(
            s.preset_tile(&p, "volume", &[], true).unwrap().label,
            expected
        );
    }
}
#[tokio::test]
async fn invalid_preset_action_changes_nothing_even_when_a_new_page_is_needed() {
    let (_td, mut s) = session().await;
    s.start(blank()).unwrap();
    let mut p = preset(None);
    p.action = serde_json::json!({"type":"widget","source":"","field":"level","account":""});
    let before = s.draft().unwrap().layout().clone();
    assert!(
        s.place_preset(&p, "volume", &streams(), true, AddTarget::NewPage)
            .is_err()
    );
    assert_eq!(s.draft().unwrap().layout(), &before);
}
#[tokio::test]
async fn bar_and_now_playing_presets_keep_renderers_without_firmware_and_input_only_on_bars() {
    let (_td, mut s) = session().await;
    s.start(blank()).unwrap();
    let mut p = preset(Some(TileRender::Bar));
    p.input = true;
    p.streams = Some(BTreeMap::from([
        ("value".into(), StreamType::Number),
        ("muted".into(), StreamType::Number),
    ]));
    let id = s
        .place_preset(&p, "volume", &streams(), false, AddTarget::Auto)
        .unwrap();
    assert!(tiles(&s)[0].input);
    s.edit(Edit::Input {
        tile: id.clone(),
        on: false,
    })
    .unwrap();
    assert!(!tiles(&s)[0].input);
    s.edit(Edit::Input {
        tile: id.clone(),
        on: true,
    })
    .unwrap();
    assert!(tiles(&s)[0].input);
    let binding = tiles(&s)[0].action.clone();
    s.edit(Edit::Renderer {
        tile: id.clone(),
        appearance: Appearance::Plain {
            icon: None,
            asset: None,
            action: Some(binding.clone()),
        },
    })
    .unwrap();
    assert!(!tiles(&s)[0].input);
    assert_eq!(tiles(&s)[0].render, None);
    s.edit(Edit::Renderer {
        tile: id.clone(),
        appearance: Appearance::Live {
            render: TileRender::Graph,
            streams: BTreeMap::from([("value".into(), StreamType::Number)]),
            input: false,
            action: binding.clone(),
        },
    })
    .unwrap();
    assert!(
        s.edit(Edit::Input {
            tile: id.clone(),
            on: true
        })
        .is_err()
    );
    s.edit(Edit::Renderer {
        tile: id.clone(),
        appearance: Appearance::Gif {
            asset: format!("sha256:{}", "a".repeat(64)),
        },
    })
    .unwrap();
    assert!(!tiles(&s)[0].input);
    assert!(tiles(&s)[0].streams.is_none());
    s.edit(Edit::Renderer {
        tile: id,
        appearance: Appearance::Live {
            render: TileRender::Bar,
            streams: BTreeMap::from([("value".into(), StreamType::Number)]),
            input: true,
            action: binding,
        },
    })
    .unwrap();
    assert!(tiles(&s)[0].asset.is_none());
    assert!(tiles(&s)[0].input);
    p.render = Some(TileRender::NowPlaying);
    p.input = false;
    p.streams = Some(BTreeMap::from([
        ("title".into(), StreamType::Text),
        ("artist".into(), StreamType::Text),
        ("cover".into(), StreamType::Image),
        ("state".into(), StreamType::Number),
    ]));
    let id = s
        .place_preset(&p, "volume", &streams(), false, AddTarget::Auto)
        .unwrap();
    let tile = s.draft().unwrap().effective_tile(&id).unwrap();
    assert_eq!(tile.render, Some(TileRender::NowPlaying));
    assert!(!tile.input);
    assert!(s.edit(Edit::Input { tile: id, on: true }).is_err());
}

#[tokio::test]
async fn invalid_import_explains_file_and_preserves_drafts_selection_and_storage() {
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    let id = s.add(AddKind::Key, AddTarget::Auto).unwrap();
    let before = s.export().unwrap();
    let error = s
        .import("broken.json", br#"{"schema": 1, "name": "Broken""#)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("broken.json is not a Layout file. Nothing changed")
    );
    assert_eq!(s.export().unwrap(), before);
    assert_eq!(s.selection().1, Some(id.as_str()));
    let reopened = td.handle().open_editor("mock0001").unwrap();
    assert_eq!(reopened.export().unwrap(), before);
}
#[tokio::test]
async fn import_opens_its_orientation_and_graph_export_import_round_trips() {
    let (_td, mut s) = session().await;
    s.start(blank()).unwrap();
    s.place_preset(
        &preset(Some(TileRender::Graph)),
        "volume",
        &streams(),
        true,
        AddTarget::Auto,
    )
    .unwrap();
    let graph = s.export().unwrap();
    s.import("graph.json", graph.as_bytes()).unwrap();
    assert_eq!(tiles(&s)[0].render, Some(TileRender::Graph));
    assert_eq!(
        tiles(&s)[0].streams.as_ref().unwrap()["value"],
        StreamType::Number
    );
    assert_eq!(s.export().unwrap(), graph);
    let mut portrait = blank();
    portrait.orientation = Orientation::Portrait;
    portrait.name = "Tall".into();
    s.import("tall.json", &portrait.to_canonical_json())
        .unwrap();
    assert_eq!(s.orientation(), Orientation::Portrait);
    assert_eq!(s.selection(), (Some("page1"), None));
    assert_eq!(s.draft().unwrap().layout().name, "Tall");
    s.select_orientation(Orientation::Landscape);
    assert_eq!(s.export().unwrap(), graph);
}
