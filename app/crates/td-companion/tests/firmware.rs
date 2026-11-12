use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use td_companion::{
    api::NoQuery, devices::FirmwareChannel, features::firmware::*, jobs::JobState,
    testing::TestCompanion,
};
#[derive(Default)]
struct Prompt {
    answer: Mutex<Option<bool>>,
    wake: Condvar,
    seen: Mutex<Vec<Confirmation>>,
    closed: AtomicBool,
}
impl Prompt {
    fn answer(&self, answer: bool) {
        *self.answer.lock().unwrap() = Some(answer);
        self.wake.notify_all();
    }
}
impl Prompter for Prompt {
    fn pick(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(
            include_bytes!("../../td-link/tests/fixtures/touchdeck-test.tdfw").to_vec(),
        ))
    }
    fn confirm(&self, c: &Confirmation, closed: &(dyn Fn() -> bool + Sync)) -> bool {
        self.seen.lock().unwrap().push(c.clone());
        let started = Instant::now();
        let mut a = self.answer.lock().unwrap();
        while a.is_none() {
            if closed() {
                self.closed.store(true, Ordering::SeqCst);
                return false;
            }
            if started.elapsed() > Duration::from_secs(10) {
                return false;
            }
            a = self
                .wake
                .wait_timeout(a, Duration::from_millis(10))
                .unwrap()
                .0;
        }
        a.take().unwrap()
    }
}
/// Waits for the Job to end, and says why when it didn't succeed.
async fn succeeded(td: &TestCompanion, job: &td_companion::jobs::Job) {
    let done = td.wait(job).await;
    assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
}

fn request() -> FirmwareRequest {
    FirmwareRequest {
        bundle: "bundled".into(),
        channel: None,
    }
}
#[tokio::test]
async fn consent_consumes_selection_and_decline_writes_nothing() {
    let td = TestCompanion::start().await;
    let a = td.plug_device("mocka").await;
    let b = td.plug_device("mockb").await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());
    let info = td
        .call::<Pick>((), NoQuery, FirmwareTarget { device: a.clone() })
        .await
        .unwrap()
        .bundle
        .unwrap();
    let wrong = td
        .request::<Update>(
            FirmwareTarget { device: b },
            NoQuery,
            &FirmwareRequest {
                bundle: info.id.clone(),
                channel: None,
            },
        )
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 404);
    let job = td
        .call::<Repair>(
            FirmwareTarget { device: a.clone() },
            NoQuery,
            FirmwareRequest {
                bundle: info.id.clone(),
                channel: None,
            },
        )
        .await
        .unwrap();
    td.until(|_| !prompt.seen.lock().unwrap().is_empty()).await;
    assert_eq!(td.deck("mocka").reboots(), 0);
    {
        let seen = prompt.seen.lock().unwrap();
        assert_eq!(seen[0].target, "mocka");
        assert_eq!(seen[0].version, info.version);
        assert_eq!(seen[0].build, info.build);
        assert!(!seen[0].port.is_empty());
        assert_eq!(seen[0].current, "mock");
    }
    let reused = td
        .request::<Repair>(
            FirmwareTarget { device: a },
            NoQuery,
            &FirmwareRequest {
                bundle: info.id,
                channel: None,
            },
        )
        .send()
        .await
        .unwrap();
    assert_eq!(reused.status(), 404);
    prompt.answer(false);
    let done = td.wait(&job).await;
    assert_eq!(
        td.deck("mocka").reboots(),
        0,
        "a declined repair wrote Firmware"
    );
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, "firmware.declined");
}
#[tokio::test]
async fn update_reports_success_and_rollback() {
    for rollback in [false, true] {
        let td = TestCompanion::start().await;
        let id = td.plug_device("mocka").await;
        let prompt = Arc::new(Prompt::default());
        prompt.answer(true);
        td.firmware_prompter(prompt);
        if rollback {
            td.deck("mocka").update_mode("rollback");
        }
        let job = td
            .call::<Update>(FirmwareTarget { device: id }, NoQuery, request())
            .await
            .unwrap();
        let done = td.wait(&job).await;
        if rollback {
            assert_eq!(done.state, JobState::Failed);
            assert_eq!(done.error.unwrap().code, "firmware.rolled_back");
        } else {
            assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
            let result: FirmwareResult = serde_json::from_value(done.result.unwrap()).unwrap();
            assert_eq!(result.version, "test-only");
        }
    }
}
#[tokio::test]
async fn picked_bundle_expires() {
    let td = TestCompanion::start().await;
    let _events = td.events().await;
    let id = td.plug_device("mocka").await;
    td.firmware_prompter(Arc::new(Prompt::default()));
    let info = td
        .call::<Pick>((), NoQuery, FirmwareTarget { device: id.clone() })
        .await
        .unwrap()
        .bundle
        .unwrap();
    td.clock().advance(std::time::Duration::from_secs(601));
    let response = td
        .request::<Update>(
            FirmwareTarget { device: id },
            NoQuery,
            &FirmwareRequest {
                bundle: info.id,
                channel: None,
            },
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn new_board_install_requires_consent_and_needs_setup() {
    use td_companion::devices::{BoardKind, SetupState};
    let td = TestCompanion::start().await;
    let mut events = td.events().await;
    let board = td.plug_board("newboard", BoardKind::OtherFirmware).await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());
    let job = td
        .call::<Install>(
            FirmwareTarget {
                device: board.id.clone(),
            },
            NoQuery,
            request(),
        )
        .await
        .unwrap();
    td.until(|_| !prompt.seen.lock().unwrap().is_empty()).await;
    assert_eq!(td.deck("newboard").reboots(), 0);
    prompt.answer(true);
    succeeded(&td, &job).await;
    td.until(|td| {
        td.device(&board.id)
            .is_some_and(|d| d.setup == SetupState::NeedsSetup)
    })
    .await;
    use futures_util::StreamExt;
    let expected_bytes: usize = td_link::bundle::Bundle::parse(include_bytes!(
        "../../td-link/tests/fixtures/touchdeck-test.tdfw"
    ))
    .unwrap()
    .images
    .iter()
    .map(|image| image.data.len())
    .sum();
    let mut recovered = false;
    let mut added = false;
    let mut removed = false;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(Ok(message)) = events.next().await {
            let Ok(text) = message.to_text() else {
                continue;
            };
            let frame: serde_json::Value = serde_json::from_str(text).unwrap();
            if frame["type"] == "job.updated"
                && frame["data"]["id"] == job.id
                && frame["data"]["progress"]["phase"] == "recovering"
            {
                assert_eq!(frame["data"]["progress"]["bytes"], expected_bytes);
                recovered = true;
            }
            if frame["type"] == "new-board.added" {
                assert_eq!(frame["data"]["id"], board.id.as_str());
                added = true;
            }
            if frame["type"] == "new-board.removed" {
                assert!(added);
                removed = true;
            }
            if frame["type"] == "device.added" {
                assert!(removed);
                assert!(recovered);
                assert_eq!(frame["data"]["id"], board.id.as_str());
                assert_eq!(frame["data"]["setup"], "needs-setup");
                break;
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn install_lock_is_owner_bound_and_expires() {
    use td_companion::features::system::{InstallLock, Lock, LockRenew, Unlock};
    let td = TestCompanion::start().await;
    let _socket = td.events().await;
    let lock = td.call::<Lock>((), NoQuery, ()).await.unwrap();
    assert!(lock.lock.starts_with("lock_"), "{}", lock.lock);
    let other: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/sessions", td.url))
        .bearer_auth(&td.pairing_token)
        .json(&serde_json::json!({"kind":"window"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for method in [reqwest::Method::PUT, reqwest::Method::DELETE] {
        let response = reqwest::Client::new()
            .request(
                method,
                format!("{}/api/v1/system/lock/{}", td.url, lock.lock),
            )
            .bearer_auth(other["token"].as_str().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
    }
    td.clock().advance(std::time::Duration::from_secs(500));
    td.call::<LockRenew>(
        InstallLock {
            lock: lock.lock.clone(),
        },
        NoQuery,
        (),
    )
    .await
    .unwrap();
    td.clock().advance(std::time::Duration::from_secs(101));
    assert_eq!(
        td.request::<Lock>((), NoQuery, &())
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    td.call::<Unlock>(lock, NoQuery, ()).await.unwrap();
    td.call::<Lock>((), NoQuery, ()).await.unwrap();
    td.clock().advance(std::time::Duration::from_secs(601));
    td.call::<Lock>((), NoQuery, ()).await.unwrap();
}

#[tokio::test]
async fn confirmation_holds_only_its_target_and_reports_busy_job() {
    use td_companion::{
        devices::LinkState,
        features::layouts::{LayoutBody, LayoutPath, Push},
    };
    let td = TestCompanion::start().await;
    let a = td.plug_device("mocka").await;
    let b = td.plug_device("mockb").await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());
    let job = td
        .call::<Repair>(FirmwareTarget { device: a.clone() }, NoQuery, request())
        .await
        .unwrap();
    td.until(|_| !prompt.seen.lock().unwrap().is_empty()).await;
    assert!(
        matches!(td.device(&a).unwrap().link, LinkState::Updating { job: ref id } if id == &job.id)
    );
    let busy = td
        .request::<Update>(FirmwareTarget { device: a }, NoQuery, &request())
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), 409);
    let body: serde_json::Value = busy.json().await.unwrap();
    assert_eq!(body["error"]["code"], "device.busy");
    assert_eq!(body["error"]["details"]["job"]["id"], job.id);
    let layout = td_core::Layout::parse(br#"{"schema":1,"name":"Other Device","orientation":"landscape","pages":[{"id":"main","title":"Main","kind":"grid","rows":1,"cols":1,"tiles":[{"id":"a","label":"A","row":0,"col":0,"rowSpan":1,"colSpan":1,"icon":null,"action":{"type":"none"}}]}]}"#).unwrap();
    let push = td
        .call::<Push>(
            LayoutPath {
                device: b.to_string(),
                orientation: td_core::Orientation::Landscape,
            },
            NoQuery,
            LayoutBody::new(&layout),
        )
        .await
        .unwrap();
    succeeded(&td, &push).await;
    prompt.answer(false);
    assert_eq!(td.wait(&job).await.error.unwrap().code, "firmware.declined");
}

#[tokio::test]
async fn same_or_newer_bundle_needs_no_prompt_and_emits_progress() {
    use futures_util::StreamExt;
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    td.deck("mocka").flash_bundle(
        &td_link::bundle::Bundle::parse(include_bytes!(
            "../../td-link/tests/fixtures/touchdeck-test-b.tdfw"
        ))
        .unwrap(),
    );
    td.call::<td_companion::features::devices::Restart>(
        td_companion::features::devices::DevicePath { device: id.clone() },
        NoQuery,
        (),
    )
    .await
    .unwrap();
    td.until(|td| {
        td.device(&id).is_some_and(|d| {
            matches!(d.link, td_companion::devices::LinkState::Connected { .. })
                && d.firmware.is_some_and(|f| f.version == "test-only")
        })
    })
    .await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());
    let mut events = td.events().await;
    let job = td
        .call::<Update>(FirmwareTarget { device: id }, NoQuery, request())
        .await
        .unwrap();
    succeeded(&td, &job).await;
    assert!(prompt.seen.lock().unwrap().is_empty());
    let mut sending = false;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(Ok(message)) = events.next().await {
            let Ok(text) = message.to_text() else {
                continue;
            };
            let frame: serde_json::Value = serde_json::from_str(text).unwrap();
            if frame["type"] != "job.updated" || frame["data"]["id"] != job.id {
                continue;
            }
            if frame["data"]["progress"]["phase"] == "sending"
                && frame["data"]["progress"]["bytes"].as_u64().unwrap_or(0) > 0
            {
                sending = true;
            }
            if frame["data"]["state"] == "succeeded" {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(sending);
}

#[tokio::test]
async fn the_job_stays_restarting_until_the_device_goes_away() {
    use futures_util::StreamExt;
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let prompt = Arc::new(Prompt::default());
    prompt.answer(true);
    td.firmware_prompter(prompt);
    let deck = td.deck("mocka");
    deck.restart_after(Duration::from_millis(600), Duration::from_millis(400));
    let mut events = td.events().await;
    let job = td
        .call::<Update>(FirmwareTarget { device: id }, NoQuery, request())
        .await
        .unwrap();
    let mut phases: Vec<String> = Vec::new();
    let mut confirming_at = None;
    tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(Ok(message)) = events.next().await {
            let Ok(text) = message.to_text() else {
                continue;
            };
            let frame: serde_json::Value = serde_json::from_str(text).unwrap();
            if frame["type"] != "job.updated" || frame["data"]["id"] != job.id {
                continue;
            }
            if let Some(phase) = frame["data"]["progress"]["phase"].as_str()
                && phases.last().map(String::as_str) != Some(phase)
            {
                if phase == "confirming" {
                    confirming_at = Some(Instant::now());
                }
                phases.push(phase.into());
            }
            if frame["data"]["state"] == "succeeded" {
                break;
            }
            if frame["data"]["state"] == "failed" {
                panic!("{frame}");
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        phases,
        ["checking", "sending", "rebooting", "confirming"],
        "the Job did not report each phase once, in order"
    );
    let dropped = deck.port_drops_at().expect("the update sent a restart");
    assert!(
        confirming_at.is_some_and(|at| at >= dropped),
        "the Job reported confirming before the Device went away"
    );
}

#[tokio::test]
async fn ending_or_revoking_lock_owner_allows_new_jobs() {
    use serde_json::{Value, json};
    for action in ["end", "revoke", "reset"] {
        let mut td = TestCompanion::start().await;
        let id = td.plug_device("mocka").await;
        let owner: Value = td
            .raw()
            .post("/api/v1/auth/sessions")
            .bearer_auth(&td.pairing_token)
            .json(&json!({"kind":"window"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let token = owner["token"].as_str().unwrap();
        assert_eq!(
            td.raw()
                .post("/api/v1/system/lock")
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            td.request::<Update>(FirmwareTarget { device: id.clone() }, NoQuery, &request())
                .send()
                .await
                .unwrap()
                .status(),
            503
        );
        let response = match action {
            "end" => td
                .raw()
                .request(reqwest::Method::DELETE, "/api/v1/auth/sessions/self")
                .bearer_auth(token)
                .send()
                .await
                .unwrap(),
            "revoke" => td
                .raw()
                .request(
                    reqwest::Method::DELETE,
                    &format!("/api/v1/auth/sessions/{}", owner["id"].as_str().unwrap()),
                )
                .bearer_auth(&td.token)
                .send()
                .await
                .unwrap(),
            _ => td
                .raw()
                .post("/api/v1/auth/reset")
                .bearer_auth(&td.token)
                .send()
                .await
                .unwrap(),
        };
        assert_eq!(response.status(), 204);
        if action != "reset" {
            let prompt = Arc::new(Prompt::default());
            prompt.answer(false);
            td.firmware_prompter(prompt);
            let job = td
                .call::<Repair>(FirmwareTarget { device: id }, NoQuery, request())
                .await
                .unwrap();
            assert_eq!(td.wait(&job).await.error.unwrap().code, "firmware.declined");
        } else {
            let session: Value = td
                .raw()
                .post("/api/v1/auth/sessions")
                .bearer_auth(td.current_pairing_token())
                .json(&json!({"kind":"window"}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            td.token = session["token"].as_str().unwrap().to_owned();
            let prompt = Arc::new(Prompt::default());
            prompt.answer(false);
            td.firmware_prompter(prompt);
            let job = td
                .call::<Repair>(FirmwareTarget { device: id }, NoQuery, request())
                .await
                .unwrap();
            assert_eq!(td.wait(&job).await.error.unwrap().code, "firmware.declined");
        }
    }
}

#[tokio::test]
async fn overlapping_file_pickers_are_rejected() {
    #[derive(Default)]
    struct Picker(Prompt);
    impl Prompter for Picker {
        fn pick(&self) -> Result<Option<Vec<u8>>, String> {
            self.0.confirm(
                &Confirmation {
                    question: "pick".into(),
                    target: String::new(),
                    name: String::new(),
                    port: String::new(),
                    current: String::new(),
                    version: String::new(),
                    build: String::new(),
                    channel: Default::default(),
                },
                &|| false,
            );
            Ok(None)
        }
        fn confirm(&self, _: &Confirmation, _: &(dyn Fn() -> bool + Sync)) -> bool {
            false
        }
    }
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let prompt = Arc::new(Picker::default());
    td.firmware_prompter(prompt.clone());
    let first = td.request::<Pick>((), NoQuery, &FirmwareTarget { device: id.clone() });
    let task = tokio::spawn(async move { first.send().await.unwrap() });
    td.until(|_| !prompt.0.seen.lock().unwrap().is_empty())
        .await;
    let second = td
        .request::<Pick>((), NoQuery, &FirmwareTarget { device: id })
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 409);
    assert_eq!(
        second.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "system.dialog_open"
    );
    prompt.0.answer(false);
    assert!(task.await.unwrap().status().is_success());
}

#[tokio::test]
async fn browser_cannot_take_install_lock() {
    let td = TestCompanion::start().await;
    let response = td
        .raw()
        .post("/api/v1/system/lock")
        .bearer_auth(td.browser_token())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "auth.forbidden"
    );
}

#[tokio::test]
async fn new_board_decline_preserves_other_firmware() {
    use td_companion::devices::BoardKind;
    let td = TestCompanion::start().await;
    let board = td.plug_board("newboard", BoardKind::OtherFirmware).await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());
    let job = td
        .call::<Install>(
            FirmwareTarget {
                device: board.id.clone(),
            },
            NoQuery,
            request(),
        )
        .await
        .unwrap();
    td.until(|_| !prompt.seen.lock().unwrap().is_empty()).await;
    let state: serde_json::Value = td
        .raw()
        .get(&format!("/api/v1/jobs/{}", job.id))
        .bearer_auth(&td.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["state"], "waiting_for_confirmation");
    assert_eq!(td.deck("newboard").reboots(), 0);
    {
        let seen = prompt.seen.lock().unwrap();
        assert_eq!(seen[0].target, board.id.as_str());
        assert_eq!(seen[0].port, board.port);
        assert_eq!(seen[0].current, "unknown");
        assert_eq!(seen[0].version, "test-only");
        assert_eq!(
            seen[0].text().lines().next(),
            Some("Install Firmware on this New board?")
        );
    }
    prompt.answer(false);
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, "firmware.declined");
    assert_eq!(td.deck("newboard").reboots(), 0);
    assert!(td.device(&board.id).is_none());
}

#[tokio::test]
async fn downgrade_waits_for_exact_consent_and_suppresses_every_device() {
    use futures_util::{SinkExt, StreamExt};
    struct VersionPrompt(Prompt);
    impl Prompter for VersionPrompt {
        fn pick(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(Some(include_bytes!("fixtures/firmware-v1.tdfw").to_vec()))
        }
        fn confirm(&self, c: &Confirmation, closed: &(dyn Fn() -> bool + Sync)) -> bool {
            self.0.confirm(c, closed)
        }
    }
    for accept in [false, true] {
        let td = TestCompanion::start().await;
        let id = td.plug_device("mocka").await;
        td.plug_device("mockb").await;
        let mut current = td_link::bundle::Bundle::parse(include_bytes!(
            "../../td-link/tests/fixtures/touchdeck-test-b.tdfw"
        ))
        .unwrap();
        current.manifest.version = "2.0.0".into();
        td.deck("mocka").flash_bundle(&current);
        td.call::<td_companion::features::devices::Restart>(
            td_companion::features::devices::DevicePath { device: id.clone() },
            NoQuery,
            (),
        )
        .await
        .unwrap();
        td.until(|td| {
            td.device(&id).is_some_and(|d| {
                matches!(d.link, td_companion::devices::LinkState::Connected { .. })
                    && d.firmware.is_some_and(|f| f.version == "2.0.0")
            })
        })
        .await;
        let before = td.deck("mocka").reboots();
        let prompt = Arc::new(VersionPrompt(Prompt::default()));
        td.firmware_prompter(prompt.clone());
        let selected = td
            .call::<Pick>((), NoQuery, FirmwareTarget { device: id.clone() })
            .await
            .unwrap()
            .bundle
            .unwrap();
        let mut events = td.events().await;
        for device in ["mocka", "mockb"] {
            events
                .send(tokio_tungstenite::tungstenite::Message::text(
                    serde_json::json!({"op":"subscribe", "topic":"key-mirror", "device":device})
                        .to_string(),
                ))
                .await
                .unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let job = td
            .call::<Update>(
                FirmwareTarget { device: id },
                NoQuery,
                FirmwareRequest {
                    bundle: selected.id,
                    channel: None,
                },
            )
            .await
            .unwrap();
        td.until(|_| !prompt.0.seen.lock().unwrap().is_empty())
            .await;
        {
            let seen = prompt.0.seen.lock().unwrap();
            assert_eq!(seen[0].target, "mocka");
            assert_eq!(seen[0].current, "2.0.0");
            assert_eq!(seen[0].version, "1.0.0");
            assert_eq!(
                seen[0].text().lines().next(),
                Some("Downgrade this Device's Firmware?")
            );
            assert_eq!(seen[0].build, selected.build);
        }
        assert_eq!(td.deck("mocka").reboots(), before);
        assert!(
            !td.deck("mocka")
                .received()
                .iter()
                .any(|v| v["m"] == "update.begin")
        );
        for device in ["mocka", "mockb"] {
            td.deck(device)
                .emit_key(&serde_json::json!({"type":"kbd","mods":0,"keys":[4]}));
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut suppressed = std::collections::BTreeSet::new();
            let mut waiting = false;
            while let Some(Ok(message)) = events.next().await {
                let Ok(text) = message.to_text() else {
                    continue;
                };
                let frame: serde_json::Value = serde_json::from_str(text).unwrap();
                if frame["type"] == "job.updated"
                    && frame["data"]["id"] == job.id
                    && frame["data"]["state"] == "waiting_for_confirmation"
                {
                    waiting = true;
                }
                if frame["type"] == "device.key-mirror" {
                    assert_eq!(frame["data"]["outcome"], "suppressed");
                    suppressed.insert(frame["data"]["device"].as_str().unwrap().to_owned());
                }
                if waiting && suppressed.len() == 2 {
                    break;
                }
            }
            assert!(waiting);
            assert_eq!(suppressed.len(), 2);
        })
        .await
        .unwrap();
        assert!(td.input_log().is_empty());
        prompt.0.answer(accept);
        let done = td.wait(&job).await;
        if accept {
            assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
            assert!(td.deck("mocka").reboots() > before);
        } else {
            assert_eq!(done.error.unwrap().code, "firmware.declined");
            assert_eq!(td.deck("mocka").reboots(), before);
        }
    }
}

#[tokio::test]
async fn all_install_control_routes_require_a_window() {
    let td = TestCompanion::start().await;
    let browser = td.browser_token();
    for (method, path) in [
        (reqwest::Method::POST, "/api/v1/system/lock"),
        (reqwest::Method::PUT, "/api/v1/system/lock/unknown"),
        (reqwest::Method::DELETE, "/api/v1/system/lock/unknown"),
        (reqwest::Method::POST, "/api/v1/system/shutdown"),
    ] {
        let response = td
            .raw()
            .request(method, path)
            .bearer_auth(&browser)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "auth.forbidden"
        );
    }
    assert!(!td.shutdown_requested());
}

#[tokio::test]
async fn active_firmware_job_blocks_install_lock() {
    use td_companion::features::system::Lock;
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());
    let job = td
        .call::<Repair>(FirmwareTarget { device: id }, NoQuery, request())
        .await
        .unwrap();
    td.until(|_| !prompt.seen.lock().unwrap().is_empty()).await;
    let response = td.request::<Lock>((), NoQuery, &()).send().await.unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "system.busy"
    );
    prompt.answer(false);
    td.wait(&job).await;
    td.call::<Lock>((), NoQuery, ()).await.unwrap();
}

#[tokio::test]
async fn expired_lease_allows_new_jobs() {
    use td_companion::features::system::Lock;
    let td = TestCompanion::start().await;
    let _socket = td.events().await;
    let id = td.plug_device("mocka").await;
    td.call::<Lock>((), NoQuery, ()).await.unwrap();
    td.clock().advance(std::time::Duration::from_secs(601));
    let prompt = Arc::new(Prompt::default());
    prompt.answer(false);
    td.firmware_prompter(prompt);
    let job = td
        .call::<Repair>(FirmwareTarget { device: id }, NoQuery, request())
        .await
        .unwrap();
    assert_eq!(td.wait(&job).await.error.unwrap().code, "firmware.declined");
}

#[tokio::test]
async fn firmware_requests_cannot_supply_bytes_or_consent() {
    let td = TestCompanion::start().await;
    td.plug_device("mocka").await;
    for extra in [
        serde_json::json!({"bytes":[1,2,3]}),
        serde_json::json!({"consent":true}),
    ] {
        let mut body = extra;
        body["bundle"] = serde_json::json!("bundled");
        let response = td
            .raw()
            .post("/api/v1/devices/mocka/firmware/update")
            .bearer_auth(&td.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error());
    }
    assert_eq!(td.deck("mocka").reboots(), 0);
    assert!(
        !td.deck("mocka")
            .received()
            .iter()
            .any(|v| v["m"] == "update.begin")
    );
}

#[tokio::test]
async fn repair_waits_then_flashes_only_after_acceptance() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());
    let job = td
        .call::<Repair>(FirmwareTarget { device: id.clone() }, NoQuery, request())
        .await
        .unwrap();
    td.until(|_| !prompt.seen.lock().unwrap().is_empty()).await;
    let state: serde_json::Value = td
        .raw()
        .get(&format!("/api/v1/jobs/{}", job.id))
        .bearer_auth(&td.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["state"], "waiting_for_confirmation");
    assert_eq!(td.deck("mocka").reboots(), 0);
    assert_eq!(
        prompt.seen.lock().unwrap()[0].text().lines().next(),
        Some("Repair this Device's Firmware?")
    );
    prompt.answer(true);
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
    let result: FirmwareResult = serde_json::from_value(done.result.unwrap()).unwrap();
    assert_eq!(result.version, "test-only");
    assert_eq!(td.deck("mocka").reboots(), 1);
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, td_companion::devices::LinkState::Connected { .. }))
    })
    .await;
}

#[tokio::test]
async fn consent_dialog_escapes_a_renamed_device() {
    use td_companion::features::devices::{DevicePath, DeviceSettingsPatch, SettingsPatch};
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let name = "Desk\u{2028}Port: COM99\u{2029}kced";
    td.call::<SettingsPatch>(
        DevicePath { device: id.clone() },
        NoQuery,
        DeviceSettingsPatch {
            name: Some(name.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| d.name.as_deref() == Some(name))
    })
    .await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());
    let job = td
        .call::<Repair>(FirmwareTarget { device: id }, NoQuery, request())
        .await
        .unwrap();
    td.until(|_| !prompt.seen.lock().unwrap().is_empty()).await;
    let text = prompt.seen.lock().unwrap()[0].text();
    prompt.answer(false);
    td.wait(&job).await;
    assert!(text.contains(r"\u{2028}"), "{text}");
    assert!(text.contains(r"\u{2029}"), "{text}");
    assert!(!text.contains(['\u{2028}', '\u{2029}']), "{text}");
    assert_eq!(
        text.lines().filter(|l| l.starts_with("Port:")).count(),
        1,
        "{text}"
    );
}

fn bundle_with_version(version: &str) -> Vec<u8> {
    use std::io::{Cursor, Read, Write};
    const FIXTURE: &[u8] = include_bytes!("../../td-link/tests/fixtures/touchdeck-test.tdfw");
    let mut input = zip::ZipArchive::new(Cursor::new(FIXTURE)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        let name = file.name().unwrap().to_string();
        if name == "manifest.json" {
            let mut manifest: serde_json::Value = serde_json::from_slice(&data).unwrap();
            manifest["version"] = version.into();
            data = serde_json::to_vec(&manifest).unwrap();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[tokio::test]
async fn pick_refuses_a_long_or_unprintable_version() {
    struct Picker(Vec<u8>);
    impl Prompter for Picker {
        fn pick(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(Some(self.0.clone()))
        }
        fn confirm(&self, _: &Confirmation, _: &(dyn Fn() -> bool + Sync)) -> bool {
            false
        }
    }
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    for version in ["9".repeat(65), "1.0.0\u{2028}Port: COM99".into()] {
        td.firmware_prompter(Arc::new(Picker(bundle_with_version(&version))));
        let response = td
            .request::<Pick>((), NoQuery, &FirmwareTarget { device: id.clone() })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 422, "{version:?}");
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "firmware.bundle_invalid"
        );
    }
}

#[tokio::test]
async fn pick_outlives_the_whole_request_limit() {
    struct SlowPicker;
    impl Prompter for SlowPicker {
        fn pick(&self) -> Result<Option<Vec<u8>>, String> {
            std::thread::sleep(std::time::Duration::from_millis(600));
            Ok(Some(
                include_bytes!("../../td-link/tests/fixtures/touchdeck-test.tdfw").to_vec(),
            ))
        }
        fn confirm(&self, _: &Confirmation, _: &(dyn Fn() -> bool + Sync)) -> bool {
            false
        }
    }
    let td = TestCompanion::start_with(td_companion::Limits {
        request: std::time::Duration::from_millis(300),
        ..Default::default()
    })
    .await;
    let id = td.plug_device("mocka").await;
    td.firmware_prompter(Arc::new(SlowPicker));
    let response = td
        .request::<Pick>((), NoQuery, &FirmwareTarget { device: id })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let picked: PickResult = response.json().await.unwrap();
    assert_eq!(picked.bundle.unwrap().version, "test-only");
}

async fn waiting_repair(
    td: &TestCompanion,
    prompt: Arc<dyn Prompter>,
) -> (td_companion::devices::DeviceId, td_companion::jobs::Job) {
    let id = td.plug_device("mocka").await;
    td.firmware_prompter(prompt);
    let job = td
        .call::<Repair>(FirmwareTarget { device: id.clone() }, NoQuery, request())
        .await
        .unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, td_companion::devices::LinkState::Updating { .. }))
    })
    .await;
    (id, job)
}
async fn job_state(td: &TestCompanion, job: &td_companion::jobs::Job) -> serde_json::Value {
    td.raw()
        .get(&format!("/api/v1/jobs/{}", job.id))
        .bearer_auth(&td.token)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["state"]
        .clone()
}
async fn ended_quickly(
    td: &TestCompanion,
    job: &td_companion::jobs::Job,
) -> td_companion::jobs::Job {
    tokio::time::timeout(Duration::from_secs(2), td.wait(job))
        .await
        .expect("the consent wait ends within 2 s")
}
async fn released_without_writes(td: &TestCompanion, id: &td_companion::devices::DeviceId) {
    use td_companion::features::system::{Lock, Unlock};
    let deck = td.deck("mocka");
    assert_eq!(deck.reboots(), 0);
    assert!(!deck.received().iter().any(|v| v["m"] == "update.begin"));
    td.until(|td| {
        td.device(id)
            .is_some_and(|d| matches!(d.link, td_companion::devices::LinkState::Connected { .. }))
    })
    .await;
    let lock = td.call::<Lock>((), NoQuery, ()).await.unwrap();
    td.call::<Unlock>(lock, NoQuery, ()).await.unwrap();
    deck.emit_key(&serde_json::json!({"type": "kbd", "mods": 0, "keys": [4]}));
    deck.emit_key(&serde_json::json!({"type": "kbd", "mods": 0, "keys": []}));
    td.until(|td| td.input_log() == ["down A", "up A"]).await;
}

#[tokio::test]
async fn unanswered_consent_expires_after_five_minutes() {
    let td = TestCompanion::start().await;
    let _socket = td.events().await;
    let prompt = Arc::new(Prompt::default());
    let (id, job) = waiting_repair(&td, prompt.clone()).await;
    td.clock().advance(Duration::from_secs(299));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(job_state(&td, &job).await, "waiting_for_confirmation");
    td.clock().advance(Duration::from_secs(2));
    let done = ended_quickly(&td, &job).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, "firmware.declined");
    assert!(prompt.closed.load(Ordering::SeqCst));
    released_without_writes(&td, &id).await;
}

#[tokio::test]
async fn cancel_while_waiting_ends_the_job_and_closes_the_dialog() {
    use td_companion::features::jobs::{Cancel, JobPath};
    let td = TestCompanion::start().await;
    let _socket = td.events().await;
    let prompt = Arc::new(Prompt::default());
    let (id, job) = waiting_repair(&td, prompt.clone()).await;
    td.call::<Cancel>(
        JobPath {
            job: job.id.clone(),
        },
        NoQuery,
        (),
    )
    .await
    .unwrap();
    let done = ended_quickly(&td, &job).await;
    assert_eq!(done.state, JobState::Cancelled);
    assert!(prompt.closed.load(Ordering::SeqCst));
    prompt.answer(true);
    released_without_writes(&td, &id).await;
}

#[tokio::test]
async fn an_answer_after_cancel_writes_nothing() {
    use td_companion::features::jobs::{Cancel, JobPath};
    #[derive(Default)]
    struct AcceptOnClose(Prompt);
    impl Prompter for AcceptOnClose {
        fn pick(&self) -> Result<Option<Vec<u8>>, String> {
            self.0.pick()
        }
        fn confirm(&self, c: &Confirmation, closed: &(dyn Fn() -> bool + Sync)) -> bool {
            self.0.seen.lock().unwrap().push(c.clone());
            let started = Instant::now();
            while !closed() && started.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(10));
            }
            true
        }
    }
    let td = TestCompanion::start().await;
    let _socket = td.events().await;
    let (id, job) = waiting_repair(&td, Arc::new(AcceptOnClose::default())).await;
    td.call::<Cancel>(
        JobPath {
            job: job.id.clone(),
        },
        NoQuery,
        (),
    )
    .await
    .unwrap();
    let done = ended_quickly(&td, &job).await;
    assert_eq!(done.state, JobState::Cancelled);
    released_without_writes(&td, &id).await;
}

#[tokio::test]
async fn installing_on_an_unknown_new_board_is_new_board_not_found() {
    let td = TestCompanion::start().await;
    let known = td.plug_device("mocka").await;
    td.unplug("mocka").await;
    let response = td
        .request::<Install>(FirmwareTarget { device: known }, NoQuery, &request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "new_board.not_found"
    );
}

async fn script_consent(td: &TestCompanion, answers: serde_json::Value) {
    let response = td
        .raw()
        .post("/api/v1/test/firmware/script")
        .bearer_auth(&td.token)
        .json(&serde_json::json!({ "answers": answers }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
}
async fn install(
    td: &TestCompanion,
    id: &td_companion::devices::DeviceId,
) -> td_companion::jobs::Job {
    td.call::<Install>(FirmwareTarget { device: id.clone() }, NoQuery, request())
        .await
        .unwrap()
}

#[tokio::test]
async fn scripted_consent_answers_new_board_installs_over_http() {
    use td_companion::devices::{BoardKind, SetupState};
    let td = TestCompanion::start().await;
    script_consent(&td, serde_json::json!([])).await;

    let declined = td.plug_board("declined", BoardKind::OtherFirmware).await;
    let job = install(&td, &declined.id).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(job_state(&td, &job).await, "waiting_for_confirmation");
    script_consent(&td, serde_json::json!(["decline"])).await;
    let done = ended_quickly(&td, &job).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, "firmware.declined");
    assert_eq!(td.deck("declined").reboots(), 0);
    assert!(td.device(&declined.id).is_none());

    script_consent(&td, serde_json::json!(["accept"])).await;
    let accepted = td.plug_board("accepted", BoardKind::OtherFirmware).await;
    let job = install(&td, &accepted.id).await;
    succeeded(&td, &job).await;
    td.until(|td| {
        td.device(&accepted.id)
            .is_some_and(|d| d.setup == SetupState::NeedsSetup)
    })
    .await;

    use td_companion::features::devices::{DevicePath, DeviceSettingsPatch, SettingsPatch};
    let patch = DeviceSettingsPatch {
        idle_timeout_s: None,
        orientation: None,
        name: Some("Workbench".into()),
        brightness: None,
        photo_frame: None,
    };
    td.call::<SettingsPatch>(
        DevicePath {
            device: accepted.id.clone(),
        },
        NoQuery,
        patch,
    )
    .await
    .unwrap();
    let named = td.device(&accepted.id).unwrap();
    assert_eq!(named.name.as_deref(), Some("Workbench"));
    assert_eq!(named.setup, SetupState::Ready);
}

#[tokio::test]
async fn an_unknown_consent_answer_is_request_invalid() {
    let td = TestCompanion::start().await;
    let response = td
        .raw()
        .post("/api/v1/test/firmware/script")
        .bearer_auth(&td.token)
        .json(&serde_json::json!({ "answers": ["maybe"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn diagnostics_follow_the_hello_after_an_update() {
    use futures_util::StreamExt;
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let before = td.device(&id).unwrap();
    assert_eq!(before.diagnostics.as_ref().unwrap().running_from, "ota_0");
    let mut events = td.events().await;
    let prompt = Arc::new(Prompt::default());
    prompt.answer(true);
    td.firmware_prompter(prompt);
    let job = td
        .call::<Update>(FirmwareTarget { device: id.clone() }, NoQuery, request())
        .await
        .unwrap();
    succeeded(&td, &job).await;
    let running = |td: &TestCompanion| {
        td.device(&id)
            .and_then(|d| d.diagnostics)
            .map(|d| d.running_from)
    };
    td.until(|td| running(td).as_deref() == Some("ota_1")).await;
    assert!(td.device(&id).unwrap().rev > before.rev);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = events.next().await.unwrap().unwrap();
            let Ok(frame) =
                serde_json::from_str::<serde_json::Value>(message.to_text().unwrap_or(""))
            else {
                continue;
            };
            if frame["type"] == "device.updated"
                && frame["device"] == id.as_str()
                && frame["data"]["diagnostics"]["running_from"] == "ota_1"
            {
                return;
            }
        }
    })
    .await
    .expect("device.updated carried the new running_from");
}

const NORMAL_BUILD: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const RADIO_BUILD: &str = "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f";
const RADIO_BUNDLE: &[u8] =
    include_bytes!("../../td-link/tests/fixtures/touchdeck-test-radio.tdfw");

fn channel_of(td: &TestCompanion, id: &td_companion::devices::DeviceId) -> Option<FirmwareChannel> {
    td.device(id).and_then(|d| d.firmware).map(|f| f.channel)
}
/// True once the Device is linked again, free for the next Job, and runs `channel`.
fn ready_on(
    td: &TestCompanion,
    id: &td_companion::devices::DeviceId,
    channel: FirmwareChannel,
) -> bool {
    td.device(id).is_some_and(|d| {
        matches!(d.link, td_companion::devices::LinkState::Connected { .. })
            && d.firmware.is_some_and(|f| f.channel == channel)
    })
}
fn build_of(td: &TestCompanion, id: &td_companion::devices::DeviceId) -> Option<String> {
    td.device(id).and_then(|d| d.firmware).map(|f| f.build)
}
/// Makes the mock Device run the radio build and waits until its hello says so.
async fn run_radio_build(td: &TestCompanion, serial: &str, id: &td_companion::devices::DeviceId) {
    td.deck(serial).set_radio(true);
    td.call::<td_companion::features::devices::Restart>(
        td_companion::features::devices::DevicePath { device: id.clone() },
        NoQuery,
        (),
    )
    .await
    .unwrap();
    td.until(|td| ready_on(td, id, FirmwareChannel::Radio))
        .await;
}
fn bundled(channel: Option<FirmwareChannel>) -> FirmwareRequest {
    FirmwareRequest {
        bundle: "bundled".into(),
        channel,
    }
}
async fn update_error(
    td: &TestCompanion,
    id: &td_companion::devices::DeviceId,
    body: &FirmwareRequest,
) -> (u16, serde_json::Value) {
    let response = td
        .request::<Update>(FirmwareTarget { device: id.clone() }, NoQuery, body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json::<serde_json::Value>().await.unwrap())
}
struct Picker(&'static [u8]);
impl Prompter for Picker {
    fn pick(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(self.0.to_vec()))
    }
    fn confirm(&self, _: &Confirmation, _: &(dyn Fn() -> bool + Sync)) -> bool {
        panic!("a refused bundle asked for consent")
    }
}

#[tokio::test]
async fn bundled_reports_the_normal_and_the_radio_build() {
    let td = TestCompanion::start().await;
    let build = td.call::<Bundled>((), NoQuery, ()).await.unwrap();
    assert_eq!(build.version, "test-only");
    assert_eq!(build.build, NORMAL_BUILD);
    let radio = build.radio.expect("the Companion carries a radio build");
    assert_eq!(radio.version, "test-only-radio");
    assert_eq!(radio.build, RADIO_BUILD);
}

#[tokio::test]
async fn an_update_takes_the_build_the_device_runs() {
    let td = TestCompanion::start().await;
    let normal = td.plug_device("mocka").await;
    let radio = td.plug_device("mockb").await;
    assert_eq!(channel_of(&td, &normal), Some(FirmwareChannel::Normal));
    run_radio_build(&td, "mockb", &radio).await;
    let prompt = Arc::new(Prompt::default());
    prompt.answer(true);
    td.firmware_prompter(prompt.clone());

    let job = td
        .call::<Update>(
            FirmwareTarget {
                device: normal.clone(),
            },
            NoQuery,
            bundled(None),
        )
        .await
        .unwrap();
    succeeded(&td, &job).await;
    td.until(|td| build_of(td, &normal).as_deref() == Some(NORMAL_BUILD))
        .await;
    assert_eq!(channel_of(&td, &normal), Some(FirmwareChannel::Normal));

    prompt.answer(true);
    let job = td
        .call::<Update>(
            FirmwareTarget {
                device: radio.clone(),
            },
            NoQuery,
            bundled(None),
        )
        .await
        .unwrap();
    let done = td.wait(&job).await;
    assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
    let result: FirmwareResult = serde_json::from_value(done.result.unwrap()).unwrap();
    assert_eq!(result.version, "test-only-radio");
    td.until(|td| build_of(td, &radio).as_deref() == Some(RADIO_BUILD))
        .await;
    assert_eq!(channel_of(&td, &radio), Some(FirmwareChannel::Radio));
    let seen = prompt.seen.lock().unwrap();
    let last = seen.last().unwrap();
    assert_eq!(last.channel, FirmwareChannel::Radio);
    assert!(!last.question.starts_with("Switch"), "{}", last.question);
    assert!(
        last.text()
            .contains("Bundle: \"test-only-radio\" (radio build)"),
        "{}",
        last.text()
    );
}

#[tokio::test]
async fn switching_builds_asks_first_and_works_both_ways() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let prompt = Arc::new(Prompt::default());
    td.firmware_prompter(prompt.clone());

    // A declined switch writes nothing.
    let job = td
        .call::<Update>(
            FirmwareTarget { device: id.clone() },
            NoQuery,
            bundled(Some(FirmwareChannel::Radio)),
        )
        .await
        .unwrap();
    td.until(|_| !prompt.seen.lock().unwrap().is_empty()).await;
    prompt.answer(false);
    let done = td.wait(&job).await;
    assert_eq!(done.error.unwrap().code, "firmware.declined");
    assert_eq!(td.deck("mocka").reboots(), 0);
    assert_eq!(channel_of(&td, &id), Some(FirmwareChannel::Normal));
    td.until(|td| ready_on(td, &id, FirmwareChannel::Normal))
        .await;

    prompt.answer(true);
    let job = td
        .call::<Update>(
            FirmwareTarget { device: id.clone() },
            NoQuery,
            bundled(Some(FirmwareChannel::Radio)),
        )
        .await
        .unwrap();
    succeeded(&td, &job).await;
    td.until(|td| ready_on(td, &id, FirmwareChannel::Radio))
        .await;
    assert_eq!(build_of(&td, &id).as_deref(), Some(RADIO_BUILD));

    prompt.answer(true);
    let job = td
        .call::<Update>(
            FirmwareTarget { device: id.clone() },
            NoQuery,
            bundled(Some(FirmwareChannel::Normal)),
        )
        .await
        .unwrap();
    succeeded(&td, &job).await;
    td.until(|td| channel_of(td, &id) == Some(FirmwareChannel::Normal))
        .await;
    assert_eq!(build_of(&td, &id).as_deref(), Some(NORMAL_BUILD));

    let seen = prompt.seen.lock().unwrap();
    let asked: Vec<_> = seen
        .iter()
        .map(|c| (c.question.as_str(), c.channel))
        .collect();
    assert_eq!(asked.len(), 3);
    assert!(asked[..2].iter().all(|(q, c)| {
        q.starts_with("Switch this Device to the radio build?") && *c == FirmwareChannel::Radio
    }));
    assert!(
        asked[2]
            .0
            .starts_with("Switch this Device back to the normal build?")
    );
    assert!(asked[2].0.contains("stays stored but unused"));
    assert_eq!(asked[2].1, FirmwareChannel::Normal);
}

#[tokio::test]
async fn a_repair_that_changes_the_build_says_so_before_it_writes() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;
    run_radio_build(&td, "mocka", &id).await;
    let reboots = td.deck("mocka").reboots();
    let prompt = Arc::new(Prompt::default());
    prompt.answer(false);
    td.firmware_prompter(prompt.clone());

    // A repair keeps the build the Device runs and says nothing more.
    let job = td
        .call::<Repair>(
            FirmwareTarget { device: id.clone() },
            NoQuery,
            bundled(None),
        )
        .await
        .unwrap();
    assert_eq!(td.wait(&job).await.error.unwrap().code, "firmware.declined");
    td.until(|td| ready_on(td, &id, FirmwareChannel::Radio))
        .await;

    // A repair onto the other build names the change.
    prompt.answer(false);
    let job = td
        .call::<Repair>(
            FirmwareTarget { device: id.clone() },
            NoQuery,
            bundled(Some(FirmwareChannel::Normal)),
        )
        .await
        .unwrap();
    assert_eq!(td.wait(&job).await.error.unwrap().code, "firmware.declined");
    assert_eq!(
        td.deck("mocka").reboots(),
        reboots,
        "a declined repair wrote Firmware"
    );
    let seen = prompt.seen.lock().unwrap();
    assert_eq!(seen[0].question, "Repair this Device's Firmware?");
    assert_eq!(seen[0].channel, FirmwareChannel::Radio);
    assert!(
        seen[1]
            .question
            .starts_with("Repair this Device's Firmware? This installs the normal build instead of the radio build."),
        "{}",
        seen[1].question
    );
    assert_eq!(seen[1].channel, FirmwareChannel::Normal);
}

#[tokio::test]
async fn a_bundle_of_the_other_build_is_refused_unless_asked_for() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mocka").await;

    // A radio file is never the normal build's update.
    td.firmware_prompter(Arc::new(Picker(RADIO_BUNDLE)));
    let info = td
        .call::<Pick>((), NoQuery, FirmwareTarget { device: id.clone() })
        .await
        .unwrap()
        .bundle
        .unwrap();
    assert_eq!(info.channel, FirmwareChannel::Radio);
    for channel in [None, Some(FirmwareChannel::Normal)] {
        let (status, body) = update_error(
            &td,
            &id,
            &FirmwareRequest {
                bundle: info.id.clone(),
                channel,
            },
        )
        .await;
        assert_eq!(status, 409, "{body}");
        assert_eq!(body["error"]["code"], "firmware.channel_mismatch");
        assert_eq!(
            body["error"]["details"],
            serde_json::json!({"channel":"normal","bundle":"radio"})
        );
    }
    assert_eq!(td.deck("mocka").reboots(), 0);

    // A normal file on a radio Device is refused the same way.
    run_radio_build(&td, "mocka", &id).await;
    td.firmware_prompter(Arc::new(Picker(include_bytes!(
        "../../td-link/tests/fixtures/touchdeck-test.tdfw"
    ))));
    let info = td
        .call::<Pick>((), NoQuery, FirmwareTarget { device: id.clone() })
        .await
        .unwrap()
        .bundle
        .unwrap();
    assert_eq!(info.channel, FirmwareChannel::Normal);
    let (status, body) = update_error(
        &td,
        &id,
        &FirmwareRequest {
            bundle: info.id.clone(),
            channel: None,
        },
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"]["details"],
        serde_json::json!({"channel":"radio","bundle":"normal"})
    );

    // Asking to switch with a file of the build asked for is allowed, and asks first.
    let prompt = Arc::new(Prompt::default());
    prompt.answer(true);
    td.firmware_prompter(prompt.clone());
    let job = td
        .call::<Update>(
            FirmwareTarget { device: id.clone() },
            NoQuery,
            FirmwareRequest {
                bundle: info.id,
                channel: Some(FirmwareChannel::Normal),
            },
        )
        .await
        .unwrap();
    succeeded(&td, &job).await;
    assert!(
        prompt.seen.lock().unwrap()[0]
            .question
            .starts_with("Switch this Device back to the normal build?")
    );
    td.until(|td| channel_of(td, &id) == Some(FirmwareChannel::Normal))
        .await;
}

#[tokio::test]
async fn a_new_board_gets_the_normal_build_unless_the_radio_build_is_asked_for() {
    use td_companion::devices::BoardKind;
    let td = TestCompanion::start().await;
    script_consent(&td, serde_json::json!(["accept", "accept"])).await;
    let plain = td.plug_board("plain", BoardKind::OtherFirmware).await;
    let job = install(&td, &plain.id).await;
    succeeded(&td, &job).await;
    td.until(|td| channel_of(td, &plain.id) == Some(FirmwareChannel::Normal))
        .await;
    assert!(!td.deck("plain").radio());

    let radio = td.plug_board("radio", BoardKind::OtherFirmware).await;
    let job = td
        .call::<Install>(
            FirmwareTarget {
                device: radio.id.clone(),
            },
            NoQuery,
            bundled(Some(FirmwareChannel::Radio)),
        )
        .await
        .unwrap();
    succeeded(&td, &job).await;
    td.until(|td| channel_of(td, &radio.id) == Some(FirmwareChannel::Radio))
        .await;
    assert!(td.deck("radio").radio());
}
