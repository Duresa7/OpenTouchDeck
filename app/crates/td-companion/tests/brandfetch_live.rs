//! The opt-in live check against the real Brandfetch, with your own keys. It never runs in CI:
//! it's ignored, and it skips itself when the keys are missing. Run it by hand from `app/`:
//!
//! ```text
//! TOUCHDECK_ENV_FILE=../.env cargo test -p td-companion --test brandfetch_live -- --ignored --nocapture
//! ```
//!
//! `TOUCHDECK_ENV_FILE` names a file with `BRANDFETCH_CLIENT_ID=` and `BRANDFETCH_API_KEY=` lines,
//! like the repository's `.env.example`. A run makes two Brandfetch calls: one Brand Search query
//! and one Brand API pick, rendered into an Asset. The pick is for brandfetch.com, which Brandfetch
//! doesn't count against the quota. It prints only pass or fail, sizes and formats: never a key, a
//! client ID or a URL.
use serde_json::{Value, json};
use td_companion::{
    api::NoQuery,
    features::{
        assets::{Create, CreateAsset},
        brandfetch::{ApiKeyBody, KeyPut},
    },
    jobs::JobState,
    testing::TestCompanion,
};

/// The pick's brand. Brandfetch doesn't count Brand API lookups of its own domain.
const BRAND: &str = "brandfetch.com";
/// The search query: a brand everyone knows.
const QUERY: &str = "spotify";

/// The client ID and API key from the file `TOUCHDECK_ENV_FILE` names, if both are set.
fn keys() -> Option<(String, String)> {
    let text = std::fs::read_to_string(std::env::var_os("TOUCHDECK_ENV_FILE")?).ok()?;
    let mut client_id = String::new();
    let mut key = String::new();
    for line in text.lines() {
        let Some((name, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']).to_owned();
        match name.trim() {
            "BRANDFETCH_CLIENT_ID" => client_id = value,
            "BRANDFETCH_API_KEY" => key = value,
            _ => {}
        }
    }
    (!client_id.is_empty() && !key.is_empty()).then_some((client_id, key))
}

/// One Brand Search query, as the UI sends it. Says how many hits came back.
fn search(client_id: &str) -> Result<String, String> {
    let mut url = url::Url::parse("https://api.brandfetch.io/v2/search/").unwrap();
    url.path_segments_mut().unwrap().pop_if_empty().push(QUERY);
    url.query_pairs_mut()
        .append_pair("c", client_id)
        .append_pair("limit", "5");
    // Errors stay vague on purpose: the transport's would quote the URL, client ID and all.
    let mut response = ureq::get(url.as_str())
        .config()
        .https_only(true)
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|_| "no answer".to_owned())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let hits: Vec<Value> = response
        .body_mut()
        .read_to_string()
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| "unreadable answer".to_owned())?;
    let icons = hits.iter().filter(|h| h["icon"].is_string()).count();
    if hits.is_empty() {
        return Err("no hits".into());
    }
    Ok(format!("{} hits, {icons} with an icon", hits.len()))
}

/// One pick through the Companion, live. Says the original's format and size and the Asset's.
async fn pick(key: String) -> Result<String, String> {
    let td = TestCompanion::start().await;
    td.brandfetch().use_live();
    td.call::<KeyPut>((), NoQuery, ApiKeyBody { value: key })
        .await
        .map_err(|_| "the key wasn't stored".to_owned())?;
    let asset: CreateAsset = serde_json::from_value(json!({"source": {
        "kind": "brandfetch", "brand": BRAND, "logo_type": "icon", "theme": "auto",
        "padding": 0, "background": null, "tint": null,
    }}))
    .unwrap();
    let job = td
        .call::<Create>((), NoQuery, asset)
        .await
        .map_err(|_| "the Companion refused the pick".to_owned())?;
    let done = td.wait(&job).await;
    if done.state != JobState::Succeeded {
        return Err(done.error.map(|e| e.code).unwrap_or_default());
    }
    let result = done.result.unwrap_or_default();
    let dir = td.data_dir().join("brandfetch");
    let book: Value =
        serde_json::from_slice(&std::fs::read(dir.join("provenance.json")).unwrap()).unwrap();
    let record = book["logos"]
        .as_object()
        .and_then(|logos| logos.values().next())
        .cloned()
        .unwrap_or_default();
    let format = record["format"].as_str().unwrap_or("?");
    let original = dir.join("originals").join(format!(
        "{}.{format}",
        record["original"].as_str().unwrap_or("")
    ));
    let original = std::fs::metadata(original).map(|m| m.len()).unwrap_or(0);
    Ok(format!(
        "{format} original, {original} bytes; Asset {} {}x{}, {} bytes",
        result["kind"].as_str().unwrap_or("?"),
        result["width"],
        result["height"],
        result["bytes"],
    ))
}

#[tokio::test]
#[ignore = "calls the real Brandfetch with your own keys; run by hand with TOUCHDECK_ENV_FILE"]
async fn live_search_and_pick() {
    let Some((client_id, key)) = keys() else {
        println!("live check skipped (no key yet)");
        return;
    };
    let searched = tokio::task::spawn_blocking(move || search(&client_id))
        .await
        .unwrap();
    let picked = pick(key).await;
    for (name, outcome) in [("search", &searched), ("pick", &picked)] {
        match outcome {
            Ok(detail) => println!("{name}: pass ({detail})"),
            Err(why) => println!("{name}: fail ({why})"),
        }
    }
    let pass = searched.is_ok() && picked.is_ok();
    println!("live check: {}", if pass { "pass" } else { "fail" });
    assert!(pass);
}
