//! Where ports come from: the PC's serial ports, or mock ports for tests.
use std::{io, sync::Arc};
use td_link::Transport;

/// An Espressif USB port (`303a:1001`) that's plugged in now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub port: String,
    pub usb_serial: Option<String>,
}
/// Called when USB ports may have changed.
pub type Changed = Arc<dyn Fn() + Send + Sync>;
pub trait PortSource: Send + Sync + 'static {
    /// The Espressif ports plugged in now. Listing never opens a port.
    fn list(&self) -> Vec<PortInfo>;
    /// Opens a port for a probe that may become the Device's session.
    fn open(&self, port: &str) -> io::Result<Box<dyn Transport>>;
    /// Calls `changed` on USB changes. The scan loop also polls every second, so a source
    /// that can't watch loses nothing but speed.
    fn watch(&self, _changed: Changed) {}
}

/// The PC's serial ports.
#[derive(Default)]
pub struct RealPorts {
    #[cfg(windows)]
    watch: parking_lot::Mutex<Option<windows::Watch>>,
}
impl PortSource for RealPorts {
    fn list(&self) -> Vec<PortInfo> {
        let mut ports: Vec<PortInfo> = serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| match p.port_type {
                serialport::SerialPortType::UsbPort(u)
                    if u.vid == td_link::VID && u.pid == td_link::PID =>
                {
                    Some(PortInfo {
                        port: p.port_name,
                        usb_serial: u.serial_number,
                    })
                }
                _ => None,
            })
            .collect();
        ports.sort_by(|a, b| a.port.cmp(&b.port));
        ports.dedup_by(|a, b| a.port == b.port);
        ports
    }
    fn open(&self, port: &str) -> io::Result<Box<dyn Transport>> {
        Ok(Box::new(td_link::Serial::open(port)?))
    }
    fn watch(&self, changed: Changed) {
        #[cfg(windows)]
        match windows::Watch::start(changed) {
            Ok(watch) => *self.watch.lock() = Some(watch),
            Err(code) => eprintln!(
                "Companion: USB change notifications are off (CONFIGRET {code}); polling every second."
            ),
        }
        #[cfg(not(windows))]
        let _ = changed;
    }
}

#[cfg(windows)]
mod windows {
    use super::Changed;
    use std::ffi::c_void;
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_NOTIFY_ACTION, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
        CM_NOTIFY_FILTER_FLAG_ALL_INTERFACE_CLASSES, CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
        CM_Register_Notification, CM_Unregister_Notification, CR_SUCCESS, HCMNOTIFICATION,
    };

    /// A `CM_Register_Notification` registration for every device interface arriving or
    /// leaving. A COM port is one of them; the scan that follows sorts out which changed.
    pub(super) struct Watch {
        handle: HCMNOTIFICATION,
        context: *mut Changed,
    }
    // SAFETY: the handle and context are only used to unregister and free, once, on drop.
    unsafe impl Send for Watch {}
    unsafe impl Sync for Watch {}
    impl Watch {
        pub(super) fn start(changed: Changed) -> Result<Self, u32> {
            let context = Box::into_raw(Box::new(changed));
            let filter = CM_NOTIFY_FILTER {
                cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
                Flags: CM_NOTIFY_FILTER_FLAG_ALL_INTERFACE_CLASSES,
                FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
                ..Default::default()
            };
            let mut handle: HCMNOTIFICATION = std::ptr::null_mut();
            // SAFETY: the filter is initialized, and `context` stays valid until `drop`
            // unregisters, which waits for running callbacks to finish.
            let code = unsafe {
                CM_Register_Notification(
                    &filter,
                    context as *const c_void,
                    Some(notify),
                    &mut handle,
                )
            };
            if code != CR_SUCCESS {
                // SAFETY: registration failed, so no callback holds the context.
                drop(unsafe { Box::from_raw(context) });
                return Err(code);
            }
            Ok(Self { handle, context })
        }
    }
    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: unregistering waits for callbacks in flight, so the context is free.
            unsafe {
                CM_Unregister_Notification(self.handle);
                drop(Box::from_raw(self.context));
            }
        }
    }
    unsafe extern "system" fn notify(
        _handle: HCMNOTIFICATION,
        context: *const c_void,
        _action: CM_NOTIFY_ACTION,
        _data: *const CM_NOTIFY_EVENT_DATA,
        _size: u32,
    ) -> u32 {
        // SAFETY: `context` is the `Changed` that `Watch` keeps alive while registered.
        let changed = unsafe { &*(context as *const Changed) };
        changed();
        0
    }
}

/// What a mock port holds.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
pub enum BoardKind {
    /// A Device with v2 Firmware.
    Device,
    /// A Device with v1 Firmware: no id, name or capabilities in `hello`.
    DeviceV1,
    /// A board that never answers.
    Silent,
    /// A board that answers `hello` as something else.
    OtherFirmware,
    /// A port another program holds.
    Busy,
}

/// Mock ports, each holding a mock Device or board under a USB serial.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default, Clone)]
pub struct MockPorts {
    state: Arc<parking_lot::Mutex<MockState>>,
}
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default)]
struct MockState {
    /// Every board ever plugged. A board keeps its state across unplug and replug.
    boards: std::collections::BTreeMap<String, (BoardKind, td_link::MockDeck)>,
    plugged: std::collections::BTreeSet<String>,
    changed: Option<Changed>,
}
#[cfg(any(test, feature = "test-hooks"))]
impl MockPorts {
    fn port_name(serial: &str) -> String {
        format!("mock:{serial}")
    }
    /// Plugs a board with this USB serial in.
    pub fn plug(&self, serial: &str, kind: BoardKind) {
        self.plug_board(serial, kind, false);
    }
    /// Plugs in a Device with v2 Firmware that advertises media capabilities from the moment its port is
    /// published, so a Companion that connects on the change notice never sees it without them.
    pub fn plug_media_device(&self, serial: &str) {
        self.plug_board(serial, BoardKind::Device, true);
    }
    /// The board under this USB serial, made if it doesn't exist, without plugging it in. A
    /// fake Wi-Fi Device wraps the same mock, so plugging it in later is the same Device.
    pub fn board(&self, serial: &str, kind: BoardKind) -> td_link::MockDeck {
        let mut state = self.state.lock();
        Self::make(&mut state, serial, kind);
        state.boards[serial].1.clone()
    }
    fn make(state: &mut MockState, serial: &str, kind: BoardKind) {
        if state.boards.get(serial).is_none_or(|(k, _)| *k != kind) {
            let deck = match kind {
                BoardKind::Device => td_link::MockDeck::v2(),
                _ => td_link::MockDeck::new(),
            };
            match kind {
                BoardKind::Device => deck.set_name(&format!("Mock Device {serial}")),
                BoardKind::Silent => deck.stop(),
                BoardKind::OtherFirmware => deck.set_product("esp-at"),
                BoardKind::DeviceV1 | BoardKind::Busy => {}
            }
            state.boards.insert(serial.into(), (kind, deck));
        }
    }
    fn plug_board(&self, serial: &str, kind: BoardKind, media: bool) {
        let changed = {
            let mut state = self.state.lock();
            Self::make(&mut state, serial, kind);
            if media {
                state.boards[serial].1.set_media_capabilities(true);
            }
            state.plugged.insert(serial.into());
            state.changed.clone()
        };
        if let Some(changed) = changed {
            changed();
        }
    }
    /// Unplugs it: its stream ends and its port leaves the list.
    pub fn unplug(&self, serial: &str) {
        let changed = {
            let mut state = self.state.lock();
            if state.plugged.remove(serial)
                && let Some((_, deck)) = state.boards.get(serial)
            {
                deck.hang_up();
            }
            state.changed.clone()
        };
        if let Some(changed) = changed {
            changed();
        }
    }
    /// The mock behind a USB serial, plugged or not.
    pub fn deck(&self, serial: &str) -> Option<td_link::MockDeck> {
        let state = self.state.lock();
        state.boards.get(serial).map(|(_, deck)| deck.clone())
    }
}
#[cfg(any(test, feature = "test-hooks"))]
impl PortSource for MockPorts {
    fn list(&self) -> Vec<PortInfo> {
        let state = self.state.lock();
        state
            .plugged
            .iter()
            .map(|serial| PortInfo {
                port: Self::port_name(serial),
                usb_serial: Some(serial.clone()),
            })
            .collect()
    }
    fn open(&self, port: &str) -> io::Result<Box<dyn Transport>> {
        let state = self.state.lock();
        let serial = state
            .plugged
            .iter()
            .find(|s| Self::port_name(s) == port)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        match &state.boards[serial] {
            (BoardKind::Busy, _) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Access is denied.",
            )),
            (_, deck) => Ok(Box::new(deck.try_connect()?)),
        }
    }
    fn watch(&self, changed: Changed) {
        self.state.lock().changed = Some(changed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use td_link::{Session, SessionConfig};

    #[test]
    fn media_capabilities_are_ready_when_the_port_is_published() {
        let ports = MockPorts::default();
        let observed = ports.clone();
        ports.watch(Arc::new(move || {
            let port = observed.list().pop().unwrap();
            let (session, _) = Session::open(
                observed.open(&port.port).unwrap(),
                SessionConfig {
                    ping_every: None,
                    ..SessionConfig::default()
                },
            )
            .unwrap();
            let identity = session.identity(Duration::from_secs(1)).unwrap();
            assert_eq!(identity.capabilities.unwrap().live_images, Some(4));
        }));
        ports.plug_media_device("mock0001");
        ports.watch(Arc::new(|| {}));
    }
}
