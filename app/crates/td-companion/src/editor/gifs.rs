use super::{AddTarget, EditorError, Session};
use crate::{
    api::{ApiError, ErrorBody},
    features::assets::{self, Fit, GifTarget},
    jobs::{Job, JobState},
};
use std::{collections::BTreeMap, sync::Arc};
use td_core::{
    Orientation,
    editing::{Appearance, Edit, EditError},
    layout::{Action, Page, TileRender},
    live::ScreenSize,
};

#[derive(Clone)]
pub(crate) struct GifSource {
    bytes: Arc<Vec<u8>>,
    file: String,
    fit: Fit,
    aim: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifProblem {
    pub text: String,
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GifWork {
    Queued {
        file: String,
    },
    Preparing {
        file: String,
    },
    Failed {
        file: String,
        problem: GifProblem,
        retry: bool,
    },
}
pub enum GifEdit {
    Choose { file: String, bytes: Vec<u8> },
    Cancel,
    Retry,
    Fit(Fit),
}
struct Run {
    source: GifSource,
    fit: Fit,
    aim: String,
    job: Option<String>,
    work: GifWork,
}
#[derive(Default)]
pub(super) struct GifState {
    work: BTreeMap<(u8, String), Run>,
    prepared: BTreeMap<(u8, String), (String, String, Fit)>,
}
fn key(orientation: Orientation, tile: &str) -> (u8, String) {
    (
        if orientation == Orientation::Landscape {
            0
        } else {
            1
        },
        tile.into(),
    )
}
fn orientation(key: &(u8, String)) -> Orientation {
    if key.0 == 0 {
        Orientation::Landscape
    } else {
        Orientation::Portrait
    }
}

impl Session {
    pub fn offers_gif(&self) -> bool {
        assets::gif_caps(&self.handle, &self.device).is_ok()
    }
    pub fn gif_target(&self, tile: &str) -> Option<GifTarget> {
        self.gif_target_at(self.orientation, tile)
    }
    fn gif_target_at(&self, orientation: Orientation, tile: &str) -> Option<GifTarget> {
        let device = self
            .handle
            .devices()
            .get(&crate::devices::DeviceId::parse(&self.device)?)?;
        let caps = device.capabilities?;
        let screen = ScreenSize {
            w: caps.screen[0].try_into().ok()?,
            h: caps.screen[1].try_into().ok()?,
        }
        .oriented(orientation);
        let layout = self.model.draft(orientation)?.layout();
        let rect = layout
            .pages
            .iter()
            .find_map(|p| td_core::live::tile_rect(layout, p.id(), tile, screen))?;
        (rect.w > 0 && rect.h > 0).then_some(GifTarget {
            width: rect.w,
            height: rect.h,
            background: [24, 32, 49],
        })
    }
    fn gif_aim(
        &self,
        orientation: Orientation,
        tile: &str,
        fit: Fit,
    ) -> Option<(String, GifTarget)> {
        let caps = assets::gif_caps(&self.handle, &self.device).ok()?;
        let target = self.gif_target_at(orientation, tile)?;
        let aim = serde_json::to_string(&(
            &self.device,
            &caps.anim,
            caps.gif_frames,
            caps.gif_pixels,
            caps.asset_bytes,
            &target,
            fit,
        ))
        .ok()?;
        Some((aim, target))
    }
    pub fn gif_work(&self, tile: &str) -> Option<&GifWork> {
        self.gifs
            .work
            .get(&key(self.orientation, tile))
            .map(|r| &r.work)
    }
    pub fn gif_fit(&self, tile: &str) -> Fit {
        if let Some(run) = self.gifs.work.get(&key(self.orientation, tile)) {
            return run.fit;
        }
        self.draft()
            .and_then(|d| d.effective_tile(tile))
            .and_then(|t| t.asset)
            .and_then(|a| {
                self.handle
                    .editor_gif_sources
                    .lock()
                    .get(a.trim_start_matches("sha256:"))
                    .map(|s| s.fit)
            })
            .unwrap_or_default()
    }
    pub fn add_gif(
        &mut self,
        file: String,
        bytes: Vec<u8>,
        target: AddTarget,
    ) -> Result<String, EditorError> {
        let id = self.place(target, "GIF".into(), Action::None, None, None)?;
        self.gif(&id, GifEdit::Choose { file, bytes })?;
        Ok(id)
    }
    pub fn gif(&mut self, tile: &str, edit: GifEdit) -> Result<(), EditorError> {
        let key = key(self.orientation, tile);
        match edit {
            GifEdit::Cancel => {
                self.cancel_gif(&key);
            }
            GifEdit::Choose { file, bytes } => {
                let fit = self.gif_fit(tile);
                self.start_gif(
                    key,
                    GifSource {
                        bytes: Arc::new(bytes),
                        file,
                        fit,
                        aim: String::new(),
                    },
                    fit,
                )?;
            }
            GifEdit::Retry => {
                if let Some(run) = self.gifs.work.get(&key)
                    && matches!(run.work, GifWork::Failed { retry: true, .. })
                {
                    let (source, fit) = (run.source.clone(), run.fit);
                    self.start_gif(key, source, fit)?;
                }
            }
            GifEdit::Fit(fit) => {
                if let Some(run) = self.gifs.work.get(&key) {
                    if run.fit != fit {
                        let source = run.source.clone();
                        self.start_gif(key, source, fit)?;
                    }
                } else {
                    let source = self
                        .draft()
                        .and_then(|d| d.effective_tile(tile))
                        .filter(|t| t.render == Some(TileRender::Gif))
                        .and_then(|t| t.asset)
                        .and_then(|a| {
                            self.handle
                                .editor_gif_sources
                                .lock()
                                .get(a.trim_start_matches("sha256:"))
                                .cloned()
                        });
                    if let Some(source) = source {
                        if source.fit != fit {
                            self.start_gif(key, source, fit)?;
                        }
                    } else {
                        self.gifs.work.insert(
                            key,
                            Run {
                                source: GifSource {
                                    bytes: Arc::new(vec![]),
                                    file: String::new(),
                                    fit,
                                    aim: String::new(),
                                },
                                fit,
                                aim: String::new(),
                                job: None,
                                work: GifWork::Failed {
                                    file: String::new(),
                                    problem: GifProblem {
                                        text: "Choose this GIF again to refit it.".into(),
                                        detail: String::new(),
                                    },
                                    retry: false,
                                },
                            },
                        );
                    }
                }
            }
        }
        Ok(())
    }
    fn cancel_gif(&mut self, key: &(u8, String)) {
        if let Some(run) = self.gifs.work.remove(key)
            && let Some(job) = run.job
        {
            let _ = self.handle.jobs().cancel(&job);
        }
    }
    fn start_gif(
        &mut self,
        key: (u8, String),
        source: GifSource,
        fit: Fit,
    ) -> Result<(), EditorError> {
        let o = orientation(&key);
        if self
            .model
            .draft(o)
            .and_then(|d| d.effective_tile(&key.1))
            .is_none()
        {
            return Err(EditError::MissingTile(key.1).into());
        }
        self.cancel_gif(&key);
        let aimed = self.gif_aim(o, &key.1, fit);
        if source.bytes.len() <= 16 * 1024 * 1024
            && let Some((aim, _)) = &aimed
            && self.gifs.work.values().any(|r| r.job.is_some())
        {
            self.gifs.work.insert(
                key,
                Run {
                    fit,
                    aim: aim.clone(),
                    job: None,
                    work: GifWork::Queued {
                        file: source.file.clone(),
                    },
                    source,
                },
            );
            return Ok(());
        }
        let result = if source.bytes.len() > 16 * 1024 * 1024 {
            Err(ErrorBody {
                code: "request.too_large".into(),
                message: "The GIF is larger than 16 MiB.".into(),
                details: Some(
                    serde_json::json!({"limit":16*1024*1024,"actual":source.bytes.len(),"unit":"bytes"}),
                ),
            })
        } else if let Some((_, target)) = &aimed {
            assets::start_gif(
                &self.handle,
                self.device.clone(),
                source.bytes.clone(),
                target.clone(),
                fit,
            )
            .map_err(|e| e.body())
        } else {
            Err(assets::unsupported("anim", "enabled", "disabled", "unsupported").body())
        };
        let (job, work) = match result {
            Ok(job) => (Some(job.id.clone()), work_for(&job, &source.file)),
            Err(error) if error.code == "limits.exceeded" => (
                None,
                GifWork::Queued {
                    file: source.file.clone(),
                },
            ),
            Err(error) => (
                None,
                GifWork::Failed {
                    file: source.file.clone(),
                    problem: self
                        .gif_problem(&error, false)
                        .expect("preparation always has a message"),
                    retry: true,
                },
            ),
        };
        self.gifs.work.insert(
            key,
            Run {
                source,
                fit,
                aim: aimed.map(|a| a.0).unwrap_or_default(),
                job,
                work,
            },
        );
        Ok(())
    }
    pub(super) fn cancel_gifs(&mut self) {
        for key in self.gifs.work.keys().cloned().collect::<Vec<_>>() {
            self.cancel_gif(&key);
        }
    }
    pub(super) fn sync_gifs(&mut self) -> Result<(), EditorError> {
        let keys = self.gifs.work.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let o = orientation(&key);
            if self
                .model
                .draft(o)
                .and_then(|d| d.effective_tile(&key.1))
                .is_none()
            {
                self.cancel_gif(&key);
                continue;
            }
            let run = &self.gifs.work[&key];
            let Some(id) = &run.job else {
                if matches!(run.work, GifWork::Queued { .. }) {
                    let (source, fit) = (run.source.clone(), run.fit);
                    self.start_gif(key, source, fit)?;
                }
                continue;
            };
            let aim = self.gif_aim(o, &key.1, run.fit);
            if aim.as_ref().map(|a| &a.0) != Some(&run.aim) {
                let (source, fit) = (run.source.clone(), run.fit);
                self.start_gif(key, source, fit)?;
                continue;
            }
            let Some(job) = self.handle.jobs().get(id) else {
                continue;
            };
            if !job.state.finished() {
                self.gifs.work.get_mut(&key).unwrap().work = work_for(&job, &run.source.file);
                continue;
            }
            let mut run = self.gifs.work.remove(&key).unwrap();
            if job.state == JobState::Succeeded {
                let asset: assets::AssetRef =
                    serde_json::from_value(job.result.unwrap_or_default())
                        .map_err(|e| EditorError::Io(std::io::Error::other(e)))?;
                self.model.edit(
                    o,
                    Edit::Renderer {
                        tile: key.1.clone(),
                        appearance: Appearance::Gif {
                            asset: format!("sha256:{}", asset.id),
                        },
                    },
                )?;
                self.gifs.prepared.insert(
                    key.clone(),
                    (format!("sha256:{}", asset.id), run.aim.clone(), run.fit),
                );
                run.source.fit = run.fit;
                run.source.aim = run.aim;
                self.handle
                    .editor_gif_sources
                    .lock()
                    .insert(asset.id, run.source);
                self.model.persist()?;
            } else if job.state != JobState::Cancelled {
                let error = job.error.unwrap_or(ErrorBody {
                    code: "asset.render_failed".into(),
                    message: "Preparation failed".into(),
                    details: None,
                });
                run.job = None;
                run.work = GifWork::Failed {
                    file: run.source.file.clone(),
                    problem: self.gif_problem(&error, false).unwrap(),
                    retry: true,
                };
                self.gifs.work.insert(key, run);
            }
        }
        // A completed GIF keeps its source for resizing, importing on another Device, or new limits.
        let mut refits = Vec::new();
        for o in [Orientation::Landscape, Orientation::Portrait] {
            let Some(draft) = self.model.draft(o) else {
                continue;
            };
            for page in &draft.layout().pages {
                if let Page::Grid { tiles, .. } = page {
                    for tile in tiles {
                        if tile.render != Some(TileRender::Gif)
                            || self.gifs.work.contains_key(&key(o, &tile.id))
                        {
                            continue;
                        }
                        let source = tile.asset.as_ref().and_then(|a| {
                            self.handle
                                .editor_gif_sources
                                .lock()
                                .get(a.trim_start_matches("sha256:"))
                                .cloned()
                        });
                        if let Some(source) = source
                            && let Some((aim, _)) = self.gif_aim(o, &tile.id, source.fit)
                            && aim
                                != self
                                    .gifs
                                    .prepared
                                    .get(&key(o, &tile.id))
                                    .filter(|(asset, _, _)| tile.asset.as_ref() == Some(asset))
                                    .map(|(_, aim, _)| aim)
                                    .unwrap_or(&source.aim)
                                    .as_str()
                        {
                            refits.push((key(o, &tile.id), source));
                        }
                    }
                }
            }
        }
        for (key, source) in refits {
            let fit = source.fit;
            self.start_gif(key, source, fit)?;
        }
        if !self.gifs.work.values().any(|r| r.job.is_some()) {
            let next = self
                .gifs
                .work
                .iter()
                .find(|(_, r)| matches!(r.work, GifWork::Queued { .. }))
                .map(|(key, r)| (key.clone(), r.source.clone(), r.fit));
            if let Some((key, source, fit)) = next {
                self.start_gif(key, source, fit)?;
            }
        }
        Ok(())
    }
    pub(super) fn device_name(&self) -> String {
        self.handle
            .devices()
            .get(&crate::devices::DeviceId::parse(&self.device).unwrap())
            .and_then(|d| d.name)
            .unwrap_or_else(|| "the Device".into())
    }
    /// A preparation refusal always gets a message; a send refusal unrelated to GIFs returns None.
    pub fn gif_problem(&self, error: &ErrorBody, push: bool) -> Option<GifProblem> {
        let device = self.device_name();
        let details = error.details.as_ref();
        let field = details
            .and_then(|d| d.get("field"))
            .and_then(|v| v.as_str());
        let reason = details
            .and_then(|d| d.get("reason"))
            .and_then(|v| v.as_str());
        let large = || format!("This GIF is too large for {device}. Choose a smaller GIF.");
        let text = match error.code.as_str() {
            "asset.too_large" if matches!(field, Some("gif_frames" | "source_frames")) => format!("This GIF is too long for {device}. Choose a shorter GIF."),
            "asset.too_large" if field == Some("decoded_rgba") => "This GIF needs more than 64 MiB to decode all its frames. Choose a smaller or shorter GIF.".into(),
            "asset.too_large" => large(),
            "asset.too_short" => format!("This GIF is too short for {device}. Choose a GIF that plays for at least {} ms.", details.and_then(|d| d.get("limit")).and_then(|v| v.as_u64()).unwrap_or(100)),
            "asset.render_failed" => "Couldn't prepare this GIF. Choose another GIF.".into(),
            "asset.store_full" => "This PC's Asset store is full. Retry in an hour, or remove Tiles you don't use and send the Layout.".into(),
            "asset.unsupported" => format!("{device} can't play this GIF. Update its Firmware, then retry."),
            "layout.animation_limit" => format!("This Page has too many animations for {device}. Remove an animated Tile."),
            "asset.refused" if reason == Some("full") => format!("{device} is out of space. Remove unused Assets, then retry."),
            "asset.refused" if reason == Some("memory") => large(),
            _ if push => return None,
            "request.too_large" => large(),
            "client.unreachable" => "Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry.".into(),
            "limits.exceeded" => "Other Assets are still being prepared. Wait a moment, then retry.".into(),
            "device.not_connected" | "device.not_found" => format!("Couldn't prepare this GIF. Check that {device} is plugged in, then retry."),
            _ => "Couldn't prepare this GIF. Choose another GIF.".into(),
        };
        Some(GifProblem {
            text,
            detail: format!(
                "{}: {}{}",
                error.code,
                error.message,
                details.map(|d| format!(" {d}")).unwrap_or_default()
            ),
        })
    }
    pub fn inspect_gif(&self, bytes: &[u8]) -> Option<GifPreview> {
        inspect(bytes)
    }
    pub fn fitted_text(&self, info: &GifPreview) -> String {
        let fps = if info.frames > 1 && info.duration_ms > 0 {
            let fps = (f64::from(info.frames) * 10000.0 / info.duration_ms as f64).round() / 10.0;
            format!(", {fps} fps")
        } else {
            String::new()
        };
        format!(
            "Fitted to {}: {} × {}{fps}.",
            self.device_name(),
            info.width,
            info.height
        )
    }
}
fn work_for(job: &Job, file: &str) -> GifWork {
    if job.state == JobState::Queued {
        GifWork::Queued { file: file.into() }
    } else {
        GifWork::Preparing { file: file.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifPreview {
    pub width: u16,
    pub height: u16,
    pub frames: u32,
    pub duration_ms: u64,
    pub poster: Vec<u8>,
}
fn inspect(b: &[u8]) -> Option<GifPreview> {
    if b.len() < 13 || ![b"GIF89a".as_slice(), b"GIF87a".as_slice()].contains(&&b[..6]) {
        return None;
    }
    let table = |packed: u8| {
        if packed & 128 != 0 {
            3 * (1usize << ((packed & 7) + 1))
        } else {
            0
        }
    };
    let u16_at = |at| Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]));
    let skip = |mut at: usize| -> Option<usize> {
        loop {
            let n = usize::from(*b.get(at)?);
            at = at.checked_add(1 + n)?;
            if at > b.len() {
                return None;
            }
            if n == 0 {
                return Some(at);
            }
        }
    };
    let head = 13 + table(b[10]);
    if head > b.len() {
        return None;
    }
    let (mut at, mut frames, mut duration_ms, mut delay) = (head, 0, 0, 0u16);
    let (mut control, mut first, mut first_control) = (None, None, None);
    while at < b.len() {
        match b[at] {
            0x3b => {
                let (start, end) = first?;
                let mut poster = b[..head].to_vec();
                if let Some((start, end)) = first_control {
                    poster.extend_from_slice(&b[start..end]);
                }
                poster.extend_from_slice(&b[start..end]);
                poster.push(0x3b);
                return Some(GifPreview {
                    width: u16_at(6)?,
                    height: u16_at(8)?,
                    frames,
                    duration_ms,
                    poster,
                });
            }
            0x21 => {
                let kind = *b.get(at + 1)?;
                let end = skip(at + 2)?;
                if kind == 0xf9 && *b.get(at + 2)? >= 4 {
                    delay = u16_at(at + 4)?;
                    control = Some((at, end));
                }
                at = end;
            }
            0x2c => {
                let data = at + 10 + table(*b.get(at + 9)?);
                let end = skip(data + 1)?;
                if first.is_none() {
                    first = Some((at, end));
                    first_control = control;
                }
                frames += 1;
                duration_ms += u64::from(delay) * 10;
                delay = 0;
                control = None;
                at = end;
            }
            _ => return None,
        }
    }
    None
}
