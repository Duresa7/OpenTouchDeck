//! Routes for the e2e harness. Compiled only with `test-hooks`, and they answer only when the
//! Companion runs on mock ports, or for `test.console` when it was started with
//! `--test-real-device-console`; otherwise every one is `route.not_found`.
use super::devices::{DeviceError, DevicePath};
use crate::{
    api::{CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req, Topic},
    devices::{BoardKind, DeviceId, MockPorts},
};
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use td_core::{InputPhase, Orientation};
use td_link::MockDeck;
use ts_rs::TS;

fn mock_ports(ctx: &Ctx) -> Result<MockPorts, DeviceError> {
    let (ports, _) = ctx.devices.mock().ok_or(CommonError::NotFound)?;
    Ok(ports.clone())
}
fn mock_deck(ctx: &Ctx, id: &DeviceId) -> Result<MockDeck, DeviceError> {
    mock_ports(ctx)?
        .deck(id.as_str())
        .ok_or(DeviceError::NotFound)
}

const CONSOLE_WAIT: Duration = Duration::from_millis(3000);
const CONSOLE_WAIT_MAX: Duration = Duration::from_millis(15_000);
const CONSOLE_LINES_MAX: usize = 256;
const FIRMWARE_CMD_ECHO_MAX: usize = 63;

#[derive(Serialize, Deserialize, TS)]
pub struct TestConsoleLine {
    pub line: String,
    /// Milliseconds to wait for the `@td` reply: 3000 when absent, 0 to 15000. With 0 the
    /// route returns what arrived while it sent the line.
    #[serde(default)]
    pub wait_ms: Option<u64>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct ConsoleReply {
    /// The `@td` JSON whose `cmd` is the line's first word, or null when none came in time.
    pub reply: Option<Value>,
    /// The Device's console lines from the send until the reply or the deadline, at most 256.
    pub lines: Vec<String>,
}
pub struct Console;
impl Endpoint for Console {
    const NAME: &'static str = "test.console";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test/devices/{device}/console";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = TestConsoleLine;
    type Ok = ConsoleReply;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        if ctx.devices.mock().is_some() {
            mock_deck(&ctx, &req.path.device)?;
        } else if !ctx.real_device_console {
            return Err(CommonError::NotFound.into());
        }
        let wait = req.body.wait_ms.map_or(CONSOLE_WAIT, Duration::from_millis);
        if wait > CONSOLE_WAIT_MAX {
            return Err(CommonError::invalid("wait_ms", "Expected 0 to 15000.").into());
        }
        let device = req.path.device;
        let listener = ctx.web_events.listen(Topic::Console, device.as_str());
        let deadline = tokio::time::Instant::now() + wait;
        let line = req.body.line;
        let word = line
            .split(' ')
            .find(|word| !word.is_empty())
            .unwrap_or_default()
            .to_owned();
        ctx.devices
            .call(&device, move |s| s.send_line(&line))
            .await??;
        let mut lines = Vec::new();
        while let Some(data) = listener.next(deadline).await {
            let Some(text) = data["line"].as_str() else {
                continue;
            };
            if lines.len() < CONSOLE_LINES_MAX {
                lines.push(text.to_owned());
            }
            if let Some(reply) = reply_to(&word, text) {
                return Ok(ConsoleReply {
                    reply: Some(reply),
                    lines,
                });
            }
        }
        Ok(ConsoleReply { reply: None, lines })
    }
}
fn reply_to(word: &str, text: &str) -> Option<Value> {
    let at = text.find("@td ")?;
    let reply: Value = serde_json::from_str(&text[at + 4..]).ok()?;
    let cmd = reply["cmd"].as_str()?;
    (cmd == word || (cmd.len() >= FIRMWARE_CMD_ECHO_MAX && word.starts_with(cmd))).then_some(reply)
}

#[derive(Serialize, Deserialize, TS)]
pub struct MockAction {
    /// `no-confirm`, `rollback`, `normal-update`, `stop` or `hang-up`.
    pub action: String,
}
pub struct Mock;
impl Endpoint for Mock {
    const NAME: &'static str = "test.mock";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test/devices/{device}/mock";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = MockAction;
    type Ok = ();
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let deck = mock_deck(&ctx, &req.path.device)?;
        match req.body.action.as_str() {
            mode @ ("no-confirm" | "rollback" | "normal-update") => deck.update_mode(mode),
            "stop" => deck.stop(),
            "hang-up" => deck.hang_up(),
            _ => return Err(CommonError::invalid("body", "unknown mock action").into()),
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct MockPress {
    pub page: String,
    pub tile: String,
    pub kind: String,
}
pub struct Press;
impl Endpoint for Press {
    const NAME: &'static str = "test.press";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test/devices/{device}/press";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = MockPress;
    type Ok = ();
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let press = req.body;
        mock_deck(&ctx, &req.path.device)?.emit_press(&press.page, &press.tile, &press.kind);
        Ok(())
    }
}

pub struct Sets;
impl Endpoint for Sets {
    const NAME: &'static str = "test.sets";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/test/devices/{device}/sets";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<Value>;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        Ok(mock_deck(&ctx, &req.path.device)?.sets())
    }
}

/// What a mock Device received and stores.
#[derive(Serialize, Deserialize, TS)]
pub struct MockRecord {
    pub received: Vec<Value>,
    pub lines: Vec<String>,
    pub settings: Value,
    pub reboots: u32,
    pub assets: Vec<String>,
    pub layouts: Value,
}
pub struct Record;
impl Endpoint for Record {
    const NAME: &'static str = "test.record";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/test/devices/{device}/record";
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = ();
    type Ok = MockRecord;
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let deck = mock_deck(&ctx, &req.path.device)?;
        let layout = |orientation| {
            deck.layout(orientation)
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        };
        Ok(MockRecord {
            received: deck.received(),
            lines: deck.lines(),
            settings: serde_json::to_value(deck.settings()).unwrap_or_default(),
            reboots: deck.reboots(),
            assets: deck.icons().into_iter().collect(),
            layouts: json!({
                "landscape": layout(Orientation::Landscape),
                "portrait": layout(Orientation::Portrait),
            }),
        })
    }
}

/// Events for a mock Device to send: a `key.report` with `type` and `mods` and `keys` or
/// `usage`, a `key.resync` with `mods` and `keys`, a `press` with `page`, `tile`, `kind` and an
/// optional renderer `part`, a `v2.input` drag with `page`, `tile`, `stream`, `phase` and `value`,
/// or `{"m": "drop", "count": n}`, which loses `n` key reports. The list runs `repeat` times
/// with `every_ms` between entries, on its own thread.
#[derive(Serialize, Deserialize, TS)]
pub struct MockEmit {
    pub events: Vec<Value>,
    #[serde(default)]
    pub repeat: Option<u32>,
    #[serde(default)]
    pub every_ms: Option<u64>,
}
pub struct Emit;
impl Endpoint for Emit {
    const NAME: &'static str = "test.emit";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test/devices/{device}/emit";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = DevicePath;
    type Query = NoQuery;
    type Body = MockEmit;
    type Ok = ();
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let deck = mock_deck(&ctx, &req.path.device)?;
        let MockEmit {
            events,
            repeat,
            every_ms,
        } = req.body;
        let every = Duration::from_millis(every_ms.unwrap_or(0));
        std::thread::spawn(move || {
            for _ in 0..repeat.unwrap_or(1) {
                for event in &events {
                    emit(&deck, event);
                    std::thread::sleep(every);
                }
            }
        });
        Ok(())
    }
}
fn emit(deck: &MockDeck, event: &Value) {
    let text = |name: &str| event.get(name).and_then(Value::as_str).unwrap_or_default();
    let number = |name: &str| event.get(name).and_then(Value::as_u64).unwrap_or(0);
    match text("m") {
        "key.report" => {
            deck.emit_key(event);
        }
        "key.resync" => {
            let keys: Vec<u8> = event
                .get("keys")
                .and_then(Value::as_array)
                .map(|keys| {
                    keys.iter()
                        .filter_map(Value::as_u64)
                        .map(|k| k as u8)
                        .collect()
                })
                .unwrap_or_default();
            deck.emit_resync(number("mods") as u8, &keys);
        }
        "press" if event.get("part").is_some() => {
            deck.emit_press_part(text("page"), text("tile"), text("kind"), text("part"));
        }
        "press" => {
            deck.emit_press(text("page"), text("tile"), text("kind"));
        }
        "v2.input" => {
            let phase = if text("phase") == "end" {
                InputPhase::End
            } else {
                InputPhase::Move
            };
            let value = event.get("value").and_then(Value::as_f64).unwrap_or(0.0);
            deck.emit_input(text("page"), text("tile"), text("stream"), phase, value);
        }
        "drop" => deck.drop_key_reports(number("count") as u32),
        _ => {}
    }
}

pub struct InputLog;
impl Endpoint for InputLog {
    const NAME: &'static str = "test.input_log";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/test/input-log";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<String>;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let (_, sink) = ctx.devices.mock().ok_or(CommonError::NotFound)?;
        Ok(input_log(sink))
    }
}
/// The keys the fake sink typed, oldest first, as `down <name>` or `up <name>`.
pub fn input_log(sink: &td_host::FakeSink) -> Vec<String> {
    sink.events
        .lock()
        .unwrap()
        .iter()
        .map(|event| match event {
            td_host::KeyEvent::Down(code) => format!("down {}", td_host::keymap::name(*code)),
            td_host::KeyEvent::Up(code) => format!("up {}", td_host::keymap::name(*code)),
        })
        .collect()
}

#[derive(Serialize, Deserialize, TS)]
pub struct PlugBoard {
    pub id: DeviceId,
    pub kind: BoardKind,
}
#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MockPortChange {
    Plug(PlugBoard),
    Unplug(DeviceId),
}
pub struct Ports;
impl Endpoint for Ports {
    const NAME: &'static str = "test.ports";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test/ports";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = NoQuery;
    type Body = MockPortChange;
    type Ok = ();
    type Error = DeviceError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let ports = mock_ports(&ctx)?;
        match req.body {
            MockPortChange::Plug(board) => ports.plug(board.id.as_str(), board.kind),
            MockPortChange::Unplug(id) => ports.unplug(id.as_str()),
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct ClockAdvance {
    pub advance_ms: u64,
}
pub struct Clock;
impl Endpoint for Clock {
    const NAME: &'static str = "test.clock";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test/clock";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = NoQuery;
    type Body = ClockAdvance;
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        ctx.devices.mock().ok_or(CommonError::NotFound)?;
        ctx.clock
            .advance(Duration::from_millis(req.body.advance_ms));
        Ok(())
    }
}

fn feature() -> Feature {
    Feature::new("test")
        .endpoint::<VolumeRead>()
        .endpoint::<VolumeWrite>()
        .endpoint::<Console>()
        .endpoint::<Mock>()
        .endpoint::<Press>()
        .endpoint::<Sets>()
        .endpoint::<Record>()
        .endpoint::<Emit>()
        .endpoint::<InputLog>()
        .endpoint::<Ports>()
        .endpoint::<Clock>()
        .endpoint::<ApprovalsScript>()
        .endpoint::<FirmwareScript>()
        .endpoint::<MediaScript>()
}
inventory::submit! { Registration(feature) }

#[derive(Serialize, Deserialize, TS)]
pub struct ApprovalScript {
    pub answers: Vec<crate::actions::ScriptAnswer>,
}
pub struct ApprovalsScript;
impl Endpoint for ApprovalsScript {
    const NAME: &'static str = "test.approvals_script";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test/approvals/script";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = NoQuery;
    type Body = ApprovalScript;
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<(), CommonError> {
        ctx.devices.mock().ok_or(CommonError::NotFound)?;
        if ctx.actions.script(req.body.answers) {
            Ok(())
        } else {
            Err(CommonError::NotFound)
        }
    }
}

#[derive(Serialize, Deserialize, TS)]
pub struct ConsentScript {
    pub answers: Vec<crate::features::firmware::ConsentAnswer>,
}
pub struct FirmwareScript;
impl Endpoint for FirmwareScript {
    const NAME: &'static str = "test.firmware_script";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test/firmware/script";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = NoQuery;
    type Body = ConsentScript;
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<(), CommonError> {
        ctx.devices.mock().ok_or(CommonError::NotFound)?;
        ctx.firmware.script(req.body.answers);
        Ok(())
    }
}

#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MediaFixture {
    pub title: String,
    pub artist: String,
    pub state: u8,
    pub cover: Option<String>,
}
#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MediaScriptBody {
    pub session: Option<MediaFixture>,
}
pub struct MediaScript;
impl Endpoint for MediaScript {
    const NAME: &'static str = "test.media";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test-hooks/media";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    const BODY_LIMIT: usize = 6 * 1024 * 1024;
    type Path = ();
    type Query = NoQuery;
    type Body = MediaScriptBody;
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<(), CommonError> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use td_host::integrations::media::{MAX_THUMBNAIL_BYTES, Playback};
        ctx.devices.mock().ok_or(CommonError::NotFound)?;
        let fake = ctx.integrations.media_fake().ok_or(CommonError::NotFound)?;
        if !fake.running() {
            return Err(CommonError::invalid("session", "Media is disabled"));
        }
        if let Some(session) = req.body.session {
            if session.title.len() > 4096 || session.artist.len() > 4096 {
                return Err(CommonError::invalid("session", "text is too long"));
            }
            let playback = match session.state {
                0 => Playback::Idle,
                1 => Playback::Paused,
                2 => Playback::Playing,
                _ => return Err(CommonError::invalid("session", "unknown playback state")),
            };
            let cover = session
                .cover
                .map(|encoded| {
                    if encoded.len() > MAX_THUMBNAIL_BYTES.div_ceil(3) * 4 {
                        return Err(CommonError::invalid("cover", "image is too large"));
                    }
                    let bytes = STANDARD
                        .decode(encoded)
                        .map_err(|_| CommonError::invalid("cover", "expected base64"))?;
                    if bytes.len() > MAX_THUMBNAIL_BYTES {
                        return Err(CommonError::invalid("cover", "image is too large"));
                    }
                    Ok(bytes)
                })
                .transpose()?;
            let generation = fake.begin();
            fake.metadata(generation, &session.title, &session.artist, playback);
            fake.feed().clear_cover(generation);
            if let Some(bytes) = cover {
                fake.cover(generation, &bytes);
            }
        } else {
            fake.idle();
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct VolumeInput {
    pub level: f64,
    pub muted: bool,
}
pub struct VolumeWrite;
impl Endpoint for VolumeWrite {
    const NAME: &'static str = "test.volume_write";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/test-hooks/volume";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = ();
    type Query = NoQuery;
    type Body = VolumeInput;
    type Ok = ();
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<(), CommonError> {
        ctx.devices.mock().ok_or(CommonError::NotFound)?;
        ctx.integrations
            .mock_volume()
            .ok_or(CommonError::NotFound)?
            .publish(Some(td_host::integrations::volume::Snapshot {
                level: req.body.level,
                muted: req.body.muted,
            }))
            .map_err(|_| CommonError::invalid("level", "Expected a number from 0 to 1."))
    }
}
pub struct VolumeRead;
impl Endpoint for VolumeRead {
    const NAME: &'static str = "test.volume_read";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/test-hooks/volume";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Value;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Value, CommonError> {
        ctx.devices.mock().ok_or(CommonError::NotFound)?;
        let fake = ctx
            .integrations
            .mock_volume()
            .ok_or(CommonError::NotFound)?;
        let snapshot = fake.snapshot().ok_or(CommonError::NotFound)?;
        Ok(json!({"level":snapshot.level,"muted":snapshot.muted,"sets":fake.sets()}))
    }
}
