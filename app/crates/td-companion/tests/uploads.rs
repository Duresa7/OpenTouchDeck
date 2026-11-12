use serde_json::{Value, json};
use td_companion::{
    api::NoQuery,
    features::{
        assets::{AssetPath, Create, CreateAsset, File, OnDevice},
        devices::DevicePath,
        jobs::{Cancel, JobPath},
        layouts::{LayoutBody, LayoutPath, Push},
    },
    jobs::JobState,
    testing::TestCompanion,
};
use td_core::{Layout, Orientation};

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(128, 64, image::Rgba([255, 0, 0, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}
async fn upload(td: &TestCompanion) -> Value {
    td.raw()
        .post("/api/v1/uploads")
        .bearer_auth(&td.token)
        .header("content-type", "image/png")
        .body(png())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}
async fn render(td: &TestCompanion, id: &str, fit: &str) -> td_companion::jobs::Job {
    let body: CreateAsset =
        serde_json::from_value(json!({"source":{"kind":"upload","upload":id},"fit":fit})).unwrap();
    let job = td.call::<Create>((), NoQuery, body).await.unwrap();
    td.wait(&job).await
}
#[tokio::test]
async fn upload_fits_and_is_consumed_once() {
    let td = TestCompanion::start().await;
    for fit in ["contain", "cover"] {
        let uploaded = upload(&td).await;
        let id = uploaded["id"].as_str().unwrap();
        let done = render(&td, id, fit).await;
        assert_eq!(done.state, JobState::Succeeded, "{done:?}");
        let result = done.result.unwrap();
        let hash = result["id"].as_str().unwrap();
        assert_eq!(hash.len(), 64);
        let file = td
            .request::<File>(AssetPath { asset: hash.into() }, NoQuery, &())
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(td_link::asset::asset_id(&file), hash);
        let alpha = &file[td_host::icon_pack::HEADER_BYTES + td_host::icon_pack::COLOR_BYTES..];
        if fit == "cover" {
            assert!(alpha.iter().all(|a| *a == 255));
        } else {
            assert_eq!(alpha.iter().filter(|a| **a == 255).count(), 2048);
        }
        assert_eq!(
            render(&td, id, fit).await.error.unwrap().code,
            "upload.not_found"
        );
    }
}
#[tokio::test]
async fn invalid_uploads_release_capacity() {
    let td = TestCompanion::start().await;
    for _ in 0..10 {
        let response = td
            .raw()
            .post("/api/v1/uploads")
            .bearer_auth(&td.token)
            .header("content-type", "image/png")
            .body("not an image")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
    }
    upload(&td).await;
}
#[tokio::test]
async fn second_layout_push_sends_no_asset() {
    let td = TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    let uploaded = upload(&td).await;
    let done = render(&td, uploaded["id"].as_str().unwrap(), "cover").await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    let hash = done.result.unwrap()["id"].as_str().unwrap().to_owned();
    let layout = Layout::parse(&serde_json::to_vec(&json!({"schema":2,"name":"Assets","orientation":"landscape","pages":[{"id":"main","title":"Main","kind":"grid","rows":1,"cols":1,"tiles":[{"id":"a","label":"A","row":0,"col":0,"rowSpan":1,"colSpan":1,"icon":null,"asset":format!("sha256:{hash}"),"action":{"type":"none"}}]}]})).unwrap()).unwrap();
    for count in [1, 0] {
        let before = td.deck("mock0001").chunk_bytes();
        let job = td
            .call::<Push>(
                LayoutPath {
                    device: device.to_string(),
                    orientation: Orientation::Landscape,
                },
                NoQuery,
                LayoutBody::new(&layout),
            )
            .await
            .unwrap();
        let done = td.wait(&job).await;
        assert_eq!(done.state, JobState::Succeeded, "{done:?}");
        assert_eq!(
            td.deck("mock0001").chunk_bytes() - before,
            layout.to_canonical_json().len() + count * td_host::icon_pack::FILE_BYTES
        );
        assert_eq!(
            done.result.unwrap()["assets"]["sent"]
                .as_array()
                .unwrap()
                .len(),
            count
        );
    }
    let stored = td
        .call::<OnDevice>(DevicePath { device }, NoQuery, ())
        .await
        .unwrap();
    assert_eq!(stored.ids, vec![hash]);
    assert!(stored.storage_free > 0);
}

#[tokio::test]
async fn uploads_belong_to_the_creating_session() {
    let mut td = TestCompanion::start().await;
    let uploaded = upload(&td).await;
    let owner = td.token.clone();
    let session: Value = td
        .raw()
        .post("/api/v1/auth/sessions")
        .bearer_auth(&td.pairing_token)
        .json(&json!({"kind":"window"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    td.token = session["token"].as_str().unwrap().into();
    let id = uploaded["id"].as_str().unwrap();
    assert_eq!(
        render(&td, id, "contain").await.error.unwrap().code,
        "upload.not_found"
    );
    td.token = owner;
    assert_eq!(render(&td, id, "contain").await.state, JobState::Succeeded);
}

#[tokio::test]
async fn pending_upload_limit_recovers_after_consumption() {
    let td = TestCompanion::start().await;
    let first = upload(&td).await;
    for _ in 1..8 {
        upload(&td).await;
    }
    let response = td
        .raw()
        .post("/api/v1/uploads")
        .bearer_auth(&td.token)
        .header("content-type", "image/png")
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 429);
    assert_eq!(
        render(&td, first["id"].as_str().unwrap(), "cover")
            .await
            .state,
        JobState::Succeeded
    );
    upload(&td).await;
}

#[tokio::test]
async fn gif_upload_is_accepted_for_preparation() {
    let td = TestCompanion::start().await;
    let bytes = vec![
        71, 73, 70, 56, 57, 97, 1, 0, 1, 0, 128, 0, 0, 0, 0, 0, 255, 255, 255, 44, 0, 0, 0, 0, 1,
        0, 1, 0, 0, 2, 2, 68, 1, 0, 59,
    ];
    let response = td
        .raw()
        .post("/api/v1/uploads")
        .bearer_auth(&td.token)
        .header("content-type", "image/gif")
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["media_type"],
        "image/gif"
    );
}

#[tokio::test]
async fn gif_prepare_and_push_respects_page_budget() {
    let td = TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    td.unplug(&device).await;
    td.deck(&device).set_gif_capabilities(true);
    td.plug_device("mock0001").await;
    let bytes = vec![
        71, 73, 70, 56, 57, 97, 1, 0, 1, 0, 128, 0, 0, 0, 0, 0, 255, 255, 255, 44, 0, 0, 0, 0, 1,
        0, 1, 0, 0, 2, 2, 68, 1, 0, 59,
    ];
    let upload: Value = td
        .raw()
        .post("/api/v1/uploads")
        .bearer_auth(&td.token)
        .header("content-type", "image/gif")
        .body(bytes)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let body = json!({"source":{"kind":"gif","upload":upload["id"],"device":device,"target":{"width":128,"height":128,"background":[20,30,40]}},"fit":"contain"});
    let response = td
        .raw()
        .post("/api/v1/assets")
        .bearer_auth(&td.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let job = response.json().await.unwrap();
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    let result = done.result.unwrap();
    assert_eq!(result["kind"], "gif");
    let id = result["id"].as_str().unwrap();
    let make_layout = |count| {
        serde_json::from_value::<Layout>(json!({
        "schema":2,"name":"Animations","orientation":"landscape",
        "pages":[{"kind":"grid","id":"main","title":"Main","rows":1,"cols":5,
        "tiles":(0..count).map(|col| json!({"id":format!("t{col}"),"label":"GIF","row":0,"col":col,"rowSpan":1,"colSpan":1,"render":"gif","asset":format!("sha256:{id}"),"action":{"type":"none"}})).collect::<Vec<_>>() }]
    })).unwrap()
    };
    let path = || LayoutPath {
        device: device.to_string(),
        orientation: Orientation::Landscape,
    };
    let job = td
        .call::<Push>(path(), NoQuery, LayoutBody::new(&make_layout(5)))
        .await
        .unwrap();
    let done = td.wait(&job).await;
    assert_eq!(done.error.unwrap().code, "layout.animation_limit");
    let job = td
        .call::<Push>(path(), NoQuery, LayoutBody::new(&make_layout(1)))
        .await
        .unwrap();
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    assert_eq!(done.result.unwrap()["assets"]["sent"], json!([id]));
}

fn gif_bytes(side: u16, delays: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder =
            gif::Encoder::new(&mut bytes, side, side, &[0, 0, 0, 255, 0, 0, 0, 255, 0]).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for (i, &delay) in delays.iter().enumerate() {
            encoder
                .write_frame(&gif::Frame {
                    width: side,
                    height: side,
                    delay,
                    buffer: std::borrow::Cow::Owned(vec![
                        (i % 3) as u8;
                        usize::from(side) * usize::from(side)
                    ]),
                    ..Default::default()
                })
                .unwrap();
        }
    }
    bytes
}

async fn gif_device(td: &TestCompanion) -> String {
    let device = td.plug_device("mock0001").await;
    td.unplug(&device).await;
    td.deck(&device).set_gif_capabilities(true);
    td.plug_device("mock0001").await;
    device.to_string()
}

async fn gif_upload(td: &TestCompanion, bytes: Vec<u8>) -> String {
    let upload: Value = td
        .raw()
        .post("/api/v1/uploads")
        .bearer_auth(&td.token)
        .header("content-type", "image/gif")
        .body(bytes)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    upload["id"].as_str().unwrap().to_owned()
}

fn gif_request(upload: &str, device: &str, width: u16, height: u16) -> Value {
    json!({"source":{"kind":"gif","upload":upload,"device":device,"target":{"width":width,"height":height,"background":[0,0,0]}},"fit":"contain"})
}

async fn prepare_gif(td: &TestCompanion, device: &str, bytes: Vec<u8>) -> td_companion::jobs::Job {
    let upload = gif_upload(td, bytes).await;
    let response = td
        .raw()
        .post("/api/v1/assets")
        .bearer_auth(&td.token)
        .json(&gif_request(&upload, device, 64, 64))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    td.wait(&response.json().await.unwrap()).await
}

#[tokio::test]
async fn gif_preparation_errors_are_typed() {
    let td = TestCompanion::start().await;
    let device = gif_device(&td).await;
    let mut corrupt = gif_bytes(8, &[10]);
    corrupt.truncate(corrupt.len() - 3);
    let failed = prepare_gif(&td, &device, corrupt).await.error.unwrap();
    assert_eq!(failed.code, "asset.render_failed");
    assert_eq!(
        failed.message,
        "Couldn't prepare this GIF. Choose another GIF."
    );
    assert_eq!(failed.details, Some(json!({"reason":"corrupt"})));

    let short = prepare_gif(&td, &device, gif_bytes(8, &[2, 2]))
        .await
        .error
        .unwrap();
    assert_eq!(short.code, "asset.too_short");
    assert_eq!(
        short.message,
        "This GIF is too short. Choose a GIF that plays for at least 100 ms."
    );
    assert_eq!(
        short.details,
        Some(json!({"field":"duration_ms","limit":100,"actual":40,"unit":"ms"}))
    );

    let long = prepare_gif(&td, &device, gif_bytes(8, &[10; 61]))
        .await
        .error
        .unwrap();
    assert_eq!(long.code, "asset.too_large");
    assert_eq!(long.message, "This GIF is too long. Choose a shorter GIF.");
    assert_eq!(long.details.unwrap()["field"], "gif_frames");
}

#[tokio::test]
async fn gif_target_must_be_a_real_canvas() {
    let td = TestCompanion::start().await;
    let device = gif_device(&td).await;
    for (width, height) in [(0, 64), (64, 0), (4097, 64)] {
        let upload = gif_upload(&td, gif_bytes(8, &[10])).await;
        let response = td
            .raw()
            .post("/api/v1/assets")
            .bearer_auth(&td.token)
            .json(&gif_request(&upload, &device, width, height))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{width}x{height}");
        let body = &response.json::<Value>().await.unwrap()["error"];
        assert_eq!(body["code"], "request.invalid");
        assert_eq!(body["details"]["part"], "body");
    }
}

#[tokio::test]
async fn a_full_asset_store_refuses_with_a_typed_error() {
    let td = TestCompanion::start_with_asset_quota(td_host::icon_pack::FILE_BYTES as u64).await;
    let first = upload(&td).await;
    let done = render(&td, first["id"].as_str().unwrap(), "contain").await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 255, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let second: Value = td
        .raw()
        .post("/api/v1/uploads")
        .bearer_auth(&td.token)
        .header("content-type", "image/png")
        .body(png.into_inner())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let error = render(&td, second["id"].as_str().unwrap(), "contain")
        .await
        .error
        .unwrap();
    assert_eq!(error.code, "asset.store_full");
    let bytes = td_host::icon_pack::FILE_BYTES;
    assert_eq!(
        error.details,
        Some(json!({"limit":bytes,"actual":2 * bytes,"unit":"bytes"}))
    );
}

fn slow_gif() -> Vec<u8> {
    let side = 400u16;
    let mut seed = 0x2545_f491_u32;
    let mut bytes = Vec::new();
    {
        let palette = [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];
        let mut encoder = gif::Encoder::new(&mut bytes, side, side, &palette).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for _ in 0..100 {
            let buffer = (0..usize::from(side) * usize::from(side))
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed >> 30) as u8
                })
                .collect();
            encoder
                .write_frame(&gif::Frame {
                    width: side,
                    height: side,
                    delay: 6,
                    buffer: std::borrow::Cow::Owned(buffer),
                    ..Default::default()
                })
                .unwrap();
        }
    }
    bytes
}

#[tokio::test]
async fn a_cancelled_gif_render_frees_its_slot_at_once() {
    let td = TestCompanion::start().await;
    let device = gif_device(&td).await;
    let source = slow_gif();
    let mut running = Vec::new();
    for _ in 0..2 {
        let upload = gif_upload(&td, source.clone()).await;
        let job: td_companion::jobs::Job =
            serde_json::from_value(prepare_request(&td, &upload, &device).await).unwrap();
        running.push(job);
    }
    let third = gif_upload(&td, source).await;
    let refused = td
        .raw()
        .post("/api/v1/assets")
        .bearer_auth(&td.token)
        .json(&gif_request(&third, &device, 128, 128))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 429);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let cancelled_at = std::time::Instant::now();
    for job in &running {
        td.call::<Cancel>(
            JobPath {
                job: job.id.clone(),
            },
            NoQuery,
            (),
        )
        .await
        .unwrap();
    }
    for job in &running {
        assert_eq!(td.wait(job).await.state, JobState::Cancelled);
    }
    let freed = cancelled_at.elapsed();
    assert!(
        freed < std::time::Duration::from_secs(2),
        "slots freed after {freed:?}"
    );
    let accepted: td_companion::jobs::Job =
        serde_json::from_value(prepare_request(&td, &third, &device).await).unwrap();
    td.call::<Cancel>(
        JobPath {
            job: accepted.id.clone(),
        },
        NoQuery,
        (),
    )
    .await
    .unwrap();
    assert_eq!(td.wait(&accepted).await.state, JobState::Cancelled);
}

async fn prepare_request(td: &TestCompanion, upload: &str, device: &str) -> Value {
    let response = td
        .raw()
        .post("/api/v1/assets")
        .bearer_auth(&td.token)
        .json(&gif_request(upload, device, 128, 128))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    response.json().await.unwrap()
}
async fn job_state(td: &TestCompanion, job: &td_companion::jobs::Job) -> JobState {
    td.call::<td_companion::features::jobs::Get>(
        JobPath {
            job: job.id.clone(),
        },
        NoQuery,
        (),
    )
    .await
    .unwrap()
    .state
}

#[tokio::test]
async fn one_gif_prepares_at_a_time_and_the_next_waits_queued() {
    let td = TestCompanion::start().await;
    let device = gif_device(&td).await;
    let source = slow_gif();
    let mut jobs = Vec::new();
    for _ in 0..2 {
        let upload = gif_upload(&td, source.clone()).await;
        let job: td_companion::jobs::Job =
            serde_json::from_value(prepare_request(&td, &upload, &device).await).unwrap();
        jobs.push(job);
    }
    let (first, second) = (&jobs[0], &jobs[1]);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while job_state(&td, second).await != JobState::Queued {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the second GIF waits while the first prepares");
    assert_eq!(job_state(&td, first).await, JobState::Running);
    let cancel = |job: &td_companion::jobs::Job| {
        td.call::<Cancel>(
            JobPath {
                job: job.id.clone(),
            },
            NoQuery,
            (),
        )
    };
    cancel(first).await.unwrap();
    assert_eq!(td.wait(first).await.state, JobState::Cancelled);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while job_state(&td, second).await != JobState::Running {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the second GIF starts once the first ends");
    cancel(second).await.unwrap();
    assert_eq!(td.wait(second).await.state, JobState::Cancelled);
}

#[tokio::test]
async fn an_icon_renders_while_one_gif_prepares_and_another_waits() {
    let td = TestCompanion::start().await;
    let device = gif_device(&td).await;
    let source = slow_gif();
    let mut gifs = Vec::new();
    for _ in 0..2 {
        let up = gif_upload(&td, source.clone()).await;
        let job: td_companion::jobs::Job =
            serde_json::from_value(prepare_request(&td, &up, &device).await).unwrap();
        gifs.push(job);
    }
    let (first, second) = (&gifs[0], &gifs[1]);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while job_state(&td, second).await != JobState::Queued {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the second GIF waits while the first prepares");
    assert_eq!(job_state(&td, first).await, JobState::Running);

    td.hold_jobs();
    let icon = upload(&td).await;
    let icon_body: CreateAsset = serde_json::from_value(
        json!({"source": {"kind": "upload", "upload": icon["id"]}, "fit": "contain"}),
    )
    .unwrap();
    let icon_job = td.call::<Create>((), NoQuery, icon_body).await.unwrap();

    let extra_icon = upload(&td).await;
    let extra_refused = td
        .raw()
        .post("/api/v1/assets")
        .bearer_auth(&td.token)
        .json(&json!({"source": {"kind": "upload", "upload": extra_icon["id"]}, "fit": "contain"}))
        .send()
        .await
        .unwrap();
    assert_eq!(extra_refused.status(), 429);

    td.release_jobs();
    let icon_done = td.wait(&icon_job).await;
    assert_eq!(icon_done.state, JobState::Succeeded, "{icon_done:?}");
    assert_eq!(job_state(&td, first).await, JobState::Running);
    assert_eq!(job_state(&td, second).await, JobState::Queued);

    for job in &gifs {
        td.call::<Cancel>(
            JobPath {
                job: job.id.clone(),
            },
            NoQuery,
            (),
        )
        .await
        .unwrap();
        assert_eq!(td.wait(job).await.state, JobState::Cancelled);
    }
}

#[tokio::test]
async fn cancelling_a_waiting_gif_leaves_its_upload_usable_until_expiry() {
    let td = TestCompanion::start().await;
    let _events = td.events().await;
    let device = gif_device(&td).await;
    let slow = slow_gif();

    let first_up = gif_upload(&td, slow.clone()).await;
    let first: td_companion::jobs::Job =
        serde_json::from_value(prepare_request(&td, &first_up, &device).await).unwrap();
    let reused_up = gif_upload(&td, gif_bytes(8, &[10])).await;
    let waiting: td_companion::jobs::Job =
        serde_json::from_value(prepare_request(&td, &reused_up, &device).await).unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while job_state(&td, &waiting).await != JobState::Queued {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the second GIF waits while the first prepares");

    assert_eq!(
        render(&td, &reused_up, "contain").await.error.unwrap().code,
        "upload.not_found"
    );

    let cancel = |job: &td_companion::jobs::Job| {
        td.call::<Cancel>(
            JobPath {
                job: job.id.clone(),
            },
            NoQuery,
            (),
        )
    };
    cancel(&waiting).await.unwrap();
    assert_eq!(td.wait(&waiting).await.state, JobState::Cancelled);
    cancel(&first).await.unwrap();
    assert_eq!(td.wait(&first).await.state, JobState::Cancelled);

    let retried: td_companion::jobs::Job =
        serde_json::from_value(prepare_request(&td, &reused_up, &device).await).unwrap();
    let done = td.wait(&retried).await;
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");

    let consumed: td_companion::jobs::Job =
        serde_json::from_value(prepare_request(&td, &reused_up, &device).await).unwrap();
    assert_eq!(
        td.wait(&consumed).await.error.unwrap().code,
        "upload.not_found"
    );

    let blocker_up = gif_upload(&td, slow).await;
    let blocker: td_companion::jobs::Job =
        serde_json::from_value(prepare_request(&td, &blocker_up, &device).await).unwrap();
    let expiring_up = gif_upload(&td, gif_bytes(8, &[10])).await;
    let expiring: td_companion::jobs::Job =
        serde_json::from_value(prepare_request(&td, &expiring_up, &device).await).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while job_state(&td, &expiring).await != JobState::Queued {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("waiting GIF enters queued");
    cancel(&expiring).await.unwrap();
    assert_eq!(td.wait(&expiring).await.state, JobState::Cancelled);
    cancel(&blocker).await.unwrap();
    assert_eq!(td.wait(&blocker).await.state, JobState::Cancelled);

    td.clock().advance(std::time::Duration::from_secs(3601));
    let expired: td_companion::jobs::Job =
        serde_json::from_value(prepare_request(&td, &expiring_up, &device).await).unwrap();
    assert_eq!(
        td.wait(&expired).await.error.unwrap().code,
        "upload.not_found"
    );
}
