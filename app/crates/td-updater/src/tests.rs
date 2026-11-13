use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use serde_json::{Value, json};
use std::{cell::Cell, collections::HashMap, io::Cursor};

const DATA: &[u8] = b"Disposable updater test payload, not an installer.";
const VERSION: &str = "2.1.0";

#[derive(Default)]
struct FakeHttp {
    responses: HashMap<String, Vec<u8>>,
    requests: Cell<usize>,
}

impl HttpClient for FakeHttp {
    fn get(&self, url: &str, user_agent: &str) -> Result<Box<dyn Read>, Failure> {
        self.requests.set(self.requests.get() + 1);
        assert!(user_agent.starts_with("OpenTouchDeck/"));
        self.responses
            .get(url)
            .cloned()
            .map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Read>)
            .ok_or_else(|| Failure::Http("offline fixture".into()))
    }
}

#[derive(Default)]
struct FakeClock(Cell<Duration>);

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

// Disposable keys are generated in memory for every test. No production secret
// is read, generated, changed or needed. This mirrors release-signer/minisign.mjs.
fn key() -> (SigningKey, &'static str) {
    let key = SigningKey::generate(&mut OsRng);
    let raw = [
        b"Ed".as_slice(),
        b"testonly",
        key.verifying_key().as_bytes(),
    ]
    .concat();
    let public = STANDARD.encode(format!(
        "untrusted comment: test only\n{}\n",
        STANDARD.encode(raw)
    ));
    (key, Box::leak(public.into_boxed_str()))
}

fn sign(key: &SigningKey, data: &[u8], filename: &str, version: &str) -> Vec<u8> {
    let signature = key.sign(&Blake2b512::digest(data)).to_bytes();
    let comment = format!("timestamp:123\tfile:{filename}\tversion:{version}");
    let global = key.sign(&[signature.as_slice(), comment.as_bytes()].concat());
    let packet = [b"ED".as_slice(), b"testonly", &signature].concat();
    STANDARD
        .encode(format!(
            "untrusted comment: test only\n{}\ntrusted comment: {comment}\n{}\n",
            STANDARD.encode(packet),
            STANDARD.encode(global.to_bytes())
        ))
        .into_bytes()
}

fn release(version: &str) -> Value {
    let name = format!("OpenTouchDeck-{version}-windows-x64.msi");
    json!({
        "tag_name":format!("v{version}"), "draft":false, "prerelease":false,
        "assets":[
            {"name":name,"browser_download_url":format!("{DOWNLOADS}/v{version}/{name}"),"size":DATA.len()},
            {"name":format!("{name}.sig"),"browser_download_url":format!("{DOWNLOADS}/v{version}/{name}.sig"),"size":1}
        ]
    })
}

fn feed(http: &mut FakeHttp, releases: Vec<Value>, page: usize) {
    http.responses.insert(
        format!("{RELEASES}?per_page={PAGE_SIZE}&page={page}"),
        serde_json::to_vec(&releases).unwrap(),
    );
}

fn fixture() -> Updater<FakeHttp, FakeClock> {
    let (key, public_key) = key();
    let name = format!("OpenTouchDeck-{VERSION}-windows-x64.msi");
    let signature = sign(&key, DATA, &name, VERSION);
    let mut release = release(VERSION);
    release["assets"][1]["size"] = json!(signature.len());
    let mut http = FakeHttp::default();
    feed(&mut http, vec![release], 1);
    http.responses
        .insert(format!("{DOWNLOADS}/v{VERSION}/{name}"), DATA.to_vec());
    http.responses
        .insert(format!("{DOWNLOADS}/v{VERSION}/{name}.sig"), signature);
    let mut updater = Updater::new(http, FakeClock::default(), Version::parse("2.0.0").unwrap());
    updater.public_key = public_key;
    updater
}

fn poll(updater: &mut Updater<FakeHttp, FakeClock>) -> UpdateResult {
    updater.poll(&AppSettings::default(), &|| false).unwrap()
}

fn edit_release(updater: &mut Updater<FakeHttp, FakeClock>, edit: impl FnOnce(&mut Value)) {
    let url = format!("{RELEASES}?per_page={PAGE_SIZE}&page=1");
    let mut releases: Value = serde_json::from_slice(&updater.http.responses[&url]).unwrap();
    edit(&mut releases[0]);
    updater
        .http
        .responses
        .insert(url, serde_json::to_vec(&releases).unwrap());
}

#[test]
fn verifies_a_disposable_signature_and_cleans_downloads_on_drop() {
    let mut updater = fixture();
    let UpdateResult::UpdateReady(ready) = poll(&mut updater) else {
        panic!("expected verified MSI")
    };
    assert_eq!(std::fs::read(ready.path()).unwrap(), DATA);
    assert_eq!(ready.version().to_string(), VERSION);
    assert!(ready.signature_path().is_file());
    let directory = ready.directory().to_owned();
    assert_eq!(updater.http.requests.get(), 3);
    drop(ready);
    assert!(!directory.exists());
}

#[test]
fn embeds_the_durable_committed_public_key() {
    let updater = Updater::new(
        FakeHttp::default(),
        FakeClock::default(),
        Version::new(2, 0, 0),
    );
    assert_eq!(updater.public_key, include_str!("../updater.pub"));
    let decoded = STANDARD.decode(PUBLIC_KEY.trim()).unwrap();
    assert_eq!(
        std::str::from_utf8(&decoded)
            .unwrap()
            .lines()
            .nth(1)
            .unwrap(),
        "RWRv4ycKG3q0N6FwOVclF2zA0kZSS+jhXPyW6JOnw2GzUuYFkYX1M1qH"
    );
    // build.rs also compares this snapshot byte for byte with the release
    // signer's committed key as soon as PR #364 is present in the checkout.
}

#[test]
fn accepts_the_actual_tauri_cli_fixture() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../.github/release-signer-test/fixtures/tauri-cli-2.12.4-test-only-key.json"
    ))
    .unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), fixture["data"].as_str().unwrap()).unwrap();
    signature::verify(
        file.path(),
        fixture["signature"].as_str().unwrap().as_bytes(),
        fixture["publicKey"].as_str().unwrap(),
        fixture["file"].as_str().unwrap(),
        &Version::parse(fixture["version"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
}

#[test]
fn rejects_another_key_even_with_the_same_key_id() {
    let mut updater = fixture();
    updater.public_key = key().1;
    assert!(matches!(
        poll(&mut updater),
        UpdateResult::Failed(Failure::Signature)
    ));
}

#[test]
fn rejects_tampered_and_truncated_files() {
    for truncate in [false, true] {
        let mut updater = fixture();
        let bytes = updater
            .http
            .responses
            .get_mut(&format!(
                "{DOWNLOADS}/v{VERSION}/OpenTouchDeck-{VERSION}-windows-x64.msi"
            ))
            .unwrap();
        if truncate {
            bytes.pop();
        } else {
            bytes[0] ^= 1;
        }
        assert!(matches!(
            poll(&mut updater),
            UpdateResult::Failed(Failure::Signature)
        ));
    }
}

#[test]
fn rejects_a_tampered_global_comment_signature() {
    let mut updater = fixture();
    let bytes = updater
        .http
        .responses
        .get_mut(&format!(
            "{DOWNLOADS}/v{VERSION}/OpenTouchDeck-{VERSION}-windows-x64.msi.sig"
        ))
        .unwrap();
    let decoded = String::from_utf8(STANDARD.decode(&*bytes).unwrap()).unwrap();
    *bytes = STANDARD
        .encode(decoded.replace("timestamp:123", "timestamp:124"))
        .into_bytes();
    assert!(matches!(
        poll(&mut updater),
        UpdateResult::Failed(Failure::Signature)
    ));
}

#[test]
fn rejects_authentic_signatures_for_a_different_version_or_filename() {
    for (filename, version) in [
        ("other.msi", VERSION),
        ("OpenTouchDeck-2.1.0-windows-x64.msi", "2.0.1"),
    ] {
        let mut updater = fixture();
        let (key, public) = key();
        updater.public_key = public;
        let bytes = sign(&key, DATA, filename, version);
        edit_release(&mut updater, |release| {
            release["assets"][1]["size"] = json!(bytes.len())
        });
        updater.http.responses.insert(
            format!("{DOWNLOADS}/v{VERSION}/OpenTouchDeck-{VERSION}-windows-x64.msi.sig"),
            bytes,
        );
        assert!(matches!(
            poll(&mut updater),
            UpdateResult::Failed(Failure::Signature)
        ));
    }
}

#[test]
fn refuses_lower_versions_without_downloading() {
    let mut updater = fixture();
    updater.current = Version::new(2, 2, 0);
    assert!(matches!(
        poll(&mut updater),
        UpdateResult::Failed(Failure::LowerVersion { .. })
    ));
    assert_eq!(updater.http.requests.get(), 1);
}

#[test]
fn equal_versions_ignore_build_metadata() {
    let mut updater = fixture();
    updater.current = Version::parse("2.1.0+local").unwrap();
    assert!(matches!(poll(&mut updater), UpdateResult::UpToDate));
    assert_eq!(updater.http.requests.get(), 1);
}

#[test]
fn a_pre_release_copy_is_offered_the_final_but_not_another_pre_release() {
    // A 2.0.0-preview.1 install finds the final once it is published as a stable release.
    let mut updater = fixture();
    updater.current = Version::parse(&format!("{VERSION}-preview.1")).unwrap();
    let UpdateResult::UpdateReady(ready) = poll(&mut updater) else {
        panic!("expected the final release")
    };
    assert_eq!(ready.version().to_string(), VERSION);

    // A later preview published as a GitHub pre-release is never offered.
    let mut updater = fixture();
    updater.current = Version::parse(&format!("{VERSION}-preview.1")).unwrap();
    edit_release(&mut updater, |release| release["prerelease"] = json!(true));
    assert!(matches!(poll(&mut updater), UpdateResult::UpToDate));
    assert_eq!(updater.http.requests.get(), 1);
}

#[test]
fn selects_the_highest_stable_2x_across_pages() {
    let mut updater = fixture();
    let newest: Vec<Value> = serde_json::from_slice(
        updater
            .http
            .responses
            .get(&format!("{RELEASES}?per_page={PAGE_SIZE}&page=1"))
            .unwrap(),
    )
    .unwrap();
    let mut first = vec![release("1.99.0"); PAGE_SIZE];
    first[0] = release("3.0.0");
    first[1] = release("2.9.0-beta.1");
    first[2] = release("2.9.0");
    first[2]["prerelease"] = json!(true);
    first[3] = release("2.8.0");
    first[3]["draft"] = json!(true);
    first[4] = release("2.0.9");
    feed(&mut updater.http, first, 1);
    feed(&mut updater.http, newest, 2);
    assert!(matches!(poll(&mut updater), UpdateResult::UpdateReady(_)));
    assert_eq!(updater.http.requests.get(), 4);
}

#[test]
fn missing_or_invalid_signatures_and_unsafe_urls_fail_closed() {
    for kind in 0..4 {
        let mut updater = fixture();
        edit_release(&mut updater, |release| match kind {
            0 => {
                release["assets"].as_array_mut().unwrap().pop();
            }
            1 => {
                release["assets"][0]["browser_download_url"] =
                    json!("http://example.invalid/update.msi")
            }
            2 => release["assets"][0]["size"] = json!(MAX_MSI + 1),
            _ => {
                release["assets"][0]["browser_download_url"] =
                    json!("https://example.invalid/update.msi")
            }
        });
        assert!(matches!(poll(&mut updater), UpdateResult::Failed(_)));
        assert_eq!(updater.http.requests.get(), 1);
    }
    let mut updater = fixture();
    let bytes = updater
        .http
        .responses
        .get_mut(&format!(
            "{DOWNLOADS}/v{VERSION}/OpenTouchDeck-{VERSION}-windows-x64.msi.sig"
        ))
        .unwrap();
    bytes.fill(b'!');
    assert!(matches!(
        poll(&mut updater),
        UpdateResult::Failed(Failure::Signature)
    ));
}

#[test]
fn checks_off_makes_no_requests_even_when_due() {
    let mut updater = fixture();
    let off = AppSettings {
        automatic_update_checks: false,
        ..Default::default()
    };
    assert!(updater.poll(&off, &|| false).is_none());
    updater.clock.0.set(CHECK_INTERVAL * 2);
    assert!(updater.poll(&off, &|| false).is_none());
    assert_eq!(updater.http.requests.get(), 0);
    assert!(matches!(poll(&mut updater), UpdateResult::UpdateReady(_)));
    updater.clock.0.set(CHECK_INTERVAL * 4);
    assert!(updater.poll(&off, &|| false).is_none());
    assert_eq!(updater.http.requests.get(), 3);
}

#[test]
fn waits_24_hours_between_attempts_including_failures() {
    let mut updater = Updater::new(
        FakeHttp::default(),
        FakeClock::default(),
        Version::new(2, 0, 0),
    );
    assert!(matches!(
        poll(&mut updater),
        UpdateResult::Failed(Failure::Http(_))
    ));
    updater.clock.0.set(CHECK_INTERVAL - Duration::from_secs(1));
    assert!(updater.poll(&AppSettings::default(), &|| false).is_none());
    updater.clock.0.set(CHECK_INTERVAL);
    assert!(matches!(
        poll(&mut updater),
        UpdateResult::Failed(Failure::Http(_))
    ));
    assert_eq!(updater.http.requests.get(), 2);
}

#[test]
fn app_preference_defaults_on_and_persists_an_opt_out() {
    let directory = tempfile::tempdir().unwrap();
    let dirs = td_core::app_dirs::Dirs {
        config: directory.path().to_owned(),
    };
    assert!(AppSettings::load(&dirs).unwrap().automatic_update_checks);
    std::fs::write(dirs.app_settings(), b"{}").unwrap();
    assert!(AppSettings::load(&dirs).unwrap().automatic_update_checks);
    let off = AppSettings {
        automatic_update_checks: false,
        ..Default::default()
    };
    off.save(&dirs).unwrap();
    let loaded = AppSettings::load(&dirs).unwrap();
    assert_eq!(loaded, off);
    let mut updater = fixture();
    assert!(updater.poll(&loaded, &|| false).is_none());
    assert_eq!(updater.http.requests.get(), 0);
    std::fs::write(dirs.app_settings(), b"broken").unwrap();
    assert!(AppSettings::load(&dirs).is_err());
}

#[derive(Clone, Copy, Debug)]
enum BlockAt {
    Request(usize),
    MsiBody,
}

struct BlockingHttp {
    inner: FakeHttp,
    block_at: BlockAt,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
}

impl HttpClient for BlockingHttp {
    fn get(&self, url: &str, user_agent: &str) -> Result<Box<dyn Read>, Failure> {
        let response = self.inner.get(url, user_agent)?;
        if matches!(self.block_at, BlockAt::Request(n) if n == self.inner.requests.get()) {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
        }
        if matches!(self.block_at, BlockAt::MsiBody) && url.ends_with(".msi") {
            Ok(Box::new(BlockingBody {
                inner: response,
                reads: 0,
                entered: self.entered.clone(),
                release: self.release.clone(),
            }))
        } else {
            Ok(response)
        }
    }
}

struct BlockingBody {
    inner: Box<dyn Read>,
    reads: usize,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
}

impl Read for BlockingBody {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.reads == 2 {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
        }
        // Stage one byte first so cancellation also exercises partial-file cleanup.
        let limit = bytes.len().min(1);
        self.inner.read(&mut bytes[..limit])
    }
}

#[test]
fn shutdown_during_requests_or_download_stops_and_removes_staging() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    for block_at in [
        BlockAt::Request(1),
        BlockAt::Request(2),
        BlockAt::Request(3),
        BlockAt::MsiBody,
    ] {
        let staging = tempfile::tempdir().unwrap();
        let fixture = fixture();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = stopped.clone();
        let mut updater = Updater::new(
            BlockingHttp {
                inner: fixture.http,
                block_at,
                entered: entered_tx,
                release: Arc::new(Mutex::new(release_rx)),
            },
            fixture.clock,
            fixture.current,
        );
        updater.public_key = fixture.public_key;
        updater.staging_root = staging.path().to_owned();
        let worker = std::thread::spawn(move || {
            let result = updater.poll(&AppSettings::default(), &|| {
                worker_stop.load(Ordering::SeqCst)
            });
            (result, updater.http.inner.requests.get())
        });
        // The response cannot return until shutdown has been signalled: no timing race.
        entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let staged: Vec<_> = std::fs::read_dir(staging.path()).unwrap().collect();
        if matches!(block_at, BlockAt::MsiBody) {
            assert_eq!(staged.len(), 1);
            let msi = staged[0]
                .as_ref()
                .unwrap()
                .path()
                .join(format!("OpenTouchDeck-{VERSION}-windows-x64.msi"));
            assert_eq!(std::fs::metadata(msi).unwrap().len(), 1);
        } else {
            assert!(staged.is_empty(), "staged while blocked at {block_at:?}");
        }
        stopped.store(true, Ordering::SeqCst);
        release_tx.send(()).unwrap();
        let (result, requests) = worker.join().unwrap();
        assert!(result.is_none(), "published a result after {block_at:?}");
        let expected = match block_at {
            BlockAt::Request(n) => n,
            BlockAt::MsiBody => 3,
        };
        assert_eq!(requests, expected, "requested again after {block_at:?}");
        assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
    }
}

#[test]
fn shutdown_before_poll_makes_no_request_or_staging_directory() {
    let staging = tempfile::tempdir().unwrap();
    let mut updater = fixture();
    updater.staging_root = staging.path().to_owned();
    assert!(updater.poll(&AppSettings::default(), &|| true).is_none());
    assert_eq!(updater.http.requests.get(), 0);
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
}

#[test]
fn startup_sweep_removes_staging_left_by_a_read_blocked_through_shutdown() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    let staging = tempfile::tempdir().unwrap();
    let root = staging.path();
    // Folders and files the sweep must leave alone.
    let foreign = root.join(format!("{STAGING_PREFIX}keep"));
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("notes.txt"), b"not ours").unwrap();
    let nested = root.join(format!("{STAGING_PREFIX}nest"));
    std::fs::create_dir_all(nested.join("OpenTouchDeck-2.1.0-windows-x64.msi")).unwrap();
    let unsuffixed = root.join(STAGING_PREFIX);
    std::fs::create_dir(&unsuffixed).unwrap();
    let other_app = root.join("other-app-update-abc123");
    std::fs::create_dir(&other_app).unwrap();
    std::fs::write(other_app.join("OpenTouchDeck-2.1.0-windows-x64.msi"), b"x").unwrap();
    let plain_file = root.join(format!("{STAGING_PREFIX}file"));
    std::fs::write(&plain_file, b"a file, not a folder").unwrap();
    #[cfg(unix)]
    let link = {
        let target = tempfile::tempdir().unwrap();
        let link = root.join(format!("{STAGING_PREFIX}link"));
        std::os::unix::fs::symlink(target.path(), &link).unwrap();
        (target, link)
    };
    let unrelated: Vec<_> = {
        let mut names: Vec<_> = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };

    // First run: the MSI body read stalls after one byte is staged and stays blocked
    // through shutdown, so the process would exit before TempDir could clean up.
    let first = fixture();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stop = stopped.clone();
    let mut updater = Updater::new(
        BlockingHttp {
            inner: first.http,
            block_at: BlockAt::MsiBody,
            entered: entered_tx,
            release: Arc::new(Mutex::new(release_rx)),
        },
        first.clock,
        first.current,
    );
    updater.public_key = first.public_key;
    updater.staging_root = root.to_owned();
    let worker = std::thread::spawn(move || {
        updater.poll(&AppSettings::default(), &|| {
            worker_stop.load(Ordering::SeqCst)
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    stopped.store(true, Ordering::SeqCst);
    // Shutdown gives up here; the read is still blocked and the staging folder remains.
    let leftover: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| is_stale_staging(path))
        .collect();
    assert_eq!(leftover.len(), 1);
    let msi = leftover[0].join(format!("OpenTouchDeck-{VERSION}-windows-x64.msi"));
    assert_eq!(std::fs::metadata(msi).unwrap().len(), 1);
    assert!(
        leftover[0]
            .join(format!("OpenTouchDeck-{VERSION}-windows-x64.msi.sig"))
            .exists()
    );

    // Next run: the startup sweep removes only the leftover.
    let mut next = fixture();
    next.staging_root = root.to_owned();
    assert_eq!(next.remove_stale_staging(), 1);
    assert!(!leftover[0].exists());
    let mut remaining: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    remaining.sort();
    assert_eq!(remaining, unrelated);
    assert_eq!(
        std::fs::read(foreign.join("notes.txt")).unwrap(),
        b"not ours"
    );
    assert!(nested.join("OpenTouchDeck-2.1.0-windows-x64.msi").is_dir());
    #[cfg(unix)]
    assert!(link.0.path().is_dir());
    assert_eq!(next.remove_stale_staging(), 0);
    assert_eq!(next.http.requests.get(), 0);

    // Let the old worker finish so the test owns no thread; it publishes nothing.
    release_tx.send(()).unwrap();
    assert!(worker.join().unwrap().is_none());
    let mut remaining: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    remaining.sort();
    assert_eq!(remaining, unrelated);
}
