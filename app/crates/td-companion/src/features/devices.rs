//! Devices, New boards, ports and the read-only console.
use crate::{
    api::{ApiError, CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    devices::{
        DeviceAdded, DeviceConsole, DeviceEvent, DeviceId, DeviceKeyMirror, DeviceRemoved,
        DeviceSummary, DeviceUpdated, Forget as Forgotten, NewBoard, NewBoardAdded,
        NewBoardRemoved, PortProbe, console,
    },
};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use td_link::wifi::{self, WifiError, WifiPassword};
use ts_rs::TS;

pub use crate::devices::DeviceError;
impl ApiError for DeviceError {
    const CODES: &'static [&'static str] = &[
        "device.not_found",
        "device.not_connected",
        "device.connected",
        "device.timeout",
        "device.link",
        "device.unknown_outcome",
        "device.needs_firmware",
        "device.usb_only",
        "console.forbidden",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::NotConnected { .. }
            | Self::Connected
            | Self::NeedsFirmware { .. }
            | Self::UsbOnly => StatusCode::CONFLICT,
            Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Link { .. } | Self::UnknownOutcome { .. } => StatusCode::BAD_GATEWAY,
            Self::ConsoleForbidden => StatusCode::FORBIDDEN,
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "device.not_found",
            Self::NotConnected { .. } => "device.not_connected",
            Self::Connected => "device.connected",
            Self::Timeout => "device.timeout",
            Self::Link { .. } => "device.link",
            Self::UnknownOutcome { .. } => "device.unknown_outcome",
            Self::NeedsFirmware { .. } => "device.needs_firmware",
            Self::UsbOnly => "device.usb_only",
            Self::ConsoleForbidden => "console.forbidden",
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::NotConnected { link } => Some(serde_json::json!({ "link": link })),
            Self::Link { reason } | Self::UnknownOutcome { reason } => {
                Some(serde_json::json!({ "reason": reason }))
            }
            Self::NeedsFirmware { field } => {
                Some(serde_json::json!({ "field": field, "version": null }))
            }
            Self::Common(e) => e.details(),
            _ => None,
        }
    }
}
#[derive(Serialize, Deserialize, TS)]
pub struct DevicePath {
    pub device: DeviceId,
}
pub struct List;
impl Endpoint for List {
    const NAME: &'static str = "devices.list";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<DeviceSummary>;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(ctx.devices.summaries())
    }
}
pub struct Get;
impl Endpoint for Get {
    const NAME: &'static str = "devices.get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices/{device}";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceSummary;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.devices
            .get(&req.path.device)
            .ok_or(DeviceError::NotFound)
    }
}
pub struct Connect;
impl Endpoint for Connect {
    const NAME: &'static str = "devices.connect";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/devices/{device}/connect";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceSummary;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.devices
            .connect(&req.path.device)
            .ok_or(DeviceError::NotFound)
    }
}
pub struct Disconnect;
impl Endpoint for Disconnect {
    const NAME: &'static str = "devices.disconnect";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/devices/{device}/disconnect";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceSummary;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.devices
            .disconnect(&req.path.device)
            .ok_or(DeviceError::NotFound)
    }
}
pub struct Forget;
impl Endpoint for Forget {
    const NAME: &'static str = "devices.forget";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/devices/{device}";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let devices = ctx.devices.clone();
        let forgotten = tokio::task::spawn_blocking(move || devices.forget(&req.path.device))
            .await
            .map_err(|_| CommonError::Internal)?;
        forgotten.map_err(|e| match e {
            Forgotten::NotFound => DeviceError::NotFound,
            Forgotten::Connected => DeviceError::Connected,
        })
    }
}
pub struct Restart;
impl Endpoint for Restart {
    const NAME: &'static str = "devices.restart";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/devices/{device}/restart";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(ctx.devices.restart(&req.path.device).await??)
    }
}

pub use crate::devices::{
    DeviceSettings, DeviceSettingsPatch, PhotoFramePatch, PhotoFrameSettings, SettingsPatched,
};
pub struct SettingsGet;
impl Endpoint for SettingsGet {
    const NAME: &'static str = "devices.settings_get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices/{device}/settings";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceSettings;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.device_operations().settings_get(&req.path.device).await
    }
}
pub struct SettingsPatch;
impl Endpoint for SettingsPatch {
    const NAME: &'static str = "devices.settings_patch";
    const METHOD: Method = Method::PATCH;
    const PATH: &'static str = "/devices/{device}/settings";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = DeviceSettingsPatch;
    type Ok = SettingsPatched;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let outcome = ctx
            .device_operations()
            .settings_patch(&req.path.device, req.body)
            .await?;
        if let Some(error) = outcome.error {
            return Err(error);
        }
        Ok(SettingsPatched {
            settings: outcome.settings,
            restart_required: outcome.restart_required,
        })
    }
}
/// How the Device's Wi-Fi station is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum WifiConnection {
    /// No network is stored.
    Off,
    Connecting,
    Connected,
    /// The last join failed or the connection dropped. The Device keeps retrying.
    Failed,
}
/// The Device's Wi-Fi network. Never its password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DeviceWifi {
    pub configured: bool,
    /// The stored network's name, `""` when none is stored.
    pub ssid: String,
    pub state: WifiConnection,
    /// Why the station failed: `not-found`, `wrong-password`, `no-ip`, `security`, `refused`, `lost`, `driver` or
    /// `other`. `null` unless `state` is `failed`.
    pub reason: Option<String>,
    /// The Device's IPv4 address while connected.
    pub ip: Option<String>,
    /// The signal in dBm while connected.
    pub rssi: Option<i32>,
}
impl From<wifi::WifiState> for DeviceWifi {
    fn from(state: wifi::WifiState) -> Self {
        Self {
            configured: state.configured,
            ssid: state.ssid,
            state: match state.state {
                wifi::WifiStatus::Off => WifiConnection::Off,
                wifi::WifiStatus::Connecting => WifiConnection::Connecting,
                wifi::WifiStatus::Connected => WifiConnection::Connected,
                wifi::WifiStatus::Failed => WifiConnection::Failed,
            },
            reason: state.reason,
            ip: state.ip,
            rssi: state.rssi,
        }
    }
}
/// A Wi-Fi password on its way to the Device. It is never printed, logged or sent back. A newtype struct, so serde
/// reads and writes it as the bare string.
#[derive(Clone, Default, Deserialize, Serialize, TS)]
#[ts(type = "string")]
pub struct WifiSecret(String);
impl WifiSecret {
    pub fn new(password: impl Into<String>) -> Self {
        Self(password.into())
    }
}
impl std::fmt::Debug for WifiSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WifiSecret(redacted)")
    }
}
/// The network to join: an SSID of 1 to 32 bytes and a password that is empty for an open network, 8 to 63
/// printable ASCII characters, or 64 hex digits.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WifiSetBody {
    pub ssid: String,
    pub password: WifiSecret,
}
impl From<WifiError> for DeviceError {
    fn from(error: WifiError) -> Self {
        match error {
            WifiError::Link(error) => error.into(),
            WifiError::Invalid(reason) => CommonError::invalid("body", reason).into(),
            // A Device without the station has no `wifi.*` messages and answers the unknown-message error.
            WifiError::Refused { reason, error }
                if reason == "unsupported" || (reason.is_empty() && error == "unknown message") =>
            {
                Self::NeedsFirmware { field: "wifi" }
            }
            WifiError::Refused { reason, .. } if reason == "usb-only" => Self::UsbOnly,
            WifiError::Refused { reason, error } if reason == "args" => {
                CommonError::invalid("body", error).into()
            }
            error => Self::Link {
                reason: error.to_string(),
            },
        }
    }
}
/// Runs one Wi-Fi setup request on a Device whose Firmware has a Wi-Fi station (the radio build). The Device takes
/// its network only over USB, so one connected over Wi-Fi hears nothing.
async fn on_wifi_device(
    ctx: &Ctx,
    id: &DeviceId,
    work: impl FnOnce(&td_link::Session) -> Result<wifi::WifiState, WifiError> + Send + 'static,
) -> Result<DeviceWifi, DeviceError> {
    let summary = ctx.devices.get(id).ok_or(DeviceError::NotFound)?;
    if summary.capabilities.as_ref().and_then(|c| c.wifi) != Some(true) {
        return Err(DeviceError::NeedsFirmware { field: "wifi" });
    }
    if summary.carrier == Some(crate::devices::Carrier::Wifi) {
        return Err(DeviceError::UsbOnly);
    }
    let state = ctx.devices.call(id, work).await??;
    Ok(state.into())
}
pub struct WifiGet;
impl Endpoint for WifiGet {
    const NAME: &'static str = "devices.wifi_get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices/{device}/wifi";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceWifi;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        on_wifi_device(&ctx, &req.path.device, wifi::wifi_state).await
    }
}
pub struct WifiSet;
impl Endpoint for WifiSet {
    const NAME: &'static str = "devices.wifi_set";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/devices/{device}/wifi";
    const BODY_LIMIT: usize = 4096;
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = WifiSetBody;
    type Ok = DeviceWifi;
    type Error = DeviceError;
    /// A body that doesn't parse gets one fixed reason, so no part of it, the password included, comes back.
    async fn read_body(
        ctx: &Ctx,
        headers: &axum::http::HeaderMap,
        _client: Option<&crate::api::ClientInfo>,
        body: axum::body::Body,
    ) -> Result<WifiSetBody, CommonError> {
        let bytes = tokio::time::timeout(
            ctx.limits.body_read,
            axum::body::to_bytes(body, Self::BODY_LIMIT),
        )
        .await
        .map_err(|_| CommonError::Timeout)?
        .map_err(|_| CommonError::TooLarge)?;
        if !crate::api::is_json(headers) {
            return Err(CommonError::MediaType);
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            CommonError::invalid(
                "body",
                "the body is {\"ssid\": string, \"password\": string}",
            )
        })
    }
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let WifiSetBody { ssid, password } = req.body;
        let password = WifiPassword::new(password.0);
        if !wifi::valid_ssid(&ssid) {
            return Err(CommonError::invalid(
                "body",
                "ssid is 1 to 32 bytes without control characters",
            )
            .into());
        }
        if !wifi::valid_password(&password) {
            return Err(CommonError::invalid(
                "body",
                "password is empty for an open network, 8 to 63 printable ASCII characters, or 64 hex digits",
            )
            .into());
        }
        on_wifi_device(&ctx, &req.path.device, move |s| {
            wifi::wifi_set(s, &ssid, &password)
        })
        .await
    }
}
pub struct WifiForget;
impl Endpoint for WifiForget {
    const NAME: &'static str = "devices.wifi_forget";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/devices/{device}/wifi";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceWifi;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        on_wifi_device(&ctx, &req.path.device, wifi::wifi_forget).await
    }
}
fn devices() -> Feature {
    Feature::new("devices")
        .endpoint::<List>()
        .endpoint::<Get>()
        .endpoint::<Connect>()
        .endpoint::<Disconnect>()
        .endpoint::<Forget>()
        .endpoint::<Restart>()
        .endpoint::<SettingsGet>()
        .endpoint::<SettingsPatch>()
        .endpoint::<WifiGet>()
        .endpoint::<WifiSet>()
        .endpoint::<WifiForget>()
        .event::<DeviceAdded>()
        .event::<DeviceUpdated>()
        .event::<DeviceRemoved>()
        .event::<DeviceKeyMirror>()
        .event::<DeviceEvent>()
        .snapshot("devices", |ctx| ctx.devices.summaries())
}
inventory::submit! { Registration(devices) }

pub struct NewBoardsList;
impl Endpoint for NewBoardsList {
    const NAME: &'static str = "new_boards.list";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/new-boards";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<NewBoard>;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(ctx.devices.new_boards())
    }
}
fn new_boards() -> Feature {
    Feature::new("new_boards")
        .endpoint::<NewBoardsList>()
        .event::<NewBoardAdded>()
        .event::<NewBoardRemoved>()
        .snapshot("new_boards", |ctx| ctx.devices.new_boards())
}
inventory::submit! { Registration(new_boards) }

pub struct PortsList;
impl Endpoint for PortsList {
    const NAME: &'static str = "ports.list";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/ports";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<PortProbe>;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(ctx.devices.ports())
    }
}
pub struct PortsRescan;
impl Endpoint for PortsRescan {
    const NAME: &'static str = "ports.rescan";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/ports/rescan";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.devices.rescan();
        Ok(())
    }
}
fn ports() -> Feature {
    Feature::new("ports")
        .endpoint::<PortsList>()
        .endpoint::<PortsRescan>()
}
inventory::submit! { Registration(ports) }

#[derive(Serialize, Deserialize, TS)]
pub struct ConsoleLine {
    pub line: String,
}
pub struct ConsoleSend;
impl Endpoint for ConsoleSend {
    const NAME: &'static str = "console.send";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/devices/{device}/console";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ConsoleLine;
    type Ok = ();
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        if !console::allowed(&req.body.line) {
            return Err(DeviceError::ConsoleForbidden);
        }
        let line = req.body.line;
        Ok(ctx
            .devices
            .call(&req.path.device, move |s| s.send_line(&line))
            .await??)
    }
}
fn console_feature() -> Feature {
    Feature::new("console")
        .endpoint::<ConsoleSend>()
        .event::<DeviceConsole>()
}
inventory::submit! { Registration(console_feature) }
