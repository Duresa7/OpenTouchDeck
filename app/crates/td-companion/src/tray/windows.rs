use super::{routing, *};
use std::{
    ptr::{null, null_mut},
    sync::{OnceLock, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{LIM_SMALL, LoadIconMetric},
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
    core::GUID,
};

static NOTICES: OnceLock<mpsc::SyncSender<String>> = OnceLock::new();
static EVENTS: OnceLock<mpsc::Sender<Event>> = OnceLock::new();
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
const CALLBACK: u32 = WM_APP + 1;
const STOP: u32 = WM_APP + 2;

#[derive(Default)]
struct State {
    // Store the HWND as an integer so the shared state can cross threads.
    hwnd: isize,
    stopping: bool,
}

pub(crate) struct Tray {
    thread: Option<JoinHandle<()>>,
    state: Arc<Mutex<State>>,
}

impl Drop for Tray {
    fn drop(&mut self) {
        {
            let mut state = self.state.lock();
            state.stopping = true;
            if state.hwnd != 0 {
                unsafe { PostMessageW(state.hwnd as HWND, STOP, 0, 0) };
            }
        }
        let thread = self.thread.take().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                eprintln!("Tray cleanup timed out; continuing shutdown.");
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.join().is_err() {
            eprintln!("The tray thread panicked during shutdown.");
        }
    }
}
enum Event {
    Callback(WPARAM, LPARAM),
    Recreate,
    ReloadIcon,
}
// Permanent identity: never change this GUID between versions or rebuilds.
const TRAY_GUID: GUID = GUID::from_u128(0xb48d722e_6be7_4f73_a636_cff1b93761c7);
// Matches app.rc; keep the current app icon until a replacement is selected.
const APP_ICON: *const u16 = std::ptr::without_provenance(1);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn set_app_identity() -> std::io::Result<()> {
    // Share the version-independent identity used by the Electron shell and
    // installer. Set it before creating any UI (including the hidden tray HWND).
    let result = unsafe {
        SetCurrentProcessExplicitAppUserModelID(wide(td_core::app_dirs::APP_ID).as_ptr())
    };
    if result < 0 {
        Err(std::io::Error::other(format!(
            "Could not set the Companion AppUserModelID ({result:#x})."
        )))
    } else {
        Ok(())
    }
}

pub(super) fn open(url: &str) -> Result<(), CommonError> {
    let url = wide(url);
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            url.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err(CommonError::Internal)
    } else {
        Ok(())
    }
}

pub(super) fn notify(message: &str) {
    if let Some(tx) = NOTICES.get() {
        let _ = tx.try_send(message.into());
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == STOP {
        // TrackPopupMenu runs a nested loop: close it too, so shutdown cannot
        // leave the tray thread waiting for the user to dismiss the menu.
        unsafe {
            EndMenu();
            PostQuitMessage(0);
        }
        return 0;
    }
    // Explorer may send messages directly to the window procedure, bypassing
    // GetMessage. Queue work so tray state stays thread-local and callbacks
    // cannot reenter it. A channel also preserves events during TrackPopupMenu's
    // nested message loop, which could consume private posted window messages.
    let forward = if msg == CALLBACK {
        Some(Event::Callback(w, l))
    } else if TASKBAR_CREATED.get().copied() == Some(msg) {
        Some(Event::Recreate)
    } else if matches!(msg, WM_DPICHANGED | WM_SETTINGCHANGE | WM_DISPLAYCHANGE) {
        Some(Event::ReloadIcon)
    } else {
        None
    };
    if let Some(event) = forward {
        if let Some(tx) = EVENTS.get() {
            let _ = tx.send(event);
            unsafe { PostMessageW(hwnd, WM_NULL, 0, 0) };
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn copy_wide<const N: usize>(out: &mut [u16; N], value: &str) {
    for (dest, src) in out.iter_mut().take(N - 1).zip(value.encode_utf16()) {
        *dest = src;
    }
}

unsafe fn load_icon() -> HICON {
    let mut icon = null_mut();
    let result =
        unsafe { LoadIconMetric(GetModuleHandleW(null()), APP_ICON, LIM_SMALL, &mut icon) };
    if result < 0 {
        eprintln!("Could not load the embedded tray icon ({result:#x}).");
    }
    icon
}

unsafe fn add_icon(icon: &mut NOTIFYICONDATAW) {
    icon.uFlags = NIF_GUID | NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    if unsafe { Shell_NotifyIconW(NIM_ADD, icon) } == 0 {
        eprintln!("Could not add the tray icon.");
        return;
    }
    icon.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    if unsafe { Shell_NotifyIconW(NIM_SETVERSION, icon) } == 0 {
        eprintln!("Could not set tray callback version 4.");
    }
}

unsafe fn menu(icon: &NOTIFYICONDATAW, mut x: i32, mut y: i32) -> i32 {
    unsafe {
        // Keyboard context activation can supply (-1, -1). Anchor to the icon,
        // including when it lives in the overflow area, rather than the mouse.
        if (x, y) == (-1, -1) {
            let id = NOTIFYICONIDENTIFIER {
                cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: icon.hWnd,
                uID: 0,
                guidItem: TRAY_GUID,
            };
            let mut rect = std::mem::zeroed();
            if Shell_NotifyIconGetRect(&id, &mut rect) >= 0 {
                x = rect.left;
                y = rect.bottom;
            } else {
                let mut pos: POINT = std::mem::zeroed();
                GetCursorPos(&mut pos);
                x = pos.x;
                y = pos.y;
            }
        }
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        for (id, label) in [
            (1, "Open"),
            (2, "Open in browser"),
            (3, "Reset pairing"),
            (4, "Quit"),
        ] {
            AppendMenuW(menu, MF_STRING, id, wide(label).as_ptr());
        }
        SetForegroundWindow(icon.hWnd);
        let action = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY,
            x,
            y,
            0,
            icon.hWnd,
            null(),
        );
        DestroyMenu(menu);
        // Complete the Shell menu protocol even when Escape/outside-click
        // cancels it, so keyboard focus returns to the notification area.
        PostMessageW(icon.hWnd, WM_NULL, 0, 0);
        Shell_NotifyIconW(NIM_SETFOCUS, icon);
        action
    }
}

pub(super) fn start(ctx: Ctx, packaged: bool) -> Tray {
    let handle = tokio::runtime::Handle::current();
    let (tx, rx) = mpsc::sync_channel(16);
    let _ = NOTICES.set(tx);
    let (tx, events) = mpsc::channel();
    let _ = EVENTS.set(tx);
    let state = Arc::new(Mutex::new(State::default()));
    let thread_state = state.clone();
    let thread = std::thread::spawn(move || unsafe {
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        if taskbar_created == 0 {
            eprintln!("Could not register the taskbar restart message.");
            return;
        }
        let _ = TASKBAR_CREATED.set(taskbar_created);
        let class = wide("OpenTouchDeckTray");
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        // A hidden top-level HWND, not HWND_MESSAGE: it must receive Explorer's
        // TaskbarCreated broadcast even while the app window is closed.
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        {
            let mut state = thread_state.lock();
            // Shutdown can arrive before the thread has created its window.
            if state.stopping {
                DestroyWindow(hwnd);
                return;
            }
            state.hwnd = hwnd as isize;
        }
        let mut icon: NOTIFYICONDATAW = std::mem::zeroed();
        icon.cbSize = std::mem::size_of_val(&icon) as u32;
        icon.hWnd = hwnd;
        icon.guidItem = TRAY_GUID;
        icon.uCallbackMessage = CALLBACK;
        icon.hIcon = load_icon();
        if icon.hIcon.is_null() {
            thread_state.lock().hwnd = 0;
            DestroyWindow(hwnd);
            return;
        }
        copy_wide(&mut icon.szTip, "Open TouchDeck");
        add_icon(&mut icon);
        SetTimer(hwnd, 1, 100, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if *ctx.shutdown.borrow() || thread_state.lock().stopping {
                break;
            }
            if let Ok(message) = rx.try_recv() {
                icon.uFlags = NIF_GUID | NIF_INFO;
                icon.szInfo.fill(0);
                copy_wide(&mut icon.szInfo, &message);
                copy_wide(&mut icon.szInfoTitle, "Open TouchDeck");
                icon.dwInfoFlags = NIIF_INFO;
                Shell_NotifyIconW(NIM_MODIFY, &icon);
                icon.szInfo.fill(0);
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
            while let Ok(event) = events.try_recv() {
                match event {
                    Event::Recreate | Event::ReloadIcon => {
                        let next = load_icon();
                        if !next.is_null() {
                            let old = std::mem::replace(&mut icon.hIcon, next);
                            DestroyIcon(old);
                        }
                        if matches!(event, Event::Recreate) {
                            add_icon(&mut icon);
                        } else {
                            icon.uFlags = NIF_GUID | NIF_ICON;
                            Shell_NotifyIconW(NIM_MODIFY, &icon);
                        }
                    }
                    Event::Callback(w, l) => {
                        let action = match routing::route(w, l) {
                            Some(routing::Action::Open) => 1,
                            Some(routing::Action::Menu { x, y }) => menu(&icon, x, y),
                            None => 0,
                        };
                        match action {
                            1 => {
                                if open_app(packaged).is_err() {
                                    ctx.desktop
                                        .notify("Open TouchDeck could not open its window.");
                                }
                            }
                            2 => {
                                let ctx = ctx.clone();
                                handle.spawn(async move {
                                    let _ = ctx.pair_browser().await;
                                });
                            }
                            3 => {
                                let ctx = ctx.clone();
                                handle.spawn(async move {
                                    let _ = ctx.reset_pairing().await;
                                });
                            }
                            4 => {
                                let _ = ctx.shutdown.send(true);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Shell_NotifyIconW(NIM_DELETE, &icon);
        DestroyIcon(icon.hIcon);
        KillTimer(hwnd, 1);
        // Serialize with the owner posting STOP; never post to a destroyed HWND.
        thread_state.lock().hwnd = 0;
        DestroyWindow(hwnd);
    });
    Tray {
        thread: Some(thread),
        state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_tray_wakes_and_joins_an_idle_window_thread() {
        let state = Arc::new(Mutex::new(State::default()));
        let thread_state = state.clone();
        let (ready, window) = mpsc::channel();
        let (finished, cleanup) = mpsc::channel();
        let thread = std::thread::spawn(move || unsafe {
            let class = wide("OpenTouchDeckTrayShutdownTest");
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!hwnd.is_null());
            thread_state.lock().hwnd = hwnd as isize;
            ready.send(()).unwrap();
            // No timer: only the owner's posted shutdown message can wake this loop.
            let mut msg = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                DispatchMessageW(&msg);
            }
            thread_state.lock().hwnd = 0;
            assert_ne!(DestroyWindow(hwnd), 0);
            UnregisterClassW(class.as_ptr(), instance);
            finished.send(()).unwrap();
        });
        let tray = Tray {
            thread: Some(thread),
            state,
        };
        window.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(tray);
        assert_eq!(cleanup.try_recv(), Ok(()), "cleanup finished before return");
    }

    #[test]
    fn a_stuck_tray_thread_cannot_block_exit() {
        let (release, stuck) = mpsc::channel();
        let (finished, cleanup) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            // Even a regression to an unbounded join must finish this test.
            let _ = stuck.recv_timeout(Duration::from_secs(5));
            finished.send(()).unwrap();
        });
        let state = Arc::new(Mutex::new(State::default()));
        let tray = Tray {
            thread: Some(thread),
            state: state.clone(),
        };
        drop(tray);
        assert!(
            state.lock().stopping,
            "shutdown also covers window creation"
        );
        assert_eq!(cleanup.try_recv(), Err(mpsc::TryRecvError::Empty));
        release.send(()).unwrap();
        cleanup.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
