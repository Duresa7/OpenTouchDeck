//! Icon packs: list, install as a Job, remove and import Iconify sets, serve their icons as
//! SVG thumbnails, and merge their names into catalog search.
use crate::{
    api::{
        ApiError, ClientInfo, CommonError, Ctx, Endpoint, Feature, JobKind, NoQuery, Registration,
        Req, is_json,
    },
    features::assets::CatalogHit,
    jobs::{Job, JobError},
};
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use td_host::{
    catalog,
    packs::{MAX_DOWNLOAD, PackError, PackHit, PackInfo, PackStage, Packs},
};
use ts_rs::TS;

/// The catalog name pack hits carry, followed by the pack's prefix.
pub const SOURCE: &str = "iconify:";
/// The most hits `assets.catalog_search` returns.
pub const SEARCH_LIMIT: usize = 500;
/// The policy an Icon pack thumbnail is served under, in place of the app's.
pub const THUMBNAIL_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

#[derive(Debug, thiserror::Error)]
pub enum IconPackError {
    #[error(transparent)]
    Pack(#[from] PackError),
    #[error(transparent)]
    Job(#[from] JobError),
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl IconPackError {
    /// A pack error's code and status. Cancelling and disk errors answer as their Job and
    /// common counterparts do.
    fn pack(error: &PackError) -> (&'static str, StatusCode) {
        match error {
            PackError::NotFound => ("icon_pack.not_found", StatusCode::NOT_FOUND),
            PackError::Exists(_) => ("icon_pack.exists", StatusCode::CONFLICT),
            PackError::Busy => ("icon_pack.busy", StatusCode::CONFLICT),
            PackError::TooMany => ("icon_pack.too_many", StatusCode::CONFLICT),
            PackError::Invalid(_) => ("icon_pack.invalid", StatusCode::UNPROCESSABLE_ENTITY),
            PackError::Integrity => ("icon_pack.integrity", StatusCode::UNPROCESSABLE_ENTITY),
            PackError::Download(_) => ("icon_pack.download_failed", StatusCode::BAD_GATEWAY),
            PackError::Cancelled => (JobError::Cancelled.code(), JobError::Cancelled.status()),
            PackError::Io(_) => (CommonError::Internal.code(), CommonError::Internal.status()),
        }
    }
}
impl ApiError for IconPackError {
    const CODES: &'static [&'static str] = &[
        "icon_pack.not_found",
        "icon_pack.exists",
        "icon_pack.busy",
        "icon_pack.too_many",
        "icon_pack.invalid",
        "icon_pack.integrity",
        "icon_pack.download_failed",
        "job.cancelled",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::Pack(e) => Self::pack(e).1,
            Self::Job(e) => e.status(),
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::Pack(e) => Self::pack(e).0,
            Self::Job(e) => e.code(),
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Pack(_) => None,
            Self::Job(e) => e.details(),
            Self::Common(e) => e.details(),
        }
    }
}

fn typed_error(error: std::sync::Arc<crate::icon_packs::Error>) -> IconPackError {
    match std::sync::Arc::try_unwrap(error) {
        Ok(crate::icon_packs::Error::Pack(error)) => error.into(),
        _ => CommonError::Internal.into(),
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CommonError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CommonError::Internal)
}

/// The curated packs, installed or not, then imported ones.
pub struct List;
impl Endpoint for List {
    const NAME: &'static str = "icon_packs.list";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/icon-packs";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<PackInfo>;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.icon_packs()
            .list()
            .await
            .map_err(|_| CommonError::Internal)
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct PackPath {
    pub prefix: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct InstallProgress {
    pub stage: PackStage,
}
pub struct InstallJob;
impl JobKind for InstallJob {
    const NAME: &'static str = "icon_packs.install";
    const COMPANION_LIMIT: usize = 2;
    type Progress = InstallProgress;
    type Result = PackInfo;
}
/// Downloads a curated pack from the npm registry, checks its pinned SHA-512 and unpacks it.
pub struct Install;
impl Endpoint for Install {
    const NAME: &'static str = "icon_packs.install";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/icon-packs/{prefix}";
    const STATUS: StatusCode = StatusCode::ACCEPTED;
    type Path = PackPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = Job;
    type Error = IconPackError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.icon_packs().start_install_job(req.path.prefix)
    }
}

/// Deletes an installed pack's folder.
pub struct Remove;
impl Endpoint for Remove {
    const NAME: &'static str = "icon_packs.remove";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/icon-packs/{prefix}";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = PackPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = IconPackError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.icon_packs()
            .remove(&req.path.prefix)
            .await
            .map_err(typed_error)
    }
}

/// An Iconify JSON file, sent as the raw `application/json` body.
#[derive(Serialize, Deserialize, TS)]
#[ts(type = "Blob")]
pub struct PackFile(pub Vec<u8>);
/// Validates and installs an Iconify JSON file someone provided. Its license is theirs.
pub struct Import;
impl Endpoint for Import {
    const NAME: &'static str = "icon_packs.import";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/icon-packs/import";
    const BODY_LIMIT: usize = MAX_DOWNLOAD as usize;
    type Path = ();
    type Query = NoQuery;
    type Body = PackFile;
    type Ok = PackInfo;
    type Error = IconPackError;
    async fn read_body(
        ctx: &Ctx,
        headers: &HeaderMap,
        _: Option<&ClientInfo>,
        body: Body,
    ) -> Result<PackFile, CommonError> {
        if !is_json(headers) {
            return Err(CommonError::MediaType);
        }
        let bytes = tokio::time::timeout(ctx.limits.body_read, to_bytes(body, Self::BODY_LIMIT))
            .await
            .map_err(|_| CommonError::Timeout)?
            .map_err(|_| CommonError::TooLarge)?;
        Ok(PackFile(bytes.to_vec()))
    }
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.icon_packs()
            .import(req.body.0)
            .await
            .map_err(typed_error)
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct PackIconPath {
    pub prefix: String,
    /// The icon's name followed by `.svg`.
    pub icon: String,
}
/// A standalone SVG of one installed icon. The response body is the SVG itself.
#[derive(Serialize, Deserialize, TS)]
#[ts(type = "Blob")]
pub struct PackSvg(pub String);
pub struct Thumbnail;
impl Endpoint for Thumbnail {
    const NAME: &'static str = "icon_packs.thumbnail";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/assets/packs/{prefix}/{icon}";
    type Path = PackIconPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = PackSvg;
    type Error = IconPackError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let PackIconPath { prefix, icon } = req.path;
        let name = icon
            .strip_suffix(".svg")
            .ok_or(PackError::NotFound)?
            .to_owned();
        Ok(PackSvg(
            blocking(move || ctx.packs.svg(&prefix, &name)).await??,
        ))
    }
    /// The SVG is served under its own, stricter policy: nothing loads, scripts never run,
    /// and inline styles still apply.
    fn respond(value: PackSvg) -> Response {
        (
            [
                (header::CONTENT_TYPE, "image/svg+xml"),
                (header::CONTENT_SECURITY_POLICY, THUMBNAIL_CSP),
            ],
            value.0,
        )
            .into_response()
    }
}

/// Catalog and pack hits for `assets.catalog_search`, merged by rank: catalog hits first
/// within a rank, then packs by prefix. `catalog` limits it to one catalog or one pack.
pub(crate) fn search(
    packs: &Packs,
    query: &str,
    catalog: Option<&str>,
    entry: impl Fn(&catalog::Entry) -> CatalogHit,
) -> Vec<CatalogHit> {
    let pack = catalog.map(|c| c.strip_prefix(SOURCE));
    let entries = match pack {
        Some(Some(_)) => Vec::new(),
        _ => catalog::search_ranked(query, catalog),
    };
    let hits = match pack {
        Some(None) => Vec::new(),
        Some(Some(prefix)) => packs.search(query, Some(prefix), SEARCH_LIMIT),
        None => packs.search(query, None, SEARCH_LIMIT),
    };
    let mut merged: Vec<(u8, CatalogHit)> = entries
        .into_iter()
        .map(|(rank, e)| (rank, entry(e)))
        .chain(hits.into_iter().map(|(rank, hit)| (rank, pack_hit(hit))))
        .collect();
    merged.sort_by_key(|(rank, _)| *rank);
    merged.truncate(SEARCH_LIMIT);
    merged.into_iter().map(|(_, hit)| hit).collect()
}

fn pack_hit(hit: PackHit) -> CatalogHit {
    let license = hit
        .license
        .spdx
        .clone()
        .unwrap_or_else(|| hit.license.title.clone());
    let mut attribution = if hit.attribution {
        format!(
            "Icons from {} by {} ({}); resized, padded or tinted.",
            hit.pack, hit.author.name, hit.license.title
        )
    } else {
        format!("Icons from {} ({license}).", hit.pack)
    };
    if hit.brands {
        attribution.push_str(" Logos belong to their owners.");
    }
    CatalogHit {
        source: format!("{SOURCE}{}", hit.prefix),
        id: hit.name.clone(),
        name: hit.name,
        aliases: hit.aliases,
        hex: None,
        tags: Vec::new(),
        category: Some(hit.pack),
        svg: true,
        light: false,
        dark: false,
        license,
        license_url: hit.license.url.clone(),
        guidelines: None,
        attribution_url: hit.author.url.or(hit.license.url).unwrap_or_default(),
        attribution,
    }
}

fn feature() -> Feature {
    Feature::new("icon_packs")
        .endpoint::<List>()
        .endpoint::<Install>()
        .endpoint::<Remove>()
        .endpoint::<Import>()
        .endpoint::<Thumbnail>()
        .job::<InstallJob>()
}
inventory::submit! { Registration(feature) }
