//! The kernel: the `Endpoint` trait, feature registration, the request guard and dispatch.
//! `auth` is private, so a feature can't reach tokens or declare a public route.
mod auth;
mod error;
pub mod security;
mod socket;
mod ui;

pub use crate::events::Events;
pub use crate::jobs::{JobKind, Jobs};
pub(crate) use auth::Auth;
pub use auth::{ClientInfo, SessionInfo, SessionKind};
use axum::{
    Router,
    body::to_bytes,
    extract::{MatchedPath, Path, Request, State},
    http::{HeaderMap, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{MethodFilter, on},
};
pub use error::{ApiError, CommonError, ErrorBody};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
pub use socket::{Event, Topic};
use std::{collections::HashMap, future::Future, sync::Arc};
use ts_rs::TS;

/// The local web adapter's state. Product behaviour lives in `companion::Handle`.
#[derive(Clone)]
pub struct Ctx {
    companion: crate::companion::Handle,
    pub(crate) web_events: socket::WebEvents,
    pub(crate) uploads: crate::features::uploads::Uploads,
    auth: Arc<Mutex<Auth>>,
    pub(crate) port: u16,
    pub(crate) runtime: Option<Arc<crate::runtime::Runtime>>,
    pub(crate) limits: crate::Limits,
    pub(crate) started_ms: u64,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) handler_hits: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) real_device_console: bool,
}
impl std::ops::Deref for Ctx {
    type Target = crate::companion::Handle;
    fn deref(&self) -> &Self::Target {
        &self.companion
    }
}
impl Ctx {
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn companion_handle(&self) -> crate::companion::Handle {
        self.companion.clone()
    }
    pub(crate) fn new(
        companion: crate::companion::Handle,
        port: u16,
        runtime: Option<Arc<crate::runtime::Runtime>>,
        limits: crate::Limits,
    ) -> Self {
        let auth = Arc::new(Mutex::new(Auth::new()));
        let lease_auth = auth.clone();
        let lease_clock = companion.clock.clone();
        companion.jobs.session_check(Arc::new(move |id| {
            lease_auth.lock().active(id, lease_clock.now())
        }));
        Self {
            web_events: socket::WebEvents::new(&companion.events),
            companion,
            auth,
            port,
            runtime,
            limits,
            uploads: Default::default(),
            started_ms: crate::now_ms(),
            #[cfg(any(test, feature = "test-hooks"))]
            handler_hits: Default::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            real_device_console: false,
        }
    }
    #[cfg(windows)]
    pub(crate) async fn pair_browser(&self) -> Result<(), CommonError> {
        auth::pair_code(self.clone()).await.map(|_| ())
    }
    #[cfg(windows)]
    pub(crate) async fn reset_pairing(&self) -> Result<(), CommonError> {
        auth::reset(self.clone()).await
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn pairing_token(&self) -> String {
        self.auth.lock().pairing().to_owned()
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn browser_token(&self) -> String {
        self.auth.lock().browser_token(self.clock.now())
    }
    pub(crate) fn sweep_sessions(&self) {
        let now = self.clock.now();
        let mut auth = self.auth.lock();
        self.uploads.sweep(now, |id| auth.active(id, now));
        let ended_with_sockets = auth.sweep(now);
        drop(auth);
        if ended_with_sockets {
            self.web_events.notify();
        }
    }
    pub(crate) fn runtime_value(&self) -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "api": ["v1"],
            "port": self.port,
            "token": self.auth.lock().pairing(),
            "pid": std::process::id(),
            "build": env!("TD_BUILD"),
            "started_at_ms": self.started_ms,
        })
    }
}

pub trait RequestBody: DeserializeOwned + Serialize + TS + Send + 'static {}
impl<T: DeserializeOwned + Serialize + TS + Send + 'static> RequestBody for T {}
pub trait ResponseBody: DeserializeOwned + Serialize + TS + Send + 'static {}
impl<T: DeserializeOwned + Serialize + TS + Send + 'static> ResponseBody for T {}

/// One route, declared once. The router, the typed test client and the generated TypeScript
/// client all read this declaration.
pub trait Endpoint: Sized + Send + Sync + 'static {
    const NAME: &'static str;
    const METHOD: Method;
    const PATH: &'static str;
    const STATUS: StatusCode = StatusCode::OK;
    const BODY_LIMIT: usize = 1 << 20;
    /// Waits for a person at a native dialog, so the whole-request limit doesn't apply.
    /// The header and body limits still do.
    const UNTIMED: bool = false;
    fn read_body(
        ctx: &Ctx,
        headers: &HeaderMap,
        _client: Option<&ClientInfo>,
        body: axum::body::Body,
    ) -> impl Future<Output = Result<Self::Body, CommonError>> + Send {
        async move {
            let bytes =
                tokio::time::timeout(ctx.limits.body_read, to_bytes(body, Self::BODY_LIMIT))
                    .await
                    .map_err(|_| CommonError::Timeout)?
                    .map_err(|_| CommonError::TooLarge)?;
            if !bytes.is_empty() && !is_json(headers) {
                return Err(CommonError::MediaType);
            }
            serde_json::from_slice(if bytes.is_empty() { b"null" } else { &bytes })
                .map_err(|e| CommonError::invalid("body", e))
        }
    }
    type Path: DeserializeOwned + Serialize + TS + Send + 'static;
    type Query: DeserializeOwned + Serialize + TS + Send + Default + 'static;
    type Body: RequestBody;
    type Ok: ResponseBody;
    type Error: ApiError;
    fn handle(
        ctx: Ctx,
        req: Req<Self>,
    ) -> impl Future<Output = Result<Self::Ok, Self::Error>> + Send;
    /// Turns a success into the response: `STATUS` with the JSON value, or no body for `204`.
    /// Routes that send headers or raw bytes override it.
    fn respond(value: Self::Ok) -> Response {
        if Self::STATUS == StatusCode::NO_CONTENT {
            Self::STATUS.into_response()
        } else {
            (Self::STATUS, axum::Json(value)).into_response()
        }
    }
}
pub struct Req<E: Endpoint> {
    pub path: E::Path,
    pub query: E::Query,
    pub body: E::Body,
    /// The request headers, for routes that read one such as `If-Match`.
    pub headers: HeaderMap,
    client: Option<ClientInfo>,
}
impl<E: Endpoint> Req<E> {
    /// The caller's session. Every route a feature declares needs one, so this always holds a
    /// session there. Only the public routes `api::auth` declares have none.
    pub fn client(&self) -> &ClientInfo {
        self.client
            .as_ref()
            .expect("feature routes always carry a session")
    }
}
pub type NoBody = ();
/// The query type of a route that takes none. Any query string is ignored.
#[derive(Debug, Default, Serialize, TS)]
pub struct NoQuery;
impl<'de> Deserialize<'de> for NoQuery {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(d).map(|_| NoQuery)
    }
}

/// Who may call a route. Only `api::auth` registers anything but `Session`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// A valid session token.
    Session,
    /// The pairing token as the bearer.
    Pairing,
    /// No credential.
    Public,
}

/// Collects TypeScript declarations for the generated client.
#[cfg(any(test, feature = "test-hooks"))]
pub type CollectTypes = fn(&mut crate::bindings::Decls);
pub struct EndpointInfo {
    pub name: &'static str,
    pub method: Method,
    pub path: &'static str,
    pub access: Access,
    pub untimed: bool,
    pub errors: &'static [&'static str],
    #[cfg(any(test, feature = "test-hooks"))]
    pub path_type: String,
    #[cfg(any(test, feature = "test-hooks"))]
    pub query_type: String,
    #[cfg(any(test, feature = "test-hooks"))]
    pub body_type: String,
    #[cfg(any(test, feature = "test-hooks"))]
    pub ok_type: String,
    #[cfg(any(test, feature = "test-hooks"))]
    pub types: CollectTypes,
}
pub struct JobKindInfo {
    pub name: &'static str,
    #[cfg(any(test, feature = "test-hooks"))]
    pub progress_type: String,
    #[cfg(any(test, feature = "test-hooks"))]
    pub result_type: String,
    #[cfg(any(test, feature = "test-hooks"))]
    pub types: CollectTypes,
}
pub struct EventInfo {
    pub name: &'static str,
    #[cfg(any(test, feature = "test-hooks"))]
    pub data_type: String,
    #[cfg(any(test, feature = "test-hooks"))]
    pub types: CollectTypes,
}
pub type ReadSnapshot = Box<dyn Fn(&Ctx) -> serde_json::Value + Send + Sync>;
pub struct Snapshot {
    pub key: &'static str,
    pub read: ReadSnapshot,
    #[cfg(any(test, feature = "test-hooks"))]
    pub data_type: String,
    #[cfg(any(test, feature = "test-hooks"))]
    pub types: CollectTypes,
}
pub struct Feature {
    pub name: &'static str,
    pub endpoints: Vec<EndpointInfo>,
    pub events: Vec<EventInfo>,
    pub snapshots: Vec<Snapshot>,
    pub jobs: Vec<JobKindInfo>,
    router: Router<Ctx>,
}
#[cfg(any(test, feature = "test-hooks"))]
fn collect<T: TS + 'static + ?Sized>(decls: &mut crate::bindings::Decls) {
    decls.add::<T>();
}
impl Feature {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            endpoints: vec![],
            events: vec![],
            snapshots: vec![],
            jobs: vec![],
            router: Router::new(),
        }
    }
    pub fn event<E: Event>(mut self) -> Self {
        self.events.push(EventInfo {
            name: E::NAME,
            #[cfg(any(test, feature = "test-hooks"))]
            data_type: E::name(),
            #[cfg(any(test, feature = "test-hooks"))]
            types: collect::<E>,
        });
        self
    }
    /// A Job kind, so the generated `JobMap` types its progress and result.
    pub fn job<K: JobKind>(mut self) -> Self {
        #[cfg(any(test, feature = "test-hooks"))]
        fn types<K: JobKind>(decls: &mut crate::bindings::Decls) {
            decls.add::<K::Progress>();
            decls.add::<K::Result>();
        }
        self.jobs.push(JobKindInfo {
            name: K::NAME,
            #[cfg(any(test, feature = "test-hooks"))]
            progress_type: K::Progress::name(),
            #[cfg(any(test, feature = "test-hooks"))]
            result_type: K::Result::name(),
            #[cfg(any(test, feature = "test-hooks"))]
            types: types::<K>,
        });
        self
    }
    /// A `sync` key, filled from `read` for every new socket.
    pub fn snapshot<T: Serialize + TS + 'static>(
        mut self,
        key: &'static str,
        read: fn(&Ctx) -> T,
    ) -> Self {
        self.snapshots.push(Snapshot {
            key,
            read: Box::new(move |ctx| serde_json::to_value(read(ctx)).unwrap()),
            #[cfg(any(test, feature = "test-hooks"))]
            data_type: T::name(),
            #[cfg(any(test, feature = "test-hooks"))]
            types: collect::<T>,
        });
        self
    }
    pub fn endpoint<E: Endpoint>(self) -> Self {
        self.route::<E>(Access::Session)
    }
    /// Registers a route with another access level. Visible only inside `api`.
    fn route<E: Endpoint>(mut self, access: Access) -> Self {
        #[cfg(any(test, feature = "test-hooks"))]
        fn types<E: Endpoint>(decls: &mut crate::bindings::Decls) {
            decls.add::<E::Path>();
            decls.add::<E::Query>();
            decls.add::<E::Body>();
            decls.add::<E::Ok>();
        }
        self.endpoints.push(EndpointInfo {
            name: E::NAME,
            method: E::METHOD,
            path: E::PATH,
            access,
            untimed: E::UNTIMED,
            errors: E::Error::CODES,
            #[cfg(any(test, feature = "test-hooks"))]
            path_type: E::Path::inline(),
            #[cfg(any(test, feature = "test-hooks"))]
            query_type: E::Query::inline(),
            #[cfg(any(test, feature = "test-hooks"))]
            body_type: E::Body::inline(),
            #[cfg(any(test, feature = "test-hooks"))]
            ok_type: E::Ok::name(),
            #[cfg(any(test, feature = "test-hooks"))]
            types: types::<E>,
        });
        self.router = self.router.route(
            &format!("/api/v1{}", E::PATH),
            on(MethodFilter::try_from(E::METHOD).unwrap(), dispatch::<E>),
        );
        self
    }
}
pub struct Registration(pub fn() -> Feature);
inventory::collect!(Registration);

/// Every feature, sorted by name. Registrations that share a name, such as the `system`
/// routes `api::auth` declares next to the `system` feature, merge into one.
pub fn registry() -> Vec<Feature> {
    let mut all: Vec<Feature> = inventory::iter::<Registration>
        .into_iter()
        .map(|r| (r.0)())
        .collect();
    all.sort_by_key(|f| f.name);
    let mut merged: Vec<Feature> = Vec::new();
    for feature in all {
        match merged.last_mut() {
            Some(last) if last.name == feature.name => {
                last.endpoints.extend(feature.endpoints);
                last.events.extend(feature.events);
                last.snapshots.extend(feature.snapshots);
                last.jobs.extend(feature.jobs);
                let router = std::mem::take(&mut last.router);
                last.router = router.merge(feature.router);
            }
            _ => merged.push(feature),
        }
    }
    for feature in &mut merged {
        feature.endpoints.sort_by_key(|e| e.name);
    }
    merged
}
/// The frames the kernel itself sends.
pub fn kernel_events() -> Vec<EventInfo> {
    socket::kernel_events()
}

async fn dispatch<E: Endpoint>(State(ctx): State<Ctx>, request: Request) -> Response {
    let result: Result<_, CommonError> = async {
        let (mut parts, body) = request.into_parts();
        let client = parts.extensions.remove::<ClientInfo>();
        let path = if !E::PATH.contains('{') {
            serde_json::from_value(serde_json::Value::Null)
                .map_err(|e| CommonError::invalid("path", e))?
        } else {
            use axum::extract::FromRequestParts;
            Path::<E::Path>::from_request_parts(&mut parts, &ctx)
                .await
                .map_err(|e| CommonError::invalid("path", e.body_text()))?
                .0
        };
        let query = serde_urlencoded::from_str(parts.uri.query().unwrap_or(""))
            .map_err(|e| CommonError::invalid("query", e))?;
        let body = E::read_body(&ctx, &parts.headers, client.as_ref(), body).await?;
        #[cfg(any(test, feature = "test-hooks"))]
        ctx.handler_hits
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(E::handle(
            ctx,
            Req {
                path,
                query,
                body,
                headers: parts.headers,
                client,
            },
        ))
    }
    .await;
    match result {
        Err(e) => e.into_response(),
        Ok(handled) => match handled.await {
            Err(e) => error::respond(&e),
            Ok(value) => E::respond(value),
        },
    }
}

/// Builds the router from the registry. Every request, routed or not, passes `guard` first.
pub fn router(ctx: Ctx) -> Router {
    build_router(ctx, registry())
}
/// The production route set, also exercised by tests built with test hooks.
pub fn release_router(ctx: Ctx) -> Router {
    build_router(
        ctx,
        registry()
            .into_iter()
            .filter(|f| f.name != "test")
            .collect(),
    )
}
fn build_router(ctx: Ctx, features: Vec<Feature>) -> Router {
    let mut router = Router::new();
    let mut access = HashMap::new();
    for feature in features {
        for e in &feature.endpoints {
            access.insert(
                (e.method.clone(), format!("/api/v1{}", e.path)),
                (e.access, e.untimed),
            );
        }
        router = router.merge(feature.router);
    }
    router = router.route("/api/v1/events", axum::routing::get(socket::upgrade));
    let access = Arc::new(access);
    router
        .fallback(ui::serve)
        .method_not_allowed_fallback(|| async { CommonError::MethodNotAllowed })
        .layer(middleware::from_fn_with_state((ctx.clone(), access), guard))
        .with_state(ctx)
}

type Routes = HashMap<(Method, String), (Access, bool)>;
type Guard = (Ctx, Arc<Routes>);
async fn guard(State((ctx, access)): State<Guard>, request: Request, next: Next) -> Response {
    let port = ctx.port;
    let response = match admit(&ctx, &access, request) {
        // The upgrade answers at once; the socket then runs outside this request.
        Ok(request) if request.uri().path() == "/api/v1/events" || untimed(&access, &request) => {
            error::normalize(next.run(request).await)
        }
        Ok(request) => match tokio::time::timeout(ctx.limits.request, next.run(request)).await {
            Ok(response) => error::normalize(response),
            Err(_) => CommonError::Timeout.into_response(),
        },
        Err(e) => e.into_response(),
    };
    with_security_headers(response, port, ctx.brandfetch.search_enabled())
}
fn untimed(access: &Routes, request: &Request) -> bool {
    request
        .extensions()
        .get::<MatchedPath>()
        .and_then(|p| access.get(&(request.method().clone(), p.as_str().to_owned())))
        .is_some_and(|(_, untimed)| *untimed)
}
fn admit(ctx: &Ctx, access: &Routes, mut request: Request) -> Result<Request, CommonError> {
    let h = request.headers();
    // A repeated header is ambiguous, so it counts as an invalid value, never as its first one.
    let header = |name: &str| {
        let mut values = h.get_all(name).iter();
        let first = values.next().map(|v| v.to_str().unwrap_or(""));
        if values.next().is_some() {
            Some("")
        } else {
            first
        }
    };
    security::admit(
        &security::RequestMeta {
            host: header("host"),
            origin: header("origin"),
            fetch_site: header("sec-fetch-site"),
            fetch_mode: header("sec-fetch-mode"),
            api: ui::is_api(request.uri().path()),
            get: request.method() == Method::GET,
        },
        ctx.port,
    )?;
    if matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH
    ) && h.contains_key("content-type")
        && !is_json(h)
        && !(request.method() == Method::POST
            && request.uri().path() == "/api/v1/uploads"
            && header("content-type").is_some_and(|v| v.starts_with("image/")))
    {
        return Err(CommonError::MediaType);
    }
    if !ui::is_api(request.uri().path()) {
        return Ok(request);
    }
    // Access is looked up by the route the request matched, so a request that matches no
    // public declaration needs a session. That includes unknown paths and HEAD.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned());
    let level = route
        .and_then(|r| access.get(&(request.method().clone(), r)).map(|(a, _)| *a))
        .unwrap_or(Access::Session);
    let websocket = request.uri().path() == "/api/v1/events";
    let now = ctx.clock.now();
    match level {
        Access::Public => {}
        Access::Pairing => ctx.auth.lock().check_pairing(request.headers())?,
        Access::Session => {
            let client = ctx
                .auth
                .lock()
                .authenticate(request.headers(), websocket, now)?;
            request.extensions_mut().insert(client);
        }
    }
    Ok(request)
}
/// `brandfetch` opens the CSP to Brandfetch's search API and logo images, only after the user
/// saved a Brandfetch client ID. Without it the policy keeps every request on this origin.
fn with_security_headers(mut response: Response, port: u16, brandfetch: bool) -> Response {
    let headers = response.headers_mut();
    for (k, v) in [
        ("x-frame-options", "DENY"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        headers.insert(k, v.parse().unwrap());
    }
    headers
        .entry("cache-control")
        .or_insert("no-store".parse().unwrap());
    let (img, connect) = if brandfetch {
        (
            " https://cdn.brandfetch.io https://asset.brandfetch.io",
            " https://api.brandfetch.io",
        )
    } else {
        ("", "")
    };
    let csp = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' blob: data:{img}; connect-src 'self' ws://127.0.0.1:{port}{connect}; \
         font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'none'"
    );
    // A response that sets a stricter policy of its own, such as an Icon pack thumbnail,
    // keeps it.
    headers
        .entry("content-security-policy")
        .or_insert(csp.parse().unwrap());
    response
}

/// Whether the request declares a JSON body. A body without this header is refused, so a
/// cross-site form post never reaches a handler.
pub(crate) fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next().unwrap_or("").trim() == "application/json")
}

/// Checks the runtime file off the async runtime and rewrites it on drift. Returns false when
/// it couldn't be secured after retries.
pub(crate) async fn rewrite_runtime(ctx: &Ctx) -> bool {
    let Some(runtime) = ctx.runtime.clone() else {
        return true;
    };
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || runtime.repair(&ctx.runtime_value()).is_ok())
        .await
        .unwrap_or(false)
}
/// The web adapter's 60 s maintenance step: sweeps ended sessions, then
/// checks the runtime file. Returns false when the Companion must stop.
pub(crate) async fn maintain(ctx: &Ctx) -> bool {
    ctx.sweep_sessions();
    let ok = rewrite_runtime(ctx).await;
    if !ok {
        ctx.desktop
            .notify("The runtime file failed its check. Restart Open TouchDeck.");
    }
    ok
}
