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
