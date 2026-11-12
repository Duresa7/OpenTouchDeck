//! Each Device's Layout: read it from the Device with its revision, push one as a Job, and
//! read the copy this PC last saw.
pub use crate::layouts::{
    AssetSync, DeviceLayoutState, IfMatch, LastSeenError, LayoutError, LayoutIssue, LayoutLastSeen,
    LayoutRead, PushJob, PushProgress, PushReport,
};
use crate::{
    api::{Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    jobs::Job,
};
use axum::{
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use td_core::{Layout, Orientation};
use ts_rs::TS;

#[derive(Serialize, Deserialize, TS)]
pub struct LayoutPath {
    pub device: String,
    pub orientation: Orientation,
}
/// A Layout body, kept as sent so `td_core::Layout` validation sees the exact text once.
#[derive(Serialize, Deserialize, TS)]
#[ts(type = "Layout")]
pub struct LayoutBody(pub Box<RawValue>);
impl LayoutBody {
    pub fn new(layout: &Layout) -> Self {
        Self(RawValue::from_string(String::from_utf8(layout.to_canonical_json()).unwrap()).unwrap())
    }
}
/// Reads the Layout the Device stores for an orientation, with its revision.
pub struct Get;
impl Endpoint for Get {
    const NAME: &'static str = "layouts.get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices/{device}/layouts/{orientation}";
    type Path = LayoutPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = LayoutRead;
    type Error = LayoutError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.layout_operations()
            .read(req.path.device, req.path.orientation)
            .await
    }
    fn respond(value: LayoutRead) -> Response {
        let etag = format!("\"{}\"", value.revision);
        (StatusCode::OK, [(header::ETAG, etag)], axum::Json(value)).into_response()
    }
}

/// The Layout this PC last saw for a known Device and orientation, from the Layout store.
/// It works whether or not the Device is connected and never reads the Device.
pub struct LastSeen;
impl Endpoint for LastSeen {
    const NAME: &'static str = "layouts.last_seen";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices/{device}/layouts/{orientation}/last-seen";
    type Path = LayoutPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = LayoutLastSeen;
    type Error = LastSeenError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.layout_operations()
            .last_seen(req.path.device, req.path.orientation)
            .await
    }
}

/// Validates the Layout, checks `If-Match`, then pushes it as a Job.
pub struct Push;
impl Endpoint for Push {
    const NAME: &'static str = "layouts.push";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/devices/{device}/layouts/{orientation}";
    const STATUS: StatusCode = StatusCode::ACCEPTED;
    type Path = LayoutPath;
    type Query = NoQuery;
    type Body = LayoutBody;
    type Ok = Job;
    type Error = LayoutError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.layout_operations()
            .push(
                req.path.device,
                req.path.orientation,
                req.body.0.get().as_bytes().to_vec(),
                IfMatch::from_headers(&req.headers),
            )
            .await
    }
}
fn feature() -> Feature {
    Feature::new("layouts")
        .endpoint::<Get>()
        .endpoint::<LastSeen>()
        .endpoint::<Push>()
        .job::<PushJob>()
}
inventory::submit! { Registration(feature) }

impl IfMatch {
    fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let value = headers.get(header::IF_MATCH)?.to_str().unwrap_or("");
        Some(Self(
            value
                .split(',')
                .map(|tag| {
                    let tag = tag.trim();
                    let tag = tag.strip_prefix("W/").unwrap_or(tag);
                    tag.trim_matches('"').to_ascii_lowercase()
                })
                .collect(),
        ))
    }
}
