//! The Integrations screen's session through the in-process Companion: setup, the switch, one
//! Save for settings and secrets, Forget, checks, delivery errors and the approval wait, with no
//! window.
use serde_json::json;
use std::time::Duration;
use td_companion::{
    checkpoints::Checkpoint,
    integrations::session::{
        Busy, FLOOR_TEXT, FieldValue, IntegrationsSession, IntegrationsView, Load, NoticeKind,
        Tone, WAITING_FOR_APPROVAL,
    },
    testing::InProcessCompanion,
};

const KEY: &str = "fixture-private-value";

async fn until(
    session: &IntegrationsSession,
    what: &str,
    done: impl Fn(&IntegrationsView) -> bool,
) -> IntegrationsView {
    let mut views = session.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            {
                let view = views.borrow_and_update().clone();
                if done(&view) {
                    return view;
                }
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}: {:?}", session.view()))
}

fn text(value: &str) -> FieldValue {
    FieldValue::Text(value.into())
}

fn on(session: &IntegrationsSession, id: &str) -> bool {
    session.view().row(id).unwrap().switch.checked
}

/// cliproxyapi: the switch opens setup, one Save sends the address and the key and turns it on.
#[tokio::test]
async fn cliproxyapi_sets_up_turns_on_and_off_and_forgets_its_key() {
    let mut td = InProcessCompanion::start().await;
    let session = IntegrationsSession::new(td.handle.clone());
    let row = session.view().row("cliproxyapi").unwrap().clone();
    assert!(!row.switch.checked);
    assert!(row.settings);
    assert_eq!(row.status.words, "Not set up");

    // On before setup: the page opens to set it up and the switch reads on, waiting.
    assert!(session.flip("cliproxyapi", true).await);
    let view = session.view();
    let switch = view.row("cliproxyapi").unwrap().switch;
    assert!(switch.checked && switch.busy && !switch.locked);
    let detail = view.detail.unwrap();
    assert!(detail.set_up);
    assert_eq!(detail.load, Load::Ready);
    assert_eq!(detail.settings_title, "Connection");
    assert_eq!(detail.status.words, "Finish setup");
    assert_eq!(
        detail.refresh.as_ref().unwrap().line,
        "Finish setup to check."
    );
    let names: Vec<_> = detail.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(names, ["Hub address", "Check every"]);
    assert_eq!(detail.secrets[0].field.label, "Management key");
    assert!(!td.handle.integrations().enabled("cliproxyapi"));

    // The address without its key: setup can't finish, so it stays off and names the key.
    session.set_value("url", text("https://hub.example"));
    session.set_value("interval_s", text("120"));
    assert!(!session.save().await);
    let detail = session.view().detail.unwrap();
    assert_eq!(
        detail.errors.get("management_key").map(String::as_str),
        Some("Enter the management key.")
    );
    assert_eq!(detail.notice.unwrap().event, "save-failed");
    assert!(!td.handle.integrations().enabled("cliproxyapi"));
    assert!(on(&session, "cliproxyapi"), "still waiting for setup");

    // One Save with the key finishes setup and turns it on.
    session.enter_secret("management_key", KEY);
    assert!(session.view().detail.unwrap().secrets[0].entered);
    assert!(session.save().await);
    let view = session.view();
    let detail = view.detail.as_ref().unwrap();
    assert_eq!(detail.notice.as_ref().unwrap().event, "saved");
    assert!(detail.secrets[0].set && !detail.secrets[0].entered);
    assert_eq!(detail.status.words, "Connected");
    assert_eq!(detail.header_line(0), "Running");
    assert!(!detail.dirty());
    let row = view.row("cliproxyapi").unwrap();
    assert!(row.switch.checked && !row.switch.busy);
    assert!(td.handle.integrations().enabled("cliproxyapi"));
    assert!(!format!("{view:?}").contains(KEY), "the key never shows");

    // Off and on again keep the address and the key.
    assert!(session.flip("cliproxyapi", false).await);
    assert!(!on(&session, "cliproxyapi"));
    let detail = session.view().detail.unwrap();
    assert_eq!(detail.status.words, "Off");
    assert!(detail.secrets[0].set);
    assert_eq!(
        detail.refresh.unwrap().line,
        "Turn on CLIProxyAPI to check."
    );
    assert!(session.flip("cliproxyapi", true).await);
    assert!(td.handle.integrations().enabled("cliproxyapi"));

    // Forget the key, then turn it off: removed.
    assert!(session.forget_secret("management_key").await);
    let detail = session.view().detail.unwrap();
    assert!(!detail.secrets[0].set);
    assert_eq!(detail.status.words, "Finish setup");
    assert!(session.flip("cliproxyapi", false).await);
    let view = td.handle.integrations().view("cliproxyapi").unwrap();
    assert!(!view.summary.enabled);
    assert!(!view.secrets["management_key"].set);
    assert_eq!(view.config["url"], "https://hub.example", "nothing deleted");
    assert_eq!(
        session.view().row("cliproxyapi").unwrap().status.words,
        "Not set up"
    );
    td.shutdown().await;
}

/// this_pc, media and volume have only their switch: setup is a Save of nothing.
#[tokio::test]
async fn switch_only_integrations_set_up_turn_on_and_off_and_have_no_secret() {
    let mut td = InProcessCompanion::start().await;
    let session = IntegrationsSession::new(td.handle.clone());
    for id in ["this_pc", "media", "volume"] {
        let row = session.view().row(id).unwrap().clone();
        assert!(!row.settings, "{id}");
        assert!(!row.line.is_empty(), "{id}");

        assert!(session.open(id).await, "{id}");
        let detail = session.view().detail.unwrap();
        assert_eq!(detail.load, Load::Ready, "{id}");
        assert!(
            detail.fields.is_empty() && detail.secrets.is_empty(),
            "{id}"
        );
        assert!(session.save().await, "{id}");
        assert_eq!(
            session.view().detail.unwrap().notice.unwrap().event,
            "saved"
        );

        assert!(session.flip(id, false).await, "{id}");
        assert!(!on(&session, id), "{id}");
        assert!(!td.handle.integrations().enabled(id), "{id}");
        assert_eq!(session.view().detail.unwrap().status.words, "Off", "{id}");

        assert!(session.flip(id, true).await, "{id}");
        assert!(on(&session, id), "{id}");
        assert!(td.handle.integrations().enabled(id), "{id}");
        assert_ne!(session.view().row(id).unwrap().status.tone, None, "{id}");

        assert!(!session.forget_secret("key").await, "{id} keeps no secret");
        assert!(
            td.handle
                .integrations()
                .view(id)
                .unwrap()
                .secrets
                .is_empty()
        );
        session.close();
        assert!(session.view().detail.is_none());
    }
    td.shutdown().await;
}

fn http_source() -> String {
    json!([{
        "id": "temperature",
        "name": "Temperature",
        "kind": "number",
        "interval_s": 60,
        "input": {
            "type": "http",
            "url": "https://example.invalid/metrics",
            "value_path": "$.value",
            "headers": {"Authorization": "auth"}
        }
    }])
    .to_string()
}

/// custom: setup saves its sources, then the header secret they name.
#[tokio::test]
async fn custom_sets_up_turns_on_and_off_and_forgets_its_header_secret() {
    let mut td = InProcessCompanion::start().await;
    let session = IntegrationsSession::new(td.handle.clone());
    assert_eq!(
        session.view().row("custom").unwrap().status.words,
        "Not set up"
    );

    assert!(session.flip("custom", true).await);
    let detail = session.view().detail.unwrap();
    assert_eq!(detail.settings_title, "Sources");
    assert_eq!(detail.fields[0].label, "Custom sources (JSON)");
    assert_eq!(detail.values["sources"], text("[]"));
    assert!(detail.secrets.is_empty());

    session.set_value("sources", text("[{"));
    assert!(!session.save().await);
    assert_eq!(
        session.view().detail.unwrap().errors["sources"],
        "Enter valid JSON."
    );
    assert!(!td.handle.integrations().enabled("custom"));

    session.set_value("sources", text(&http_source()));
    assert!(session.save().await);
    assert!(td.handle.integrations().enabled("custom"));
    let detail = session.view().detail.unwrap();
    assert_eq!(detail.secrets.len(), 1, "the header's secret field appears");
    assert_eq!(detail.secrets[0].field.name, "auth");
    assert!(!detail.secrets[0].set);

    session.enter_secret("auth", KEY);
    assert!(session.save().await);
    let view = session.view();
    assert!(view.detail.as_ref().unwrap().secrets[0].set);
    assert!(!format!("{view:?}").contains(KEY));

    assert!(session.flip("custom", false).await);
    assert!(!td.handle.integrations().enabled("custom"));
    assert!(session.flip("custom", true).await);
    assert!(td.handle.integrations().enabled("custom"));

    assert!(session.forget_secret("auth").await);
    assert!(!session.view().detail.unwrap().secrets[0].set);
    assert!(!td.handle.integrations().view("custom").unwrap().secrets["auth"].set);
    td.shutdown().await;
}

#[tokio::test]
async fn a_cancelled_setup_does_not_enable() {
    let mut td = InProcessCompanion::start().await;
    let session = IntegrationsSession::new(td.handle.clone());

    // Off while the switch waits for setup cancels it; the page stays open.
    assert!(session.flip("cliproxyapi", true).await);
    assert!(session.flip("cliproxyapi", false).await);
    assert!(!on(&session, "cliproxyapi"));
    session.set_value("url", text("https://hub.example"));
    session.set_value("interval_s", text("120"));
    session.enter_secret("management_key", KEY);
    assert!(session.save().await);
    let detail = session.view().detail.unwrap();
    assert!(detail.secrets[0].set, "saved");
    assert_eq!(detail.status.words, "Off");
    assert!(!td.handle.integrations().enabled("cliproxyapi"));

    // Closing the page cancels it too.
    session.forget_secret("management_key").await;
    assert!(session.flip("cliproxyapi", true).await);
    session.close();
    assert!(!on(&session, "cliproxyapi"));
    assert!(session.open("cliproxyapi").await);
    session.enter_secret("management_key", KEY);
    assert!(session.save().await);
    assert!(!td.handle.integrations().enabled("cliproxyapi"));
    assert!(!on(&session, "cliproxyapi"));

    // So does opening another Integration's page.
    assert!(session.flip("custom", true).await);
    assert!(session.open("this_pc").await);
    assert!(session.open("custom").await);
    session.set_value("sources", text(&http_source()));
    assert!(session.save().await);
    assert!(!td.handle.integrations().enabled("custom"));
    td.shutdown().await;
}

async fn set_up_cliproxyapi(td: &InProcessCompanion) {
    let integrations = td.handle.integrations();
    integrations
        .configure(
            "cliproxyapi",
            json!({"url": "https://hub.example", "interval_s": 120}),
        )
        .unwrap();
    integrations
        .secret_put("cliproxyapi", "management_key", KEY)
        .unwrap();
}

#[tokio::test]
async fn a_stale_form_does_not_undo_a_switch() {
    let mut td = InProcessCompanion::start().await;
    set_up_cliproxyapi(&td).await;
    let session = IntegrationsSession::new(td.handle.clone());
    let other = IntegrationsSession::new(td.handle.clone());
    for enabled in [true, false] {
        assert!(session.flip("cliproxyapi", !enabled).await);
        assert!(session.open("cliproxyapi").await);
        session.set_value("interval_s", text("300"));
        // Another window changes the switch after this form was loaded.
        assert!(other.flip("cliproxyapi", enabled).await);
        until(&session, "the switch reaches this window", |v| {
            v.row("cliproxyapi").unwrap().switch.checked == enabled
        })
        .await;
        let detail = session.view().detail.unwrap();
        assert_eq!(
            detail.values["interval_s"],
            text("300"),
            "what's typed stays"
        );
        assert!(session.save().await);
        let view = td.handle.integrations().view("cliproxyapi").unwrap();
        assert_eq!(view.summary.enabled, enabled);
        assert_eq!(view.config["interval_s"], 300);
        assert!(view.secrets["management_key"].set);
        assert_eq!(on(&session, "cliproxyapi"), enabled);
        session.set_value("interval_s", text("120"));
        assert!(session.save().await);
    }

    // This window's own switch, flipped while its form is changed, holds too.
    session.set_value("interval_s", text("600"));
    assert!(session.flip("cliproxyapi", true).await);
    assert_eq!(
        session.view().detail.unwrap().values["interval_s"],
        text("600")
    );
    assert!(session.save().await);
    assert!(td.handle.integrations().enabled("cliproxyapi"));
    td.shutdown().await;
}

#[tokio::test]
async fn fields_are_checked_and_a_moved_address_forgets_the_key() {
    let mut td = InProcessCompanion::start().await;
    set_up_cliproxyapi(&td).await;
    let session = IntegrationsSession::new(td.handle.clone());
    assert!(session.open("cliproxyapi").await);

    session.set_value("interval_s", text("soon"));
    assert!(!session.save().await);
    let detail = session.view().detail.unwrap();
    assert_eq!(detail.errors["interval_s"], "Enter a whole number.");
    assert_eq!(detail.notice, None);
    assert_eq!(
        td.handle.integrations().view("cliproxyapi").unwrap().config["interval_s"],
        120,
        "nothing sent"
    );
    session.set_value("interval_s", text("120"));
    assert!(session.view().detail.unwrap().errors.is_empty());

    session.set_value("url", text("http://hub.example"));
    assert_eq!(
        session.view().detail.unwrap().warning("url"),
        Some(
            "This address uses http:// and isn't on your network, so the key isn't sent. Use https://."
        )
    );
    session.set_value("url", text("http://192.168.1.20:8317"));
    assert!(
        session
            .view()
            .detail
            .unwrap()
            .warning("url")
            .unwrap()
            .contains("unencrypted")
    );
    session.set_value("url", text("http://localhost:8317"));
    assert_eq!(session.view().detail.unwrap().warning("url"), None);

    session.set_value("url", text("https://other.example"));
    assert!(session.save().await);
    let detail = session.view().detail.unwrap();
    let notice = detail.notice.unwrap();
    assert_eq!(notice.kind, NoticeKind::Info);
    assert_eq!(
        notice.text,
        "Saved. The address changed, so the management key was forgotten. Enter it again."
    );
    assert!(!detail.secrets[0].set);
    td.shutdown().await;
}

#[tokio::test]
async fn checks_keep_their_floor() {
    let mut td = InProcessCompanion::start().await;
    set_up_cliproxyapi(&td).await;
    td.handle
        .integrations()
        .set_enabled("cliproxyapi", true)
        .unwrap();
    let session = IntegrationsSession::new(td.handle.clone());
    assert!(session.open("cliproxyapi").await);
    let refresh = session.view().detail.unwrap().refresh.unwrap();
    assert!(refresh.usable);
    assert_eq!(refresh.line, FLOOR_TEXT);

    assert!(session.refresh().await);
    let detail = session.view().detail.unwrap();
    assert_eq!(detail.notice.unwrap().text, "Check started");
    assert_eq!(detail.busy, None);
    assert!(detail.refresh.unwrap().waiting(std::time::Instant::now()));

    assert!(!session.refresh().await);
    let notice = session.view().detail.unwrap().notice.unwrap();
    assert_eq!(notice.event, "refresh-floor");
    assert_eq!(notice.text, FLOOR_TEXT);
    td.shutdown().await;
}

#[tokio::test]
async fn delivery_errors_name_the_device() {
    let mut td = InProcessCompanion::start().await;
    td.plug_device("mock0001").await;
    let session = IntegrationsSession::new(td.handle.clone());
    assert!(session.open("this_pc").await);
    td.handle.integrations().delivered(
        "this_pc",
        td_host::integrations::Delivery {
            device: "mock0001".into(),
            sent: 1,
            error: Some("The Device didn't answer.".into()),
        },
    );
    let view = until(&session, "the delivery error", |v| {
        v.detail.as_ref().is_some_and(|d| !d.deliveries.is_empty())
    })
    .await;
    let problem = &view.detail.as_ref().unwrap().deliveries[0];
    assert_eq!(problem.device, "mock0001");
    assert_eq!(
        problem.text,
        "Couldn't send values to Mock Device mock0001. See Details."
    );
    assert_eq!(problem.detail, "The Device didn't answer.");
    assert_eq!(view.row("this_pc").unwrap().delivery_failures, 1);
    td.shutdown().await;
}

#[tokio::test]
async fn a_command_waits_for_approval_on_this_pc_and_can_show_its_prompt_again() {
    let mut td = InProcessCompanion::start().await;
    let program = if cfg!(windows) {
        r"C:\Windows\System32\whoami.exe"
    } else {
        "/usr/bin/whoami"
    };
    let sources = json!([{
        "id": "user",
        "name": "User",
        "kind": "text",
        "interval_s": 60,
        "input": {"type": "command", "program": program, "args": []}
    }]);
    let session = IntegrationsSession::new(td.handle.clone());
    assert!(session.flip("custom", true).await);
    session.set_value("sources", text(&sources.to_string()));
    assert!(session.save().await);

    let view = until(&session, "the approval wait", |v| {
        v.detail.as_ref().is_some_and(|d| d.approval.is_some())
    })
    .await;
    let detail = view.detail.as_ref().unwrap();
    assert_eq!(detail.status.words, WAITING_FOR_APPROVAL);
    assert_eq!(detail.status.tone, Some(Tone::Wait));
    assert_eq!(detail.header_line(0), WAITING_FOR_APPROVAL);
    let approval = detail.approval.as_ref().unwrap();
    assert_eq!(approval.program, program);
    assert_eq!(view.row("custom").unwrap().line, WAITING_FOR_APPROVAL);

    assert!(session.show_prompt().await);
    assert!(!session.show_prompt().await, "at most once every 5 s");

    // Denied on this PC: the wait ends and the reason shows instead.
    td.handle
        .actions()
        .script(vec![td_companion::actions::ScriptAnswer::Deny]);
    let view = until(&session, "the denial", |v| {
        v.detail
            .as_ref()
            .is_some_and(|d| d.approval.is_none() && d.status.tone == Some(Tone::Bad))
    })
    .await;
    let detail = view.detail.unwrap();
    assert_eq!(detail.status.words, "Can't connect");
    assert_eq!(detail.status.detail.as_deref(), Some("Denied on this PC."));
    assert!(!session.show_prompt().await);
    td.shutdown().await;
}

/// One Save sends new sources together with the header secret they name.
#[tokio::test]
async fn one_save_sends_custom_sources_with_the_header_secret_they_name() {
    let mut td = InProcessCompanion::start().await;
    let session = IntegrationsSession::new(td.handle.clone());
    assert!(session.flip("custom", true).await);
    session.set_value("sources", text(&http_source()));
    let detail = session.view().detail.unwrap();
    assert_eq!(detail.secrets.len(), 1, "the draft names the secret");
    assert_eq!(detail.secrets[0].field.name, "auth");
    session.enter_secret("auth", KEY);
    assert!(session.save().await);
    assert!(td.handle.integrations().enabled("custom"));
    assert!(td.handle.integrations().view("custom").unwrap().secrets["auth"].set);
    assert!(session.view().detail.unwrap().secrets[0].set);
    td.shutdown().await;
}

/// A Save that fails changes nothing: not the address, and not the key it would replace.
#[tokio::test]
async fn a_failed_save_leaves_settings_and_secrets_as_they_were() {
    let mut td = InProcessCompanion::start().await;
    let service = td.handle.integrations();
    service
        .configure(
            "cliproxyapi",
            json!({"url": "https://old.example", "interval_s": 120}),
        )
        .unwrap();
    service
        .secret_put("cliproxyapi", "management_key", "fixture-old")
        .unwrap();
    let session = IntegrationsSession::new(td.handle.clone());
    assert!(session.open("cliproxyapi").await);
    session.set_value("url", text("https://new.example"));
    session.enter_secret("management_key", "invalid\nreplacement");
    assert!(!session.save().await);
    let view = service.view("cliproxyapi").unwrap();
    assert_eq!(view.config["url"], "https://old.example");
    assert!(view.secrets["management_key"].set, "the old key stays");
    let detail = session.view().detail.unwrap();
    assert_eq!(detail.notice.unwrap().event, "save-failed");
    assert_eq!(
        detail.values["url"],
        text("https://new.example"),
        "the form keeps what's typed"
    );
    assert!(detail.secrets[0].set);
    td.shutdown().await;
}

/// Closing the setup page while its Save waits for a blocking thread cancels the enable.
#[test]
fn closing_setup_while_its_save_is_queued_does_not_enable() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut td = InProcessCompanion::start().await;
        let session = IntegrationsSession::new(td.handle.clone());
        assert!(session.flip("cliproxyapi", true).await);
        session.set_value("url", text("https://hub.example"));
        session.set_value("interval_s", text("120"));
        session.enter_secret("management_key", KEY);

        // Hold the only blocking thread, so the Save waits in its queue.
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (started, running) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            held.recv().unwrap();
        });
        running.await.unwrap();
        let saving = tokio::spawn({
            let session = session.clone();
            async move { session.save().await }
        });
        until(&session, "the queued save", |v| {
            v.detail
                .as_ref()
                .is_some_and(|d| d.busy == Some(Busy::Save))
        })
        .await;
        session.close();
        release.send(()).unwrap();
        blocker.await.unwrap();
        saving.await.unwrap();
        assert!(!td.handle.integrations().enabled("cliproxyapi"));
        assert!(!on(&session, "cliproxyapi"));
        td.shutdown().await;
    });
}

/// A view sent to the window is never older than one sent before it: a change waits while an
/// earlier change's view is on its way.
#[test]
fn publication_never_runs_backwards() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut td = InProcessCompanion::start().await;
        set_up_cliproxyapi(&td).await;
        let session = IntegrationsSession::new(td.handle.clone());
        assert!(session.open("cliproxyapi").await);
        let points = td.checkpoints().clone();
        points.hold_once(Checkpoint::IntegrationsPublish);
        let first = std::thread::spawn({
            let session = session.clone();
            move || session.set_value("interval_s", text("111"))
        });
        td.until(|_| points.waiting(Checkpoint::IntegrationsPublish))
            .await;
        let (done, finished) = std::sync::mpsc::channel();
        let second = std::thread::spawn({
            let session = session.clone();
            move || {
                session.set_value("interval_s", text("222"));
                done.send(()).unwrap();
            }
        });
        assert!(
            finished.recv_timeout(Duration::from_millis(500)).is_err(),
            "the newer change waits for the older one's view to be sent"
        );
        points.release(Checkpoint::IntegrationsPublish, false);
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(
            session.view().detail.unwrap().values["interval_s"],
            text("222")
        );
        td.shutdown().await;
    });
}
