#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(not(windows))]
fn main() {
    eprintln!("MouseUp requires Windows.");
}
#[cfg(windows)]
mod backend;
#[cfg(windows)]
mod bundle;
#[cfg(windows)]
mod catalog;
#[cfg(windows)]
mod model;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod tray;

#[cfg(windows)]
mod app {
    use crate::{
        backend::{self, Shared},
        bundle,
        catalog::SHAPES,
        model::{self, Config},
        overlay, tray,
    };
    use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        ptr::null_mut,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::Receiver,
            Arc, Mutex,
        },
        thread,
        time::Duration,
    };
    use windows_sys::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};

    const COLORS: [(&str, Color32); 12] = [
        ("Blue", Color32::from_rgb(37, 166, 248)),
        ("DarkBlue", Color32::from_rgb(33, 73, 153)),
        ("Green", Color32::from_rgb(65, 203, 109)),
        ("Yellow", Color32::from_rgb(242, 213, 62)),
        ("Orange", Color32::from_rgb(242, 143, 46)),
        ("Red", Color32::from_rgb(224, 68, 75)),
        ("Pink", Color32::from_rgb(241, 116, 178)),
        ("Purple", Color32::from_rgb(161, 99, 234)),
        ("Light", Color32::from_rgb(245, 245, 248)),
        ("Silver", Color32::from_rgb(164, 172, 185)),
        ("Dark", Color32::from_rgb(45, 47, 57)),
        ("Inverted", Color32::from_rgb(120, 120, 130)),
    ];
    const BG: Color32 = Color32::from_rgb(13, 9, 27);
    const CARD: Color32 = Color32::from_rgb(27, 21, 43);
    const PURPLE: Color32 = Color32::from_rgb(141, 99, 244);
    const MUTED: Color32 = Color32::from_rgb(157, 151, 177);

    fn background_gradient(ui: &egui::Ui, margin: f32, top: Color32, bottom: Color32) {
        let rect = ui.max_rect().expand(margin);
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(rect.left_top(), top);
        mesh.colored_vertex(rect.right_top(), top);
        mesh.colored_vertex(rect.right_bottom(), bottom);
        mesh.colored_vertex(rect.left_bottom(), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        ui.painter().add(egui::Shape::mesh(mesh));
    }

    fn cursor_id(shape: &str, color: &str) -> String {
        format!("@yolo\\{shape}\\{color}.cur")
    }
    fn cursor_parts(path: &str) -> Option<(&str, &str)> {
        let (shape, file) = path.strip_prefix("@yolo\\")?.split_once('\\')?;
        let color = file.strip_suffix(".cur")?;
        if SHAPES.contains(&shape) && COLORS.iter().any(|(name, _)| *name == color) {
            Some((shape, color))
        } else {
            None
        }
    }
    fn init_library(bin: &Path, cfg: &mut Config) {
        cfg.library.clear();
        for shape in SHAPES {
            for (color, _) in COLORS {
                let id = cursor_id(shape, color);
                if backend::cursor_path(&id, bin).is_file() {
                    cfg.library.push(id);
                }
            }
        }
        if !cfg.library.contains(&cfg.preferred_cursor) {
            let default = cursor_id("Arrow", "Blue");
            cfg.preferred_cursor = if cfg.library.contains(&default) {
                default
            } else {
                cfg.library.first().cloned().unwrap_or_default()
            };
        }
    }
    fn start_hotkeys(shared: Arc<Mutex<Shared>>, alive: Arc<AtomicBool>, path: PathBuf) {
        thread::spawn(move || unsafe {
            let keys = [(101, b'Y'), (102, b'1'), (103, b'2'), (104, b'D')];
            let registered: Vec<_> = keys
                .into_iter()
                .filter_map(|(id, key)| {
                    (RegisterHotKey(
                        null_mut(),
                        id,
                        MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                        key as u32,
                    ) != 0)
                        .then_some(id)
                })
                .collect();
            if registered.len() != keys.len() {
                shared.lock().unwrap().hotkey_ok = false;
            }
            let mut msg: MSG = std::mem::zeroed();
            while alive.load(Ordering::Relaxed) {
                while PeekMessageW(&mut msg, null_mut(), WM_HOTKEY, WM_HOTKEY, PM_REMOVE) != 0 {
                    let active_app = backend::foreground_path();
                    let mut state = shared.lock().unwrap();
                    let changed = match msg.wParam as i32 {
                        101 => {
                            state.config.enabled = !state.config.enabled;
                            true
                        }
                        102 | 103 => match active_app.as_deref() {
                            Some(path) => state
                                .config
                                .cycle_app_cursor(path, msg.wParam as i32 == 102),
                            None => state.config.cycle_cursor(msg.wParam as i32 == 102),
                        },
                        104 => {
                            if let Some(path) = active_app.as_deref() {
                                state.config.set_app_cursor(path, "@original");
                            } else {
                                state.config.enabled = false;
                            }
                            true
                        }
                        _ => false,
                    };
                    if changed {
                        if let Err(e) = model::save(&path, &state.config) {
                            state.status = format!("Could not save settings: {e}");
                        }
                    }
                }
                thread::sleep(Duration::from_millis(50));
            }
            for id in registered {
                UnregisterHotKey(null_mut(), id);
            }
        });
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Page {
        Cursor,
        Applications,
        Settings,
    }
    struct App {
        shared: Arc<Mutex<Shared>>,
        alive: Arc<AtomicBool>,
        config_path: PathBuf,
        bin: PathBuf,
        previews: HashMap<String, egui::TextureHandle>,
        app_icons: HashMap<String, Option<egui::TextureHandle>>,
        brand: egui::TextureHandle,
        page: Page,
        error: String,
        tray_rx: Receiver<tray::Event>,
        tray_thread: Option<thread::JoinHandle<()>>,
        tray_ready: bool,
        hidden: bool,
    }
    impl Drop for App {
        fn drop(&mut self) {
            self.alive.store(false, Ordering::Relaxed);
            if let Some(t) = self.tray_thread.take() {
                let _ = t.join();
            }
        }
    }
    impl App {
        fn new(cc: &eframe::CreationContext<'_>, dir: PathBuf) -> Self {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            if let Ok(font) = fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
                let mut fonts = egui::FontDefinitions::default();
                fonts
                    .font_data
                    .insert("Segoe UI".into(), egui::FontData::from_owned(font).into());
                fonts
                    .families
                    .get_mut(&egui::FontFamily::Proportional)
                    .unwrap()
                    .insert(0, "Segoe UI".into());
                cc.egui_ctx.set_fonts(fonts);
            }
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals.window_fill = BG;
            style.visuals.panel_fill = BG;
            style.visuals.override_text_color = Some(Color32::from_rgb(242, 239, 250));
            style.spacing.item_spacing = Vec2::new(10., 10.);
            cc.egui_ctx.set_style(style);
            let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/brand.png"))
                .expect("valid application icon");
            let brand = cc.egui_ctx.load_texture(
                "brand",
                egui::ColorImage::from_rgba_unmultiplied(
                    [icon.width as usize, icon.height as usize],
                    &icon.rgba,
                ),
                egui::TextureOptions::LINEAR,
            );
            let config_path = dir.join("config.json");
            let mut error = String::new();
            let mut config = match model::load(&config_path) {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
                Err(e) => {
                    error = format!("Could not read settings: {e}");
                    Config::default()
                }
            };
            let bin = match bundle::prepare(&dir) {
                Ok(Some(path)) => path,
                Ok(None) => std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(Path::to_path_buf))
                    .unwrap_or_default(),
                Err(e) => {
                    error = format!("Could not unpack application files: {e}");
                    PathBuf::new()
                }
            };
            init_library(&bin, &mut config);
            if config.library.is_empty() {
                if error.is_empty() {
                    error = "Cursor files not found. Build the standalone package with scripts/build.ps1.".into();
                }
            }
            let shared = Arc::new(Mutex::new(Shared {
                config,
                active: false,
                status: "Waiting for an active application".into(),
                hotkey_ok: true,
                hovered_profile: None,
            }));
            let alive = Arc::new(AtomicBool::new(true));
            let window = cc
                .window_handle()
                .ok()
                .and_then(|h| match h.as_raw() {
                    RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
                    _ => None,
                })
                .unwrap_or(0);
            let (tray_rx, tray_thread) =
                tray::start(cc.egui_ctx.clone(), alive.clone(), dir.clone(), window);
            backend::start(
                shared.clone(),
                alive.clone(),
                bin.clone(),
                config_path.clone(),
            );
            overlay::start(shared.clone(), alive.clone(), bin.clone());
            start_hotkeys(shared.clone(), alive.clone(), config_path.clone());
            Self {
                shared,
                alive,
                config_path,
                bin,
                previews: HashMap::new(),
                app_icons: HashMap::new(),
                brand,
                page: Page::Cursor,
                error,
                tray_rx,
                tray_thread: Some(tray_thread),
                tray_ready: false,
                hidden: false,
            }
        }
        fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
            ui.label(RichText::new(title).size(26.).strong());
            ui.label(RichText::new(subtitle).size(14.).color(MUTED));
            ui.add_space(15.);
        }
        fn card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
            egui::Frame::new()
                .fill(CARD)
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(49, 41, 69)))
                .corner_radius(14)
                .inner_margin(18)
                .show(ui, body);
        }
        fn applications_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, cfg: &mut Config) {
            Self::heading(
                ui,
                "Applications",
                "Choose how the cursor behaves in each app.",
            );
            let default = cursor_parts(&cfg.preferred_cursor)
                .map(|(shape, color)| format!("{shape} · {color}"))
                .unwrap_or_else(|| "Not selected".into());
            if !self.previews.contains_key(&cfg.preferred_cursor) {
                if let Some(image) =
                    overlay::preview(&backend::cursor_path(&cfg.preferred_cursor, &self.bin), 48)
                {
                    self.previews.insert(
                        cfg.preferred_cursor.clone(),
                        ctx.load_texture(
                            &cfg.preferred_cursor,
                            image,
                            egui::TextureOptions::LINEAR,
                        ),
                    );
                }
            }
            egui::Frame::new()
                .fill(Color32::from_rgb(38, 27, 62))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(78, 58, 120)))
                .corner_radius(12)
                .inner_margin(14)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if let Some(image) = self.previews.get(&cfg.preferred_cursor) {
                            ui.add(egui::Image::new(image).fit_to_exact_size(Vec2::splat(38.)));
                        }
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("DEFAULT CURSOR")
                                    .size(10.)
                                    .color(Color32::from_rgb(186, 165, 237))
                                    .strong(),
                            );
                            ui.label(RichText::new(&default).size(15.).strong());
                        });
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), 38.),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .add(
                                        egui::Button::new("Change")
                                            .fill(Color32::from_rgb(67, 49, 105))
                                            .corner_radius(6)
                                            .min_size(Vec2::new(72., 28.)),
                                    )
                                    .clicked()
                                {
                                    self.page = Page::Cursor;
                                }
                            },
                        );
                    });
                });
            ui.add_space(20.);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("SAVED APPLICATIONS")
                        .size(11.)
                        .strong()
                        .color(MUTED),
                );
                ui.label(
                    RichText::new(format!("{}", cfg.profiles.len()))
                        .size(11.)
                        .color(Color32::from_rgb(187, 165, 241)),
                );
            });
            ui.add_space(5.);
            if cfg.profiles.is_empty() {
                Self::card(ui, |ui| {
                    ui.label(RichText::new("No applications yet").size(15.).strong());
                    ui.label(
                        RichText::new("Apps appear here when their cursor is changed.")
                            .color(MUTED),
                    );
                });
            }
            for profile in &mut cfg.profiles {
                let path = profile.executable.clone();
                let file = Path::new(&path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&path);
                let name = match file.to_ascii_lowercase().as_str() {
                    "explorer.exe" => "File Explorer",
                    "msedgewebview2.exe" => "Microsoft Edge WebView",
                    "chrome.exe" => "Google Chrome",
                    "chatgpt.exe" => "Codex",
                    _ => file.strip_suffix(".exe").unwrap_or(file),
                };
                let custom = cursor_parts(&profile.cursor)
                    .map(|(shape, color)| format!("{shape} · {color}"));
                let selected = if profile.cursor == "@original" {
                    "Original cursor".to_owned()
                } else {
                    custom
                        .clone()
                        .unwrap_or_else(|| format!("Global · {default}"))
                };
                let icon = self.app_icons.entry(path.clone()).or_insert_with(|| {
                    overlay::app_icon(Path::new(&path), 40).map(|image| {
                        ctx.load_texture(format!("app:{path}"), image, egui::TextureOptions::LINEAR)
                    })
                });
                egui::Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(49, 41, 69)))
                    .corner_radius(10)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if let Some(icon) = icon.as_ref() {
                                ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(36.)));
                            } else {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(36.), egui::Sense::hover());
                                ui.painter()
                                    .rect_filled(rect, 9., Color32::from_rgb(61, 44, 95));
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    name.chars()
                                        .next()
                                        .unwrap_or('?')
                                        .to_uppercase()
                                        .to_string(),
                                    egui::FontId::proportional(17.),
                                    Color32::WHITE,
                                );
                            }
                            ui.vertical(|ui| {
                                ui.set_width(145.);
                                ui.spacing_mut().item_spacing.y = 3.;
                                ui.add(
                                    egui::Label::new(RichText::new(name).size(14.).strong())
                                        .truncate(),
                                )
                                .on_hover_text(&path);
                                ui.add(
                                    egui::Label::new(RichText::new(file).size(11.).color(MUTED))
                                        .truncate(),
                                )
                                .on_hover_text(&path);
                            });
                            ui.allocate_ui_with_layout(
                                Vec2::new(ui.available_width(), 40.),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.style_mut().visuals.widgets.inactive.bg_fill =
                                        Color32::from_rgb(43, 35, 59);
                                    ui.style_mut().visuals.widgets.inactive.weak_bg_fill =
                                        Color32::from_rgb(43, 35, 59);
                                    ui.style_mut().visuals.widgets.hovered.bg_fill =
                                        Color32::from_rgb(61, 49, 83);
                                    ui.style_mut().visuals.widgets.hovered.weak_bg_fill =
                                        Color32::from_rgb(61, 49, 83);
                                    ui.style_mut().spacing.button_padding = Vec2::new(9., 6.);
                                    egui::ComboBox::from_id_salt(&path)
                                        .selected_text(selected)
                                        .width(170.)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut profile.cursor,
                                                "@global".into(),
                                                "Use global cursor",
                                            );
                                            ui.selectable_value(
                                                &mut profile.cursor,
                                                "@original".into(),
                                                "Original cursor",
                                            );
                                            if let Some(label) = custom {
                                                ui.label(label);
                                            }
                                        });
                                },
                            );
                        });
                    });
                ui.add_space(6.);
            }
        }
        fn cursor_page(
            &mut self,
            ui: &mut egui::Ui,
            ctx: &egui::Context,
            cfg: &mut Config,
            status: &str,
            active: bool,
        ) {
            Self::heading(
                ui,
                "Cursor",
                "The default cursor for your applications. Changes apply automatically.",
            );
            if cfg.library.is_empty() {
                ui.colored_label(
                    Color32::LIGHT_RED,
                    "The cursor files are missing from this build.",
                );
                return;
            }
            let (shape, color) = cursor_parts(&cfg.preferred_cursor).unwrap_or(("Arrow", "Blue"));
            let shape = shape.to_owned();
            let color = color.to_owned();
            ui.label(RichText::new("SHAPE").size(11.).strong().color(MUTED));
            ui.add_space(2.);
            ui.horizontal_wrapped(|ui| {
                for item in SHAPES {
                    let id = cursor_id(item, &color);
                    let selected = item == shape;
                    let available = cfg.library.contains(&id);
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(64., 78.), egui::Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        10.,
                        if selected {
                            Color32::from_rgb(62, 45, 99)
                        } else {
                            CARD
                        },
                    );
                    ui.painter().rect_stroke(
                        rect,
                        10.,
                        Stroke::new(
                            1.0_f32,
                            if selected {
                                PURPLE
                            } else {
                                Color32::from_rgb(49, 41, 69)
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if available && !self.previews.contains_key(&id) {
                        if let Some(image) =
                            overlay::preview(&backend::cursor_path(&id, &self.bin), 48)
                        {
                            self.previews.insert(
                                id.clone(),
                                ctx.load_texture(&id, image, egui::TextureOptions::LINEAR),
                            );
                        }
                    }
                    if let Some(image) = self.previews.get(&id) {
                        let r = egui::Rect::from_center_size(
                            rect.center_top() + Vec2::new(0., 30.),
                            Vec2::splat(46.),
                        );
                        ui.painter().image(
                            image.id(),
                            r,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                            Color32::WHITE,
                        );
                    }
                    ui.painter().text(
                        rect.center_bottom() - Vec2::new(0., 13.),
                        egui::Align2::CENTER_CENTER,
                        item,
                        egui::FontId::proportional(11.),
                        if available { Color32::WHITE } else { MUTED },
                    );
                    if available && response.clicked() {
                        cfg.select_cursor(&id);
                    }
                }
            });
            ui.add_space(18.);
            ui.horizontal(|ui| {
                ui.label(RichText::new("COLOR").size(11.).strong().color(MUTED));
                ui.label(RichText::new(&color).color(MUTED).size(12.));
            });
            ui.add_space(2.);
            ui.horizontal_wrapped(|ui| {
                for (label, fill) in COLORS {
                    let id = cursor_id(&shape, label);
                    let available = cfg.library.contains(&id);
                    let selected = label == color;
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(30.), egui::Sense::click());
                    let inner = rect.shrink(3.);
                    ui.painter().rect_filled(inner, 4., fill);
                    if label == "Inverted" {
                        let half = inner.width() / 2.;
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(inner.min, Vec2::splat(half)),
                            0.,
                            Color32::WHITE,
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(inner.center(), Vec2::splat(half)),
                            0.,
                            Color32::WHITE,
                        );
                    }
                    ui.painter().rect_stroke(
                        rect,
                        6.,
                        Stroke::new(
                            if selected { 2.0_f32 } else { 1.0_f32 },
                            if selected {
                                PURPLE
                            } else {
                                Color32::from_rgb(68, 60, 82)
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if available && response.clicked() {
                        cfg.select_cursor(&id);
                    }
                    response.on_hover_text(label);
                }
            });
            ui.add_space(18.);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SIZE").size(11.).strong().color(MUTED));
                ui.add(egui::Slider::new(&mut cfg.cursor_size, 16..=256).suffix(" px"));
            });
            ui.add_space(10.);
            ui.separator();
            ui.add_space(4.);
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut cfg.enabled, "Enable MouseUp");
                ui.label(
                    RichText::new(if active { "Active" } else { status })
                        .color(MUTED)
                        .small(),
                );
            });
            ui.label(
                RichText::new("Ctrl+Alt+1 / 2: app shape   •   Ctrl+Alt+D: app original")
                    .color(MUTED)
                    .small(),
            );
        }
    }
    impl eframe::App for App {
        fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
            ctx.request_repaint_after(Duration::from_millis(200));
            let mut reopened = false;
            while let Ok(event) = self.tray_rx.try_recv() {
                match event {
                    tray::Event::Ready => self.tray_ready = true,
                    tray::Event::Hidden => self.hidden = true,
                    tray::Event::Unavailable => {
                        self.tray_ready = false;
                        self.error =
                            "Notification area unavailable; normal minimize remains enabled."
                                .into();
                    }
                    tray::Event::Open => {
                        self.hidden = false;
                        reopened = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    tray::Event::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                }
            }
            if self.tray_ready
                && !self.hidden
                && !reopened
                && ctx.input(|i| i.viewport().minimized == Some(true))
            {
                self.hidden = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            let (mut cfg, status, active, hotkey_ok) = {
                let s = self.shared.lock().unwrap();
                (s.config.clone(), s.status.clone(), s.active, s.hotkey_ok)
            };
            let before = cfg.clone();
            egui::SidePanel::left("navigation")
                .exact_width(155.)
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(22, 16, 37))
                        .inner_margin(16),
                )
                .show(ctx, |ui| {
                    background_gradient(
                        ui,
                        16.,
                        Color32::from_rgb(31, 21, 53),
                        Color32::from_rgb(18, 13, 31),
                    );
                    ui.add_space(10.);
                    ui.horizontal(|ui| {
                        ui.add(egui::Image::new(&self.brand).fit_to_exact_size(Vec2::splat(29.)));
                        ui.label(RichText::new("MouseUp").size(18.).strong());
                    });
                    ui.add_space(29.);
                    ui.label(RichText::new("WORKSPACE").size(10.).color(MUTED).strong());
                    ui.add_space(4.);
                    for (page, label) in [
                        (Page::Cursor, "Cursor"),
                        (Page::Applications, "Applications"),
                        (Page::Settings, "Settings"),
                    ] {
                        let selected = self.page == page;
                        let button = egui::Button::new(
                            RichText::new(label).size(14.).color(Color32::TRANSPARENT),
                        )
                        .fill(if selected {
                            Color32::from_rgb(70, 48, 125)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::NONE)
                        .corner_radius(8)
                        .min_size(Vec2::new(120., 38.));
                        let response = ui.add(button);
                        let color = if selected { Color32::WHITE } else { MUTED };
                        ui.painter().text(
                            egui::pos2(response.rect.left() + 34., response.rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(14.),
                            color,
                        );
                        let origin =
                            egui::pos2(response.rect.left() + 12., response.rect.center().y - 7.);
                        let stroke = Stroke::new(1.3_f32, color);
                        match page {
                            Page::Cursor => {
                                ui.painter().add(egui::Shape::line(
                                    vec![
                                        origin,
                                        origin + Vec2::new(0., 13.),
                                        origin + Vec2::new(4., 9.),
                                        origin + Vec2::new(7., 14.),
                                        origin + Vec2::new(9., 13.),
                                        origin + Vec2::new(6., 8.),
                                        origin + Vec2::new(12., 8.),
                                        origin,
                                    ],
                                    stroke,
                                ));
                            }
                            Page::Applications => {
                                for y in [0., 7.] {
                                    for x in [0., 7.] {
                                        ui.painter().rect_stroke(
                                            egui::Rect::from_min_size(
                                                origin + Vec2::new(x, y),
                                                Vec2::splat(5.),
                                            ),
                                            1.,
                                            stroke,
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                }
                            }
                            Page::Settings => {
                                let center = origin + Vec2::splat(6.);
                                ui.painter().circle_stroke(center, 5.5, stroke);
                                ui.painter().circle_filled(center, 1.5, color);
                                for d in [
                                    Vec2::new(0., -8.),
                                    Vec2::new(0., 8.),
                                    Vec2::new(-8., 0.),
                                    Vec2::new(8., 0.),
                                ] {
                                    ui.painter()
                                        .line_segment([center + d * 0.7, center + d], stroke);
                                }
                            }
                        }
                        if response.clicked() {
                            self.page = page;
                        }
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        ui.label(
                            RichText::new(if cfg.enabled {
                                "●  Cursor enabled"
                            } else {
                                "●  Paused"
                            })
                            .color(if cfg.enabled {
                                Color32::from_rgb(116, 207, 168)
                            } else {
                                MUTED
                            })
                            .small(),
                        );
                    });
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(BG).inner_margin(24))
                .show(ctx, |ui| {
                    background_gradient(ui, 24., Color32::from_rgb(24, 15, 43), BG);
                    if !self.error.is_empty() {
                        ui.colored_label(Color32::LIGHT_RED, &self.error);
                    }
                    egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                        Page::Cursor => self.cursor_page(ui, ctx, &mut cfg, &status, active),
                        Page::Applications => self.applications_page(ui, ctx, &mut cfg),
                        Page::Settings => {
                            Self::heading(
                                ui,
                                "Settings",
                                "Control how MouseUp works in the background.",
                            );
                            Self::card(ui, |ui| {
                                ui.checkbox(
                                    &mut cfg.software_overlay,
                                    "Show the selected cursor when Windows hides it",
                                );
                                ui.label(
                                    RichText::new(
                                        "Some games may still draw their own cursor underneath.",
                                    )
                                    .color(MUTED)
                                    .small(),
                                );
                                ui.add_space(10.);
                                if self.tray_ready && ui.button("Hide window to tray").clicked() {
                                    self.hidden = true;
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                                }
                                ui.label(
                                    RichText::new("Minimize to tray; click its icon to reopen.")
                                        .color(MUTED)
                                        .small(),
                                );
                                ui.add_space(12.);
                                ui.label(RichText::new("HOTKEYS").color(MUTED).size(11.).strong());
                                ui.label("Ctrl+Alt+1 / 2  —  change shape for active app");
                                ui.label("Ctrl+Alt+D  —  original cursor for active app");
                                ui.label("Ctrl+Alt+Y  —  pause / resume");
                                if !hotkey_ok {
                                    ui.colored_label(
                                        Color32::YELLOW,
                                        "One or more hotkeys are already in use.",
                                    );
                                }
                            });
                            ui.add_space(12.);
                            ui.collapsing("Open-source licenses", |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(180.)
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(include_str!(
                                                "../THIRD_PARTY_NOTICES.txt"
                                            ))
                                            .monospace()
                                            .size(11.),
                                        );
                                    });
                            });
                        }
                    });
                });
            if cfg != before {
                let mut shared = self.shared.lock().unwrap();
                shared.config.apply_ui_changes(&before, &cfg);
                if let Err(e) = model::save(&self.config_path, &shared.config) {
                    self.error = format!("Could not save settings: {e}");
                } else {
                    self.error.clear();
                }
            }
        }
    }
    pub fn run() {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS},
            System::Threading::CreateMutexW,
        };
        let name: Vec<u16> = "Local\\MouseUp-single-instance\0".encode_utf16().collect();
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if mutex.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            if !mutex.is_null() {
                unsafe {
                    let class: Vec<u16> = "MouseUpTrayWindow\0".encode_utf16().collect();
                    let previous = FindWindowW(class.as_ptr(), class.as_ptr());
                    if !previous.is_null() {
                        PostMessageW(previous, tray::RESTORE_MESSAGE, 0, 0);
                    }
                    CloseHandle(mutex);
                }
            }
            return;
        }
        let dir = match model::data_dir() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([780., 560.])
                .with_min_inner_size([710., 500.])
                .with_icon(
                    eframe::icon_data::from_png_bytes(include_bytes!("../assets/brand.png"))
                        .expect("valid application icon"),
                )
                .with_title("MouseUp"),
            ..Default::default()
        };
        if let Err(e) = eframe::run_native(
            "MouseUp",
            options,
            Box::new(move |cc| Ok(Box::new(App::new(cc, dir)))),
        ) {
            eprintln!("{e}");
        }
        unsafe {
            CloseHandle(mutex);
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn library_only_contains_requested_shapes_and_migrates_old_selection() {
            let dir =
                std::env::temp_dir().join(format!("mouseup-curated-test-{}", std::process::id()));
            for shape in ["Arrow", "Circle", "Bag"] {
                let p = dir.join("Cursors").join(shape);
                fs::create_dir_all(&p).unwrap();
                fs::write(p.join("Blue.cur"), []).unwrap();
            }
            let mut c = Config::default();
            c.preferred_cursor = "@yolo\\Bag\\Blue.cur".into();
            init_library(&dir, &mut c);
            assert_eq!(c.library.len(), 2);
            assert_eq!(c.preferred_cursor, cursor_id("Arrow", "Blue"));
            assert!(!c.library.iter().any(|p| p.contains("Bag")));
            assert_eq!(
                cursor_parts(&cursor_id("Circle", "Blue")),
                Some(("Circle", "Blue"))
            );
            fs::remove_dir_all(dir).unwrap();
        }
        #[test]
        #[ignore = "requires original cursor files extracted from the supplied installer"]
        fn all_curated_cursors_load_and_preview() {
            let root = std::env::var_os("MOUSEUP_TEST_ASSET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/yolomouse"));
            let mut c = Config::default();
            init_library(&root, &mut c);
            assert_eq!(c.library.len(), SHAPES.len() * COLORS.len());
            for id in &c.library {
                let path = backend::cursor_path(id, &root);
                backend::validate_cursor(&path).unwrap();
                assert!(overlay::preview(&path, 64).is_some());
            }
        }
    }
}
#[cfg(windows)]
fn main() {
    app::run();
}
