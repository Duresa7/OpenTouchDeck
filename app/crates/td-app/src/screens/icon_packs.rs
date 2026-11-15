//! Icon packs: its row in the Integrations list and its page, fed from the `IconPacks` global. A slot
//! for its session; until it lands, the row and the page show their fixed words.
use super::Screens;
use crate::AppWindow;
use std::rc::Rc;

impl Screens {
    pub(super) fn wire_icon_packs(self: &Rc<Self>, _ui: &AppWindow) {}

    pub(super) fn refresh_icon_packs(&self, _ui: &AppWindow) {}
}
