#[cfg(windows)]
mod hook {
    use std::{
        cell::RefCell,
        ptr::null_mut,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        sync::{Mutex, Once, OnceLock},
    };
    use windows_sys::Win32::{
        Foundation::*,
        System::{
            LibraryLoader::*,
            Memory::*,
            Threading::{GetCurrentProcessId, GetCurrentThreadId},
        },
        UI::WindowsAndMessaging::*,
    };

    struct Cursor {
        spec: String,
        handle: HCURSOR,
        requested: Option<HCURSOR>,
        show_hidden: bool,
    }
    impl Drop for Cursor {
        fn drop(&mut self) {
            unsafe {
                if !self.handle.is_null() {
                    DestroyCursor(self.handle);
                }
            }
        }
    }
    thread_local! { static CURSOR: RefCell<Option<Cursor>> = const { RefCell::new(None) }; }
    static INSTALL: Once = Once::new();
    static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static TARGET: AtomicUsize = AtomicUsize::new(0);
    static ENABLED: AtomicBool = AtomicBool::new(false);
    static ORIGINAL_GET: AtomicUsize = AtomicUsize::new(0);
    static TARGET_GET: AtomicUsize = AtomicUsize::new(0);
    static ENABLED_GET: AtomicBool = AtomicBool::new(false);
    static DETOUR_LOCK: Mutex<()> = Mutex::new(());
    static DETACH_MESSAGE: OnceLock<u32> = OnceLock::new();

    unsafe fn session_exists() -> bool {
        let name: Vec<u16> = format!(
            "Local\\MouseUp-cursor-{}-{}\0",
            GetCurrentProcessId(),
            GetCurrentThreadId()
        )
        .encode_utf16()
        .collect();
        let mapping = OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr());
        if mapping.is_null() {
            return false;
        }
        CloseHandle(mapping);
        true
    }

    #[no_mangle]
    pub unsafe extern "system" fn mouseup_actual_cursor() -> HCURSOR {
        let original = ORIGINAL_GET.load(Ordering::Acquire);
        if original == 0 {
            return GetCursor();
        }
        let real: unsafe extern "system" fn() -> HCURSOR = std::mem::transmute(original);
        real()
    }

    unsafe extern "system" fn get_cursor() -> HCURSOR {
        let logical = CURSOR
            .try_with(|slot| {
                let cursor = slot.try_borrow().ok()?;
                let requested = cursor.as_ref()?.requested?;
                session_exists().then_some(requested)
            })
            .ok()
            .flatten();
        logical.unwrap_or_else(|| mouseup_actual_cursor())
    }

    unsafe extern "system" fn set_cursor(requested: HCURSOR) -> HCURSOR {
        let original = ORIGINAL.load(Ordering::Acquire);
        if original == 0 {
            return null_mut();
        }
        let real: unsafe extern "system" fn(HCURSOR) -> HCURSOR = std::mem::transmute(original);
        let result = CURSOR
            .try_with(|slot| {
                let Ok(mut cursor) = slot.try_borrow_mut() else {
                    return None;
                };
                let cursor = cursor.as_mut()?;
                if !session_exists() {
                    return None;
                }
                let previous = cursor.requested.unwrap_or_else(|| mouseup_actual_cursor());
                cursor.requested = Some(requested);
                real(if requested.is_null() && !cursor.show_hidden {
                    requested
                } else {
                    cursor.handle
                });
                Some(previous)
            })
            .ok()
            .flatten();
        result.unwrap_or_else(|| real(requested))
    }

    unsafe fn install_detour() {
        INSTALL.call_once(|| {
            let _ = std::panic::catch_unwind(|| unsafe {
                let user32: Vec<u16> = "user32.dll\0".encode_utf16().collect();
                let module = GetModuleHandleW(user32.as_ptr());
                let Some(target) = GetProcAddress(module, b"SetCursor\0".as_ptr()) else {
                    return;
                };
                let Some(target_get) = GetProcAddress(module, b"GetCursor\0".as_ptr()) else {
                    return;
                };
                // A target can outlive MouseUp or be interrupted during teardown. Pin this
                // module so the detour never points at unloaded code; without the shared
                // session mapping it immediately delegates to Windows unchanged.
                let mut own = null_mut();
                if GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                    mouseup_hook as *const () as *const u16,
                    &mut own,
                ) == 0
                {
                    return;
                }
                let target = target as *mut std::ffi::c_void;
                if let Ok(original) =
                    minhook::MinHook::create_hook(target, set_cursor as *mut std::ffi::c_void)
                {
                    let target_get = target_get as *mut std::ffi::c_void;
                    if let Ok(original_get) = minhook::MinHook::create_hook(
                        target_get,
                        get_cursor as *mut std::ffi::c_void,
                    ) {
                        ORIGINAL.store(original as usize, Ordering::Release);
                        TARGET.store(target as usize, Ordering::Release);
                        ORIGINAL_GET.store(original_get as usize, Ordering::Release);
                        TARGET_GET.store(target_get as usize, Ordering::Release);
                    } else {
                        let _ = minhook::MinHook::remove_hook(target);
                    }
                }
            });
        });
        if ENABLED.load(Ordering::Acquire) && ENABLED_GET.load(Ordering::Acquire) {
            return;
        }
        let Ok(_guard) = DETOUR_LOCK.lock() else {
            return;
        };
        let target = TARGET.load(Ordering::Acquire);
        let target_get = TARGET_GET.load(Ordering::Acquire);
        if target_get != 0
            && !ENABLED_GET.load(Ordering::Acquire)
            && minhook::MinHook::enable_hook(target_get as *mut _).is_ok()
        {
            ENABLED_GET.store(true, Ordering::Release);
        }
        if target != 0
            && ENABLED_GET.load(Ordering::Acquire)
            && !ENABLED.load(Ordering::Acquire)
            && minhook::MinHook::enable_hook(target as *mut _).is_ok()
        {
            ENABLED.store(true, Ordering::Release);
        }
    }

    unsafe fn disable_detour() {
        let Ok(_guard) = DETOUR_LOCK.lock() else {
            return;
        };
        let target = TARGET.load(Ordering::Acquire);
        if target != 0
            && ENABLED.load(Ordering::Acquire)
            && minhook::MinHook::disable_hook(target as *mut _).is_ok()
        {
            ENABLED.store(false, Ordering::Release);
        }
        let target_get = TARGET_GET.load(Ordering::Acquire);
        if target_get != 0
            && ENABLED_GET.load(Ordering::Acquire)
            && minhook::MinHook::disable_hook(target_get as *mut _).is_ok()
        {
            ENABLED_GET.store(false, Ordering::Release);
        }
        let _ = CURSOR.try_with(|slot| {
            if let Ok(mut cursor) = slot.try_borrow_mut() {
                if let Some(cursor) = cursor.as_mut() {
                    cursor.requested = None;
                }
            }
        });
    }

    #[no_mangle]
    pub unsafe extern "system" fn mouseup_hook(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 && lparam != 0 {
            let msg = &*(lparam as *const CWPRETSTRUCT);
            let detach_message = *DETACH_MESSAGE.get_or_init(|| {
                let name: Vec<u16> = "MouseUp.DetachCursorHook.v1\0".encode_utf16().collect();
                RegisterWindowMessageW(name.as_ptr())
            });
            if detach_message != 0 && msg.message == detach_message && msg.wParam == 0x4d555031 {
                disable_detour();
                return CallNextHookEx(null_mut(), code, wparam, lparam);
            }
            if matches!(
                msg.message,
                WM_SETCURSOR
                    | WM_MOUSEMOVE
                    | WM_KEYDOWN
                    | WM_CHAR
                    | WM_PAINT
                    | WM_TIMER
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONUP
                    | WM_RBUTTONDOWN
                    | WM_RBUTTONUP
            ) || msg.message >= WM_USER
            {
                let name: Vec<u16> = format!(
                    "Local\\MouseUp-cursor-{}-{}\0",
                    GetCurrentProcessId(),
                    GetCurrentThreadId()
                )
                .encode_utf16()
                .collect();
                let mapping = OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr());
                let spec = if mapping.is_null() {
                    None
                } else {
                    let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 65536);
                    let spec = if view.Value.is_null() {
                        None
                    } else {
                        let bytes = std::slice::from_raw_parts(view.Value as *const u8, 65536);
                        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
                        let result = std::str::from_utf8(&bytes[..end]).ok().map(str::to_owned);
                        UnmapViewOfFile(view);
                        result
                    };
                    CloseHandle(mapping);
                    spec
                };
                if let Some(spec) = spec {
                    CURSOR.with(|slot| {
                        let Ok(mut cursor) = slot.try_borrow_mut() else {
                            return;
                        };
                        if cursor.as_ref().map(|c| c.spec.as_str()) != Some(spec.as_str()) {
                            let mut lines = spec.lines();
                            let size = lines
                                .next()
                                .and_then(|s| s.parse::<i32>().ok())
                                .unwrap_or(0)
                                .clamp(0, 256);
                            if let Some(path) = lines.next() {
                                let show_hidden = lines.next() == Some("1");
                                let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
                                let handle = LoadImageW(
                                    null_mut(),
                                    wide.as_ptr(),
                                    IMAGE_CURSOR,
                                    size,
                                    size,
                                    LR_LOADFROMFILE,
                                ) as HCURSOR;
                                if !handle.is_null() {
                                    let requested = cursor.as_ref().and_then(|c| c.requested);
                                    *cursor = Some(Cursor {
                                        spec,
                                        handle,
                                        requested,
                                        show_hidden,
                                    });
                                }
                            }
                        }
                        if let Some(cursor) = cursor.as_ref() {
                            install_detour();
                            if !GetCursor().is_null() {
                                SetCursor(cursor.handle);
                            }
                        }
                    });
                }
            }
        }
        CallNextHookEx(null_mut(), code, wparam, lparam)
    }
}
