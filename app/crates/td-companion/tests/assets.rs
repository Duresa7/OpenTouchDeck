use serde_json::{Value, json};
use td_companion::{
    api::NoQuery,
    features::{
        assets::{
            AssetPath, BuiltinList, Create, CreateAsset, File, Prune, PruneRequest, Suggest,
            SuggestRequest,
        },
        devices::DevicePath,
    },
    jobs::JobState,
    testing::TestCompanion,
};
use td_core::{Layout, Orientation};

async fn failure(request: reqwest::RequestBuilder) -> (u16, Value) {
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    (
        status,
        response.json::<Value>().await.unwrap()["error"].clone(),
    )
}

#[tokio::test]
async fn builtin_icons_suggestions_and_files() {
    let td = TestCompanion::start().await;
    let builtin = td.call::<BuiltinList>((), NoQuery, ()).await.unwrap();
    assert!(builtin.len() >= 6);
    assert!(builtin.iter().all(|icon| icon.id.len() == 16));

    let suggest = |action: Value| SuggestRequest {
        action: serde_json::from_value(action).unwrap(),
    };
    let command = json!({"type": "command", "program": "x", "args": [], "cwd": "", "shell": false});
    let terminal = td
        .call::<Suggest>((), NoQuery, suggest(command))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(terminal.name, "terminal");
    assert!(builtin.iter().any(|icon| icon.id == terminal.id));
    let web = json!({"type": "open", "target": "https://example.com"});
    let browser = td.call::<Suggest>((), NoQuery, suggest(web)).await.unwrap();
    assert_eq!(browser.unwrap().name, "browser");
    let none = td
        .call::<Suggest>((), NoQuery, suggest(json!({"type": "none"})))
        .await
        .unwrap();
    assert!(none.is_none());

    let response = td
        .request::<File>(AssetPath { asset: terminal.id }, NoQuery, &())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/vnd.touchdeck.tdi1"
    );
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.len(), td_host::icon_pack::FILE_BYTES);
    assert!(bytes.starts_with(b"TDI1"));
    let missing = AssetPath {
        asset: "0123456789abcdef".into(),
    };
    let (status, error) = failure(td.request::<File>(missing, NoQuery, &())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("asset.not_found"))
    );
}

#[tokio::test]
async fn prune_keeps_what_stored_layouts_and_the_caller_use() {
    let td = TestCompanion::start().await;
    let a = td.plug_device("mock0001").await;
    let used = td_host::icons::builtin()[0].id.clone();
    let layout: Layout = serde_json::from_value(json!({
        "schema": 1, "name": "Icons", "orientation": "landscape",
        "pages": [{"id": "main", "title": "Main", "kind": "grid", "rows": 1, "cols": 1, "tiles": [
            {"id": "a", "label": "A", "row": 0, "col": 0, "rowSpan": 1, "colSpan": 1,
             "icon": format!("file:{used}"), "action": {"type": "none"}}
        ]}]
    }))
    .unwrap();
    let job = td
        .handle()
        .layout_operations()
        .push(
            a.to_string(),
            Orientation::Landscape,
            layout.to_canonical_json(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(td.wait(&job).await.state, JobState::Succeeded);
    let ids = |s: &[&str]| s.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    td.deck(&a).add_icons(ids(&[
        "00000000000000aa",
        "00000000000000bb",
        "00000000000000cc",
    ]));

    let job = td
        .call::<Prune>(
            DevicePath { device: a.clone() },
            NoQuery,
            PruneRequest {
                also_keep: ids(&["00000000000000bb"]),
            },
        )
        .await
        .unwrap();
    assert_eq!(job.kind, "assets.prune");
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    assert_eq!(
        done.result.unwrap()["removed"],
        json!(["00000000000000aa", "00000000000000cc"])
    );
    assert_eq!(td.deck(&a).icons().into_iter().collect::<Vec<_>>(), {
        let mut kept = vec!["00000000000000bb".to_string(), used];
        kept.sort();
        kept
    });
    let missing = DevicePath {
        device: td_companion::devices::DeviceId::parse("mock0009").unwrap(),
    };
    let (status, error) =
        failure(td.request::<Prune>(missing, NoQuery, &PruneRequest::default())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("device.not_found"))
    );
}

#[tokio::test]
async fn at_most_two_renders_run_at_once() {
    let td = TestCompanion::start().await;
    let create = || CreateAsset {
        source: serde_json::from_value(json!({"kind": "app", "path": "/no/such/program"})).unwrap(),
        fit: None,
    };
    td.hold_jobs();
    let first = td.call::<Create>((), NoQuery, create()).await.unwrap();
    let second = td.call::<Create>((), NoQuery, create()).await.unwrap();
    assert_eq!(first.kind, "assets.render");
    let (status, error) = failure(td.request::<Create>((), NoQuery, &create())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (429, Some("limits.exceeded"))
    );
    td.release_jobs();
    for job in [first, second] {
        let done = td.wait(&job).await;
        assert_eq!(done.state, JobState::Failed);
        assert_eq!(done.error.unwrap().code, "asset.render_failed");
    }
    // The finished renders free their slots.
    let third = td.call::<Create>((), NoQuery, create()).await.unwrap();
    td.wait(&third).await;
}

struct CatalogFetch(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl td_host::catalog::Fetcher for CatalogFetch {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, td_host::catalog::Error> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert!(url.contains("simple-icons@16.36.0/icons/github.svg"));
        Ok(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><path d="M0 0h10v10H0z"/></svg>"#.to_vec())
    }
}
#[tokio::test]
async fn catalog_search_and_render_jobs() {
    use td_companion::features::assets::{CatalogQuery, CatalogSearch};
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let td = TestCompanion::start_with_catalog(
        Default::default(),
        Box::new(CatalogFetch(calls.clone())),
    )
    .await;
    let hits = td
        .call::<CatalogSearch>(
            (),
            CatalogQuery {
                q: "github".into(),
                catalog: None,
            },
            (),
        )
        .await
        .unwrap();
    for source in ["simple-icons", "selfhst"] {
        assert!(hits.iter().any(|e| e.source == source));
    }
    let create = || {
        serde_json::from_value::<CreateAsset>(json!({"source":{"kind":"catalog","catalog":"simple-icons","id":"github","padding":4,"tint":[255,0,0]}})).unwrap()
    };
    let job = td.call::<Create>((), NoQuery, create()).await.unwrap();
    assert_eq!(job.kind, "assets.render");
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    let result = done.result.unwrap();
    assert_eq!(result["width"], 64);
    assert_eq!(result["height"], 64);
    let response = td
        .request::<File>(
            AssetPath {
                asset: result["id"].as_str().unwrap().into(),
            },
            NoQuery,
            &(),
        )
        .send()
        .await
        .unwrap();
    let bytes = response.bytes().await.unwrap();
    assert!(td_host::icon_pack::is_icon(&bytes));
    let center = 32 * 64 + 32;
    assert_eq!(&bytes[12 + center * 2..14 + center * 2], &[0, 248]);
    assert_eq!(bytes[12 + 64 * 64 * 2], 0);
    assert_eq!(bytes[12 + 64 * 64 * 2 + center], 255);
    let again = td.call::<Create>((), NoQuery, create()).await.unwrap();
    assert_eq!(td.wait(&again).await.state, JobState::Succeeded);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let td = TestCompanion::start().await;
    let job = td.call::<Create>((), NoQuery, create()).await.unwrap();
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, "asset.render_failed");
}

struct PanicFetch;
impl td_host::catalog::Fetcher for PanicFetch {
    fn fetch(&self, _: &str) -> Result<Vec<u8>, td_host::catalog::Error> {
        panic!("simulated catalog panic");
    }
}

#[tokio::test]
async fn a_panicking_render_job_fails_and_frees_its_slot() {
    let td = TestCompanion::start_with_catalog(Default::default(), Box::new(PanicFetch)).await;
    let panicking = |id: &str| {
        serde_json::from_value::<CreateAsset>(
            json!({"source": {"kind": "catalog", "catalog": "simple-icons", "id": id}}),
        )
        .unwrap()
    };
    for id in ["github", "gitlab"] {
        let job = td.call::<Create>((), NoQuery, panicking(id)).await.unwrap();
        let done = td.wait(&job).await;
        assert_eq!(done.state, JobState::Failed, "{done:?}");
        assert_eq!(done.error.unwrap().code, "internal");
    }

    let app_icon = || CreateAsset {
        source: serde_json::from_value(json!({"kind": "app", "path": "/no/such/program"})).unwrap(),
        fit: None,
    };
    td.hold_jobs();
    let first = td.call::<Create>((), NoQuery, app_icon()).await.unwrap();
    let second = td.call::<Create>((), NoQuery, app_icon()).await.unwrap();
    td.release_jobs();
    for job in [first, second] {
        assert_eq!(td.wait(&job).await.state, JobState::Failed);
    }
}

fn png_color(rgb: [u8; 3]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn render_png(td: &TestCompanion, rgb: [u8; 3]) -> td_companion::jobs::Job {
    let uploaded: Value = td
        .raw()
        .post("/api/v1/uploads")
        .bearer_auth(&td.token)
        .header("content-type", "image/png")
        .body(png_color(rgb))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let body: CreateAsset = serde_json::from_value(
        json!({"source": {"kind": "upload", "upload": uploaded["id"]}, "fit": "contain"}),
    )
    .unwrap();
    let job = td.call::<Create>((), NoQuery, body).await.unwrap();
    td.wait(&job).await
}

#[tokio::test]
async fn a_running_layout_push_pins_its_assets_against_a_sweep() {
    let td = TestCompanion::start_with_asset_quota(td_host::icon_pack::FILE_BYTES as u64).await;
    let _events = td.events().await;
    let device = td.plug_device("mock0001").await;

    let first = render_png(&td, [255, 0, 0]).await;
    assert_eq!(first.state, JobState::Succeeded, "{first:?}");
    let pinned_id = first.result.unwrap()["id"].as_str().unwrap().to_owned();

    let layout: Layout = serde_json::from_value(json!({
        "schema": 2,
        "name": "Pinned",
        "orientation": "landscape",
        "pages": [{
            "id": "main",
            "title": "Main",
            "kind": "grid",
            "rows": 1,
            "cols": 1,
            "tiles": [{
                "id": "t0",
                "label": "Pinned",
                "row": 0,
                "col": 0,
                "rowSpan": 1,
                "colSpan": 1,
                "icon": null,
                "asset": format!("sha256:{pinned_id}"),
                "action": {"type": "none"}
            }]
        }]
    }))
    .unwrap();

    let before = td.deck(&device).received().len();
    td.deck(&device)
        .delay_replies(std::time::Duration::from_millis(800));
    let push_job = td
        .handle()
        .layout_operations()
        .push(
            device.to_string(),
            Orientation::Landscape,
            layout.to_canonical_json(),
            None,
        )
        .await
        .unwrap();
    td.until(|td| {
        td.deck(&device).received()[before..]
            .iter()
            .any(|m| m["m"] == "icon.list")
    })
    .await;
    td.clock()
        .advance(std::time::Duration::from_secs(2 * 60 * 60));

    let contender = render_png(&td, [0, 0, 255]).await;
    td.deck(&device).delay_replies(std::time::Duration::ZERO);

    assert_eq!(contender.state, JobState::Failed, "{contender:?}");
    assert_eq!(contender.error.unwrap().code, "asset.store_full");

    let pushed = td.wait(&push_job).await;
    assert_eq!(pushed.state, JobState::Succeeded, "{pushed:?}");
    assert!(td.deck(&device).assets().contains_key(&pinned_id));
}

#[tokio::test]
async fn prune_skips_an_unknown_cover_in_use_and_finishes_on_repeat() {
    let td = TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    let deck = td.deck(&device);
    let mut files: Vec<_> = (0..3u8)
        .map(|n| {
            let bytes = vec![0xff, 0xd8, 0xff, n];
            (td_link::asset::asset_id(&bytes), bytes)
        })
        .collect();
    files.sort();
    let cover = files[1].0.clone();
    let unused = vec![files[0].0.clone(), files[2].0.clone()];
    for (id, bytes) in files {
        deck.seed_asset(bytes, id == cover);
    }
    for expected in [json!(unused), json!([])] {
        let job = td
            .call::<Prune>(
                DevicePath {
                    device: device.clone(),
                },
                NoQuery,
                PruneRequest::default(),
            )
            .await
            .unwrap();
        let done = td.wait(&job).await;
        assert_eq!(done.state, JobState::Succeeded, "{done:?}");
        let result = done.result.unwrap();
        assert_eq!(result["removed"], expected);
        assert_eq!(result["kept_in_use"], 1);
        assert_eq!(
            deck.assets().keys().cloned().collect::<Vec<_>>(),
            vec![cover.clone()]
        );
    }
}

#[tokio::test]
async fn prune_still_fails_for_other_delete_refusals_with_a_delete_message() {
    let td = TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    let deck = td.deck(&device);
    deck.seed_asset(vec![0xff, 0xd8, 0xff, 0], false);
    deck.refuse_assets("v2.asset.delete", "busy", 1);
    let job = td
        .call::<Prune>(DevicePath { device }, NoQuery, PruneRequest::default())
        .await
        .unwrap();
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Failed);
    let error = done.error.unwrap();
    assert_eq!(error.code, "asset.refused");
    assert_eq!(error.details.unwrap()["reason"], "busy");
    assert_eq!(error.message, "The Device could not remove this Asset.");
    assert_eq!(deck.assets().len(), 1);
}
