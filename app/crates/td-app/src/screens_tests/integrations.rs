use super::*;
use crate::Integrations;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};

const KEY: &str = "fixture-private-value";

fn page(rig: &Rig) -> crate::IntegrationPage {
    rig.app.ui.global::<Integrations>().get_page()
}

fn page_id(rig: &Rig) -> String {
    rig.app.ui.global::<Integrations>().get_page_id().into()
}

fn enabled(rig: &Rig, id: &str) -> bool {
    rig.handle().integrations().enabled(id)
}

fn secret_set(rig: &Rig, id: &str, field: &str) -> bool {
    rig.handle().integrations().view(id).unwrap().secrets[field].set
}

/// Types into a field without committing it, as a person typing does.
fn type_in(rig: &Rig, label: &str, text: &str) {
    rig.one(label, AccessibleRole::TextInput)
        .set_accessible_value(text);
}

fn flip(rig: &Rig, name: &str) {
    rig.one(name, AccessibleRole::Switch)
        .invoke_accessible_default_action();
}

fn checked(rig: &Rig, name: &str) -> bool {
    rig.one(name, AccessibleRole::Switch)
        .accessible_checked()
        .unwrap()
}

fn open_list(rig: &Rig) {
    rig.app.ui.set_page(Page::Integrations);
    rig.until("the Integrations list", |r| {
        r.has("CLIProxyAPI settings") && r.has("Custom settings")
    });
}

/// Presses Save once the form has something to save.
fn save(rig: &Rig) {
    rig.until("Save to be enabled", |r| {
        r.one("Save", AccessibleRole::Button).accessible_enabled() == Some(true)
    });
    rig.press("Save");
}

fn back(rig: &Rig) {
    rig.press("Integrations");
    rig.until("the list", |r| page_id(r).is_empty());
}

#[test]
fn the_rail_opens_integrations_between_settings_and_about() {
    let rig = Rig::start(960.0);
    let tabs: Vec<String> = rig
        .app
        .ui
        .root_element()
        .query_descendants()
        .match_accessible_role(AccessibleRole::Tab)
        .find_all()
        .into_iter()
        .map(|tab| tab.accessible_label().unwrap().into())
        .collect();
    assert_eq!(tabs, ["Home", "Setup", "Settings", "Integrations", "About"]);
    rig.one("Integrations", AccessibleRole::Tab)
        .invoke_accessible_default_action();
    assert_eq!(rig.app.ui.get_page(), Page::Integrations);
    rig.until("the list", |r| r.has("CLIProxyAPI"));

    // The five Integrations, then the icon sources.
    let list = rig.one("Integrations", AccessibleRole::List);
    assert_eq!(list.accessible_item_count(), Some(5));
    for name in ["CLIProxyAPI", "This PC", "Media", "Volume", "Custom"] {
        rig.one(name, AccessibleRole::Switch);
    }
    assert_eq!(
        rig.one("Icons", AccessibleRole::List)
            .accessible_item_count(),
        Some(2)
    );

    // Their pages are slots of their own, and the way back leads to the list.
    rig.press("Open Icon packs");
    rig.until("the Icon packs page", |r| page_id(r) == "icon-packs");
    assert!(rig.text_containing("Downloading and importing icon packs"));
    back(&rig);
    rig.press("Open Brandfetch logos");
    rig.until("the Brandfetch page", |r| page_id(r) == "brandfetch");
    back(&rig);
}

#[test]
fn cliproxyapi_sets_up_switches_and_is_removed() {
    let rig = Rig::start(960.0);
    open_list(&rig);
    assert!(!checked(&rig, "CLIProxyAPI"));

    // On before setup opens its setup page, with the switch on and waiting.
    flip(&rig, "CLIProxyAPI");
    rig.until("the setup page", |r| r.has("Hub address"));
    assert_eq!(page_id(&rig), "cliproxyapi");
    rig.until("Finish setup", |r| page(r).header == "Finish setup");
    assert!(checked(&rig, "CLIProxyAPI"));
    assert!(!enabled(&rig, "cliproxyapi"));
    assert!(rig.text_containing("Stored in "));

    // An http:// address off this PC warns before the key would travel.
    type_in(&rig, "Hub address", "http://hub.example");
    rig.until("the warning", |r| {
        r.text_containing("isn't on your network")
    });

    // The address without its key: Save names the key, and it stays off.
    type_in(&rig, "Hub address", "https://hub.example");
    type_in(&rig, "Check every", "120");
    rig.until("Unsaved changes", |r| r.has("Unsaved changes"));
    save(&rig);
    rig.until("the missing key", |r| r.has("Enter the management key."));
    assert!(!enabled(&rig, "cliproxyapi"));

    // One Save with the key finishes setup and turns it on. The key never reaches the
    // accessibility tree.
    type_in(&rig, "Management key", KEY);
    let field = rig.one("Management key", AccessibleRole::TextInput);
    assert_eq!(field.accessible_value().unwrap(), "");
    assert!(!rig.text_containing(KEY));
    save(&rig);
    rig.until("connected", |r| page(r).header == "Running");
    assert!(rig.has("Management key: Saved"));
    assert!(enabled(&rig, "cliproxyapi"));
    assert!(secret_set(&rig, "cliproxyapi", "management_key"));
    assert!(!rig.text_containing(KEY));

    // Replace shows the field again; keeping the saved one puts "Saved" back.
    rig.press("Replace the management key");
    rig.until("the key field", |r| {
        !r.labelled("Management key").is_empty() && !r.has("Management key: Saved")
    });
    rig.press("Keep the saved one");
    rig.until("Saved", |r| r.has("Management key: Saved"));

    // Off and on again from the list keep the address and the key.
    back(&rig);
    flip(&rig, "CLIProxyAPI");
    rig.until("off", |r| {
        !enabled(r, "cliproxyapi") && !checked(r, "CLIProxyAPI")
    });
    flip(&rig, "CLIProxyAPI");
    rig.until("on", |r| {
        enabled(r, "cliproxyapi") && checked(r, "CLIProxyAPI")
    });

    // Removed: the key forgotten, after a question, and the switch off.
    rig.press("CLIProxyAPI settings");
    rig.until("the page", |r| r.has("Management key: Saved"));
    rig.press("Forget the management key");
    rig.until("the question", |r| r.has("Forget the management key?"));
    rig.press("Cancel");
    rig.until("the question to go", |r| {
        !r.has("Forget the management key?")
    });
    assert!(secret_set(&rig, "cliproxyapi", "management_key"));
    rig.press("Forget the management key");
    rig.until("the question", |r| r.has("Forget the management key?"));
    rig.press("Forget");
    rig.until("the key to go", |r| {
        !secret_set(r, "cliproxyapi", "management_key") && !r.has("Management key: Saved")
    });
    rig.until("Finish setup", |r| page(r).header == "Finish setup");
    flip(&rig, "CLIProxyAPI");
    rig.until("off", |r| !enabled(r, "cliproxyapi"));
    back(&rig);
    let view = rig.handle().integrations().view("cliproxyapi").unwrap();
    assert!(!view.summary.enabled);
    assert!(!view.secrets["management_key"].set);
    rig.until("the row", |r| !checked(r, "CLIProxyAPI"));
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

#[test]
fn custom_sets_up_switches_and_is_removed() {
    let rig = Rig::start(960.0);
    open_list(&rig);
    flip(&rig, "Custom");
    rig.until("the setup page", |r| r.has("Custom sources (JSON)"));
    let sources = rig.one("Custom sources (JSON)", AccessibleRole::TextInput);
    assert_eq!(sources.accessible_value().unwrap(), "[]");

    // JSON that doesn't parse is refused under the field.
    type_in(&rig, "Custom sources (JSON)", "[{");
    save(&rig);
    rig.until("the error", |r| r.text_containing("Enter valid JSON."));
    assert!(!enabled(&rig, "custom"));

    // The sources save and turn it on; the header they name asks for its secret.
    type_in(&rig, "Custom sources (JSON)", &http_source());
    rig.until("the header's secret", |r| !r.labelled("Auth").is_empty());
    type_in(&rig, "Auth", KEY);
    save(&rig);
    rig.until("saved", |r| r.has("Auth: Saved") && enabled(r, "custom"));
    assert!(secret_set(&rig, "custom", "auth"));
    assert!(!rig.text_containing(KEY));
    // The saved sources come back pretty-printed, over several lines of the multi-line field.
    assert!(
        rig.one("Custom sources (JSON)", AccessibleRole::TextInput)
            .accessible_value()
            .unwrap()
            .contains('\n')
    );

    // Each flip waits for the switch too: it sends the opposite of what it shows.
    back(&rig);
    rig.until("the switch on", |r| checked(r, "Custom"));
    flip(&rig, "Custom");
    rig.until("off", |r| !enabled(r, "custom") && !checked(r, "Custom"));
    flip(&rig, "Custom");
    rig.until("on", |r| enabled(r, "custom") && checked(r, "Custom"));

    rig.press("Custom settings");
    rig.until("the page", |r| r.has("Auth: Saved"));
    rig.press("Forget the auth");
    rig.until("the question", |r| r.has("Forget the auth?"));
    rig.press("Forget");
    rig.until("the secret to go", |r| !secret_set(r, "custom", "auth"));
    flip(&rig, "Custom");
    rig.until("off", |r| !enabled(r, "custom"));
    back(&rig);
    assert!(!checked(&rig, "Custom"));
}

/// The session follows the Companion inside the runtime. Started outside it, a change made
/// elsewhere would never reach the open page.
#[test]
fn the_open_page_follows_a_change_made_elsewhere() {
    let rig = Rig::start(960.0);
    let integrations = rig.handle().integrations().clone();
    integrations
        .configure(
            "cliproxyapi",
            json!({"url": "https://hub.example", "interval_s": 120}),
        )
        .unwrap();
    integrations
        .secret_put("cliproxyapi", "management_key", KEY)
        .unwrap();
    open_list(&rig);
    rig.press("CLIProxyAPI settings");
    rig.until("the address", |r| {
        r.has("Hub address")
            && r.one("Hub address", AccessibleRole::TextInput)
                .accessible_value()
                .unwrap()
                == "https://hub.example"
    });

    // Another window moves the hub: the page reads it again, and the key it forgot.
    integrations
        .configure(
            "cliproxyapi",
            json!({"url": "https://other.example", "interval_s": 120}),
        )
        .unwrap();
    rig.until("the new address", |r| {
        r.one("Hub address", AccessibleRole::TextInput)
            .accessible_value()
            .unwrap()
            == "https://other.example"
    });
}

/// A CLIProxyAPI hub on this PC with one account, answering every request the same way.
fn hub() -> String {
    // The limits reset in 3 hours and 2 days.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut length = 0;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let reply = if line.starts_with("GET /v0/management/auth-files") {
                json!({"files": [{
                    "provider": "codex",
                    "label": "Work account with a long name",
                    "status": "active",
                    "auth_index": "fixture-index",
                    "success": 1284567,
                    "failed": 2,
                    "recent_requests": [{"success": 5, "failed": 1}],
                    "id_token": {"chatgpt_account_id": "fixture-account"}
                }]})
            } else {
                let usage = json!({
                    "plan_type": "pro",
                    "rate_limit": {
                        "primary_window": {"used_percent": 42, "reset_at": now + 3 * 3600},
                        "secondary_window": {"used_percent": 10, "reset_at": now + 2 * 86400}
                    }
                });
                json!({"status_code": 200, "body": usage.to_string()})
            }
            .to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    url
}

#[test]
fn a_hub_lists_its_accounts_as_stacked_rows() {
    let rig = Rig::start(960.0);
    let integrations = rig.handle().integrations().clone();
    integrations
        .configure("cliproxyapi", json!({"url": hub(), "interval_s": 120}))
        .unwrap();
    integrations
        .secret_put("cliproxyapi", "management_key", KEY)
        .unwrap();
    integrations.set_enabled("cliproxyapi", true).unwrap();
    open_list(&rig);
    rig.press("CLIProxyAPI settings");
    rig.until("the accounts", |r| {
        r.has("Accounts") && r.has("Work account with a long name")
    });
    let list = rig.one("Accounts", AccessibleRole::List);
    assert_eq!(list.accessible_item_count(), Some(1));
    assert!(rig.has("Service: codex"));
    assert!(rig.has("Plan: pro"));
    assert!(rig.text_containing("5-hour limit: 42%. Resets in "));
    assert!(rig.text_containing("Status: Active. 1,284,567 OK · 2 failed"));
    assert!(page(&rig).header.starts_with("1 account · checked "));
}

#[test]
fn at_the_largest_text_size_integrations_fit_the_narrowest_window() {
    let rig = Rig::start(500.0);
    rig.app
        .ui
        .window()
        .set_size(LogicalSize::new(500.0, 12000.0));
    look::apply(
        &rig.app.ui,
        &look::Look {
            text_scale: 2.25,
            ..Default::default()
        },
    );
    let integrations = rig.handle().integrations().clone();
    integrations
        .configure("cliproxyapi", json!({"url": hub(), "interval_s": 120}))
        .unwrap();
    integrations
        .secret_put("cliproxyapi", "management_key", KEY)
        .unwrap();
    integrations.set_enabled("cliproxyapi", true).unwrap();
    open_list(&rig);
    fits(&rig, "Integrations");

    // A page with every kind of row: settings, a saved key with its question, checks and an
    // account.
    rig.press("CLIProxyAPI settings");
    rig.until("the accounts", |r| r.has("Work account with a long name"));
    type_in(&rig, "Hub address", "http://hub.example");
    rig.press("Forget the management key");
    rig.until("the question", |r| r.has("Forget the management key?"));
    fits(&rig, "CLIProxyAPI");
    rig.press("Cancel");
    rig.press("Replace the management key");
    rig.until("the key field", |r| !r.has("Management key: Saved"));
    fits(&rig, "CLIProxyAPI replacing its key");
    back(&rig);

    // Custom's multi-line sources.
    rig.press("Custom settings");
    rig.until("the sources", |r| r.has("Custom sources (JSON)"));
    type_in(&rig, "Custom sources (JSON)", &http_source());
    rig.until("the header's secret", |r| !r.labelled("Auth").is_empty());
    fits(&rig, "Custom");
    back(&rig);

    for page in ["Open Icon packs", "Open Brandfetch logos"] {
        rig.press(page);
        rig.until("the page", |r| !page_id(r).is_empty());
        fits(&rig, page);
        back(&rig);
    }
}

/// Every control, list row and text line ends inside the 500 px window.
fn fits(rig: &Rig, page: &str) {
    let width = 500.0;
    assert_fits(rig, width, page);
    for role in [
        AccessibleRole::ListItem,
        AccessibleRole::Text,
        AccessibleRole::List,
    ] {
        for element in rig
            .app
            .ui
            .root_element()
            .query_descendants()
            .match_accessible_role(role)
            .find_all()
        {
            let left = element.absolute_position().x;
            let right = left + element.size().width;
            let label = element.accessible_label().unwrap_or_default();
            assert!(
                left >= 0.0 && right <= width + 0.5,
                "{page}: {label:?} spans {left}..{right} in a {width} px window"
            );
        }
    }
}
