use std::collections::BTreeMap;
use td_core::{
    editing::Appearance,
    layout::{Action, StreamType, TileRender},
};
use td_host::integrations::{DataStream, PresetTile};

/// Renderer policy shared by placement and previews. Unsupported graphs fall back to their
/// plain value; bars and now-playing keep their renderer and can show a Firmware requirement.
pub fn placed_appearance(
    preset: &PresetTile,
    integration: &str,
    streams: &[DataStream],
    draws_graphs: bool,
) -> Option<Appearance> {
    let action: Action = serde_json::from_value(preset.action.clone()).ok()?;
    let Action::Widget { source, field, .. } = &action else {
        return None;
    };
    if source != integration {
        return None;
    }
    let render = preset.render?;
    if render == TileRender::Gif || (preset.input && render != TileRender::Bar) {
        return None;
    }
    let declared = preset
        .streams
        .clone()
        .unwrap_or_else(|| BTreeMap::from([("value".into(), StreamType::Number)]));
    let required: &[(&str, StreamType)] = match render {
        TileRender::Graph | TileRender::Bar => &[("value", StreamType::Number)],
        TileRender::NowPlaying => &[
            ("title", StreamType::Text),
            ("artist", StreamType::Text),
            ("cover", StreamType::Image),
            ("state", StreamType::Number),
        ],
        TileRender::Gif => return None,
    };
    if required
        .iter()
        .any(|(key, kind)| declared.get(*key) != Some(kind))
        || declared.iter().any(|(key, kind)| {
            !required.iter().any(|(k, t)| key == k && kind == t)
                && !(render == TileRender::Bar && key == "muted" && *kind == StreamType::Number)
        })
    {
        return None;
    }
    if render == TileRender::Graph
        && (!draws_graphs
            || !streams
                .iter()
                .any(|s| s.id == *field && s.kind == "numeric"))
    {
        return None;
    }
    Some(Appearance::Live {
        render,
        streams: declared,
        input: preset.input,
        action,
    })
}
