use eframe::egui;
use std::{
    fs,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Ready,
    Unavailable,
    Hidden,
    Open,
    Quit,
}
static SIGNAL: OnceLock<(Sender<Event>, egui::Context)> = OnceLock::new();
static ICON: AtomicIsize = AtomicIsize::new(0);
static MAIN: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static AVAILABLE: AtomicBool = AtomicBool::new(false);
static HIDDEN: AtomicBool = AtomicBool::new(false);
static RESTORING: AtomicBool = AtomicBool::new(false);
const TRAY_MESSAGE: u32 = WM_APP + 42;
pub const RESTORE_MESSAGE: u32 = WM_APP + 43;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn emit(event: Event) {
    if let Some((sender, ctx)) = SIGNAL.get() {
        let _ = sender.send(event);
        ctx.request_repaint();
    }
}
unsafe fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = ICON.load(Ordering::Relaxed) as HICON;
    let tip = wide("MouseUp — click to open");
    data.szTip[..tip.len()].copy_from_slice(&tip);
    data
}
unsafe fn restore() {
    RESTORING.store(true, Ordering::Relaxed);
    HIDDEN.store(false, Ordering::Relaxed);
    let hwnd = MAIN.load(Ordering::Relaxed) as HWND;
    if !hwnd.is_null() {
        ShowWindowAsync(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
    emit(Event::Open);
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == RESTORE_MESSAGE {
        restore();
        return 0;
    }
    if msg == TASKBAR_CREATED.load(Ordering::Relaxed) && msg != 0 {
        if Shell_NotifyIconW(NIM_ADD, &notify_data(hwnd)) != 0 {
            AVAILABLE.store(true, Ordering::Relaxed);
            emit(Event::Ready);
        } else {
            AVAILABLE.store(false, Ordering::Relaxed);
            restore();
            emit(Event::Unavailable);
        }
        return 0;
    }
    if msg == TRAY_MESSAGE {
        match l as u32 {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => restore(),
            WM_RBUTTONUP => {
                let menu = CreatePopupMenu();
                if !menu.is_null() {
                    AppendMenuW(menu, MF_STRING, 1, wide("Open MouseUp").as_ptr());
                    AppendMenuW(menu, MF_SEPARATOR, 0, null());
                    AppendMenuW(menu, MF_STRING, 2, wide("Exit MouseUp").as_ptr());
                    let mut point: POINT = zeroed();
                    GetCursorPos(&mut point);
                    SetForegroundWindow(hwnd);
                    let choice = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        point.x,
                        point.y,
                        0,
                        hwnd,
                        null(),
                    );
                    DestroyMenu(menu);
                    PostMessageW(hwnd, WM_NULL, 0, 0);
                    match choice {
                        1 => restore(),
                        2 => {
                            restore();
                            emit(Event::Quit)
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        };
        return 0;
    }
    DefWindowProcW(hwnd, msg, w, l)
}
pub fn start(
    ctx: egui::Context,
    alive: Arc<AtomicBool>,
    dir: PathBuf,
    main: isize,
) -> (Receiver<Event>, JoinHandle<()>) {
    let (sender, rx) = mpsc::channel();
    let _ = SIGNAL.set((sender, ctx));
    MAIN.store(main, Ordering::Relaxed);
    let join = thread::spawn(move || unsafe {
        let _ = fs::create_dir_all(&dir);
        let icon_path = dir.join("mouseup-icon.ico");
        if fs::read(&icon_path).ok().as_deref() != Some(include_bytes!("../assets/brand.ico")) {
            let _ = fs::write(&icon_path, include_bytes!("../assets/brand.ico"));
        }
        let icon = LoadImageW(
            null_mut(),
            wide(&icon_path.to_string_lossy()).as_ptr(),
            IMAGE_ICON,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        ) as HICON;
        let own_icon = !icon.is_null();
        let icon = if own_icon {
            icon
        } else {
            LoadIconW(null_mut(), IDI_APPLICATION)
        };
        ICON.store(icon as isize, Ordering::Relaxed);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            Ordering::Relaxed,
        );
        let instance = GetModuleHandleW(null());
        let class = wide("MouseUpTrayWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            emit(Event::Unavailable);
            if own_icon {
                DestroyIcon(icon);
            }
            return;
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
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
            emit(Event::Unavailable);
            if own_icon {
                DestroyIcon(icon);
            }
            return;
        }
        if Shell_NotifyIconW(NIM_ADD, &notify_data(hwnd)) != 0 {
            AVAILABLE.store(true, Ordering::Relaxed);
            emit(Event::Ready);
        } else {
            emit(Event::Unavailable);
        }
        let mut msg: MSG = zeroed();
        while alive.load(Ordering::Relaxed) {
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let main = MAIN.load(Ordering::Relaxed) as HWND;
            if !main.is_null() {
                if RESTORING.load(Ordering::Relaxed) {
                    if IsIconic(main) == 0 {
                        RESTORING.store(false, Ordering::Relaxed);
                    }
                } else if AVAILABLE.load(Ordering::Relaxed)
                    && !HIDDEN.load(Ordering::Relaxed)
                    && IsIconic(main) != 0
                {
                    HIDDEN.store(true, Ordering::Relaxed);
                    ShowWindowAsync(main, SW_HIDE);
                    emit(Event::Hidden);
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        Shell_NotifyIconW(NIM_DELETE, &notify_data(hwnd));
        DestroyWindow(hwnd);
        if own_icon {
            DestroyIcon(icon);
        }
    });
    (rx, join)
}
