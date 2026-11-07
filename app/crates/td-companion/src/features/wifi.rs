//! Wi-Fi: finding Devices on the LAN, pairing with the code a Device shows, and the Pairings
//! this Companion holds.
use crate::{
    api::{ApiError, CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    devices::{DeviceId, PairError, PairingStarted, WifiDevice, WifiPairing},
};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ts_rs::TS;

/// How long a discovery browses the LAN.
const BROWSE: Duration = Duration::from_secs(3);

#[derive(Debug, thiserror::Error)]
pub enum WifiError {
    #[error(transparent)]
    Pair(#[from] PairError),
    #[error("Discovery couldn't run: {0}. Type the Device's address instead.")]
    Discovery(String),
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl ApiError for WifiError {
    const CODES: &'static [&'static str] = &[
        "wifi.unreachable",
        "wifi.window_closed",
        "wifi.wrong_code",
        "wifi.locked",
        "wifi.full",
        "wifi.already_paired",
        "wifi.expired",
        "wifi.busy",
        "wifi.attempt_not_found",
        "wifi.not_paired",
        "wifi.keychain",
        "wifi.failed",
        "wifi.discovery",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::Pair(error) => match error {
                PairError::Unreachable(_) | PairError::Failed(_) => StatusCode::BAD_GATEWAY,
                PairError::WrongCode { .. } => StatusCode::UNPROCESSABLE_ENTITY,
                PairError::Locked => StatusCode::TOO_MANY_REQUESTS,
                PairError::AttemptNotFound | PairError::NotPaired => StatusCode::NOT_FOUND,
                PairError::Keychain(_) => StatusCode::SERVICE_UNAVAILABLE,
                PairError::WindowClosed
                | PairError::Full
                | PairError::AlreadyPaired
                | PairError::Expired
                | PairError::Busy => StatusCode::CONFLICT,
            },
            Self::Discovery(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Common(error) => error.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::Pair(error) => match error {
                PairError::Unreachable(_) => "wifi.unreachable",
                PairError::WindowClosed => "wifi.window_closed",
                PairError::WrongCode { .. } => "wifi.wrong_code",
                PairError::Locked => "wifi.locked",
                PairError::Full => "wifi.full",
                PairError::AlreadyPaired => "wifi.already_paired",
                PairError::Expired => "wifi.expired",
                PairError::Busy => "wifi.busy",
                PairError::AttemptNotFound => "wifi.attempt_not_found",
                PairError::NotPaired => "wifi.not_paired",
                PairError::Keychain(_) => "wifi.keychain",
                PairError::Failed(_) => "wifi.failed",
            },
            Self::Discovery(_) => "wifi.discovery",
            Self::Common(error) => error.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Pair(PairError::WrongCode { attempts_left }) => {
                Some(serde_json::json!({ "attempts_left": attempts_left }))
            }
            Self::Pair(PairError::Unreachable(reason) | PairError::Failed(reason)) => {
                Some(serde_json::json!({ "reason": reason }))
            }
            Self::Common(error) => error.details(),
            _ => None,
        }
    }
}

/// Runs blocking Wi-Fi work off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, WifiError> + Send + 'static,
) -> Result<T, WifiError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CommonError::Internal)?
}

pub struct Discover;
impl Endpoint for Discover {
    const NAME: &'static str = "wifi.discover";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/wifi/discover";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<WifiDevice>;
    type Error = WifiError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let devices = ctx.devices.clone();
        blocking(move || devices.wifi_discover(BROWSE).map_err(WifiError::Discovery)).await
    }
}

pub struct PairingsList;
impl Endpoint for PairingsList {
    const NAME: &'static str = "wifi.pairings_list";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/wifi/pairings";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<WifiPairing>;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(ctx.devices.wifi_pairings())
    }
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct PairStart {
    /// An IPv4 or IPv6 address or a host name, with an optional port (47820 by default).
    pub address: String,
}
pub struct PairStartEndpoint;
impl Endpoint for PairStartEndpoint {
    const NAME: &'static str = "wifi.pair_start";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/wifi/pairings";
    type Path = ();
    type Query = NoQuery;
    type Body = PairStart;
    type Ok = PairingStarted;
    type Error = WifiError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let devices = ctx.devices.clone();
        let address = req.body.address;
        blocking(move || Ok(devices.wifi_pair_start(&address)?)).await
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct AttemptPath {
    pub attempt: String,
}
#[derive(Debug, Serialize, Deserialize, TS)]
pub struct PairCode {
    /// The six digits the Device shows.
    pub code: String,
}
pub struct PairCodeEndpoint;
impl Endpoint for PairCodeEndpoint {
    const NAME: &'static str = "wifi.pair_code";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/wifi/attempts/{attempt}/code";
    type Path = AttemptPath;
    type Query = NoQuery;
    type Body = PairCode;
    type Ok = WifiPairing;
    type Error = WifiError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let code = req.body.code.trim().to_owned();
        if !td_link::net::wire::valid_code(&code) {
            return Err(CommonError::invalid("body", "code is six digits").into());
        }
        let devices = ctx.devices.clone();
        let attempt = req.path.attempt;
        blocking(move || Ok(devices.wifi_pair_code(&attempt, &code)?)).await
    }
}

pub struct PairCancel;
impl Endpoint for PairCancel {
    const NAME: &'static str = "wifi.pair_cancel";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/wifi/attempts/{attempt}";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = AttemptPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = WifiError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let devices = ctx.devices.clone();
        let attempt = req.path.attempt;
        blocking(move || Ok(devices.wifi_pair_cancel(&attempt)?)).await
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct PairingPath {
    pub device: DeviceId,
}
pub struct PairingsForget;
impl Endpoint for PairingsForget {
    const NAME: &'static str = "wifi.pairings_forget";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/wifi/pairings/{device}";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = PairingPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = WifiError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let devices = ctx.devices.clone();
        let device = req.path.device;
        blocking(move || Ok(devices.wifi_forget(&device)?)).await
    }
}

fn wifi() -> Feature {
    Feature::new("wifi")
        .endpoint::<Discover>()
        .endpoint::<PairingsList>()
        .endpoint::<PairStartEndpoint>()
        .endpoint::<PairCodeEndpoint>()
        .endpoint::<PairCancel>()
        .endpoint::<PairingsForget>()
}
inventory::submit! { Registration(wifi) }
