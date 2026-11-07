//! Link sessions for Layout and Asset work, one user per Device at a time.
//!
//! The work runs on the session the Device manager keeps for the Device, since the port can
//! only be open once. The actor's own requests (settings, console) can interleave with it: a
//! session matches replies to requests by sequence number.
use crate::{
    api::{ApiError, CommonError},
    devices::{DeviceManager, DeviceSummary, LinkState, Reach},
};
use axum::http::StatusCode;
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc, time::Duration};
use td_link::{LinkError, Session, xfer::XferError};

#[derive(Debug, Clone, thiserror::Error)]
pub enum DeviceLinkError {
    #[error("The Device does not exist.")]
    NotFound,
    #[error("The Device is not connected.")]
    NotConnected(Box<LinkState>),
    #[error("The Device didn't reply in time.")]
    Timeout,
    #[error("The link to the Device failed: {0}")]
    Link(String),
    #[error(
        "The link closed before the Device answered, so it isn't known whether it did this. Check the Device, then try again if it didn't: {0}"
    )]
    UnknownOutcome(String),
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl DeviceLinkError {
    pub fn from_xfer(error: XferError) -> Self {
        match error {
            XferError::Link(LinkError::Timeout(_)) => Self::Timeout,
            XferError::Link(LinkError::Unanswered(reason)) => Self::UnknownOutcome(reason),
            other => Self::Link(other.to_string()),
        }
    }
}
impl ApiError for DeviceLinkError {
    const CODES: &'static [&'static str] = &[
        "device.not_found",
        "device.not_connected",
        "device.timeout",
        "device.link",
        "device.unknown_outcome",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::NotConnected(_) => StatusCode::CONFLICT,
            Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Link(_) | Self::UnknownOutcome(_) => StatusCode::BAD_GATEWAY,
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "device.not_found",
            Self::NotConnected(_) => "device.not_connected",
            Self::Timeout => "device.timeout",
            Self::Link(_) => "device.link",
            Self::UnknownOutcome(_) => "device.unknown_outcome",
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::NotConnected(link) => Some(serde_json::json!({ "link": link })),
            Self::Link(reason) | Self::UnknownOutcome(reason) => {
                Some(serde_json::json!({ "reason": reason }))
            }
            Self::Common(e) => e.details(),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Links {
    devices: DeviceManager,
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}
impl Links {
    pub(crate) fn new(devices: DeviceManager) -> Self {
        Self {
            devices,
            locks: Default::default(),
        }
    }
    /// The Device's summary and port, when it's connected.
    pub fn connected(&self, id: &str) -> Result<(DeviceSummary, String), DeviceLinkError> {
        let device = self
            .devices
            .summaries()
            .into_iter()
            .find(|d| d.id == id)
            .ok_or(DeviceLinkError::NotFound)?;
        match &device.link {
            LinkState::Connected { port, .. } => {
                let port = port.clone();
                Ok((device, port))
            }
            other => Err(DeviceLinkError::NotConnected(Box::new(other.clone()))),
        }
    }
    /// Runs `work` with the Device's session. Blocking: call it off the async runtime.
    /// Work on one Device waits for the work before it; other Devices don't wait.
    pub fn with<R>(
        &self,
        id: &str,
        work: impl FnOnce(&Session) -> R,
    ) -> Result<R, DeviceLinkError> {
        let lock = self.lock(id);
        let _held = lock.lock();
        self.run(id, work)
    }
    pub(crate) fn within<R>(
        &self,
        id: &str,
        wait: Duration,
        work: impl FnOnce(&Session) -> R,
    ) -> Option<Result<R, DeviceLinkError>> {
        let lock = self.lock(id);
        let _held = lock.try_lock_for(wait)?;
        Some(self.run(id, work))
    }
    fn lock(&self, id: &str) -> Arc<Mutex<()>> {
        self.locks.lock().entry(id.into()).or_default().clone()
    }
    fn run<R>(&self, id: &str, work: impl FnOnce(&Session) -> R) -> Result<R, DeviceLinkError> {
        let (device, _) = self.connected(id)?;
        let session = self
            .devices
            .session(&device.id)
            .map_err(|reach| match reach {
                Reach::NotFound => DeviceLinkError::NotFound,
                Reach::NotConnected(link) => DeviceLinkError::NotConnected(Box::new(link)),
            })?;
        Ok(work(&session))
    }
}
