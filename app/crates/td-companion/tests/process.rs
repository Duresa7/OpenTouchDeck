use serde_json::{Value, json};
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::Duration,
};

/// Drops pooled connections before the Companion's 5 s header-read limit closes them, so a
/// request never goes out on a connection the server is closing at that moment.
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(4))
        .build()
        .unwrap()
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn launch(root: &Path) -> Process {
    Process(
        Command::new(env!("CARGO_BIN_EXE_td-companion"))
            .args(["--mock-devices", "2"])
            .env("TOUCHDECK_APP_DATA_DIR", root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    )
}
/// Waits for a Companion that should refuse to start, and returns its status and stderr.
async fn refused(mut child: Process) -> (ExitStatus, String) {
    let status = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("the Companion should exit");
    let mut stderr = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (status, stderr)
}
async fn runtime(root: &Path) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(bytes) = std::fs::read(root.join("runtime/companion.json"))
                && let Ok(value) = serde_json::from_slice(&bytes)
            {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap()
}
fn url(info: &Value) -> String {
    format!("http://127.0.0.1:{}", info["port"])
}
async fn session(client: &reqwest::Client, info: &Value) -> String {
    client
        .post(format!("{}/api/v1/auth/sessions", url(info)))
        .bearer_auth(info["token"].as_str().unwrap())
        .json(&json!({"kind": "window"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .into()
}
async fn pings(client: &reqwest::Client, info: &Value) -> bool {
    client
        .get(format!("{}/api/v1/system/ping", url(info)))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}
/// Waits until `count` Devices are connected. The scan logs each mock Device before it connects.
async fn connected(client: &reqwest::Client, info: &Value, token: &str, count: usize) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let devices: Value = client
                .get(format!("{}/api/v1/devices", url(info)))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let links = devices.as_array().unwrap().iter();
            if links.filter(|d| d["link"]["state"] == "connected").count() == count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("the mock Devices connect");
}
async fn stop(child: &mut Process, client: &reqwest::Client, info: &Value, token: &str) {
    let response = client
        .post(format!("{}/api/v1/system/shutdown", url(info)))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
}
#[cfg(windows)]
fn pwsh(script: &str, vars: &[(&str, &Path)]) -> bool {
    let mut command = Command::new("pwsh");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    for (name, value) in vars {
        command.env(name, value);
    }
    command.status().unwrap().success()
}
/// The path is owned by the user and has a protected DACL with one allow entry for the user.
#[cfg(windows)]
fn owner_only(path: &Path) -> bool {
    let script = "$ErrorActionPreference='Stop'; $acl=Get-Acl -LiteralPath $env:TD_PATH; $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; $owner=$acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value; $rules=$acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier]); if (!$acl.AreAccessRulesProtected -or $owner -ne $sid -or $rules.Count -ne 1 -or $rules[0].IdentityReference.Value -ne $sid -or $rules[0].AccessControlType -ne 'Allow') {exit 1}";
    pwsh(script, &[("TD_PATH", path)])
}

#[tokio::test]
async fn one_companion_per_runtime_folder() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let client = client();
    let mut first = launch(root.path());
    let info = runtime(root.path()).await;
    let (status, stderr) = refused(launch(root.path())).await;
    assert!(!status.success());
    assert!(
        stderr.contains("The Companion is already running."),
        "{stderr}"
    );
    assert!(pings(&client, &info).await);
    // Another data folder has its own runtime folder and lock.
    let mut elsewhere = launch(other.path());
    let other_info = runtime(other.path()).await;
    assert!(pings(&client, &other_info).await);
    let token = session(&client, &other_info).await;
    stop(&mut elsewhere, &client, &other_info, &token).await;
    let token = session(&client, &info).await;
    stop(&mut first, &client, &info, &token).await;
}

#[tokio::test]
async fn runtime_file_lifecycle_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let client = client();
    let mut first = launch(root.path());
    let info = runtime(root.path()).await;
    let token = session(&client, &info).await;
    #[cfg(windows)]
    {
        assert!(owner_only(&root.path().join("runtime/companion.json")));
        assert!(owner_only(&root.path().join("runtime")));
    }
    // Reset pairing rewrites the file with a new pairing token at once.
    let reset = client
        .post(format!("{}/api/v1/auth/reset", url(&info)))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(reset.status(), 204);
    let rotated = runtime(root.path()).await;
    assert_ne!(rotated["token"], info["token"]);
    assert_eq!(rotated["port"], info["port"]);
    let token = session(&client, &rotated).await;
    stop(&mut first, &client, &info, &token).await;
    assert!(!root.path().join("runtime/companion.json").exists());
    let mut restarted = launch(root.path());
    let next = runtime(root.path()).await;
    assert_ne!(info["token"], next["token"]);
    assert_eq!(info["port"], next["port"]);
    let old = client
        .get(format!("{}/api/v1/devices", url(&info)))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), 401);
    let token = session(&client, &next).await;
    stop(&mut restarted, &client, &next, &token).await;
}

#[cfg(windows)]
#[tokio::test]
async fn a_junction_at_runtime_refuses_start() {
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    // Both paths belong to this test's temporary folders. No shell interpolation.
    assert!(pwsh(
        "New-Item -ItemType Junction -Path $env:TD_LINK -Target $env:TD_TARGET | Out-Null",
        &[
            ("TD_LINK", &root.path().join("runtime")),
            ("TD_TARGET", target.path())
        ],
    ));
    let (status, stderr) = refused(launch(root.path())).await;
    assert!(!status.success());
    assert!(stderr.contains("reparse point"), "{stderr}");
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 0);
    std::fs::remove_dir(root.path().join("runtime")).unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn a_folder_with_only_modify_access_works() {
    // The user holds only inherited Modify here, so no WRITE_OWNER.
    let restricted = tempfile::tempdir().unwrap();
    assert!(pwsh(
        "$sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; icacls $env:TD_DIR /inheritance:r /grant:r \"*$($sid):(OI)(CI)M\" | Out-Null; exit $LASTEXITCODE",
        &[("TD_DIR", restricted.path())],
    ));
    let client = client();
    let mut companion = launch(restricted.path());
    let info = runtime(restricted.path()).await;
    assert!(owner_only(&restricted.path().join("runtime")));
    let token = session(&client, &info).await;
    stop(&mut companion, &client, &info, &token).await;
}

/// Making another principal the owner needs an elevated token, as on CI runners.
#[cfg(windows)]
#[tokio::test]
async fn a_runtime_folder_another_account_owns_refuses_start() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("runtime");
    std::fs::create_dir(&dir).unwrap();
    // S-1-5-32-544 is the local Administrators group.
    if !pwsh(
        "icacls $env:TD_DIR /setowner '*S-1-5-32-544' 2>&1 | Out-Null; exit $LASTEXITCODE",
        &[("TD_DIR", &dir)],
    ) {
        // CI runners are elevated, so the check always runs there.
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must be able to set a foreign owner"
        );
        eprintln!("skipped: setting a foreign owner needs an elevated token");
        return;
    }
    let (status, stderr) = refused(launch(root.path())).await;
    assert!(!status.success());
    assert!(stderr.contains("another account owns it"), "{stderr}");
    assert!(pwsh(
        "$acl=Get-Acl -LiteralPath $env:TD_DIR; if ($acl.Owner -notmatch 'Administrators') {exit 1}",
        &[("TD_DIR", &dir)],
    ));
    assert!(!dir.join("companion.json").exists());
}

fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    })
}
struct Pairing {
    client: reqwest::Client,
    base: String,
    window: String,
    bodies: Vec<String>,
    secrets: Vec<String>,
}
impl Pairing {
    async fn send(&mut self, request: reqwest::RequestBuilder) -> u16 {
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        self.bodies.push(response.text().await.unwrap());
        status
    }
    async fn mint(&mut self) -> String {
        let request = self
            .client
            .post(format!("{}/api/v1/auth/pair-codes", self.base))
            .bearer_auth(&self.window);
        assert_eq!(self.send(request).await, 200);
        let delivery: Value = self
            .client
            .get(format!("{}/api/v1/test/pairing-delivery", self.base))
            .bearer_auth(&self.window)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let code = delivery["notification"].as_str().unwrap().to_owned();
        let link = delivery["url"].as_str().unwrap().to_owned();
        assert!(link.ends_with(&format!("#code={code}")));
        self.secrets
            .extend([code.replace('-', ""), link, code.clone()]);
        code
    }
    async fn pair(&mut self, code: &str) -> u16 {
        let request = self
            .client
            .post(format!("{}/api/v1/auth/pair", self.base))
            .json(&json!({"code": code, "label": "Secrecy check"}));
        self.send(request).await
    }
    async fn post(&mut self, path: &str, body: Option<Value>) -> u16 {
        let mut request = self
            .client
            .post(format!("{}/api/v1{path}", self.base))
            .bearer_auth(&self.window);
        if let Some(body) = body {
            request = request.json(&body);
        }
        self.send(request).await
    }
}

#[tokio::test]
async fn pairing_codes_reach_only_the_delivery_capture() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let root = tempfile::tempdir().unwrap();
    let mut child = Process(
        Command::new(env!("CARGO_BIN_EXE_td-companion"))
            .args(["--mock-devices", "2"])
            .env("TOUCHDECK_APP_DATA_DIR", root.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = drain(child.0.stdout.take().unwrap());
    let stderr = drain(child.0.stderr.take().unwrap());
    let info = runtime(root.path()).await;
    let client = client();
    let mut p = Pairing {
        window: session(&client, &info).await,
        client: client.clone(),
        base: url(&info),
        bodies: Vec::new(),
        secrets: Vec::new(),
    };

    let first = p.mint().await;
    let mut request = format!("{}/api/v1/events", p.base.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("touchdeck.v1, touchdeck.bearer.{}", p.window)
            .parse()
            .unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let frames = tokio::spawn(async move {
        let mut frames = Vec::new();
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                frames.push(text.to_string());
            }
        }
        frames
    });
    let replacement = p.mint().await;
    assert_eq!(p.pair(&first).await, 401);
    assert_eq!(p.pair("ZZZZ-ZZZZ-ZZZZ").await, 401);
    assert_eq!(p.pair(&replacement).await, 200);
    let paired: Value = serde_json::from_str(p.bodies.last().unwrap()).unwrap();
    let browser_token = paired["token"].as_str().unwrap().to_owned();
    let expired = p.mint().await;
    assert_eq!(
        p.post("/test/clock", Some(json!({"advance_ms": 121_000})))
            .await,
        204
    );
    assert_eq!(p.pair(&expired).await, 401);
    let locked = p.mint().await;
    for _ in 0..5 {
        assert_eq!(p.pair("ZZZZ-ZZZZ-ZZZZ").await, 401);
    }
    assert_eq!(p.pair(&locked).await, 429);
    let pending = p.mint().await;
    assert_eq!(p.post("/auth/reset", None).await, 204);
    assert_eq!(p.pair(&pending).await, 401);

    let frames = tokio::time::timeout(Duration::from_secs(5), frames)
        .await
        .expect("reset closes the socket")
        .unwrap();
    assert!(frames[1].contains("\"sync\""));
    assert!(frames.last().unwrap().contains("auth.revoked"));
    let info = runtime(root.path()).await;
    let token = session(&client, &info).await;
    connected(&client, &info, &token, 2).await;
    stop(&mut child, &client, &info, &token).await;
    let logs = [stdout.join().unwrap(), stderr.join().unwrap()];
    assert!(!logs[1].is_empty(), "the mock Devices log to stderr");
    for text in logs.iter().chain(&frames) {
        assert!(!text.contains(&browser_token), "a browser token leaked");
    }
    for body in &p.bodies {
        if body.contains("\"error\"") {
            assert!(
                !body.contains(&browser_token),
                "a browser token leaked in an error"
            );
        }
    }

    let seen: Vec<&String> = p.bodies.iter().chain(&frames).chain(&logs).collect();
    assert_eq!((p.secrets.len(), p.bodies.len()), (15, 18));
    for secret in &p.secrets {
        let leaked = seen.iter().position(|text| text.contains(secret.as_str()));
        assert_eq!(leaked, None, "a pairing code or link leaked");
    }
}

#[tokio::test]
async fn the_real_device_console_flag_refuses_to_start_without_a_data_folder() {
    let child = Process(
        Command::new(env!("CARGO_BIN_EXE_td-companion"))
            .arg("--test-real-device-console")
            .env_remove("TOUCHDECK_APP_DATA_DIR")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let (status, stderr) = refused(child).await;
    assert!(!status.success());
    assert!(stderr.contains("TOUCHDECK_APP_DATA_DIR"), "{stderr}");
}

/// A Wi-Fi password set through the API reaches the Device and nothing else: no log line, no response and no event.
#[tokio::test]
async fn a_wifi_password_never_reaches_the_logs() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    const PASSWORD: &str = "dummy-pass-7319";
    let root = tempfile::tempdir().unwrap();
    let mut child = Process(
        Command::new(env!("CARGO_BIN_EXE_td-companion"))
            .args(["--mock-devices", "1"])
            .env("TOUCHDECK_APP_DATA_DIR", root.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = drain(child.0.stdout.take().unwrap());
    let stderr = drain(child.0.stderr.take().unwrap());
    let info = runtime(root.path()).await;
    let client = client();
    let token = session(&client, &info).await;
    let base = url(&info);
    let mut request = format!("{}/api/v1/events", base.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("touchdeck.v1, touchdeck.bearer.{token}")
            .parse()
            .unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let frames = tokio::spawn(async move {
        let mut frames = Vec::new();
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                frames.push(text.to_string());
            }
        }
        frames
    });
    let plug = client
        .post(format!("{base}/api/v1/test/ports"))
        .bearer_auth(&token)
        .json(&json!({"plug": {"id": "mock00a1", "kind": "device-radio"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(plug.status(), 204);
    connected(&client, &info, &token, 2).await;

    let wifi = format!("{base}/api/v1/devices/mock00a1/wifi");
    let mut bodies = Vec::new();
    for (method, body, expected) in [
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": "td-test-nonexistent", "password": PASSWORD})),
            200,
        ),
        (reqwest::Method::GET, None, 200),
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": "td-test-nonexistent", "password": PASSWORD, "extra": PASSWORD})),
            400,
        ),
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": "", "password": PASSWORD})),
            400,
        ),
        (reqwest::Method::DELETE, None, 200),
    ] {
        let mut request = client.request(method, &wifi).bearer_auth(&token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status().as_u16(), expected);
        bodies.push(response.text().await.unwrap());
    }
    assert!(bodies[1].contains("not-found"), "{}", bodies[1]);

    stop(&mut child, &client, &info, &token).await;
    let frames = tokio::time::timeout(Duration::from_secs(5), frames)
        .await
        .expect("shutdown closes the socket")
        .unwrap();
    let logs = [stdout.join().unwrap(), stderr.join().unwrap()];
    assert!(!logs[1].is_empty(), "the mock Devices log to stderr");
    for text in logs.iter().chain(&bodies).chain(&frames) {
        assert!(
            !text.contains(PASSWORD),
            "the Wi-Fi password leaked: {text}"
        );
    }
}
