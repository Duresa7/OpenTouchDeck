//! The layout format of `schema/layout.schema.json`, read with the rules of
//! `schema/README.md`. Every failure carries the README's rule key.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const UPLOAD_MAX: usize = 32768;
const DEPTH_MAX: usize = 16;

macro_rules! rules {
    ($($variant:ident => $key:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Rule { $($variant,)* }

        impl Rule {
            pub const ALL: &[Rule] = &[$(Rule::$variant,)*];

            pub fn key(self) -> &'static str {
                match self { $(Rule::$variant => $key,)* }
            }
        }
    };
}

rules! {
    FileUtf8 => "file.utf8",
    FileJson => "file.json",
    DuplicateMember => "json.duplicate-member",
    IntegerForm => "json.integer-form",
    MalformedUnicode => "string.malformed-unicode",
    ControlCharacter => "string.control-character",
    ShapeType => "shape.type",
    MissingField => "shape.missing-field",
    UnknownField => "shape.unknown-field",
    SchemaVersion => "layout.schema-version",
    LayoutOrientation => "layout.orientation",
    PageCount => "layout.page-count",
    TooLarge => "layout.too-large",
    NameBlank => "name.blank",
    NameTooLong => "name.too-long",
    IdFormat => "id.format",
    IdDuplicate => "id.duplicate",
    PageKind => "page.kind",
    TitleBlank => "title.blank",
    TitleTooLong => "title.too-long",
    GridSize => "grid.size",
    TileCount => "page.tile-count",
    LabelBlank => "label.blank",
    LabelTooLong => "label.too-long",
    TilePosition => "tile.position",
    TileSpan => "tile.span",
    TileBounds => "tile.bounds",
    TileOverlap => "tile.overlap",
    TileStreams => "tile.streams",
    TileInput => "tile.input",
    IconFormat => "icon.format",
    ActionSink => "action.sink",
    ActionType => "action.type",
    PageMissing => "action.page-missing",
    KeyUsageRange => "key.usage-range",
    KeyModsDuplicate => "key.mods-duplicate",
    ModifierUnknown => "modifier.unknown",
    ConsumerUsageRange => "consumer.usage-range",
    TextEmpty => "text.empty",
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutError {
    pub rule: Rule,
    /// The JSON pointer of the value the rule refused, or empty when the rule names no single value.
    pub path: String,
    pub message: String,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.rule.key(), self.message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Landscape,
    Portrait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Win,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Layout {
    pub schema: u8,
    pub name: String,
    pub orientation: Orientation,
    /// Tiles use the whole screen, with no page title or page dots. Written only when true, so a layout that
    /// leaves it off stays readable by firmware that predates it.
    #[serde(rename = "fullScreen", default, skip_serializing_if = "is_false")]
    pub full_screen: bool,
    pub pages: Vec<Page>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Page {
    Grid {
        id: String,
        title: String,
        rows: u8,
        cols: u8,
        tiles: Vec<Tile>,
    },
    Dashboard {
        id: String,
        title: String,
        source: String,
    },
}

impl Page {
    pub fn id(&self) -> &str {
        match self {
            Page::Grid { id, .. } | Page::Dashboard { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum TileRender {
    Gif,
    Graph,
    Bar,
    #[serde(rename = "now-playing")]
    NowPlaying,
}

/// The value type of one stream a Live tile declares, as `v2.live.set` sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum StreamType {
    Number,
    Series,
    Text,
    Image,
}

impl StreamType {
    pub const ALL: [StreamType; 4] = [Self::Number, Self::Series, Self::Text, Self::Image];

    pub fn name(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Series => "series",
            Self::Text => "text",
            Self::Image => "image",
        }
    }
}

/// One stream a renderer binds: its id, the types it accepts, and whether a Tile must declare it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderStream {
    pub id: &'static str,
    pub kinds: &'static [StreamType],
    pub required: bool,
}

const fn stream(id: &'static str, kinds: &'static [StreamType], required: bool) -> RenderStream {
    RenderStream {
        id,
        kinds,
        required,
    }
}

const GRAPH_STREAMS: &[RenderStream] = &[stream(
    "value",
    &[StreamType::Number, StreamType::Series],
    true,
)];
const BAR_STREAMS: &[RenderStream] = &[
    stream("value", &[StreamType::Number], true),
    stream("muted", &[StreamType::Number], false),
];
const NOW_PLAYING_STREAMS: &[RenderStream] = &[
    stream("title", &[StreamType::Text], true),
    stream("artist", &[StreamType::Text], true),
    stream("cover", &[StreamType::Image], true),
    stream("state", &[StreamType::Number], true),
];

impl TileRender {
    pub const ALL: [TileRender; 4] = [Self::Gif, Self::Graph, Self::Bar, Self::NowPlaying];

    pub fn name(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Graph => "graph",
            Self::Bar => "bar",
            Self::NowPlaying => "now-playing",
        }
    }

    /// The streams this renderer binds, each with the types it accepts.
    pub fn streams(render: Option<&TileRender>) -> &'static [RenderStream] {
        match render {
            Some(TileRender::Graph) => GRAPH_STREAMS,
            Some(TileRender::Bar) => BAR_STREAMS,
            Some(TileRender::NowPlaying) => NOW_PLAYING_STREAMS,
            Some(TileRender::Gif) | None => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub id: String,
    pub label: String,
    pub row: u8,
    pub col: u8,
    pub row_span: u8,
    pub col_span: u8,
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<TileRender>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streams: Option<BTreeMap<String, StreamType>>,
    /// A `bar` Tile the finger can drag to set its `value` stream. Written only when true.
    #[serde(default, skip_serializing_if = "is_false")]
    pub input: bool,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ActionSink {
    Link,
    Bluetooth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "TileAction"))]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Action {
    Key {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        sink: Option<ActionSink>,
        usage: u8,
        mods: Vec<Modifier>,
    },
    Mod {
        #[serde(rename = "mod")]
        modifier: Modifier,
    },
    Consumer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        sink: Option<ActionSink>,
        usage: u16,
    },
    Page {
        page: String,
    },
    None,
    Launch {
        path: String,
        args: Vec<String>,
        cwd: String,
    },
    Open {
        target: String,
    },
    Command {
        program: String,
        args: Vec<String>,
        cwd: String,
        shell: bool,
    },
    Widget {
        source: String,
        field: String,
        account: String,
    },
}

impl Layout {
    /// Parses the raw bytes of a layout file and applies every rule of the schema README.
    pub fn parse(bytes: &[u8]) -> Result<Layout, Vec<LayoutError>> {
        let fail = |rule, message: String| vec![error(rule, message)];
        let text = std::str::from_utf8(bytes)
            .map_err(|_| fail(Rule::FileUtf8, "file is not well-formed UTF-8".into()))?;
        let tree = Parser::parse(text).map_err(|error| fail(Rule::FileJson, error))?;
        let mut check = Check::default();
        check.source_rules(&tree, "");
        check.malformed_strings(&tree, "");
        let text_errors = check.errors.len();
        check.layout(&tree);
        if check.errors.len() > text_errors {
            return Err(check.errors);
        }
        let layout: Layout = serde_json::from_value(tree.to_value()).map_err(|error| {
            fail(
                Rule::ShapeType,
                format!("/ does not match the layout types: {error}"),
            )
        })?;
        check.errors.extend(layout.semantic_errors());
        if check.errors.is_empty() {
            Ok(layout)
        } else {
            Err(check.errors)
        }
    }

    /// The canonical compact form every sender uploads.
    pub fn to_canonical_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("layouts serialize")
    }

    fn semantic_errors(&self) -> Vec<LayoutError> {
        let mut errors = Vec::new();
        let size = self.to_canonical_json().len();
        if size > UPLOAD_MAX {
            errors.push(error(
                Rule::TooLarge,
                format!("layout compact UTF-8 JSON is {size} bytes; maximum is {UPLOAD_MAX} bytes"),
            ));
        }
        let page_ids: HashSet<&str> = self.pages.iter().map(Page::id).collect();
        let mut ids: HashMap<&str, String> = HashMap::new();
        for (page_index, page) in self.pages.iter().enumerate() {
            let page_location = format!("page {page_index} \"{}\"", page.id());
            note_id(&mut ids, &mut errors, page.id(), page_location.clone());
            let Page::Grid {
                id: page_id,
                rows,
                cols,
                tiles,
                ..
            } = page
            else {
                continue;
            };
            let mut occupied: HashMap<(u8, u8), Vec<&str>> = HashMap::new();
            for (tile_index, tile) in tiles.iter().enumerate() {
                note_id(
                    &mut ids,
                    &mut errors,
                    &tile.id,
                    format!("{page_location} tile {tile_index} \"{}\"", tile.id),
                );
                if let Action::Page { page: target } = &tile.action
                    && !page_ids.contains(target.as_str())
                {
                    errors.push(error(
                        Rule::PageMissing,
                        format!(
                            "page \"{page_id}\" tile \"{}\" references missing page \"{target}\"",
                            tile.id
                        ),
                    ));
                }
                let bindings = TileRender::streams(tile.render.as_ref());
                let streams = tile.streams.clone().unwrap_or_default();
                for (id, kind) in &streams {
                    if !bindings
                        .iter()
                        .any(|binding| binding.id == id && binding.kinds.contains(kind))
                    {
                        errors.push(error(
                            Rule::TileStreams,
                            format!(
                                "page \"{page_id}\" tile \"{}\" cannot bind stream \"{id}\" as \"{}\"",
                                tile.id,
                                kind.name()
                            ),
                        ));
                    }
                }
                for binding in bindings.iter().filter(|binding| binding.required) {
                    if !streams.contains_key(binding.id) {
                        errors.push(error(
                            Rule::TileStreams,
                            format!(
                                "page \"{page_id}\" tile \"{}\" needs stream \"{}\"",
                                tile.id, binding.id
                            ),
                        ));
                    }
                }
                if tile.row + tile.row_span > *rows || tile.col + tile.col_span > *cols {
                    errors.push(error(
                        Rule::TileBounds,
                        format!(
                            "page \"{page_id}\" tile \"{}\" exceeds grid bounds {rows}x{cols}",
                            tile.id
                        ),
                    ));
                    continue;
                }
                let mut overlaps: Vec<&str> = Vec::new();
                for row in tile.row..tile.row + tile.row_span {
                    for col in tile.col..tile.col + tile.col_span {
                        let cell = occupied.entry((row, col)).or_default();
                        for other in cell.iter() {
                            if !overlaps.contains(other) {
                                overlaps.push(other);
                            }
                        }
                        cell.push(&tile.id);
                    }
                }
                for other in overlaps {
                    errors.push(error(
                        Rule::TileOverlap,
                        format!(
                            "page \"{page_id}\" tiles \"{other}\" and \"{}\" overlap",
                            tile.id
                        ),
                    ));
                }
            }
        }
        errors
    }
}

fn error(rule: Rule, message: String) -> LayoutError {
    LayoutError {
        rule,
        path: String::new(),
        message,
    }
}

fn note_id<'a>(
    ids: &mut HashMap<&'a str, String>,
    errors: &mut Vec<LayoutError>,
    id: &'a str,
    location: String,
) {
    match ids.get(id) {
        Some(first) => errors.push(error(
            Rule::IdDuplicate,
            format!("duplicate id \"{id}\" at {location}; first used at {first}"),
        )),
        None => {
            ids.insert(id, location);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct JsonStr {
    text: String,
    well_formed: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number { value: f64, integer_form: bool },
    Str(JsonStr),
    Array(Vec<Json>),
    Object(Vec<(JsonStr, Json)>),
}

impl Json {
    fn to_value(&self) -> Value {
        match self {
            Json::Null => Value::Null,
            Json::Bool(value) => Value::Bool(*value),
            Json::Number { value, .. } if value.fract() == 0.0 && value.abs() < 9.0e15 => {
                Value::from(*value as i64)
            }
            Json::Number { value, .. } => {
                serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number)
            }
            Json::Str(string) => Value::String(string.text.clone()),
            Json::Array(items) => Value::Array(items.iter().map(Json::to_value).collect()),
            Json::Object(members) => Value::Object(
                members
                    .iter()
                    .map(|(name, value)| (name.text.clone(), value.to_value()))
                    .collect(),
            ),
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn parse(text: &str) -> Result<Json, String> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        parser.whitespace();
        let value = parser.value(0)?;
        parser.whitespace();
        if parser.pos != parser.bytes.len() {
            return Err(parser.error("unexpected data after the JSON value"));
        }
        Ok(value)
    }

    fn error(&self, what: &str) -> String {
        format!("{what} at byte {}", self.pos)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", byte as char)))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if matches!(self.peek(), Some(b'{' | b'[')) && depth >= DEPTH_MAX {
            return Err(self.error(&format!("nesting deeper than {DEPTH_MAX} levels")));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("unexpected token"))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut members = Vec::new();
        self.whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a member name"));
            }
            let name = self.string()?;
            self.whitespace();
            self.expect(b':')?;
            self.whitespace();
            members.push((name, self.value(depth + 1)?));
            self.whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.whitespace();
            items.push(self.value(depth + 1)?);
            self.whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let digits = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error("short unicode escape"))?;
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return Err(self.error("bad unicode escape"));
        }
        let text = std::str::from_utf8(digits).map_err(|_| self.error("bad unicode escape"))?;
        let unit = u16::from_str_radix(text, 16).map_err(|_| self.error("bad unicode escape"))?;
        self.pos += 4;
        Ok(unit)
    }

    fn string(&mut self) -> Result<JsonStr, String> {
        self.expect(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        let mut well_formed = true;
        let push = |out: &mut Vec<u8>, c: char| {
            out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes())
        };
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| self.error("unterminated string"))?;
            self.pos += 1;
            match byte {
                b'"' => break,
                0x00..=0x1F => return Err(self.error("control character in string")),
                b'\\' => {
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let unit = self.hex4()?;
                            let low = if (0xD800..0xDC00).contains(&unit)
                                && self.bytes[self.pos..].starts_with(b"\\u")
                            {
                                let save = self.pos;
                                self.pos += 2;
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    Some(low)
                                } else {
                                    self.pos = save;
                                    None
                                }
                            } else {
                                None
                            };
                            let code = match low {
                                Some(low) => {
                                    0x10000
                                        + ((u32::from(unit) - 0xD800) << 10)
                                        + (u32::from(low) - 0xDC00)
                                }
                                None => u32::from(unit),
                            };
                            match char::from_u32(code) {
                                Some(c) => push(&mut out, c),
                                None => {
                                    well_formed = false;
                                    push(&mut out, char::REPLACEMENT_CHARACTER);
                                }
                            }
                        }
                        _ => return Err(self.error("bad escape")),
                    }
                }
                _ => out.push(byte),
            }
        }
        let text = String::from_utf8(out).map_err(|_| self.error("bad string bytes"))?;
        Ok(JsonStr { text, well_formed })
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        let digits = |parser: &mut Parser| {
            let from = parser.pos;
            while matches!(parser.peek(), Some(b'0'..=b'9')) {
                parser.pos += 1;
            }
            parser.pos - from
        };
        let negative = self.peek() == Some(b'-');
        if negative {
            self.pos += 1;
        }
        let zero = self.peek() == Some(b'0');
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return Err(self.error("bad number")),
        }
        let mut integer_form = !(negative && zero);
        if self.peek() == Some(b'.') {
            self.pos += 1;
            integer_form = false;
            if digits(self) == 0 {
                return Err(self.error("bad fraction"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            integer_form = false;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if digits(self) == 0 {
                return Err(self.error("bad exponent"));
            }
        }
        let token =
            std::str::from_utf8(&self.bytes[start..self.pos]).expect("number tokens are ASCII");
        let value = token.parse::<f64>().map_err(|_| self.error("bad number"))?;
        Ok(Json::Number {
            value,
            integer_form,
        })
    }
}

fn pointer_segment(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

fn place(pointer: &str) -> &str {
    if pointer.is_empty() { "/" } else { pointer }
}

#[derive(Clone, Copy)]
struct Obj<'a>(&'a [(JsonStr, Json)]);

impl<'a> Obj<'a> {
    fn get(self, name: &str) -> Option<&'a Json> {
        self.0
            .iter()
            .rev()
            .find(|(member, _)| member.text == name)
            .map(|(_, value)| value)
    }
}

const ICONS: [&str; 6] = [
    "lv:prev",
    "lv:play_pause",
    "lv:next",
    "lv:mute",
    "lv:volume_mid",
    "lv:volume_max",
];
const MODIFIERS: [&str; 4] = ["ctrl", "shift", "alt", "win"];

#[derive(Default)]
struct Check {
    v2: bool,
    errors: Vec<LayoutError>,
}

struct Bounded {
    max: usize,
    too_long: Rule,
    blank: Rule,
}

const NAME: Bounded = Bounded {
    max: 24,
    too_long: Rule::NameTooLong,
    blank: Rule::NameBlank,
};
const TITLE: Bounded = Bounded {
    max: 24,
    too_long: Rule::TitleTooLong,
    blank: Rule::TitleBlank,
};
const LABEL: Bounded = Bounded {
    max: 16,
    too_long: Rule::LabelTooLong,
    blank: Rule::LabelBlank,
};

impl Check {
    fn fail(&mut self, rule: Rule, pointer: &str, message: impl fmt::Display) {
        self.errors.push(LayoutError {
            rule,
            path: place(pointer).into(),
            message: format!("{} {message}", place(pointer)),
        });
    }

    fn source_rules(&mut self, value: &Json, pointer: &str) {
        match value {
            Json::Number { integer_form: false, value } => {
                self.fail(Rule::IntegerForm, pointer, format!("is written as {value} with a fraction or exponent; numbers must be JSON integers"))
            }
            Json::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.source_rules(item, &format!("{pointer}/{index}"));
                }
            }
            Json::Object(members) => {
                let mut seen = HashSet::new();
                for (name, item) in members {
                    if !seen.insert(name.text.as_str()) {
                        self.fail(Rule::DuplicateMember, pointer, format!("repeats the member name {:?}", name.text));
                    }
                    self.source_rules(item, &format!("{pointer}/{}", pointer_segment(&name.text)));
                }
            }
            _ => {}
        }
    }

    fn malformed_strings(&mut self, value: &Json, pointer: &str) {
        match value {
            Json::Str(JsonStr {
                well_formed: false, ..
            }) => self.fail(
                Rule::MalformedUnicode,
                pointer,
                "is not well-formed Unicode",
            ),
            Json::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.malformed_strings(item, &format!("{pointer}/{index}"));
                }
            }
            Json::Object(members) => {
                for (name, item) in members {
                    if !name.well_formed {
                        self.fail(
                            Rule::MalformedUnicode,
                            pointer,
                            format!(
                                "has the member name {:?}, which is not well-formed Unicode",
                                name.text
                            ),
                        );
                    }
                    self.malformed_strings(
                        item,
                        &format!("{pointer}/{}", pointer_segment(&name.text)),
                    );
                }
            }
            _ => {}
        }
    }

    fn wrong_type(&mut self, pointer: &str, expected: &str) {
        self.fail(
            Rule::ShapeType,
            pointer,
            format!("must be {expected} (type)"),
        );
    }

    fn object<'a>(&mut self, value: &'a Json, pointer: &str, fields: &[&str]) -> Option<Obj<'a>> {
        self.object_with(value, pointer, fields, &[])
    }

    /// Like `object`, with `optional` members that may be left out.
    fn object_with<'a>(
        &mut self,
        value: &'a Json,
        pointer: &str,
        fields: &[&str],
        optional: &[&str],
    ) -> Option<Obj<'a>> {
        let Json::Object(members) = value else {
            self.wrong_type(pointer, "object");
            return None;
        };
        let obj = Obj(members);
        for field in fields {
            if obj.get(field).is_none() {
                self.fail(
                    Rule::MissingField,
                    pointer,
                    format!("must have required property '{field}' (required) {field:?}"),
                );
            }
        }
        let mut reported = HashSet::new();
        for (name, _) in members {
            let known =
                fields.contains(&name.text.as_str()) || optional.contains(&name.text.as_str());
            if !known && reported.insert(name.text.as_str()) {
                self.fail(
                    Rule::UnknownField,
                    pointer,
                    format!(
                        "must NOT have additional properties (additionalProperties) {:?}",
                        name.text
                    ),
                );
            }
        }
        Some(obj)
    }

    fn field<'a>(&mut self, obj: Obj<'a>, pointer: &str, name: &str) -> Option<(&'a Json, String)> {
        obj.get(name)
            .map(|value| (value, format!("{pointer}/{name}")))
    }

    fn string<'a>(&mut self, value: &'a Json, pointer: &str) -> Option<&'a str> {
        let Json::Str(string) = value else {
            self.wrong_type(pointer, "string");
            return None;
        };
        if string.text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}') {
            self.fail(
                Rule::ControlCharacter,
                pointer,
                "must not contain U+0000 to U+001F or U+007F (pattern)",
            );
        }
        Some(&string.text)
    }

    fn text(&mut self, value: &Json, pointer: &str) {
        if self.string(value, pointer) == Some("") {
            self.fail(
                Rule::TextEmpty,
                pointer,
                "must NOT have fewer than 1 characters (minLength)",
            );
        }
    }

    fn bounded(&mut self, value: &Json, pointer: &str, rules: &Bounded) {
        let Some(text) = self.string(value, pointer) else {
            return;
        };
        if text.chars().count() > rules.max {
            self.fail(
                rules.too_long,
                pointer,
                format!(
                    "must NOT have more than {} characters (maxLength)",
                    rules.max
                ),
            );
        }
        if text.chars().all(char::is_whitespace) {
            self.fail(
                rules.blank,
                pointer,
                "must hold a character that is not whitespace (pattern)",
            );
        }
    }

    fn id(&mut self, value: &Json, pointer: &str) {
        let Json::Str(string) = value else {
            return self.wrong_type(pointer, "string");
        };
        let text = &string.text;
        if !(1..=15).contains(&text.len())
            || !text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            self.fail(
                Rule::IdFormat,
                pointer,
                "must match pattern \"^[a-z0-9_]{1,15}$\" (pattern)",
            );
        }
    }

    fn integer(&mut self, value: &Json, pointer: &str, min: f64, max: f64, rule: Rule) {
        let Json::Number { value, .. } = value else {
            return self.wrong_type(pointer, "integer");
        };
        if value.fract() != 0.0 {
            self.wrong_type(pointer, "integer");
        }
        if *value < min {
            self.fail(rule, pointer, format!("must be >= {min} (minimum)"));
        } else if *value > max {
            self.fail(rule, pointer, format!("must be <= {max} (maximum)"));
        }
    }

    fn modifier(&mut self, value: &Json, pointer: &str) {
        if !matches!(value, Json::Str(s) if MODIFIERS.contains(&s.text.as_str())) {
            self.fail(
                Rule::ModifierUnknown,
                pointer,
                "must be equal to one of the allowed values (enum)",
            );
        }
    }

    fn args(&mut self, value: &Json, pointer: &str) {
        let Json::Array(items) = value else {
            return self.wrong_type(pointer, "array");
        };
        for (index, item) in items.iter().enumerate() {
            self.string(item, &format!("{pointer}/{index}"));
        }
    }

    fn layout(&mut self, value: &Json) {
        let Some(obj) = self.object_with(
            value,
            "",
            &["schema", "name", "orientation", "pages"],
            &["fullScreen"],
        ) else {
            return;
        };
        if let Some(schema) = obj.get("schema")
            && !matches!(schema, Json::Number { value, .. } if *value == 1.0 || *value == 2.0)
        {
            self.fail(
                Rule::SchemaVersion,
                "/schema",
                "must be equal to constant (const)",
            );
        }
        self.v2 = matches!(obj.get("schema"), Some(Json::Number { value, .. }) if *value == 2.0);
        if let Some(name) = obj.get("name") {
            self.bounded(name, "/name", &NAME);
        }
        if let Some(orientation) = obj.get("orientation")
            && !matches!(orientation, Json::Str(s) if s.text == "landscape" || s.text == "portrait")
        {
            self.fail(
                Rule::LayoutOrientation,
                "/orientation",
                "must be equal to one of the allowed values (enum)",
            );
        }
        if let Some(full_screen) = obj.get("fullScreen")
            && !matches!(full_screen, Json::Bool(_))
        {
            self.wrong_type("/fullScreen", "boolean");
        }
        let Some(pages) = obj.get("pages") else {
            return;
        };
        let Json::Array(pages) = pages else {
            return self.wrong_type("/pages", "array");
        };
        if pages.is_empty() || pages.len() > 8 {
            self.fail(
                Rule::PageCount,
                "/pages",
                format!(
                    "must have 1 to 8 items, not {} (minItems, maxItems)",
                    pages.len()
                ),
            );
        }
        for (index, page) in pages.iter().enumerate() {
            self.page(page, &format!("/pages/{index}"));
        }
    }

    fn page(&mut self, value: &Json, pointer: &str) {
        let Json::Object(members) = value else {
            return self.wrong_type(pointer, "object");
        };
        let fields: &[&str] = match Obj(members).get("kind") {
            Some(Json::Str(kind)) if kind.text == "grid" => {
                &["id", "title", "kind", "rows", "cols", "tiles"]
            }
            Some(Json::Str(kind)) if kind.text == "dashboard" => &["id", "title", "kind", "source"],
            _ => {
                return self.fail(
                    Rule::PageKind,
                    pointer,
                    "must match exactly one schema in oneOf (oneOf)",
                );
            }
        };
        let Some(obj) = self.object(value, pointer, fields) else {
            return;
        };
        if let Some((id, at)) = self.field(obj, pointer, "id") {
            self.id(id, &at);
        }
        if let Some((title, at)) = self.field(obj, pointer, "title") {
            self.bounded(title, &at, &TITLE);
        }
        if let Some((source, at)) = self.field(obj, pointer, "source") {
            return self.text(source, &at);
        }
        for name in ["rows", "cols"] {
            if let Some((size, at)) = self.field(obj, pointer, name) {
                self.integer(size, &at, 1.0, 8.0, Rule::GridSize);
            }
        }
        let Some((tiles, at)) = self.field(obj, pointer, "tiles") else {
            return;
        };
        let Json::Array(tiles) = tiles else {
            return self.wrong_type(&at, "array");
        };
        if tiles.len() > 64 {
            self.fail(
                Rule::TileCount,
                &at,
                "must NOT have more than 64 items (maxItems)",
            );
        }
        for (index, tile) in tiles.iter().enumerate() {
            self.tile(tile, &format!("{at}/{index}"));
        }
    }

    fn tile(&mut self, value: &Json, pointer: &str) {
        let fields = [
            "id", "label", "row", "col", "rowSpan", "colSpan", "icon", "action",
        ];
        let Some(obj) = self.object_with(
            value,
            pointer,
            &fields,
            if self.v2 {
                &["asset", "render", "streams", "input"]
            } else {
                &[]
            },
        ) else {
            return;
        };
        if let Some((id, at)) = self.field(obj, pointer, "id") {
            self.id(id, &at);
        }
        if let Some((label, at)) = self.field(obj, pointer, "label") {
            self.bounded(label, &at, &LABEL);
        }
        for (name, min, max, rule) in [
            ("row", 0.0, 7.0, Rule::TilePosition),
            ("col", 0.0, 7.0, Rule::TilePosition),
            ("rowSpan", 1.0, 8.0, Rule::TileSpan),
            ("colSpan", 1.0, 8.0, Rule::TileSpan),
        ] {
            if let Some((number, at)) = self.field(obj, pointer, name) {
                self.integer(number, &at, min, max, rule);
            }
        }
        match self.field(obj, pointer, "icon") {
            Some((Json::Null, _)) | None => {}
            Some((Json::Str(icon), at)) => {
                let file = icon.text.strip_prefix("file:").is_some_and(|hash| {
                    hash.len() == 16 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                });
                if !file && !ICONS.contains(&icon.text.as_str()) {
                    self.fail(Rule::IconFormat, &at, "must be null, a built-in lv: name, or file: and 16 lowercase hex digits (pattern)");
                }
            }
            Some((_, at)) => self.wrong_type(&at, "string,null"),
        }
        let render = self
            .field(obj, pointer, "render")
            .filter(|_| self.v2)
            .map(|(render, at)| {
                let kind = match render {
                    Json::Str(s) => TileRender::ALL.into_iter().find(|r| r.name() == s.text),
                    _ => None,
                };
                (kind, at)
            });
        if let Some((kind, at)) = &render {
            let asset = matches!(obj.get("asset"), Some(Json::Str(_)));
            let valid = match kind {
                Some(TileRender::Gif) => asset,
                Some(TileRender::Bar | TileRender::NowPlaying) => !asset,
                Some(TileRender::Graph) => true,
                None => false,
            };
            if !valid {
                self.fail(
                    Rule::IconFormat,
                    at,
                    "render must be graph, bar or now-playing, or gif with an Asset reference; bar and now-playing take no Asset",
                );
            }
        }
        if let Some((input, at)) = self.field(obj, pointer, "input").filter(|_| self.v2) {
            let bar = matches!(render, Some((Some(TileRender::Bar), _)));
            let widget = matches!(obj.get("action"), Some(Json::Object(action)) if matches!(Obj(action).get("type"), Some(Json::Str(kind)) if kind.text == "widget"));
            if !matches!(input, Json::Bool(true)) || !bar || !widget {
                self.fail(
                    Rule::TileInput,
                    &at,
                    "input must be true, on a bar Tile with a widget action",
                );
            }
        }
        if let Some((streams, at)) = self.field(obj, pointer, "streams") {
            match streams {
                Json::Object(members) => {
                    let known = members.iter().all(|(_, value)| {
                        matches!(value, Json::Str(s) if StreamType::ALL.iter().any(|t| t.name() == s.text))
                    });
                    if !known {
                        self.fail(
                            Rule::TileStreams,
                            &at,
                            "stream types must be number, series, text or image",
                        );
                    }
                }
                _ => self.wrong_type(&at, "object"),
            }
        }
        if let Some((asset, at)) = self.field(obj, pointer, "asset") {
            let valid = matches!(asset, Json::Null)
                || matches!(asset, Json::Str(s) if s.text.strip_prefix("sha256:").is_some_and(|id| id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))));
            if !valid
                || (!matches!(asset, Json::Null) && !matches!(obj.get("icon"), Some(Json::Null)))
            {
                self.fail(
                    Rule::IconFormat,
                    &at,
                    "Asset must be sha256 followed by 64 lowercase hex digits, with icon null",
                );
            }
        }
        if let Some((action, at)) = self.field(obj, pointer, "action") {
            self.action(action, &at);
        }
    }

    fn action(&mut self, value: &Json, pointer: &str) {
        let Json::Object(members) = value else {
            return self.wrong_type(pointer, "object");
        };
        let kind = match Obj(members).get("type") {
            Some(Json::Str(kind)) => kind.text.as_str(),
            _ => "",
        };
        let fields: &[&str] = match kind {
            "key" => &["type", "usage", "mods"],
            "mod" => &["type", "mod"],
            "consumer" => &["type", "usage"],
            "page" => &["type", "page"],
            "none" => &["type"],
            "launch" => &["type", "path", "args", "cwd"],
            "open" => &["type", "target"],
            "command" => &["type", "program", "args", "cwd", "shell"],
            "widget" => &["type", "source", "field", "account"],
            _ => {
                return self.fail(
                    Rule::ActionType,
                    pointer,
                    "must match exactly one schema in oneOf (oneOf)",
                );
            }
        };
        let optional: &[&str] = if matches!(kind, "key" | "consumer") {
            &["sink"]
        } else {
            &[]
        };
        let Some(obj) = self.object_with(value, pointer, fields, optional) else {
            return;
        };
        if let Some(sink) = obj.get("sink")
            && !matches!(sink, Json::Str(s) if matches!(s.text.as_str(), "link" | "bluetooth"))
        {
            self.fail(
                Rule::ActionSink,
                &format!("{pointer}/sink"),
                "must be link or bluetooth",
            );
        }
        for name in &fields[1..] {
            let Some((field, at)) = self.field(obj, pointer, name) else {
                continue;
            };
            match (kind, *name) {
                ("key", "usage") => self.integer(field, &at, 1.0, 255.0, Rule::KeyUsageRange),
                ("consumer", "usage") => {
                    self.integer(field, &at, 1.0, 65535.0, Rule::ConsumerUsageRange)
                }
                ("key", "mods") => self.mods(field, &at),
                ("mod", "mod") => self.modifier(field, &at),
                ("page", "page") => self.id(field, &at),
                (_, "args") => self.args(field, &at),
                (_, "cwd" | "account") => {
                    self.string(field, &at);
                }
                (_, "shell") if !matches!(field, Json::Bool(_)) => self.wrong_type(&at, "boolean"),
                (_, "shell") => {}
                _ => self.text(field, &at),
            }
        }
    }

    fn mods(&mut self, value: &Json, pointer: &str) {
        let Json::Array(items) = value else {
            return self.wrong_type(pointer, "array");
        };
        if items
            .iter()
            .enumerate()
            .any(|(index, item)| items[..index].contains(item))
        {
            self.fail(
                Rule::KeyModsDuplicate,
                pointer,
                "must NOT have duplicate items (uniqueItems)",
            );
        }
        for (index, item) in items.iter().enumerate() {
            self.modifier(item, &format!("{pointer}/{index}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn schema_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../schema")
    }

    fn keys(errors: &[LayoutError]) -> Vec<&'static str> {
        errors.iter().map(|error| error.rule.key()).collect()
    }

    fn launch_layout(arg: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": 1, "name": "Size", "orientation": "landscape",
            "pages": [{"id": "p", "title": "P", "kind": "grid", "rows": 1, "cols": 1, "tiles": [
                {"id": "t", "label": "T", "row": 0, "col": 0, "rowSpan": 1, "colSpan": 1, "icon": null,
                 "action": {"type": "launch", "path": "x", "args": [arg], "cwd": ""}}
            ]}]
        }))
        .unwrap()
    }

    #[test]
    fn an_error_names_the_refused_value_by_pointer_apart_from_its_message() {
        let mut layout: Value = serde_json::from_slice(&launch_layout("a")).unwrap();
        layout["pages"][0]["tiles"][0]["action"]["path"] = Value::from("");
        let errors = Layout::parse(&serde_json::to_vec(&layout).unwrap()).unwrap_err();
        let empty = errors.iter().find(|e| e.rule == Rule::TextEmpty).unwrap();
        assert_eq!(empty.path, "/pages/0/tiles/0/action/path");
        assert!(empty.message.starts_with(&empty.path), "{}", empty.message);

        let mut layout: Value = serde_json::from_slice(&launch_layout("a")).unwrap();
        let mut copy = layout["pages"][0]["tiles"][0].clone();
        copy["id"] = Value::from("u");
        layout["pages"][0]["tiles"]
            .as_array_mut()
            .unwrap()
            .push(copy);
        let errors = Layout::parse(&serde_json::to_vec(&layout).unwrap()).unwrap_err();
        let overlap = errors.iter().find(|e| e.rule == Rule::TileOverlap).unwrap();
        assert_eq!(overlap.path, "");
    }

    #[test]
    fn bundled_layouts_are_valid() {
        for name in [
            "builtin.json",
            "dev.json",
            "full-screen.json",
            "gif.json",
            "graph.json",
            "media.json",
        ] {
            let bytes = std::fs::read(schema_dir().join("examples").join(name)).unwrap();
            let layout =
                Layout::parse(&bytes).unwrap_or_else(|errors| panic!("{name}: {errors:?}"));
            println!("ok examples/{name}: valid");
            let compact: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                layout.to_canonical_json().len(),
                serde_json::to_vec(&compact).unwrap().len(),
                "{name}"
            );
        }
    }

    #[test]
    fn graph_tiles_declare_their_value_stream() {
        let bytes = std::fs::read(schema_dir().join("examples/graph.json")).unwrap();
        let layout = Layout::parse(&bytes).unwrap();
        let Page::Grid { tiles, .. } = &layout.pages[0] else {
            panic!("grid page")
        };
        assert_eq!(tiles[0].render, Some(TileRender::Graph));
        assert_eq!(
            tiles[0].streams.as_ref().unwrap()["value"],
            StreamType::Number
        );
        assert_eq!(
            tiles[1].streams.as_ref().unwrap()["value"],
            StreamType::Series
        );
        let text = String::from_utf8(layout.to_canonical_json()).unwrap();
        assert!(text.contains(r#""render":"graph","streams":{"value":"number"}"#));
    }

    #[test]
    fn renderer_streams_match_the_shared_table() {
        let table: Value = serde_json::from_slice(
            &std::fs::read(schema_dir().join("live/renderers.json")).unwrap(),
        )
        .unwrap();
        let renderers = table["renderers"].as_object().unwrap();
        let live: Vec<TileRender> = TileRender::ALL
            .into_iter()
            .filter(|render| !TileRender::streams(Some(render)).is_empty())
            .collect();
        let mut names: Vec<&str> = live.iter().map(|render| render.name()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            renderers.keys().map(String::as_str).collect::<Vec<_>>()
        );
        for render in live {
            let ours: Value = TileRender::streams(Some(&render))
                .iter()
                .map(|stream| {
                    let kinds: Vec<&str> = stream.kinds.iter().map(|kind| kind.name()).collect();
                    (
                        stream.id.to_owned(),
                        serde_json::json!({"types": kinds, "required": stream.required}),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
                .into();
            assert_eq!(
                ours,
                renderers[render.name()]["streams"],
                "{}",
                render.name()
            );
            assert_eq!(
                serde_json::to_value(render).unwrap(),
                Value::String(render.name().into())
            );
        }
    }

    #[test]
    fn media_tiles_take_optional_streams_and_input_round_trips() {
        let bytes = std::fs::read(schema_dir().join("examples/media.json")).unwrap();
        let layout = Layout::parse(&bytes).unwrap();
        let Page::Grid { tiles, .. } = &layout.pages[0] else {
            panic!("grid page")
        };
        assert_eq!(
            [tiles[0].render, tiles[1].render, tiles[2].render],
            [
                Some(TileRender::Bar),
                Some(TileRender::Bar),
                Some(TileRender::NowPlaying)
            ]
        );
        assert_eq!(
            [tiles[0].input, tiles[1].input, tiles[2].input],
            [true, false, false]
        );
        assert!(!tiles[1].streams.as_ref().unwrap().contains_key("muted"));
        let text = String::from_utf8(layout.to_canonical_json()).unwrap();
        assert!(text.contains(r#""render":"now-playing""#));
        assert_eq!(text.matches(r#""input":true"#).count(), 1);
        assert!(!text.contains(r#""input":false"#));
        assert_eq!(Layout::parse(text.as_bytes()).unwrap(), layout);
    }

    #[test]
    fn each_invalid_example_fails_with_its_rule_key() {
        let directory = schema_dir().join("examples/invalid");
        let mut checked = HashSet::new();
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            let end = bytes.iter().position(|&b| b == b'\n').unwrap();
            let first = String::from_utf8_lossy(&bytes[..end]);
            let expected = first
                .trim_end_matches('\r')
                .strip_prefix("expected: ")
                .unwrap_or_else(|| panic!("{path:?} names no rule key"));
            let errors = Layout::parse(&bytes[end + 1..]).expect_err(&format!("{path:?} passed"));
            assert_eq!(keys(&errors), [expected], "{path:?}: {errors:?}");
            println!(
                "ok examples/invalid/{}: {expected}",
                path.file_name().unwrap().to_string_lossy()
            );
            checked.insert(expected.to_owned());
        }
        let all: HashSet<String> = Rule::ALL.iter().map(|rule| rule.key().to_owned()).collect();
        assert_eq!(checked, all, "one invalid example per rule key");
    }

    #[test]
    fn rule_keys_match_the_schema_readme() {
        let readme = std::fs::read_to_string(schema_dir().join("README.md")).unwrap();
        let documented: HashSet<&str> = readme
            .lines()
            .filter_map(|line| line.strip_prefix("| `")?.split('`').next())
            .filter(|key| {
                key.contains('.')
                    && key.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'
                    })
            })
            .collect();
        let ours: HashSet<&str> = Rule::ALL.iter().map(|rule| rule.key()).collect();
        assert_eq!(documented, ours);
    }

    #[test]
    fn size_limit_counts_the_canonical_form() {
        let base = launch_layout("").len();
        let at_limit = launch_layout(&"a".repeat(UPLOAD_MAX - base));
        assert_eq!(at_limit.len(), UPLOAD_MAX);
        assert!(Layout::parse(&at_limit).is_ok());
        let over = launch_layout(&"a".repeat(UPLOAD_MAX - base + 1));
        assert_eq!(
            keys(&Layout::parse(&over).unwrap_err()),
            ["layout.too-large"]
        );
        let pretty: Value = serde_json::from_slice(&at_limit).unwrap();
        let spaced = serde_json::to_vec_pretty(&pretty).unwrap();
        assert!(spaced.len() > UPLOAD_MAX);
        assert_eq!(
            Layout::parse(&spaced).unwrap().to_canonical_json().len(),
            UPLOAD_MAX
        );
    }

    #[test]
    fn text_rules_apply_to_raw_bytes() {
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend(launch_layout(""));
        assert_eq!(keys(&Layout::parse(&bom).unwrap_err()), ["file.json"]);
        let escaped = String::from_utf8(launch_layout(""))
            .unwrap()
            .replace("\"args\":[\"\"]", "\"args\":[\"\\u0000\"]");
        assert_eq!(
            keys(&Layout::parse(escaped.as_bytes()).unwrap_err()),
            ["string.control-character"]
        );
        let exponent = String::from_utf8(launch_layout(""))
            .unwrap()
            .replace("\"rows\":1", "\"rows\":1E0");
        assert_eq!(
            keys(&Layout::parse(exponent.as_bytes()).unwrap_err()),
            ["json.integer-form"]
        );
        let paired = String::from_utf8(launch_layout(""))
            .unwrap()
            .replace("\"label\":\"T\"", "\"label\":\"\\ud83c\\udf9b\"");
        assert!(Layout::parse(paired.as_bytes()).is_ok());
    }

    #[test]
    fn unicode_escapes_need_four_hex_digits() {
        for escape in [r"\u+041", r"\u 041", r"\u-041"] {
            let text = String::from_utf8(launch_layout(""))
                .unwrap()
                .replace("\"label\":\"T\"", &format!("\"label\":\"{escape}\""));
            assert_eq!(
                keys(&Layout::parse(text.as_bytes()).unwrap_err()),
                ["file.json"],
                "{escape}"
            );
        }
        let valid = String::from_utf8(launch_layout(""))
            .unwrap()
            .replace("\"label\":\"T\"", r#""label":"A""#);
        assert!(Layout::parse(valid.as_bytes()).is_ok());
    }

    #[test]
    fn negative_zero_is_not_an_integer_form() {
        let text = String::from_utf8(launch_layout(""))
            .unwrap()
            .replace("\"row\":0", "\"row\":-0");
        assert_eq!(
            keys(&Layout::parse(text.as_bytes()).unwrap_err()),
            ["json.integer-form"]
        );
    }

    #[test]
    fn nesting_stops_at_sixteen_levels() {
        let nested = |levels: usize| format!("{}{}", "[".repeat(levels), "]".repeat(levels));
        assert_eq!(
            keys(&Layout::parse(nested(16).as_bytes()).unwrap_err()),
            ["shape.type"]
        );
        assert_eq!(
            keys(&Layout::parse(nested(17).as_bytes()).unwrap_err()),
            ["file.json"]
        );
    }

    #[test]
    fn a_missing_discriminator_names_the_kind_or_type_rule() {
        let text = String::from_utf8(launch_layout("")).unwrap();
        assert_eq!(
            keys(&Layout::parse(text.replace("\"kind\":\"grid\",", "").as_bytes()).unwrap_err()),
            ["page.kind"]
        );
        assert_eq!(
            keys(&Layout::parse(text.replace(",\"type\":\"launch\"", "").as_bytes()).unwrap_err()),
            ["action.type"]
        );
    }

    #[test]
    fn full_screen_round_trips_and_is_written_only_when_true() {
        let canonical = |layout: &Layout| -> Value {
            serde_json::from_slice(&layout.to_canonical_json()).unwrap()
        };
        let base = String::from_utf8(launch_layout("")).unwrap();
        let plain = Layout::parse(base.as_bytes()).unwrap();
        assert!(!plain.full_screen);
        assert_eq!(canonical(&plain).get("fullScreen"), None);

        let on = base.replacen("\"pages\"", "\"fullScreen\":true,\"pages\"", 1);
        let layout = Layout::parse(on.as_bytes()).unwrap();
        assert!(layout.full_screen);
        assert_eq!(canonical(&layout)["fullScreen"], Value::Bool(true));
        assert_eq!(Layout::parse(&layout.to_canonical_json()).unwrap(), layout);

        let off = base.replacen("\"pages\"", "\"fullScreen\":false,\"pages\"", 1);
        let layout = Layout::parse(off.as_bytes()).unwrap();
        assert_eq!(layout, plain);
        assert_eq!(layout.to_canonical_json(), plain.to_canonical_json());

        for value in ["1", "0", "\"true\"", "null", "[]", "{}"] {
            let text = base.replacen("\"pages\"", &format!("\"fullScreen\":{value},\"pages\""), 1);
            assert_eq!(
                keys(&Layout::parse(text.as_bytes()).unwrap_err()),
                ["shape.type"],
                "{value}"
            );
        }

        let example = std::fs::read(schema_dir().join("examples/full-screen.json")).unwrap();
        assert!(Layout::parse(&example).unwrap().full_screen);
    }

    #[test]
    fn a_dashboard_with_tiles_is_an_unknown_field() {
        let text = r#"{"schema":1,"name":"D","orientation":"portrait","pages":[{"id":"d","title":"D","kind":"dashboard","source":"cliproxyapi","tiles":[]}]}"#;
        assert_eq!(
            keys(&Layout::parse(text.as_bytes()).unwrap_err()),
            ["shape.unknown-field"]
        );
    }
}

impl Layout {
    /// Check destination support against hello.sinks before transferring a Layout.
    pub fn validate_sinks(&self, sinks: &[String]) -> Result<(), Vec<LayoutError>> {
        let mut errors = Vec::new();
        for (pi, page) in self.pages.iter().enumerate() {
            if let Page::Grid { tiles, .. } = page {
                for (ti, tile) in tiles.iter().enumerate() {
                    let (sink, usage) = match &tile.action {
                        Action::Key { sink, .. } => (sink, None),
                        Action::Consumer { sink, usage } => (sink, Some(*usage)),
                        _ => continue,
                    };
                    if sink.as_ref() == Some(&ActionSink::Bluetooth)
                        && (!sinks.iter().any(|s| s == "bluetooth")
                            || usage.is_some_and(|u| u > 1023))
                    {
                        errors.push(LayoutError { rule: Rule::ActionSink, path: format!("/pages/{pi}/tiles/{ti}/action/sink"), message: "Bluetooth destination is unsupported on this Device or for this media key.".into() });
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
