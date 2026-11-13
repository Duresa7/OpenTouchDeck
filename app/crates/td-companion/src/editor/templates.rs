use super::{EditorError, Session};
use td_core::{
    Layout,
    layout::{Action, Modifier, Page, Tile},
};
#[derive(Debug, Clone, Copy)]
pub enum NewLayout {
    Blank,
    Starter,
}
impl Session {
    pub fn new_layout(&mut self, kind: NewLayout) -> Result<(), EditorError> {
        let mut layout = Layout {
            schema: 1,
            name: "New Layout".into(),
            orientation: self.orientation,
            full_screen: false,
            pages: vec![Page::Grid {
                id: "page1".into(),
                title: "Page 1".into(),
                rows: 3,
                cols: 4,
                tiles: vec![],
            }],
        };
        if matches!(kind, NewLayout::Starter) {
            layout.name = "Starter".into();
            let media = [
                ("prev", "Previous", 182, "lv:prev"),
                ("playpause", "Play/Pause", 205, "lv:play_pause"),
                ("next", "Next", 181, "lv:next"),
                ("mute", "Mute", 226, "lv:mute"),
                ("voldown", "Volume down", 234, "lv:volume_mid"),
                ("volup", "Volume up", 233, "lv:volume_max"),
            ]
            .into_iter()
            .enumerate()
            .map(|(n, (id, label, usage, icon))| {
                tile(
                    id,
                    label,
                    (n / 3) as u8,
                    (n % 3) as u8,
                    Some(icon),
                    Action::Consumer { usage, sink: None },
                )
            })
            .collect();
            let shortcuts = [
                ("copy", "Copy", 6, vec![Modifier::Ctrl]),
                ("paste", "Paste", 25, vec![Modifier::Ctrl]),
                ("cut", "Cut", 27, vec![Modifier::Ctrl]),
                ("selectall", "Select all", 4, vec![Modifier::Ctrl]),
                ("undo", "Undo", 29, vec![Modifier::Ctrl]),
                ("redo", "Redo", 28, vec![Modifier::Ctrl]),
                ("save", "Save", 22, vec![Modifier::Ctrl]),
                ("find", "Find", 9, vec![Modifier::Ctrl]),
                (
                    "snip",
                    "Screenshot",
                    22,
                    vec![Modifier::Shift, Modifier::Win],
                ),
                ("desktop", "Show desktop", 7, vec![Modifier::Win]),
            ]
            .into_iter()
            .enumerate()
            .map(|(n, (id, label, usage, mods))| {
                tile(
                    id,
                    label,
                    (n / 4) as u8,
                    (n % 4) as u8,
                    None,
                    Action::Key {
                        usage,
                        mods,
                        sink: None,
                    },
                )
            })
            .collect();
            layout.pages = vec![
                Page::Grid {
                    id: "media".into(),
                    title: "Media".into(),
                    rows: 2,
                    cols: 3,
                    tiles: media,
                },
                Page::Grid {
                    id: "shortcuts".into(),
                    title: "Shortcuts".into(),
                    rows: 3,
                    cols: 4,
                    tiles: shortcuts,
                },
            ];
        }
        self.start(layout)
    }
}
fn tile(id: &str, label: &str, row: u8, col: u8, icon: Option<&str>, action: Action) -> Tile {
    Tile {
        id: id.into(),
        label: label.into(),
        row,
        col,
        row_span: 1,
        col_span: 1,
        icon: icon.map(str::to_owned),
        asset: None,
        render: None,
        streams: None,
        input: false,
        action,
    }
}
