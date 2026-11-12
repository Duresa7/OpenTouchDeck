//! Coordinated Device settings: validation, actor work, Layout authority and PC photo state.
use super::DeviceError;
use super::{DeviceId, DeviceSummary, Operations};
use crate::api::CommonError;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use td_core::{Request, Settings};
use td_link::{
    LinkError,
    photo::{self, IdleSettings},
};
use ts_rs::TS;

const SETTINGS_TIMEOUT: Duration = Duration::from_secs(2);
const BRIGHTNESS_MAX: u8 = 100;

/// The Device's settings. v2 fields are `null` on v1 Firmware.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DeviceSettings {
    pub idle_timeout_s: u32,
    /// `landscape` or `portrait`.
    pub orientation: String,
    pub name: Option<String>,
    /// A percentage, 0 to 100.
    pub brightness: Option<u8>,
    /// `null` when the Firmware has no photo frame.
    pub photo_frame: Option<PhotoFrameSettings>,
}
/// What the Device shows after `timeout_s` with no touch: pictures from `folder` on this PC, a
/// new one every `interval_s`, or its clock when no picture comes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PhotoFrameSettings {
    pub enabled: bool,
    /// 10 to 86400.
    pub timeout_s: u32,
    /// 5 to 3600.
    pub interval_s: u32,
    /// An absolute path to a folder on this PC, `null` until one is chosen.
    pub folder: Option<String>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, TS)]
pub struct PhotoFramePatch {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub timeout_s: Option<u32>,
    #[serde(default)]
    pub interval_s: Option<u32>,
    /// An absolute path to an existing folder; `""` clears it.
    #[serde(default)]
    pub folder: Option<String>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, TS)]
pub struct DeviceSettingsPatch {
    #[serde(default)]
    pub idle_timeout_s: Option<u32>,
    #[serde(default)]
    pub orientation: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// A percentage, 0 to 100.
    #[serde(default)]
    pub brightness: Option<u8>,
    #[serde(default)]
    pub photo_frame: Option<PhotoFramePatch>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SettingsPatched {
    pub settings: DeviceSettings,
    /// The Device must restart before an orientation change shows.
    pub restart_required: bool,
}

/// Acknowledged changes, in execution order. A later failure never rolls these back on the Device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsField {
    IdleTimeout,
    Orientation,
    Name,
    PhotoFrame,
    PhotoFolder,
}

/// The latest acknowledged settings and any failure after the initial read.
/// An error does not imply rollback: inspect `applied` and `restart_required` before retrying.
#[derive(Debug)]
pub struct SettingsOutcome {
    pub settings: DeviceSettings,
    pub restart_required: bool,
    pub applied: Vec<SettingsField>,
    /// Requested fields that were not acknowledged; roll drafts back to `settings` before retrying.
    pub pending: Vec<SettingsField>,
    pub error: Option<DeviceError>,
}
fn orientation_name(settings: &Settings) -> String {
    serde_json::to_value(settings.orientation)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
/// The v2 fields the Device's Firmware supports: its name, if it speaks protocol 2.
fn name_if_v2(summary: &DeviceSummary) -> Option<String> {
    let v2 = summary.firmware.as_ref().is_some_and(|f| f.protocol >= 2);
    v2.then(|| summary.name.clone()).flatten()
}
fn has_photo_frame(summary: &DeviceSummary) -> bool {
    crate::photos::supports(summary.capabilities.as_ref()).0
}
fn photo_frame(
    ctx: &Operations,
    id: &DeviceId,
    idle: Option<IdleSettings>,
) -> Option<PhotoFrameSettings> {
    let idle = idle?;
    ctx.photos.idle_changed(id, idle);
    Some(PhotoFrameSettings {
        enabled: idle.enabled,
        timeout_s: idle.timeout_s,
        interval_s: idle.interval_s,
        folder: ctx.photos.folder(id),
    })
}
fn idle_patch(current: IdleSettings, patch: &PhotoFramePatch) -> IdleSettings {
    IdleSettings {
        enabled: patch.enabled.unwrap_or(current.enabled),
        timeout_s: patch.timeout_s.unwrap_or(current.timeout_s),
        interval_s: patch.interval_s.unwrap_or(current.interval_s),
    }
}
impl Operations {
    pub async fn settings_get(&self, id: &DeviceId) -> Result<DeviceSettings, DeviceError> {
        let _guard = self.settings_guard(id).await;
        let ctx = self;
        let id = id.clone();
        let summary = ctx.devices.get(&id).ok_or(DeviceError::NotFound)?;
        let photos = has_photo_frame(&summary);
        let (settings, idle) = ctx
            .devices
            .call(&id, move |s| -> Result<_, DeviceError> {
                let settings: Settings = s
                    .request(&Request::SettingsGet, SETTINGS_TIMEOUT)?
                    .fields()
                    .map_err(LinkError::Reply)?;
                let idle = photos.then(|| photo::idle_get(s)).transpose()?;
                Ok((settings, idle))
            })
            .await??;
        let summary = ctx.devices.get(&id).ok_or(DeviceError::NotFound)?;
        Ok(DeviceSettings {
            idle_timeout_s: settings.idle_timeout_s,
            orientation: orientation_name(&settings),
            name: name_if_v2(&summary),
            brightness: None,
            photo_frame: photo_frame(ctx, &id, idle),
        })
    }
    pub async fn settings_patch(
        &self,
        id: &DeviceId,
        patch: DeviceSettingsPatch,
    ) -> Result<SettingsOutcome, DeviceError> {
        let _guard = self.settings_guard(id).await;
        let ctx = self;
        let id = id.clone();
        let invalid = |reason| DeviceError::from(CommonError::invalid("body", reason));
        let orientation = match patch.orientation.as_deref() {
            None => None,
            Some("landscape") => Some(td_core::Orientation::Landscape),
            Some("portrait") => Some(td_core::Orientation::Portrait),
            Some(_) => return Err(invalid("orientation is landscape or portrait")),
        };
        if patch.idle_timeout_s.is_some_and(|t| {
            !(td_core::settings::IDLE_TIMEOUT_MIN_S..=td_core::settings::IDLE_TIMEOUT_MAX_S)
                .contains(&t)
        }) {
            return Err(invalid("idle_timeout_s is out of range"));
        }
        if patch
            .name
            .as_deref()
            .is_some_and(|n| !td_link::identity::valid_name(n))
        {
            return Err(invalid(
                "name is empty, too long or has control, format or filler characters",
            ));
        }
        if patch.brightness.is_some_and(|b| b > BRIGHTNESS_MAX) {
            return Err(invalid("brightness is a percentage from 0 to 100"));
        }
        // No Firmware reports brightness yet.
        if patch.brightness.is_some() {
            return Err(DeviceError::NeedsFirmware {
                field: "brightness",
            });
        }
        let frame = patch.photo_frame.clone();
        if let Some(frame) = &frame {
            if frame
                .timeout_s
                .is_some_and(|t| !photo::IDLE_TIMEOUT_S.contains(&t))
            {
                return Err(invalid("photo_frame.timeout_s is 10 to 86400"));
            }
            if frame
                .interval_s
                .is_some_and(|t| !photo::IDLE_INTERVAL_S.contains(&t))
            {
                return Err(invalid("photo_frame.interval_s is 5 to 3600"));
            }
            if frame
                .folder
                .as_deref()
                .is_some_and(|f| !f.is_empty() && !crate::photos::folder::valid(f))
            {
                return Err(invalid(
                    "photo_frame.folder is not the full path of a folder on this computer",
                ));
            }
        }
        let summary = ctx.devices.get(&id).ok_or(DeviceError::NotFound)?;
        if patch.name.is_some() && summary.firmware.as_ref().is_some_and(|f| f.protocol < 2) {
            return Err(DeviceError::NeedsFirmware { field: "name" });
        }
        let photos = has_photo_frame(&summary);
        if frame.is_some() && !photos {
            return Err(DeviceError::NeedsFirmware {
                field: "photo_frame",
            });
        }
        let mut requested = Vec::new();
        if patch.idle_timeout_s.is_some() {
            requested.push(SettingsField::IdleTimeout);
        }
        if orientation.is_some() {
            requested.push(SettingsField::Orientation);
        }
        if patch.name.is_some() {
            requested.push(SettingsField::Name);
        }
        if frame.is_some() {
            requested.push(SettingsField::PhotoFrame);
        }
        if frame.as_ref().is_some_and(|f| f.folder.is_some()) {
            requested.push(SettingsField::PhotoFolder);
        }
        let devices = ctx.devices.clone();
        let device = id.clone();
        let idle_frame = frame.clone();
        let authority = ctx.authority.clone();
        #[cfg(any(test, feature = "test-hooks"))]
        let checkpoints = ctx.checkpoints.clone();
        let (settings, restart_required, idle, mut applied, mut error) = ctx
            .devices
            .call(&id, move |s| -> Result<_, DeviceError> {
                let mut settings: Settings = s
                    .request(&Request::SettingsGet, SETTINGS_TIMEOUT)?
                    .fields()
                    .map_err(LinkError::Reply)?;
                let mut restart = false;
                let mut applied = Vec::new();
                let mut idle = None;
                let result = (|| -> Result<(), DeviceError> {
                    if patch.idle_timeout_s.is_some() || orientation.is_some() {
                        let proposed = Settings {
                            idle_timeout_s: patch.idle_timeout_s.unwrap_or(settings.idle_timeout_s),
                            orientation: orientation.unwrap_or(settings.orientation),
                        };
                        let mut reply = None;
                        let mut write = || {
                            #[cfg(any(test, feature = "test-hooks"))]
                            if orientation.is_some() {
                                let _ = checkpoints
                                    .pass(crate::checkpoints::Checkpoint::BeforeOrientationPut);
                            }
                            reply =
                                Some(s.request(&Request::SettingsPut(proposed), SETTINGS_TIMEOUT));
                            Ok::<_, LinkError>(None)
                        };
                        if orientation.is_some() {
                            authority.replace(device.as_str(), write).map_err(
                                |error| match error {
                                    crate::layouts::authority::ReplaceError::Write(error) => {
                                        DeviceError::from(error)
                                    }
                                    crate::layouts::authority::ReplaceError::Save(_) => {
                                        DeviceError::from(CommonError::Internal)
                                    }
                                },
                            )?;
                        } else {
                            write()?;
                        }
                        let reply = reply.unwrap()?;
                        let fields = serde_json::Value::Object(reply.fields.clone());
                        restart = fields["reboot_required"].as_bool().unwrap_or(false);
                        settings = Settings::from_value(&fields);
                        if patch.idle_timeout_s.is_some() {
                            applied.push(SettingsField::IdleTimeout);
                        }
                        if orientation.is_some() {
                            applied.push(SettingsField::Orientation);
                        }
                        #[cfg(any(test, feature = "test-hooks"))]
                        if orientation.is_some() {
                            checkpoints
                                .pass(crate::checkpoints::Checkpoint::AfterOrientationPut)
                                .map_err(|()| LinkError::Timeout(SETTINGS_TIMEOUT))?;
                        }
                    }
                    if let Some(name) = patch.name {
                        let name = s.set_name(&name, SETTINGS_TIMEOUT)?;
                        devices.named(&device, name);
                        applied.push(SettingsField::Name);
                    }
                    idle = match (photos, idle_frame) {
                        (false, _) => None,
                        (true, None) => Some(photo::idle_get(s)?),
                        (true, Some(frame)) => {
                            let idle = idle_patch(photo::idle_get(s)?, &frame);
                            let saved = photo::idle_set(s, idle)?;
                            applied.push(SettingsField::PhotoFrame);
                            Some(saved)
                        }
                    };
                    Ok(())
                })();
                Ok((settings, restart, idle, applied, result.err()))
            })
            .await??;
        if error.is_none()
            && let Some(folder) = frame.and_then(|f| f.folder)
        {
            let photos = ctx.photos.clone();
            let device = id.clone();
            let saved = tokio::task::spawn_blocking(move || {
                photos.set_folder(&device, (!folder.is_empty()).then_some(folder))
            })
            .await
            .map_err(|_| CommonError::Internal)
            .and_then(|r| r.map_err(|_| CommonError::Internal));
            match saved {
                Ok(()) => applied.push(SettingsField::PhotoFolder),
                Err(failure) => error = Some(failure.into()),
            }
        }
        let summary = ctx.devices.get(&id).ok_or(DeviceError::NotFound)?;
        Ok(SettingsOutcome {
            pending: requested
                .into_iter()
                .filter(|field| !applied.contains(field))
                .collect(),
            applied,
            error,
            settings: DeviceSettings {
                idle_timeout_s: settings.idle_timeout_s,
                orientation: orientation_name(&settings),
                name: name_if_v2(&summary),
                brightness: None,
                photo_frame: photo_frame(ctx, &id, idle),
            },
            restart_required,
        })
    }
}
