use serde::Deserialize;
use td_core::Layout;
use td_core::live::{PixelRect, ScreenSize, art_size, cell_rect, tile_rect};

#[derive(Deserialize)]
struct Size {
    w: u16,
    h: u16,
}

#[derive(Deserialize)]
struct Grid {
    rows: u8,
    cols: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Span {
    row: u8,
    col: u8,
    row_span: u8,
    col_span: u8,
}

#[derive(Deserialize)]
struct Rect {
    x: i32,
    y: i32,
    w: u16,
    h: u16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    screen: Size,
    full_screen: bool,
    grid: Grid,
    tile: Span,
    rect: Rect,
}

#[derive(Deserialize)]
struct Vectors {
    cases: Vec<Case>,
}

#[test]
fn cell_rects_match_the_firmware_vectors() {
    let vectors: Vectors = serde_json::from_str(include_str!(
        "../../../../schema/live/tile-geometry-vectors.json"
    ))
    .unwrap();
    for c in vectors.cases {
        let got = cell_rect(
            ScreenSize {
                w: c.screen.w,
                h: c.screen.h,
            },
            c.full_screen,
            [c.grid.rows, c.grid.cols],
            [c.tile.row, c.tile.col, c.tile.row_span, c.tile.col_span],
        );
        let want = (c.rect.w > 0).then_some(PixelRect {
            x: c.rect.x,
            y: c.rect.y,
            w: c.rect.w,
            h: c.rect.h,
        });
        assert_eq!(got, want, "{}", c.name);
    }
}

#[test]
fn hostile_grids_and_screens_have_no_rect() {
    let screen = ScreenSize { w: 1024, h: 600 };
    assert_eq!(cell_rect(screen, false, [1, 1], [0, 0, 255, 255]), None);
    assert_eq!(cell_rect(screen, false, [255, 255], [0, 0, 1, 1]), None);
    assert_eq!(cell_rect(screen, false, [0, 1], [0, 0, 1, 1]), None);
    assert_eq!(cell_rect(screen, false, [2, 2], [1, 1, 0, 1]), None);
    assert_eq!(
        cell_rect(ScreenSize { w: 0, h: 600 }, false, [1, 1], [0, 0, 1, 1]),
        None
    );
    assert!(
        cell_rect(
            ScreenSize { w: u16::MAX, h: 3 },
            false,
            [8, 8],
            [7, 7, 1, 1]
        )
        .is_some()
    );
    assert!(
        cell_rect(
            ScreenSize {
                w: u16::MAX,
                h: u16::MAX
            },
            true,
            [8, 8],
            [0, 0, 8, 8]
        )
        .is_some()
    );
}

#[test]
fn a_layout_tile_takes_its_page_grid_and_span() {
    let layout = Layout::parse(include_bytes!("../../../../schema/examples/graph.json")).unwrap();
    let td_core::layout::Page::Grid {
        id,
        rows,
        cols,
        tiles,
        ..
    } = &layout.pages[0]
    else {
        panic!("graph example starts with a grid Page");
    };
    let tile = &tiles[0];
    let screen = ScreenSize { w: 1024, h: 600 };
    assert_eq!(
        tile_rect(&layout, id, &tile.id, screen),
        cell_rect(
            screen,
            layout.full_screen,
            [*rows, *cols],
            [tile.row, tile.col, tile.row_span, tile.col_span]
        )
    );
    assert!(tile_rect(&layout, id, &tile.id, screen).is_some());
    assert_eq!(tile_rect(&layout, id, "missing", screen), None);
    assert_eq!(tile_rect(&layout, "missing", &tile.id, screen), None);
}

#[test]
fn art_sizes_match_the_shared_now_playing_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../schema/live/now-playing-vectors.json"
    ))
    .unwrap();
    for c in vectors["cases"].as_array().unwrap() {
        let w = u16::try_from(c["w"].as_u64().unwrap()).unwrap();
        let h = u16::try_from(c["h"].as_u64().unwrap()).unwrap();
        let want = c["art"]["size"].as_u64().map(|s| u16::try_from(s).unwrap());
        assert_eq!(art_size(w, h), want, "{}", c["name"]);
    }
    assert_eq!(art_size(u16::MAX, u16::MAX), Some(256));
    assert_eq!(art_size(0, 0), None);
}

#[test]
fn live_layouts_match_shared_vectors() {
    use serde_json::{Value, json};
    use td_core::live::{bar_layout, now_playing_layout};
    let rect = |r: PixelRect| json!({"x":r.x,"y":r.y,"w":r.w,"h":r.h});
    let players: Value = serde_json::from_str(include_str!(
        "../../../../schema/live/now-playing-vectors.json"
    ))
    .unwrap();
    for c in players["cases"].as_array().unwrap() {
        let l = now_playing_layout(
            c["w"].as_u64().unwrap() as u16,
            c["h"].as_u64().unwrap() as u16,
        );
        assert_eq!(
            l.art
                .map(|r| json!({"x":r.x,"y":r.y,"size":r.w}))
                .unwrap_or(Value::Null),
            c["art"]
        );
        assert_eq!(json!({"x":l.text_x,"w":l.text_w}), c["text"]);
        assert_eq!(l.controls, c["controls"].as_bool().unwrap());
        let parts: Vec<_> = l
            .parts
            .iter()
            .map(|(name, r)| {
                let mut v = rect(*r);
                v["part"] = json!(name);
                v
            })
            .collect();
        assert_eq!(json!(parts), c["parts"]);
        for hit in c["hits"].as_array().unwrap() {
            assert_eq!(
                json!(l.part_at(
                    hit["x"].as_i64().unwrap() as i32,
                    hit["y"].as_i64().unwrap() as i32,
                    hit["state"].as_u64().unwrap() as u8
                )),
                hit["part"]
            );
        }
    }
    let bars: Value =
        serde_json::from_str(include_str!("../../../../schema/live/bar-vectors.json")).unwrap();
    for c in bars["cases"].as_array().unwrap() {
        let l = bar_layout(
            c["w"].as_u64().unwrap() as u16,
            c["h"].as_u64().unwrap() as u16,
            c["muted_declared"].as_bool().unwrap(),
        );
        assert_eq!(rect(l.track), c["track"]);
        assert_eq!(l.capture.map(rect).unwrap_or(Value::Null), c["capture"]);
        assert_eq!(l.mute.map(rect).unwrap_or(Value::Null), c["mute"]);
        for hit in c["hits"].as_array().unwrap() {
            assert_eq!(
                json!(l.zone_at(
                    hit["x"].as_i64().unwrap() as i32,
                    hit["y"].as_i64().unwrap() as i32,
                    hit["fresh"].as_bool().unwrap(),
                    hit["input"].as_bool().unwrap()
                )),
                hit["zone"]
            );
        }
    }
}

#[test]
fn inverse_preserves_rounded_edges_and_following_gaps() {
    use td_core::live::cell_at;
    for screen in [
        ScreenSize { w: 1024, h: 600 },
        ScreenSize { w: 600, h: 1024 },
    ] {
        for full in [false, true] {
            for rows in 1..=8 {
                for cols in 1..=8 {
                    for row in 0..rows {
                        for col in 0..cols {
                            let r =
                                cell_rect(screen, full, [rows, cols], [row, col, 1, 1]).unwrap();
                            for (x, y) in [
                                (f64::from(r.x), f64::from(r.y)),
                                (
                                    f64::from(r.x) + f64::from(r.w) + 15.9,
                                    f64::from(r.y) + f64::from(r.h) + 15.9,
                                ),
                            ] {
                                assert_eq!(
                                    cell_at(screen, full, [rows, cols], x, y),
                                    Some([i32::from(row), i32::from(col)])
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn outside_points_orientation_and_cover_crops() {
    use td_core::live::{cell_at, cover_crop};
    let screen = ScreenSize { w: 1024, h: 600 };
    assert_eq!(
        screen.oriented(td_core::layout::Orientation::Portrait),
        ScreenSize { w: 600, h: 1024 }
    );
    assert_eq!(cell_at(screen, false, [3, 4], 27.0, 75.0), Some([-1, -1]));
    assert_eq!(cell_at(screen, false, [3, 4], 1012.0, 76.0), Some([0, 4]));
    assert_eq!(cell_at(screen, false, [0, 4], 0.0, 0.0), None);
    assert_eq!(cell_at(screen, false, [3, 4], f64::NAN, 0.0), None);
    let art = PixelRect {
        x: 20,
        y: 20,
        w: 100,
        h: 100,
    };
    assert_eq!(
        cover_crop(screen, art),
        PixelRect {
            x: 462,
            y: 250,
            w: 100,
            h: 100
        }
    );
    assert_eq!(
        cover_crop(ScreenSize { w: 50, h: 60 }, art),
        PixelRect {
            x: 0,
            y: 0,
            w: 50,
            h: 60
        }
    );
    assert!(td_core::live::bar_layout(71, 200, true).capture.is_none());
}
