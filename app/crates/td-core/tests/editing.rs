use std::collections::BTreeMap;
use td_core::{
    Layout,
    editing::{Appearance, Capabilities, Compatibility, Document, Edit, EditError},
    layout::{Action, ActionSink, Page, StreamType, Tile, TileRender},
};

fn document() -> Document {
    Document::parse(br#"{"schema":1,"name":"Editing","orientation":"landscape","pages":[{"kind":"grid","id":"main","title":"Main","rows":2,"cols":4,"tiles":[]}]}"#).unwrap()
}
fn tile(id: &str, row: u8, col: u8) -> Tile {
    Tile {
        id: id.into(),
        label: "Tile".into(),
        row,
        col,
        row_span: 1,
        col_span: 1,
        icon: None,
        asset: None,
        render: None,
        streams: None,
        input: false,
        action: Action::None,
    }
}
fn place(doc: &Document, tile: Tile) -> Document {
    doc.apply(&Edit::Place {
        page: "main".into(),
        tile,
    })
    .unwrap()
    .document
}
fn tiles(doc: &Document) -> &[Tile] {
    let Page::Grid { tiles, .. } = &doc.layout().pages[0] else {
        panic!()
    };
    tiles
}
fn apply(doc: &Document, edit: Edit) -> Document {
    doc.apply(&edit).unwrap().document
}
fn has_rule(result: Result<td_core::editing::Change, EditError>, rule: &str) {
    assert!(
        matches!(result, Err(EditError::Invalid(errors)) if errors.iter().any(|error| error.rule == rule))
    );
}

#[test]
fn admission_and_geometry_refusals_are_atomic_even_for_hostile_typed_edits() {
    let doc = place(&document(), tile("t", 0, 0));
    let original = doc.clone();
    has_rule(
        doc.apply(&Edit::Place {
            page: "main".into(),
            tile: tile("overlap", 0, 0),
        }),
        "tile.overlap",
    );
    has_rule(
        doc.apply(&Edit::Move {
            tile: "t".into(),
            row: 1,
            col: 4,
        }),
        "tile.bounds",
    );
    has_rule(
        doc.apply(&Edit::Move {
            tile: "t".into(),
            row: 255,
            col: 255,
        }),
        "tile.position",
    );
    has_rule(
        doc.apply(&Edit::Resize {
            tile: "t".into(),
            row_span: 255,
            col_span: 255,
        }),
        "tile.span",
    );
    has_rule(
        doc.apply(&Edit::Place {
            page: "main".into(),
            tile: tile("main", 0, 1),
        }),
        "id.duplicate",
    );
    assert_eq!(doc, original);
    let moved = apply(
        &doc,
        Edit::Move {
            tile: "t".into(),
            row: 1,
            col: 2,
        },
    );
    let resized = apply(
        &moved,
        Edit::Resize {
            tile: "t".into(),
            row_span: 1,
            col_span: 2,
        },
    );
    assert_eq!(
        (tiles(&resized)[0].row, tiles(&resized)[0].col_span),
        (1, 2)
    );
    let mut malformed = doc.layout().clone();
    malformed.pages.clear();
    assert!(Document::admit(malformed).is_err());
    assert_eq!(
        Layout::parse(&resized.layout().to_canonical_json()).unwrap(),
        *resized.layout()
    );
}
#[test]
fn duplication_searches_after_source_then_wraps_and_respects_spans_and_global_ids() {
    let mut source = tile("t", 1, 2);
    source.col_span = 2;
    let doc = place(&document(), source);
    let doc = apply(
        &doc,
        Edit::AddPage {
            page: Page::Dashboard {
                id: "tile1".into(),
                title: "Other".into(),
                source: "source".into(),
            },
        },
    );
    let doc = place(&doc, tile("block", 0, 0));
    let duplicated = doc.apply(&Edit::Duplicate { tile: "t".into() }).unwrap();
    assert_eq!(duplicated.created_tile.as_deref(), Some("tile2"));
    let copy = tiles(&duplicated.document).last().unwrap();
    assert_eq!((copy.row, copy.col, copy.col_span), (0, 1, 2));
    let mut source = tile("wide", 0, 0);
    source.row_span = 2;
    source.col_span = 4;
    let full = place(&document(), source);
    assert_eq!(
        full.apply(&Edit::Duplicate {
            tile: "wide".into()
        }),
        Err(EditError::NoSpace)
    );
}
#[test]
fn page_removal_repairs_navigation_and_cannot_remove_the_last_page() {
    let doc = apply(
        &document(),
        Edit::AddPage {
            page: Page::Grid {
                id: "other".into(),
                title: "Other".into(),
                rows: 1,
                cols: 1,
                tiles: vec![],
            },
        },
    );
    let mut nav = tile("nav", 0, 0);
    nav.action = Action::Page {
        page: "other".into(),
    };
    let doc = place(&doc, nav);
    let doc = apply(
        &doc,
        Edit::RemovePage {
            page: "other".into(),
        },
    );
    assert_eq!(tiles(&doc)[0].action, Action::None);
    has_rule(
        doc.apply(&Edit::RemovePage {
            page: "main".into(),
        }),
        "layout.page-count",
    );
    let doc = apply(&doc, Edit::RemoveTile { tile: "nav".into() });
    assert!(tiles(&doc).is_empty());
}
#[test]
fn renderer_transitions_drop_incompatible_fields_and_choose_the_schema() {
    let doc = place(&document(), tile("t", 0, 0));
    let action = Action::Widget {
        source: "this-pc".into(),
        field: "volume".into(),
        account: "".into(),
    };
    let bar = apply(
        &doc,
        Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Live {
                render: TileRender::Bar,
                streams: BTreeMap::from([("value".into(), StreamType::Number)]),
                input: true,
                action: action.clone(),
            },
        },
    );
    assert!(tiles(&bar)[0].input);
    let asset = format!("sha256:{}", "a".repeat(64));
    let gif = apply(
        &bar,
        Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Gif {
                asset: asset.clone(),
            },
        },
    );
    let t = &tiles(&gif)[0];
    assert!(!t.input);
    assert!(t.streams.is_none());
    assert_eq!(t.asset.as_ref(), Some(&asset));
    assert_eq!(gif.layout().schema, 2);
    let plain = apply(
        &gif,
        Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Plain {
                icon: None,
                asset: None,
            },
        },
    );
    assert_eq!(plain.layout().schema, 1);
    let t = &tiles(&plain)[0];
    assert!(t.render.is_none() && t.asset.is_none() && t.streams.is_none() && !t.input);
    has_rule(
        bar.apply(&Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Live {
                render: TileRender::Graph,
                streams: BTreeMap::new(),
                input: false,
                action,
            },
        }),
        "tile.streams",
    );
}
#[test]
fn action_and_label_validation_and_asset_replacement_cross_the_document_interface() {
    let doc = place(&document(), tile("t", 0, 0));
    has_rule(
        doc.apply(&Edit::Label {
            tile: "t".into(),
            label: "".into(),
        }),
        "label.blank",
    );
    has_rule(
        doc.apply(&Edit::Action {
            tile: "t".into(),
            action: Action::Page {
                page: "missing".into(),
            },
        }),
        "action.page-missing",
    );
    assert_eq!(
        doc.apply(&Edit::Sink {
            tile: "t".into(),
            sink: ActionSink::Bluetooth
        }),
        Err(EditError::SinkNotApplicable)
    );
    let old = format!("sha256:{}", "a".repeat(64));
    let new = format!("sha256:{}", "b".repeat(64));
    let doc = apply(
        &doc,
        Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Plain {
                icon: None,
                asset: Some(old.clone()),
            },
        },
    );
    let replaced = apply(
        &doc,
        Edit::ReplaceAssets {
            replacements: BTreeMap::from([(old, new.clone())]),
        },
    );
    assert_eq!(tiles(&replaced)[0].asset.as_deref(), Some(new.as_str()));
    has_rule(
        replaced.apply(&Edit::ReplaceAssets {
            replacements: BTreeMap::from([(new, "bad".into())]),
        }),
        "icon.format",
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
fn compatibility_matches_shared_unknown_offline_and_older_firmware_cases() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../../../../schema/editor-compatibility.json")).unwrap();
    for case in cases {
        let doc = place(&document(), tile("t", 0, 0));
        let doc = apply(
            &doc,
            Edit::Action {
                tile: "t".into(),
                action: case.action,
            },
        );
        let got = match doc.compatibility(Capabilities {
            sinks: case.sinks.as_deref(),
        }) {
            Compatibility::Supported => "supported",
            Compatibility::Unknown => "unknown",
            Compatibility::Unsupported(_) => "unsupported",
        };
        assert_eq!(got, case.compatibility, "{}", case.name);
        let doc = apply(
            &doc,
            Edit::Sink {
                tile: "t".into(),
                sink: ActionSink::Link,
            },
        );
        assert!(
            !String::from_utf8(doc.layout().to_canonical_json())
                .unwrap()
                .contains("\"sink\"")
        );
        assert_eq!(
            doc.compatibility(Capabilities { sinks: Some(&[]) }),
            Compatibility::Supported
        );
    }
}

#[test]
fn all_live_renderers_replace_the_previous_bindings_and_input_policy() {
    let doc = place(&document(), tile("t", 0, 0));
    let action = Action::Widget {
        source: "this-pc".into(),
        field: "volume".into(),
        account: "".into(),
    };
    let bar = apply(
        &doc,
        Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Live {
                render: TileRender::Bar,
                streams: BTreeMap::from([("value".into(), StreamType::Number)]),
                input: true,
                action: action.clone(),
            },
        },
    );
    let graph = apply(
        &bar,
        Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Live {
                render: TileRender::Graph,
                streams: BTreeMap::from([("value".into(), StreamType::Series)]),
                input: false,
                action: action.clone(),
            },
        },
    );
    assert!(!tiles(&graph)[0].input);
    let streams = BTreeMap::from([
        ("title".into(), StreamType::Text),
        ("artist".into(), StreamType::Text),
        ("cover".into(), StreamType::Image),
        ("state".into(), StreamType::Number),
    ]);
    let playing = apply(
        &graph,
        Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Live {
                render: TileRender::NowPlaying,
                streams: streams.clone(),
                input: false,
                action,
            },
        },
    );
    assert_eq!(tiles(&playing)[0].streams.as_ref(), Some(&streams));
    assert!(
        !tiles(&playing)[0]
            .streams
            .as_ref()
            .unwrap()
            .contains_key("value")
    );
    let plain = apply(
        &playing,
        Edit::Renderer {
            tile: "t".into(),
            appearance: Appearance::Plain {
                icon: Some("lv:play_pause".into()),
                asset: None,
            },
        },
    );
    assert!(tiles(&plain)[0].render.is_none() && tiles(&plain)[0].streams.is_none());
    assert_eq!(tiles(&plain)[0].icon.as_deref(), Some("lv:play_pause"));
}

#[test]
fn placing_and_adding_pages_omit_an_explicit_companion_sink() {
    let mut t = tile("t", 0, 0);
    t.action = Action::Key {
        sink: Some(ActionSink::Link),
        usage: 4,
        mods: vec![],
    };
    let placed = place(&document(), t.clone());
    assert!(
        !String::from_utf8(placed.layout().to_canonical_json())
            .unwrap()
            .contains("\"sink\"")
    );
    let added = apply(
        &document(),
        Edit::AddPage {
            page: Page::Grid {
                id: "other".into(),
                title: "Other".into(),
                rows: 1,
                cols: 1,
                tiles: vec![t],
            },
        },
    );
    assert!(
        !String::from_utf8(added.layout().to_canonical_json())
            .unwrap()
            .contains("\"sink\"")
    );
    assert_eq!(
        added.compatibility(Capabilities { sinks: Some(&[]) }),
        Compatibility::Supported
    );
}

#[test]
fn an_admitted_layout_drops_an_explicit_companion_sink() {
    let text =
        std::str::from_utf8(include_bytes!("../../../../schema/examples/bluetooth.json")).unwrap();
    let explicit = text.replacen("\"sink\": \"bluetooth\"", "\"sink\": \"link\"", 1);
    assert!(
        Layout::parse(explicit.as_bytes()).is_ok(),
        "the parser accepts an explicit link sink"
    );
    let doc = Document::parse(explicit.as_bytes()).unwrap();
    let sinks: Vec<Option<ActionSink>> = doc
        .layout()
        .pages
        .iter()
        .flat_map(|page| match page {
            Page::Grid { tiles, .. } => tiles.as_slice(),
            _ => &[],
        })
        .filter_map(|tile| match &tile.action {
            Action::Key { sink, .. } | Action::Consumer { sink, .. } => Some(sink.clone()),
            _ => None,
        })
        .collect();
    assert!(!sinks.contains(&Some(ActionSink::Link)), "{sinks:?}");
    assert!(
        sinks.contains(&Some(ActionSink::Bluetooth)),
        "the other sink is kept: {sinks:?}"
    );
    let wire = String::from_utf8(doc.layout().to_canonical_json()).unwrap();
    assert!(!wire.contains("\"sink\":\"link\""), "{wire}");
}
