//! Pure version-4 Shell callback decoding, also exercised on non-Windows hosts.

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    Open,
    Menu { x: i32, y: i32 },
}

pub(super) fn route(wparam: usize, lparam: isize) -> Option<Action> {
    // Shell puts the icon ID in HIWORD(lParam), the event in LOWORD(lParam),
    // and signed screen coordinates in wParam with NOTIFYICON_VERSION_4.
    match lparam as u16 {
        // NIN_SELECT (mouse activation) and NIN_KEYSELECT (Enter/Space).
        0x0400 | 0x0401 => Some(Action::Open),
        // WM_CONTEXTMENU covers both right-click and the context-menu key.
        0x007b => Some(Action::Menu {
            x: wparam as u16 as i16 as i32,
            y: (wparam >> 16) as u16 as i16 as i32,
        }),
        // Version 4 also sends raw mouse events. Ignore them so one click
        // cannot open the app/menu twice.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_and_keyboard_selection_open_the_app() {
        for event in [0x0400, 0x0401] {
            for id in [0, 1, 0xffff] {
                assert_eq!(route(0, (id << 16) | event), Some(Action::Open));
            }
        }
    }

    #[test]
    fn right_click_and_context_key_show_the_menu_at_the_signed_anchor() {
        for (x, y) in [(120, 240), (-1500, 42), (-1, -1)] {
            let packed = (x as u16 as usize) | ((y as u16 as usize) << 16);
            assert_eq!(route(packed, 0xbeef007b), Some(Action::Menu { x, y }));
        }
    }

    #[test]
    fn raw_mouse_and_unrelated_events_do_not_duplicate_activation() {
        for event in [0x0201, 0x0202, 0x0203, 0x0204, 0x0205, 0x0200, 0x0402] {
            assert_eq!(route(0, event), None);
        }
    }
}
