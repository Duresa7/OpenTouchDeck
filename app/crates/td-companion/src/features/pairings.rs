//! The apps a radio-build Device keeps Pairings with, under each Device: the list, and forgetting one or every one.
//! Every route works only over USB, from net-v2's `net.list` and `net.forget`. No answer carries a key.
use crate::{
    api::{ApiError, Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    devices::DeviceId,
    features::devices::DevicePath,
};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use crate::devices::{DevicePairing, DevicePairings, DevicePairingsError};
#[derive(Serialize, Deserialize, TS)]
pub struct AppPath {
    pub device: DeviceId,
    /// The app's id, 32 lower-case hex characters.
    pub app: String,
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
        ctx.device_operations().pairings_get(&req.path.device).await
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
        ctx.device_operations()
            .pairings_forget(&req.path.device, &req.path.app)
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
        ctx.device_operations()
            .pairings_forget_all(&req.path.device)
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
