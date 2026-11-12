//! Local web adapter for the running Integration module.
use crate::api::{
    ApiError, CommonError, Ctx, Endpoint, Event, Feature, NoQuery, Registration, Req,
};
use crate::integrations::{Error, Service};
pub use crate::integrations::{IntegrationStatusEvent, IntegrationSummary, IntegrationView};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

impl Event for IntegrationStatusEvent {
    const NAME: &'static str = "integration.status";
}
#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("The Integration does not exist.")]
    NotFound,
    #[error("Invalid Integration config.")]
    ConfigInvalid,
    #[error("Invalid Integration config.")]
    ConfigField {
        field: &'static str,
        message: &'static str,
    },
    #[error("The credential store is unavailable.")]
    Credentials,
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl From<Error> for IntegrationError {
    fn from(e: Error) -> Self {
        match e {
            Error::NotFound => Self::NotFound,
            Error::ConfigInvalid => Self::ConfigInvalid,
            Error::ConfigField { field, message } => Self::ConfigField { field, message },
            Error::Credentials => Self::Credentials,
            Error::Storage => Self::Common(CommonError::Internal),
        }
    }
}
impl ApiError for IntegrationError {
    const CODES: &'static [&'static str] = &[
        "integration.not_found",
        "integration.config_invalid",
        "integration.credentials",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ConfigInvalid | Self::ConfigField { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Credentials => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::NotFound => Self::CODES[0],
            Self::ConfigInvalid | Self::ConfigField { .. } => Self::CODES[1],
            Self::Credentials => Self::CODES[2],
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<Value> {
        match self {
            Self::ConfigInvalid => {
                Some(serde_json::json!({"issues":[{"message":"Check the Integration fields."}]}))
            }
            Self::ConfigField { field, message } => {
                Some(serde_json::json!({"issues":[{"path":field,"message":message}]}))
            }
            Self::Common(e) => e.details(),
            _ => None,
        }
    }
}
async fn work<T: Send + 'static>(
    ctx: Ctx,
    f: impl FnOnce(&Service) -> Result<T, Error> + Send + 'static,
) -> Result<T, IntegrationError> {
    tokio::task::spawn_blocking(move || f(ctx.integrations()))
        .await
        .map_err(|_| IntegrationError::Credentials)?
        .map_err(IntegrationError::from)
}
#[derive(Serialize, Deserialize, TS)]
pub struct IntegrationPath {
    pub integration: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct SecretPath {
    pub integration: String,
    pub field: String,
}
#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SecretInput {
    pub value: String,
}
#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct EnabledInput {
    pub enabled: bool,
}
#[derive(Serialize, Deserialize, TS)]
pub struct SecretHint {
    pub hint: String,
}

pub struct List;
impl Endpoint for List {
    const NAME: &'static str = "integrations.list";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/integrations";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<IntegrationSummary>;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, Service::list).await
    }
}
pub struct Get;
impl Endpoint for Get {
    const NAME: &'static str = "integrations.get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/integrations/{integration}";
    type Path = IntegrationPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = IntegrationView;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |service| service.view(&req.path.integration)).await
    }
}
pub struct ConfigPut;
impl Endpoint for ConfigPut {
    const NAME: &'static str = "integrations.config_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/integrations/{integration}/config";
    type Path = IntegrationPath;
    type Query = NoQuery;
    type Body = Value;
    type Ok = IntegrationView;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |service| {
            service.configure(&req.path.integration, req.body)
        })
        .await
    }
}
pub struct EnabledPut;
impl Endpoint for EnabledPut {
    const NAME: &'static str = "integrations.enabled_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/integrations/{integration}/enabled";
    type Path = IntegrationPath;
    type Query = NoQuery;
    type Body = EnabledInput;
    type Ok = IntegrationView;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |service| {
            service.set_enabled(&req.path.integration, req.body.enabled)
        })
        .await
    }
}
pub struct SecretPut;
impl Endpoint for SecretPut {
    const NAME: &'static str = "integrations.secret_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/integrations/{integration}/secrets/{field}";
    type Path = SecretPath;
    type Query = NoQuery;
    type Body = SecretInput;
    type Ok = SecretHint;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |service| {
            service.secret_put(&req.path.integration, &req.path.field, &req.body.value)?;
            Ok(SecretHint { hint: "Set".into() })
        })
        .await
    }
}
pub struct SecretDelete;
impl Endpoint for SecretDelete {
    const NAME: &'static str = "integrations.secret_delete";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/integrations/{integration}/secrets/{field}";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = SecretPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |service| {
            service.secret_delete(&req.path.integration, &req.path.field)
        })
        .await
    }
}
pub struct Refresh;
impl Endpoint for Refresh {
    const NAME: &'static str = "integrations.refresh";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/integrations/{integration}/refresh";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = IntegrationPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |service| service.refresh(&req.path.integration)).await
    }
}
fn feature() -> Feature {
    Feature::new("integrations")
        .endpoint::<List>()
        .endpoint::<Get>()
        .endpoint::<ConfigPut>()
        .endpoint::<EnabledPut>()
        .endpoint::<SecretPut>()
        .endpoint::<SecretDelete>()
        .endpoint::<Refresh>()
        .event::<IntegrationStatusEvent>()
        .snapshot("integrations", |ctx| ctx.integrations.summaries())
}
inventory::submit! { Registration(feature) }
