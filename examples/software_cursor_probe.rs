#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() {
    unsafe {
        use std::{mem::zeroed, ptr::null_mut};
        use windows_sys::Win32::{
            Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::*,
        };
        static mut X: i32 = 100;
        static mut Y: i32 = 100;
        unsafe extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            match m {
                WM_MOUSEMOVE => {
                    X = (l as u32 as u16 as i16) as i32;
                    Y = ((l as u32 >> 16) as u16 as i16) as i32;
                    InvalidateRect(h, null_mut(), 1);
                    0
                }
                WM_SETCURSOR => {
                    SetCursor(null_mut());
                    1
                }
                WM_PAINT => {
                    let mut ps: PAINTSTRUCT = zeroed();
                    let dc = BeginPaint(h, &mut ps);
                    let mut r: RECT = zeroed();
                    GetClientRect(h, &mut r);
                    let bg = CreateSolidBrush(0x00332920);
                    FillRect(dc, &r, bg);
                    DeleteObject(bg);
                    let text: Vec<u16> = "Software cursor test: move the mouse inside this window"
                        .encode_utf16()
                        .collect();
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetTextColor(dc, 0x00ffffff);
                    TextOutW(dc, 20, 20, text.as_ptr(), text.len() as i32);
                    let brush = CreateSolidBrush(0x00ffffff);
                    let old = SelectObject(dc, brush);
                    let pen = CreatePen(PS_SOLID as i32, 2, 0);
                    let op = SelectObject(dc, pen);
                    let points = [
                        POINT { x: X, y: Y },
                        POINT { x: X, y: Y + 28 },
                        POINT {
                            x: X + 8,
                            y: Y + 21,
                        },
                        POINT {
                            x: X + 13,
                            y: Y + 33,
                        },
                        POINT {
                            x: X + 18,
                            y: Y + 31,
                        },
                        POINT {
                            x: X + 12,
                            y: Y + 18,
                        },
                        POINT {
                            x: X + 25,
                            y: Y + 18,
                        },
                    ];
                    Polygon(dc, points.as_ptr(), points.len() as i32);
                    SelectObject(dc, op);
                    SelectObject(dc, old);
                    DeleteObject(pen);
                    DeleteObject(brush);
                    EndPaint(h, &ps);
                    0
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    0
                }
                _ => DefWindowProcW(h, m, w, l),
            }
        }
        let instance = GetModuleHandleW(null_mut());
        let class: Vec<u16> = "MouseUpSoftwareProbe\0".encode_utf16().collect();
        let title: Vec<u16> = "MouseUp software cursor test\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            150,
            150,
            640,
            420,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        ShowCursor(0);
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        ShowCursor(1);
    }
}
