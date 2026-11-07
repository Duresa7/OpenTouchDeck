pub mod asset;
pub mod bundle;
pub mod carrier;
pub mod discover;
pub mod live;
pub mod mock;
pub mod net;
pub mod photo;
pub mod recover;
pub mod session;
pub mod update;
pub mod wifi;
pub mod xfer;

use std::io::{self, Read, Write};
use std::time::Duration;

pub use asset::{AssetError, AssetFormat, AssetReason};
pub use carrier::{Carrier, FrameReader, FrameWriter, Outbound, Received, SendWindow, UsbCarrier};
pub use discover::{Candidate, Probe, discover, probe_transport};
pub use live::{LiveError, LiveReason, LiveSet, LiveValue};
pub use mock::MockDeck;
pub use session::{
    Incoming, IncomingReceiver, LinkError, Session, SessionConfig, Transfer, XferFrame,
};

pub const VID: u16 = 0x303A;
pub const PID: u16 = 0x1001;
const BAUD: u32 = 115_200;

/// How long a read waits before it reports `TimedOut`, so a reader can notice a close.
pub const READ_TICK: Duration = Duration::from_millis(20);

/// A byte stream to one deck, which a [`UsbCarrier`] frames. The read half returns `TimedOut` when no byte arrives within
/// [`READ_TICK`] and `Ok(0)` once the stream has ended.
pub trait Transport: Send {
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)>;
}

pub struct Serial {
    port: Box<dyn serialport::SerialPort>,
}

impl Serial {
    /// Opens the port with DTR and RTS low, so opening never resets the deck.
    pub fn open(path: &str) -> io::Result<Serial> {
        let port = serialport::new(path, BAUD)
            .timeout(READ_TICK)
            .dtr_on_open(false)
            .open()
            .map_err(io::Error::from)?;
        Ok(Serial { port })
    }
}

impl Transport for Serial {
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let reader = self.port.try_clone().map_err(io::Error::from)?;
        Ok((Box::new(reader), Box::new(self.port)))
    }
}

pub mod identity;
pub use identity::{AnimationCapabilities, Capabilities, Identity};
