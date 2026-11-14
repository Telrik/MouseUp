#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() {
    unsafe {
        use std::{fs, mem::zeroed, ptr::null_mut, time::Duration};
        use windows_sys::Win32::{
            System::{
                LibraryLoader::{GetModuleHandleW, GetProcAddress},
                Threading::{GetCurrentProcessId, GetCurrentThreadId},
            },
            UI::WindowsAndMessaging::*,
        };
        unsafe extern "system" fn proc(
            h: *mut core::ffi::c_void,
            m: u32,
            w: usize,
            l: isize,
        ) -> isize {
            if m == WM_MOUSEMOVE || m == WM_KEYDOWN || m == WM_APP + 7 {
                SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
                return 0;
            }
            DefWindowProcW(h, m, w, l)
        }
        unsafe fn actual_cursor() -> HCURSOR {
            let name: Vec<u16> = if cfg!(target_pointer_width = "64") {
                "mouseup-hook64.dll\0"
            } else {
                "mouseup-hook32.dll\0"
            }
            .encode_utf16()
            .collect();
            let module = GetModuleHandleW(name.as_ptr());
            if !module.is_null() {
                if let Some(address) = GetProcAddress(module, b"mouseup_actual_cursor\0".as_ptr()) {
                    let get: unsafe extern "system" fn() -> HCURSOR = std::mem::transmute(address);
                    return get();
                }
            }
            GetCursor()
        }
        let result = std::env::args_os().nth(1).unwrap();
        let instance = GetModuleHandleW(null_mut());
        let name: Vec<u16> = "MouseUpHookProbe\0".encode_utf16().collect();
        let arrow = LoadCursorW(null_mut(), IDC_ARROW);
        let ibeam = LoadCursorW(null_mut(), IDC_IBEAM);
        let user32: Vec<u16> = "user32.dll\0".encode_utf16().collect();
        let set_cursor = GetProcAddress(GetModuleHandleW(user32.as_ptr()), b"SetCursor\0".as_ptr())
            .unwrap() as *const u8;
        let original_bytes = std::slice::from_raw_parts(set_cursor, 16).to_vec();
        let get_cursor = GetProcAddress(GetModuleHandleW(user32.as_ptr()), b"GetCursor\0".as_ptr())
            .unwrap() as *const u8;
        let original_get = std::slice::from_raw_parts(get_cursor, 16).to_vec();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            hCursor: arrow,
            ..zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            name.as_ptr(),
            name.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            100,
            100,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        fs::write(
            &result,
            format!(
                "{} {} {} waiting",
                GetCurrentProcessId(),
                GetCurrentThreadId(),
                hwnd as isize
            ),
        )
        .unwrap();
        for _ in 0..250 {
            SetCursor(arrow);
            SendMessageW(hwnd, WM_SETCURSOR, hwnd as usize, 1);
            let setcursor = actual_cursor() != arrow;
            SendMessageW(hwnd, WM_MOUSEMOVE, 0, 0);
            let mousemove = actual_cursor() != arrow;
            SendMessageW(hwnd, WM_KEYDOWN, b'A' as usize, 0);
            let keydown = actual_cursor() != arrow;
            SendMessageW(hwnd, WM_APP + 7, 0, 0);
            let asynchronous_update = actual_cursor() != arrow;
            SetCursor(ibeam);
            let logical_ibeam = GetCursor() == ibeam;
            let previous = SetCursor(arrow);
            let logical_arrow = GetCursor() == arrow;
            let direct_api = actual_cursor() != arrow;
            SetCursor(null_mut());
            let show_hidden =
                fs::read_to_string(std::path::PathBuf::from(&result).with_extension("show-hidden"))
                    .is_ok_and(|value| value == "1");
            let hidden = GetCursor().is_null() && (actual_cursor().is_null() != show_hidden);
            let restored_from_hidden = SetCursor(arrow).is_null();
            let changed = setcursor
                && mousemove
                && keydown
                && asynchronous_update
                && direct_api
                && previous == ibeam
                && logical_ibeam
                && logical_arrow
                && hidden
                && restored_from_hidden;
            let set_patched =
                std::slice::from_raw_parts(set_cursor, 16) != original_bytes.as_slice();
            let get_patched = std::slice::from_raw_parts(get_cursor, 16) != original_get.as_slice();
            let patched = set_patched && get_patched;
            let restored = !set_patched && !get_patched;
            fs::write(
                &result,
                format!(
                    "{} {} {} {}",
                    GetCurrentProcessId(),
                    GetCurrentThreadId(),
                    hwnd as isize,
                    if changed && patched {
                        "replaced"
                    } else if !changed && restored {
                        "original"
                    } else if !changed && patched {
                        "bypassed"
                    } else {
                        "not_restored"
                    }
                ),
            )
            .unwrap();
            let mut msg = zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        DestroyWindow(hwnd);
    }
}
