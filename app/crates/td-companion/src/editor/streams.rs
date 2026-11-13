use super::Session;
use td_core::{
    editing::Appearance,
    layout::{Action, TileRender},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChoice {
    pub integration: String,
    pub stream: String,
    pub integration_name: String,
    pub name: String,
    pub unit: Option<String>,
}
impl Session {
    /// Every numeric stream, preserving Integration and stream order, named by its Preset.
    pub fn numeric_streams(&self) -> Vec<StreamChoice> {
        self.handle.integrations().summaries().iter().flat_map(|integration| {
            integration.streams.iter().filter(|s| s.kind == "numeric").map(|stream| {
                let preset = integration.presets.iter().find(|p| matches!(serde_json::from_value::<Action>(p.action.clone()), Ok(Action::Widget { source, field, .. }) if source == integration.id && field == stream.id));
                let name = preset.map(|p| super::placement::preset_label(&p.name)).unwrap_or_else(|| {
                    let words = stream.id.split(|c: char| c == '_' || c == '-' || c.is_whitespace()).filter(|s| !s.is_empty() && !["pct", "bytes", "s"].contains(s)).collect::<Vec<_>>().join(" ");
                    if words.is_empty() { return stream.id.clone(); }
                    let mut chars = words.chars();
                    chars.next().unwrap().to_uppercase().collect::<String>() + chars.as_str()
                });
                StreamChoice { integration: integration.id.clone(), stream: stream.id.clone(), integration_name: integration.name.clone(), name, unit: stream.unit.clone() }
            }).collect::<Vec<_>>()
        }).collect()
    }
    /// A bar is offered only by a matching Preset of the action's own Integration and stream.
    pub fn bar_appearance(&self, action: &Action) -> Option<Appearance> {
        let Action::Widget { source, field, .. } = action else {
            return None;
        };
        let summaries = self.handle.integrations().summaries();
        let integration = summaries.iter().find(|i| i.id == *source)?;
        integration.presets.iter().find_map(|p| {
            if !matches!(serde_json::from_value::<Action>(p.action.clone()), Ok(Action::Widget { field: bound, .. }) if bound == *field) { return None; }
            super::super::placed_appearance(p, &integration.id, &integration.streams, true).filter(|appearance| matches!(appearance, Appearance::Live { render: TileRender::Bar, .. }))
        })
    }
}
