#[cfg(target_os = "linux")]
mod backend;
#[cfg(target_os = "macos")]
#[path = "../../macos/src/backend.rs"]
mod backend;
#[path = "../../src/catalog.rs"]
mod catalog;
mod cursor;
#[path = "../../src/model.rs"]
mod model;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(target_os = "macos")]
#[path = "../../macos/src/tray.rs"]
mod tray;
#[cfg(target_os = "macos")]
#[path = "../../macos/src/platform.rs"]
mod platform;

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Arc, Mutex,
    },
    thread,
};

const COLORS: [Color32; 12] = [
    Color32::from_rgb(37, 166, 248),
    Color32::from_rgb(33, 73, 153),
    Color32::from_rgb(65, 203, 109),
    Color32::from_rgb(242, 213, 62),
    Color32::from_rgb(242, 143, 46),
    Color32::from_rgb(224, 68, 75),
    Color32::from_rgb(241, 116, 178),
    Color32::from_rgb(161, 99, 234),
    Color32::from_rgb(245, 245, 248),
    Color32::from_rgb(164, 172, 185),
    Color32::from_rgb(45, 47, 57),
    Color32::from_rgb(120, 120, 130),
];
const MUTED: Color32 = Color32::from_rgb(157, 151, 177);
const PURPLE: Color32 = Color32::from_rgb(141, 99, 244);
const CARD: Color32 = Color32::from_rgb(27, 21, 43);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Cursor,
    Applications,
    Settings,
}

struct App {
    shared: Arc<Mutex<backend::Shared>>,
    alive: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    tray_worker: Option<thread::JoinHandle<()>>,
    tray_rx: Receiver<tray::Event>,
    tray_ready: bool,
    hidden: bool,
    quitting: bool,
    #[cfg(target_os = "macos")]
    mac: Option<platform::Mac>,
    #[cfg(target_os = "macos")]
    overlay: HashMap<String, (egui::TextureHandle, (u16, u16))>,
    config_path: PathBuf,
    previews: HashMap<String, egui::TextureHandle>,
    page: Page,
    error: String,
}

impl Drop for App {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.tray_worker.take() {
            let _ = worker.join();
        }
    }
}

impl App {
    #[cfg(target_os = "macos")]
    fn show_mac_overlay(&mut self, ctx: &egui::Context) {
        let (config, pointer) = {
            let state = self.shared.lock().unwrap();
            (state.config.clone(), state.pointer)
        };
        let Some(pointer) = pointer else { return; };
        let frontmost = backend::frontmost_app();
        let profile = frontmost.as_deref().map(|p| config.app_profile(p)).unwrap_or_else(|| config.app_profile(""));
        let Some(profile) = profile else { return; };
        let key = format!("{}:{}", profile.cursor, profile.size);
        if !self.overlay.contains_key(&key) {
            let Ok((image, hotspot)) = cursor::decode(&profile.cursor, profile.size) else { return; };
            let size = profile.size as usize;
            let pixels = image.into_raw();
            let texture = ctx.load_texture(&key, egui::ColorImage::from_rgba_unmultiplied([size, size], &pixels), egui::TextureOptions::LINEAR);
            self.overlay.insert(key.clone(), (texture, hotspot));
        }
        let Some((texture, hotspot)) = self.overlay.get(&key).cloned() else { return; };
        let size = profile.size as f32;
        let builder = egui::ViewportBuilder::default()
            .with_title("MouseUp cursor")
            .with_inner_size([size, size])
            .with_position(egui::pos2(pointer[0] - hotspot.0 as f32, pointer[1] - hotspot.1 as f32))
            .with_transparent(true)
            .with_decorations(false)
            .with_has_shadow(false)
            .with_resizable(false)
            .with_active(false)
            .with_always_on_top()
            .with_mouse_passthrough(true);
        ctx.show_viewport_deferred(egui::ViewportId::from_hash_of("mouseup-cursor"), builder, move |ctx, _| {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::TRANSPARENT).inner_margin(0)).show(ctx, |ui| {
                ui.painter().image(texture.id(), ui.max_rect(), egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)), Color32::WHITE);
            });
        });
    }

    fn new(cc: &eframe::CreationContext<'_>, config_path: PathBuf) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals.window_fill = Color32::from_rgb(13, 9, 27);
        style.visuals.panel_fill = Color32::from_rgb(13, 9, 27);
        style.visuals.override_text_color = Some(Color32::from_rgb(242, 239, 250));
        style.spacing.item_spacing = Vec2::new(10., 10.);
        cc.egui_ctx.set_style(style);
        let mut error = String::new();
        let mut config = match model::load(&config_path) {
            Ok(config) => config,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => model::Config::default(),
            Err(e) => {
                error = format!("Could not read settings: {e}");
                model::Config::default()
            }
        };
        config.library = catalog::SHAPES
            .iter()
            .flat_map(|shape| {
                cursor::COLORS
                    .iter()
                    .map(move |color| cursor::id(shape, color))
            })
            .collect();
        if !config.library.contains(&config.preferred_cursor) {
            config.preferred_cursor = cursor::id("Arrow", "Blue");
        }
        let shared = Arc::new(Mutex::new(backend::Shared {
            config,
            status: if cfg!(target_os = "macos") { "Starting macOS overlay" } else { "Starting X11" }.into(),
            active: false,
            hotkeys: true,
            current_app: None,
            #[cfg(target_os = "macos")]
            pointer: None,
        }));
        let alive = Arc::new(AtomicBool::new(true));
        let (tray_rx, tray_worker) = tray::start(cc.egui_ctx.clone(), alive.clone());
        let worker = backend::start(
            shared.clone(),
            alive.clone(),
            config_path.clone(),
            cc.egui_ctx.clone(),
        );
        Self {
            shared,
            alive,
            worker: Some(worker),
            tray_worker: Some(tray_worker),
            tray_rx,
            tray_ready: false,
            hidden: false,
            quitting: false,
            #[cfg(target_os = "macos")]
            mac: None,
            #[cfg(target_os = "macos")]
            overlay: HashMap::new(),
            config_path,
            previews: HashMap::new(),
            page: Page::Cursor,
            error,
        }
    }

    fn preview(&mut self, ctx: &egui::Context, id: &str) -> Option<egui::TextureHandle> {
        if !self.previews.contains_key(id) {
            if let Ok((image, _)) = cursor::decode(id, 48) {
                let rgba = image.into_raw();
                let image = egui::ColorImage::from_rgba_unmultiplied([48, 48], &rgba);
                self.previews.insert(
                    id.to_owned(),
                    ctx.load_texture(id, image, egui::TextureOptions::LINEAR),
                );
            }
        }
        self.previews.get(id).cloned()
    }

    fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.label(RichText::new(title).size(26.).strong());
        ui.label(RichText::new(subtitle).size(14.).color(MUTED));
        ui.add_space(15.);
    }

    fn cursors(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        config: &mut model::Config,
        status: &str,
    ) {
        Self::heading(
            ui,
            "Cursor",
            "The default cursor for your applications. Changes apply automatically.",
        );
        let (shape, color) = cursor::parts(&config.preferred_cursor).unwrap_or(("Arrow", "Blue"));
        let shape = shape.to_owned();
        let color = color.to_owned();
        ui.label(RichText::new("SHAPE").size(11.).strong().color(MUTED));
        let width = ((ui.available_width() - 48.) / 8.).clamp(52., 64.);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 6.;
            ui.horizontal_wrapped(|ui| {
                for item in catalog::SHAPES {
                    let id = cursor::id(item, &color);
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(width, 78.), egui::Sense::click());
                    let selected = item == shape;
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
                            1_f32,
                            if selected {
                                PURPLE
                            } else {
                                Color32::from_rgb(49, 41, 69)
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if let Some(image) = self.preview(ctx, &id) {
                        ui.painter().image(
                            image.id(),
                            egui::Rect::from_center_size(
                                rect.center_top() + Vec2::new(0., 30.),
                                Vec2::splat(46.),
                            ),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                            Color32::WHITE,
                        );
                    }
                    ui.painter().text(
                        rect.center_bottom() - Vec2::new(0., 13.),
                        egui::Align2::CENTER_CENTER,
                        item,
                        egui::FontId::proportional(11.),
                        Color32::WHITE,
                    );
                    if response.clicked() {
                        config.select_cursor(&id);
                    }
                }
            });
        });
        ui.add_space(18.);
        ui.horizontal(|ui| {
            ui.label(RichText::new("COLOR").size(11.).strong().color(MUTED));
            ui.label(RichText::new(&color).color(MUTED).size(12.));
        });
        ui.horizontal_wrapped(|ui| {
            for (index, name) in cursor::COLORS.iter().enumerate() {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(30.), egui::Sense::click());
                ui.painter().rect_filled(rect.shrink(3.), 4., COLORS[index]);
                if *name == "Inverted" {
                    let inner = rect.shrink(3.);
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
                        if *name == color { 2_f32 } else { 1_f32 },
                        if *name == color {
                            PURPLE
                        } else {
                            Color32::from_rgb(68, 60, 82)
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
                if response.clicked() {
                    config.select_cursor(&cursor::id(&shape, name));
                }
                response.on_hover_text(*name);
            }
        });
        ui.add_space(18.);
        ui.horizontal(|ui| {
            ui.label(RichText::new("SIZE").size(11.).strong().color(MUTED));
            ui.add(egui::Slider::new(&mut config.cursor_size, 16..=256).suffix(" px"));
        });
        ui.add_space(10.);
        ui.separator();
        ui.add_space(4.);
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut config.enabled, "Enable MouseUp");
            ui.label(RichText::new(status).color(MUTED).small());
        });
        ui.label(
            RichText::new("Ctrl+Alt+1 / 2: app shape   •   Ctrl+Alt+D: app original")
                .color(MUTED)
                .small(),
        );
    }

    fn applications(&mut self, ui: &mut egui::Ui, config: &mut model::Config) {
        Self::heading(ui, "Applications", "Cursor choices saved for each app.");
        if config.profiles.is_empty() {
            ui.label(
                RichText::new("Apps appear here when their cursor is changed with hotkeys.")
                    .color(MUTED),
            );
        }
        for profile in &mut config.profiles {
            egui::Frame::new()
                .fill(CARD)
                .corner_radius(10)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let name = std::path::Path::new(&profile.executable)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&profile.executable);
                        ui.label(RichText::new(name).strong())
                            .on_hover_text(&profile.executable);
                        let selected = if profile.cursor == "@original" {
                            "Original cursor".to_owned()
                        } else if let Some((shape, color)) = cursor::parts(&profile.cursor) {
                            format!("{shape} · {color}")
                        } else {
                            "Use global cursor".to_owned()
                        };
                        egui::ComboBox::from_id_salt(&profile.executable)
                            .selected_text(selected)
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
                            });
                    });
                });
            ui.add_space(6.);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            if self.mac.is_none() { self.mac = Some(platform::Mac::new(ctx)); }
            if let Some(mac) = &mut self.mac { mac.poll(&self.shared, &self.config_path); }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut reopened = false;
        while let Ok(event) = self.tray_rx.try_recv() {
            match event {
                tray::Event::Ready => self.tray_ready = true,
                tray::Event::Unavailable => {
                    self.tray_ready = false;
                    if self.hidden {
                        self.hidden = false;
                        reopened = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    }
                }
                tray::Event::Open => {
                    self.hidden = false;
                    reopened = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::Event::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if self.tray_ready && !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hidden = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if self.tray_ready
            && !self.hidden
            && !reopened
            && ctx.input(|i| i.viewport().minimized == Some(true))
        {
            self.hidden = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        let (mut config, status, hotkeys) = {
            let state = self.shared.lock().unwrap();
            (state.config.clone(), state.status.clone(), state.hotkeys)
        };
        let before = config.clone();
        egui::SidePanel::left("navigation")
            .exact_width(155.)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(22, 16, 37))
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                ui.add_space(10.);
                ui.label(RichText::new("✦  MouseUp").size(18.).strong());
                ui.add_space(29.);
                ui.label(RichText::new("WORKSPACE").size(10.).color(MUTED).strong());
                ui.add_space(4.);
                for (page, label) in [
                    (Page::Cursor, "Cursor"),
                    (Page::Applications, "Applications"),
                    (Page::Settings, "Settings"),
                ] {
                    let button = egui::Button::new(RichText::new(label).size(14.))
                        .fill(if self.page == page {
                            Color32::from_rgb(70, 48, 125)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::NONE)
                        .corner_radius(8)
                        .min_size(Vec2::new(120., 38.));
                    if ui.add(button).clicked() {
                        self.page = page;
                    }
                }
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(13, 9, 27))
                    .inner_margin(24),
            )
            .show(ctx, |ui| {
                if !self.error.is_empty() {
                    ui.colored_label(Color32::LIGHT_RED, &self.error);
                }
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::Cursor => self.cursors(ui, ctx, &mut config, &status),
                    Page::Applications => self.applications(ui, &mut config),
                    Page::Settings => {
                        Self::heading(ui, "Settings", if cfg!(target_os = "macos") { "MouseUp for macOS" } else { "MouseUp for X11" });
                        ui.label("Ctrl+Alt+1 / 2 — change the active application's cursor");
                        ui.label("Ctrl+Alt+D — restore the active application's cursor");
                        ui.label("Ctrl+Alt+Y — pause or resume");
                        if !hotkeys {
                            ui.colored_label(Color32::YELLOW, "Some hotkeys are already in use.");
                        }
                        ui.add_space(12.);
                        if self.tray_ready {
                            if ui.button("Hide window to tray").clicked() {
                                self.hidden = true;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                            }
                            ui.label(
                                RichText::new("Close or minimize to tray; use its menu to exit.")
                                    .color(MUTED),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "System tray is unavailable; closing the window exits MouseUp.",
                                )
                                .color(MUTED),
                            );
                        }
                        ui.label(
                            RichText::new(
                                if cfg!(target_os = "macos") { "The system cursor and game-drawn cursors may remain visible beneath the overlay." } else { "Games that draw their own cursor may still show it underneath." },
                            )
                            .color(MUTED),
                        );
                    }
                });
            });
        if config != before {
            let mut state = self.shared.lock().unwrap();
            state.config.apply_ui_changes(&before, &config);
            if let Err(e) = model::save(&self.config_path, &state.config) {
                self.error = format!("Could not save settings: {e}");
            } else {
                self.error.clear();
            }
        }
        #[cfg(target_os = "macos")]
        self.show_mac_overlay(ctx);
    }

    #[cfg(target_os = "macos")]
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] { [0., 0., 0., 0.] }
}

fn main() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("MouseUp requires an X11 display. Sign in to an X11 session and try again.");
        std::process::exit(1);
    }
    let config_path = match model::data_dir() {
        Ok(dir) => dir.join("config.json"),
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/brand.png"))
        .expect("valid icon");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780., 560.])
            .with_min_inner_size([710., 500.])
            .with_maximize_button(false)
            .with_icon(icon),
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "MouseUp",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, config_path)))),
    ) {
        eprintln!("{error}");
    }
}
