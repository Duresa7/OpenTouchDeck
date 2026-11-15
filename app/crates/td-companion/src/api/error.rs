use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A feature's error type. The kernel takes the message from `Display`.
pub trait ApiError: std::fmt::Display + Send {
    /// Every code this type adds to the catalogue, listed in each route's `EndpointInfo`.
    /// Codes from `CommonError` apply to every route.
    const CODES: &'static [&'static str];
    fn status(&self) -> StatusCode;
    fn code(&self) -> &'static str;
    fn details(&self) -> Option<serde_json::Value> {
        None
    }
    fn body(&self) -> ErrorBody {
        ErrorBody {
            code: self.code().into(),
            message: self.to_string(),
            details: self.details(),
        }
    }
}

/// The one error shape: `{"error": ErrorBody}`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

pub(crate) fn respond(error: &impl ApiError) -> Response {
    (
        error.status(),
        Json(serde_json::json!({ "error": error.body() })),
    )
        .into_response()
}

/// Errors the kernel and every feature can return.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CommonError {
    #[error("A Job or another install is active.")]
    SystemBusy,
    #[error("The Companion installer holds the lock.")]
    Installing,
    #[error("The Host is not allowed.")]
    Host,
    #[error("The Origin or request site is not allowed.")]
    Origin,
    #[error("A session token is required.")]
    AuthMissing,
    #[error("The token is invalid.")]
    AuthInvalid,
    #[error("The browser session expired.")]
    AuthExpired,
    #[error(
        "This pairing code has expired or is wrong. Choose Open in browser from the tray icon again."
    )]
    CodeInvalid,
    #[error("Pairing was blocked after wrong codes. Start pairing again.")]
    CodeLocked,
    #[error("{0}")]
    AuthRevoked(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("The request is invalid.")]
    Invalid { part: &'static str, reason: String },
    #[error("The request body is too large.")]
    TooLarge,
    #[error("The request took too long.")]
    Timeout,
    #[error("The request requires JSON.")]
    MediaType,
    #[error("{0}")]
    Limits(&'static str),
    #[error("The route does not exist.")]
    NotFound,
    #[error("The method is not allowed.")]
    MethodNotAllowed,
    #[error("Something went wrong in the Companion.")]
    Internal,
}
impl CommonError {
    pub fn invalid(part: &'static str, reason: impl ToString) -> Self {
        Self::Invalid {
            part,
            reason: reason.to_string(),
        }
    }
}
impl ApiError for CommonError {
    const CODES: &'static [&'static str] = &[
        "system.busy",
        "companion.installing",
        "security.host",
        "security.origin",
        "auth.missing",
        "auth.invalid",
        "auth.expired",
        "auth.code_invalid",
        "auth.code_locked",
        "auth.revoked",
        "auth.forbidden",
        "request.invalid",
        "request.too_large",
        "request.timeout",
        "request.media_type",
        "limits.exceeded",
        "route.not_found",
        "route.method_not_allowed",
        "internal",
    ];
    fn status(&self) -> StatusCode {
        let status = match self {
            Self::SystemBusy => 409,
            Self::Installing => 503,
            Self::Host => 421,
            Self::Origin | Self::Forbidden(_) => 403,
            Self::AuthExpired | Self::CodeInvalid => 401,
            Self::CodeLocked => 429,
            Self::AuthMissing | Self::AuthInvalid | Self::AuthRevoked(_) => 401,
            Self::Invalid { .. } => 400,
            Self::TooLarge => 413,
            Self::Timeout => 408,
            Self::MediaType => 415,
            Self::Limits(_) => 429,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::Internal => 500,
        };
        StatusCode::from_u16(status).unwrap()
    }
    fn code(&self) -> &'static str {
        match self {
            Self::SystemBusy => "system.busy",
            Self::Installing => "companion.installing",
            Self::Host => "security.host",
            Self::Origin => "security.origin",
            Self::AuthMissing => "auth.missing",
            Self::AuthInvalid => "auth.invalid",
            Self::AuthExpired => "auth.expired",
            Self::CodeInvalid => "auth.code_invalid",
            Self::CodeLocked => "auth.code_locked",
            Self::AuthRevoked(_) => "auth.revoked",
            Self::Forbidden(_) => "auth.forbidden",
            Self::Invalid { .. } => "request.invalid",
            Self::TooLarge => "request.too_large",
            Self::Timeout => "request.timeout",
            Self::MediaType => "request.media_type",
            Self::Limits(_) => "limits.exceeded",
            Self::NotFound => "route.not_found",
            Self::MethodNotAllowed => "route.method_not_allowed",
            Self::Internal => "internal",
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Invalid { part, reason } => {
                Some(serde_json::json!({ "part": part, "reason": reason }))
            }
            _ => None,
        }
    }
}
impl IntoResponse for CommonError {
    fn into_response(self) -> Response {
        respond(&self)
    }
}

/// Replaces a body axum or hyper produced on its own (a rejection, a 404, a 405) with the one
/// error shape, keyed by status.
pub(crate) fn normalize(response: Response) -> Response {
    let status = response.status();
    let shaped = response
        .headers()
        .get("content-type")
        .is_some_and(|v| v.as_bytes().starts_with(b"application/json"));
    if !(status.is_client_error() || status.is_server_error()) || shaped {
        return response;
    }
    let error = match status.as_u16() {
        404 => CommonError::NotFound,
        405 => CommonError::MethodNotAllowed,
        408 => CommonError::Timeout,
        413 => CommonError::TooLarge,
        415 => CommonError::MediaType,
        400..=499 => CommonError::invalid("request", status),
        _ => CommonError::Internal,
    };
    let mut shaped = error.into_response();
    *shaped.status_mut() = status;
    shaped
}
