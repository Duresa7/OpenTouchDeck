use crate::{
    api::{ApiError, CommonError, Ctx, Endpoint, Feature, JobKind, NoQuery, Registration, Req},
    features::devices::DevicePath,
    jobs::{Job, JobError, JobHandle, JobScope, Stop},
    link::DeviceLinkError,
};
use axum::{
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use td_core::{Layout, Orientation, layout::Action};
use td_host::{catalog, icon_pack, icons};
use td_link::xfer::{self, XferError};
use ts_rs::TS;

/// A built-in icon and the id of its Asset file.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct Builtin {
    pub name: String,
    pub label: String,
    pub tags: Vec<String>,
    pub id: String,
}
impl From<&icons::Builtin> for Builtin {
    fn from(icon: &icons::Builtin) -> Self {
        Self {
            name: icon.name.clone(),
            label: icon.label.clone(),
            tags: icon.tags.clone(),
            id: icon.id.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("{}", too_large_message(.0))]
    TooLarge(serde_json::Value),
    #[error("This GIF is too short. Choose a GIF that plays for at least {limit} ms.")]
    TooShort { limit: u32, actual: u32 },
    #[error("This Device or Asset does not support the requested format.")]
    Unsupported(serde_json::Value),
    #[error("The Device could not store this Asset. Remove unused Assets, then retry.")]
    Refused { reason: String },
    #[error("The Device could not remove this Asset.")]
    DeleteRefused { reason: String },
    #[error("The Asset does not exist.")]
    NotFound,
    #[error("The upload expired or never existed.")]
    UploadNotFound,
    #[error("The image or icon couldn't be rendered: {0}")]
    RenderFailed(String),
    #[error("Couldn't prepare this GIF. Choose another GIF.")]
    GifFailed { reason: &'static str },
    #[error(
        "This PC's Asset store is full. Retry in an hour, or remove Tiles you don't use and send the Layout."
    )]
    StoreFull { limit: u64, actual: u64 },
    #[error(transparent)]
    Device(#[from] DeviceLinkError),
    #[error(transparent)]
    Job(#[from] JobError),
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl ApiError for AssetError {
    const CODES: &'static [&'static str] = &[
        "asset.too_large",
        "asset.too_short",
        "asset.unsupported",
        "asset.refused",
        "asset.not_found",
        "upload.not_found",
        "asset.render_failed",
        "asset.store_full",
        "device.not_found",
        "device.not_connected",
        "device.timeout",
        "device.link",
        "device.busy",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::TooLarge(_)
            | Self::TooShort { .. }
            | Self::Unsupported(_)
            | Self::Refused { .. }
            | Self::DeleteRefused { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::NotFound | Self::UploadNotFound => StatusCode::NOT_FOUND,
            Self::RenderFailed(_) | Self::GifFailed { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::StoreFull { .. } => StatusCode::INSUFFICIENT_STORAGE,
            Self::Device(e) => e.status(),
            Self::Job(e) => e.status(),
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::TooLarge(_) => "asset.too_large",
            Self::TooShort { .. } => "asset.too_short",
            Self::Unsupported(_) => "asset.unsupported",
            Self::Refused { .. } | Self::DeleteRefused { .. } => "asset.refused",
            Self::NotFound => "asset.not_found",
            Self::UploadNotFound => "upload.not_found",
            Self::RenderFailed(_) | Self::GifFailed { .. } => "asset.render_failed",
            Self::StoreFull { .. } => "asset.store_full",
            Self::Device(e) => e.code(),
            Self::Job(e) => e.code(),
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::TooLarge(details) | Self::Unsupported(details) => Some(details.clone()),
            Self::TooShort { limit, actual } => Some(
                serde_json::json!({"field":"duration_ms","limit":limit,"actual":actual,"unit":"ms"}),
            ),
            Self::Refused { reason } | Self::DeleteRefused { reason } => {
                Some(serde_json::json!({"reason":reason}))
            }
            Self::GifFailed { reason } => Some(serde_json::json!({"reason":reason})),
            Self::StoreFull { limit, actual } => {
                Some(serde_json::json!({"limit":limit,"actual":actual,"unit":"bytes"}))
            }
            Self::Device(e) => e.details(),
            Self::Job(e) => e.details(),
            Self::Common(e) => e.details(),
            _ => None,
        }
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CommonError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CommonError::Internal)
}

pub struct BuiltinList;
impl Endpoint for BuiltinList {
    const NAME: &'static str = "assets.builtin";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/assets/builtin";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<Builtin>;
    type Error = CommonError;
    async fn handle(_: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        // The set renders once, on first use.
        blocking(|| icons::builtin().iter().map(Builtin::from).collect()).await
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct SuggestRequest {
    pub action: Action,
}
/// The built-in icon a Tile's action suggests, if any.
pub struct Suggest;
impl Endpoint for Suggest {
    const NAME: &'static str = "assets.suggest";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/assets/builtin/suggest";
    type Path = ();
    type Query = NoQuery;
    type Body = SuggestRequest;
    type Ok = Option<Builtin>;
    type Error = CommonError;
    async fn handle(_: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let action = serde_json::to_value(&req.body.action).map_err(|_| CommonError::Internal)?;
        blocking(move || {
            let name = icons::default_for(&action)?;
            icons::builtin()
                .iter()
                .find(|icon| icon.name == name)
                .map(Builtin::from)
        })
        .await
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct AssetPath {
    pub asset: String,
}
/// An Asset file's bytes. The response body is the raw file with its media type.
#[derive(Serialize, Deserialize, TS)]
#[ts(type = "Blob")]
pub struct AssetBytes {
    pub media_type: String,
    pub bytes: Vec<u8>,
}
pub const TDI1: &str = "application/vnd.touchdeck.tdi1";
fn media_type(file: &[u8]) -> &'static str {
    if icon_pack::is_icon(file) {
        TDI1
    } else if file.starts_with(b"GIF87a") || file.starts_with(b"GIF89a") {
        "image/gif"
    } else if file.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else {
        "application/octet-stream"
    }
}
pub struct File;
impl Endpoint for File {
    const NAME: &'static str = "assets.file";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/assets/{asset}";
    type Path = AssetPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = AssetBytes;
    type Error = AssetError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let id = req.path.asset;
        let bytes = blocking(move || ctx.assets.get(&id))
            .await?
            .ok_or(AssetError::NotFound)?;
        Ok(AssetBytes {
            media_type: media_type(&bytes).into(),
            bytes,
        })
    }
    fn respond(value: AssetBytes) -> Response {
        ([(header::CONTENT_TYPE, value.media_type)], value.bytes).into_response()
    }
}

#[derive(Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AssetSource {
    Gif {
        upload: String,
        device: String,
        target: GifTarget,
    },
    /// The icon of the program at `path`. Only its icon resource is read.
    App {
        path: String,
    },
    Catalog {
        catalog: String,
        id: String,
        #[serde(default)]
        variant: CatalogVariant,
        #[serde(default)]
        padding: u32,
        background: Option<[u8; 4]>,
        tint: Option<[u8; 3]>,
    },
    Upload {
        upload: String,
    },
    /// A logo from Brandfetch, fetched with the user's API key.
    Brandfetch(super::brandfetch::BrandfetchLogo),
}
const GIF_TARGET_MAX: u16 = 4096;
#[derive(Serialize, Deserialize, TS)]
pub struct GifTarget {
    pub width: u16,
    pub height: u16,
    pub background: [u8; 3],
}
/// How an image that isn't square fills the Tile: whole inside it, or cropped to fill it.
#[derive(Clone, Copy, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Contain,
    Cover,
}
#[derive(Serialize, Deserialize, TS)]
pub struct CreateAsset {
    pub source: AssetSource,
    #[serde(default)]
    #[ts(optional)]
    pub fit: Option<Fit>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct RenderProgress {
    pub frames_done: u32,
    pub frames_total: u32,
}
#[derive(Serialize, Deserialize, TS)]
pub struct AssetRef {
    pub id: String,
    /// The encoded format: `tdi1` or `gif`.
    pub kind: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}
pub struct RenderJob;
impl JobKind for RenderJob {
    const NAME: &'static str = "assets.render";
    const COMPANION_LIMIT: usize = 2;
    type Progress = RenderProgress;
    type Result = AssetRef;
}
/// Prepares an Asset as a Job. At most two run at once across the Companion.
pub struct Create;
impl Endpoint for Create {
    const NAME: &'static str = "assets.create";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/assets";
    const STATUS: StatusCode = StatusCode::ACCEPTED;
    type Path = ();
    type Query = NoQuery;
    type Body = CreateAsset;
    type Ok = Job;
    type Error = AssetError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let owner = req.client().id.clone();
        let CreateAsset { source, fit } = req.body;
        if let AssetSource::Gif { target, .. } = &source
            && ![target.width, target.height]
                .iter()
                .all(|side| (1..=GIF_TARGET_MAX).contains(side))
        {
            return Err(CommonError::invalid(
                "body",
                format!("target width and height must be from 1 to {GIF_TARGET_MAX}"),
            )
            .into());
        }
        let gif = matches!(source, AssetSource::Gif { .. });
        let work = ctx.clone();
        let run = move |job: &JobHandle<RenderJob>| {
            render(&work, &owner, source, fit.unwrap_or_default(), job)
        };
        Ok(if gif {
            ctx.jobs
                .start_turn::<RenderJob>(JobScope::Companion, true, "gif", run)?
        } else {
            ctx.jobs
                .start::<RenderJob>(JobScope::Companion, true, run)?
        })
    }
}
fn render(
    ctx: &Ctx,
    owner: &str,
    source: AssetSource,
    fit: Fit,
    job: &JobHandle<RenderJob>,
) -> Result<AssetRef, Stop> {
    let progress = |frames_done| RenderProgress {
        frames_done,
        frames_total: 1,
    };
    job.progress(&progress(0));
    let image = match source {
        AssetSource::Gif {
            upload,
            device,
            target,
        } => {
            let _ = gif_limits(&gif_caps(ctx, &device)?)?;
            let claim = ctx
                .uploads
                .claim(owner, &upload, ctx.clock.now())
                .ok_or(AssetError::UploadNotFound)?;
            job.acquire_slot()?;
            let limits = gif_limits(&gif_caps(ctx, &device)?)?;
            let bytes = claim
                .take(ctx.clock.now())
                .ok_or(AssetError::UploadNotFound)?;
            let output = td_host::gif_normalize::normalize_gif_until(
                &bytes,
                td_host::gif_normalize::Target {
                    width: target.width,
                    height: target.height,
                    fit: match fit {
                        Fit::Contain => td_host::gif_normalize::Fit::Contain,
                        Fit::Cover => td_host::gif_normalize::Fit::Cover,
                    },
                },
                &limits,
                target.background,
                &|| job.check().is_err(),
            )
            .map_err(|error| match error {
                td_host::gif_normalize::GifError::Cancelled => Stop::Cancelled,
                other => gif_error(other).into(),
            })?;
            job.check()?;
            let id = ctx.assets.put(&output.bytes).map_err(stored)?;
            job.progress(&RenderProgress {
                frames_done: output.frames as u32,
                frames_total: output.frames as u32,
            });
            return Ok(AssetRef {
                id,
                kind: "gif".into(),
                width: output.width.into(),
                height: output.height.into(),
                bytes: output.bytes.len() as u64,
            });
        }
        AssetSource::App { path } => {
            icons::app_icon(std::path::Path::new(&path)).map_err(AssetError::RenderFailed)?
        }
        AssetSource::Catalog {
            catalog,
            id,
            variant,
            padding,
            background,
            tint,
        } => {
            let options = catalog::Options {
                cover: matches!(fit, Fit::Cover),
                padding,
                background,
                tint,
                ..Default::default()
            };
            match catalog.strip_prefix(super::icon_packs::SOURCE) {
                Some(prefix) => ctx.packs.render(prefix, &id, options),
                None => ctx.catalog.render(
                    &catalog,
                    &id,
                    match variant {
                        CatalogVariant::Base => catalog::Variant::Base,
                        CatalogVariant::Light => catalog::Variant::Light,
                        CatalogVariant::Dark => catalog::Variant::Dark,
                    },
                    options,
                ),
            }
            .map_err(|e| AssetError::RenderFailed(e.to_string()))?
        }
        AssetSource::Brandfetch(logo) => return super::brandfetch::render(ctx, logo, fit, job),
        AssetSource::Upload { upload } => {
            let bytes = ctx
                .uploads
                .take(owner, &upload, ctx.clock.now())
                .ok_or(AssetError::UploadNotFound)?;
            super::uploads::decode(&bytes)?
        }
    };
    job.check()?;
    let image = match fit {
        Fit::Contain => image,
        Fit::Cover => {
            let (w, h) = image.dimensions();
            let side = w.min(h);
            image::imageops::crop_imm(&image, (w - side) / 2, (h - side) / 2, side, side).to_image()
        }
    };
    let file = icon_pack::pack_any(&image);
    job.check()?;
    let id = ctx.assets.put(&file).map_err(stored)?;
    job.progress(&progress(1));
    Ok(AssetRef {
        id,
        kind: "tdi1".into(),
        width: icon_pack::SIDE,
        height: icon_pack::SIDE,
        bytes: file.len() as u64,
    })
}

#[derive(Default, Serialize, Deserialize, TS)]
pub struct PruneRequest {
    /// Assets to keep besides the ones the Device's stored Layouts use, such as those of a
    /// Layout being edited.
    #[serde(default)]
    pub also_keep: Vec<String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct PruneProgress {
    pub removed: u32,
    pub total: u32,
}
#[derive(Serialize, Deserialize, TS)]
pub struct PruneResult {
    pub removed: Vec<String>,
    pub kept_in_use: u32,
}
pub struct PruneJob;
impl JobKind for PruneJob {
    const NAME: &'static str = "assets.prune";
    type Progress = PruneProgress;
    type Result = PruneResult;
}
/// Removes unused Assets, preserving stored Layouts, caller-kept Assets and live covers.
pub struct Prune;
impl Endpoint for Prune {
    const NAME: &'static str = "assets.prune";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/devices/{device}/assets/prune";
    const STATUS: StatusCode = StatusCode::ACCEPTED;
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = PruneRequest;
    type Ok = Job;
    type Error = AssetError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let device = req.path.device.as_str().to_owned();
        start_prune(&ctx, device, req.body.also_keep)
    }
}
pub(crate) fn start_prune(
    ctx: &crate::companion::Handle,
    device: String,
    keep: Vec<String>,
) -> Result<Job, AssetError> {
    ctx.links.connected(&device)?;
    let scope = JobScope::Device(device.clone());
    let work = ctx.clone();
    Ok(ctx.jobs.start::<PruneJob>(scope, true, move |job| {
        prune(&work, &device, keep.into_iter().collect(), job)
    })?)
}

fn prune(
    ctx: &crate::companion::Handle,
    device: &str,
    mut keep: BTreeSet<String>,
    job: &JobHandle<PruneJob>,
) -> Result<PruneResult, Stop> {
    let link = |e: XferError| Stop::from(DeviceLinkError::from_xfer(e));
    for layout in ctx.layouts.all(device) {
        keep.extend(icons::file_ids(&layout));
        keep.extend(icons::asset_ids(&layout));
    }
    ctx.links.with(device, |session| {
        for orientation in [Orientation::Landscape, Orientation::Portrait] {
            // A slot with no stored Layout uses no Asset.
            if let Ok(report) = xfer::get_layout(session, orientation)
                && let Ok(stored) = Layout::parse(&report.data)
            {
                keep.extend(icons::file_ids(&stored));
                keep.extend(icons::asset_ids(&stored));
            }
        }
        let unused: Vec<String> = xfer::list_icons(session)
            .map_err(link)?
            .difference(&keep)
            .cloned()
            .collect();
        let mut progress = PruneProgress {
            removed: 0,
            total: unused.len() as u32,
        };
        job.progress(&progress);
        let mut removed = Vec::new();
        for id in unused {
            job.check()?;
            xfer::remove_icon(session, &id).map_err(link)?;
            removed.push(id);
            progress.removed += 1;
            job.progress(&progress);
        }
        let mut kept_in_use = 0;
        let live_covers = ctx.live.cover_assets(device);
        let (device_info, _) = ctx.links.connected(device)?;
        if device_info.capabilities.is_some() {
            for asset in td_link::asset::list_assets(session).map_err(asset_link)? {
                if live_covers.contains(&asset.id) {
                    kept_in_use += 1;
                } else if !keep.contains(&asset.id) {
                    job.check()?;
                    match td_link::asset::delete_asset(session, &asset.id) {
                        Ok(_) => {
                            ctx.live.forget_cover(device, &asset.id);
                            removed.push(asset.id);
                        }
                        Err(error)
                            if error.reason() == Some(&td_link::asset::AssetReason::InUse) =>
                        {
                            kept_in_use += 1;
                        }
                        Err(error) => {
                            let error = match asset_link(error) {
                                AssetError::Refused { reason } => {
                                    AssetError::DeleteRefused { reason }
                                }
                                other => other,
                            };
                            return Err(error.into());
                        }
                    }
                }
            }
        }
        Ok(PruneResult {
            removed,
            kept_in_use,
        })
    })?
}

fn feature() -> Feature {
    Feature::new("assets")
        .endpoint::<CatalogSearch>()
        .endpoint::<super::uploads::Upload>()
        .endpoint::<OnDevice>()
        .endpoint::<BuiltinList>()
        .endpoint::<Suggest>()
        .endpoint::<File>()
        .endpoint::<Create>()
        .endpoint::<Prune>()
        .job::<RenderJob>()
        .job::<PruneJob>()
}
inventory::submit! { Registration(feature) }

#[derive(Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum CatalogVariant {
    #[default]
    Base,
    Light,
    Dark,
}
#[derive(Default, Serialize, Deserialize, TS)]
pub struct CatalogQuery {
    #[serde(default)]
    pub q: String,
    pub catalog: Option<String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct CatalogHit {
    pub source: String,
    pub name: String,
    pub id: String,
    pub aliases: Vec<String>,
    pub hex: Option<String>,
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub svg: bool,
    pub light: bool,
    pub dark: bool,
    pub license: String,
    pub license_url: Option<String>,
    pub guidelines: Option<String>,
    pub attribution_url: String,
    pub attribution: String,
}
pub struct CatalogSearch;
impl Endpoint for CatalogSearch {
    const NAME: &'static str = "assets.catalog_search";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/assets/catalog/search";
    type Path = ();
    type Query = CatalogQuery;
    type Body = ();
    type Ok = Vec<CatalogHit>;
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        blocking(move || super::icon_packs::search(&ctx.packs, &req.query.q, req.query.catalog.as_deref(), |e| CatalogHit {
            source: e.source.clone(), name: e.name.clone(), id: e.id.clone(), aliases: e.aliases.clone(), hex: e.hex.clone(),
            tags: e.tags.clone(), category: e.category.clone(), svg: e.svg, light: e.light, dark: e.dark,
            license: e.license.clone(), license_url: e.license_url.clone(), guidelines: e.guidelines.clone(), attribution_url: e.attribution_url.clone(),
            attribution: if e.source == "selfhst" { "Icons by selfh.st (CC BY 4.0); resized, padded or tinted. Logos belong to their owners." } else { "Icons by Simple Icons; see individual license and brand guidelines. Logos belong to their owners." }.into(),
        })).await
    }
}
#[derive(Serialize, Deserialize, TS)]
pub struct DeviceAssets {
    pub ids: Vec<String>,
    pub storage_free: u64,
}
pub struct OnDevice;
impl Endpoint for OnDevice {
    const NAME: &'static str = "assets.on_device";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/devices/{device}/assets";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = DeviceAssets;
    type Error = AssetError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<DeviceAssets, AssetError> {
        blocking(move || {
            ctx.links.with(req.path.device.as_str(), |session| {
                let ids = td_link::asset::list_assets(session)
                    .map_err(asset_link)?
                    .into_iter()
                    .map(|a| a.id)
                    .collect();
                let storage_free = td_link::asset::asset_space(session)
                    .map_err(asset_link)?
                    .free_bytes;
                Ok::<_, AssetError>(DeviceAssets { ids, storage_free })
            })
        })
        .await??
    }
}
pub(crate) fn asset_link(error: td_link::asset::AssetError) -> AssetError {
    use td_link::asset::{AssetError as E, AssetReason as R};
    match error {
        E::Refused {
            reason: reason @ (R::Format | R::Unsupported),
            ..
        } => unsupported("format", "supported encoding", "refused", reason.name()),
        E::Refused {
            reason: R::Size, ..
        } => AssetError::TooLarge(
            serde_json::json!({"reason":"size","field":"device_limit","limit":null,"actual":null,"unit":"unknown"}),
        ),
        E::Refused { reason, .. } => AssetError::Refused {
            reason: reason.name().into(),
        },
        E::Size(actual) => too_large(
            "asset_bytes",
            td_link::asset::ASSET_BYTES_MAX,
            actual,
            "bytes",
        ),
        E::Link(error) => DeviceLinkError::from_xfer(XferError::Link(error)).into(),
        E::Xfer(error) => DeviceLinkError::from_xfer(error).into(),
        other => DeviceLinkError::Link(other.to_string()).into(),
    }
}

pub(crate) fn unsupported(field: &str, limit: &str, actual: &str, reason: &str) -> AssetError {
    AssetError::Unsupported(
        serde_json::json!({"field":field,"limit":limit,"actual":actual,"reason":reason}),
    )
}
pub(crate) fn too_large(field: &str, limit: usize, actual: usize, unit: &str) -> AssetError {
    AssetError::TooLarge(
        serde_json::json!({"field":field,"limit":limit,"actual":actual,"unit":unit}),
    )
}
pub(crate) fn gif_caps(
    ctx: &crate::companion::Handle,
    device: &str,
) -> Result<crate::devices::Capabilities, AssetError> {
    let (summary, _) = ctx.links.connected(device)?;
    summary
        .capabilities
        .filter(|caps| {
            caps.anim.as_ref().is_some_and(|a| {
                a.image.iter().all(|n| *n > 0)
                    && a.min_delay_ms > 0
                    && a.page_tiles > 0
                    && a.page_pixels > 0
            }) && caps.formats.iter().any(|f| f == "gif")
                && caps.tile_kinds.iter().any(|f| f == "gif")
                && caps.gif_frames.unwrap_or(0) > 0
                && caps.gif_pixels.unwrap_or(0) > 0
        })
        .ok_or_else(|| unsupported("anim", "enabled", "disabled", "unsupported"))
}
pub(crate) fn gif_limits(
    caps: &crate::devices::Capabilities,
) -> Result<td_host::gif_normalize::AnimLimits, AssetError> {
    let anim = caps
        .anim
        .as_ref()
        .ok_or_else(|| unsupported("anim", "enabled", "absent", "unsupported"))?;
    Ok(td_host::gif_normalize::AnimLimits {
        image: (
            anim.image[0].min(4096) as u16,
            anim.image[1].min(4096) as u16,
        ),
        min_delay_ms: anim.min_delay_ms,
        max_frames: caps.gif_frames.unwrap_or(0) as usize,
        max_frame_pixels_total: caps.gif_pixels.unwrap_or(0) as usize,
        max_bytes: caps.asset_bytes as usize,
    })
}
fn gif_error(error: td_host::gif_normalize::GifError) -> AssetError {
    use td_host::gif_normalize::{GifError as E, SourceLimit as S};
    match error {
        E::SourceTooLarge {
            kind,
            limit,
            actual,
        } => {
            let (field, unit) = match kind {
                S::Bytes => ("source_bytes", "bytes"),
                S::Width => ("source_width", "pixels"),
                S::Height => ("source_height", "pixels"),
                S::Frames => ("source_frames", "frames"),
                S::DecodedRgba => ("decoded_rgba", "bytes"),
            };
            too_large(field, limit, actual, unit)
        }
        E::TooLong { max_frames, needed } => too_large("gif_frames", max_frames, needed, "frames"),
        E::TooLarge {
            max_bytes,
            smallest,
        } => too_large("asset_bytes", max_bytes, smallest, "bytes"),
        E::PixelLimit { limit, smallest } => too_large("gif_pixels", limit, smallest, "pixels"),
        E::TooShort {
            min_delay_ms,
            duration_ms,
        } => AssetError::TooShort {
            limit: min_delay_ms,
            actual: duration_ms,
        },
        E::NotGif => AssetError::GifFailed { reason: "not_gif" },
        E::Corrupt => AssetError::GifFailed { reason: "corrupt" },
        E::InvalidLimits => AssetError::GifFailed {
            reason: "invalid_limits",
        },
        E::TimedOut => AssetError::GifFailed {
            reason: "timed_out",
        },
        E::Cancelled => JobError::Cancelled.into(),
    }
}

pub(crate) fn stored(error: crate::store::PutError) -> AssetError {
    use crate::store::PutError as E;
    match error {
        E::Invalid => AssetError::GifFailed { reason: "invalid" },
        E::Full { limit, actual } => AssetError::StoreFull { limit, actual },
        E::Io(_) => CommonError::Internal.into(),
    }
}

fn too_large_message(details: &serde_json::Value) -> &'static str {
    match details["field"].as_str() {
        Some("decoded_rgba") => {
            "This GIF needs more than 64 MiB to decode all its frames. Choose a smaller or shorter GIF."
        }
        Some("gif_frames" | "source_frames") => "This GIF is too long. Choose a shorter GIF.",
        _ => "This GIF is too large. Choose a smaller GIF.",
    }
}

#[cfg(test)]
mod gif_errors {
    use super::*;
    #[test]
    fn retains_device_refusal_reason() {
        for reason in ["size", "format", "memory", "full"] {
            let error = asset_link(td_link::asset::AssetError::Refused {
                reason: td_link::asset::AssetReason::parse(reason),
                error: "refused".into(),
            });
            assert_eq!(error.details().unwrap()["reason"], reason);
        }
    }

    #[test]
    fn too_large_message_follows_the_field() {
        use td_host::gif_normalize::{GifError as E, SourceLimit as S};
        let long = "This GIF is too long. Choose a shorter GIF.";
        let large = "This GIF is too large. Choose a smaller GIF.";
        let decoded = "This GIF needs more than 64 MiB to decode all its frames. Choose a smaller or shorter GIF.";
        let source = |kind| E::SourceTooLarge {
            kind,
            limit: 1,
            actual: 2,
        };
        for (error, message) in [
            (
                gif_error(E::TooLong {
                    max_frames: 60,
                    needed: 61,
                }),
                long,
            ),
            (gif_error(source(S::Frames)), long),
            (gif_error(source(S::DecodedRgba)), decoded),
            (gif_error(source(S::Bytes)), large),
            (gif_error(source(S::Width)), large),
            (
                gif_error(E::TooLarge {
                    max_bytes: 1,
                    smallest: 2,
                }),
                large,
            ),
            (
                gif_error(E::PixelLimit {
                    limit: 1,
                    smallest: 2,
                }),
                large,
            ),
            (
                asset_link(td_link::asset::AssetError::Refused {
                    reason: td_link::asset::AssetReason::parse("size"),
                    error: "refused".into(),
                }),
                large,
            ),
        ] {
            assert_eq!(error.code(), "asset.too_large");
            assert_eq!(error.to_string(), message, "{:?}", error.details());
        }
    }

    #[test]
    fn preparation_failures_carry_a_reason_not_debug_text() {
        use td_host::gif_normalize::GifError as E;
        for (error, reason) in [
            (E::NotGif, "not_gif"),
            (E::Corrupt, "corrupt"),
            (E::TimedOut, "timed_out"),
            (E::InvalidLimits, "invalid_limits"),
        ] {
            let error = gif_error(error);
            assert_eq!(error.code(), "asset.render_failed");
            assert_eq!(
                error.to_string(),
                "Couldn't prepare this GIF. Choose another GIF."
            );
            assert_eq!(error.details(), Some(serde_json::json!({"reason":reason})));
        }
    }
}
