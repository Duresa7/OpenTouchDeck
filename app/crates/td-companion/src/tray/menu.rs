//! What the tray offers and what each choice does, kept apart from the Shell calls so it is
//! tested on every platform. `windows.rs` draws [`Menu::items`] and passes the user's choice to
//! [`Actions::run`].
use super::{OpenHook, routing};
use crate::{companion::Handle, startup::Startup};

/// One thing the user can do from the tray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Show, restore and bring the window to the front.
    Open,
    /// Turn Start at sign-in on (`true`) or off.
    StartAtSignIn(bool),
    /// End the program through the Companion's bounded shutdown.
    Quit,
}

/// One menu entry. `id` is the Win32 command ID; 0 means "nothing chosen", so IDs start at 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub id: usize,
    pub label: &'static str,
    /// `Some` for a check item, with its state.
    pub checked: Option<bool>,
    pub enabled: bool,
    /// The item left-click runs, shown in bold as Windows expects.
    pub default: bool,
}

const OPEN: usize = 1;
const START_AT_SIGN_IN: usize = 2;
const QUIT: usize = 3;

/// The menu as it is when the user opens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Menu {
    /// The effective Start at sign-in state (Run entry and StartupApproved), or `None` when it
    /// could not be read. Read each time the menu opens, so a change in Windows' Startup apps
    /// list shows.
    pub start_at_sign_in: Option<bool>,
}
impl Menu {
    pub fn read(startup: &dyn Startup) -> Self {
        Self {
            start_at_sign_in: startup.get().ok(),
        }
    }

    pub fn items(&self) -> [Item; 3] {
        [
            Item {
                id: OPEN,
                label: "Open",
                checked: None,
                enabled: true,
                default: true,
            },
            Item {
                id: START_AT_SIGN_IN,
                label: "Start at sign-in",
                checked: Some(self.start_at_sign_in == Some(true)),
                // An unreadable state can't be toggled safely: it might reverse the user's choice.
                enabled: self.start_at_sign_in.is_some(),
                default: false,
            },
            Item {
                id: QUIT,
                label: "Quit",
                checked: None,
                enabled: true,
                default: false,
            },
        ]
    }

    /// The choice behind a command ID that `TrackPopupMenu` returned. 0 (dismissed) and unknown
    /// IDs choose nothing.
    pub fn choose(&self, id: usize) -> Option<Choice> {
        match id {
            OPEN => Some(Choice::Open),
            START_AT_SIGN_IN => self.start_at_sign_in.map(|on| Choice::StartAtSignIn(!on)),
            QUIT => Some(Choice::Quit),
            _ => None,
        }
    }
}

/// What a Shell callback asks for before any menu is shown.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Activation {
    /// Left-click, Enter or Space on the icon.
    Run(Choice),
    /// Right-click or the context-menu key: show the menu at this point.
    Menu { x: i32, y: i32 },
}

pub(crate) fn activation(wparam: usize, lparam: isize) -> Option<Activation> {
    match routing::route(wparam, lparam)? {
        routing::Action::Open => Some(Activation::Run(Choice::Open)),
        routing::Action::Menu { x, y } => Some(Activation::Menu { x, y }),
    }
}

/// Carries out tray choices against the Companion and the window.
pub(crate) struct Actions {
    pub handle: Handle,
    pub open: OpenHook,
}
impl Actions {
    pub fn run(&self, choice: Choice) {
        match choice {
            Choice::Open => (self.open)(),
            Choice::StartAtSignIn(on) => {
                if self.handle.desktop.startup.set(on).is_err() {
                    self.handle
                        .desktop
                        .notify("Open TouchDeck could not change Start at sign-in.");
                }
            }
            // The window's event loop and the tray loop both follow this request; the program
            // then runs `Companion::shutdown` and drops the tray, each bounded.
            Choice::Quit => self.handle.request_shutdown(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::CommonError,
        companion::{Companion, Config},
        startup::FakeStartup,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    struct Unreadable;
    impl Startup for Unreadable {
        fn get(&self) -> Result<bool, CommonError> {
            Err(CommonError::Internal)
        }
        fn set(&self, _: bool) -> Result<(), CommonError> {
            panic!("an unreadable state is never written");
        }
    }

    fn labels(menu: Menu) -> Vec<&'static str> {
        menu.items().iter().map(|item| item.label).collect()
    }

    #[test]
    fn the_menu_is_open_start_at_sign_in_and_quit_with_no_browser_items() {
        let menu = Menu {
            start_at_sign_in: Some(false),
        };
        assert_eq!(labels(menu), ["Open", "Start at sign-in", "Quit"]);
        let items = menu.items();
        assert!(items.iter().all(|item| item.enabled));
        assert_eq!(
            items.iter().filter(|item| item.default).count(),
            1,
            "one default item"
        );
        assert!(items[0].default, "Open is the default, as left-click is");
        assert_eq!(
            items.iter().map(|item| item.checked).collect::<Vec<_>>(),
            [None, Some(false), None]
        );
        let mut ids: Vec<_> = items.iter().map(|item| item.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), 3, "distinct IDs");
        assert!(!ids.contains(&0), "0 means the menu was dismissed");
    }

    #[test]
    fn the_check_item_reads_the_real_startup_state_each_time() {
        let startup = FakeStartup::default();
        assert_eq!(Menu::read(&startup).items()[1].checked, Some(false));
        startup.set(true).unwrap();
        let menu = Menu::read(&startup);
        assert_eq!(menu.items()[1].checked, Some(true));
        assert_eq!(menu.choose(2), Some(Choice::StartAtSignIn(false)));
        startup.set(false).unwrap();
        assert_eq!(
            Menu::read(&startup).choose(2),
            Some(Choice::StartAtSignIn(true))
        );
    }

    #[test]
    fn an_unreadable_startup_state_shows_unchecked_and_disabled_and_cannot_be_chosen() {
        let menu = Menu::read(&Unreadable);
        assert_eq!(menu.start_at_sign_in, None);
        let item = &menu.items()[1];
        assert_eq!((item.checked, item.enabled), (Some(false), false));
        assert_eq!(menu.choose(item.id), None);
    }

    #[test]
    fn command_ids_map_to_choices_and_dismissal_chooses_nothing() {
        let menu = Menu {
            start_at_sign_in: Some(true),
        };
        let chosen: Vec<_> = menu
            .items()
            .iter()
            .map(|item| menu.choose(item.id))
            .collect();
        assert_eq!(
            chosen,
            [
                Some(Choice::Open),
                Some(Choice::StartAtSignIn(false)),
                Some(Choice::Quit)
            ]
        );
        for id in [0, 4, usize::MAX] {
            assert_eq!(menu.choose(id), None);
        }
    }

    #[test]
    fn left_click_and_keyboard_activation_open_and_the_context_menu_shows_the_menu() {
        for event in [0x0400, 0x0401] {
            assert_eq!(
                activation(0, (1 << 16) | event),
                Some(Activation::Run(Choice::Open))
            );
        }
        let packed = 120usize | (240usize << 16);
        assert_eq!(
            activation(packed, 0x007b),
            Some(Activation::Menu { x: 120, y: 240 })
        );
        assert_eq!(activation(0, 0x0202), None, "raw button-up is ignored");
    }

    async fn companion() -> (tempfile::TempDir, Companion) {
        let data = tempfile::tempdir().unwrap();
        let mut config = Config::mock(data.path().to_owned());
        config.devices = crate::devices::Config::mock(None);
        let companion = Companion::start(config).await.unwrap();
        (data, companion)
    }

    fn counting_hook() -> (Arc<AtomicUsize>, OpenHook) {
        let opened = Arc::new(AtomicUsize::new(0));
        let count = opened.clone();
        let hook: OpenHook = Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        (opened, hook)
    }

    #[tokio::test]
    async fn open_calls_the_hook_once_from_any_thread_and_nothing_else() {
        let (_data, companion) = companion().await;
        let (opened, open) = counting_hook();
        let actions = Arc::new(Actions {
            handle: companion.handle(),
            open,
        });
        actions.run(Choice::Open);
        // The tray runs choices on its own thread, not the window's.
        let tray = actions.clone();
        std::thread::spawn(move || tray.run(Choice::Open))
            .join()
            .unwrap();
        assert_eq!(opened.load(Ordering::SeqCst), 2);
        assert!(!companion.handle().shutdown_requested());
        companion.shutdown().await;
    }

    #[tokio::test]
    async fn start_at_sign_in_writes_the_chosen_state_without_opening() {
        let (_data, companion) = companion().await;
        let (opened, open) = counting_hook();
        let handle = companion.handle();
        let actions = Actions {
            handle: handle.clone(),
            open,
        };
        let startup = handle.desktop.startup.clone();
        actions.run(Menu::read(&*startup).choose(2).unwrap());
        assert!(startup.get().unwrap());
        actions.run(Menu::read(&*startup).choose(2).unwrap());
        assert!(!startup.get().unwrap());
        assert_eq!(opened.load(Ordering::SeqCst), 0);
        companion.shutdown().await;
    }

    #[tokio::test]
    async fn quit_requests_shutdown_and_the_companion_stops_within_its_bound() {
        let (_data, companion) = companion().await;
        let handle = companion.handle();
        let window_loop = {
            let handle = handle.clone();
            // Stands in for `App::run`, which quits its event loop on this signal.
            tokio::spawn(async move { handle.stopping().await })
        };
        let (opened, open) = counting_hook();
        Actions {
            handle: handle.clone(),
            open,
        }
        .run(Choice::Quit);
        assert!(handle.shutdown_requested());
        tokio::time::timeout(std::time::Duration::from_secs(5), window_loop)
            .await
            .expect("the window loop hears Quit")
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), companion.shutdown())
            .await
            .expect("shutdown is bounded");
        assert_eq!(opened.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_failed_startup_change_is_reported_not_ignored() {
        struct Refuses(AtomicBool);
        impl Startup for Refuses {
            fn get(&self) -> Result<bool, CommonError> {
                Ok(false)
            }
            fn set(&self, _: bool) -> Result<(), CommonError> {
                self.0.store(true, Ordering::SeqCst);
                Err(CommonError::Internal)
            }
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (_data, companion) = runtime.block_on(companion());
        let mut handle = companion.handle();
        let refuses = Arc::new(Refuses(AtomicBool::new(false)));
        let desktop = crate::tray::Desktop::new(true).with_startup(refuses.clone());
        let notices = desktop.captures.as_ref().unwrap().1.clone();
        handle.desktop = Arc::new(desktop);
        let (_, open) = counting_hook();
        let actions = Actions { handle, open };
        actions.run(Menu::read(&*refuses).choose(2).unwrap());
        assert!(refuses.0.load(Ordering::SeqCst));
        assert_eq!(
            notices.0.lock().take().as_deref(),
            Some("Open TouchDeck could not change Start at sign-in.")
        );
        runtime.block_on(companion.shutdown());
    }
}
