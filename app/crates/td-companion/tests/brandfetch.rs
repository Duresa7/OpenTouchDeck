//! Brandfetch with the user's own keys, proven against a local stand-in for Brandfetch. No test
//! reaches the real Brandfetch.
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
    routing::get,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;
use td_companion::{
    api::NoQuery,
    features::{
        assets::{Create, CreateAsset},
        brandfetch::{
            ApiKeyBody, ClientIdBody, ClientIdDelete, ClientIdPut, Get, KeyDelete, KeyPut, KeyTest,
        },
        jobs::List as JobList,
        layouts::{LastSeen, LayoutBody, LayoutPath, Push},
    },
    jobs::{Job, JobState},
    testing::TestCompanion,
};
use td_core::{Layout, Orientation};

const KEY: &str = "test-api-key";
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#e01020"/></svg>"##;

/// How the stand-in answers `GET /v2/brands/{brand}`.
#[derive(Clone)]
enum Answer {
    Brand(Value),
    Status(u16, Option<&'static str>),
    /// A 302 to this URL.
    Redirect(String),
    /// A 401 whose body and headers repeat the key back.
    Echo,
}
struct Fake {
    answer: Answer,
    svg: Vec<u8>,
    /// Where `/files/moved.*` redirects to.
    moved: String,
    /// Every request as `(path and query, authorization header)`.
    seen: Vec<(String, Option<String>)>,
}
type Shared = Arc<Mutex<Fake>>;

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(20, 10, image::Rgba([16, 160, 32, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
fn redirect(to: &str) -> axum::response::Response {
    (StatusCode::FOUND, [("location", to.to_owned())]).into_response()
}
fn seen(uri: &Uri, headers: &HeaderMap) -> (String, Option<String>) {
    let auth = headers
        .get("authorization")
        .map(|v| v.to_str().unwrap().to_owned());
    (uri.path_and_query().unwrap().as_str().to_owned(), auth)
}
async fn brand(
    State(fake): State<Shared>,
    Path(_): Path<String>,
    uri: Uri,
    headers: HeaderMap,
) -> axum::response::Response {
    let mut fake = fake.lock();
    fake.seen.push(seen(&uri, &headers));
    match fake.answer.clone() {
        Answer::Brand(body) => axum::Json(body).into_response(),
        Answer::Redirect(to) => redirect(&to),
        Answer::Echo => {
            let auth = headers["authorization"].to_str().unwrap().to_owned();
            (
                StatusCode::UNAUTHORIZED,
                [
                    ("x-bf-error", auth.clone()),
                    ("www-authenticate", auth.clone()),
                ],
                format!("{{\"message\":\"Invalid key {auth}\"}}"),
            )
                .into_response()
        }
        Answer::Status(status, reason) => {
            let mut response = StatusCode::from_u16(status).unwrap().into_response();
            if let Some(reason) = reason {
                response
                    .headers_mut()
                    .insert("x-bf-error", reason.parse().unwrap());
            }
            response
        }
    }
}
async fn file(
    State(fake): State<Shared>,
    Path(name): Path<String>,
    uri: Uri,
    headers: HeaderMap,
) -> axum::response::Response {
    let mut fake = fake.lock();
    fake.seen.push(seen(&uri, &headers));
    if name.starts_with("moved") {
        return redirect(&fake.moved);
    }
    if name.ends_with(".png") {
        ([("content-type", "image/png")], png()).into_response()
    } else {
        ([("content-type", "image/svg+xml")], fake.svg.clone()).into_response()
    }
}
async fn search(State(fake): State<Shared>, Path(q): Path<String>) -> StatusCode {
    fake.lock().seen.push((format!("/v2/search/{q}"), None));
    StatusCode::OK
}

/// A local stand-in for Brandfetch, and its base URL.
async fn stand_in() -> (Shared, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fake = Arc::new(Mutex::new(Fake {
        answer: Answer::Status(500, None),
        svg: SVG.into(),
        moved: String::new(),
        seen: vec![],
    }));
    let app = Router::new()
        .route("/v2/brands/{id}", get(brand))
        .route("/v2/search/{q}", get(search))
        .route("/files/{name}", get(file))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, base)
}
/// A Companion whose Brand API calls go to a local stand-in.
async fn rig() -> (TestCompanion, Shared, String) {
    let (fake, base) = stand_in().await;
    let td = TestCompanion::start().await;
    td.brandfetch().use_api(&base);
    (td, fake, base)
}
fn logos(base: &str, kind: &str, formats: &[&str]) -> Value {
    json!({"name": "Example", "domain": "example.com", "logos": [{
        "type": kind, "theme": null,
        "formats": formats.iter().map(|f| json!({"src": format!("{base}/files/{kind}.{f}"), "format": f})).collect::<Vec<_>>(),
    }]})
}
fn pick(brand: &str, logo_type: &str) -> CreateAsset {
    serde_json::from_value(json!({"source": {
        "kind": "brandfetch", "brand": brand, "logo_type": logo_type, "theme": "auto",
        "padding": 4, "background": null, "tint": null,
    }}))
    .unwrap()
}
async fn render(td: &TestCompanion, asset: CreateAsset) -> Job {
    let job = td.call::<Create>((), NoQuery, asset).await.unwrap();
    td.wait(&job).await
}
async fn set_key(td: &TestCompanion) {
    td.call::<KeyPut>((), NoQuery, ApiKeyBody { value: KEY.into() })
        .await
        .unwrap();
}

#[tokio::test]
async fn settings_store_the_client_id_and_keep_the_key_write_only() {
    let td = TestCompanion::start().await;
    let status = td.call::<Get>((), NoQuery, ()).await.unwrap();
    assert_eq!(status.client_id, None);
    assert!(!status.key_set);

    let status = td
        .call::<ClientIdPut>(
            (),
            NoQuery,
            ClientIdBody {
                client_id: " test-client-id ".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(status.client_id.as_deref(), Some("test-client-id"));
    let status = td
        .call::<KeyPut>((), NoQuery, ApiKeyBody { value: KEY.into() })
        .await
        .unwrap();
    assert!(status.key_set);
    // No route ever sends the key back.
    let text = td
        .request::<Get>((), NoQuery, &())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!text.contains(KEY), "{text}");
    assert!(text.contains("\"key_set\":true"), "{text}");

    for (body, field) in [
        ("", "client_id"),
        ("has space", "client_id"),
        ("a/b", "client_id"),
    ] {
        let response = td
            .request::<ClientIdPut>(
                (),
                NoQuery,
                &ClientIdBody {
                    client_id: body.into(),
                },
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{body}");
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], "request.invalid");
        assert_eq!(error["error"]["details"]["part"], field);
    }
    let response = td
        .request::<KeyPut>((), NoQuery, &ApiKeyBody { value: " ".into() })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);

    let status = td.call::<KeyDelete>((), NoQuery, ()).await.unwrap();
    assert!(!status.key_set);
    let status = td.call::<ClientIdDelete>((), NoQuery, ()).await.unwrap();
    assert_eq!(status.client_id, None);
}

fn csp_of(response: &reqwest::Response) -> String {
    response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn the_csp_opens_to_brandfetch_only_while_a_client_id_is_set() {
    let td = TestCompanion::start().await;
    let port = td.url.rsplit(':').next().unwrap().to_owned();
    let closed = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' blob: data:; connect-src 'self' ws://127.0.0.1:{port}; \
         font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'none'"
    );
    let index = || td.raw().get("/");
    let before = csp_of(&index().send().await.unwrap());
    assert_eq!(before, closed);
    // An API key alone opens nothing: search needs the client ID.
    set_key(&td).await;
    assert_eq!(csp_of(&index().send().await.unwrap()), closed);

    td.call::<ClientIdPut>(
        (),
        NoQuery,
        ClientIdBody {
            client_id: "test-client-id".into(),
        },
    )
    .await
    .unwrap();
    let open = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' blob: data: https://cdn.brandfetch.io https://asset.brandfetch.io; \
         connect-src 'self' ws://127.0.0.1:{port} https://api.brandfetch.io; \
         font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'none'"
    );
    assert_eq!(csp_of(&index().send().await.unwrap()), open);
    let api = td.request::<Get>((), NoQuery, &()).send().await.unwrap();
    assert_eq!(csp_of(&api), open);
    // The document still sends no Referer; the picker's images ask for one themselves.
    assert_eq!(api.headers()["referrer-policy"], "no-referrer");

    td.call::<ClientIdDelete>((), NoQuery, ()).await.unwrap();
    assert_eq!(csp_of(&index().send().await.unwrap()), closed);
}

#[tokio::test]
async fn a_pick_renders_svg_and_png_logos_with_the_key_and_never_searches() {
    let (td, fake, base) = rig().await;
    set_key(&td).await;

    fake.lock().answer = Answer::Brand(logos(&base, "icon", &["png", "svg", "jpeg"]));
    let done = render(&td, pick("example.com", "icon")).await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    let result = done.result.unwrap();
    assert_eq!(
        (result["width"].clone(), result["kind"].clone()),
        (json!(64), json!("tdi1"))
    );

    fake.lock().answer = Answer::Brand(logos(&base, "symbol", &["jpeg", "png"]));
    let done = render(&td, pick("id_example01", "symbol")).await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    assert_ne!(done.result.unwrap()["id"], result["id"]);

    let seen = fake.lock().seen.clone();
    let bearer = Some(format!("Bearer {KEY}"));
    assert_eq!(
        seen,
        [
            ("/v2/brands/example.com".to_owned(), bearer.clone()),
            ("/files/icon.svg".to_owned(), None),
            ("/v2/brands/id_example01".to_owned(), bearer),
            ("/files/symbol.png".to_owned(), None),
        ],
        "SVG first, then PNG; the key goes only to the Brand API; no search"
    );
    let status = td.call::<Get>((), NoQuery, ()).await.unwrap();
    assert_eq!((status.logos, status.need_refresh), (2, 0));
}

#[tokio::test]
async fn every_brand_api_failure_maps_to_a_clear_job_error() {
    let (td, fake, base) = rig().await;
    fake.lock().answer = Answer::Brand(logos(&base, "icon", &["svg"]));
    let done = render(&td, pick("example.com", "icon")).await;
    assert_eq!(done.state, JobState::Failed);
    let error = done.error.unwrap();
    assert_eq!(error.code, "brandfetch.no_key");
    assert_eq!(
        error.message,
        "Add your Brandfetch API key in Settings, then retry."
    );
    assert!(fake.lock().seen.is_empty(), "no call without a key");

    set_key(&td).await;
    for (answer, code) in [
        (
            Answer::Status(401, Some("authorization_required")),
            "brandfetch.key_refused",
        ),
        (
            Answer::Status(403, Some("api_key_invalid")),
            "brandfetch.key_refused",
        ),
        (
            Answer::Status(403, Some("no_api_credits")),
            "brandfetch.quota",
        ),
        (
            Answer::Status(429, Some("quota_exceeded")),
            "brandfetch.quota",
        ),
        (
            Answer::Status(404, Some("brand_not_found")),
            "brandfetch.not_found",
        ),
        (
            Answer::Status(500, Some("internal_error")),
            "brandfetch.unavailable",
        ),
        (
            Answer::Brand(json!({"name": "Example", "logos": []})),
            "brandfetch.no_logo",
        ),
        (
            Answer::Brand(logos("https://cdn.brandfetch.io", "icon", &["svg"])),
            "brandfetch.no_logo",
        ),
    ] {
        fake.lock().answer = answer;
        let done = render(&td, pick("example.com", "icon")).await;
        assert_eq!(done.state, JobState::Failed);
        assert_eq!(done.error.unwrap().code, code);
    }
    let done = render(&td, pick("../etc", "icon")).await;
    assert_eq!(done.error.unwrap().code, "brandfetch.not_found");

    // A brand with no icon gives its symbol, or failing that its logo.
    for kind in ["symbol", "logo"] {
        fake.lock().answer = Answer::Brand(logos(&base, kind, &["png"]));
        let done = render(&td, pick("example.com", "icon")).await;
        assert_eq!(done.state, JobState::Succeeded, "{kind}");
        assert!(
            fake.lock()
                .seen
                .iter()
                .any(|(path, _)| *path == format!("/files/{kind}.png")),
            "{kind}"
        );
    }
}

#[tokio::test]
async fn a_changed_logo_is_offered_as_a_replacement_and_never_pushed() {
    let (td, fake, base) = rig().await;
    set_key(&td).await;
    let device = td.plug_device("mock0001").await;
    fake.lock().answer = Answer::Brand(logos(&base, "icon", &["svg"]));
    let done = render(&td, pick("example.com", "icon")).await;
    let old = done.result.unwrap()["id"].as_str().unwrap().to_owned();
    let layout: Layout = serde_json::from_value(json!({
        "schema": 2, "name": "Logos", "orientation": "landscape",
        "pages": [{"id": "main", "title": "Main", "kind": "grid", "rows": 1, "cols": 1, "tiles": [{
            "id": "t0", "label": "Example", "row": 0, "col": 0, "rowSpan": 1, "colSpan": 1,
            "icon": null, "asset": format!("sha256:{old}"), "action": {"type": "none"}
        }]}]
    }))
    .unwrap();
    let path = || LayoutPath {
        device: device.to_string(),
        orientation: Orientation::Landscape,
    };
    let job = td
        .call::<Push>(path(), NoQuery, LayoutBody::new(&layout))
        .await
        .unwrap();
    assert_eq!(td.wait(&job).await.state, JobState::Succeeded);
    let sent = td.deck(&device).received().len();

    fake.lock().svg = SVG.replace("#e01020", "#2040f0").into_bytes();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let report = td.refresh_brandfetch(now + 26 * 24 * 60 * 60).await;
    assert_eq!(report.replaced.len(), 1, "{report:?}");
    let new = report.replaced[0].1.clone();
    assert_ne!(new, old);

    // The UI learns the replacement and swaps it in its drafts; the Companion's copy of what
    // the Device shows stays as the Device shows it.
    let status = td.call::<Get>((), NoQuery, ()).await.unwrap();
    assert_eq!(status.replaced.get(&old), Some(&new));
    let copy = td.call::<LastSeen>(path(), NoQuery, ()).await.unwrap();
    let tile = serde_json::to_value(&copy.layout).unwrap()["pages"][0]["tiles"][0].clone();
    assert_eq!(tile["asset"], format!("sha256:{old}"));
    assert_eq!(
        td.deck(&device).received().len(),
        sent,
        "nothing went to the Device"
    );
    assert!(
        td.deck(&device).assets().contains_key(&old),
        "the Device keeps its Asset"
    );
}

#[tokio::test]
async fn the_key_stays_out_of_urls_redirects_answers_and_errors() {
    let (td, fake, base) = rig().await;
    // A second host stands for anywhere a redirect could lead.
    let (elsewhere, other) = stand_in().await;
    set_key(&td).await;
    let mut errors = vec![];

    // The Brand API redirects: the Companion doesn't follow, so the key goes nowhere else.
    fake.lock().answer = Answer::Redirect(format!("{other}/v2/brands/example.com"));
    let done = render(&td, pick("example.com", "icon")).await;
    let error = done.error.unwrap();
    assert_eq!(error.code, "brandfetch.unavailable");
    errors.push(error);
    // A logo file redirects: never followed either.
    let mut brand = logos(&base, "icon", &["svg"]);
    brand["logos"][0]["formats"][0]["src"] = json!(format!("{base}/files/moved.svg"));
    fake.lock().answer = Answer::Brand(brand);
    fake.lock().moved = format!("{other}/files/icon.svg");
    let done = render(&td, pick("example.com", "icon")).await;
    let error = done.error.unwrap();
    assert_eq!(error.code, "brandfetch.unavailable");
    errors.push(error);
    assert!(
        elsewhere.lock().seen.is_empty(),
        "nothing reached the other host"
    );

    // Brandfetch repeats the key in its answer: none of it reaches an error.
    fake.lock().answer = Answer::Echo;
    let done = render(&td, pick("example.com", "icon")).await;
    let error = done.error.unwrap();
    assert_eq!(error.code, "brandfetch.key_refused");
    errors.push(error);
    let response = td
        .request::<KeyTest>((), NoQuery, &())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    let text = response.text().await.unwrap();
    assert!(text.contains("brandfetch.key_refused"), "{text}");
    assert!(!text.contains(KEY), "{text}");

    for error in &errors {
        let text = serde_json::to_string(error).unwrap();
        assert!(!text.contains(KEY), "{text}");
    }
    // No route answers with the key: settings, every Job, and a key test.
    for text in [
        td.request::<Get>((), NoQuery, &())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        td.request::<JobList>((), NoQuery, &())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        td.request::<KeyPut>((), NoQuery, &ApiKeyBody { value: KEY.into() })
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    ] {
        assert!(!text.contains(KEY), "{text}");
    }
    // The key went only as a Bearer header, never in a URL.
    let seen = fake.lock().seen.clone();
    assert!(!seen.is_empty());
    for (url, auth) in &seen {
        assert!(!url.contains(KEY), "{url}");
        if url.starts_with("/v2/brands/") {
            assert_eq!(auth.as_deref(), Some(format!("Bearer {KEY}").as_str()));
        } else {
            assert_eq!(auth, &None, "{url}");
        }
    }
}

#[tokio::test]
async fn saving_the_key_never_calls_brandfetch_and_a_key_test_makes_one_free_call() {
    let (td, fake, _) = rig().await;
    let response = td
        .request::<KeyTest>((), NoQuery, &())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409, "no key yet");
    set_key(&td).await;
    td.call::<ClientIdPut>(
        (),
        NoQuery,
        ClientIdBody {
            client_id: "test-client-id".into(),
        },
    )
    .await
    .unwrap();
    assert!(fake.lock().seen.is_empty(), "saving calls nothing");

    fake.lock().answer = Answer::Brand(json!({"name": "Brandfetch", "logos": []}));
    let status = td.call::<KeyTest>((), NoQuery, ()).await.unwrap();
    assert!(status.key_set);
    assert_eq!(
        fake.lock().seen,
        [(
            "/v2/brands/brandfetch.com".to_owned(),
            Some(format!("Bearer {KEY}"))
        )],
        "one Brand API call, for Brandfetch's own brand"
    );
    fake.lock().answer = Answer::Status(401, None);
    let response = td
        .request::<KeyTest>((), NoQuery, &())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
}

#[tokio::test]
async fn the_refresh_loop_goes_by_the_companion_clock_and_catches_up_after_sleep() {
    let (td, fake, base) = rig().await;
    set_key(&td).await;
    fake.lock().answer = Answer::Brand(logos(&base, "icon", &["svg"]));
    assert_eq!(
        render(&td, pick("example.com", "icon")).await.state,
        JobState::Succeeded
    );
    let brand_calls = || {
        fake.lock()
            .seen
            .iter()
            .filter(|(path, _)| path.starts_with("/v2/brands/"))
            .count()
    };
    assert_eq!(brand_calls(), 1);
    let refresh = td.start_brandfetch_refresh(std::time::Duration::from_millis(20));
    // Several checks pass, and a fresh logo makes no call.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(brand_calls(), 1);
    // The PC "sleeps" 26 days: the next check reads the wall clock and calls once.
    td.clock()
        .advance(std::time::Duration::from_secs(26 * 24 * 60 * 60));
    td.until(|_| brand_calls() == 2).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(brand_calls(), 2, "renewed, so later checks call nothing");
    refresh.abort();
}

#[tokio::test]
async fn a_hostile_svg_fails_cleanly_and_reads_nothing_outside_the_file() {
    use base64::Engine;
    use std::io::Write;
    let (td, fake, base) = rig().await;
    set_key(&td).await;
    fake.lock().answer = Answer::Brand(logos(&base, "icon", &["svg"]));
    // A local file the hostile SVG links to. Had it been read, the pick would draw its red.
    let local = td.data_dir().join("local.svg");
    std::fs::write(&local, SVG).unwrap();
    let href = url::Url::from_file_path(&local).unwrap();
    let image = |href: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image href="{href}" xlink:href="{href}" width="10" height="10"/></svg>"#
        )
        .into_bytes()
    };
    let nested = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(SVG)
    );
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(SVG.as_bytes()).unwrap();
    let entity = r##"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY red "#e01020">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="&red;"/></svg>"##;
    for (name, svg) in [
        ("an ENTITY", entity.as_bytes().to_vec()),
        ("gzip", gzip.finish().unwrap()),
        ("a local file image", image(href.as_str())),
        ("a nested SVG data: image", image(&nested)),
    ] {
        fake.lock().svg = svg;
        let done = render(&td, pick("example.com", "icon")).await;
        assert_eq!(done.state, JobState::Failed, "{name}");
        let error = done.error.unwrap();
        assert_eq!(
            error.code, "asset.render_failed",
            "{name}: {}",
            error.message
        );
    }
    // The same stand-in still serves a plain SVG that works.
    fake.lock().svg = SVG.into();
    assert_eq!(
        render(&td, pick("example.com", "icon")).await.state,
        JobState::Succeeded
    );
}
