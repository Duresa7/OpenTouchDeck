//! Setup and user changes cross the same interface a native window uses.
use serde_json::json;
use std::collections::BTreeMap;
use td_companion::{companion::Change, integrations::Error, testing::InProcessCompanion};

fn key() -> BTreeMap<String, String> {
    BTreeMap::from([("management_key".into(), "fixture-private-value".into())])
}

#[tokio::test]
async fn setup_saves_the_origin_then_the_secret_then_enables_and_publishes() {
    let mut td = InProcessCompanion::start().await;
    let integrations = td.handle.integrations();
    let mut changes = td.handle.observe();
    let mut setup = integrations.setup("cliproxyapi", true).unwrap();
    assert!(!integrations.enabled("cliproxyapi"));
    let saved = setup
        .save(json!({"url":"https://hub.example","interval_s":120}), key())
        .unwrap();
    assert!(saved.summary.enabled);
    assert!(saved.secrets["management_key"].set);
    assert_eq!(saved.summary.status.state, "ok");
    assert_eq!(saved.config["url"], "https://hub.example");
    assert!(integrations.enabled("cliproxyapi"));
    assert!(
        td.handle
            .snapshot()
            .integrations
            .iter()
            .any(|s| s.id == "cliproxyapi" && s.enabled)
    );
    // There is no intermediate notification of the new config without its secret.
    let mut published = false;
    while let Ok(change) = changes.try_recv() {
        if let Change::IntegrationStatus(event) = change
            && event.0.id == "cliproxyapi"
        {
            assert!(event.0.enabled);
            assert_ne!(event.0.status.state, "not_configured");
            published = true;
        }
    }
    assert!(published);
    let view = integrations.view("cliproxyapi").unwrap();
    assert!(view.secrets["management_key"].set);
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("fixture-private-value")
    );
    td.shutdown().await;
}

#[tokio::test]
async fn cancelled_or_dropped_setup_cannot_enable_on_a_later_save() {
    let mut td = InProcessCompanion::start().await;
    let integrations = td.handle.integrations();
    integrations.setup("cliproxyapi", true).unwrap().cancel();
    drop(integrations.setup("cliproxyapi", true).unwrap());
    let mut settings = integrations.setup("cliproxyapi", false).unwrap();
    let saved = settings
        .save(
            json!({"enabled":true,"url":"https://hub.example","interval_s":120}),
            key(),
        )
        .unwrap();
    assert!(!saved.summary.enabled, "a form switch is not setup intent");
    integrations.refresh("cliproxyapi").unwrap();
    assert!(!integrations.view("cliproxyapi").unwrap().summary.enabled);
    assert!(
        integrations
            .set_enabled("cliproxyapi", true)
            .unwrap()
            .summary
            .enabled
    );
    td.shutdown().await;
}

#[tokio::test]
async fn incomplete_or_failed_setup_stays_off_and_can_be_corrected() {
    let mut td = InProcessCompanion::start().await;
    let integrations = td.handle.integrations();
    let mut setup = integrations.setup("cliproxyapi", true).unwrap();
    let config = json!({"url":"https://hub.example","interval_s":120});
    assert!(matches!(
        setup.save(config.clone(), BTreeMap::new()),
        Err(Error::ConfigInvalid)
    ));
    assert!(!integrations.enabled("cliproxyapi"));
    assert!(matches!(
        setup.save(
            config.clone(),
            BTreeMap::from([("management_key".into(), "".into())])
        ),
        Err(Error::ConfigInvalid)
    ));
    assert!(!integrations.enabled("cliproxyapi"));
    assert!(setup.save(config.clone(), key()).unwrap().summary.enabled);
    // Successful completion consumes intent: another client can turn it off while the
    // same settings window stays open, and its next save must preserve that switch.
    integrations.set_enabled("cliproxyapi", false).unwrap();
    assert!(!setup.save(config, BTreeMap::new()).unwrap().summary.enabled);
    td.shutdown().await;
}

#[tokio::test]
async fn stale_form_saves_preserve_either_switch_value_through_the_handle() {
    let mut td = InProcessCompanion::start().await;
    let integrations = td.handle.integrations();
    integrations
        .configure(
            "cliproxyapi",
            json!({"url":"https://hub.example","interval_s":120}),
        )
        .unwrap();
    integrations
        .secret_put("cliproxyapi", "management_key", "fixture-private-value")
        .unwrap();
    for enabled in [true, false] {
        integrations.set_enabled("cliproxyapi", !enabled).unwrap();
        let mut stale = integrations.view("cliproxyapi").unwrap().config;
        stale.as_object_mut().unwrap().remove("enabled");
        stale["interval_s"] = json!(300);
        // The other window changes the switch after this form was loaded.
        integrations.set_enabled("cliproxyapi", enabled).unwrap();
        let saved = integrations.configure("cliproxyapi", stale).unwrap();
        assert_eq!(saved.summary.enabled, enabled);
        assert_eq!(saved.config["interval_s"], 300);
        assert!(saved.secrets["management_key"].set);
        assert_eq!(integrations.enabled("cliproxyapi"), enabled);
    }
    td.shutdown().await;
}

#[tokio::test]
async fn missing_integrations_and_invalid_writes_keep_the_public_error_contract() {
    let mut td = InProcessCompanion::start().await;
    let integrations = td.handle.integrations();
    assert!(matches!(
        integrations.setup("missing", true),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        integrations.configure("missing", json!({})),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        integrations.set_enabled("missing", true),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        integrations.secret_put("missing", "key", "value"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        integrations.secret_delete("missing", "key"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        integrations.refresh("missing"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        integrations.configure("this_pc", json!(null)),
        Err(Error::ConfigInvalid)
    ));
    td.shutdown().await;
}
