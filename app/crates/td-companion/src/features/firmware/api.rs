//! Frozen local web API: translate requests into the Firmware workflow.
use super::*;
use crate::api::{ApiError, Ctx, Endpoint, Feature, NoQuery, Registration, Req};
use axum::http::{Method, StatusCode};

impl ApiError for FirmwareError {
    const CODES: &'static [&'static str] = &[
        "firmware.bundle_invalid",
        "firmware.bundle_not_found",
        "new_board.not_found",
        "firmware.usb_only",
        "system.dialog_open",
        "firmware.declined",
        "firmware.channel_mismatch",
        "firmware.failed",
        "firmware.rolled_back",
        "firmware.confirm_timeout",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Missing | Self::NewBoardNotFound => StatusCode::NOT_FOUND,
            Self::Common(e) => e.status(),
            Self::Job(e) => e.status(),
            Self::Device(e) => e.status(),
            _ => StatusCode::CONFLICT,
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "firmware.bundle_invalid",
            Self::Missing => "firmware.bundle_not_found",
            Self::NewBoardNotFound => "new_board.not_found",
            Self::UsbOnly => "firmware.usb_only",
            Self::DialogOpen => "system.dialog_open",
            Self::Declined => "firmware.declined",
            Self::ChannelMismatch { .. } => "firmware.channel_mismatch",
            Self::Failed(s) if s == "rolled back" => "firmware.rolled_back",
            Self::Failed(s) if s == "confirm timeout" => "firmware.confirm_timeout",
            Self::Failed(_) => "firmware.failed",
            Self::Common(e) => e.code(),
            Self::Job(e) => e.code(),
            Self::Device(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Failed(reason) => Some(serde_json::json!({"reason":reason})),
            Self::ChannelMismatch { channel, bundle } => {
                Some(serde_json::json!({"channel":channel,"bundle":bundle}))
            }
            Self::Job(e) => e.details(),
            Self::Device(e) => e.details(),
            _ => None,
        }
    }
}
pub struct Bundled;
impl Endpoint for Bundled {
    const NAME: &'static str = "firmware.bundled";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/firmware/bundled";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = FirmwareBuild;
    type Error = FirmwareError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.firmware.builds()
    }
}
pub struct Pick;
impl Endpoint for Pick {
    const NAME: &'static str = "firmware.pick";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/firmware/bundles/pick";
    const UNTIMED: bool = true;
    type Path = ();
    type Query = NoQuery;
    type Body = FirmwareTarget;
    type Ok = PickResult;
    type Error = FirmwareError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.firmware.pick(req.body.device).await
    }
}
macro_rules! operation {
    ($name:ident, $kind:literal, $path:literal) => {
        impl Endpoint for $name {
            const NAME: &'static str = $kind;
            const METHOD: Method = Method::POST;
            const PATH: &'static str = $path;
            const STATUS: StatusCode = StatusCode::ACCEPTED;
            type Path = FirmwareTarget;
            type Query = NoQuery;
            type Body = FirmwareRequest;
            type Ok = Job;
            type Error = FirmwareError;
            async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Job, FirmwareError> {
                ctx.firmware
                    .start(Operation::$name, req.path.device, req.body)
            }
        }
    };
}
operation!(
    Update,
    "firmware.update",
    "/devices/{device}/firmware/update"
);
operation!(
    Repair,
    "firmware.repair",
    "/devices/{device}/firmware/repair"
);
operation!(Install, "firmware.install", "/new-boards/{device}/install");
fn feature() -> Feature {
    Feature::new("firmware")
        .endpoint::<Bundled>()
        .endpoint::<Pick>()
        .endpoint::<Update>()
        .endpoint::<Repair>()
        .endpoint::<Install>()
        .job::<Update>()
        .job::<Repair>()
        .job::<Install>()
}
inventory::submit! { Registration(feature) }
