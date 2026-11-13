//! Validated Layout editing for native adapters. Admission uses the file parser; edits either
//! produce a valid Document or leave the original untouched. Compatibility stays separate so
//! offline drafts remain editable.
use crate::layout::{Action, ActionSink, Layout, LayoutError, Page, StreamType, Tile, TileRender};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub rule: String,
    pub path: String,
    pub message: String,
}
impl From<LayoutError> for Problem {
    fn from(error: LayoutError) -> Self {
        Self {
            rule: error.rule.key().into(),
            path: error.path,
            message: error.message,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditError {
    Invalid(Vec<Problem>),
    MissingPage(String),
    NotGrid(String),
    MissingTile(String),
    NoSpace,
    SinkNotApplicable,
    LiveActionRequired,
    BarRequired,
}
impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EditError {}

/// A renderer change replaces its fields together. Assets use wire references (`sha256:…`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    Plain {
        /// Optional widget binding when returning a Live tile to its plain value.
        action: Option<Action>,
        icon: Option<String>,
        asset: Option<String>,
    },
    Gif {
        asset: String,
    },
    Live {
        render: TileRender,
        streams: BTreeMap<String, StreamType>,
        input: bool,
        action: Action,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edit {
    Name {
        name: String,
    },
    PageTitle {
        page: String,
        title: String,
    },
    GridSize {
        page: String,
        rows: u8,
        cols: u8,
    },
    FullScreen {
        on: bool,
    },
    Place {
        page: String,
        tile: Tile,
    },
    Move {
        tile: String,
        row: u8,
        col: u8,
    },
    Resize {
        tile: String,
        row_span: u8,
        col_span: u8,
    },
    Duplicate {
        tile: String,
    },
    RemoveTile {
        tile: String,
    },
    AddPage {
        page: Page,
    },
    RemovePage {
        page: String,
    },
    Label {
        tile: String,
        label: String,
    },
    Action {
        tile: String,
        action: Action,
    },
    Input {
        tile: String,
        on: bool,
    },
    Sink {
        tile: String,
        sink: ActionSink,
    },
    Renderer {
        tile: String,
        appearance: Appearance,
    },
    /// Simultaneous, one-step replacements; keys and values are full wire references.
    ReplaceAssets {
        replacements: BTreeMap<String, String>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document(Layout);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub document: Document,
    pub created_tile: Option<String>,
}
/// None means diagnostics are unknown, including an offline Device without cached diagnostics.
/// Some(empty) means Firmware reported no Bluetooth support.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities<'a> {
    pub sinks: Option<&'a [String]>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    Supported,
    Unknown,
    Unsupported(Vec<Problem>),
}
impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, EditError> {
        let mut layout = Layout::parse(bytes).map_err(invalid)?;
        // An explicit Companion sink means the same as none, and older Firmware refuses the field, so a Layout
        // imported or read from a Device never carries it into a push.
        for page in &mut layout.pages {
            if let Page::Grid { tiles, .. } = page {
                for tile in tiles {
                    normalize_sink(&mut tile.action);
                }
            }
        }
        Ok(Self(layout))
    }
    /// Public Layout fields cannot bypass admission into the editor.
    pub fn admit(layout: Layout) -> Result<Self, EditError> {
        Self::parse(&layout.to_canonical_json())
    }
    pub fn layout(&self) -> &Layout {
        &self.0
    }
    pub fn compatibility(&self, capabilities: Capabilities<'_>) -> Compatibility {
        match capabilities.sinks {
            Some(sinks) => match self.0.validate_sinks(sinks) {
                Ok(()) => Compatibility::Supported,
                Err(errors) => Compatibility::Unsupported(problems(errors)),
            },
            None => {
                // A too-wide Bluetooth usage is unsupported even without diagnostics.
                if let Err(errors) = self.0.validate_sinks(&["bluetooth".into()]) {
                    Compatibility::Unsupported(problems(errors))
                } else if self.0.validate_sinks(&[]).is_err() {
                    Compatibility::Unknown
                } else {
                    Compatibility::Supported
                }
            }
        }
    }
    pub fn apply(&self, edit: &Edit) -> Result<Change, EditError> {
        let mut next = self.0.clone();
        let mut created_tile = None;
        match edit {
            Edit::Name { name } => next.name = name.clone(),
            Edit::FullScreen { on } => next.full_screen = *on,
            Edit::PageTitle { page, title } => {
                match next.pages.iter_mut().find(|p| p.id() == page) {
                    Some(
                        Page::Grid { title: target, .. } | Page::Dashboard { title: target, .. },
                    ) => *target = title.clone(),
                    None => return Err(EditError::MissingPage(page.clone())),
                }
            }
            Edit::GridSize { page, rows, cols } => {
                match next.pages.iter_mut().find(|p| p.id() == page) {
                    Some(Page::Grid {
                        rows: r, cols: c, ..
                    }) => {
                        *r = *rows;
                        *c = *cols;
                    }
                    Some(_) => return Err(EditError::NotGrid(page.clone())),
                    None => return Err(EditError::MissingPage(page.clone())),
                }
            }
            Edit::Place { page, tile } => {
                let mut placed = tile.clone();
                normalize_sink(&mut placed.action);
                grid_mut(&mut next, page)?.push(placed);
                created_tile = Some(tile.id.clone());
            }
            Edit::Move { tile, row, col } => {
                let tile = tile_mut(&mut next, tile)?;
                tile.row = *row;
                tile.col = *col;
            }
            Edit::Resize {
                tile,
                row_span,
                col_span,
            } => {
                let tile = tile_mut(&mut next, tile)?;
                tile.row_span = *row_span;
                tile.col_span = *col_span;
            }
            Edit::Duplicate { tile } => {
                let id = fresh_tile_id(&next);
                let (rows, cols, tiles) = next
                    .pages
                    .iter_mut()
                    .find_map(|page| match page {
                        Page::Grid {
                            rows, cols, tiles, ..
                        } if tiles.iter().any(|t| t.id == *tile) => Some((*rows, *cols, tiles)),
                        _ => None,
                    })
                    .ok_or_else(|| EditError::MissingTile(tile.clone()))?;
                let mut copy = tiles.iter().find(|t| t.id == *tile).unwrap().clone();
                let count = u16::from(rows) * u16::from(cols);
                let start = u16::from(copy.row) * u16::from(cols) + u16::from(copy.col);
                let spot = (1..=count)
                    .map(|offset| (start + offset) % count)
                    .map(|cell| {
                        (
                            (cell / u16::from(cols)) as u8,
                            (cell % u16::from(cols)) as u8,
                        )
                    })
                    .find(|&(row, col)| fits(tiles, &copy, rows, cols, row, col))
                    .ok_or(EditError::NoSpace)?;
                copy.id = id.clone();
                (copy.row, copy.col) = spot;
                tiles.push(copy);
                created_tile = Some(id);
            }
            Edit::RemoveTile { tile } => {
                tile_mut(&mut next, tile)?;
                for page in &mut next.pages {
                    if let Page::Grid { tiles, .. } = page {
                        tiles.retain(|t| t.id != *tile);
                    }
                }
            }
            Edit::AddPage { page } => {
                let mut page = page.clone();
                if let Page::Grid { tiles, .. } = &mut page {
                    for tile in tiles {
                        normalize_sink(&mut tile.action);
                    }
                }
                next.pages.push(page);
            }
            Edit::RemovePage { page } => {
                if !next.pages.iter().any(|p| p.id() == page) {
                    return Err(EditError::MissingPage(page.clone()));
                }
                next.pages.retain(|p| p.id() != page);
                for other in &mut next.pages {
                    if let Page::Grid { tiles, .. } = other {
                        for tile in tiles {
                            if matches!(&tile.action, Action::Page { page: target } if target == page)
                            {
                                tile.action = Action::None;
                            }
                        }
                    }
                }
            }
            Edit::Label { tile, label } => tile_mut(&mut next, tile)?.label = label.clone(),
            Edit::Action { tile, action } => {
                let mut action = action.clone();
                normalize_sink(&mut action);
                tile_mut(&mut next, tile)?.action = action;
            }
            Edit::Input { tile, on } => {
                let target = tile_mut(&mut next, tile)?;
                if target.render != Some(TileRender::Bar) {
                    return Err(EditError::BarRequired);
                }
                target.input = *on;
            }
            Edit::Sink { tile, sink } => {
                let target = tile_mut(&mut next, tile)?;
                target.action = action_with_sink(target.action.clone(), sink.clone())?;
            }
            Edit::Renderer { tile, appearance } => {
                let tile = tile_mut(&mut next, tile)?;
                tile.icon = None;
                tile.asset = None;
                tile.render = None;
                tile.streams = None;
                tile.input = false;
                match appearance {
                    Appearance::Plain {
                        icon,
                        asset,
                        action,
                    } => {
                        if let Some(action) = action {
                            if !matches!(action, Action::Widget { .. }) {
                                return Err(EditError::LiveActionRequired);
                            }
                            tile.action = action.clone();
                        }
                        tile.icon = icon.clone();
                        tile.asset = asset.clone();
                    }
                    Appearance::Gif { asset } => {
                        tile.render = Some(TileRender::Gif);
                        tile.asset = Some(asset.clone());
                    }
                    Appearance::Live {
                        render,
                        streams,
                        input,
                        action,
                    } => {
                        if !matches!(action, Action::Widget { .. }) || *render == TileRender::Gif {
                            return Err(EditError::LiveActionRequired);
                        }
                        tile.render = Some(*render);
                        tile.streams = Some(streams.clone());
                        tile.input = *input;
                        tile.action = action.clone();
                    }
                }
            }
            Edit::ReplaceAssets { replacements } => {
                for page in &mut next.pages {
                    if let Page::Grid { tiles, .. } = page {
                        for tile in tiles {
                            if let Some(asset) = &tile.asset
                                && let Some(replacement) = replacements.get(asset)
                            {
                                tile.asset = Some(replacement.clone());
                            }
                        }
                    }
                }
            }
        }
        if matches!(
            edit,
            Edit::Renderer { .. }
                | Edit::Place { .. }
                | Edit::AddPage { .. }
                | Edit::RemoveTile { .. }
                | Edit::RemovePage { .. }
                | Edit::ReplaceAssets { .. }
        ) {
            next.schema = if next.pages.iter().any(|page| matches!(page, Page::Grid { tiles, .. }
                if tiles.iter().any(|tile| tile.asset.is_some() || tile.render.is_some() || tile.streams.is_some() || tile.input))) { 2 } else { 1 };
        }
        Ok(Change {
            document: Self::admit(next)?,
            created_tile,
        })
    }
}
/// Build a destination change without making a UI adapter know legacy wire defaults.
pub fn action_with_sink(mut action: Action, destination: ActionSink) -> Result<Action, EditError> {
    match &mut action {
        Action::Key { sink, .. } | Action::Consumer { sink, .. } => {
            *sink = (destination == ActionSink::Bluetooth).then_some(ActionSink::Bluetooth);
            Ok(action)
        }
        _ => Err(EditError::SinkNotApplicable),
    }
}
fn normalize_sink(action: &mut Action) {
    match action {
        Action::Key { sink, .. } | Action::Consumer { sink, .. }
            if *sink == Some(ActionSink::Link) =>
        {
            *sink = None
        }
        _ => {}
    }
}
fn problems(errors: Vec<LayoutError>) -> Vec<Problem> {
    errors.into_iter().map(Problem::from).collect()
}
fn invalid(errors: Vec<LayoutError>) -> EditError {
    EditError::Invalid(problems(errors))
}
fn grid_mut<'a>(layout: &'a mut Layout, id: &str) -> Result<&'a mut Vec<Tile>, EditError> {
    match layout.pages.iter_mut().find(|page| page.id() == id) {
        Some(Page::Grid { tiles, .. }) => Ok(tiles),
        Some(_) => Err(EditError::NotGrid(id.into())),
        None => Err(EditError::MissingPage(id.into())),
    }
}
fn tile_mut<'a>(layout: &'a mut Layout, id: &str) -> Result<&'a mut Tile, EditError> {
    layout
        .pages
        .iter_mut()
        .find_map(|page| match page {
            Page::Grid { tiles, .. } => tiles.iter_mut().find(|tile| tile.id == id),
            _ => None,
        })
        .ok_or_else(|| EditError::MissingTile(id.into()))
}
fn fresh_tile_id(layout: &Layout) -> String {
    (1..)
        .map(|n| format!("tile{n}"))
        .find(|id| {
            !layout.pages.iter().any(|page| page.id() == id
        || matches!(page, Page::Grid { tiles, .. } if tiles.iter().any(|tile| tile.id == *id)))
        })
        .unwrap()
}
fn fits(tiles: &[Tile], tile: &Tile, rows: u8, cols: u8, row: u8, col: u8) -> bool {
    let end_row = u16::from(row) + u16::from(tile.row_span);
    let end_col = u16::from(col) + u16::from(tile.col_span);
    end_row <= u16::from(rows)
        && end_col <= u16::from(cols)
        && tiles.iter().all(|other| {
            end_row <= u16::from(other.row)
                || u16::from(row) >= u16::from(other.row) + u16::from(other.row_span)
                || end_col <= u16::from(other.col)
                || u16::from(col) >= u16::from(other.col) + u16::from(other.col_span)
        })
}
