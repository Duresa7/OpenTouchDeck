//! Toolbar fields map directly to typed edits. File picking and reads run off the UI thread.
use super::*;
use crate::{EditorField, EditorTools, PresetChoice, ToolbarState};
use slint::ComponentHandle;
use td_companion::{
    devices::{ControlLayout, ControlLayoutUnavailable},
    editor::{GifEdit, GifWork},
    features::assets::Fit,
};
use td_core::{
    Orientation,
    editing::Appearance,
    layout::{Action, ActionSink, Tile, TileRender},
};

impl Screens {
    pub(super) fn wire_toolbar(self: &Rc<Self>, ui: &AppWindow) {
        let tools = ui.global::<EditorTools>();
        tools.set_fields(ModelRc::from(self.editor.models.fields.clone()));
        tools.set_presets(ModelRc::from(self.editor.models.presets.clone()));
        on!(
            self,
            tools,
            on_edit,
            |s, key: SharedString, value: SharedString| {
                let result = s.with_shown(|view, _| command(view, &key, &value));
                match result {
                    Some(Ok(command)) => {
                        s.dispatch(command);
                        if let Some(ui) = s.ui.upgrade() {
                            ui.global::<EditorTools>().set_error("".into());
                        }
                    }
                    Some(Err(error)) => {
                        if let Some(ui) = s.ui.upgrade() {
                            ui.global::<EditorTools>().set_error(error.into());
                        }
                    }
                    None => {}
                }
            }
        );
        on!(self, tools, on_done, |s| {
            if let Some(page) = s.with_shown(|view, _| view.page.clone()).flatten() {
                s.dispatch(Command::Select { page, tile: None });
            }
        });
        on!(self, tools, on_open_picker, |s, row: i32, col: i32| s
            .open_picker(row, col));
        on!(self, tools, on_add, |s, kind: SharedString| {
            if let (Some(kind), Some(target)) = (add_kind(&kind), s.editor.add_target.get()) {
                s.dispatch(Command::Add { kind, target });
            }
        });
        on!(
            self,
            tools,
            on_preset,
            |s, integration: SharedString, preset: SharedString| {
                if let Some(target) = s.editor.add_target.get()
                    && let Some(summary) = s
                        .handle
                        .integrations()
                        .summaries()
                        .into_iter()
                        .find(|i| i.id == integration.as_str() && i.enabled)
                    && let Some(preset) = summary
                        .presets
                        .iter()
                        .find(|p| p.id == preset.as_str())
                        .cloned()
                {
                    let draws_graphs = s
                        .with_shown(|v, _| td_companion::devices::DeviceId::parse(&v.device))
                        .flatten()
                        .and_then(|id| s.handle.devices().get(&id))
                        .and_then(|d| d.capabilities)
                        .is_some_and(|c| c.tile_kinds.iter().any(|k| k == "graph"));
                    s.dispatch(Command::PlacePreset {
                        integration: integration.into(),
                        preset,
                        streams: summary.streams,
                        draws_graphs,
                        target,
                    });
                }
            }
        );
        on!(self, tools, on_gif, |s, operation: SharedString| {
            if let Some(tile) = s.with_shown(|view, _| view.tile.clone()).flatten() {
                let edit = match operation.as_str() {
                    "cancel" => GifEdit::Cancel,
                    "retry" => GifEdit::Retry,
                    "cover" => GifEdit::Fit(Fit::Cover),
                    "contain" => GifEdit::Fit(Fit::Contain),
                    _ => return,
                };
                s.dispatch(Command::Gif { tile, edit });
            }
        });
        on!(self, tools, on_pick_gif, |s, add: bool| s.pick_gif(add));
    }

    pub(super) fn open_picker(&self, row: i32, col: i32) {
        let target = if row >= 0 && col >= 0 {
            u8::try_from(row)
                .ok()
                .zip(u8::try_from(col).ok())
                .map(|(row, col)| AddTarget::Cell { row, col })
        } else {
            self.with_shown(|view, _| view.add_target).flatten()
        };
        self.editor.add_target.set(target);
        if target.is_some()
            && let Some(ui) = self.ui.upgrade()
        {
            let tools = ui.global::<EditorTools>();
            let mut state = tools.get_state();
            state.destination = destination(target).into();
            tools.set_state(state);
            tools.set_error("".into());
            tools.set_popup("add".into());
        }
    }

    pub(super) fn show_toolbar(&self, ui: &AppWindow) {
        let tools = ui.global::<EditorTools>();
        if let Some((mut state, fields)) = self.with_shown(|view, _| map_toolbar(view)) {
            if tools.get_popup() == "add" {
                state.destination = destination(self.editor.add_target.get()).into();
            }
            if tools.get_state().tile != state.tile || tools.get_state().context != state.context {
                tools.set_popup(
                    if state.problem.is_empty() {
                        ""
                    } else {
                        "action"
                    }
                    .into(),
                );
                tools.set_error("".into());
            }
            tools.set_state(state);
            sync(&self.editor.models.fields, fields);
        }
        let presets = self
            .handle
            .integrations()
            .summaries()
            .into_iter()
            .filter(|i| i.enabled)
            .flat_map(|i| {
                i.presets.into_iter().map(move |p| PresetChoice {
                    integration: i.id.as_str().into(),
                    id: p.id.into(),
                    label: format!("{}: {}", i.name, p.name).into(),
                })
            })
            .collect();
        sync(&self.editor.models.presets, presets);
    }

    fn pick_gif(&self, add: bool) {
        let Some(worker) = self.editor.worker.borrow().clone() else {
            return;
        };
        let tile = self.with_shown(|view, _| view.tile.clone()).flatten();
        let context = self.with_shown(|view, _| (view.orientation, view.page.clone()));
        let target = self.editor.add_target.get();
        let ui = self.ui.clone();
        self.runtime.spawn(async move {
            let picked = tokio::task::spawn_blocking(pick_gif_file).await;
            let result = match picked {
                Ok(Ok(Some((file, bytes)))) => {
                    let now = worker.view();
                    if context != Some((now.orientation, now.page.clone())) {
                        let _ = ui.upgrade_in_event_loop(|ui| {
                            ui.global::<EditorTools>().set_error("The Page changed while choosing the GIF. Choose it again on the intended Page.".into());
                            ui.global::<EditorTools>().set_popup("look".into());
                        });
                        return;
                    }
                    if add {
                        if let Some(target) = target {
                            worker.dispatch(Command::AddGif {
                                file,
                                bytes,
                                target,
                            });
                        }
                    } else if let Some(tile) = tile {
                        worker.dispatch(Command::Gif {
                            tile,
                            edit: GifEdit::Choose { file, bytes },
                        });
                    }
                    Ok(())
                }
                Ok(Ok(None)) => Ok(()),
                _ => Err("Couldn't read that GIF. Choose a readable GIF file and retry."),
            };
            if let Err(error) = result {
                let _ = ui.upgrade_in_event_loop(move |ui| {
                    ui.global::<EditorTools>().set_error(error.into());
                    ui.global::<EditorTools>().set_popup("look".into());
                });
            }
        });
    }
}

fn pick_gif_file() -> Result<Option<(String, Vec<u8>)>, ()> {
    #[cfg(windows)]
    let mut process = {
        use std::os::windows::process::CommandExt;
        let mut process = std::process::Command::new("powershell.exe");
        process.args(["-NoProfile", "-NonInteractive", "-STA", "-Command",
            r#"Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Filter = 'GIF (*.gif)|*.gif'; $d.Title = 'Choose GIF'; if ($d.ShowDialog() -eq 'OK') { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; [Console]::Write($d.FileName) }; $d.Dispose()"#]);
        process.creation_flags(0x08000000);
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = std::process::Command::new("zenity");
        process.args([
            "--file-selection",
            "--title=Choose GIF",
            "--file-filter=*.gif",
        ]);
        process
    };
    let output = process.output().map_err(|_| ())?;
    if !output.status.success() {
        return if output.status.code() == Some(1) {
            Ok(None)
        } else {
            Err(())
        };
    }
    let path = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if path.as_os_str().is_empty() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|_| ())?;
    let file = path.file_name().ok_or(())?.to_string_lossy().into_owned();
    Ok(Some((file, bytes)))
}

fn add_kind(kind: &str) -> Option<AddKind> {
    Some(match kind {
        "key" => AddKind::Key,
        "consumer" => AddKind::Consumer,
        "mod" => AddKind::Mod,
        "page" => AddKind::Page,
        "launch" => AddKind::Launch,
        "open" => AddKind::Open,
        "command" => AddKind::Command,
        "none" => AddKind::None,
        _ => return None,
    })
}
fn destination(target: Option<AddTarget>) -> String {
    match target {
        Some(AddTarget::Cell { row, col }) => format!("Add at row {}, column {}", row + 1, col + 1),
        Some(AddTarget::NewPage) => "Add on a new Page".into(),
        _ => String::new(),
    }
}
pub(super) fn gif_status(work: Option<&GifWork>, has_asset: bool) -> String {
    match work {
        Some(GifWork::Queued { file }) => format!("Queued: {file}"),
        Some(GifWork::Preparing { file }) => format!("Preparing: {file}"),
        Some(GifWork::Failed { problem, .. }) => format!("GIF failed: {}", problem.text),
        None if has_asset => "GIF ready".into(),
        None => String::new(),
    }
}
pub(super) fn controls_notice(state: &ControlLayout) -> &'static str {
    match state {
        ControlLayout::Matches { .. } => "",
        ControlLayout::NoPushedCopy {
            has_controls: true, ..
        } => "Controls are paused. Send this Layout to create a Pushed copy.",
        ControlLayout::Differs {
            has_controls: true, ..
        } => {
            "Controls are paused. The Device's Layout differs from its Pushed copy. Send this Layout again."
        }
        ControlLayout::NoPushedCopy {
            has_controls: false,
            ..
        }
        | ControlLayout::Differs {
            has_controls: false,
            ..
        } => "",
        ControlLayout::Unavailable { reason } => match reason {
            ControlLayoutUnavailable::Checking => "Checking Controls…",
            ControlLayoutUnavailable::Offline => "Controls are paused while the Device is offline.",
            ControlLayoutUnavailable::Unsupported => "This Firmware doesn't support Controls.",
            ControlLayoutUnavailable::ReadFailed => {
                "Couldn't check Controls. Reconnect the Device to retry."
            }
        },
    }
}
fn map_toolbar(view: &EditorView) -> (ToolbarState, Vec<EditorField>) {
    let Some((layout, _, page)) = current(view) else {
        return (
            ToolbarState {
                portrait: view.orientation == Orientation::Portrait,
                ..Default::default()
            },
            vec![],
        );
    };
    let selected = page
        .tiles
        .iter()
        .find(|t| Some(&t.tile.id) == view.tile.as_ref());
    let mut state = ToolbarState {
        context: format!("{}:{:?}:{}", view.device, view.orientation, page.id).into(),
        page: page.title.as_str().into(),
        layout: layout.name.as_str().into(),
        rows: page.rows.into(),
        cols: page.cols.into(),
        usage: page.usage,
        full_screen: layout.full_screen,
        portrait: view.orientation == Orientation::Portrait,
        offers_gif: view.offers_gif,
        can_add: view.add_target.is_some()
            && !(view.add_target == Some(AddTarget::NewPage) && layout.pages.len() >= MAX_PAGES),
        destination: destination(view.add_target).into(),
        ..Default::default()
    };
    let Some(selected) = selected else {
        return (state, vec![]);
    };
    let t = &selected.tile;
    state.selected = true;
    state.tile = t.id.as_str().into();
    state.label = t.label.as_str().into();
    state.row = i32::from(t.row) + 1;
    state.col = i32::from(t.col) + 1;
    state.row_span = t.row_span.into();
    state.col_span = t.col_span.into();
    state.problem = selected
        .problems
        .iter()
        .map(problem_text)
        .collect::<Vec<_>>()
        .join(" ")
        .into();
    state.gif_status = gif_status(
        selected.gif.as_ref(),
        t.asset.is_some() && t.render == Some(TileRender::Gif),
    )
    .into();
    state.gif_busy = matches!(
        selected.gif,
        Some(GifWork::Queued { .. } | GifWork::Preparing { .. })
    );
    state.gif_retry = matches!(selected.gif, Some(GifWork::Failed { retry: true, .. }));
    state.gif_cover = selected.gif_fit == Fit::Cover;
    state.sink_applicable = matches!(t.action, Action::Key { .. } | Action::Consumer { .. });
    state.bluetooth = matches!(
        t.action,
        Action::Key {
            sink: Some(ActionSink::Bluetooth),
            ..
        } | Action::Consumer {
            sink: Some(ActionSink::Bluetooth),
            ..
        }
    );
    state.input_applicable = t.render == Some(TileRender::Bar);
    state.input = t.input;
    let (action, fields) = action_fields(&t.action);
    state.action = action.into();
    (state, fields)
}
fn field(key: &str, label: &str, value: impl Into<String>, hint: &str) -> EditorField {
    EditorField {
        key: key.into(),
        label: label.into(),
        value: value.into().into(),
        hint: hint.into(),
    }
}
fn action_fields(action: &Action) -> (&'static str, Vec<EditorField>) {
    let json = |v: &Vec<String>| serde_json::to_string(v).expect("strings serialize");
    match action {
        Action::Key { usage, mods, .. } => (
            "Key",
            vec![
                field(
                    "usage",
                    "Key usage",
                    usage.to_string(),
                    "HID key usage, 0–255",
                ),
                field(
                    "mods",
                    "Modifiers",
                    mods.iter()
                        .map(|m| {
                            serde_json::to_value(m)
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                    "ctrl, shift, alt, win; separated by commas",
                ),
            ],
        ),
        Action::Consumer { usage, .. } => (
            "Media key",
            vec![field(
                "usage",
                "Media usage",
                usage.to_string(),
                "HID media usage, 0–65535",
            )],
        ),
        Action::Mod { modifier } => (
            "Modifier",
            vec![field(
                "mod",
                "Modifier",
                serde_json::to_value(modifier).unwrap().as_str().unwrap(),
                "ctrl, shift, alt or win",
            )],
        ),
        Action::Page { page } => (
            "Go to Page",
            vec![field(
                "target-page",
                "Target Page ID",
                page.clone(),
                "ID of a Page in this Layout",
            )],
        ),
        Action::Launch { path, args, cwd } => (
            "Launch program",
            vec![
                field("path", "Program path", path.clone(), ""),
                field(
                    "args",
                    "Arguments",
                    json(args),
                    "JSON array of separate arguments, for example [\"--help\"]",
                ),
                field("cwd", "Working folder", cwd.clone(), "Optional"),
            ],
        ),
        Action::Command {
            program,
            args,
            cwd,
            shell,
        } => (
            "Run command",
            vec![
                field("program", "Program", program.clone(), ""),
                field(
                    "args",
                    "Arguments",
                    json(args),
                    "JSON array of separate arguments",
                ),
                field("cwd", "Working folder", cwd.clone(), "Optional"),
                field("shell", "Use shell", shell.to_string(), "true or false"),
            ],
        ),
        Action::Open { target } => (
            "Open URL or file",
            vec![field("target", "URL or file", target.clone(), "")],
        ),
        Action::Widget {
            source,
            field: name,
            account,
        } => (
            "Integration action",
            vec![
                field("source", "Integration", source.clone(), ""),
                field("field", "Action or field", name.clone(), ""),
                field("account", "Account", account.clone(), "Optional"),
            ],
        ),
        Action::None => ("No action", vec![]),
    }
}
fn number(value: &str) -> Result<u8, &'static str> {
    value
        .parse()
        .map_err(|_| "Enter a whole number from 0 to 255.")
}
fn flag(value: &str) -> Result<bool, &'static str> {
    value.parse().map_err(|_| "Enter true or false.")
}
fn command(view: &EditorView, key: &str, value: &str) -> Result<Command, &'static str> {
    if key == "discard" {
        return Ok(Command::DiscardRejected);
    }
    if key == "orientation" {
        return Ok(Command::Orientation(if value == "portrait" {
            Orientation::Portrait
        } else {
            Orientation::Landscape
        }));
    }
    let (_, _, page) = current(view).ok_or("Start a Layout first.")?;
    let edit = match key {
        "page" => Edit::PageTitle {
            page: page.id.clone(),
            title: value.into(),
        },
        "name" => Edit::Name { name: value.into() },
        "full-screen" => Edit::FullScreen { on: flag(value)? },
        "rows" | "cols" => Edit::GridSize {
            page: page.id.clone(),
            rows: if key == "rows" {
                number(value)?
            } else {
                page.rows
            },
            cols: if key == "cols" {
                number(value)?
            } else {
                page.cols
            },
        },
        _ => {
            let t = page
                .tiles
                .iter()
                .find(|t| Some(&t.tile.id) == view.tile.as_ref())
                .map(|t| &t.tile)
                .ok_or("Select a Tile first.")?;
            let tile = t.id.clone();
            match key {
                "kind" => Edit::Action {
                    tile,
                    action: match add_kind(value).ok_or("Choose an action kind.")? {
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
                            modifier: td_core::layout::Modifier::Ctrl,
                        },
                        AddKind::Page => Action::Page {
                            page: page.id.clone(),
                        },
                        AddKind::Launch => Action::Launch {
                            path: String::new(),
                            args: vec![],
                            cwd: String::new(),
                        },
                        AddKind::Open => Action::Open {
                            target: String::new(),
                        },
                        AddKind::Command => Action::Command {
                            program: String::new(),
                            args: vec![],
                            cwd: String::new(),
                            shell: false,
                        },
                        AddKind::None => Action::None,
                    },
                },
                "label" => Edit::Label {
                    tile,
                    label: value.into(),
                },
                "row" | "col" => Edit::Move {
                    tile,
                    row: if key == "row" {
                        number(value)?.checked_sub(1).ok_or("Rows start at 1.")?
                    } else {
                        t.row
                    },
                    col: if key == "col" {
                        number(value)?.checked_sub(1).ok_or("Columns start at 1.")?
                    } else {
                        t.col
                    },
                },
                "row-span" | "col-span" => Edit::Resize {
                    tile,
                    row_span: if key == "row-span" {
                        number(value)?
                    } else {
                        t.row_span
                    },
                    col_span: if key == "col-span" {
                        number(value)?
                    } else {
                        t.col_span
                    },
                },
                "duplicate" => Edit::Duplicate { tile },
                "remove" => Edit::RemoveTile { tile },
                "bluetooth" => Edit::Sink {
                    tile,
                    sink: if flag(value)? {
                        ActionSink::Bluetooth
                    } else {
                        ActionSink::Link
                    },
                },
                "input" => Edit::Input {
                    tile,
                    on: flag(value)?,
                },
                "plain" => Edit::Renderer {
                    tile,
                    appearance: Appearance::Plain {
                        action: None,
                        icon: None,
                        asset: None,
                    },
                },
                _ => Edit::Action {
                    tile,
                    action: change_action(t, key, value)?,
                },
            }
        }
    };
    Ok(Command::Edit(edit))
}
fn change_action(tile: &Tile, key: &str, value: &str) -> Result<Action, &'static str> {
    let mut action = tile.action.clone();
    let modifier = |text: &str| {
        serde_json::from_value(serde_json::Value::String(text.trim().to_lowercase()))
            .map_err(|_| "Use ctrl, shift, alt or win.")
    };
    match (&mut action, key) {
        (Action::Key { usage, .. }, "usage") => *usage = number(value)?,
        (Action::Consumer { usage, .. }, "usage") => {
            *usage = value
                .parse()
                .map_err(|_| "Enter a media usage from 0 to 65535.")?
        }
        (Action::Key { mods, .. }, "mods") => {
            *mods = value
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(modifier)
                .collect::<Result<_, _>>()?
        }
        (Action::Mod { modifier: m }, "mod") => *m = modifier(value)?,
        (Action::Page { page }, "target-page") => *page = value.into(),
        (Action::Launch { path, .. }, "path") => *path = value.into(),
        (Action::Command { program, .. }, "program") => *program = value.into(),
        (Action::Launch { args, .. } | Action::Command { args, .. }, "args") => {
            *args = serde_json::from_str(value)
                .map_err(|_| "Enter a JSON array of separate argument strings.")?
        }
        (Action::Launch { cwd, .. } | Action::Command { cwd, .. }, "cwd") => *cwd = value.into(),
        (Action::Command { shell, .. }, "shell") => *shell = flag(value)?,
        (Action::Open { target }, "target") => *target = value.into(),
        (Action::Widget { source, .. }, "source") => *source = value.into(),
        (Action::Widget { field, .. }, "field") => *field = value.into(),
        (Action::Widget { account, .. }, "account") => *account = value.into(),
        _ => return Err("This field doesn't apply to the selected Tile."),
    }
    Ok(action)
}
