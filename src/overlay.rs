use crate::backend::{self, Shared};
use std::{
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

struct CursorImage {
    handle: HCURSOR,
    size: i32,
    hotspot: POINT,
}

fn needs_overlay(flags: u32, cursor_is_null: bool, cursor_is_empty: bool) -> bool {
    flags == 0 || (flags == CURSOR_SHOWING && (cursor_is_null || cursor_is_empty))
}
impl Drop for CursorImage {
    fn drop(&mut self) {
        unsafe {
            if !self.handle.is_null() {
                DestroyCursor(self.handle);
            }
        }
    }
}

impl CursorImage {
    unsafe fn load(path: &Path, size: i32) -> Option<Self> {
        let wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let handle = LoadImageW(
            null_mut(),
            wide.as_ptr(),
            IMAGE_CURSOR,
            size,
            size,
            LR_LOADFROMFILE,
        ) as HCURSOR;
        if handle.is_null() {
            return None;
        }
        let mut info: ICONINFO = zeroed();
        if GetIconInfo(handle, &mut info) == 0 {
            DestroyCursor(handle);
            return None;
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        Some(Self {
            handle,
            size,
            hotspot: POINT {
                x: info.xHotspot as i32,
                y: info.yHotspot as i32,
            },
        })
    }
}

// Render against black and white to recover alpha for both PNG and mask cursors.
pub(crate) unsafe fn render(handle: HCURSOR, size: i32) -> Option<Vec<u32>> {
    let screen = GetDC(null_mut());
    if screen.is_null() {
        return None;
    }
    let dc = CreateCompatibleDC(screen);
    ReleaseDC(null_mut(), screen);
    if dc.is_null() {
        return None;
    }
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = size;
    info.bmiHeader.biHeight = -size;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        DeleteDC(dc);
        return None;
    }
    let old = SelectObject(dc, bitmap);
    let count = (size * size) as usize;
    let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, count);
    pixels.fill(0);
    let a = DrawIconEx(dc, 0, 0, handle, size, size, 0, null_mut(), DI_NORMAL);
    GdiFlush();
    let black = pixels.to_vec();
    pixels.fill(0x00ffffff);
    let b = DrawIconEx(dc, 0, 0, handle, size, size, 0, null_mut(), DI_NORMAL);
    GdiFlush();
    let result = if a != 0 && b != 0 {
        Some(
            black
                .iter()
                .zip(pixels.iter())
                .map(|(&black, &white)| {
                    let delta = (0..3)
                        .map(|i| {
                            (((white >> (8 * i)) & 255) as i32 - ((black >> (8 * i)) & 255) as i32)
                                .clamp(0, 255)
                        })
                        .max()
                        .unwrap_or(255) as u32;
                    let alpha = 255 - delta;
                    let blue = (black & 255).min(alpha);
                    let green = ((black >> 8) & 255).min(alpha);
                    let red = ((black >> 16) & 255).min(alpha);
                    (alpha << 24) | (red << 16) | (green << 8) | blue
                })
                .collect(),
        )
    } else {
        None
    };
    SelectObject(dc, old);
    DeleteObject(bitmap);
    DeleteDC(dc);
    result
}

pub fn preview(path: &Path, size: i32) -> Option<eframe::egui::ColorImage> {
    unsafe {
        let cursor = CursorImage::load(path, size)?;
        icon_image(cursor.handle, size)
    }
}
unsafe fn icon_image(handle: HICON, size: i32) -> Option<eframe::egui::ColorImage> {
    let rgba: Vec<u8> = render(handle, size)?
        .into_iter()
        .flat_map(|p| {
            [
                ((p >> 16) & 255) as u8,
                ((p >> 8) & 255) as u8,
                (p & 255) as u8,
                (p >> 24) as u8,
            ]
        })
        .collect();
    Some(eframe::egui::ColorImage::from_rgba_premultiplied(
        [size as usize, size as usize],
        &rgba,
    ))
}
pub fn app_icon(path: &Path, size: i32) -> Option<eframe::egui::ColorImage> {
    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    unsafe {
        let wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut icon = null_mut();
        if ExtractIconExW(wide.as_ptr(), 0, &mut icon, null_mut(), 1) == 0 || icon.is_null() {
            return None;
        }
        let image = icon_image(icon, size);
        DestroyIcon(icon);
        image
    }
}
unsafe fn present(hwnd: HWND, cursor: &CursorImage, point: POINT) -> bool {
    let Some(data) = render(cursor.handle, cursor.size) else {
        return false;
    };
    present_pixels(hwnd, &data, cursor.size, cursor.hotspot, point)
}
unsafe fn present_pixels(
    hwnd: HWND,
    data: &[u32],
    dimension: i32,
    hotspot: POINT,
    point: POINT,
) -> bool {
    let screen = GetDC(null_mut());
    if screen.is_null() {
        return false;
    }
    let dc = CreateCompatibleDC(screen);
    if dc.is_null() {
        ReleaseDC(null_mut(), screen);
        return false;
    }
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = dimension;
    info.bmiHeader.biHeight = -dimension;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        DeleteDC(dc);
        ReleaseDC(null_mut(), screen);
        return false;
    }
    let old = SelectObject(dc, bitmap);
    std::ptr::copy_nonoverlapping(data.as_ptr(), bits as *mut u32, data.len());
    let dst = POINT {
        x: point.x - hotspot.x,
        y: point.y - hotspot.y,
    };
    let source = POINT { x: 0, y: 0 };
    let size = SIZE {
        cx: dimension,
        cy: dimension,
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let ok = UpdateLayeredWindow(hwnd, screen, &dst, &size, dc, &source, 0, &blend, ULW_ALPHA) != 0;
    SelectObject(dc, old);
    DeleteObject(bitmap);
    DeleteDC(dc);
    ReleaseDC(null_mut(), screen);
    ok
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCHITTEST {
        HTTRANSPARENT as LRESULT
    } else {
        DefWindowProcW(hwnd, msg, w, l)
    }
}

pub fn start(shared: Arc<Mutex<Shared>>, alive: Arc<AtomicBool>, bin: PathBuf) {
    thread::spawn(move || unsafe {
        let instance = GetModuleHandleW(null());
        let class: Vec<u16> = "MouseUpCursorOverlay\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            64,
            64,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        let mut cached: Option<(String, i32, CursorImage)> = None;
        let mut shown = false;
        let mut last_system_cursor: HCURSOR = null_mut();
        let mut system_cursor_empty = false;
        let mut hidden_since: Option<Instant> = None;
        while alive.load(Ordering::Relaxed) {
            let selected = {
                let s = shared.lock().unwrap();
                if s.config.enabled && s.config.software_overlay {
                    s.hovered_profile.clone()
                } else {
                    None
                }
            };
            let mut info: CURSORINFO = zeroed();
            info.cbSize = size_of::<CURSORINFO>() as u32;
            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point);
            let fg = GetAncestor(WindowFromPoint(point), GA_ROOT);
            let mut pid = 0;
            GetWindowThreadProcessId(fg, &mut pid);
            let mut bounds: RECT = zeroed();
            let mut top = POINT { x: 0, y: 0 };
            let mut clip: RECT = zeroed();
            let cursor_info_ok = GetCursorInfo(&mut info) != 0;
            if selected.as_ref().is_some_and(|(p, _)| *p == pid)
                && cursor_info_ok
                && info.flags == CURSOR_SHOWING
                && !info.hCursor.is_null()
                && info.hCursor != last_system_cursor
            {
                last_system_cursor = info.hCursor;
                system_cursor_empty = render(info.hCursor, 32)
                    .is_some_and(|pixels| pixels.iter().all(|p| p >> 24 == 0));
            }
            let hidden = cursor_info_ok
                && needs_overlay(
                    info.flags,
                    info.hCursor.is_null(),
                    system_cursor_empty && info.hCursor == last_system_cursor,
                );
            // A short hidden state is common during text input and window changes.
            // Wait for it to settle before creating an independent cursor window.
            if hidden && selected.as_ref().is_some_and(|(p, _)| *p == pid) {
                hidden_since.get_or_insert_with(Instant::now);
            } else {
                hidden_since = None;
            }
            let valid = selected.as_ref().is_some_and(|(p, _)| *p == pid)
                && !fg.is_null()
                && hidden_since.is_some_and(|since| since.elapsed() >= Duration::from_millis(250))
                && GetClientRect(fg, &mut bounds) != 0
                && ClientToScreen(fg, &mut top) != 0
                && info.ptScreenPos.x >= top.x
                && info.ptScreenPos.y >= top.y
                && info.ptScreenPos.x < top.x + bounds.right
                && info.ptScreenPos.y < top.y + bounds.bottom
                && (GetClipCursor(&mut clip) == 0
                    || clip.right - clip.left > 32
                    || clip.bottom - clip.top > 32);
            if valid {
                let profile = &selected.as_ref().unwrap().1;
                let path = backend::cursor_path(&profile.cursor, &bin);
                let key = path.to_string_lossy().into_owned();
                let size = profile.size as i32;
                if cached
                    .as_ref()
                    .is_none_or(|(p, s, _)| *p != key || *s != size)
                {
                    if shown {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                    cached = CursorImage::load(&path, size).map(|c| (key, size, c));
                    shown = false;
                }
                if let Some((_, _, cursor)) = &mut cached {
                    if !shown {
                        shown = present(hwnd, cursor, info.ptScreenPos);
                        if shown {
                            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        } else {
                            ShowWindow(hwnd, SW_HIDE);
                        }
                    }
                    if shown {
                        SetWindowPos(
                            hwnd,
                            HWND_TOPMOST,
                            info.ptScreenPos.x - cursor.hotspot.x,
                            info.ptScreenPos.y - cursor.hotspot.y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        );
                    }
                }
            } else {
                if shown {
                    ShowWindow(hwnd, SW_HIDE);
                    shown = false;
                }
            }
            let mut msg: MSG = zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            thread::sleep(Duration::from_millis(16));
        }
        DestroyWindow(hwnd);
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_cursor_renders_with_transparency() {
        unsafe {
            let cursor = LoadCursorW(null_mut(), IDC_ARROW);
            assert!(!cursor.is_null());
            let pixels = render(cursor, 64).unwrap();
            assert_eq!(pixels.len(), 64 * 64);
            assert!(pixels.iter().any(|p| p >> 24 == 0));
            assert!(pixels.iter().any(|p| p >> 24 > 200));
        }
    }
    #[test]
    fn overlay_detects_hidden_and_invisible_cursors_but_not_suppressed_input() {
        assert!(needs_overlay(0, false, false));
        assert!(needs_overlay(CURSOR_SHOWING, true, false));
        assert!(needs_overlay(CURSOR_SHOWING, false, true));
        assert!(!needs_overlay(CURSOR_SHOWING, false, false));
        assert!(!needs_overlay(CURSOR_SUPPRESSED, true, true));
    }
}
