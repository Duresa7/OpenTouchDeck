//! Brandfetch logos with the user's own keys. The UI searches the Brand Search API itself with
//! the client ID; the Companion keeps the client ID and the API key, turns a pick into an Asset
//! through the Brand API, and refreshes each logo before Brandfetch's 30-day cache limit.
//! See docs/companion-api.md, "Brandfetch".
mod client;
mod refresh;
mod session;
pub use session::{BrandfetchSession, BrandfetchView, KeyTestResult, Operation, Problem};

use crate::{
    api::{ApiError, CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    features::assets::{self, AssetError, AssetRef, Fit, RenderJob, RenderProgress},
    jobs::{JobHandle, Stop},
};
use axum::http::{Method, StatusCode};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use td_host::{
    icon_pack,
    integrations::secrets::{MemorySecrets, OsAppSecrets, Secret, SecretStore},
};
use ts_rs::TS;

pub use client::{API_BASE, ApiKey, FILE_ORIGINS};
pub(crate) use refresh::run as refresh_loop;
pub use refresh::{CALLS_PER_RUN, EXPIRE_AFTER_S, REFRESH_AFTER_S, REFRESH_TICK, RefreshReport};

/// Which kind of logo a pick asks for, in Brandfetch's words.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename = "BrandfetchLogoType")]
pub enum LogoType {
    #[default]
    Icon,
    Symbol,
    Logo,
}
impl LogoType {
    fn name(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::Symbol => "symbol",
            Self::Logo => "logo",
        }
    }
}
/// `light` is a light logo for dark backgrounds, `dark` the reverse. `auto` prefers `light`,
/// since Tiles are dark.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename = "BrandfetchTheme")]
pub enum LogoTheme {
    Light,
    Dark,
    #[default]
    Auto,
}
/// A Brandfetch logo to render as an Asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct BrandfetchLogo {
    /// A domain such as `example.com`, or a Brandfetch brand id.
    pub brand: String,
    #[serde(default)]
    pub logo_type: LogoType,
    #[serde(default)]
    pub theme: LogoTheme,
    #[serde(default)]
    pub padding: u32,
    pub background: Option<[u8; 4]>,
    pub tint: Option<[u8; 3]>,
}

#[derive(Debug, thiserror::Error)]
pub enum BrandfetchError {
    #[error("Add your Brandfetch API key in Settings, then retry.")]
    NoKey,
    #[error("Brandfetch refused the API key. Check it in Settings, then retry.")]
    KeyRefused,
    #[error(
        "Your Brandfetch API requests are used up. Check your plan in the Brandfetch developer portal."
    )]
    Quota,
    #[error("Brandfetch doesn't know that brand.")]
    NotFound,
    #[error("Brandfetch has no logo of that kind for this brand. Choose another result.")]
    NoLogo,
    #[error("Brandfetch didn't answer. Check the internet connection, then retry. ({0})")]
    Unavailable(String),
    #[error("Couldn't save the Brandfetch settings: {0}")]
    Store(String),
    /// The system keychain can't be reached. The key never falls back to a file. The UI says
    /// which keychain this platform uses and how to start it.
    #[error("The system keychain isn't available.")]
    NoKeychain,
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl ApiError for BrandfetchError {
    const CODES: &'static [&'static str] = &[
        "brandfetch.no_key",
        "brandfetch.key_refused",
        "brandfetch.quota",
        "brandfetch.not_found",
        "brandfetch.no_logo",
        "brandfetch.unavailable",
        "brandfetch.store",
        "brandfetch.no_keychain",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::NoKey => StatusCode::CONFLICT,
            Self::KeyRefused | Self::Unavailable(_) => StatusCode::BAD_GATEWAY,
            Self::Quota => StatusCode::TOO_MANY_REQUESTS,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::NoLogo => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NoKeychain => StatusCode::SERVICE_UNAVAILABLE,
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::NoKey => "brandfetch.no_key",
            Self::KeyRefused => "brandfetch.key_refused",
            Self::Quota => "brandfetch.quota",
            Self::NotFound => "brandfetch.not_found",
            Self::NoLogo => "brandfetch.no_logo",
            Self::Unavailable(_) => "brandfetch.unavailable",
            Self::Store(_) => "brandfetch.store",
            Self::NoKeychain => "brandfetch.no_keychain",
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Common(e) => e.details(),
            _ => None,
        }
    }
}

/// Where the Companion sends Brand API calls and which origins logo files may come from.
#[derive(Clone)]
struct Net {
    http: Arc<dyn client::Http>,
    base: String,
    files: Vec<String>,
}
struct Inner {
    dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
    net: RwLock<Net>,
    client_id: RwLock<Option<String>>,
    /// Mock mode: the key lives in memory and Brand API calls get recorded answers. Only the
    /// test-hooks environment prefill reads it.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    mock: bool,
    /// Serializes every read-modify-write of the provenance file.
    book: Mutex<()>,
    refreshing: Mutex<()>,
    operations: Arc<tokio::sync::Mutex<()>>,
}
/// The Brandfetch settings, the API key and each Brandfetch Asset's provenance, kept under
/// `<data>/brandfetch/`. The key lives in the OS credential store.
#[derive(Clone)]
pub struct Brandfetch(Arc<Inner>);

const OWNER: &str = "brandfetch";
const KEY_FIELD: &str = "api_key";

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    client_id: Option<String>,
}

impl Brandfetch {
    /// Mock mode keeps the key in memory and answers Brand API calls from a recorded fixture;
    /// otherwise the key lives in the OS credential store.
    pub(crate) fn new(dir: PathBuf, mock: bool) -> Self {
        if mock {
            Self::build(
                dir,
                true,
                Arc::new(MemorySecrets::default()),
                Arc::new(client::FixtureHttp),
            )
        } else {
            Self::build(
                dir,
                false,
                Arc::new(OsAppSecrets),
                Arc::new(client::UreqHttp::https()),
            )
        }
    }
    fn build(
        dir: PathBuf,
        mock: bool,
        secrets: Arc<dyn SecretStore>,
        http: Arc<dyn client::Http>,
    ) -> Self {
        let client_id = std::fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .and_then(|s| s.client_id)
            .filter(|id| valid_client_id(id));
        Self(Arc::new(Inner {
            dir,
            secrets,
            net: RwLock::new(Net {
                http,
                base: API_BASE.into(),
                files: FILE_ORIGINS.iter().map(|o| (*o).to_owned()).collect(),
            }),
            client_id: RwLock::new(client_id),
            mock,
            book: Mutex::new(()),
            refreshing: Mutex::new(()),
            operations: Arc::new(tokio::sync::Mutex::new(())),
        }))
    }
    /// Whether the user turned Brandfetch search on by saving a client ID. The Companion's CSP
    /// lets the UI reach Brandfetch only then.
    pub fn search_enabled(&self) -> bool {
        self.0.client_id.read().is_some()
    }
    /// Sends Brand API calls and logo downloads to `base` over real HTTP, for tests against a
    /// local server.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn use_api(&self, base: &str) {
        let origin = url::Url::parse(base)
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_default();
        *self.0.net.write() = Net {
            http: Arc::new(client::UreqHttp::local()),
            base: base.trim_end_matches('/').into(),
            files: vec![origin],
        };
    }
    /// Takes the client ID and API key from `TOUCHDECK_BRANDFETCH_CLIENT_ID` and
    /// `TOUCHDECK_BRANDFETCH_API_KEY`, so a mock-mode run can search and pick live without
    /// anyone typing the key. Mock mode only: the key goes to memory, never the keyring or a
    /// file, and with a key Brand API calls go to Brandfetch for real.
    #[cfg(feature = "test-hooks")]
    pub(crate) fn prefill_from_env(&self) {
        let read = |name: &str| std::env::var(name).ok();
        self.prefill(
            read("TOUCHDECK_BRANDFETCH_CLIENT_ID"),
            read("TOUCHDECK_BRANDFETCH_API_KEY"),
        );
    }
    #[cfg(any(test, feature = "test-hooks"))]
    fn prefill(&self, client_id: Option<String>, key: Option<String>) {
        if !self.0.mock {
            return;
        }
        let clean = |v: Option<String>| v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        if let Some(id) = clean(client_id).filter(|id| valid_client_id(id)) {
            *self.0.client_id.write() = Some(id);
        }
        if let Some(value) = clean(key).filter(|k| valid_key(k))
            && self
                .0
                .secrets
                .put(
                    OWNER,
                    KEY_FIELD,
                    Secret {
                        origin: API_BASE.into(),
                        value,
                    },
                )
                .is_ok()
        {
            self.use_live();
        }
    }
    /// Sends Brand API calls to Brandfetch for real, as outside mock mode.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn use_live(&self) {
        *self.0.net.write() = Net {
            http: Arc::new(client::UreqHttp::https()),
            base: API_BASE.into(),
            files: FILE_ORIGINS.iter().map(|o| (*o).to_owned()).collect(),
        };
    }
    /// The API key, if one is stored for the Brand API's origin.
    fn key(&self) -> Result<Option<ApiKey>, BrandfetchError> {
        Ok(self
            .0
            .secrets
            .get(OWNER, KEY_FIELD)
            .map_err(|_| BrandfetchError::NoKeychain)?
            .filter(|s| s.origin == API_BASE)
            .map(|s| ApiKey::new(s.value)))
    }
    /// Keeps the key in the keychain, bound to the Brand API's origin. Saving never calls
    /// Brandfetch: each call costs a request. Logos the old key failed for refresh again.
    /// Blocking: callers on a UI thread should use [`BrandfetchSession::save_key`].
    pub fn save_key(&self, key: ApiKey) -> Result<(), BrandfetchError> {
        let value = key.expose().trim().to_owned();
        if !valid_key(&value) {
            return Err(CommonError::invalid(
                "value",
                "Paste the API key from the Brandfetch developer portal.",
            )
            .into());
        }
        let secret = Secret {
            origin: API_BASE.into(),
            value,
        };
        self.0
            .secrets
            .put(OWNER, KEY_FIELD, secret)
            .map_err(|_| BrandfetchError::NoKeychain)?;
        if self.resume_after_new_key().is_err() {
            eprintln!("Companion: couldn't resume Brandfetch refreshes after a new key.");
        }
        Ok(())
    }
    /// Blocking keychain operation. The native window uses the session.
    pub fn forget_key(&self) -> Result<(), BrandfetchError> {
        self.0
            .secrets
            .delete(OWNER, KEY_FIELD)
            .map_err(|_| BrandfetchError::NoKeychain)
    }
    /// Blocking network call for Brandfetch's own brand. Use the session from a UI thread.
    pub fn test_key(&self) -> Result<(), BrandfetchError> {
        let key = self.key()?.ok_or(BrandfetchError::NoKey)?;
        let net = self.0.net.read().clone();
        client::check(net.http.as_ref(), &net.base, &key)
    }
    /// Saves a validated client ID without making a network call. Blocking disk I/O.
    pub fn save_client_id(&self, client_id: String) -> Result<(), BrandfetchError> {
        let id = client_id.trim().to_owned();
        if !valid_client_id(&id) {
            return Err(CommonError::invalid("client_id", "Use the client ID from the Brandfetch developer portal: letters, digits, '-', '_' or '.', up to 128.").into());
        }
        self.save_settings(Some(id))
    }
    /// Forgets only the client ID. Blocking disk I/O.
    pub fn forget_client_id(&self) -> Result<(), BrandfetchError> {
        self.save_settings(None)
    }
    fn save_settings(&self, client_id: Option<String>) -> Result<(), BrandfetchError> {
        let mut current = self.0.client_id.write();
        let path = self.0.dir.join("settings.json");
        let damaged = || BrandfetchError::Store("Couldn't read Brandfetch settings.".into());
        let mut fields = match std::fs::read(&path) {
            Ok(bytes) => {
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|_| damaged())?;
                serde_json::from_value::<Settings>(value.clone()).map_err(|_| damaged())?;
                match value {
                    serde_json::Value::Object(fields) => fields,
                    _ => return Err(damaged()),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(_) => return Err(damaged()),
        };
        fields.insert("client_id".into(), serde_json::json!(client_id));
        let bytes = serde_json::to_vec(&fields).map_err(|_| damaged())?;
        td_core::app_dirs::write_atomic(&path, &bytes)
            .map_err(|_| BrandfetchError::Store("Couldn't write Brandfetch settings.".into()))?;
        *current = client_id;
        Ok(())
    }
    /// What Settings shows at `now`, in Unix seconds by the Companion's clock.
    pub fn status(&self, now: u64) -> Result<BrandfetchStatus, BrandfetchError> {
        let summary = self.summary(now);
        Ok(BrandfetchStatus {
            client_id: self.0.client_id.read().clone(),
            // A keychain that can't be reached reads as no key; saving one says why.
            key_set: matches!(self.key(), Ok(Some(_))),
            logos: summary.logos,
            need_refresh: summary.need_refresh,
            expired: summary.expired,
            replaced: summary.replaced_by,
        })
    }
    /// Calls the Brand API with the stored key and downloads the chosen logo file, unless
    /// `cancelled` says to stop first.
    fn fetch(
        &self,
        logo: &BrandfetchLogo,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<client::Original, BrandfetchError> {
        let key = self.key()?.ok_or(BrandfetchError::NoKey)?;
        let net = self.0.net.read().clone();
        client::fetch(
            net.http.as_ref(),
            &net.base,
            &net.files,
            &key,
            logo,
            cancelled,
        )
    }
}

fn valid_client_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn valid_key(key: &str) -> bool {
    (1..=512).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic())
}

/// Renders a Brandfetch logo as a Tile icon and records where it came from.
pub(crate) fn render(
    ctx: &Ctx,
    logo: BrandfetchLogo,
    fit: Fit,
    job: &JobHandle<RenderJob>,
) -> Result<AssetRef, Stop> {
    let original = ctx.brandfetch.fetch(&logo, &|| job.check().is_err());
    job.check()?;
    let original = original?;
    let image = client::draw(&original, &logo, fit).map_err(AssetError::RenderFailed)?;
    let file = icon_pack::pack_any(&image);
    job.check()?;
    let id = ctx.assets.put(&file).map_err(assets::stored)?;
    ctx.brandfetch
        .record(&id, &logo, fit, &original, ctx.clock.unix_s())
        .map_err(|e| BrandfetchError::Store(e.to_string()))?;
    job.progress(&RenderProgress {
        frames_done: 1,
        frames_total: 1,
    });
    Ok(AssetRef {
        id,
        kind: "tdi1".into(),
        width: icon_pack::SIDE,
        height: icon_pack::SIDE,
        bytes: file.len() as u64,
    })
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CommonError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CommonError::Internal)
}

/// What the Settings section shows. The API key itself is never returned.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BrandfetchStatus {
    /// The client ID the UI searches with; not a secret.
    pub client_id: Option<String>,
    pub key_set: bool,
    /// Brandfetch Assets this PC keeps provenance for.
    pub logos: u32,
    /// Of those, the ones at least 25 days old that the daily refresh hasn't renewed yet.
    pub need_refresh: u32,
    /// Assets whose cached original was deleted after 30 days without a successful refresh.
    pub expired: Vec<String>,
    /// Each Asset a refresh replaced with a changed logo, and the Asset that replaced it. The UI
    /// swaps them in its Layout drafts.
    pub replaced: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct ClientIdBody {
    pub client_id: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct ApiKeyBody {
    pub value: String,
}

pub struct Get;
impl Endpoint for Get {
    const NAME: &'static str = "brandfetch.get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/brandfetch";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = BrandfetchStatus;
    type Error = BrandfetchError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        blocking(move || ctx.brandfetch.status(ctx.clock.unix_s())).await?
    }
}
/// Saves the client ID, which turns search on.
pub struct ClientIdPut;
impl Endpoint for ClientIdPut {
    const NAME: &'static str = "brandfetch.client_id_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/brandfetch/client-id";
    type Path = ();
    type Query = NoQuery;
    type Body = ClientIdBody;
    type Ok = BrandfetchStatus;
    type Error = BrandfetchError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        blocking(move || {
            ctx.brandfetch.save_client_id(req.body.client_id)?;
            ctx.brandfetch.status(ctx.clock.unix_s())
        })
        .await?
    }
}
/// Forgets the client ID, which turns search off.
pub struct ClientIdDelete;
impl Endpoint for ClientIdDelete {
    const NAME: &'static str = "brandfetch.client_id_delete";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/brandfetch/client-id";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = BrandfetchStatus;
    type Error = BrandfetchError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        blocking(move || {
            ctx.brandfetch.forget_client_id()?;
            ctx.brandfetch.status(ctx.clock.unix_s())
        })
        .await?
    }
}
/// Stores the API key in the OS credential store. It's write-only.
pub struct KeyPut;
impl Endpoint for KeyPut {
    const NAME: &'static str = "brandfetch.key_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/brandfetch/key";
    type Path = ();
    type Query = NoQuery;
    type Body = ApiKeyBody;
    type Ok = BrandfetchStatus;
    type Error = BrandfetchError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        blocking(move || {
            ctx.brandfetch.save_key(ApiKey::new(req.body.value))?;
            ctx.brandfetch.status(ctx.clock.unix_s())
        })
        .await?
    }
}
pub struct KeyDelete;
impl Endpoint for KeyDelete {
    const NAME: &'static str = "brandfetch.key_delete";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/brandfetch/key";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = BrandfetchStatus;
    type Error = BrandfetchError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        blocking(move || {
            ctx.brandfetch.forget_key()?;
            ctx.brandfetch.status(ctx.clock.unix_s())
        })
        .await?
    }
}
/// Checks the stored key with one Brand API call for Brandfetch's own brand, which Brandfetch
/// doesn't count. Only on the user's request: saving the key never calls Brandfetch.
pub struct KeyTest;
impl Endpoint for KeyTest {
    const NAME: &'static str = "brandfetch.key_test";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/brandfetch/key/test";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = BrandfetchStatus;
    type Error = BrandfetchError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        blocking(move || {
            ctx.brandfetch.test_key()?;
            ctx.brandfetch.status(ctx.clock.unix_s())
        })
        .await?
    }
}

fn feature() -> Feature {
    Feature::new("brandfetch")
        .endpoint::<Get>()
        .endpoint::<ClientIdPut>()
        .endpoint::<ClientIdDelete>()
        .endpoint::<KeyPut>()
        .endpoint::<KeyDelete>()
        .endpoint::<KeyTest>()
}
inventory::submit! { Registration(feature) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_client_id_is_read_back_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let first = Brandfetch::new(dir.path().to_owned(), true);
        assert!(!first.search_enabled());
        first.save_settings(Some("test-client-id".into())).unwrap();
        let again = Brandfetch::new(dir.path().to_owned(), true);
        assert!(again.search_enabled());
        assert_eq!(
            again.status(0).unwrap().client_id.as_deref(),
            Some("test-client-id")
        );
        again.save_settings(None).unwrap();
        assert!(!Brandfetch::new(dir.path().to_owned(), true).search_enabled());
    }

    /// A credential store that can't be reached, as on Linux with no Secret Service running.
    struct NoKeychain;
    impl SecretStore for NoKeychain {
        fn get(&self, _: &str, _: &str) -> Result<Option<Secret>, String> {
            Err("Credential store unavailable".into())
        }
        fn put(&self, _: &str, _: &str, _: Secret) -> Result<(), String> {
            Err("Credential store unavailable".into())
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), String> {
            Err("Credential store unavailable".into())
        }
    }
    fn every_file(dir: &std::path::Path, out: &mut Vec<Vec<u8>>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                every_file(&path, out);
            } else {
                out.push(std::fs::read(path).unwrap());
            }
        }
    }

    #[test]
    fn without_a_keychain_saving_the_key_fails_clearly_and_writes_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let brandfetch = Brandfetch::build(
            dir.path().to_owned(),
            false,
            Arc::new(NoKeychain),
            Arc::new(client::FixtureHttp),
        );
        brandfetch
            .save_settings(Some("test-client-id".into()))
            .unwrap();
        let error = brandfetch.save_key("test-api-key".into()).unwrap_err();
        assert_eq!(error.code(), "brandfetch.no_keychain");
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        // The UI words it for the platform; the Companion's text names none.
        assert_eq!(error.to_string(), "The system keychain isn't available.");
        assert!(matches!(
            brandfetch.forget_key(),
            Err(BrandfetchError::NoKeychain)
        ));
        // Settings still load, and a pick or a key test says why it can't go on.
        assert!(!brandfetch.status(0).unwrap().key_set);
        let logo = BrandfetchLogo {
            brand: "example.com".into(),
            logo_type: LogoType::Icon,
            theme: LogoTheme::Auto,
            padding: 0,
            background: None,
            tint: None,
        };
        assert!(matches!(
            brandfetch.fetch(&logo, &|| false),
            Err(BrandfetchError::NoKeychain)
        ));
        assert!(matches!(
            brandfetch.test_key(),
            Err(BrandfetchError::NoKeychain)
        ));
        // Nothing fell back to a file.
        let mut files = vec![];
        every_file(dir.path(), &mut files);
        assert!(!files.is_empty(), "the client ID was saved");
        assert!(
            files
                .iter()
                .all(|f| !String::from_utf8_lossy(f).contains("test-api-key"))
        );
    }

    #[test]
    fn the_env_prefill_works_only_in_mock_mode_and_only_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let real = Brandfetch::build(
            dir.path().to_owned(),
            false,
            Arc::new(NoKeychain),
            Arc::new(client::FixtureHttp),
        );
        real.prefill(Some("test-client-id".into()), Some("test-api-key".into()));
        assert!(!real.search_enabled(), "never outside mock mode");

        let mock = Brandfetch::new(dir.path().to_owned(), true);
        mock.prefill(Some(" ".into()), None);
        assert!(!mock.search_enabled());
        mock.prefill(Some(" test-client-id ".into()), Some("test-api-key".into()));
        let status = mock.status(0).unwrap();
        assert_eq!(status.client_id.as_deref(), Some("test-client-id"));
        assert!(status.key_set);
        let mut files = vec![];
        every_file(dir.path(), &mut files);
        assert!(files.is_empty(), "the prefill writes nothing to disk");
    }
}
