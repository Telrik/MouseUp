use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use x11rb::{
    connection::Connection,
    protocol::{
        shape::{self, SK, SO},
        xfixes,
        xproto::{
            self, AtomEnum, ClipOrdering, ConfigureWindowAux, ConnectionExt, CreateGCAux,
            CreateWindowAux, EventMask, GrabMode, ImageFormat, ModMask, Rectangle, WindowClass,
        },
        Event,
    },
    rust_connection::RustConnection,
    COPY_FROM_PARENT,
};

use crate::{
    cursor,
    model::{self, Config},
};

pub struct Shared {
    pub config: Config,
    pub status: String,
    pub active: bool,
    pub hotkeys: bool,
    pub current_app: Option<String>,
}

struct Session {
    conn: RustConnection,
    root: u32,
    window: u32,
    gc: u32,
    hidden: bool,
    screen: usize,
    pid_atom: u32,
    image: Vec<u8>,
    size: u16,
    hotspot: (u16, u16),
    mapped: bool,
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.hidden {
            let _ = xfixes::show_cursor(&self.conn, self.root);
        }
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}

impl Session {
    fn new() -> Result<Self, String> {
        let (conn, screen) =
            x11rb::connect(None).map_err(|e| format!("Could not connect to X11: {e}"))?;
        xfixes::query_version(&conn, 5, 0)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| format!("XFixes is unavailable: {e}"))?;
        shape::query_version(&conn)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| format!("XShape is unavailable: {e}"))?;
        let root = conn.setup().roots[screen].root;
        let window = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            COPY_FROM_PARENT as u8,
            window,
            root,
            0,
            0,
            64,
            64,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(0)
                .event_mask(EventMask::EXPOSURE),
        )
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| e.to_string())?;
        shape::rectangles(
            &conn,
            SO::SET,
            SK::INPUT,
            ClipOrdering::UNSORTED,
            window,
            0,
            0,
            &[],
        )
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("Could not make overlay click-through: {e}"))?;
        let gc = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_gc(gc, window, &CreateGCAux::new())
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;
        let pid_atom = conn
            .intern_atom(false, b"_NET_WM_PID")
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .atom;
        Ok(Self {
            conn,
            root,
            window,
            gc,
            hidden: false,
            screen,
            pid_atom,
            image: Vec::new(),
            size: 64,
            hotspot: (0, 0),
            mapped: false,
        })
    }

    fn focused_app(&self) -> Option<String> {
        let mut window = self.conn.get_input_focus().ok()?.reply().ok()?.focus;
        for _ in 0..16 {
            if window <= 1 || window == self.root {
                return None;
            }
            if let Some(path) = self.window_app(window) {
                return Some(path);
            }
            window = self.conn.query_tree(window).ok()?.reply().ok()?.parent;
        }
        None
    }

    fn window_app(&self, window: u32) -> Option<String> {
        let reply = self
            .conn
            .get_property(false, window, self.pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let pid = reply.value32()?.next()?;
        std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    fn hovered_app(&self) -> Option<String> {
        let mut window = self.root;
        let mut app = None;
        for _ in 0..16 {
            let child = self.conn.query_pointer(window).ok()?.reply().ok()?.child;
            if child == 0 {
                break;
            }
            window = child;
            if let Some(path) = self.window_app(window) {
                app = Some(path);
            }
        }
        app
    }

    fn is_resize_cursor(&self) -> bool {
        let Some(image) = xfixes::get_cursor_image_and_name(&self.conn)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            return false;
        };
        matches!(
            image.name.as_slice(),
            b"left_side"
                | b"right_side"
                | b"top_side"
                | b"bottom_side"
                | b"top_left_corner"
                | b"top_right_corner"
                | b"bottom_left_corner"
                | b"bottom_right_corner"
                | b"sb_h_double_arrow"
                | b"sb_v_double_arrow"
                | b"ew-resize"
                | b"ns-resize"
                | b"nwse-resize"
                | b"nesw-resize"
                | b"col-resize"
                | b"row-resize"
                | b"fleur"
        )
    }

    fn update_image(&mut self, id: &str, size: u32) -> Result<(), String> {
        let (image, hotspot) = cursor::decode(id, size)?;
        let mut rectangles = Vec::new();
        for y in 0..size {
            let mut x = 0;
            while x < size {
                if image.get_pixel(x, y)[3] < 32 {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < size && image.get_pixel(x, y)[3] >= 32 {
                    x += 1;
                }
                rectangles.push(Rectangle {
                    x: start as i16,
                    y: y as i16,
                    width: (x - start) as u16,
                    height: 1,
                });
            }
        }
        let setup = self.conn.setup();
        let screen = &setup.roots[self.screen];
        let visual = screen
            .allowed_depths
            .iter()
            .flat_map(|d| &d.visuals)
            .find(|v| v.visual_id == screen.root_visual)
            .ok_or("X11 root visual unavailable")?;
        let format = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == screen.root_depth)
            .ok_or("X11 pixel format unavailable")?;
        if format.bits_per_pixel != 32 {
            return Err("X11 display must support 32-bit pixels".into());
        }
        let mut bytes = Vec::with_capacity((size * size * 4) as usize);
        for pixel in image.pixels() {
            let value = ((pixel[0] as u32) << visual.red_mask.trailing_zeros()) & visual.red_mask
                | ((pixel[1] as u32) << visual.green_mask.trailing_zeros()) & visual.green_mask
                | ((pixel[2] as u32) << visual.blue_mask.trailing_zeros()) & visual.blue_mask;
            if setup.image_byte_order == xproto::ImageOrder::LSB_FIRST {
                bytes.extend_from_slice(&value.to_le_bytes());
            } else {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
        }
        self.conn
            .configure_window(
                self.window,
                &ConfigureWindowAux::new().width(size).height(size),
            )
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;
        shape::rectangles(
            &self.conn,
            SO::SET,
            SK::BOUNDING,
            ClipOrdering::UNSORTED,
            self.window,
            0,
            0,
            &rectangles,
        )
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| e.to_string())?;
        self.image = bytes;
        self.size = size as u16;
        self.hotspot = hotspot;
        self.draw()?;
        Ok(())
    }

    fn draw(&self) -> Result<(), String> {
        if self.image.is_empty() {
            return Ok(());
        }
        let depth = self.conn.setup().roots[self.screen].root_depth;
        let stride = self.size as usize * 4;
        for (index, pixels) in self.image.chunks(stride * 32).enumerate() {
            self.conn
                .put_image(
                    ImageFormat::Z_PIXMAP,
                    self.window,
                    self.gc,
                    self.size,
                    (pixels.len() / stride) as u16,
                    0,
                    (index * 32) as i16,
                    0,
                    depth,
                    pixels,
                )
                .map_err(|e| e.to_string())?
                .check()
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn show(&mut self) -> Result<(), String> {
        let pointer = self
            .conn
            .query_pointer(self.root)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        if !pointer.same_screen {
            return self.hide();
        }
        self.conn
            .configure_window(
                self.window,
                &ConfigureWindowAux::new()
                    .x(pointer.root_x as i32 - self.hotspot.0 as i32)
                    .y(pointer.root_y as i32 - self.hotspot.1 as i32)
                    .stack_mode(xproto::StackMode::ABOVE),
            )
            .map_err(|e| e.to_string())?;
        if !self.mapped {
            self.conn
                .map_window(self.window)
                .map_err(|e| e.to_string())?
                .check()
                .map_err(|e| e.to_string())?;
            self.mapped = true;
            self.draw()?;
        }
        if !self.hidden {
            xfixes::hide_cursor(&self.conn, self.root)
                .map_err(|e| e.to_string())?
                .check()
                .map_err(|e| format!("Could not hide X11 cursor: {e}"))?;
            self.hidden = true;
        }
        self.conn.flush().map_err(|e| e.to_string())
    }

    fn hide(&mut self) -> Result<(), String> {
        if self.hidden {
            xfixes::show_cursor(&self.conn, self.root)
                .map_err(|e| e.to_string())?
                .check()
                .map_err(|e| e.to_string())?;
            self.hidden = false;
        }
        if self.mapped {
            self.conn
                .unmap_window(self.window)
                .map_err(|e| e.to_string())?;
            self.mapped = false;
        }
        self.conn.flush().map_err(|e| e.to_string())
    }
}

fn grab_hotkeys(session: &Session) -> Vec<(u8, u8)> {
    let setup = session.conn.setup();
    let count = setup.max_keycode - setup.min_keycode + 1;
    let Some(mapping) = session
        .conn
        .get_keyboard_mapping(setup.min_keycode, count)
        .ok()
        .and_then(|c| c.reply().ok())
    else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    for (action, sym) in [
        (1, b'1' as u32),
        (2, b'2' as u32),
        (3, b'd' as u32),
        (4, b'y' as u32),
    ] {
        let Some(index) = mapping
            .keysyms
            .chunks(mapping.keysyms_per_keycode as usize)
            .position(|chunk| chunk.contains(&sym))
        else {
            continue;
        };
        let code = setup.min_keycode + index as u8;
        let mut success = true;
        for extra in [
            ModMask::from(0u16),
            ModMask::LOCK,
            ModMask::M2,
            ModMask::LOCK | ModMask::M2,
        ] {
            let grab = session.conn.grab_key(
                false,
                session.root,
                ModMask::CONTROL | ModMask::M1 | extra,
                code,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            );
            if !grab.ok().is_some_and(|g| g.check().is_ok()) {
                success = false;
            }
        }
        if success {
            keys.push((code, action));
        } else {
            for extra in [
                ModMask::from(0u16),
                ModMask::LOCK,
                ModMask::M2,
                ModMask::LOCK | ModMask::M2,
            ] {
                let _ = session.conn.ungrab_key(
                    code,
                    session.root,
                    ModMask::CONTROL | ModMask::M1 | extra,
                );
            }
        }
    }
    keys
}

pub fn start(
    shared: Arc<Mutex<Shared>>,
    alive: Arc<AtomicBool>,
    config_path: PathBuf,
    ctx: eframe::egui::Context,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut session = match Session::new() {
            Ok(s) => s,
            Err(error) => {
                shared.lock().unwrap().status = error;
                ctx.request_repaint();
                return;
            }
        };
        let keys = grab_hotkeys(&session);
        shared.lock().unwrap().hotkeys = keys.len() == 4;
        let own = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let mut last_image = (String::new(), 0);
        let mut ticks = 0u32;
        while alive.load(Ordering::Relaxed) {
            while let Ok(Some(event)) = session.conn.poll_for_event() {
                match event {
                    Event::KeyPress(event) => {
                        if let Some((_, action)) = keys.iter().find(|(key, _)| *key == event.detail)
                        {
                            let target = session.focused_app().filter(|p| Some(p) != own.as_ref());
                            let mut state = shared.lock().unwrap();
                            match action {
                                1 | 2 => {
                                    if let Some(path) = &target {
                                        state.config.cycle_app_cursor(path, *action == 1);
                                    } else {
                                        state.config.cycle_cursor(*action == 1);
                                    }
                                }
                                3 => {
                                    if let Some(path) = &target {
                                        state.config.set_app_cursor(path, "@original");
                                    }
                                }
                                4 => state.config.enabled = !state.config.enabled,
                                _ => (),
                            }
                            if let Err(e) = model::save(&config_path, &state.config) {
                                state.status = format!("Could not save settings: {e}");
                            }
                            ctx.request_repaint();
                        }
                    }
                    Event::Expose(_) => {
                        let _ = session.draw();
                    }
                    _ => (),
                }
            }
            let focused = session.hovered_app().filter(|p| Some(p) != own.as_ref());
            let config = {
                let mut state = shared.lock().unwrap();
                state.current_app = focused.clone();
                state.config.clone()
            };
            let profile = focused
                .as_deref()
                .map(|p| config.app_profile(p))
                .unwrap_or_else(|| config.app_profile(""));
            let result = if session.is_resize_cursor() {
                session.hide()
            } else if let Some(profile) = profile {
                if last_image != (profile.cursor.clone(), profile.size) {
                    match session.update_image(&profile.cursor, profile.size) {
                        Ok(()) => last_image = (profile.cursor, profile.size),
                        Err(e) => {
                            let _ = session.hide();
                            shared.lock().unwrap().status = e;
                            thread::sleep(Duration::from_millis(250));
                            continue;
                        }
                    }
                }
                session.show()
            } else {
                session.hide()
            };
            {
                let mut state = shared.lock().unwrap();
                match result {
                    Ok(()) => {
                        state.active = session.hidden;
                        state.status = if session.hidden {
                            "Cursor active"
                        } else {
                            "Original cursor"
                        }
                        .into();
                    }
                    Err(e) => {
                        state.active = false;
                        state.status = e;
                        let _ = session.hide();
                    }
                }
            }
            ticks += 1;
            if ticks % 15 == 0 {
                ctx.request_repaint();
            }
            thread::sleep(Duration::from_millis(16));
        }
    })
}
