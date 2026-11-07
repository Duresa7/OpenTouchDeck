//! Devices, New boards, ports and the read-only console.
use crate::{
    api::{ApiError, CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    devices::{
        DeviceAdded, DeviceConsole, DeviceEvent, DeviceId, DeviceKeyMirror, DeviceRemoved,
        DeviceSummary, DeviceUpdated, Forget as Forgotten, LinkState, NewBoard, NewBoardAdded,
        NewBoardRemoved, PortProbe, Reach, console,
    },
};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use td_core::{Request, Settings};
use td_link::{
    LinkError,
    photo::{self, IdleSettings, PhotoError},
    wifi::{self, WifiError, WifiPassword},
};
use ts_rs::TS;

/// How long a settings or name request may take on the link.
const SETTINGS_TIMEOUT: Duration = Duration::from_secs(2);
const BRIGHTNESS_MAX: u8 = 100;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("The Device does not exist.")]
    NotFound,
    #[error("The Device is not connected.")]
    NotConnected { link: LinkState },
    #[error("The Device is connected. Disconnect it first.")]
    Connected,
    #[error("The Device didn't reply in time.")]
    Timeout,
    #[error("The link to the Device failed: {reason}")]
    Link { reason: String },
    #[error(
        "The link closed before the Device answered, so it isn't known whether it did this. Check the Device, then try again if it didn't: {reason}"
    )]
    UnknownOutcome { reason: String },
    #[error("This needs newer Firmware on the Device.")]
    NeedsFirmware { field: &'static str },
    #[error("The Device takes this only over USB. Plug it in with a USB cable.")]
    UsbOnly,
    #[error("The console line isn't on the read-only allowlist.")]
    ConsoleForbidden,
    #[error(transparent)]
    Common(#[from] CommonError),
}
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
impl From<Reach> for DeviceError {
    fn from(reach: Reach) -> Self {
        match reach {
            Reach::NotFound => Self::NotFound,
            Reach::NotConnected(link) => Self::NotConnected { link },
        }
    }
}
impl From<LinkError> for DeviceError {
    fn from(error: LinkError) -> Self {
        match error {
            LinkError::Timeout(_) => Self::Timeout,
            LinkError::Unanswered(reason) => Self::UnknownOutcome { reason },
            error => Self::Link {
                reason: error.to_string(),
            },
        }
    }
}
impl From<PhotoError> for DeviceError {
    fn from(error: PhotoError) -> Self {
        match error {
            PhotoError::Link(error) => error.into(),
            PhotoError::Invalid(reason) => CommonError::invalid("body", reason).into(),
            error => Self::Link {
                reason: error.to_string(),
            },
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

/// The Device's settings. v2 fields are `null` on v1 Firmware.
#[derive(Debug, Serialize, Deserialize, TS)]
pub struct DeviceSettings {
    pub idle_timeout_s: u32,
    /// `landscape` or `portrait`.
    pub orientation: String,
    pub name: Option<String>,
    /// A percentage, 0 to 100.
    pub brightness: Option<u8>,
    /// `null` when the Firmware has no photo frame.
    pub photo_frame: Option<PhotoFrameSettings>,
}
/// What the Device shows after `timeout_s` with no touch: pictures from `folder` on this PC, a
/// new one every `interval_s`, or its clock when no picture comes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PhotoFrameSettings {
    pub enabled: bool,
    /// 10 to 86400.
    pub timeout_s: u32,
    /// 5 to 3600.
    pub interval_s: u32,
    /// An absolute path to a folder on this PC, `null` until one is chosen.
    pub folder: Option<String>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, TS)]
pub struct PhotoFramePatch {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub timeout_s: Option<u32>,
    #[serde(default)]
    pub interval_s: Option<u32>,
    /// An absolute path to an existing folder; `""` clears it.
    #[serde(default)]
    pub folder: Option<String>,
}
#[derive(Debug, Default, Serialize, Deserialize, TS)]
pub struct DeviceSettingsPatch {
    #[serde(default)]
    pub idle_timeout_s: Option<u32>,
    #[serde(default)]
    pub orientation: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// A percentage, 0 to 100.
    #[serde(default)]
    pub brightness: Option<u8>,
    #[serde(default)]
    pub photo_frame: Option<PhotoFramePatch>,
}
#[derive(Debug, Serialize, Deserialize, TS)]
pub struct SettingsPatched {
    pub settings: DeviceSettings,
    /// The Device must restart before an orientation change shows.
    pub restart_required: bool,
}
fn orientation_name(settings: &Settings) -> String {
    serde_json::to_value(settings.orientation)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
/// The v2 fields the Device's Firmware supports: its name, if it speaks protocol 2.
fn name_if_v2(summary: &DeviceSummary) -> Option<String> {
    let v2 = summary.firmware.as_ref().is_some_and(|f| f.protocol >= 2);
    v2.then(|| summary.name.clone()).flatten()
}
fn has_photo_frame(summary: &DeviceSummary) -> bool {
    crate::photos::supports(summary.capabilities.as_ref()).0
}
fn photo_frame(ctx: &Ctx, id: &DeviceId, idle: Option<IdleSettings>) -> Option<PhotoFrameSettings> {
    let idle = idle?;
    ctx.photos.idle_changed(id, idle);
    Some(PhotoFrameSettings {
        enabled: idle.enabled,
        timeout_s: idle.timeout_s,
        interval_s: idle.interval_s,
        folder: ctx.photos.folder(id),
    })
}
fn idle_patch(current: IdleSettings, patch: &PhotoFramePatch) -> IdleSettings {
    IdleSettings {
        enabled: patch.enabled.unwrap_or(current.enabled),
        timeout_s: patch.timeout_s.unwrap_or(current.timeout_s),
        interval_s: patch.interval_s.unwrap_or(current.interval_s),
    }
}
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
        let id = req.path.device;
        let summary = ctx.devices.get(&id).ok_or(DeviceError::NotFound)?;
        let photos = has_photo_frame(&summary);
        let (settings, idle) = ctx
            .devices
            .call(&id, move |s| -> Result<_, DeviceError> {
                let settings: Settings = s
                    .request(&Request::SettingsGet, SETTINGS_TIMEOUT)?
                    .fields()
                    .map_err(LinkError::Reply)?;
                let idle = photos.then(|| photo::idle_get(s)).transpose()?;
                Ok((settings, idle))
            })
            .await??;
        let summary = ctx.devices.get(&id).ok_or(DeviceError::NotFound)?;
        Ok(DeviceSettings {
            idle_timeout_s: settings.idle_timeout_s,
            orientation: orientation_name(&settings),
            name: name_if_v2(&summary),
            brightness: None,
            photo_frame: photo_frame(&ctx, &id, idle),
        })
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
        let id = req.path.device;
        let patch = req.body;
        let invalid = |reason| DeviceError::from(CommonError::invalid("body", reason));
        let orientation = match patch.orientation.as_deref() {
            None => None,
            Some("landscape") => Some(td_core::Orientation::Landscape),
            Some("portrait") => Some(td_core::Orientation::Portrait),
            Some(_) => return Err(invalid("orientation is landscape or portrait")),
        };
        if patch.idle_timeout_s.is_some_and(|t| {
            !(td_core::settings::IDLE_TIMEOUT_MIN_S..=td_core::settings::IDLE_TIMEOUT_MAX_S)
                .contains(&t)
        }) {
            return Err(invalid("idle_timeout_s is out of range"));
        }
        if patch
            .name
            .as_deref()
            .is_some_and(|n| !td_link::identity::valid_name(n))
        {
            return Err(invalid(
                "name is empty, too long or has control, format or filler characters",
            ));
        }
        if patch.brightness.is_some_and(|b| b > BRIGHTNESS_MAX) {
            return Err(invalid("brightness is a percentage from 0 to 100"));
        }
        // No Firmware reports brightness yet.
        if patch.brightness.is_some() {
            return Err(DeviceError::NeedsFirmware {
                field: "brightness",
            });
        }
        let frame = patch.photo_frame.clone();
        if let Some(frame) = &frame {
            if frame
                .timeout_s
                .is_some_and(|t| !photo::IDLE_TIMEOUT_S.contains(&t))
            {
                return Err(invalid("photo_frame.timeout_s is 10 to 86400"));
            }
            if frame
                .interval_s
                .is_some_and(|t| !photo::IDLE_INTERVAL_S.contains(&t))
            {
                return Err(invalid("photo_frame.interval_s is 5 to 3600"));
            }
            if frame
                .folder
                .as_deref()
                .is_some_and(|f| !f.is_empty() && !crate::photos::folder::valid(f))
            {
                return Err(invalid(
                    "photo_frame.folder is not the full path of a folder on this computer",
                ));
            }
        }
        let summary = ctx.devices.get(&id).ok_or(DeviceError::NotFound)?;
        if patch.name.is_some() && summary.firmware.as_ref().is_some_and(|f| f.protocol < 2) {
            return Err(DeviceError::NeedsFirmware { field: "name" });
        }
        let photos = has_photo_frame(&summary);
        if frame.is_some() && !photos {
            return Err(DeviceError::NeedsFirmware {
                field: "photo_frame",
            });
        }
        let devices = ctx.devices.clone();
        let device = id.clone();
        let idle_frame = frame.clone();
        let authority = ctx.authority.clone();
        #[cfg(any(test, feature = "test-hooks"))]
        let checkpoints = ctx.checkpoints.clone();
        let (settings, restart_required, idle) = ctx
            .devices
            .call(&id, move |s| -> Result<_, DeviceError> {
                let mut settings: Settings = s
                    .request(&Request::SettingsGet, SETTINGS_TIMEOUT)?
                    .fields()
                    .map_err(LinkError::Reply)?;
                let mut restart = false;
                if patch.idle_timeout_s.is_some() || orientation.is_some() {
                    settings.idle_timeout_s =
                        patch.idle_timeout_s.unwrap_or(settings.idle_timeout_s);
                    settings.orientation = orientation.unwrap_or(settings.orientation);
                    let mut reply = None;
                    let mut write = || {
                        #[cfg(any(test, feature = "test-hooks"))]
                        if orientation.is_some() {
                            let _ = checkpoints
                                .pass(crate::checkpoints::Checkpoint::BeforeOrientationPut);
                        }
                        reply = Some(s.request(&Request::SettingsPut(settings), SETTINGS_TIMEOUT));
                        Ok::<_, LinkError>(None)
                    };
                    if orientation.is_some() {
                        authority
                            .replace(device.as_str(), write)
                            .map_err(|error| match error {
                                crate::layouts::authority::ReplaceError::Write(error) => {
                                    DeviceError::from(error)
                                }
                                crate::layouts::authority::ReplaceError::Save(_) => {
                                    DeviceError::from(CommonError::Internal)
                                }
                            })?;
                    } else {
                        write()?;
                    }
                    let reply = reply.unwrap()?;
                    let fields = serde_json::Value::Object(reply.fields.clone());
                    restart = fields["reboot_required"].as_bool().unwrap_or(false);
                    settings = Settings::from_value(&fields);
                    #[cfg(any(test, feature = "test-hooks"))]
                    if orientation.is_some() {
                        checkpoints
                            .pass(crate::checkpoints::Checkpoint::AfterOrientationPut)
                            .map_err(|()| LinkError::Timeout(SETTINGS_TIMEOUT))?;
                    }
                }
                if let Some(name) = patch.name {
                    let name = s.set_name(&name, SETTINGS_TIMEOUT)?;
                    devices.named(&device, name);
                }
                let idle = match (photos, idle_frame) {
                    (false, _) => None,
                    (true, None) => Some(photo::idle_get(s)?),
                    (true, Some(frame)) => {
                        let idle = idle_patch(photo::idle_get(s)?, &frame);
                        Some(photo::idle_set(s, idle)?)
                    }
                };
                Ok((settings, restart, idle))
            })
            .await??;
        if let Some(folder) = frame.and_then(|f| f.folder) {
            let photos = ctx.photos.clone();
            let device = id.clone();
            tokio::task::spawn_blocking(move || {
                photos.set_folder(&device, (!folder.is_empty()).then_some(folder))
            })
            .await
            .map_err(|_| CommonError::Internal)?
            .map_err(|_| CommonError::Internal)?;
        }
        let summary = ctx.devices.get(&id).ok_or(DeviceError::NotFound)?;
        Ok(SettingsPatched {
            settings: DeviceSettings {
                idle_timeout_s: settings.idle_timeout_s,
                orientation: orientation_name(&settings),
                name: name_if_v2(&summary),
                brightness: None,
                photo_frame: photo_frame(&ctx, &id, idle),
            },
            restart_required,
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
