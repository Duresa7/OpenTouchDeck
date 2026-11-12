//! Errors shared by in-process Device operations and their adapters.
use super::{LinkState, Reach};
use crate::api::CommonError;
use td_link::{LinkError, photo::PhotoError};
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
