//! Negotiated USB identity. Legacy hello remains available for v1 callers.
use crate::{LinkError, Session};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use td_core::{Hello, ReplyError, Request};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimationCapabilities {
    pub image: [u32; 2],
    pub min_delay_ms: u32,
    pub page_tiles: u32,
    pub page_pixels: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<AnimationCapabilities>,
    /// The radio build's Bluetooth keyboard: the most bonded hosts, 1 to 8. Absent means no keyboard and no `ble.*`
    /// messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ble_hid: Option<u32>,
    pub screen: [u32; 2],
    pub grid: [u32; 2],
    pub pages: u32,
    pub tiles: u32,
    pub layout_bytes: u32,
    pub layout_schemas: Vec<u32>,
    pub free_bytes: u32,
    pub tile_kinds: Vec<String>,
    pub asset_bytes: u32,
    pub formats: Vec<String>,
    pub image: [u32; 2],
    pub gif_frames: u32,
    pub gif_pixels: u32,
    pub photo_bytes: u32,
    pub live_hz: u32,
    pub live_total_hz: u32,
    pub series: u32,
    pub text_bytes: u32,
    pub streams: u32,
    /// The most Tiles in a Layout that declare an `image` stream. Absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_images: Option<u32>,
}
impl Capabilities {
    pub fn gif_enabled(&self) -> bool {
        self.anim.as_ref().is_some_and(|a| {
            a.image.iter().all(|n| *n > 0)
                && a.min_delay_ms > 0
                && a.page_tiles > 0
                && a.page_pixels > 0
        }) && self.formats.iter().any(|s| s == "gif")
            && self.tile_kinds.iter().any(|s| s == "gif")
            && self.gif_frames > 0
            && self.gif_pixels > 0
    }

    fn valid(&self) -> bool {
        self.screen.iter().all(|n| (1..=4096).contains(n))
            && self.grid.iter().all(|n| (1..=8).contains(n))
            && self.pages <= 8
            && self.tiles <= 64
            && self.layout_bytes <= 32768
            && self.layout_schemas.contains(&1)
            && self.tile_kinds.iter().any(|s| s == "static")
            && self.formats.iter().any(|s| s == "tdi1")
            && (1..=1048576).contains(&self.asset_bytes)
            && self.image[0] <= self.screen[0]
            && self.image[1] <= self.screen[1]
            && self.photo_bytes <= 131072
            && self.live_hz <= 2
            && self.live_total_hz <= 20
            && self.series <= 32
            && self.text_bytes <= 128
            && self.streams <= 8
            && self.live_images.is_none_or(|n| n <= 64)
            && self.ble_hid.is_none_or(|n| n <= crate::ble::BONDS_MAX)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: String,
    pub capabilities: Option<Capabilities>,
    pub protocol: u32,
}
/// The format characters (General_Category Cf) of Unicode 17.0 as inclusive ranges, pinned by
/// link-v2.md and net-v2.md so no toolchain's Unicode version changes the rule.
const FORMAT_RANGES: [(u32, u32); 21] = [
    (0x00ad, 0x00ad),
    (0x0600, 0x0605),
    (0x061c, 0x061c),
    (0x06dd, 0x06dd),
    (0x070f, 0x070f),
    (0x0890, 0x0891),
    (0x08e2, 0x08e2),
    (0x180e, 0x180e),
    (0x200b, 0x200f),
    (0x202a, 0x202e),
    (0x2060, 0x2064),
    (0x2066, 0x206f),
    (0xfeff, 0xfeff),
    (0xfff9, 0xfffb),
    (0x110bd, 0x110bd),
    (0x110cd, 0x110cd),
    (0x13430, 0x1343f),
    (0x1bca0, 0x1bca3),
    (0x1d173, 0x1d17a),
    (0xe0001, 0xe0001),
    (0xe0020, 0xe007f),
];
/// Letters (Lo) that render as blank space.
const HANGUL_FILLERS: [u32; 4] = [0x115f, 0x1160, 0x3164, 0xffa0];
fn hides_text(c: char) -> bool {
    let code = c as u32;
    code <= 0x1f
        || code == 0x7f
        || HANGUL_FILLERS.contains(&code)
        || FORMAT_RANGES
            .iter()
            .any(|&(first, last)| (first..=last).contains(&code))
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name.chars().any(|c| !c.is_whitespace())
        && !name.chars().any(hides_text)
}
fn invalid(message: &str) -> LinkError {
    LinkError::Reply(ReplyError::Fields(message.into()))
}
impl Identity {
    pub fn matches_usb_serial(&self, serial: &str) -> bool {
        self.id
            .as_ref()
            .is_some_and(|id| *id == serial.replace([':', '-'], "").to_ascii_lowercase())
    }
}
impl Session {
    /// Discover v1 first; select v2 only when explicitly advertised.
    pub fn identity(&self, timeout: Duration) -> Result<Identity, LinkError> {
        let reply = self.request(&Request::Hello, timeout)?;
        let hello: Hello = reply.fields().map_err(LinkError::Reply)?;
        if !hello.is_deck() {
            return Err(invalid("not a v1 discovery reply"));
        }
        if !reply
            .fields
            .get("protocols")
            .and_then(|v| v.as_array())
            .is_some_and(|v| v.contains(&json!(2)))
        {
            return Ok(Identity {
                id: None,
                name: None,
                version: hello.version,
                capabilities: None,
                protocol: 1,
            });
        }
        let selected = self.call(&json!({"m":"hello", "proto":2}), timeout)?;
        let result = (|| {
            let hello: Hello = selected.fields().map_err(LinkError::Reply)?;
            if hello.product != "touchdeck" || hello.proto != 2 || hello.version.len() > 32 {
                return Err(invalid("invalid v2 hello"));
            }
            #[derive(Deserialize)]
            struct Fields {
                id: String,
                name: String,
                caps: Capabilities,
                protocols: Vec<u32>,
            }
            let fields: Fields = selected.fields().map_err(LinkError::Reply)?;
            if fields.id.len() != 12
                || !fields
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !valid_name(&fields.name)
                || !fields.caps.valid()
                || !fields.protocols.contains(&2)
                || !fields.protocols.contains(&1)
            {
                return Err(invalid("invalid v2 identity or capabilities"));
            }
            Ok(Identity {
                id: Some(fields.id),
                name: Some(fields.name),
                version: hello.version,
                capabilities: Some(fields.caps),
                protocol: 2,
            })
        })();
        if result.is_err() {
            let _ = self.hello(timeout);
        }
        result
    }
    pub fn name(&self, timeout: Duration) -> Result<String, LinkError> {
        if self.identity(timeout)?.protocol != 2 {
            return Err(invalid("Device does not support names"));
        }
        self.name_request(json!({"m":"v2.name.get"}), timeout)
    }
    pub fn set_name(&self, name: &str, timeout: Duration) -> Result<String, LinkError> {
        if !valid_name(name) {
            return Err(invalid("Invalid name"));
        }
        if self.identity(timeout)?.protocol != 2 {
            return Err(invalid("Device does not support names"));
        }
        self.name_request(json!({"m":"v2.name.set", "name":name}), timeout)
    }
    fn name_request(
        &self,
        value: serde_json::Value,
        timeout: Duration,
    ) -> Result<String, LinkError> {
        #[derive(Deserialize)]
        struct Named {
            name: String,
        }
        let named: Named = self
            .call(&value, timeout)?
            .fields()
            .map_err(LinkError::Reply)?;
        if !valid_name(&named.name) {
            return Err(invalid("Invalid stored name"));
        }
        Ok(named.name)
    }
}
