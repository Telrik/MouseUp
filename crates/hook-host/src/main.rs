#![cfg_attr(windows, windows_subsystem = "console")]
#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() {
    use std::{
        io::{self, Read, Write},
        ptr::null_mut,
        thread,
    };
    use windows_sys::Win32::{
        Foundation::{FreeLibrary, HWND},
        System::{LibraryLoader::*, Threading::GetCurrentThreadId},
        UI::WindowsAndMessaging::*,
    };
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 4 && args[1] == "--system-cursor" {
        unsafe {
            let old = CopyIcon(LoadCursorW(null_mut(), IDC_ARROW)) as HCURSOR;
            let path: Vec<u16> = args[2]
                .to_string_lossy()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let size = args[3]
                .to_string_lossy()
                .parse::<i32>()
                .unwrap_or(0)
                .clamp(16, 256);
            let replacement = LoadImageW(
                null_mut(),
                path.as_ptr(),
                IMAGE_CURSOR,
                size,
                size,
                LR_LOADFROMFILE,
            ) as HCURSOR;
            if old.is_null() || replacement.is_null() {
                if !old.is_null() {
                    DestroyCursor(old);
                }
                if !replacement.is_null() {
                    DestroyCursor(replacement);
                }
                eprintln!("Could not load system cursor");
                std::process::exit(6);
            }
            if SetSystemCursor(replacement, 32512) == 0 {
                DestroyCursor(old);
                eprintln!(
                    "Could not set system cursor: {}",
                    std::io::Error::last_os_error()
                );
                std::process::exit(7);
            }
            println!("READY");
            let _ = io::stdout().flush();
            let _ = io::stdin().read(&mut [0]);
            if SetSystemCursor(old, 32512) == 0 {
                eprintln!(
                    "Could not restore system cursor: {}",
                    std::io::Error::last_os_error()
                );
                std::process::exit(8);
            }
        }
        return;
    }
    if args.len() != 4 {
        eprintln!("expected target thread id, hook DLL path and window handle");
        std::process::exit(2);
    }
    let tid: u32 = args[1].to_string_lossy().parse().unwrap_or(0);
    let target_window: isize = args[3].to_string_lossy().parse().unwrap_or(0);
    let dll: Vec<u16> = args[2]
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let module = LoadLibraryW(dll.as_ptr());
        if module.is_null() {
            eprintln!("failed to load hook DLL");
            std::process::exit(3);
        }
        let proc = GetProcAddress(module, b"mouseup_hook\0".as_ptr());
        if proc.is_none() {
            eprintln!("hook export missing");
            std::process::exit(4);
        }
        let hook = SetWindowsHookExW(
            WH_CALLWNDPROCRET,
            Some(std::mem::transmute(proc.unwrap())),
            module,
            tid,
        );
        if hook.is_null() {
            eprintln!(
                "failed to install hook for thread {tid}: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(5);
        }
        // Ensure a message queue exists before the stdin watcher can post WM_QUIT.
        let mut msg = std::mem::zeroed();
        PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
        let thread_id = GetCurrentThreadId();
        thread::spawn(move || {
            let _ = io::stdin().read(&mut [0]);
            PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
        });
        println!("READY");
        let _ = io::stdout().flush();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let detach: Vec<u16> = "MouseUp.DetachCursorHook.v1\0".encode_utf16().collect();
        let mut result = 0usize;
        if target_window != 0 {
            SendMessageTimeoutW(
                target_window as HWND,
                RegisterWindowMessageW(detach.as_ptr()),
                0x4d555031,
                0,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                500,
                &mut result,
            );
        }
        UnhookWindowsHookEx(hook);
        FreeLibrary(module);
    }
}
