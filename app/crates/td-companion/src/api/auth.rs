//! Tokens, sessions and the routes that mint and revoke them. Private to `api`.
use super::{Access, CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req};
use axum::http::{HeaderMap, Method, StatusCode};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum SessionKind {
    Window,
    Browser,
}
/// Who is calling: the session id and kind, for audit and for routes limited to a kind.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ClientInfo {
    pub id: String,
    pub kind: SessionKind,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SessionInfo {
    pub id: String,
    pub kind: SessionKind,
    pub label: String,
    pub created_ms: u64,
    pub last_used_ms: u64,
    pub current: bool,
}

/// The Companion's clock. Tests move it forward.
#[derive(Clone, Default)]
pub(crate) struct Clock(Arc<AtomicU64>);
impl Clock {
    pub(crate) fn now(&self) -> Instant {
        Instant::now() + Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
    /// Wall-clock seconds since the Unix epoch, moved forward as far as tests moved [`Self::now`].
    /// For deadlines that must hold across sleep, such as Brandfetch's 30 days.
    pub(crate) fn unix_s(&self) -> u64 {
        crate::photos::clock::unix_s() + self.0.load(Ordering::SeqCst) / 1000
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}

/// A window session with no open WebSocket for this long ends.
const WINDOW_IDLE: Duration = Duration::from_secs(60);
/// Revoked sessions are kept so their tokens get `auth.revoked`, until the table is full.
const MAX_SESSIONS: usize = 1024;
const MAX_LABEL: usize = 64;
const BROWSER_IDLE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

struct Session {
    info: SessionInfo,
    revoked: bool,
    sockets: usize,
    /// When the session last had no open socket.
    idle_since: Instant,
    last_request: Instant,
    marked_expired: bool,
}
impl Session {
    fn expired(&self, now: Instant) -> bool {
        self.marked_expired
            || (self.info.kind == SessionKind::Browser
                && now.saturating_duration_since(self.last_request) >= BROWSER_IDLE)
            || self.info.kind == SessionKind::Window
                && self.sockets == 0
                && now.saturating_duration_since(self.idle_since) >= WINDOW_IDLE
    }
    fn live(&self, now: Instant) -> bool {
        !self.revoked && !self.expired(now)
    }
}
pub(crate) struct Auth {
    pairing: String,
    pending: Option<([u8; 32], Instant)>,
    wrong: u8,
    sessions: HashMap<[u8; 32], Session>,
}
pub(crate) fn token(prefix: &str) -> String {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).expect("OS random source unavailable");
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}
fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}
impl Auth {
    pub(crate) fn new() -> Self {
        Self {
            pairing: token("tdp_"),
            pending: None,
            wrong: 0,
            sessions: HashMap::new(),
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn browser_token(&mut self, now: Instant) -> String {
        let created = self.mint(None, now).unwrap();
        self.sessions
            .get_mut(&digest(&created.token))
            .unwrap()
            .info
            .kind = SessionKind::Browser;
        created.token
    }
    pub(crate) fn pairing(&self) -> &str {
        &self.pairing
    }
    pub(crate) fn check_pairing(&self, headers: &HeaderMap) -> Result<(), CommonError> {
        let supplied = bearer(headers).ok_or(CommonError::AuthMissing)?;
        if bool::from(self.pairing.as_bytes().ct_eq(supplied.as_bytes())) {
            Ok(())
        } else {
            Err(CommonError::AuthInvalid)
        }
    }
    /// Reads the session token from `Authorization`, or on the events socket from the
    /// `touchdeck.bearer.*` subprotocol entry.
    pub(crate) fn authenticate(
        &mut self,
        headers: &HeaderMap,
        websocket: bool,
        now: Instant,
    ) -> Result<ClientInfo, CommonError> {
        let token = if websocket {
            headers
                .get_all("sec-websocket-protocol")
                .iter()
                .filter_map(|h| h.to_str().ok())
                .flat_map(|h| h.split(','))
                .find_map(|p| p.trim().strip_prefix("touchdeck.bearer."))
        } else {
            bearer(headers)
        };
        let token = token.ok_or(CommonError::AuthMissing)?;
        let session = self
            .sessions
            .get_mut(&digest(token))
            .ok_or(CommonError::AuthInvalid)?;
        if session.revoked {
            return Err(CommonError::AuthRevoked("The session was revoked."));
        }
        if session.expired(now) {
            if session.info.kind == SessionKind::Browser {
                return Err(CommonError::AuthExpired);
            }
            session.revoked = true;
            return Err(CommonError::AuthRevoked("The window session ended."));
        }
        session.info.last_used_ms = crate::now_ms();
        session.last_request = now;
        Ok(ClientInfo {
            id: session.info.id.clone(),
            kind: session.info.kind,
        })
    }
    fn mint(&mut self, label: Option<String>, now: Instant) -> Result<SessionCreated, CommonError> {
        if self.sessions.len() >= MAX_SESSIONS {
            // A revoked row still holding sockets keeps counting toward the socket caps.
            self.sessions.retain(|_, s| !s.revoked || s.sockets > 0);
        }
        if self.sessions.len() >= MAX_SESSIONS {
            return Err(CommonError::Limits(
                "Too many sessions have been created. Restart the Companion.",
            ));
        }
        let label: String = label
            .unwrap_or_else(|| "Window".into())
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_LABEL)
            .collect();
        let id = token("session_");
        let token = token("tds_");
        let wall = crate::now_ms();
        self.sessions.insert(
            digest(&token),
            Session {
                info: SessionInfo {
                    id: id.clone(),
                    kind: SessionKind::Window,
                    label,
                    created_ms: wall,
                    last_used_ms: wall,
                    current: false,
                },
                revoked: false,
                sockets: 0,
                idle_since: now,
                last_request: now,
                marked_expired: false,
            },
        );
        Ok(SessionCreated { id, token })
    }
    pub(crate) fn active(&self, id: &str, now: Instant) -> bool {
        self.sessions
            .values()
            .any(|s| s.info.id == id && s.live(now))
    }
    pub(crate) fn socket_open(&mut self, id: &str, now: Instant) -> Result<(), CommonError> {
        if self.sessions.values().map(|s| s.sockets).sum::<usize>() >= 16 {
            return Err(CommonError::Limits("The Companion has too many sockets."));
        }
        let s = self
            .sessions
            .values_mut()
            .find(|s| s.info.id == id && s.live(now))
            .ok_or(CommonError::AuthRevoked("The session was revoked."))?;
        if s.sockets >= 4 {
            return Err(CommonError::Limits("The session has too many sockets."));
        }
        s.sockets += 1;
        Ok(())
    }
    pub(crate) fn sweep(&mut self, now: Instant) -> bool {
        let mut ended_with_sockets = false;
        for s in self.sessions.values_mut() {
            if !s.revoked && !s.marked_expired && s.expired(now) {
                s.marked_expired = true;
                ended_with_sockets |= s.sockets > 0;
            }
        }
        self.sessions
            .retain(|_, s| s.sockets > 0 || !(s.revoked || s.expired(now)));
        ended_with_sockets
    }
    pub(crate) fn socket_close(&mut self, id: &str, now: Instant) {
        if let Some(s) = self.sessions.values_mut().find(|s| s.info.id == id) {
            s.sockets = s.sockets.saturating_sub(1);
            if s.sockets == 0 {
                s.idle_since = now;
            }
        }
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct ProofRequest {
    pub nonce: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct Proof {
    pub proof: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct SessionRequest {
    pub kind: SessionKind,
    pub label: Option<String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct SessionCreated {
    pub id: String,
    pub token: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct Versions {
    pub api: Vec<String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct SessionPath {
    pub id: String,
}

/// Public routes read at most this much body. Nothing is authenticated yet.
const PUBLIC_BODY_LIMIT: usize = 4096;

pub struct Ping;
impl Endpoint for Ping {
    const NAME: &'static str = "system.ping";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/system/ping";
    const BODY_LIMIT: usize = PUBLIC_BODY_LIMIT;
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Versions;
    type Error = CommonError;
    async fn handle(_: Ctx, _: Req<Self>) -> Result<Versions, CommonError> {
        Ok(Versions {
            api: vec!["v1".into()],
        })
    }
}
pub struct ProofEndpoint;
impl Endpoint for ProofEndpoint {
    const NAME: &'static str = "auth.proof";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/auth/proof";
    const BODY_LIMIT: usize = PUBLIC_BODY_LIMIT;
    type Path = ();
    type Query = NoQuery;
    type Body = ProofRequest;
    type Ok = Proof;
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Proof, CommonError> {
        let nonce = URL_SAFE_NO_PAD
            .decode(req.body.nonce)
            .map_err(|_| CommonError::invalid("nonce", "Expected base64url."))?;
        if nonce.len() != 32 {
            return Err(CommonError::invalid("nonce", "Expected 32 bytes."));
        }
        let pairing = ctx.auth.lock().pairing.clone();
        let mut mac = Hmac::<Sha256>::new_from_slice(pairing.as_bytes()).unwrap();
        mac.update(b"touchdeck-companion-proof\0");
        mac.update(&nonce);
        Ok(Proof {
            proof: URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()),
        })
    }
}
/// The guard checked the pairing token before the body was read.
pub struct SessionCreate;
impl Endpoint for SessionCreate {
    const NAME: &'static str = "auth.session_create";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/auth/sessions";
    const BODY_LIMIT: usize = PUBLIC_BODY_LIMIT;
    type Path = ();
    type Query = NoQuery;
    type Body = SessionRequest;
    type Ok = SessionCreated;
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<SessionCreated, CommonError> {
        if req.body.kind != SessionKind::Window {
            return Err(CommonError::Forbidden(
                "Only window sessions can be created here.",
            ));
        }
        let now = ctx.clock.now();
        ctx.auth.lock().mint(req.body.label, now)
    }
}
pub struct Sessions;
impl Endpoint for Sessions {
    const NAME: &'static str = "auth.sessions";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/auth/sessions";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<SessionInfo>;
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Vec<SessionInfo>, CommonError> {
        let own = &req.client().id;
        let now = ctx.clock.now();
        let mut list: Vec<_> = ctx
            .auth
            .lock()
            .sessions
            .values()
            .filter(|s| s.live(now))
            .map(|s| SessionInfo {
                current: s.info.id == *own,
                ..s.info.clone()
            })
            .collect();
        list.sort_by_key(|s| s.created_ms);
        Ok(list)
    }
}
#[derive(Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum BrowserKind {
    #[default]
    Browser,
}
#[derive(Default, Serialize, Deserialize, TS)]
pub struct SessionsDeleteQuery {
    pub kind: BrowserKind,
}
pub struct SessionsDelete;
impl Endpoint for SessionsDelete {
    const NAME: &'static str = "auth.sessions_delete";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/auth/sessions";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = SessionsDeleteQuery;
    type Body = ();
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<(), CommonError> {
        if req.client().kind != SessionKind::Window {
            return Err(CommonError::Forbidden("A window session is required."));
        }
        {
            let mut auth = ctx.auth.lock();
            for session in auth.sessions.values_mut() {
                if session.info.kind == SessionKind::Browser {
                    session.revoked = true;
                }
            }
        }
        ctx.events.notify();
        Ok(())
    }
}
pub struct SessionDelete;
impl Endpoint for SessionDelete {
    const NAME: &'static str = "auth.session_delete";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/auth/sessions/{id}";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = SessionPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<(), CommonError> {
        let id = match req.path.id.as_str() {
            "self" => req.client().id.clone(),
            id => id.to_owned(),
        };
        if let Some(s) = ctx
            .auth
            .lock()
            .sessions
            .values_mut()
            .find(|s| s.info.id == id)
        {
            s.revoked = true;
        }
        ctx.events.notify();
        Ok(())
    }
}
/// Reset pairing: a new pairing token, every session revoked, the runtime file rewritten.
pub struct Reset;
impl Endpoint for Reset {
    const NAME: &'static str = "auth.reset";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/auth/reset";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<(), CommonError> {
        reset(ctx).await
    }
}
pub(super) async fn reset(ctx: Ctx) -> Result<(), CommonError> {
    {
        let mut auth = ctx.auth.lock();
        auth.pairing = token("tdp_");
        auth.pending = None;
        for s in auth.sessions.values_mut() {
            s.revoked = true;
        }
    }
    ctx.events.notify();
    if !crate::process::rewrite_runtime(&ctx).await {
        let _ = ctx.shutdown.send(true);
        return Err(CommonError::Internal);
    }
    Ok(())
}

fn auth_feature() -> Feature {
    Feature::new("auth")
        .route::<ProofEndpoint>(Access::Public)
        .route::<SessionCreate>(Access::Pairing)
        .route::<Pair>(Access::Public)
        .endpoint::<PairCode>()
        .endpoint::<Sessions>()
        .endpoint::<SessionDelete>()
        .endpoint::<SessionsDelete>()
        .endpoint::<Reset>()
}
inventory::submit! { Registration(auth_feature) }
/// `system.ping` is public, so it's declared here and merges into the `system` feature.
fn ping_feature() -> Feature {
    Feature::new("system").route::<Ping>(Access::Public)
}
inventory::submit! { Registration(ping_feature) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_normalization_accepts_every_alias() {
        assert_eq!(normalized(" oIiL- abcd 2345 "), "0111ABCD2345");
    }

    #[test]
    fn a_full_table_prunes_revoked_sessions_without_sockets() {
        let mut auth = Auth::new();
        let now = Instant::now();
        let mut ids = Vec::new();
        for _ in 0..MAX_SESSIONS {
            ids.push(auth.mint(None, now).unwrap().id);
        }
        assert!(auth.mint(None, now).is_err());
        auth.socket_open(&ids[0], now).unwrap();
        for s in auth.sessions.values_mut() {
            s.revoked = true;
        }
        let fresh = auth.mint(None, now).unwrap();
        // The revoked row with an open socket stays, so the global socket cap still counts it.
        assert_eq!(auth.sessions.len(), 2);
        let open: usize = auth.sessions.values().map(|s| s.sockets).sum();
        assert_eq!(open, 1);
        assert!(auth.active(&fresh.id, now));
    }

    #[test]
    fn the_sweep_keeps_live_sessions_and_sessions_with_sockets() {
        let mut auth = Auth::new();
        let start = Instant::now();
        let revoked = auth.mint(None, start).unwrap().id;
        let idle = auth.mint(None, start).unwrap().id;
        let connected = auth.mint(None, start).unwrap().id;
        let revoked_connected = auth.mint(None, start).unwrap().id;
        auth.socket_open(&connected, start).unwrap();
        auth.socket_open(&revoked_connected, start).unwrap();
        for s in auth.sessions.values_mut() {
            if s.info.id == revoked || s.info.id == revoked_connected {
                s.revoked = true;
            }
        }
        auth.sweep(start);
        assert_eq!(auth.sessions.len(), 3);
        let later = start + WINDOW_IDLE;
        auth.sweep(later);
        assert_eq!(auth.sessions.len(), 2);
        let left: Vec<_> = auth.sessions.values().map(|s| s.info.id.clone()).collect();
        assert!(left.contains(&connected) && left.contains(&revoked_connected));
        assert!(!left.contains(&idle));
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct PairRequest {
    pub code: String,
    pub label: Option<String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct PairExpires {
    pub expires_ms: u64,
}
fn normalized(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect()
}
pub(super) async fn pair_code(ctx: Ctx) -> Result<PairExpires, CommonError> {
    let mut bytes = [0; 12];
    getrandom::fill(&mut bytes).map_err(|_| CommonError::Internal)?;
    let alphabet = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let raw: String = bytes
        .iter()
        .map(|b| alphabet[(b & 31) as usize] as char)
        .collect();
    let code = format!("{}-{}-{}", &raw[..4], &raw[4..8], &raw[8..]);
    {
        let mut auth = ctx.auth.lock();
        auth.pending = Some((digest(&raw), ctx.clock.now() + Duration::from_secs(120)));
        auth.wrong = 0;
    }
    let expires_ms = crate::now_ms() + 120_000;
    let desktop = ctx.desktop.clone();
    let port = ctx.port;
    tokio::task::spawn_blocking(move || desktop.deliver(port, &code))
        .await
        .map_err(|_| CommonError::Internal)??;
    Ok(PairExpires { expires_ms })
}
pub struct PairCode;
impl Endpoint for PairCode {
    const NAME: &'static str = "auth.pair_code";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/auth/pair-codes";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = PairExpires;
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        if req.client().kind != SessionKind::Window {
            return Err(CommonError::Forbidden("A window session is required."));
        }
        pair_code(ctx).await
    }
}
pub struct Pair;
impl Endpoint for Pair {
    const NAME: &'static str = "auth.pair";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/auth/pair";
    const BODY_LIMIT: usize = PUBLIC_BODY_LIMIT;
    type Path = ();
    type Query = NoQuery;
    type Body = PairRequest;
    type Ok = SessionCreated;
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let mut auth = ctx.auth.lock();
        if auth.wrong >= 5 {
            return Err(CommonError::CodeLocked);
        }
        let now = ctx.clock.now();
        let supplied = digest(&normalized(&req.body.code));
        if !auth
            .pending
            .as_ref()
            .is_some_and(|(hash, until)| now < *until && bool::from(hash.ct_eq(&supplied)))
        {
            auth.wrong += 1;
            if auth.wrong == 5 {
                auth.pending = None;
                ctx.desktop
                    .notify("Pairing was blocked after wrong codes. Start pairing again.");
            }
            return Err(CommonError::CodeInvalid);
        }
        auth.pending = None;
        auth.wrong = 0;
        let created = auth.mint(req.body.label.or(Some("Browser".into())), now)?;
        auth.sessions
            .get_mut(&digest(&created.token))
            .unwrap()
            .info
            .kind = SessionKind::Browser;
        Ok(created)
    }
}
