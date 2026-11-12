use futures_util::StreamExt;
use serde_json::{Value, json};
use td_companion::{
    api::NoQuery,
    features::{
        jobs::{Cancel, JobPath, List},
        layouts::{Get, LastSeen, LayoutBody, LayoutPath, Push, PushReport},
    },
    jobs::{Job, JobState},
    testing::TestCompanion,
};
use td_core::{Layout, Orientation};

fn layout(name: &str, orientation: Orientation, icon: Option<&str>) -> Layout {
    serde_json::from_value(json!({
        "schema": 1, "name": name, "orientation": orientation,
        "pages": [{"id": "main", "title": "Main", "kind": "grid", "rows": 2, "cols": 2, "tiles": [
            {"id": "a", "label": "A", "row": 0, "col": 0, "rowSpan": 1, "colSpan": 1,
             "icon": icon, "action": {"type": "none"}}
        ]}]
    }))
    .unwrap()
}
fn path(device: &str, orientation: Orientation) -> LayoutPath {
    LayoutPath {
        device: device.into(),
        orientation,
    }
}
async fn push(td: &TestCompanion, device: &str, layout: &Layout) -> Job {
    td.handle()
        .layout_operations()
        .push(
            device.into(),
            layout.orientation,
            layout.to_canonical_json(),
            None,
        )
        .await
        .unwrap()
}
/// The status and error body of a request that fails.
async fn failure(request: reqwest::RequestBuilder) -> (u16, Value) {
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    (
        status,
        response.json::<Value>().await.unwrap()["error"].clone(),
    )
}
fn report(job: &Job) -> PushReport {
    assert_eq!(job.state, JobState::Succeeded, "{job:?}");
    serde_json::from_value(job.result.clone().unwrap()).unwrap()
}

#[tokio::test]
async fn two_devices_keep_their_own_layouts() {
    let td = TestCompanion::start().await;
    let a = td.plug_device("mock0001").await;
    let b = td.plug_device("mock0002").await;
    let first = layout("First", Orientation::Landscape, None);
    let second = layout("Second", Orientation::Landscape, None);
    let upright = layout("Upright", Orientation::Portrait, None);
    for (device, layout) in [(&a, &first), (&b, &second), (&a, &upright)] {
        let job = push(&td, device, layout).await;
        assert_eq!(job.kind, "layouts.push");
        let pushed = report(&td.wait(&job).await);
        let crc = td_link::xfer::crc32(&layout.to_canonical_json());
        assert_eq!(pushed.revision, format!("{crc:08x}"));
        assert_eq!(pushed.bytes, layout.to_canonical_json().len() as u64);
    }
    for (device, orientation, expected) in [
        (&a, Orientation::Landscape, &first),
        (&b, Orientation::Landscape, &second),
        (&a, Orientation::Portrait, &upright),
    ] {
        let response = td
            .request::<Get>(path(device, orientation), NoQuery, &())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let etag = response.headers()["etag"].to_str().unwrap().to_owned();
        let read: Value = response.json().await.unwrap();
        let stored = td.deck(device).layout(orientation).unwrap();
        let revision = format!("{:08x}", td_link::xfer::crc32(&stored));
        assert_eq!(read["revision"], revision);
        assert_eq!(etag, format!("\"{revision}\""));
        assert_eq!(
            serde_json::from_value::<Layout>(read["layout"].clone()).unwrap(),
            *expected
        );
    }
    // The second Device never had a portrait Layout.
    let (status, error) =
        failure(td.request::<Get>(path(&b, Orientation::Portrait), NoQuery, &())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("layout.not_found"))
    );
}

#[tokio::test]
async fn an_offline_device_refuses_a_layout_read_even_with_a_copy_on_this_pc() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let desk = layout("Desk", Orientation::Landscape, None);
    report(&td.wait(&push(&td, id.as_str(), &desk).await).await);
    td.unplug("mock0001").await;
    let (status, error) =
        failure(td.request::<Get>(path(id.as_str(), Orientation::Landscape), NoQuery, &())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (409, Some("device.not_connected"))
    );
    assert_eq!(error["details"]["link"]["state"], "lost");
}

#[tokio::test]
async fn an_offline_device_shows_the_layout_this_pc_last_saw() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let desk = layout("Desk", Orientation::Landscape, None);
    report(&td.wait(&push(&td, id.as_str(), &desk).await).await);
    td.unplug("mock0001").await;

    let seen = td
        .handle()
        .layout_operations()
        .last_seen((id.as_str()).to_string(), Orientation::Landscape)
        .await
        .unwrap();
    assert_eq!(seen.layout, desk);
    // No revision, so nobody can push against this copy with `If-Match`.
    let body: Value = td
        .request::<LastSeen>(path(id.as_str(), Orientation::Landscape), NoQuery, &())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["layout"]
    );

    // No copy of the other orientation was ever seen.
    let (status, error) =
        failure(td.request::<LastSeen>(path(id.as_str(), Orientation::Portrait), NoQuery, &()))
            .await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("layout.not_found"))
    );

    let (status, error) =
        failure(td.request::<LastSeen>(path("mock9999", Orientation::Landscape), NoQuery, &()))
            .await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("device.not_found"))
    );
}

#[tokio::test]
async fn revisions_orientation_and_validation_are_checked() {
    let td = TestCompanion::start().await;
    let a = td.plug_device("mock0001").await;
    let first = layout("First", Orientation::Landscape, None);
    td.wait(&push(&td, &a, &first).await).await;
    let read = td
        .handle()
        .layout_operations()
        .read(a.to_string(), Orientation::Landscape)
        .await
        .unwrap();
    let put = |layout: &Layout, if_match: &str| {
        td.request::<Push>(
            path(&a, layout.orientation),
            NoQuery,
            &LayoutBody::new(layout),
        )
        .header("if-match", if_match)
    };

    // A stale revision is refused before any Job starts.
    let second = layout("Second", Orientation::Landscape, None);
    let (status, error) = failure(put(&second, "\"00000000\"")).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (412, Some("layout.changed"))
    );
    assert_eq!(error["details"]["revision"], read.revision.as_str());
    assert!(td.call::<List>((), NoQuery, ()).await.unwrap().len() == 1);

    // The current revision passes, and then it's stale.
    let response = put(&second, &format!("\"{}\"", read.revision))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let pushed = report(&td.wait(&response.json().await.unwrap()).await);
    assert_ne!(pushed.revision, read.revision);
    let (status, _) = failure(put(&first, &read.revision)).await;
    assert_eq!(status, 412);
    let response = put(&first, &pushed.revision).send().await.unwrap();
    assert_eq!(response.status(), 202);

    // The path and the body must name the same orientation.
    let raw = |orientation: &str, body: Value| {
        td.raw()
            .request(
                reqwest::Method::PUT,
                &format!("/api/v1/devices/{a}/layouts/{orientation}"),
            )
            .bearer_auth(&td.token)
            .json(&body)
    };
    let landscape = serde_json::to_value(&first).unwrap();
    let (status, error) = failure(raw("portrait", landscape.clone())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (422, Some("layout.orientation_mismatch"))
    );

    // A Layout that fails validation lists its issues.
    let mut overlapping = landscape.clone();
    let mut tile = overlapping["pages"][0]["tiles"][0].clone();
    tile["id"] = json!("b");
    overlapping["pages"][0]["tiles"]
        .as_array_mut()
        .unwrap()
        .push(tile);
    let (status, error) = failure(raw("landscape", overlapping)).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (422, Some("layout.invalid"))
    );
    let issues = error["details"]["issues"].as_array().unwrap();
    assert!(
        issues.iter().any(|i| i["rule"] == "tile.overlap"),
        "{issues:?}"
    );
    let mut unknown = landscape;
    unknown["colour"] = json!("red");
    let (status, error) = failure(raw("landscape", unknown)).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (422, Some("layout.invalid"))
    );
    assert_eq!(error["details"]["issues"][0]["rule"], "shape.unknown-field");
    let mut empty = serde_json::to_value(&first).unwrap();
    empty["pages"][0]["tiles"][0]["action"] = json!({"type": "open", "target": ""});
    let (status, error) = failure(raw("landscape", empty)).await;
    assert_eq!(status, 422);
    assert_eq!(
        (
            &error["details"]["issues"][0]["rule"],
            &error["details"]["issues"][0]["path"]
        ),
        (
            &json!("text.empty"),
            &json!("/pages/0/tiles/0/action/target")
        )
    );

    // An unknown Device.
    let (status, error) =
        failure(td.request::<Get>(path("mock0009", Orientation::Landscape), NoQuery, &())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("device.not_found"))
    );
}

#[tokio::test]
async fn a_second_push_gets_the_busy_job() {
    let td = TestCompanion::start().await;
    let a = td.plug_device("mock0001").await;
    let b = td.plug_device("mock0002").await;
    let mut events = td.events().await;
    td.hold_jobs();
    let first = push(&td, &a, &layout("First", Orientation::Landscape, None)).await;
    let error = td
        .handle()
        .layout_operations()
        .push(
            a.to_string(),
            Orientation::Landscape,
            layout("Second", Orientation::Landscape, None).to_canonical_json(),
            None,
        )
        .await
        .err()
        .unwrap();
    let td_companion::features::layouts::LayoutError::Job(td_companion::jobs::JobError::Busy(job)) =
        error
    else {
        panic!("expected busy Job");
    };
    assert_eq!(job.id, first.id);
    // A Job on one Device doesn't block another.
    let other = push(&td, &b, &layout("Other", Orientation::Landscape, None)).await;
    td.release_jobs();
    let done = td.wait(&first).await;
    assert_eq!(report(&done).state.name, "First");
    report(&td.wait(&other).await);
    assert_eq!(
        td.deck(&a).layout(Orientation::Landscape).unwrap(),
        layout("First", Orientation::Landscape, None).to_canonical_json()
    );

    // `job.updated` carries the full Job with a growing `rev`, ending in `succeeded`.
    let mut revs = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let frame: Value =
                serde_json::from_str(events.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            if frame["type"] == "job.updated" && frame["data"]["id"] == first.id.as_str() {
                assert_eq!(frame["device"], a.as_str());
                revs.push(frame["data"]["rev"].as_u64().unwrap());
                if frame["data"]["state"] == "succeeded" {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(revs.windows(2).all(|w| w[0] < w[1]), "{revs:?}");
    assert_eq!(*revs.last().unwrap(), done.rev);
}

#[tokio::test]
async fn a_cancelled_push_leaves_the_device_alone() {
    let td = TestCompanion::start().await;
    let a = td.plug_device("mock0001").await;
    td.hold_jobs();
    let job = push(&td, &a, &layout("First", Orientation::Landscape, None)).await;
    let job_path = || JobPath {
        job: job.id.clone(),
    };
    let cancelled = td.call::<Cancel>(job_path(), NoQuery, ()).await.unwrap();
    assert_eq!(cancelled.id, job.id);
    td.release_jobs();
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Cancelled);
    assert_eq!(done.error.unwrap().code, "job.cancelled");
    assert_eq!(td.deck(&a).layout(Orientation::Landscape), None);
    let (status, error) = failure(td.request::<Cancel>(job_path(), NoQuery, &())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (409, Some("job.not_cancellable"))
    );
    let missing = JobPath {
        job: "job_missing".into(),
    };
    let (status, error) = failure(td.request::<Cancel>(missing, NoQuery, &())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("job.not_found"))
    );
}

#[tokio::test]
async fn a_push_sends_the_assets_the_device_lacks() {
    let td = TestCompanion::start().await;
    let a = td.plug_device("mock0001").await;
    let builtin = td_host::icons::builtin()[0].id.clone();
    let with_icon = layout(
        "Icons",
        Orientation::Landscape,
        Some(&format!("file:{builtin}")),
    );
    let pushed = report(&td.wait(&push(&td, &a, &with_icon).await).await);
    assert_eq!(pushed.assets.sent, std::slice::from_ref(&builtin));
    assert!(td.deck(&a).icons().contains(&builtin));
    // A second push sends nothing, and an id with no file is reported.
    let pushed = report(&td.wait(&push(&td, &a, &with_icon).await).await);
    assert!(pushed.assets.sent.is_empty());
    let missing = layout(
        "Missing",
        Orientation::Landscape,
        Some("file:0123456789abcdef"),
    );
    let pushed = report(&td.wait(&push(&td, &a, &missing).await).await);
    assert_eq!(pushed.assets.unavailable, ["0123456789abcdef"]);
}

#[tokio::test]
async fn image_budget_is_checked_before_starting_a_push() {
    let td = TestCompanion::start().await;
    let device = td.plug_device("mock0001").await;
    let mut value = serde_json::to_value(layout("Media", Orientation::Landscape, None)).unwrap();
    value["schema"] = json!(2);
    let tile = &mut value["pages"][0]["tiles"][0];
    tile["render"] = json!("now-playing");
    tile["action"] = json!({"type":"widget","source":"media","account":"","field":"title"});
    tile["streams"] = json!({"title":"text","artist":"text","state":"number","cover":"image"});
    let layout = Layout::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let (status, error) = failure(td.request::<Push>(
        path(device.as_str(), Orientation::Landscape),
        NoQuery,
        &LayoutBody::new(&layout),
    ))
    .await;
    assert_eq!(
        (status, error["code"].as_str()),
        (422, Some("layout.live_image_limit"))
    );
    assert_eq!(error["details"], json!({"limit":0,"actual":1}));
    assert!(
        td.deck(device.as_str())
            .layout(Orientation::Landscape)
            .is_none()
    );
}

#[tokio::test]
async fn control_layout_get_sync_and_updates_agree_after_push_and_reconnect() {
    use td_companion::devices::ControlLayout;
    use td_companion::features::devices::{DevicePath, Get as DeviceGet, List as Devices};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let td = TestCompanion::start().await;
    let id = td.plug_media_device("mock0001").await;
    td.until(|td| {
        matches!(
            td.device(&id).unwrap().control_layout,
            ControlLayout::NoPushedCopy {
                has_controls: false,
                ..
            }
        )
    })
    .await;
    let mut events = td.events().await;
    let layout = Layout::parse(include_bytes!("../../../../schema/examples/media.json")).unwrap();
    td.deck(&id).corrupt_next_put();
    let failed = push(&td, &id, &layout).await;
    assert_eq!(td.wait(&failed).await.state, JobState::Failed);
    td.until(|td| {
        matches!(
            td.device(&id).unwrap().control_layout,
            ControlLayout::NoPushedCopy {
                has_controls: false,
                ..
            }
        )
    })
    .await;
    let job = push(&td, &id, &layout).await;
    assert_eq!(td.wait(&job).await.state, JobState::Succeeded);
    let updated = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let message = events.next().await.unwrap().unwrap();
            let frame: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            if frame["type"] == "device.updated"
                && frame["data"]["control_layout"]["state"] == "matches"
            {
                break frame["data"].clone();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(updated["control_layout"]["has_controls"], true);
    let get = td
        .call::<DeviceGet>(DevicePath { device: id.clone() }, NoQuery, ())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&get).unwrap(), updated);
    assert_eq!(td.call::<Devices>((), NoQuery, ()).await.unwrap()[0], get);
    let mut request = format!("{}/api/v1/events", td.url.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("touchdeck.v1, touchdeck.bearer.{}", td.token)
            .parse()
            .unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket.next().await.unwrap().unwrap();
    let sync = socket.next().await.unwrap().unwrap();
    let sync: Value = serde_json::from_str(sync.to_text().unwrap()).unwrap();
    assert_eq!(sync["data"]["devices"][0], updated);
    td.unplug("mock0001").await;
    assert!(matches!(
        td.device(&id).unwrap().control_layout,
        ControlLayout::Unavailable { .. }
    ));
    td.plug_media_device("mock0001").await;
    td.until(|td| {
        matches!(
            td.device(&id).unwrap().control_layout,
            ControlLayout::Matches { .. }
        )
    })
    .await;
}

async fn states_until_matches(
    events: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    device: &str,
) -> Vec<String> {
    let mut states: Vec<String> = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let message = events.next().await.unwrap().unwrap();
            let frame: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            if frame["type"] != "device.updated" || frame["device"] != device {
                continue;
            }
            let status = &frame["data"]["control_layout"];
            let state = status["reason"]
                .as_str()
                .or(status["state"].as_str())
                .unwrap()
                .to_owned();
            if (states.is_empty() && state != "checking") || states.last() == Some(&state) {
                continue;
            }
            states.push(state);
            if states.last().unwrap() == "matches" {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no matches after {states:?}"));
    states
}

#[tokio::test]
async fn a_push_publishes_only_checking_then_matches() {
    use td_companion::checkpoints::Checkpoint;
    use td_companion::devices::ControlLayout;
    let td = TestCompanion::start().await;
    let id = td.plug_media_device("mock0001").await;
    td.until(|td| {
        matches!(
            td.device(&id).unwrap().control_layout,
            ControlLayout::NoPushedCopy { .. }
        )
    })
    .await;
    let mut events = td.events().await;
    td.checkpoints().hold(Checkpoint::LayoutPut);
    let layout = Layout::parse(include_bytes!("../../../../schema/examples/media.json")).unwrap();
    let job = push(&td, &id, &layout).await;
    td.until(|td| td.checkpoints().waiting(Checkpoint::LayoutPut))
        .await;
    td.let_live_rounds_run(3).await;
    td.checkpoints().release(Checkpoint::LayoutPut, false);
    assert_eq!(td.wait(&job).await.state, JobState::Succeeded);
    assert_eq!(
        states_until_matches(&mut events, id.as_str()).await,
        ["checking", "matches"]
    );
}

async fn matching_device_without_samples(
    td: &TestCompanion,
) -> (td_companion::devices::DeviceId, Layout) {
    use td_companion::devices::ControlLayout;
    let id = td.plug_media_device("mock0001").await;
    let mut unsampled: Value =
        serde_json::from_slice(include_bytes!("../../../../schema/examples/media.json")).unwrap();
    for tile in unsampled["pages"][0]["tiles"].as_array_mut().unwrap() {
        tile["action"]["account"] = json!("no-samples");
    }
    let layout: Layout = serde_json::from_value(unsampled).unwrap();
    assert_eq!(
        td.wait(&push(td, &id, &layout).await).await.state,
        JobState::Succeeded
    );
    td.until(|td| {
        matches!(
            td.device(&id).unwrap().control_layout,
            ControlLayout::Matches { .. }
        )
    })
    .await;
    let mut other = serde_json::to_value(&layout).unwrap();
    other["pages"][0]["tiles"][2]["action"]["account"] = json!("other-account");
    (id, serde_json::from_value(other).unwrap())
}

async fn differs_within(
    td: &TestCompanion,
    id: &td_companion::devices::DeviceId,
    within: std::time::Duration,
) -> bool {
    use td_companion::devices::ControlLayout;
    tokio::time::timeout(within, async {
        while !matches!(
            td.device(id).unwrap().control_layout,
            ControlLayout::Differs { .. }
        ) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

fn to_portrait() -> td_companion::features::devices::DeviceSettingsPatch {
    td_companion::features::devices::DeviceSettingsPatch {
        orientation: Some("portrait".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_round_during_an_orientation_change_leaves_no_stale_layout() {
    use td_companion::checkpoints::Checkpoint;
    use td_companion::devices::ControlLayout;
    use td_companion::features::devices::{DevicePath, SettingsPatch};
    let td = TestCompanion::start().await;
    let (id, other) = matching_device_without_samples(&td).await;
    td.checkpoints().hold(Checkpoint::BeforeOrientationPut);
    td.checkpoints().hold(Checkpoint::AfterOrientationPut);
    let patching =
        td.call::<SettingsPatch>(DevicePath { device: id.clone() }, NoQuery, to_portrait());
    let steering = async {
        td.until(|td| td.checkpoints().waiting(Checkpoint::BeforeOrientationPut))
            .await;
        td.until(|td| {
            matches!(
                td.device(&id).unwrap().control_layout,
                ControlLayout::Matches { .. }
            )
        })
        .await;
        td.write_layout_unseen(&id, &other).await;
        td.checkpoints()
            .release(Checkpoint::BeforeOrientationPut, false);
        td.until(|td| td.checkpoints().waiting(Checkpoint::AfterOrientationPut))
            .await;
        let reported = differs_within(&td, &id, std::time::Duration::from_secs(2)).await;
        td.checkpoints()
            .release(Checkpoint::AfterOrientationPut, false);
        reported
    };
    let (patched, reported) = tokio::join!(patching, steering);
    assert!(patched.unwrap().restart_required);
    assert!(reported, "the Layout read before the put stayed cached");
}

#[tokio::test]
async fn an_orientation_change_that_fails_after_the_put_still_reads_the_layout_again() {
    use td_companion::checkpoints::Checkpoint;
    use td_companion::devices::ControlLayout;
    use td_companion::features::devices::{DevicePath, SettingsPatch};
    let td = TestCompanion::start().await;
    let (id, other) = matching_device_without_samples(&td).await;
    td.checkpoints().hold(Checkpoint::BeforeOrientationPut);
    let patching =
        td.call::<SettingsPatch>(DevicePath { device: id.clone() }, NoQuery, to_portrait());
    let steering = async {
        td.until(|td| td.checkpoints().waiting(Checkpoint::BeforeOrientationPut))
            .await;
        td.until(|td| {
            matches!(
                td.device(&id).unwrap().control_layout,
                ControlLayout::Matches { .. }
            )
        })
        .await;
        td.write_layout_unseen(&id, &other).await;
        td.checkpoints()
            .release(Checkpoint::AfterOrientationPut, true);
        td.checkpoints()
            .release(Checkpoint::BeforeOrientationPut, false);
    };
    let (patched, ()) = tokio::join!(patching, steering);
    assert!(patched.is_err());
    assert_eq!(
        td.deck(id.as_str()).settings().orientation,
        Orientation::Portrait
    );
    assert!(
        differs_within(&td, &id, std::time::Duration::from_secs(2)).await,
        "the Layout read before the put stayed cached"
    );
}

#[tokio::test]
async fn control_layout_reports_unsupported_firmware_and_recovers_after_an_abandoned_read() {
    use td_companion::devices::{ControlLayout, ControlLayoutUnavailable};
    let unavailable = |reason| ControlLayout::Unavailable { reason };
    let td = TestCompanion::start().await;
    let old = td.plug_device_v1("mock0002").await;
    td.until(|td| {
        td.device(&old).unwrap().control_layout
            == unavailable(ControlLayoutUnavailable::Unsupported)
    })
    .await;
    let id = td.plug_media_device("mock0001").await;
    let layout = Layout::parse(include_bytes!("../../../../schema/examples/media.json")).unwrap();
    let hellos = || {
        td.deck(id.as_str())
            .received()
            .iter()
            .filter(|r| r["m"] == "hello")
            .count()
    };
    let before = hellos();
    td.deck(id.as_str()).lose_sent_chunks([0; 5]);
    assert_eq!(
        td.wait(&push(&td, &id, &layout).await).await.state,
        JobState::Succeeded
    );
    // An abandoned read closes its session; reconnect reads the committed Layout again.
    td.until(|td| {
        hellos() > before
            && matches!(
                td.device(&id).unwrap().control_layout,
                ControlLayout::Matches { .. }
            )
    })
    .await;
    assert!(
        hellos() > before,
        "the abandoned read required a fresh session"
    );
}
