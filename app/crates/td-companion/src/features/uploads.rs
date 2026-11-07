use crate::api::{ClientInfo, CommonError, Ctx, Endpoint, NoQuery, Req};
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Method},
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use ts_rs::TS;

const MAX: usize = 16 << 20;
const TOTAL: usize = 64 << 20;
struct Entry {
    owner: String,
    expires: Instant,
    reserved: usize,
    bytes: Option<Vec<u8>>,
    claimed: bool,
}
#[derive(Clone, Default)]
pub(crate) struct Uploads(Arc<Mutex<HashMap<String, Entry>>>);
struct Reservation {
    uploads: Uploads,
    id: String,
    committed: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.committed {
            self.uploads.0.lock().remove(&self.id);
        }
    }
}
pub(crate) struct UploadClaim {
    uploads: Uploads,
    id: String,
    taken: bool,
}
impl Drop for UploadClaim {
    fn drop(&mut self) {
        if !self.taken
            && let Some(entry) = self.uploads.0.lock().get_mut(&self.id)
        {
            entry.claimed = false;
        }
    }
}
impl UploadClaim {
    pub(crate) fn take(mut self, now: Instant) -> Option<Vec<u8>> {
        self.taken = true;
        let mut entries = self.uploads.0.lock();
        if entries.get(&self.id).is_none_or(|e| e.expires <= now) {
            entries.remove(&self.id);
            return None;
        }
        entries.remove(&self.id)?.bytes
    }
}
impl Uploads {
    pub(crate) fn sweep(&self, now: Instant, active: impl Fn(&str) -> bool) {
        self.0
            .lock()
            .retain(|_, e| e.expires > now && active(&e.owner));
    }
    fn reserve(&self, owner: &str, now: Instant) -> Result<Reservation, CommonError> {
        let mut entries = self.0.lock();
        entries.retain(|_, e| e.expires > now);
        if entries.values().filter(|e| e.owner == owner).count() >= 8
            || entries.values().map(|e| e.reserved).sum::<usize>() + MAX > TOTAL
        {
            return Err(CommonError::Limits("Too many pending image uploads."));
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| CommonError::Internal)?;
        let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
        entries.insert(
            id.clone(),
            Entry {
                owner: owner.into(),
                expires: now + Duration::from_secs(3600),
                reserved: MAX,
                bytes: None,
                claimed: false,
            },
        );
        Ok(Reservation {
            uploads: self.clone(),
            id,
            committed: false,
        })
    }
    pub(crate) fn claim(&self, owner: &str, id: &str, now: Instant) -> Option<UploadClaim> {
        let mut entries = self.0.lock();
        let entry = entries.get_mut(id)?;
        if entry.owner != owner || entry.expires <= now || entry.bytes.is_none() || entry.claimed {
            return None;
        }
        entry.claimed = true;
        Some(UploadClaim {
            uploads: self.clone(),
            id: id.to_owned(),
            taken: false,
        })
    }
    pub(crate) fn take(&self, owner: &str, id: &str, now: Instant) -> Option<Vec<u8>> {
        self.claim(owner, id, now)?.take(now)
    }
}
#[derive(Serialize, Deserialize, TS)]
pub struct Uploaded {
    pub id: String,
    pub media_type: String,
    pub bytes: u64,
}
#[derive(Serialize, Deserialize, TS)]
#[ts(type = "Blob")]
pub struct UploadBody(pub Uploaded);
pub struct Upload;
impl Endpoint for Upload {
    const NAME: &'static str = "assets.upload";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/uploads";
    const BODY_LIMIT: usize = MAX;
    type Path = ();
    type Query = NoQuery;
    type Body = UploadBody;
    type Ok = Uploaded;
    type Error = CommonError;
    async fn read_body(
        ctx: &Ctx,
        headers: &HeaderMap,
        client: Option<&ClientInfo>,
        body: Body,
    ) -> Result<UploadBody, CommonError> {
        let media_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.starts_with("image/"))
            .ok_or(CommonError::MediaType)?
            .to_owned();
        let client = client.ok_or(CommonError::AuthMissing)?;
        let mut reservation = ctx.uploads.reserve(&client.id, ctx.clock.now())?;
        let bytes = tokio::time::timeout(ctx.limits.body_read, to_bytes(body, MAX))
            .await
            .map_err(|_| CommonError::Timeout)?
            .map_err(|_| CommonError::TooLarge)?
            .to_vec();
        let len = bytes.len();
        let bytes = tokio::task::spawn_blocking(move || {
            if !bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a") {
                decode(&bytes)?;
            }
            Ok::<_, CommonError>(bytes)
        })
        .await
        .map_err(|_| CommonError::Internal)??;
        let mut entries = ctx.uploads.0.lock();
        let entry = entries
            .get_mut(&reservation.id)
            .ok_or(CommonError::AuthRevoked("The session ended."))?;
        entry.reserved = len;
        entry.bytes = Some(bytes);
        reservation.committed = true;
        Ok(UploadBody(Uploaded {
            id: reservation.id.clone(),
            media_type,
            bytes: len as u64,
        }))
    }
    async fn handle(_: Ctx, req: Req<Self>) -> Result<Uploaded, CommonError> {
        Ok(req.body.0)
    }
}
/// The longer side an uploaded SVG is drawn at before it is fitted to the Tile.
const SVG_SIDE: u32 = 256;
/// Whether an upload is an SVG document: it opens with markup and names an `<svg` root early.
fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    head.trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with('<')
        && head.contains("<svg")
}
pub(crate) fn decode(bytes: &[u8]) -> Result<image::RgbaImage, CommonError> {
    if is_svg(bytes) {
        // Drawn with nothing outside the file: no linked images, files or URLs.
        return td_host::icon_pack::render_svg_upload(bytes, SVG_SIDE)
            .map_err(|e| CommonError::invalid("image", e));
    }
    let invalid = |e| CommonError::invalid("image", e);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(invalid)?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
    ) {
        return Err(CommonError::invalid(
            "image",
            "Upload a PNG, JPEG or SVG image.",
        ));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    reader
        .decode()
        .map(|i| i.to_rgba8())
        .map_err(|e| CommonError::invalid("image", e))
}
