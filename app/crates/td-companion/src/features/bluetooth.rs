//! The radio build's Bluetooth keyboard, under each Device: on and off, the pairing window, and the computers and
//! phones it keeps bonded. Every route works only over USB, from link-v2's `ble.*` messages.
use crate::{
    api::{ApiError, CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    devices::{Carrier, DeviceId, Reach},
    features::devices::{DeviceError, DevicePath},
};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use td_link::ble::{self, BleError, BleState};
use ts_rs::TS;

/// Whether the Device's Bluetooth controller and host run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum BluetoothStack {
    Off,
    /// Coming up. Read again in a moment.
    Starting,
    On,
    /// It did not start. The setting stays on.
    Failed,
}
/// What the Device advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum BluetoothAdvertising {
    Off,
    /// The pairing window is open.
    Pairing,
    /// For bonded hosts, every 20-30 ms.
    Fast,
    /// For bonded hosts, every 1-2.5 s.
    Slow,
}
/// A computer or phone the Device keeps bonded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BluetoothBond {
    /// The host's identity address, `aa:bb:cc:dd:ee:ff` in lower case.
    pub address: String,
    /// A random static identity address rather than a public one.
    pub random: bool,
    /// Connected, encrypted with authenticated keys, and bonded.
    pub connected: bool,
}
/// The Device's Bluetooth keyboard. Never the pairing passkey, which stays on the Device's screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DeviceBluetooth {
    /// The stored setting. Off on a fresh Device.
    pub enabled: bool,
    pub state: BluetoothStack,
    /// Whether the pairing window is open.
    pub pairing: bool,
    /// Milliseconds left in the pairing window, 0 when it is closed.
    pub pairing_ms: u32,
    /// Whether the Device shows a six-digit passkey for a host to type.
    pub passkey: bool,
    pub advertising: BluetoothAdvertising,
    /// The computers and phones the Device keeps. Accepted in any state: the Firmware lists none while `off`, and
    /// still lists them while `starting` after a host reset.
    pub bonds: Vec<BluetoothBond>,
    /// The most bonds the Device keeps.
    pub bonds_max: u32,
    /// The battery level the Device reports to hosts, 0 to 100.
    pub battery: u8,
}
impl From<BleState> for DeviceBluetooth {
    fn from(state: BleState) -> Self {
        Self {
            enabled: state.enabled,
            state: match state.state {
                ble::BleStack::Off => BluetoothStack::Off,
                ble::BleStack::Starting => BluetoothStack::Starting,
                ble::BleStack::On => BluetoothStack::On,
                ble::BleStack::Failed => BluetoothStack::Failed,
            },
            pairing: state.pairing,
            pairing_ms: state.pairing_ms,
            passkey: state.passkey,
            advertising: match state.advertising {
                ble::BleAdvertising::Off => BluetoothAdvertising::Off,
                ble::BleAdvertising::Pairing => BluetoothAdvertising::Pairing,
                ble::BleAdvertising::Fast => BluetoothAdvertising::Fast,
                ble::BleAdvertising::Slow => BluetoothAdvertising::Slow,
            },
            bonds: state
                .bonds
                .into_iter()
                .map(|bond| BluetoothBond {
                    address: bond.address,
                    random: bond.random,
                    connected: bond.connected,
                })
                .collect(),
            bonds_max: state.bonds_max,
            battery: state.battery,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BluetoothSetBody {
    pub enabled: bool,
}
/// What forgetting answers: how many bonds the Device deleted, and the keyboard after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BluetoothForgotten {
    pub forgotten: u32,
    pub bluetooth: DeviceBluetooth,
}
#[derive(Serialize, Deserialize, TS)]
pub struct BondPath {
    pub device: DeviceId,
    // Six hex bytes separated by colons, in either case. Not a doc comment: the route check counts the colons in the
    // generated path type.
    pub address: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BluetoothError {
    #[error("Bluetooth is off on the Device. Turn it on first.")]
    Off { state: Box<DeviceBluetooth> },
    #[error("Bluetooth is still starting on the Device. Try again in a moment.")]
    Starting { state: Box<DeviceBluetooth> },
    #[error("The Device keeps as many computers and phones as it can. Forget one first.")]
    Full { state: Box<DeviceBluetooth> },
    #[error("Bluetooth didn't start on the Device. The setting stays on.")]
    Radio { state: Option<Box<DeviceBluetooth>> },
    #[error("The Device couldn't save the change.")]
    Storage { state: Option<Box<DeviceBluetooth>> },
    #[error(transparent)]
    Device(#[from] DeviceError),
}
impl ApiError for BluetoothError {
    const CODES: &'static [&'static str] = &[
        "bluetooth.off",
        "bluetooth.starting",
        "bluetooth.full",
        "bluetooth.radio",
        "bluetooth.storage",
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
            Self::Off { .. } | Self::Starting { .. } | Self::Full { .. } => StatusCode::CONFLICT,
            Self::Radio { .. } | Self::Storage { .. } => StatusCode::BAD_GATEWAY,
            Self::Device(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::Off { .. } => "bluetooth.off",
            Self::Starting { .. } => "bluetooth.starting",
            Self::Full { .. } => "bluetooth.full",
            Self::Radio { .. } => "bluetooth.radio",
            Self::Storage { .. } => "bluetooth.storage",
            Self::Device(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        let state = match self {
            Self::Off { state } | Self::Starting { state } | Self::Full { state } => Some(state),
            Self::Radio { state } | Self::Storage { state } => state.as_ref(),
            Self::Device(e) => return e.details(),
        };
        state.map(|state| serde_json::json!({ "bluetooth": state }))
    }
}
impl From<CommonError> for BluetoothError {
    fn from(error: CommonError) -> Self {
        Self::Device(error.into())
    }
}
impl From<Reach> for BluetoothError {
    fn from(reach: Reach) -> Self {
        Self::Device(reach.into())
    }
}
impl From<BleError> for BluetoothError {
    fn from(error: BleError) -> Self {
        match error {
            BleError::Link(error) => DeviceError::from(error).into(),
            BleError::Invalid(reason) => CommonError::invalid("path", reason).into(),
            BleError::Refused {
                reason,
                error,
                state,
            } => {
                let state = state.map(|state| Box::new(DeviceBluetooth::from(*state)));
                match (reason.as_str(), state) {
                    ("off", Some(state)) => Self::Off { state },
                    ("starting", Some(state)) => Self::Starting { state },
                    ("full", Some(state)) => Self::Full { state },
                    ("radio", state) => Self::Radio { state },
                    ("storage", state) => Self::Storage { state },
                    ("usb-only", _) => DeviceError::UsbOnly.into(),
                    // A Device without `ble.*` answers the unknown-message error, with no reason.
                    ("unsupported" | "", _) => {
                        DeviceError::NeedsFirmware { field: "bluetooth" }.into()
                    }
                    _ => DeviceError::Link {
                        reason: format!("the Device refused: {error} ({reason})"),
                    }
                    .into(),
                }
            }
        }
    }
}

/// Runs one `ble.*` request on a Device whose Firmware has the Bluetooth keyboard. The Device takes these only over
/// USB, so one connected over Wi-Fi hears nothing.
async fn on_keyboard<T: Send + 'static>(
    ctx: &Ctx,
    id: &DeviceId,
    work: impl FnOnce(&td_link::Session) -> Result<T, BleError> + Send + 'static,
) -> Result<T, BluetoothError> {
    let summary = ctx.devices.get(id).ok_or(DeviceError::NotFound)?;
    if summary
        .capabilities
        .as_ref()
        .and_then(|c| c.ble_hid)
        .is_none()
    {
        return Err(DeviceError::NeedsFirmware { field: "bluetooth" }.into());
    }
    if summary.carrier == Some(Carrier::Wifi) {
        return Err(DeviceError::UsbOnly.into());
    }
    Ok(ctx.devices.call(id, work).await??)
}

pub struct BluetoothGet;
impl Endpoint for BluetoothGet {
    const NAME: &'static str = "devices.bluetooth_get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices/{device}/bluetooth";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceBluetooth;
    type Error = BluetoothError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(on_keyboard(&ctx, &req.path.device, ble::ble_state)
            .await?
            .into())
    }
}
pub struct BluetoothSet;
impl Endpoint for BluetoothSet {
    const NAME: &'static str = "devices.bluetooth_set";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/devices/{device}/bluetooth";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = BluetoothSetBody;
    type Ok = DeviceBluetooth;
    type Error = BluetoothError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let enabled = req.body.enabled;
        Ok(
            on_keyboard(&ctx, &req.path.device, move |s| ble::ble_set(s, enabled))
                .await?
                .into(),
        )
    }
}
pub struct BluetoothPairingOpen;
impl Endpoint for BluetoothPairingOpen {
    const NAME: &'static str = "devices.bluetooth_pairing_open";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/devices/{device}/bluetooth/pairing";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceBluetooth;
    type Error = BluetoothError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(
            on_keyboard(&ctx, &req.path.device, |s| ble::ble_pair(s, true))
                .await?
                .into(),
        )
    }
}
pub struct BluetoothPairingClose;
impl Endpoint for BluetoothPairingClose {
    const NAME: &'static str = "devices.bluetooth_pairing_close";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/devices/{device}/bluetooth/pairing";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceBluetooth;
    type Error = BluetoothError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(
            on_keyboard(&ctx, &req.path.device, |s| ble::ble_pair(s, false))
                .await?
                .into(),
        )
    }
}
fn forgotten(forgotten: ble::BleForgotten) -> BluetoothForgotten {
    BluetoothForgotten {
        forgotten: forgotten.forgotten,
        bluetooth: forgotten.state.into(),
    }
}
pub struct BluetoothForget;
impl Endpoint for BluetoothForget {
    const NAME: &'static str = "devices.bluetooth_forget";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/devices/{device}/bluetooth/bonds/{address}";
    type Path = BondPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = BluetoothForgotten;
    type Error = BluetoothError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let address = req.path.address.to_ascii_lowercase();
        if !ble::valid_address(&address) {
            return Err(CommonError::invalid(
                "path",
                "address is six hex bytes, aa:bb:cc:dd:ee:ff",
            )
            .into());
        }
        let done = on_keyboard(&ctx, &req.path.device, move |s| {
            ble::ble_forget(s, Some(&address))
        })
        .await?;
        Ok(forgotten(done))
    }
}
pub struct BluetoothForgetAll;
impl Endpoint for BluetoothForgetAll {
    const NAME: &'static str = "devices.bluetooth_forget_all";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/devices/{device}/bluetooth/bonds";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = BluetoothForgotten;
    type Error = BluetoothError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let done = on_keyboard(&ctx, &req.path.device, |s| ble::ble_forget(s, None)).await?;
        Ok(forgotten(done))
    }
}
/// Registered under `devices`, so the routes and the generated client sit with the Device's other settings.
fn bluetooth() -> Feature {
    Feature::new("devices")
        .endpoint::<BluetoothGet>()
        .endpoint::<BluetoothSet>()
        .endpoint::<BluetoothPairingOpen>()
        .endpoint::<BluetoothPairingClose>()
        .endpoint::<BluetoothForget>()
        .endpoint::<BluetoothForgetAll>()
}
inventory::submit! { Registration(bluetooth) }
