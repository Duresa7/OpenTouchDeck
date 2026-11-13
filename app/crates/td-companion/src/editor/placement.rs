use super::{EditorError, Session};
use td_core::{
    editing::{Appearance, Document, Edit, EditError},
    layout::{Action, Modifier, Page, Tile},
};
use td_host::integrations::{DataStream, PresetTile};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddKind {
    Key,
    Launch,
    Consumer,
    Page,
    Mod,
    Open,
    Command,
    None,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddTarget {
    Auto,
    Cell { row: u8, col: u8 },
    NewPage,
}

impl Session {
    /// Preview the default Add Tile destination, accounting for spanning Tiles.
    pub fn add_target(&self) -> Result<AddTarget, EditorError> {
        let layout = self.draft().ok_or(EditorError::NoDraft)?.layout();
        if let Some(Page::Grid {
            rows, cols, tiles, ..
        }) = layout
            .pages
            .iter()
            .find(|p| Some(p.id()) == self.page.as_deref())
        {
            for row in 0..*rows {
                for col in 0..*cols {
                    if !tiles.iter().any(|t| {
                        row >= t.row
                            && row < t.row + t.row_span
                            && col >= t.col
                            && col < t.col + t.col_span
                    }) {
                        return Ok(AddTarget::Cell { row, col });
                    }
                }
            }
        }
        Ok(AddTarget::NewPage)
    }
    pub fn add(&mut self, kind: AddKind, target: AddTarget) -> Result<String, EditorError> {
        let pages = &self.draft().ok_or(EditorError::NoDraft)?.layout().pages;
        let action = match kind {
            AddKind::Key => Action::Key {
                usage: 58,
                mods: vec![],
                sink: None,
            },
            AddKind::Consumer => Action::Consumer {
                usage: 205,
                sink: None,
            },
            AddKind::Mod => Action::Mod {
                modifier: Modifier::Ctrl,
            },
            AddKind::Page => Action::Page {
                page: pages.get(1).unwrap_or(&pages[0]).id().into(),
            },
            _ => Action::None,
        };
        let pending = match kind {
            AddKind::Launch => Some(Action::Launch {
                path: String::new(),
                args: vec![],
                cwd: String::new(),
            }),
            AddKind::Open => Some(Action::Open {
                target: String::new(),
            }),
            AddKind::Command => Some(Action::Command {
                program: String::new(),
                args: vec![],
                cwd: String::new(),
                shell: false,
            }),
            _ => None,
        };
        self.place(target, "Tile".into(), action, None, pending)
    }
    /// Build the same validated 1×1 Tile for both preview and placement.
    pub fn preset_tile(
        &self,
        preset: &PresetTile,
        integration: &str,
        streams: &[DataStream],
        draws_graphs: bool,
    ) -> Result<Tile, EditorError> {
        let action = serde_json::from_value(preset.action.clone())
            .map_err(|_| EditError::LiveActionRequired)?;
        let mut tile = plain_tile("preset".into(), preset_label(&preset.name), action, 0, 0);
        let layout = td_core::Layout {
            schema: 1,
            name: "Preview".into(),
            orientation: self.orientation,
            full_screen: false,
            pages: vec![Page::Grid {
                id: "preview".into(),
                title: "Preview".into(),
                rows: 1,
                cols: 1,
                tiles: vec![tile.clone()],
            }],
        };
        let mut doc = Document::admit(layout)?;
        if let Some(appearance) =
            super::super::placed_appearance(preset, integration, streams, draws_graphs)
        {
            doc = doc
                .apply(&Edit::Renderer {
                    tile: "preset".into(),
                    appearance,
                })?
                .document;
            if let Page::Grid { tiles, .. } = &doc.layout().pages[0] {
                tile = tiles[0].clone();
            }
        }
        Ok(tile)
    }
    pub fn place_preset(
        &mut self,
        preset: &PresetTile,
        integration: &str,
        streams: &[DataStream],
        draws_graphs: bool,
        target: AddTarget,
    ) -> Result<String, EditorError> {
        let tile = self.preset_tile(preset, integration, streams, draws_graphs)?;
        let appearance =
            super::super::placed_appearance(preset, integration, streams, draws_graphs);
        self.place(target, tile.label, tile.action, appearance, None)
    }
    fn place(
        &mut self,
        target: AddTarget,
        label: String,
        action: Action,
        appearance: Option<Appearance>,
        pending: Option<Action>,
    ) -> Result<String, EditorError> {
        let target = if target == AddTarget::Auto {
            self.add_target()?
        } else {
            target
        };
        let layout = self.draft().ok_or(EditorError::NoDraft)?.layout();
        let fresh = |prefix: &str| {
            (1..).map(|n| format!("{prefix}{n}")).find(|id| !layout.pages.iter().any(|p| p.id() == id || matches!(p, Page::Grid { tiles, .. } if tiles.iter().any(|t| t.id == *id)))).unwrap()
        };
        let id = fresh("tile");
        let mut model = self.model.clone();
        let (page, row, col) = match target {
            AddTarget::NewPage => {
                let page = fresh("page");
                model.edit(
                    self.orientation,
                    Edit::AddPage {
                        page: Page::Grid {
                            id: page.clone(),
                            title: format!("Page {}", layout.pages.len() + 1),
                            rows: 3,
                            cols: 4,
                            tiles: vec![],
                        },
                    },
                )?;
                (page, 0, 0)
            }
            AddTarget::Cell { row, col } => {
                (self.page.clone().ok_or(EditorError::NoDraft)?, row, col)
            }
            AddTarget::Auto => unreachable!(),
        };
        model.edit(
            self.orientation,
            Edit::Place {
                page: page.clone(),
                tile: plain_tile(id.clone(), label, action, row, col),
            },
        )?;
        if let Some(appearance) = appearance {
            model.edit(
                self.orientation,
                Edit::Renderer {
                    tile: id.clone(),
                    appearance,
                },
            )?;
        }
        if let Some(action) = pending {
            let _ = model.edit(
                self.orientation,
                Edit::Action {
                    tile: id.clone(),
                    action,
                },
            );
        }
        let mut replacements = self.replacements.clone();
        replacements.extend(self.handle.brandfetch.replacements());
        model.replace_assets(replacements)?;
        model.persist()?;
        self.model = model;
        self.page = Some(page);
        self.tile = Some(id.clone());
        Ok(id)
    }
}
fn plain_tile(id: String, label: String, action: Action, row: u8, col: u8) -> Tile {
    Tile {
        id,
        label,
        action,
        row,
        col,
        row_span: 1,
        col_span: 1,
        icon: None,
        asset: None,
        render: None,
        streams: None,
        input: false,
    }
}
pub(super) fn preset_label(name: &str) -> String {
    let name = name.trim();
    let mut chars = name.chars();
    let sentence = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default();
    let mut label = String::new();
    for grapheme in sentence.graphemes(true) {
        if label.chars().count() + grapheme.chars().count() > 16 {
            break;
        }
        label.push_str(grapheme);
    }
    if label.trim().is_empty() {
        "Tile".into()
    } else {
        label.trim().into()
    }
}
