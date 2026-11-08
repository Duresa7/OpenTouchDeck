use futures_util::StreamExt;
use serde_json::{Value, json};
use td_companion::{api::NoQuery, features::integrations::*, testing::TestCompanion};
fn path() -> IntegrationPath {
    IntegrationPath {
        integration: "cliproxyapi".into(),
    }
}
fn secret() -> SecretPath {
    SecretPath {
        integration: "cliproxyapi".into(),
        field: "management_key".into(),
    }
}
fn config(url: &str) -> Value {
    json!({"enabled":true,"url":url,"interval_s":120})
}

#[tokio::test]
async fn routes_preserve_same_origin_and_clear_changed_origins_and_emit_safe_status() {
    let td = TestCompanion::start().await;
    let list = td.call::<List>((), NoQuery, ()).await.unwrap();
    assert_eq!(list[0].id, "cliproxyapi");
    assert_eq!(list[0].status.state, "not_configured");
    assert!(!list[0].presets.is_empty());
    for next in [
        "http://hub.example",
        "https://other.example",
        "https://hub.example:8443",
    ] {
        td.call::<ConfigPut>(path(), NoQuery, config("https://hub.example"))
            .await
            .unwrap();
        let mut events = td.events().await;
        let hint = td
            .call::<SecretPut>(
                secret(),
                NoQuery,
                SecretInput {
                    value: "fixture-private-value".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(hint.hint, "Set");
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), events.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = message.to_text().unwrap();
        assert!(!text.contains("fixture-private-value"));
        let event: Value = serde_json::from_str(text).unwrap();
        assert_eq!(event["type"], "integration.status");
        assert_eq!(event["data"]["id"], "cliproxyapi");
        let same = td
            .call::<ConfigPut>(path(), NoQuery, config("https://hub.example:443/path"))
            .await
            .unwrap();
        assert!(same.secrets["management_key"].set);
        assert!(
            !serde_json::to_string(&same)
                .unwrap()
                .contains("fixture-private-value")
        );
        let (refresh, changed) = tokio::join!(
            td.call::<Refresh>(path(), NoQuery, ()),
            td.call::<ConfigPut>(path(), NoQuery, config(next))
        );
        refresh.unwrap();
        assert!(!changed.unwrap().secrets["management_key"].set);
        assert!(!td.call::<Get>(path(), NoQuery, ()).await.unwrap().secrets["management_key"].set);
    }
    td.call::<SecretPut>(secret(), NoQuery, SecretInput { value: "ab".into() })
        .await
        .unwrap();
    td.call::<SecretDelete>(secret(), NoQuery, ())
        .await
        .unwrap();
    assert!(!td.call::<Get>(path(), NoQuery, ()).await.unwrap().secrets["management_key"].set);
}
#[tokio::test]
async fn sync_contains_safe_integration_summaries() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let td = TestCompanion::start().await;
    td.call::<SecretPut>(
        secret(),
        NoQuery,
        SecretInput {
            value: "fixture-private-value".into(),
        },
    )
    .await
    .unwrap();
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
    let message = socket.next().await.unwrap().unwrap();
    let text = message.to_text().unwrap();
    assert!(!text.contains("fixture-private-value"));
    let frame: Value = serde_json::from_str(text).unwrap();
    assert_eq!(frame["type"], "sync");
    assert_eq!(frame["data"]["integrations"][0]["id"], "cliproxyapi");
}
#[tokio::test]
async fn invalid_config_unknown_fields_and_unauthenticated_calls_are_rejected() {
    let td = TestCompanion::start().await;
    let client = reqwest::Client::new();
    for body in [
        json!({"enabled":true,"url":"https://hub.example","interval_s":120,"management_key":"fixture-private-value"}),
        config("https://user:fixture-private-value@hub.example"),
    ] {
        let response = client
            .put(format!("{}/api/v1/integrations/cliproxyapi/config", td.url))
            .bearer_auth(&td.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
        let text = response.text().await.unwrap();
        assert!(!text.contains("fixture-private-value"));
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["error"]["code"],
            "integration.config_invalid"
        );
    }
    assert_eq!(
        td.call::<Get>(
            IntegrationPath {
                integration: "missing".into()
            },
            NoQuery,
            ()
        )
        .await
        .err()
        .unwrap()
        .status()
        .unwrap(),
        404
    );
    for (method, suffix, body) in [
        ("GET", "", Value::Null),
        ("GET", "/cliproxyapi", Value::Null),
        ("PUT", "/cliproxyapi/config", config("https://hub.example")),
        ("PUT", "/cliproxyapi/enabled", json!({"enabled":false})),
        (
            "PUT",
            "/cliproxyapi/secrets/management_key",
            json!({"value":"fixture-private-value"}),
        ),
        ("DELETE", "/cliproxyapi/secrets/management_key", Value::Null),
        ("POST", "/cliproxyapi/refresh", Value::Null),
    ] {
        let response = client
            .request(
                method.parse().unwrap(),
                format!("{}/api/v1/integrations{suffix}", td.url),
            )
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
}

#[tokio::test]
async fn this_pc_is_listed_with_streams_and_presets() {
    let td = TestCompanion::start().await;
    let list = td.call::<List>((), NoQuery, ()).await.unwrap();
    let pc = list.iter().find(|entry| entry.id == "this_pc").unwrap();
    assert_eq!(pc.streams.len(), 10);
    assert_eq!(pc.presets.len(), 10);
    let view = td
        .call::<Get>(
            IntegrationPath {
                integration: "this_pc".into(),
            },
            NoQuery,
            (),
        )
        .await
        .unwrap();
    assert_eq!(view.config, json!({"enabled": true}));
    assert!(view.secrets.is_empty());
    assert_eq!(view.summary.presets[0].name, "CPU usage");
}

#[tokio::test]
async fn local_stub_accounts_revision_and_wire_outputs_are_safe() {
    if std::env::var_os("TD_ACCOUNT_DETAIL_CHILD").is_none() {
        let output = tokio::task::spawn_blocking(|| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "local_stub_accounts_revision_and_wire_outputs_are_safe",
                    "--nocapture",
                ])
                .env("TD_ACCOUNT_DETAIL_CHILD", "1")
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        let logs = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for secret in [
            "AX17-private-BX29",
            "CX31-private-DX43",
            "EX59-private-FX61",
            "GX73-private-HX89",
            "IX97-private-JX03",
        ] {
            for part in [secret, &secret[..4], &secret[secret.len() - 4..]] {
                assert!(!logs.contains(part));
            }
        }
        assert!(output.status.success(), "{logs}");
        return;
    }

    use axum::{
        Json, Router,
        routing::{get, post},
    };
    use std::sync::{Arc, Mutex};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let paths = Arc::new(Mutex::new(Vec::new()));
    let auth_paths = paths.clone();
    let usage_paths = paths.clone();
    let unexpected_paths = paths.clone();
    let router = Router::new()
        .route("/v0/management/auth-files", get(move || async move {
            auth_paths.lock().unwrap().push("GET /v0/management/auth-files");
            Json(json!({"files":[
                {"provider":"codex","label":"Work","status":"active","auth_index":"AX17-private-BX29","success":128,"failed":2,"recent_requests":[{"success":5,"failed":1}],"id_token":{"chatgpt_account_id":"CX31-private-DX43"}},
                {"provider":"claude","label":"Personal","status":"active","auth_index":"EX59-private-FX61","success":64,"failed":0}
            ]}))
        }))
        .route("/v0/management/api-call", post(move |Json(body): Json<Value>| async move {
            usage_paths.lock().unwrap().push("POST /v0/management/api-call");
            assert_eq!(body["method"], "GET");
            let url = body["url"].as_str().unwrap();
            assert!([td_host::sources::cliproxy::CODEX_USAGE, td_host::sources::cliproxy::CLAUDE_USAGE].contains(&url));
            let codex = url == td_host::sources::cliproxy::CODEX_USAGE;
            let auth_index = ["auth_index", "authIndex", "AuthIndex"].into_iter()
                .filter_map(|field| body[field].as_str()).map(str::trim)
                .find(|index| !index.is_empty());
            let expected_index = if codex { "AX17-private-BX29" } else { "EX59-private-FX61" };
            // Enforce the known-good served management center's usage request profile.
            // This fixture does not establish which headers live providers require.
            let expected_header = if codex {
                json!({"Authorization":"Bearer $TOKEN$","Content-Type":"application/json","User-Agent":"codex_cli_rs/0.76.0 (Debian 13.0.0; x86_64) WindowsTerminal","Chatgpt-Account-Id":"CX31-private-DX43"})
            } else {
                json!({"Authorization":"Bearer $TOKEN$","Content-Type":"application/json","anthropic-beta":"oauth-2025-04-20"})
            };
            if auth_index != Some(expected_index) || body["header"] != expected_header {
                return Json(json!({"status_code":401,"body":""}));
            }
            if url == td_host::sources::cliproxy::CODEX_USAGE {
                Json(json!({"status_code":200,"body":json!({"plan_type":"pro","rate_limit":{"primary_window":{"used_percent":42,"reset_at":1800000000},"secondary_window":{"used_percent":10,"reset_at":1800500000}}}).to_string()}))
            } else {
                Json(json!({"status_code":403,"body":"GX73-private-HX89"}))
            }
        }));
    let router = router.fallback(move || async move {
        unexpected_paths.lock().unwrap().push("unexpected request");
        axum::http::StatusCode::NOT_FOUND
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let td = TestCompanion::start().await;
    td.call::<ConfigPut>(path(), NoQuery, config(&url))
        .await
        .unwrap();
    td.call::<SecretPut>(
        secret(),
        NoQuery,
        SecretInput {
            value: "IX97-private-JX03".into(),
        },
    )
    .await
    .unwrap();
    let before = td.call::<Get>(path(), NoQuery, ()).await.unwrap();
    assert!(before.detail.is_none());
    let mut events = td.events().await;
    td.call::<Refresh>(path(), NoQuery, ()).await.unwrap();
    let after = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let view = td.call::<Get>(path(), NoQuery, ()).await.unwrap();
            if view.detail.is_some() {
                break view;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let detail = after.detail.as_ref().unwrap();
    assert!(after.summary.revision > before.summary.revision);
    assert_eq!(
        serde_json::to_value(&detail.accounts).unwrap(),
        json!([
            {"provider":"codex","account":"Work","status":"active","success":128,"failed":2,"recent_10m":6,"plan":"pro","five_hour":{"pct":42.0,"resets_at_ms":1800000000000i64},"seven_day":{"pct":10.0,"resets_at_ms":1800500000000i64},"error":null},
            {"provider":"claude","account":"Personal","status":"active","success":64,"failed":0,"recent_10m":0,"plan":"Claude","five_hour":null,"seven_day":null,"error":"The service refused this account's sign-in (HTTP 403). Sign in to it again in CLIProxyAPI."}
        ])
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event_text = event.to_text().unwrap();
    let event: Value = serde_json::from_str(event_text).unwrap();
    assert_eq!(event["type"], "integration.status");
    assert_eq!(event["data"]["revision"], after.summary.revision);
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
    let sync_text = sync.to_text().unwrap();
    let frame: Value = serde_json::from_str(sync_text).unwrap();
    assert_eq!(
        frame["data"]["integrations"][0]["revision"],
        after.summary.revision
    );
    let view = serde_json::to_string(&after).unwrap();
    for output in [&view, event_text, sync_text] {
        for secret in [
            "AX17-private-BX29",
            "CX31-private-DX43",
            "EX59-private-FX61",
            "GX73-private-HX89",
            "IX97-private-JX03",
            &td.token,
        ] {
            for part in [secret, &secret[..4], &secret[secret.len() - 4..]] {
                assert!(!output.contains(part));
            }
        }
        assert!(!output.contains("key_hint"));
    }
    assert_eq!(
        after
            .summary
            .presets
            .iter()
            .map(|p| (p.id.as_str(), p.name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("status", "Account status"),
            ("success", "Requests OK"),
            ("failed", "Requests failed"),
            ("recent_10m", "Requests 10 min"),
            ("five_hour_pct", "5-hour used"),
            ("seven_day_pct", "7-day used"),
            ("five_hour_reset", "5-hour reset in"),
            ("seven_day_reset", "7-day reset in"),
            ("plan", "Plan")
        ]
    );
    assert_eq!(
        paths.lock().unwrap().as_slice(),
        [
            "GET /v0/management/auth-files",
            "POST /v0/management/api-call",
            "POST /v0/management/api-call"
        ]
    );
    socket.close(None).await.unwrap();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn custom_list_and_get_publish_schema_and_number_presets() {
    let td = TestCompanion::start().await;
    assert!(
        td.call::<List>((), NoQuery, ())
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == "custom")
    );
    let path = || IntegrationPath {
        integration: "custom".into(),
    };
    assert_eq!(
        td.call::<Get>(path(), NoQuery, ()).await.unwrap().config,
        json!({"enabled":false,"sources":[]})
    );
    let view = td.call::<ConfigPut>(path(), NoQuery, json!({"enabled":false,"sources":[{"id":"temperature","name":"Temperature","kind":"number","interval_s":1,"input":{"type":"http","url":"https://example.invalid/metrics","value_path":"$.value"}}]})).await.unwrap();
    assert_eq!(view.summary.presets.len(), 2);
    assert_eq!(view.summary.streams[0].id, "temperature");
}

fn integration(id: &str) -> IntegrationPath {
    IntegrationPath {
        integration: id.into(),
    }
}
/// Reads events until `integration.status` reports the Integration `enabled` as asked.
async fn status_event<S>(events: &mut S, id: &str, enabled: bool)
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let message = events.next().await.unwrap().unwrap();
            let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            if event["type"] == "integration.status"
                && event["data"]["id"] == id
                && event["data"]["enabled"] == enabled
            {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no integration.status with {id} enabled {enabled}"));
}

#[tokio::test]
async fn enabled_put_switches_cliproxyapi_and_keeps_url_interval_and_management_key() {
    let td = TestCompanion::start().await;
    td.call::<ConfigPut>(path(), NoQuery, config("https://hub.example"))
        .await
        .unwrap();
    td.call::<SecretPut>(
        secret(),
        NoQuery,
        SecretInput {
            value: "fixture-private-value".into(),
        },
    )
    .await
    .unwrap();
    let mut events = td.events().await;
    for enabled in [false, true] {
        let view = td
            .call::<EnabledPut>(path(), NoQuery, EnabledInput { enabled })
            .await
            .unwrap();
        status_event(&mut events, "cliproxyapi", enabled).await;
        for view in [view, td.call::<Get>(path(), NoQuery, ()).await.unwrap()] {
            assert_eq!(view.summary.enabled, enabled);
            assert_eq!(
                view.config,
                json!({"enabled":enabled,"url":"https://hub.example","interval_s":120})
            );
            assert!(view.secrets["management_key"].set);
            assert!(
                !serde_json::to_string(&view)
                    .unwrap()
                    .contains("fixture-private-value")
            );
        }
    }
    let list = td.call::<List>((), NoQuery, ()).await.unwrap();
    assert!(list.iter().any(|s| s.id == "cliproxyapi" && s.enabled));
}

#[tokio::test]
async fn enabled_put_changes_only_enabled_on_the_other_integrations() {
    let td = TestCompanion::start().await;
    td.call::<ConfigPut>(
        integration("custom"),
        NoQuery,
        json!({"enabled":true,"sources":[{"id":"temperature","name":"Temperature","kind":"number","interval_s":60,"input":{"type":"http","url":"https://example.invalid/metrics","value_path":"$.value","headers":{"Authorization":"auth"}}}]}),
    )
    .await
    .unwrap();
    td.call::<SecretPut>(
        SecretPath {
            integration: "custom".into(),
            field: "auth".into(),
        },
        NoQuery,
        SecretInput {
            value: "fixture-private-value".into(),
        },
    )
    .await
    .unwrap();
    let mut events = td.events().await;
    for id in ["this_pc", "volume", "media", "custom"] {
        let before = td.call::<Get>(integration(id), NoQuery, ()).await.unwrap();
        assert!(before.summary.enabled, "{id}");
        for enabled in [false, true] {
            let view = td
                .call::<EnabledPut>(integration(id), NoQuery, EnabledInput { enabled })
                .await
                .unwrap();
            status_event(&mut events, id, enabled).await;
            let mut expected = before.config.clone();
            expected["enabled"] = enabled.into();
            assert_eq!(view.summary.enabled, enabled, "{id}");
            assert_eq!(view.config, expected, "{id}");
            assert_eq!(
                serde_json::to_value(&view.secrets).unwrap(),
                serde_json::to_value(&before.secrets).unwrap(),
                "{id}"
            );
        }
    }
    let custom = td
        .call::<Get>(integration("custom"), NoQuery, ())
        .await
        .unwrap();
    assert!(custom.secrets["auth"].set);
}

#[tokio::test]
async fn config_put_without_enabled_keeps_the_stored_value() {
    let td = TestCompanion::start().await;
    for enabled in [true, false] {
        td.call::<ConfigPut>(
            path(),
            NoQuery,
            json!({"enabled":enabled,"url":"https://hub.example","interval_s":120}),
        )
        .await
        .unwrap();
        let view = td
            .call::<ConfigPut>(
                path(),
                NoQuery,
                json!({"url":"https://hub.example/next","interval_s":300}),
            )
            .await
            .unwrap();
        assert_eq!(
            view.config,
            json!({"enabled":enabled,"url":"https://hub.example/next","interval_s":300})
        );
        assert_eq!(view.summary.enabled, enabled);
    }
    for enabled in [false, true] {
        td.call::<EnabledPut>(integration("this_pc"), NoQuery, EnabledInput { enabled })
            .await
            .unwrap();
        let view = td
            .call::<ConfigPut>(integration("this_pc"), NoQuery, json!({}))
            .await
            .unwrap();
        assert_eq!(view.config, json!({"enabled":enabled}));
    }
    let view = td
        .call::<ConfigPut>(integration("this_pc"), NoQuery, json!({"enabled":false}))
        .await
        .unwrap();
    assert_eq!(view.config, json!({"enabled":false}));
    let custom = td
        .call::<ConfigPut>(integration("custom"), NoQuery, json!({"sources":[]}))
        .await
        .unwrap();
    assert_eq!(custom.config, json!({"enabled":false,"sources":[]}));
}

#[tokio::test]
async fn a_form_save_after_another_clients_switch_keeps_cliproxyapi_off() {
    let td = TestCompanion::start().await;
    td.call::<ConfigPut>(path(), NoQuery, config("https://hub.example"))
        .await
        .unwrap();
    td.call::<SecretPut>(
        secret(),
        NoQuery,
        SecretInput {
            value: "fixture-private-value".into(),
        },
    )
    .await
    .unwrap();
    // Client A turns it off.
    td.call::<EnabledPut>(path(), NoQuery, EnabledInput { enabled: false })
        .await
        .unwrap();
    // Client B still shows the form it loaded while it was on, and saves the URL and interval.
    let response = reqwest::Client::new()
        .put(format!("{}/api/v1/integrations/cliproxyapi/config", td.url))
        .bearer_auth(&td.token)
        .json(&json!({"url":"https://hub.example","interval_s":300}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let saved: Value = response.json().await.unwrap();
    assert_eq!(saved["summary"]["enabled"], false);
    let view = td.call::<Get>(path(), NoQuery, ()).await.unwrap();
    assert!(!view.summary.enabled);
    assert_eq!(
        view.config,
        json!({"enabled":false,"url":"https://hub.example","interval_s":300})
    );
    assert!(view.secrets["management_key"].set);
}

#[tokio::test]
async fn enabled_put_rejects_a_missing_integration_and_any_other_body() {
    let td = TestCompanion::start().await;
    let client = reqwest::Client::new();
    let put = |suffix: &str, body: Value| {
        client
            .put(format!("{}/api/v1/integrations{suffix}", td.url))
            .bearer_auth(&td.token)
            .json(&body)
            .send()
    };
    let response = put("/missing/enabled", json!({"enabled":true}))
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "integration.not_found");
    for body in [
        json!({"enabled":true,"url":"https://other.example"}),
        json!({"enabled":"true"}),
        json!({}),
        json!(true),
    ] {
        let response = put("/cliproxyapi/enabled", body).await.unwrap();
        assert_eq!(response.status(), 400);
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], "request.invalid");
    }
    let view = td.call::<Get>(path(), NoQuery, ()).await.unwrap();
    assert!(!view.summary.enabled);
    assert_eq!(view.config["url"], "http://localhost:8317");
}
