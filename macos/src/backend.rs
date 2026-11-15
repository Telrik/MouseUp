use std::{ffi::{c_char, c_void, CStr}, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::Duration};
use objc::{class, msg_send, sel, sel_impl};

use crate::model::Config;

pub struct Shared {
    pub config: Config,
    pub status: String,
    pub active: bool,
    pub hotkeys: bool,
    pub current_app: Option<String>,
    pub pointer: Option<[f32; 2]>,
}

#[repr(C)]
struct Point { x: f64, y: f64 }

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *const c_void) -> Point;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" { fn CFRelease(value: *const c_void); }
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

pub fn frontmost_app() -> Option<String> {
    unsafe {
        let workspace: *mut objc::runtime::Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() { return None; }
        let app: *mut objc::runtime::Object = msg_send![workspace, frontmostApplication];
        if app.is_null() { return None; }
        let pid: i32 = msg_send![app, processIdentifier];
        if pid <= 0 || pid as u32 == std::process::id() { return None; }
        let url: *mut objc::runtime::Object = msg_send![app, executableURL];
        if url.is_null() { return None; }
        let path: *mut objc::runtime::Object = msg_send![url, path];
        if path.is_null() { return None; }
        let bytes: *const c_char = msg_send![path, UTF8String];
        if bytes.is_null() { return None; }
        Some(CStr::from_ptr(bytes).to_string_lossy().into_owned())
    }
}

pub fn start(shared: Arc<Mutex<Shared>>, alive: Arc<AtomicBool>, _: PathBuf, ctx: eframe::egui::Context) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last = None;
        while alive.load(Ordering::Relaxed) {
            let position = unsafe {
                let event = CGEventCreate(std::ptr::null());
                if event.is_null() { None } else {
                    let point = CGEventGetLocation(event);
                    CFRelease(event);
                    Some([point.x as f32, point.y as f32])
                }
            };
            let mut state = shared.lock().unwrap();
            state.pointer = position;
            state.active = state.config.enabled && position.is_some();
            state.status = if position.is_none() { "Unable to read the mouse position" } else if state.active { "Overlay active (system cursor may remain visible)" } else { "Paused" }.into();
            drop(state);
            if position != last { last = position; ctx.request_repaint(); }
            thread::sleep(Duration::from_millis(20));
        }
    })
}
