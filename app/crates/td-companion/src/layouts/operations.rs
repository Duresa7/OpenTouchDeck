//! Complete Layout operations shared by the native app and the local web adapter.
use crate::{
    api::{ApiError, CommonError, JobKind},
    companion::Handle,
    jobs::{Job, JobError, JobHandle, JobScope, Stop},
    link::DeviceLinkError,
};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use td_core::{Layout, Orientation, ReplyError};
use td_link::{
    LinkError, Session,
    xfer::{self, XferError},
};
use ts_rs::TS;
/// A push precondition: `*` matches any stored Layout, otherwise a listed revision.
/// Revisions are lowercase CRC-32 strings without HTTP quoting.
pub struct IfMatch(pub Vec<String>);
impl IfMatch {
    fn check(&self, current: Option<String>) -> Result<(), LayoutError> {
        let matched = current
            .as_ref()
            .is_some_and(|c| self.0.iter().any(|tag| tag == "*" || tag == c));
        if matched {
            Ok(())
        } else {
            Err(LayoutError::Changed { revision: current })
        }
    }
}

/// Reads and pushes Layouts while hiding transfer and authority sequencing.
pub struct Operations {
    ctx: Handle,
}
impl Operations {
    pub(crate) fn new(ctx: Handle) -> Self {
        Self { ctx }
    }
    pub async fn read(
        &self,
        device: String,
        orientation: Orientation,
    ) -> Result<LayoutRead, LayoutError> {
        let ctx = self.ctx.clone();
        ctx.links.connected(&device)?;
        blocking(move || {
            let (data, revision) = ctx
                .links
                .with(&device, |session| read(session, orientation))??
                .ok_or(LayoutError::NotFound)?;
            let layout = Layout::parse(&data).map_err(|e| LayoutError::StoredInvalid(issues(e)))?;
            let _ = ctx.layouts.put(&device, &layout);
            Ok(LayoutRead { layout, revision })
        })
        .await?
    }
    pub async fn last_seen(
        &self,
        device: String,
        orientation: Orientation,
    ) -> Result<LayoutLastSeen, LastSeenError> {
        let ctx = self.ctx.clone();
        let known = crate::devices::DeviceId::parse(&device).and_then(|id| ctx.devices.get(&id));
        if known.is_none() {
            return Err(LastSeenError::DeviceNotFound);
        }
        blocking(move || ctx.layouts.get(&device, orientation))
            .await?
            .map(|layout| LayoutLastSeen { layout })
            .ok_or(LastSeenError::NotFound)
    }
    pub async fn push(
        &self,
        device: String,
        orientation: Orientation,
        data: Vec<u8>,
        expected: Option<IfMatch>,
    ) -> Result<Job, LayoutError> {
        let ctx = self.ctx.clone();
        let layout = Layout::parse(&data).map_err(|e| LayoutError::Invalid(issues(e)))?;
        if layout.orientation != orientation {
            return Err(LayoutError::OrientationMismatch);
        }
        ctx.links.connected(&device)?;
        check_live_images(&ctx, &device, &layout)?;
        let scope = JobScope::Device(device.clone());
        if let Some(job) = ctx.jobs.holding(&scope) {
            return Err(JobError::Busy(Box::new(job)).into());
        }
        if let Some(expected) = &expected {
            let (link, id) = (ctx.links.clone(), device.clone());
            let current = blocking(move || link.with(&id, |s| read(s, orientation))).await???;
            expected.check(current.map(|(_, revision)| revision))?;
        }
        let work = ctx.clone();
        Ok(ctx.jobs.start::<PushJob>(scope, true, move |job| {
            push(&work, &device, &layout, expected, job)
        })?)
    }
}
fn revision(crc32: u32) -> String {
    format!("{crc32:08x}")
}
/// The Layout the Device stores for `orientation` and its revision, or `None` for an empty slot.
fn read(
    session: &Session,
    orientation: Orientation,
) -> Result<Option<(Vec<u8>, String)>, LayoutError> {
    match xfer::get_layout(session, orientation) {
        Ok(report) => Ok(Some((report.data, revision(report.crc32)))),
        Err(XferError::Link(LinkError::Reply(ReplyError::Failed(reason))))
            if reason.starts_with("no stored layout") =>
        {
            Ok(None)
        }
        Err(e) => Err(DeviceLinkError::from_xfer(e).into()),
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CommonError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CommonError::Internal)
}

fn push(
    ctx: &Handle,
    device: &str,
    layout: &Layout,
    expected: Option<IfMatch>,
    job: &JobHandle<PushJob>,
) -> Result<PushReport, Stop> {
    let link = |e: XferError| Stop::from(DeviceLinkError::from_xfer(e));
    let _pin = ctx.assets.pin(td_host::icons::asset_ids(layout));
    ctx.links.with(device, |session| {
        // The Device may have changed between the request's check and this Job.
        if let Some(expected) = &expected {
            expected.check(read(session, layout.orientation)?.map(|(_, r)| r))?;
        }
        let (summary, _) = ctx.links.connected(device)?;
        let sinks = summary
            .diagnostics
            .as_ref()
            .map(|d| d.sinks.as_slice())
            .unwrap_or(&[]);
        layout
            .validate_sinks(sinks)
            .map_err(|e| LayoutError::Invalid(issues(e)))?;
        check_animations(ctx, device, layout)?;
        let mut assets = send_missing_assets(ctx, session, layout, job)?;
        let wanted = td_host::icons::asset_ids(layout);
        if layout.schema == 2 {
            let (summary, _) = ctx.links.connected(device)?;
            if !summary.capabilities.as_ref().is_some_and(|c| {
                c.layout_schemas.contains(&2)
                    && c.formats.iter().any(|f| f == "tdi1")
                    && c.asset_bytes >= td_host::icon_pack::FILE_BYTES as u32
            }) {
                return Err(DeviceLinkError::Link(
                    "The Device needs Firmware with schema 2 Asset support.".into(),
                )
                .into());
            }
            let stored: std::collections::BTreeSet<_> = td_link::asset::list_assets(session)
                .map_err(crate::features::assets::asset_link)?
                .into_iter()
                .map(|a| a.id)
                .collect();
            let missing: Vec<_> = wanted.difference(&stored).collect();
            let mut progress = PushProgress {
                assets_total: missing.len() as u32,
                ..Default::default()
            };
            for id in missing {
                job.check()?;
                let file = ctx
                    .assets
                    .get(id)
                    .ok_or(crate::features::assets::AssetError::NotFound)?;
                td_link::asset::put_asset(
                    session,
                    &file,
                    td_link::asset::AssetFormat::sniff(&file).ok_or_else(|| {
                        crate::features::assets::unsupported(
                            "format",
                            "tdi1 or gif",
                            "unknown",
                            "format",
                        )
                    })?,
                )
                .map_err(crate::features::assets::asset_link)?;
                assets.sent.push(id.clone());
                progress.assets_sent += 1;
                progress.bytes_sent += file.len() as u64;
                job.progress(&progress);
            }
        }
        job.check()?;
        job.set_cancellable(false);
        let mut put = None;
        ctx.authority
            .replace(device, || {
                put = Some(xfer::put_layout(session, layout.orientation, layout).map_err(link)?);
                #[cfg(any(test, feature = "test-hooks"))]
                let _ = ctx
                    .checkpoints
                    .pass(crate::checkpoints::Checkpoint::LayoutPut);
                let _ = ctx.layouts.put(device, layout);
                Ok::<_, Stop>(Some(layout.clone()))
            })
            .map_err(|error| match error {
                crate::layouts::authority::ReplaceError::Write(error) => error,
                crate::layouts::authority::ReplaceError::Save(_) => CommonError::Internal.into(),
            })?;
        let put = put.unwrap();
        let state = serde_json::from_value(serde_json::to_value(&put.state).unwrap())
            .map_err(|_| CommonError::Internal)?;
        Ok::<_, Stop>(PushReport {
            state,
            bytes: put.bytes as u64,
            chunks: put.chunks,
            retransmits: put.retransmits,
            elapsed_ms: put.elapsed_ms,
            assets,
            revision: revision(xfer::crc32(&layout.to_canonical_json())),
        })
    })?
}
fn send_missing_assets(
    ctx: &Handle,
    session: &Session,
    layout: &Layout,
    job: &JobHandle<PushJob>,
) -> Result<AssetSync, Stop> {
    let link = |e: XferError| Stop::from(DeviceLinkError::from_xfer(e));
    let wanted = td_host::icons::file_ids(layout);
    let stored = xfer::list_icons(session).map_err(link)?;
    let missing: Vec<(String, Option<Vec<u8>>)> = wanted
        .difference(&stored)
        .map(|id| (id.clone(), ctx.assets.get(id)))
        .collect();
    let mut progress = PushProgress {
        assets_total: missing.iter().filter(|(_, f)| f.is_some()).count() as u32,
        bytes_total: missing
            .iter()
            .flat_map(|(_, f)| f)
            .map(|f| f.len() as u64)
            .sum(),
        ..Default::default()
    };
    job.progress(&progress);
    let mut sync = AssetSync::default();
    for (id, file) in missing {
        job.check()?;
        let Some(file) = file else {
            sync.unavailable.push(id);
            continue;
        };
        xfer::put_icon(session, &id, &file).map_err(link)?;
        progress.assets_sent += 1;
        progress.bytes_sent += file.len() as u64;
        job.progress(&progress);
        sync.sent.push(id);
    }
    Ok(sync)
}

fn check_animations(ctx: &Handle, device: &str, layout: &Layout) -> Result<(), Stop> {
    use crate::features::assets::{AssetError, gif_caps, gif_limits, too_large, unsupported};
    use td_core::layout::{Page, TileRender};
    for page in &layout.pages {
        let Page::Grid { id, tiles, .. } = page else {
            continue;
        };
        let mut count = 0u64;
        let mut pixels = 0u64;
        for tile in tiles {
            let Some(asset) = &tile.asset else { continue };
            let Some(bytes) = ctx
                .assets
                .get(asset.strip_prefix("sha256:").unwrap_or(asset))
            else {
                if tile.render == Some(TileRender::Gif) {
                    return Err(AssetError::NotFound.into());
                }
                continue;
            };
            let gif = td_link::asset::AssetFormat::sniff(&bytes)
                == Some(td_link::asset::AssetFormat::Gif);
            if gif != (tile.render == Some(TileRender::Gif)) {
                return Err(
                    unsupported("render", "matching Asset format", "mismatch", "format").into(),
                );
            }
            if !gif {
                continue;
            }
            let caps = gif_caps(ctx, device)?;
            let limits = gif_limits(&caps)?;
            let info = td_host::gif_normalize::inspect_normalized(&bytes)
                .map_err(|_| unsupported("format", "normalized gif", "invalid", "format"))?;
            for (field, limit, actual, unit) in [
                ("asset_bytes", limits.max_bytes, bytes.len(), "bytes"),
                (
                    "anim.width",
                    usize::from(limits.image.0),
                    usize::from(info.width),
                    "pixels",
                ),
                (
                    "anim.height",
                    usize::from(limits.image.1),
                    usize::from(info.height),
                    "pixels",
                ),
                ("gif_frames", limits.max_frames, info.frames, "frames"),
                (
                    "gif_pixels",
                    limits.max_frame_pixels_total,
                    usize::from(info.width) * usize::from(info.height) * info.frames,
                    "pixels",
                ),
            ] {
                if actual > limit {
                    return Err(too_large(field, limit, actual, unit).into());
                }
            }
            if info.min_delay_ms < limits.min_delay_ms {
                return Err(unsupported(
                    "min_delay_ms",
                    &limits.min_delay_ms.to_string(),
                    &info.min_delay_ms.to_string(),
                    "format",
                )
                .into());
            }
            count += 1;
            pixels += u64::from(info.width) * u64::from(info.height);
            if let Some(anim) = caps.anim {
                for (field, limit, actual) in [
                    ("page_tiles", u64::from(anim.page_tiles), count),
                    ("page_pixels", u64::from(anim.page_pixels), pixels),
                ] {
                    if actual > limit {
                        return Err(LayoutError::AnimationLimit {
                            page: id.clone(),
                            field,
                            limit,
                            actual,
                        }
                        .into());
                    }
                }
            }
        }
    }
    Ok(())
}

/// Refuses a Layout with more Tiles that declare an `image` stream than the Device's `live_images`, across every
/// Page, because the Device builds every Page when it shows the Layout.
fn check_live_images(ctx: &Handle, device: &str, layout: &Layout) -> Result<(), LayoutError> {
    let (device, _) = ctx.links.connected(device)?;
    let limit = device.capabilities.map_or(0, |caps| caps.live_images);
    let actual = layout
        .pages
        .iter()
        .filter_map(|page| match page {
            td_core::layout::Page::Grid { tiles, .. } => Some(tiles),
            _ => None,
        })
        .flatten()
        .filter(|tile| {
            tile.streams
                .iter()
                .flatten()
                .any(|(_, kind)| *kind == td_core::layout::StreamType::Image)
        })
        .count();
    if actual > limit as usize {
        return Err(LayoutError::LiveImageLimit { limit, actual });
    }
    Ok(())
}

#[derive(Serialize, Deserialize, TS)]
pub struct LayoutRead {
    pub layout: Layout,
    /// The Device's CRC-32 of the stored Layout as 8 hex digits, also sent as `ETag`.
    pub revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct LayoutIssue {
    /// The rule key from `schema/README.md`, such as `tile.overlap`.
    pub rule: String,
    /// The JSON pointer of the value the rule refused, or empty when the rule names no single value.
    pub path: String,
    pub message: String,
}
pub(crate) fn issues(errors: Vec<td_core::layout::LayoutError>) -> Vec<LayoutIssue> {
    errors
        .into_iter()
        .map(|e| LayoutIssue {
            rule: e.rule.key().into(),
            path: e.path,
            message: e.message,
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("This Layout has too many Now playing Tiles. Remove one, then send it again.")]
    LiveImageLimit { limit: u32, actual: usize },
    #[error("This Page has too many animations. Remove an animated Tile.")]
    AnimationLimit {
        page: String,
        field: &'static str,
        limit: u64,
        actual: u64,
    },
    #[error("The Layout has {} problems.", .0.len())]
    Invalid(Vec<LayoutIssue>),
    #[error("The path and the Layout name different orientations.")]
    OrientationMismatch,
    #[error("The Device's Layout changed since it was loaded.")]
    Changed { revision: Option<String> },
    #[error("The Device stores no Layout for this orientation.")]
    NotFound,
    #[error("The Layout stored on the Device fails validation.")]
    StoredInvalid(Vec<LayoutIssue>),
    #[error(transparent)]
    Device(#[from] DeviceLinkError),
    #[error(transparent)]
    Job(#[from] JobError),
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl ApiError for LayoutError {
    const CODES: &'static [&'static str] = &[
        "layout.live_image_limit",
        "layout.animation_limit",
        "layout.invalid",
        "layout.orientation_mismatch",
        "layout.changed",
        "layout.not_found",
        "layout.stored_invalid",
        "device.not_found",
        "device.not_connected",
        "device.timeout",
        "device.link",
        "device.busy",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::LiveImageLimit { .. } | Self::AnimationLimit { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::Invalid(_) | Self::OrientationMismatch => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Changed { .. } => StatusCode::PRECONDITION_FAILED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::StoredInvalid(_) => StatusCode::BAD_GATEWAY,
            Self::Device(e) => e.status(),
            Self::Job(e) => e.status(),
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::LiveImageLimit { .. } => "layout.live_image_limit",
            Self::AnimationLimit { .. } => "layout.animation_limit",
            Self::Invalid(_) => "layout.invalid",
            Self::OrientationMismatch => "layout.orientation_mismatch",
            Self::Changed { .. } => "layout.changed",
            Self::NotFound => "layout.not_found",
            Self::StoredInvalid(_) => "layout.stored_invalid",
            Self::Device(e) => e.code(),
            Self::Job(e) => e.code(),
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::LiveImageLimit { limit, actual } => {
                Some(serde_json::json!({"limit":limit,"actual":actual}))
            }
            Self::AnimationLimit {
                page,
                field,
                limit,
                actual,
            } => Some(serde_json::json!({"page":page,"field":field,"limit":limit,"actual":actual})),
            Self::Invalid(issues) | Self::StoredInvalid(issues) => {
                Some(serde_json::json!({ "issues": issues }))
            }
            Self::Changed { revision } => Some(serde_json::json!({ "revision": revision })),
            Self::Device(e) => e.details(),
            Self::Job(e) => e.details(),
            Self::Common(e) => e.details(),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct LayoutLastSeen {
    /// The PC's copy, which may be stale. It has no revision, so it can't back an `If-Match`.
    pub layout: Layout,
}
#[derive(Debug, thiserror::Error)]
pub enum LastSeenError {
    #[error("The Device does not exist.")]
    DeviceNotFound,
    #[error("This PC has no copy of the Device's Layout for this orientation.")]
    NotFound,
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl ApiError for LastSeenError {
    const CODES: &'static [&'static str] = &["layout.not_found", "device.not_found"];
    fn status(&self) -> StatusCode {
        match self {
            Self::DeviceNotFound | Self::NotFound => StatusCode::NOT_FOUND,
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::DeviceNotFound => "device.not_found",
            Self::NotFound => "layout.not_found",
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Common(e) => e.details(),
            _ => None,
        }
    }
}

#[derive(Default, Serialize, Deserialize, TS)]
pub struct PushProgress {
    pub assets_sent: u32,
    pub assets_total: u32,
    pub bytes_sent: u64,
    pub bytes_total: u64,
}
/// The Device's `layout state` after the put.
#[derive(Serialize, Deserialize, TS)]
pub struct DeviceLayoutState {
    pub source: String,
    pub name: String,
    pub orientation: Orientation,
    pub pages: u32,
    pub tiles: u32,
    pub icons_loaded: Option<u32>,
    pub icons_missing: Option<u32>,
    pub bytes: Option<u32>,
    pub load_ms: Option<u64>,
    pub error: Option<String>,
}
#[derive(Default, Serialize, Deserialize, TS)]
pub struct AssetSync {
    /// Assets the Layout uses that the Device lacked and the Companion sent.
    pub sent: Vec<String>,
    /// Assets the Layout uses that the Device lacked and the Companion has no file for.
    pub unavailable: Vec<String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct PushReport {
    pub state: DeviceLayoutState,
    pub bytes: u64,
    pub chunks: u16,
    pub retransmits: u32,
    pub elapsed_ms: u64,
    pub assets: AssetSync,
    /// The pushed Layout's revision, for the next `If-Match`.
    pub revision: String,
}
pub struct PushJob;
impl JobKind for PushJob {
    const NAME: &'static str = "layouts.push";
    type Progress = PushProgress;
    type Result = PushReport;
}
