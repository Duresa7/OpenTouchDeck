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

#[tokio::test]
async fn load_without_draft_then_keep_mine_and_replace_preserve_conditional_revisions() {
    use td_companion::editor::Decision;
    use td_core::editing::Capabilities;
    let (td, mut writer) = session().await;
    writer.start(blank()).unwrap();
    writer.send(Capabilities::default()).await.unwrap();
    // Delete its local checkpoint to arrange a first open with no draft.
    std::fs::remove_file(td.data_dir().join("editor-drafts-v1/mock0001.json")).unwrap();
    let mut s = td.handle().open_editor("mock0001").unwrap();
    assert!(s.draft().is_none());
    s.load().await.unwrap();
    assert!(s.draft().unwrap().is_sent());
    s.edit(Edit::Name {
        name: "Mine".into(),
    })
    .unwrap();
    s.load().await.unwrap();
    assert_eq!(s.draft().unwrap().layout().name, "Mine");
    assert!(s.draft().unwrap().conflict().is_some());
    s.resolve(Decision::KeepMine).unwrap();
    s.send(Capabilities::default()).await.unwrap();
    assert!(s.draft().unwrap().is_sent());
    let remote = {
        let mut l = blank();
        l.name = "Elsewhere".into();
        l
    };
    td.write_layout_unseen(
        &td_companion::devices::DeviceId::parse("mock0001").unwrap(),
        &remote,
    )
    .await;
    s.edit(Edit::Name {
        name: "Local".into(),
    })
    .unwrap();
    let error = s.send(Capabilities::default()).await.unwrap_err();
    assert_eq!(error.refusal().unwrap().code, "layout.changed");
    s.resolve(Decision::UseDevice).unwrap();
    assert_eq!(s.draft().unwrap().layout().name, "Elsewhere");
    assert!(s.draft().unwrap().is_sent());
}
#[tokio::test]
async fn orientation_without_device_layout_preserves_draft_and_disconnected_send_is_typed() {
    use td_core::editing::Capabilities;
    let (td, mut s) = session().await;
    s.select_orientation(Orientation::Portrait);
    assert_eq!(
        s.load().await.unwrap_err().refusal().unwrap().code,
        "layout.not_found"
    );
    let mut portrait = blank();
    portrait.orientation = Orientation::Portrait;
    s.start(portrait.clone()).unwrap();
    assert_eq!(
        s.load().await.unwrap_err().refusal().unwrap().code,
        "layout.not_found"
    );
    assert_eq!(s.draft().unwrap().layout(), &portrait);
    td.unplug("mock0001").await;
    assert_eq!(
        s.send(Capabilities::default())
            .await
            .unwrap_err()
            .refusal()
            .unwrap()
            .code,
        "device.not_connected"
    );
}
#[tokio::test]
async fn busy_send_is_typed_and_success_follows_job_updated_with_progress() {
    use td_companion::{companion::Change, jobs::JobState};
    use td_core::editing::Capabilities;
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    td.hold_jobs();
    let job = td
        .handle()
        .layout_operations()
        .push(
            "mock0001".into(),
            Orientation::Landscape,
            blank().to_canonical_json(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        s.send(Capabilities::default())
            .await
            .unwrap_err()
            .refusal()
            .unwrap()
            .code,
        "device.busy"
    );
    td.release_jobs();
    td.wait(&job).await;
    let mut changes = s.observe();
    s.send(Capabilities::default()).await.unwrap();
    let mut seen = vec![];
    while let Ok(event) = changes.try_recv() {
        if let Change::JobUpdated(update) = event {
            seen.push(update.0);
        }
    }
    assert!(seen.iter().any(|j| j.state == JobState::Running));
    assert!(seen.iter().any(|j| j.state == JobState::Succeeded));
    assert_eq!(s.last_job().unwrap().state, JobState::Succeeded);
    assert!(
        seen.iter()
            .any(|j| j.progress.get("assets_total").is_some())
    );
}
#[tokio::test]
async fn persisted_drafts_are_per_device_orientation_and_invalid_saved_layout_is_dropped() {
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    let mut tall = blank();
    tall.orientation = Orientation::Portrait;
    tall.name = "Tall".into();
    s.start(tall).unwrap();
    let mut reopened = td.handle().open_editor("mock0001").unwrap();
    assert_eq!(reopened.draft().unwrap().layout().name, "New Layout");
    reopened.select_orientation(Orientation::Portrait);
    assert_eq!(reopened.draft().unwrap().layout().name, "Tall");
    assert!(
        td.handle()
            .open_editor("mock0002")
            .unwrap()
            .draft()
            .is_none()
    );
    let path = td.data_dir().join("editor-drafts-v1/mock0001.json");
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stored["drafts"][0]["layout"] = "{\"schema\":1}".into();
    std::fs::write(path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let mut reopened = td.handle().open_editor("mock0001").unwrap();
    assert!(reopened.draft().is_none());
    reopened.select_orientation(Orientation::Portrait);
    assert_eq!(reopened.draft().unwrap().layout().name, "Tall");
}
#[tokio::test]
async fn rejected_send_selects_invalid_tile_and_repair_preserves_machine_readable_issues() {
    use td_core::editing::Capabilities;
    let (_td, mut s) = session().await;
    s.start(blank()).unwrap();
    let id = s.add(AddKind::Command, AddTarget::Auto).unwrap();
    s.select("page1", None);
    assert!(s.send(Capabilities::default()).await.is_err());
    assert_eq!(s.selection().1, Some(id.as_str()));
    let error = s
        .edit(Edit::Action {
            tile: id.clone(),
            action: Action::Open { target: "".into() },
        })
        .unwrap_err();
    assert!(
        matches!(error, td_companion::editor::EditorError::Edit(td_core::editing::EditError::Invalid(ref problems)) if problems[0].path.ends_with("/action/target"))
    );
    s.edit(Edit::Action {
        tile: id,
        action: Action::Open {
            target: "https://example.com".into(),
        },
    })
    .unwrap();
    s.send(Capabilities::default()).await.unwrap();
    assert!(s.draft().unwrap().is_sent());
}

#[tokio::test]
async fn companion_422_and_rejected_fields_keep_structured_rules_and_paths() {
    use td_companion::api::ApiError;
    use td_core::editing::Capabilities;
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    let limit = td.handle().snapshot().devices[0]
        .capabilities
        .as_ref()
        .unwrap()
        .live_images;
    assert!(limit < 64);
    let mut p = preset(Some(TileRender::NowPlaying));
    p.streams = Some(BTreeMap::from([
        ("title".into(), StreamType::Text),
        ("artist".into(), StreamType::Text),
        ("cover".into(), StreamType::Image),
        ("state".into(), StreamType::Number),
    ]));
    for _ in 0..=limit {
        s.place_preset(&p, "volume", &streams(), true, AddTarget::Auto)
            .unwrap();
    }
    let error = s.send(Capabilities::default()).await.unwrap_err();
    assert_eq!(error.refusal().unwrap().code, "layout.live_image_limit");
    assert!(
        matches!(error, td_companion::editor::SessionError::Layout(ref e) if e.status() == axum::http::StatusCode::UNPROCESSABLE_ENTITY)
    );
    let id = tiles(&s)[0].id.clone();
    let error = s
        .edit(Edit::Label {
            tile: id,
            label: "a label over sixteen characters".into(),
        })
        .unwrap_err();
    let refusal = td_companion::editor::SessionError::from(error)
        .refusal()
        .unwrap();
    assert_eq!(refusal.code, "layout.invalid");
    assert_eq!(
        refusal.details.as_ref().unwrap()["issues"][0]["rule"],
        "label.too-long"
    );
    assert!(
        refusal.details.unwrap()["issues"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("/label")
    );
}

#[tokio::test]
async fn asset_replacement_notifications_update_open_stored_and_recent_drafts_and_next_send() {
    use td_core::editing::Capabilities;
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    let id = s.add(AddKind::None, AddTarget::Auto).unwrap();
    let old = td_link::asset::asset_id(&td_host::icons::builtin()[0].file);
    let new = td_link::asset::asset_id(&td_host::icons::builtin()[1].file);
    let assets = td.data_dir().join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    for (id, icon) in [
        (&old, &td_host::icons::builtin()[0]),
        (&new, &td_host::icons::builtin()[1]),
    ] {
        std::fs::write(assets.join(format!("{id}.bin")), &icon.file).unwrap();
    }
    let appearance = |asset: &str| Edit::Renderer {
        tile: id.clone(),
        appearance: Appearance::Plain {
            icon: None,
            asset: Some(format!("sha256:{asset}")),
            action: None,
        },
    };
    s.edit(appearance(&old)).unwrap();
    s.send(Capabilities::default()).await.unwrap();
    let path = td.data_dir().join("brandfetch/provenance.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"replaced_by": {old.clone(): new.clone()}}))
            .unwrap(),
    )
    .unwrap();
    td.notify_asset_replacements();
    s.changed().await.unwrap();
    assert_eq!(tiles(&s)[0].asset, Some(format!("sha256:{new}")));
    assert!(!s.draft().unwrap().is_sent());
    let mut reopened = td.handle().open_editor("mock0001").unwrap();
    assert_eq!(tiles(&reopened)[0].asset, Some(format!("sha256:{new}")));
    reopened.edit(appearance(&old)).unwrap();
    assert_eq!(tiles(&reopened)[0].asset, Some(format!("sha256:{new}")));
    reopened.send(Capabilities::default()).await.unwrap();
    assert!(reopened.draft().unwrap().is_sent());
    let device = td
        .handle()
        .layout_operations()
        .read("mock0001".into(), Orientation::Landscape)
        .await
        .unwrap();
    assert!(
        String::from_utf8(device.layout.to_canonical_json())
            .unwrap()
            .contains(&new)
    );
    let unchanged = reopened.export().unwrap();
    reopened
        .replace_assets(BTreeMap::from([(
            format!("sha256:{}", "c".repeat(64)),
            format!("sha256:{}", "d".repeat(64)),
        )]))
        .unwrap();
    assert_eq!(reopened.export().unwrap(), unchanged);
}
#[tokio::test]
async fn pruning_keeps_assets_used_by_both_orientation_drafts() {
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    let device = td_companion::devices::DeviceId::parse("mock0001").unwrap();
    let ids = ["00000000000000aa", "00000000000000bb", "00000000000000cc"];
    td.deck(&device)
        .add_icons(ids.iter().map(|id| (*id).into()).collect::<Vec<String>>());
    let id = s.add(AddKind::None, AddTarget::Auto).unwrap();
    s.edit(Edit::Renderer {
        tile: id,
        appearance: Appearance::Plain {
            icon: Some(format!("file:{}", ids[0])),
            asset: None,
            action: None,
        },
    })
    .unwrap();
    let mut tall = blank();
    tall.orientation = Orientation::Portrait;
    s.start(tall).unwrap();
    let id = s.add(AddKind::None, AddTarget::Auto).unwrap();
    s.edit(Edit::Renderer {
        tile: id,
        appearance: Appearance::Plain {
            icon: Some(format!("file:{}", ids[1])),
            asset: None,
            action: None,
        },
    })
    .unwrap();
    let result = s.prune_assets().await.unwrap();
    assert_eq!(result.removed, vec![ids[2].to_string()]);
    assert!(td.deck(&device).icons().contains(ids[0]));
    assert!(td.deck(&device).icons().contains(ids[1]));
}

#[tokio::test]
async fn stream_picker_lists_numeric_streams_in_order_with_preset_names_units_and_empty_fallback() {
    let (td, s) = session().await;
    let mut fixture = td.handle().integrations().summaries()[0].clone();
    fixture.id = "hub".into();
    fixture.name = "Hub".into();
    fixture.presets = vec![preset(Some(TileRender::Graph))];
    fixture.presets[0].action =
        serde_json::json!({"type":"widget","source":"hub","field":"level","account":""});
    fixture.streams = vec![
        DataStream {
            id: "title".into(),
            kind: "text".into(),
            unit: None,
            settable: false,
        },
        streams()[0].clone(),
        DataStream {
            id: "network_send_bytes_s".into(),
            kind: "numeric".into(),
            unit: Some("B/s".into()),
            settable: false,
        },
    ];
    td.handle()
        .integrations()
        .set_summary_fixture(vec![fixture.clone()]);
    let choices = s.numeric_streams();
    assert_eq!(choices.len(), 2);
    assert_eq!(
        (
            &choices[0].integration,
            &choices[0].stream,
            &choices[0].name
        ),
        (
            &"hub".to_string(),
            &"level".to_string(),
            &"Five hour pct".to_string()
        )
    );
    assert_eq!(choices[0].unit.as_deref(), Some("%"));
    assert_eq!(choices[1].name, "Network send");
    assert_eq!(choices[1].unit.as_deref(), Some("B/s"));
    fixture.streams.retain(|s| s.kind != "numeric");
    td.handle()
        .integrations()
        .set_summary_fixture(vec![fixture]);
    assert!(s.numeric_streams().is_empty());
    td.handle().integrations().set_summary_fixture(vec![]);
    assert!(s.numeric_streams().is_empty());
}
#[tokio::test]
async fn bar_lookup_requires_matching_integration_stream_and_valid_renderer_declarations() {
    let (td, s) = session().await;
    let mut fixture = td.handle().integrations().summaries()[0].clone();
    fixture.id = "volume".into();
    fixture.streams = streams();
    let mut p = preset(Some(TileRender::Bar));
    p.input = true;
    fixture.presets = vec![p];
    td.handle()
        .integrations()
        .set_summary_fixture(vec![fixture.clone()]);
    let action = Action::Widget {
        source: "volume".into(),
        field: "level".into(),
        account: "".into(),
    };
    assert!(matches!(
        s.bar_appearance(&action),
        Some(Appearance::Live {
            render: TileRender::Bar,
            input: true,
            ..
        })
    ));
    for action in [
        Action::None,
        Action::Widget {
            source: "other".into(),
            field: "level".into(),
            account: "".into(),
        },
        Action::Widget {
            source: "volume".into(),
            field: "muted".into(),
            account: "".into(),
        },
    ] {
        assert!(s.bar_appearance(&action).is_none());
    }
    fixture.presets[0].streams = Some(BTreeMap::from([("value".into(), StreamType::Series)]));
    td.handle()
        .integrations()
        .set_summary_fixture(vec![fixture]);
    assert!(s.bar_appearance(&action).is_none());
}

#[tokio::test]
async fn preset_renderer_fallback_validates_streams_source_input_and_numeric_binding() {
    let (_td, s) = session().await;
    let mut p = preset(Some(TileRender::Graph));
    assert_eq!(
        s.preset_tile(&p, "other", &streams(), true).unwrap().render,
        None
    );
    let mut text_stream = streams();
    text_stream[0].kind = "text".into();
    assert_eq!(
        s.preset_tile(&p, "volume", &text_stream, true)
            .unwrap()
            .render,
        None
    );
    p.streams = Some(BTreeMap::from([("value".into(), StreamType::Series)]));
    assert_eq!(
        s.preset_tile(&p, "volume", &streams(), true)
            .unwrap()
            .streams
            .unwrap()["value"],
        StreamType::Number
    );
    p.render = Some(TileRender::Bar);
    for declared in [
        BTreeMap::from([("value".into(), StreamType::Series)]),
        BTreeMap::from([
            ("value".into(), StreamType::Number),
            ("level".into(), StreamType::Number),
        ]),
    ] {
        p.streams = Some(declared);
        assert_eq!(
            s.preset_tile(&p, "volume", &streams(), true)
                .unwrap()
                .render,
            None
        );
    }
    p.render = Some(TileRender::NowPlaying);
    p.streams = None;
    assert_eq!(
        s.preset_tile(&p, "volume", &streams(), true)
            .unwrap()
            .render,
        None
    );
    p.streams = Some(BTreeMap::from([
        ("title".into(), StreamType::Number),
        ("artist".into(), StreamType::Text),
        ("cover".into(), StreamType::Image),
        ("state".into(), StreamType::Number),
    ]));
    assert_eq!(
        s.preset_tile(&p, "volume", &streams(), true)
            .unwrap()
            .render,
        None
    );
    p.render = Some(TileRender::Graph);
    p.streams = None;
    p.input = true;
    assert_eq!(
        s.preset_tile(&p, "volume", &streams(), true)
            .unwrap()
            .render,
        None
    );
}

fn gif_bytes() -> Vec<u8> {
    let mut out = vec![];
    {
        let mut encoder = gif::Encoder::new(&mut out, 8, 4, &[0, 0, 0, 255, 0, 0]).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
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
fn gif_caps(td: &TestCompanion, device: &str) -> td_companion::devices::Capabilities {
    let id = td_companion::devices::DeviceId::parse(device).unwrap();
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
    td.deck(device).set_gif_capabilities(true);
    td.handle()
        .devices()
        .set_capability_fixture(&id, Some(caps.clone()));
    caps
}
async fn settle_gif(s: &mut Session, tile: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while s.gif_work(tile).is_some() {
            assert!(
                !matches!(
                    s.gif_work(tile),
                    Some(td_companion::editor::GifWork::Failed { .. })
                ),
                "{:?}",
                s.gif_work(tile)
            );
            s.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn gif_preparation_uses_tile_pixels_replaces_icons_and_preserves_action() {
    use td_companion::editor::GifEdit;
    let (td, mut s) = session().await;
    gif_caps(&td, "mock0001");
    s.start(blank()).unwrap();
    let id = s
        .add_gif(
            "cat.gif".into(),
            gif_bytes(),
            AddTarget::Cell { row: 0, col: 2 },
        )
        .unwrap();
    assert_eq!(s.draft().unwrap().effective_tile(&id).unwrap().label, "GIF");
    let target = s.gif_target(&id).unwrap();
    assert_eq!(
        (target.width, target.height, target.background),
        (312, 226, [24, 32, 49])
    );
    settle_gif(&mut s, &id).await;
    let tile = s.draft().unwrap().effective_tile(&id).unwrap();
    assert_eq!(tile.render, Some(TileRender::Gif));
    assert_eq!(tile.icon, None);
    assert!(tile.asset.unwrap().starts_with("sha256:"));
    assert_eq!(s.draft().unwrap().layout().schema, 2);
    let key = s.add(AddKind::Consumer, AddTarget::Auto).unwrap();
    let action = s.draft().unwrap().effective_tile(&key).unwrap().action;
    s.gif(
        &key,
        GifEdit::Choose {
            file: "cat.gif".into(),
            bytes: gif_bytes(),
        },
    )
    .unwrap();
    settle_gif(&mut s, &key).await;
    assert_eq!(
        s.draft().unwrap().effective_tile(&key).unwrap().action,
        action
    );
    assert_eq!(s.gif_target("missing"), None);
}
#[tokio::test]
async fn queued_gifs_cancel_replace_fail_retry_and_drop_deleted_tile_results() {
    use td_companion::editor::{GifEdit, GifWork};
    let (td, mut s) = session().await;
    gif_caps(&td, "mock0001");
    s.start(blank()).unwrap();
    let id = s.add(AddKind::Key, AddTarget::Auto).unwrap();
    let before = s.export().unwrap();
    td.hold_jobs();
    s.gif(
        &id,
        GifEdit::Choose {
            file: "first.gif".into(),
            bytes: gif_bytes(),
        },
    )
    .unwrap();
    assert!(matches!(s.gif_work(&id), Some(GifWork::Queued { .. })));
    s.gif(&id, GifEdit::Cancel).unwrap();
    assert!(s.gif_work(&id).is_none());
    td.release_jobs();
    assert_eq!(s.export().unwrap(), before);
    s.gif(
        &id,
        GifEdit::Choose {
            file: "bad.gif".into(),
            bytes: b"not a gif".to_vec(),
        },
    )
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !matches!(s.gif_work(&id), Some(GifWork::Failed { .. })) {
            s.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(s.export().unwrap(), before);
    s.gif(&id, GifEdit::Retry).unwrap();
    assert!(matches!(
        s.gif_work(&id),
        Some(GifWork::Queued { .. }) | Some(GifWork::Preparing { .. })
    ));
    s.gif(
        &id,
        GifEdit::Choose {
            file: "good.gif".into(),
            bytes: gif_bytes(),
        },
    )
    .unwrap();
    settle_gif(&mut s, &id).await;
    let good = s.export().unwrap();
    s.gif(
        &id,
        GifEdit::Choose {
            file: "bad.gif".into(),
            bytes: b"bad".to_vec(),
        },
    )
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !matches!(s.gif_work(&id), Some(GifWork::Failed { .. })) {
            s.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(s.export().unwrap(), good);
    td.hold_jobs();
    s.gif(
        &id,
        GifEdit::Choose {
            file: "late.gif".into(),
            bytes: gif_bytes(),
        },
    )
    .unwrap();
    s.edit(Edit::RemoveTile { tile: id.clone() }).unwrap();
    td.release_jobs();
    assert!(s.gif_work(&id).is_none());
    assert!(tiles(&s).is_empty());
}
#[tokio::test]
async fn gif_fit_geometry_limits_and_another_device_refit_from_retained_source() {
    use td_companion::{editor::GifEdit, features::assets::Fit};
    let (td, mut s) = session().await;
    let mut caps = gif_caps(&td, "mock0001");
    s.start(blank()).unwrap();
    let id = s
        .add_gif("cat.gif".into(), gif_bytes(), AddTarget::Auto)
        .unwrap();
    settle_gif(&mut s, &id).await;
    assert_eq!(s.gif_fit(&id), Fit::Contain);
    s.gif(&id, GifEdit::Fit(Fit::Cover)).unwrap();
    settle_gif(&mut s, &id).await;
    assert_eq!(s.gif_fit(&id), Fit::Cover);
    td.hold_jobs();
    s.gif(
        &id,
        GifEdit::Choose {
            file: "resizing.gif".into(),
            bytes: gif_bytes(),
        },
    )
    .unwrap();
    s.edit(Edit::Resize {
        tile: id.clone(),
        row_span: 1,
        col_span: 2,
    })
    .unwrap();
    assert_eq!(s.gif_target(&id).unwrap().width, 640);
    td.release_jobs();
    settle_gif(&mut s, &id).await;
    s.edit(Edit::FullScreen { on: true }).unwrap();
    assert_eq!(s.gif_target(&id).unwrap().width, 677);
    settle_gif(&mut s, &id).await;
    caps.anim.as_mut().unwrap().image = [96, 96];
    let device = td_companion::devices::DeviceId::parse("mock0001").unwrap();
    td.handle()
        .devices()
        .set_capability_fixture(&device, Some(caps));
    s.changed().await.unwrap();
    assert!(s.gif_work(&id).is_some());
    settle_gif(&mut s, &id).await;
    let exported = s.export().unwrap();
    td.plug_device("mock0002").await;
    gif_caps(&td, "mock0002");
    let mut other = td.handle().open_editor("mock0002").unwrap();
    other.import("gif.json", exported.as_bytes()).unwrap();
    assert!(other.gif_work(&id).is_some());
    settle_gif(&mut other, &id).await;
}
#[tokio::test]
async fn gif_capability_loss_upload_limit_and_old_asset_refit_have_recoverable_messages() {
    use td_companion::{
        editor::{GifEdit, GifWork},
        features::assets::Fit,
    };
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    let id = s.add(AddKind::None, AddTarget::Auto).unwrap();
    assert!(!s.offers_gif());
    s.gif(
        &id,
        GifEdit::Choose {
            file: "cat.gif".into(),
            bytes: gif_bytes(),
        },
    )
    .unwrap();
    assert!(matches!(
        s.gif_work(&id),
        Some(GifWork::Failed { retry: true, .. })
    ));
    let caps = gif_caps(&td, "mock0001");
    assert!(s.offers_gif());
    s.gif(&id, GifEdit::Retry).unwrap();
    settle_gif(&mut s, &id).await;
    s.gif(
        &id,
        GifEdit::Choose {
            file: "huge.gif".into(),
            bytes: vec![0; 16 * 1024 * 1024 + 1],
        },
    )
    .unwrap();
    assert!(
        matches!(s.gif_work(&id),Some(GifWork::Failed{problem,..}) if problem.detail.contains("request.too_large"))
    );
    td.hold_jobs();
    s.gif(
        &id,
        GifEdit::Choose {
            file: "cat.gif".into(),
            bytes: gif_bytes(),
        },
    )
    .unwrap();
    let device = td_companion::devices::DeviceId::parse("mock0001").unwrap();
    td.handle().devices().set_capability_fixture(&device, None);
    s.changed().await.unwrap();
    assert!(matches!(
        s.gif_work(&id),
        Some(GifWork::Failed { retry: true, .. })
    ));
    td.release_jobs();
    td.handle()
        .devices()
        .set_capability_fixture(&device, Some(caps));
    s.gif(&id, GifEdit::Retry).unwrap();
    settle_gif(&mut s, &id).await;
    s.gif(&id, GifEdit::Cancel).unwrap();
    s.edit(Edit::Renderer {
        tile: id.clone(),
        appearance: Appearance::Gif {
            asset: format!("sha256:{}", "a".repeat(64)),
        },
    })
    .unwrap();
    s.gif(&id, GifEdit::Fit(Fit::Cover)).unwrap();
    assert!(
        matches!(s.gif_work(&id),Some(GifWork::Failed{retry:false,problem,..}) if problem.text=="Choose this GIF again to refit it.")
    );
}
#[tokio::test]
async fn gif_messages_keep_raw_details_and_distinguish_send_from_preparation_errors() {
    let (_td, s) = session().await;
    for (code, details, text) in [
        (
            "asset.too_short",
            serde_json::json!({"limit":100}),
            "at least 100 ms",
        ),
        (
            "asset.store_full",
            serde_json::json!({}),
            "Asset store is full",
        ),
        (
            "asset.unsupported",
            serde_json::json!({}),
            "can't play this GIF",
        ),
        (
            "layout.animation_limit",
            serde_json::json!({}),
            "too many animations",
        ),
        (
            "asset.too_large",
            serde_json::json!({"field":"gif_frames"}),
            "too long",
        ),
        (
            "asset.too_large",
            serde_json::json!({"field":"decoded_rgba"}),
            "64 MiB",
        ),
        (
            "asset.too_large",
            serde_json::json!({"reason":"size","limit":null}),
            "too large",
        ),
        (
            "asset.refused",
            serde_json::json!({"reason":"full"}),
            "out of space",
        ),
        (
            "asset.refused",
            serde_json::json!({"reason":"memory"}),
            "too large",
        ),
    ] {
        let error = td_companion::api::ErrorBody {
            code: code.into(),
            message: "raw message".into(),
            details: Some(details),
        };
        for push in [false, true] {
            let problem = s.gif_problem(&error, push).unwrap();
            assert!(problem.text.contains(text));
            assert!(problem.detail.contains(code));
            assert!(problem.detail.contains("raw message"));
        }
    }
    let error = td_companion::api::ErrorBody {
        code: "device.timeout".into(),
        message: "raw".into(),
        details: None,
    };
    assert!(s.gif_problem(&error, true).is_none());
    assert!(s.gif_problem(&error, false).is_some());
}
#[tokio::test]
async fn gif_inspection_poster_bounds_and_fitted_line_match_reference() {
    let (_td, s) = session().await;
    let bytes = gif_bytes();
    let info = s.inspect_gif(&bytes).unwrap();
    assert_eq!(
        (info.width, info.height, info.frames, info.duration_ms),
        (8, 4, 2, 400)
    );
    let poster = s.inspect_gif(&info.poster).unwrap();
    assert_eq!(poster.frames, 1);
    assert_eq!(poster.duration_ms, 200);
    assert!(!info.poster.windows(11).any(|b| b == b"NETSCAPE2.0"));
    assert!(s.fitted_text(&info).contains("8 × 4, 5 fps"));
    assert!(!s.fitted_text(&poster).contains("fps"));
    for end in 0..bytes.len() {
        assert!(s.inspect_gif(&bytes[..end]).is_none());
    }
    assert!(s.inspect_gif(b"not a gif").is_none());
    let mut corrupt = bytes.clone();
    corrupt.insert(corrupt.len() - 1, 0x99);
    assert!(s.inspect_gif(&corrupt).is_none());
}

#[tokio::test]
async fn gif_capability_gate_checks_every_required_limit_and_geometry_follows_portrait() {
    let (td, mut s) = session().await;
    let valid = gif_caps(&td, "mock0001");
    s.start(blank()).unwrap();
    let id = s.add(AddKind::None, AddTarget::Auto).unwrap();
    let device = td_companion::devices::DeviceId::parse("mock0001").unwrap();
    let mut cases = vec![];
    let mut c = valid.clone();
    c.anim = None;
    cases.push(c);
    let mut c = valid.clone();
    c.formats.retain(|f| f != "gif");
    cases.push(c);
    let mut c = valid.clone();
    c.tile_kinds.retain(|f| f != "gif");
    cases.push(c);
    let mut c = valid.clone();
    c.gif_frames = Some(0);
    cases.push(c);
    let mut c = valid.clone();
    c.gif_pixels = Some(0);
    cases.push(c);
    let mut c = valid.clone();
    c.anim.as_mut().unwrap().image = [0, 128];
    cases.push(c);
    let mut c = valid.clone();
    c.anim.as_mut().unwrap().min_delay_ms = 0;
    cases.push(c);
    let mut c = valid.clone();
    c.anim.as_mut().unwrap().page_tiles = 0;
    cases.push(c);
    let mut c = valid.clone();
    c.anim.as_mut().unwrap().page_pixels = 0;
    cases.push(c);
    for caps in cases {
        td.handle()
            .devices()
            .set_capability_fixture(&device, Some(caps));
        assert!(!s.offers_gif());
    }
    td.handle()
        .devices()
        .set_capability_fixture(&device, Some(valid));
    assert!(s.offers_gif());
    let target = s.gif_target(&id).unwrap();
    assert_eq!((target.width, target.height), (312, 226));
    let mut tall = blank();
    tall.orientation = Orientation::Portrait;
    if let Page::Grid { rows, cols, .. } = &mut tall.pages[0] {
        *rows = 2;
        *cols = 3;
    }
    s.start(tall).unwrap();
    let id = s.add(AddKind::None, AddTarget::Auto).unwrap();
    let target = s.gif_target(&id).unwrap();
    assert_eq!((target.width, target.height), (170, 438));
}
#[tokio::test]
async fn gif_store_failure_and_animation_push_limit_use_real_companion_refusals() {
    use td_companion::editor::{GifEdit, GifWork};
    use td_core::editing::Capabilities;
    let td = TestCompanion::start_with_asset_quota(1).await;
    td.plug_device("mock0001").await;
    gif_caps(&td, "mock0001");
    let mut s = td.handle().open_editor("mock0001").unwrap();
    s.start(blank()).unwrap();
    let id = s
        .add_gif("cat.gif".into(), gif_bytes(), AddTarget::Auto)
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !matches!(s.gif_work(&id), Some(GifWork::Failed { .. })) {
            s.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(s.gif_work(&id),Some(GifWork::Failed{problem,retry:true,..}) if problem.detail.contains("asset.store_full"))
    );
    s.gif(&id, GifEdit::Cancel).unwrap();
    drop(s);
    drop(td);
    let (td, mut s) = session().await;
    let mut caps = gif_caps(&td, "mock0001");
    caps.anim.as_mut().unwrap().page_tiles = 1;
    td.handle().devices().set_capability_fixture(
        &td_companion::devices::DeviceId::parse("mock0001").unwrap(),
        Some(caps),
    );
    s.start(blank()).unwrap();
    let first = s
        .add_gif("one.gif".into(), gif_bytes(), AddTarget::Auto)
        .unwrap();
    settle_gif(&mut s, &first).await;
    let second = s
        .add_gif("two.gif".into(), gif_bytes(), AddTarget::Auto)
        .unwrap();
    settle_gif(&mut s, &second).await;
    let error = s
        .send(Capabilities::default())
        .await
        .unwrap_err()
        .refusal()
        .unwrap();
    assert_eq!(error.code, "layout.animation_limit");
    assert!(
        s.gif_problem(&error, true)
            .unwrap()
            .text
            .contains("too many animations")
    );
}

#[tokio::test]
async fn split_send_allows_edits_progress_orientation_changes_and_preserves_newer_unsent_values() {
    use td_companion::editor::{NewLayout, SaveState};
    use td_core::editing::Capabilities;
    let (td, mut s) = session().await;
    assert_eq!(s.save_state(), SaveState::None);
    s.new_layout(NewLayout::Starter).unwrap();
    assert_eq!(s.save_state(), SaveState::Unsent);
    td.hold_jobs();
    let pending = s.begin_send(Capabilities::default()).unwrap();
    assert_eq!(s.save_state(), SaveState::Sending);
    td.until(|td| !td.handle().snapshot().jobs.is_empty()).await;
    s.edit(Edit::Label {
        tile: "prev".into(),
        label: "Newer edit".into(),
    })
    .unwrap();
    assert!(s.begin_send(Capabilities::default()).is_err());
    td.release_jobs();
    let receipt = pending.wait().await;
    s.finish_send(receipt).unwrap();
    assert_eq!(s.save_state(), SaveState::Unsent);
    assert_eq!(
        s.draft().unwrap().effective_tile("prev").unwrap().label,
        "Newer edit"
    );
    let report = s.push_report().unwrap();
    assert_eq!(report.state.pages, 2);
    assert_eq!(report.state.tiles, 16);
    assert!(s.push_progress().is_some());
    s.send(Capabilities::default()).await.unwrap();
    assert_eq!(s.save_state(), SaveState::Saved);
    td.hold_jobs();
    let pending = s.begin_send(Capabilities::default()).unwrap();
    s.select_orientation(Orientation::Portrait);
    s.new_layout(NewLayout::Blank).unwrap();
    td.release_jobs();
    s.finish_send(pending.wait().await).unwrap();
    assert_eq!(s.orientation(), Orientation::Portrait);
    assert_eq!(s.save_state(), SaveState::Unsent);
    s.select_orientation(Orientation::Landscape);
    assert_eq!(s.save_state(), SaveState::Saved);
}
#[tokio::test]
async fn native_send_readback_mismatch_never_marks_the_draft_as_saved() {
    use td_companion::{checkpoints::Checkpoint, editor::SaveState};
    use td_core::editing::Capabilities;
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    s.send(Capabilities::default()).await.unwrap();
    td.checkpoints().hold(Checkpoint::BeforeEditorReadback);
    let pending = s.begin_send(Capabilities::default()).unwrap();
    td.until(|td| td.checkpoints().waiting(Checkpoint::BeforeEditorReadback))
        .await;
    let mut remote = blank();
    remote.name = "Different copy".into();
    td.write_layout_unseen(
        &td_companion::devices::DeviceId::parse("mock0001").unwrap(),
        &remote,
    )
    .await;
    td.checkpoints()
        .release(Checkpoint::BeforeEditorReadback, false);
    assert!(matches!(
        s.finish_send(pending.wait().await),
        Err(td_companion::editor::SessionError::Editor(
            td_companion::editor::EditorError::PushMismatch
        ))
    ));
    assert_eq!(s.save_state(), SaveState::Unsent);
    assert!(s.draft().unwrap().conflict().is_some());
    let reopened = td.handle().open_editor("mock0001").unwrap();
    assert_eq!(reopened.save_state(), SaveState::Unsent);
    assert!(reopened.draft().unwrap().conflict().is_some());
}
#[tokio::test]
async fn starter_geometry_duplicate_navigation_and_page_toolbar_rules_cross_session() {
    use td_companion::editor::NewLayout;
    let (_td, mut s) = session().await;
    s.new_layout(NewLayout::Starter).unwrap();
    assert_eq!(s.draft().unwrap().layout().name, "Starter");
    assert_eq!(s.selection(), (Some("media"), None));
    assert!(
        s.edit(Edit::Duplicate {
            tile: "next".into()
        })
        .is_err()
    );
    assert_eq!(tiles(&s).len(), 16);
    s.edit(Edit::GridSize {
        page: "media".into(),
        rows: 3,
        cols: 3,
    })
    .unwrap();
    s.edit(Edit::Move {
        tile: "prev".into(),
        row: 2,
        col: 2,
    })
    .unwrap();
    assert!(
        s.edit(Edit::Move {
            tile: "prev".into(),
            row: 0,
            col: 1
        })
        .is_err()
    );
    assert_eq!(s.draft().unwrap().effective_tile("prev").unwrap().row, 2);
    s.edit(Edit::Move {
        tile: "prev".into(),
        row: 0,
        col: 0,
    })
    .unwrap();
    for id in ["playpause", "mute", "voldown"] {
        s.edit(Edit::RemoveTile { tile: id.into() }).unwrap();
    }
    s.edit(Edit::Resize {
        tile: "prev".into(),
        row_span: 2,
        col_span: 2,
    })
    .unwrap();
    assert!(
        s.edit(Edit::Resize {
            tile: "prev".into(),
            row_span: 2,
            col_span: 4
        })
        .is_err()
    );
    let copy = s
        .edit(Edit::Duplicate {
            tile: "next".into(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(s.selection().1, Some(copy.as_str()));
    assert_eq!(
        s.draft().unwrap().effective_tile(&copy).unwrap().label,
        "Next"
    );
    let before = s.export().unwrap();
    assert!(
        s.edit(Edit::GridSize {
            page: "media".into(),
            rows: 1,
            cols: 1
        })
        .is_err()
    );
    assert_eq!(s.export().unwrap(), before);
    s.edit(Edit::AddPage {
        page: Page::Grid {
            id: "extra".into(),
            title: "Games".into(),
            rows: 3,
            cols: 4,
            tiles: vec![],
        },
    })
    .unwrap();
    assert_eq!(s.selection(), (Some("extra"), None));
    s.edit(Edit::PageTitle {
        page: "extra".into(),
        title: "Renamed".into(),
    })
    .unwrap();
    assert!(
        s.edit(Edit::PageTitle {
            page: "extra".into(),
            title: "".into()
        })
        .is_err()
    );
    s.select("media", Some("next"));
    s.edit(Edit::Action {
        tile: "next".into(),
        action: Action::Page {
            page: "extra".into(),
        },
    })
    .unwrap();
    s.edit(Edit::RemovePage {
        page: "extra".into(),
    })
    .unwrap();
    assert_eq!(
        s.draft().unwrap().effective_tile("next").unwrap().action,
        Action::None
    );
    assert_eq!(s.selection(), (Some("media"), None));
    s.new_layout(NewLayout::Blank).unwrap();
    assert!(
        s.edit(Edit::RemovePage {
            page: "page1".into()
        })
        .is_err()
    );
}
#[tokio::test]
async fn rejected_actions_labels_fields_duplicate_clear_new_layout_and_graph_bindings_match_reference()
 {
    use td_companion::editor::NewLayout;
    use td_core::editing::{Capabilities, EditError};
    let (_td, mut s) = session().await;
    s.new_layout(NewLayout::Starter).unwrap();
    s.edit(Edit::GridSize {
        page: "media".into(),
        rows: 3,
        cols: 3,
    })
    .unwrap();
    let command = Action::Command {
        program: "".into(),
        args: vec![],
        cwd: "".into(),
        shell: false,
    };
    assert!(
        s.edit(Edit::Action {
            tile: "next".into(),
            action: command.clone()
        })
        .is_err()
    );
    assert_eq!(
        s.draft().unwrap().effective_tile("next").unwrap().action,
        command
    );
    let mut refused = s.draft().unwrap().rejected()[0].clone();
    assert_eq!(refused.action_field(), Some("program"));
    if let EditError::Invalid(problems) = &mut refused.reason {
        problems[0].message = "Reworded".into();
    }
    assert_eq!(refused.action_field(), Some("program"));
    if let EditError::Invalid(problems) = &mut refused.reason {
        problems[0].path = "/pages/1/tiles/0/action/args/2".into();
    }
    assert_eq!(refused.action_field(), Some("args"));
    if let EditError::Invalid(problems) = &mut refused.reason {
        problems[0].path = "".into();
    }
    assert_eq!(refused.action_field(), None);
    assert!(
        s.edit(Edit::Action {
            tile: "next".into(),
            action: Action::Open { target: "".into() }
        })
        .is_err()
    );
    assert!(s.send(Capabilities::default()).await.is_err());
    s.edit(Edit::Action {
        tile: "next".into(),
        action: Action::Open {
            target: "https://example.com".into(),
        },
    })
    .unwrap();
    assert!(s.draft().unwrap().rejected().is_empty());
    s.send(Capabilities::default()).await.unwrap();
    assert!(s.draft().unwrap().is_sent());
    assert!(
        s.edit(Edit::Label {
            tile: "next".into(),
            label: "   ".into()
        })
        .is_err()
    );
    assert_eq!(
        s.draft().unwrap().effective_tile("next").unwrap().label,
        "   "
    );
    s.edit(Edit::Label {
        tile: "next".into(),
        label: "Skip".into(),
    })
    .unwrap();
    assert!(s.draft().unwrap().rejected().is_empty());
    assert!(
        s.edit(Edit::Action {
            tile: "next".into(),
            action: Action::Launch {
                path: "".into(),
                args: vec![],
                cwd: "".into()
            }
        })
        .is_err()
    );
    let copy = s
        .edit(Edit::Duplicate {
            tile: "next".into(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(s.draft().unwrap().rejected().len(), 2);
    for id in ["next", copy.as_str()] {
        s.edit(Edit::RemoveTile { tile: id.into() }).unwrap();
    }
    assert!(s.draft().unwrap().rejected().is_empty());
    assert!(
        s.edit(Edit::Label {
            tile: "prev".into(),
            label: "".into()
        })
        .is_err()
    );
    s.new_layout(NewLayout::Starter).unwrap();
    assert!(s.draft().unwrap().rejected().is_empty());
    for stream in ["ram_pct", "cpu_pct"] {
        s.edit(Edit::Renderer {
            tile: "prev".into(),
            appearance: Appearance::Live {
                render: TileRender::Graph,
                streams: BTreeMap::from([("value".into(), StreamType::Number)]),
                input: false,
                action: Action::Widget {
                    source: "this_pc".into(),
                    field: stream.into(),
                    account: "".into(),
                },
            },
        })
        .unwrap();
        assert!(
            matches!(s.draft().unwrap().effective_tile("prev").unwrap().action,Action::Widget{field,..} if field==stream)
        );
    }
    s.edit(Edit::Renderer {
        tile: "prev".into(),
        appearance: Appearance::Plain {
            icon: None,
            asset: None,
            action: Some(Action::Widget {
                source: "this_pc".into(),
                field: "cpu_pct".into(),
                account: "".into(),
            }),
        },
    })
    .unwrap();
    assert_eq!(
        s.draft().unwrap().effective_tile("prev").unwrap().render,
        None
    );
    assert!(
        s.draft()
            .unwrap()
            .effective_tile("prev")
            .unwrap()
            .streams
            .is_none()
    );
}

#[tokio::test]
async fn parameterized_gif_refusals_and_bounded_inspection_cases_match_typescript() {
    let (_td, s) = session().await;
    for field in [
        "gif_frames",
        "source_frames",
        "decoded_rgba",
        "asset_bytes",
        "gif_pixels",
        "source_bytes",
        "source_width",
        "device_limit",
    ] {
        let error = td_companion::api::ErrorBody {
            code: "asset.too_large".into(),
            message: "raw message".into(),
            details: Some(serde_json::json!({"field":field,"limit":1,"actual":2,"unit":"frames"})),
        };
        let problem = s.gif_problem(&error, false).unwrap();
        assert!(
            problem
                .text
                .contains(if ["gif_frames", "source_frames"].contains(&field) {
                    "too long"
                } else if field == "decoded_rgba" {
                    "64 MiB"
                } else {
                    "too large"
                })
        );
        assert!(problem.detail.contains(field));
    }
    for reason in [
        "not_gif",
        "corrupt",
        "timed_out",
        "invalid_limits",
        "invalid",
    ] {
        let error = td_companion::api::ErrorBody {
            code: "asset.render_failed".into(),
            message: "raw".into(),
            details: Some(serde_json::json!({"reason":reason})),
        };
        assert_eq!(
            s.gif_problem(&error, false).unwrap().text,
            "Couldn't prepare this GIF. Choose another GIF."
        );
    }
    for (code, part) in [
        ("request.too_large", "too large"),
        ("request.media_type", "Choose another GIF"),
        ("something.new", "Choose another GIF"),
        ("limits.exceeded", "Other Assets"),
        ("client.unreachable", "running in the tray"),
        ("device.not_connected", "plugged in"),
    ] {
        let error = td_companion::api::ErrorBody {
            code: code.into(),
            message: "raw".into(),
            details: None,
        };
        assert!(s.gif_problem(&error, false).unwrap().text.contains(part));
    }
    let bytes = gif_bytes();
    let read = s.inspect_gif(&bytes).unwrap();
    let head = 13 + 3 * (1usize << ((read.poster[10] & 7) + 1));
    let block = &read.poster[head..read.poster.len() - 1];
    let mut many = read.poster[..head].to_vec();
    for _ in 0..5000 {
        many.extend_from_slice(block);
    }
    many.push(0x3b);
    assert_eq!(s.inspect_gif(&many).unwrap().frames, 5000);
    let mut no_frame = read.poster[..head].to_vec();
    no_frame.push(0x3b);
    assert!(s.inspect_gif(&no_frame).is_none());
    let mut overrun = read.poster[..head].to_vec();
    overrun.extend_from_slice(&[0x21, 0xfe, 0xff, 0x41]);
    assert!(s.inspect_gif(&overrun).is_none());
    let mut info = read.clone();
    info.width = 64;
    info.height = 64;
    info.frames = 3;
    info.duration_ms = 900;
    assert!(s.fitted_text(&info).ends_with("64 × 64, 3.3 fps."));
    let error = td_companion::editor::SessionError::Job(td_companion::api::ErrorBody {
        code: "asset.refused".into(),
        message: "raw".into(),
        details: Some(serde_json::json!({"reason":"storage"})),
    });
    assert!(
        s.send_problem(&error)
            .text
            .contains("Check that it's plugged in")
    );
    let invalid = td_companion::editor::SessionError::Job(td_companion::api::ErrorBody {
        code: "layout.invalid".into(),
        message: "raw".into(),
        details: Some(
            serde_json::json!({"issues":[{"rule":"label.too-long","path":"/pages/0/tiles/0/label","message":"too long"}]}),
        ),
    });
    let problem = s.send_problem(&invalid);
    assert_eq!(problem.text, "Tile labels can be up to 16 characters.");
    assert!(problem.detail.contains("label.too-long"));
}

#[tokio::test]
async fn gif_queue_prepares_all_tiles_serially_and_cancelling_one_keeps_the_others() {
    use td_companion::editor::{GifEdit, GifWork};
    let (td, mut s) = session().await;
    gif_caps(&td, "mock0001");
    s.start(blank()).unwrap();
    td.hold_jobs();
    let a = s
        .add_gif("a.gif".into(), gif_bytes(), AddTarget::Auto)
        .unwrap();
    let b = s
        .add_gif("b.gif".into(), gif_bytes(), AddTarget::Auto)
        .unwrap();
    let c = s
        .add_gif("c.gif".into(), gif_bytes(), AddTarget::Auto)
        .unwrap();
    assert!(matches!(s.gif_work(&b), Some(GifWork::Queued { .. })));
    assert!(matches!(s.gif_work(&c), Some(GifWork::Queued { .. })));
    s.gif(&b, GifEdit::Cancel).unwrap();
    td.release_jobs();
    settle_gif(&mut s, &a).await;
    settle_gif(&mut s, &c).await;
    assert_eq!(s.draft().unwrap().effective_tile(&b).unwrap().render, None);
    assert_eq!(
        s.draft().unwrap().effective_tile(&c).unwrap().render,
        Some(TileRender::Gif)
    );
}

#[tokio::test]
async fn loading_and_replacing_reset_selection_while_keep_mine_preserves_it() {
    use td_companion::editor::{Decision, NewLayout};
    use td_core::editing::Capabilities;
    let (_td, mut s) = session().await;
    s.new_layout(NewLayout::Starter).unwrap();
    s.send(Capabilities::default()).await.unwrap();
    s.select("shortcuts", Some("copy"));
    s.load().await.unwrap();
    assert_eq!(s.selection(), (Some("media"), None));
    s.select("shortcuts", Some("copy"));
    s.edit(Edit::Name {
        name: "Mine".into(),
    })
    .unwrap();
    s.load().await.unwrap();
    assert_eq!(s.selection(), (Some("shortcuts"), Some("copy")));
    s.resolve(Decision::KeepMine).unwrap();
    assert_eq!(s.selection(), (Some("shortcuts"), Some("copy")));
    s.load().await.unwrap();
    s.resolve(Decision::UseDevice).unwrap();
    assert_eq!(s.selection(), (Some("media"), None));
    assert_eq!(s.draft().unwrap().layout().name, "Starter");
}

#[tokio::test]
async fn lost_readback_keeps_reverted_draft_unconfirmed_and_preserves_conditional_revision() {
    use td_companion::{
        checkpoints::Checkpoint,
        editor::{Decision, SaveState},
    };
    use td_core::editing::Capabilities;
    let (td, mut s) = session().await;
    s.start(blank()).unwrap();
    s.send(Capabilities::default()).await.unwrap();
    let original = s.export().unwrap();
    s.edit(Edit::Name { name: "B".into() }).unwrap();
    td.checkpoints().hold(Checkpoint::BeforeEditorReadback);
    let pending = s.begin_send(Capabilities::default()).unwrap();
    td.until(|td| td.checkpoints().waiting(Checkpoint::BeforeEditorReadback))
        .await;
    assert_eq!(
        td_core::Layout::parse(&td.deck("mock0001").layout(Orientation::Landscape).unwrap())
            .unwrap()
            .name,
        "B"
    );
    s.import("original.json", original.as_bytes()).unwrap();
    td.checkpoints()
        .release(Checkpoint::BeforeEditorReadback, true);
    let error = s.finish_send(pending.wait().await).unwrap_err();
    assert_eq!(error.refusal().unwrap().code, "device.timeout");
    assert_eq!(s.save_state(), SaveState::Unconfirmed);
    assert!(!s.draft().unwrap().is_sent());
    let reopened = td.handle().open_editor("mock0001").unwrap();
    assert_eq!(reopened.save_state(), SaveState::Unconfirmed);
    // The old conditional revision is retained: retry cannot overwrite B blindly.
    assert_eq!(
        s.send(Capabilities::default())
            .await
            .unwrap_err()
            .refusal()
            .unwrap()
            .code,
        "layout.changed"
    );
    s.load().await.unwrap();
    s.resolve(Decision::UseDevice).unwrap();
    assert_eq!(s.draft().unwrap().layout().name, "B");
    assert_eq!(s.save_state(), SaveState::Saved);
}

#[tokio::test]
async fn stale_send_receipts_cannot_mutate_a_newer_load_on_any_outcome() {
    use td_companion::{
        checkpoints::Checkpoint,
        editor::{EditorError, SaveState, SessionError},
    };
    use td_core::editing::Capabilities;
    for outcome in [
        "mismatch", "match", "missing", "invalid", "timeout", "conflict",
    ] {
        let (td, mut s) = session().await;
        let mut b = blank();
        b.name = "B".into();
        s.start(b.clone()).unwrap();
        s.send(Capabilities::default()).await.unwrap();
        let mut c = b.clone();
        c.name = "Obsolete C".into();
        if outcome == "conflict" {
            td.deck("mock0001")
                .seed_layout(Orientation::Landscape, Some(c.to_canonical_json()));
        } else {
            td.checkpoints().hold(Checkpoint::BeforeEditorReadback);
        }
        let pending = s.begin_send(Capabilities::default()).unwrap();
        if outcome != "conflict" {
            td.until(|td| td.checkpoints().waiting(Checkpoint::BeforeEditorReadback))
                .await;
            let bytes = match outcome {
                "mismatch" => Some(c.to_canonical_json()),
                "missing" => None,
                "invalid" => Some(b"{}".to_vec()),
                _ => Some(b.to_canonical_json()),
            };
            td.deck("mock0001")
                .seed_layout(Orientation::Landscape, bytes);
            td.checkpoints()
                .release(Checkpoint::BeforeEditorReadback, outcome == "timeout");
        }
        let receipt = pending.wait().await;
        // Same values and same revision as the original baseline, but a NEW observation.
        td.deck("mock0001")
            .seed_layout(Orientation::Landscape, Some(b.to_canonical_json()));
        s.load().await.unwrap();
        assert!(
            matches!(
                s.finish_send(receipt),
                Err(SessionError::Editor(EditorError::StalePush))
            ),
            "{outcome}"
        );
        assert_eq!(s.save_state(), SaveState::Saved, "{outcome}");
        assert!(s.draft().unwrap().conflict().is_none(), "{outcome}");
        assert_eq!(s.draft().unwrap().layout(), &b);
        let reopened = td.handle().open_editor("mock0001").unwrap();
        assert_eq!(reopened.save_state(), SaveState::Saved, "{outcome}");
        assert!(reopened.draft().unwrap().conflict().is_none());
    }
}

#[tokio::test]
async fn replacing_a_layout_cancels_old_gif_work_even_when_tile_identity_matches() {
    // TypeScript 1.x keyed GIF runs by Device, orientation and Tile id only, so a
    // replacement Layout with the same Tile id inherited the old result. Fixed here.
    use td_companion::editor::{Decision, GifEdit};
    for replacement in ["import", "start", "replace", "load"] {
        let (td, mut s) = session().await;
        gif_caps(&td, "mock0001");
        s.start(blank()).unwrap();
        let id = s.add(AddKind::Key, AddTarget::Auto).unwrap();
        let original = s.draft().unwrap().layout().clone();
        let mut imported = original.clone();
        if replacement != "load" {
            imported.name = "Imported".into();
        }
        if replacement == "replace" || replacement == "load" {
            td.deck("mock0001")
                .seed_layout(Orientation::Landscape, Some(imported.to_canonical_json()));
        }
        if replacement == "replace" {
            s.load().await.unwrap();
            assert!(s.draft().unwrap().conflict().is_some());
        }
        td.hold_jobs();
        s.gif(
            &id,
            GifEdit::Choose {
                file: "old.gif".into(),
                bytes: gif_bytes(),
            },
        )
        .unwrap();
        let job = td
            .handle()
            .snapshot()
            .jobs
            .into_iter()
            .find(|j| j.kind == "assets.render")
            .unwrap();
        // Rejected imports preserve both the Layout and the work that belongs to it.
        assert!(s.import("bad.json", b"{}").is_err());
        assert!(s.gif_work(&id).is_some());
        match replacement {
            "import" => s.import("new.json", &imported.to_canonical_json()).unwrap(),
            "start" => s.start(imported.clone()).unwrap(),
            "replace" => s.resolve(Decision::UseDevice).unwrap(),
            "load" => s.load().await.unwrap(),
            _ => unreachable!(),
        }
        td.release_jobs();
        let finished = td.wait(&job).await;
        // Process queued events after completion: the old result must never attach.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let td_companion::companion::Change::JobUpdated(update) =
                    s.changed().await.unwrap()
                    && update.0.id == finished.id
                    && update.0.state.finished()
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(s.gif_work(&id).is_none(), "{replacement}");
        let reopened = td.handle().open_editor("mock0001").unwrap();
        if replacement == "load" {
            // A Load that confirms the same Layout replaces nothing: the GIF still lands.
            let tile = s.draft().unwrap().effective_tile(&id).unwrap();
            assert_eq!(tile.render, Some(td_core::layout::TileRender::Gif));
            assert_eq!(
                reopened.draft().unwrap().layout(),
                s.draft().unwrap().layout()
            );
        } else {
            assert_eq!(s.draft().unwrap().layout(), &imported, "{replacement}");
            assert_eq!(
                reopened.draft().unwrap().layout(),
                &imported,
                "{replacement}"
            );
        }
    }
}
