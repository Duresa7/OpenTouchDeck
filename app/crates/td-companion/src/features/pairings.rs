//! The apps a radio-build Device keeps Pairings with, under each Device: the list, and forgetting one or every one.
//! Every route works only over USB, from net-v2's `net.list` and `net.forget`. No answer carries a key.
use crate::{
    api::{ApiError, CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    devices::{Carrier, DeviceId, Reach},
    features::devices::{DeviceError, DevicePath},
};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use td_link::{
    Session,
    net::CompanionId,
    pairings::{self, Forget, NetPairings, PairingsError},
};
use ts_rs::TS;

/// One app the Device keeps a Pairing with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DevicePairing {
    /// The app's id: 32 lower-case hex characters.
    pub app: String,
    /// The name the app gave when it paired.
    pub name: String,
    /// The app is this Companion.
    pub this_app: bool,
}
/// The apps a Device keeps Pairings with, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DevicePairings {
    pub pairings: Vec<DevicePairing>,
    /// The most Pairings a Device keeps, 4.
    pub max: u32,
    /// Whether the Device's pairing window is open.
    pub pairing: bool,
    /// Milliseconds left in a lockout after wrong codes, 0 when none.
    pub locked_ms: u64,
}
#[derive(Serialize, Deserialize, TS)]
pub struct AppPath {
    pub device: DeviceId,
    /// The app's id, 32 lower-case hex characters.
    pub app: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DevicePairingsError {
    #[error("The Device couldn't erase the Pairing from its storage. Try again.")]
    Storage,
    #[error(transparent)]
    Device(#[from] DeviceError),
}
impl ApiError for DevicePairingsError {
    const CODES: &'static [&'static str] = &[
        "pairings.storage",
        "device.not_found",
        "device.not_connected",
        "device.timeout",
        "device.link",
        "device.unknown_outcome",
        "device.needs_firmware",
        "device.usb_only",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::Storage => StatusCode::BAD_GATEWAY,
            Self::Device(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::Storage => "pairings.storage",
            Self::Device(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Storage => None,
            Self::Device(e) => e.details(),
        }
    }
}
impl From<CommonError> for DevicePairingsError {
    fn from(error: CommonError) -> Self {
        Self::Device(error.into())
    }
}
impl From<Reach> for DevicePairingsError {
    fn from(reach: Reach) -> Self {
        Self::Device(reach.into())
    }
}
impl From<PairingsError> for DevicePairingsError {
    fn from(error: PairingsError) -> Self {
        match error {
            PairingsError::Link(error) => DeviceError::from(error).into(),
            PairingsError::Refused { reason, error } => match reason.as_str() {
                "storage" => Self::Storage,
                "usb-only" => DeviceError::UsbOnly.into(),
                "args" => CommonError::invalid("path", error).into(),
                // A Device without `net.*` answers the unknown-message error, with no reason.
                "unsupported" | "" => DeviceError::NeedsFirmware { field: "pairings" }.into(),
                _ => DeviceError::Link {
                    reason: format!("the Device refused: {error} ({reason})"),
                }
                .into(),
            },
        }
    }
}

fn view(listed: NetPairings, own: &CompanionId) -> DevicePairings {
    DevicePairings {
        pairings: listed
            .pairings
            .into_iter()
            .map(|p| DevicePairing {
                app: pairings::app_id_hex(&p.app_id),
                name: p.app_name,
                this_app: p.app_id == *own,
            })
            .collect(),
        max: pairings::MAX_PAIRINGS,
        pairing: listed.pairing,
        locked_ms: listed.locked_ms,
    }
}

/// Runs Pairing work on a radio-build Device over USB. A Device connected over Wi-Fi hears nothing.
async fn on_radio(
    ctx: &Ctx,
    id: &DeviceId,
    work: impl FnOnce(&Session) -> Result<NetPairings, PairingsError> + Send + 'static,
) -> Result<DevicePairings, DevicePairingsError> {
    let summary = ctx.devices.get(id).ok_or(DeviceError::NotFound)?;
    if summary.capabilities.as_ref().and_then(|c| c.wifi) != Some(true) {
        return Err(DeviceError::NeedsFirmware { field: "pairings" }.into());
    }
    if summary.carrier == Some(Carrier::Wifi) {
        return Err(DeviceError::UsbOnly.into());
    }
    let listed = ctx.devices.call(id, work).await??;
    Ok(view(listed, &ctx.devices.app_id()))
}

pub struct PairingsGet;
impl Endpoint for PairingsGet {
    const NAME: &'static str = "devices.pairings_get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices/{device}/pairings";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DevicePairings;
    type Error = DevicePairingsError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let devices = ctx.devices.clone();
        let id = req.path.device.clone();
        on_radio(&ctx, &req.path.device, move |s| {
            devices.device_pairings(&id, s)
        })
        .await
    }
}
pub struct PairingsForget;
impl Endpoint for PairingsForget {
    const NAME: &'static str = "devices.pairings_forget";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/devices/{device}/pairings/{app}";
    type Path = AppPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DevicePairings;
    type Error = DevicePairingsError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let Some(app) = pairings::parse_app_id(&req.path.app) else {
            return Err(CommonError::invalid("path", "app is 32 lower-case hex characters").into());
        };
        let devices = ctx.devices.clone();
        let id = req.path.device.clone();
        on_radio(&ctx, &req.path.device, move |s| {
            devices.device_forget(&id, s, Forget::One(app))
        })
        .await
    }
}
pub struct PairingsForgetAll;
impl Endpoint for PairingsForgetAll {
    const NAME: &'static str = "devices.pairings_forget_all";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/devices/{device}/pairings";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DevicePairings;
    type Error = DevicePairingsError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let devices = ctx.devices.clone();
        let id = req.path.device.clone();
        on_radio(&ctx, &req.path.device, move |s| {
            devices.device_forget(&id, s, Forget::All)
        })
        .await
    }
}
/// Registered under `devices`, so the routes and the generated client sit with the Device's other settings.
fn device_pairings() -> Feature {
    Feature::new("devices")
        .endpoint::<PairingsGet>()
        .endpoint::<PairingsForget>()
        .endpoint::<PairingsForgetAll>()
}
inventory::submit! { Registration(device_pairings) }
