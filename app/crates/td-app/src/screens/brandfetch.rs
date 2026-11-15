//! Brandfetch: its row in the Integrations list and its page, fed from the `Brandfetch` global. A slot
//! for its session; until it lands, the row and the page show their fixed words.
use super::Screens;
use crate::AppWindow;
use std::rc::Rc;

impl Screens {
    pub(super) fn wire_brandfetch(self: &Rc<Self>, _ui: &AppWindow) {}

    pub(super) fn refresh_brandfetch(&self, _ui: &AppWindow) {}
}
