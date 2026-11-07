use crate::api::{CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req, SessionKind};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
#[derive(Serialize, Deserialize, TS)]
pub struct SystemInfo {
    pub version: String,
    pub build: String,
    pub api: String,
    pub mock: bool,
    pub data_dir_override: bool,
    pub platform: Platform,
}
/// The operating system the Companion runs on: Windows or Linux, which Open TouchDeck
/// supports, or another. The UI names the keychain from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
    Other,
}
impl Platform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }
}
pub struct Get;
impl Endpoint for Get {
    const NAME: &'static str = "system.get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/system";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = SystemInfo;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(SystemInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            build: env!("TD_BUILD").into(),
            api: "v1".into(),
            mock: ctx.mock,
            data_dir_override: std::env::var_os("TOUCHDECK_APP_DATA_DIR").is_some(),
            platform: Platform::current(),
        })
    }
}
pub struct Shutdown;
impl Endpoint for Shutdown {
    const NAME: &'static str = "system.shutdown";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/system/shutdown";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        if req.client().kind != SessionKind::Window {
            return Err(CommonError::Forbidden("A window session is required."));
        }
        let _ = ctx.shutdown.send(true);
        Ok(())
    }
}
fn feature() -> Feature {
    Feature::new("system")
        .endpoint::<Get>()
        .endpoint::<Shutdown>()
        .endpoint::<Lock>()
        .endpoint::<LockRenew>()
        .endpoint::<Unlock>()
}
inventory::submit! {Registration(feature)}

#[derive(Serialize, Deserialize, TS)]
pub struct InstallLock {
    pub lock: String,
}
pub struct Lock;
impl Endpoint for Lock {
    const NAME: &'static str = "system.lock";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/system/lock";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = InstallLock;
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        if req.client().kind != SessionKind::Window {
            return Err(CommonError::Forbidden("A window session is required."));
        }
        Ok(InstallLock {
            lock: ctx.jobs.install_lock(&req.client().id)?,
        })
    }
}
macro_rules! lock_route {
    ($name:ident, $method:ident, $route:literal, $release:literal) => {
        pub struct $name;
        impl Endpoint for $name {
            const NAME: &'static str = $route;
            const METHOD: Method = Method::$method;
            const PATH: &'static str = "/system/lock/{lock}";
            const STATUS: StatusCode = StatusCode::NO_CONTENT;
            type Path = InstallLock;
            type Query = NoQuery;
            type Body = ();
            type Ok = ();
            type Error = CommonError;
            async fn handle(ctx: Ctx, req: Req<Self>) -> Result<(), CommonError> {
                if req.client().kind != SessionKind::Window {
                    return Err(CommonError::Forbidden("A window session is required."));
                }
                ctx.jobs
                    .install_renew(&req.client().id, &req.path.lock, $release)
            }
        }
    };
}
lock_route!(LockRenew, PUT, "system.lock_renew", false);
lock_route!(Unlock, DELETE, "system.unlock", true);
