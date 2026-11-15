use std::{path::Path, sync::{Arc, Mutex, mpsc::{self, Receiver}}};
use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::{Code, HotKey, Modifiers}};
use tray_icon::{menu::{Menu, MenuEvent, MenuItem}, Icon, TrayIcon, TrayIconBuilder};

use crate::{backend, model, tray};

pub struct Mac {
    _tray: Option<TrayIcon>,
    _hotkeys: Option<GlobalHotKeyManager>,
    ids: Vec<(u32, u8)>,
    events: Receiver<GlobalHotKeyEvent>,
    hotkeys_ok: bool,
}

impl Mac {
    pub fn new(ctx: &egui::Context) -> Self {
        MenuEvent::set_event_handler(Some(|event: MenuEvent| match event.id.as_ref() {
            "mouseup-open" => tray::emit(tray::Event::Open),
            "mouseup-quit" => tray::emit(tray::Event::Quit),
            _ => (),
        }));
        let tray = (|| {
            let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/brand.png")).ok()?;
            let icon = Icon::from_rgba(icon.rgba, icon.width, icon.height).ok()?;
            let menu = Menu::new();
            let open = MenuItem::with_id("mouseup-open", "Open MouseUp", true, None);
            let quit = MenuItem::with_id("mouseup-quit", "Exit MouseUp", true, None);
            menu.append_items(&[&open, &quit]).ok()?;
            TrayIconBuilder::new().with_tooltip("MouseUp").with_icon(icon).with_menu(Box::new(menu)).build().ok()
        })();
        tray::emit(if tray.is_some() { tray::Event::Ready } else { tray::Event::Unavailable });

        let (sender, events) = mpsc::channel();
        let repaint = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event| { let _ = sender.send(event); repaint.request_repaint(); }));
        let hotkeys = GlobalHotKeyManager::new().ok();
        let mut ids = Vec::new();
        if let Some(manager) = &hotkeys {
            for (action, code) in [(1, Code::Digit1), (2, Code::Digit2), (3, Code::KeyD), (4, Code::KeyY)] {
                let key = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), code);
                if manager.register(key).is_ok() { ids.push((key.id(), action)); }
            }
        }
        let hotkeys_ok = ids.len() == 4;
        Self { _tray: tray, _hotkeys: hotkeys, ids, events, hotkeys_ok }
    }

    pub fn poll(&mut self, shared: &Arc<Mutex<backend::Shared>>, config_path: &Path) {
        shared.lock().unwrap().hotkeys = self.hotkeys_ok;
        while let Ok(event) = self.events.try_recv() {
            if event.state != HotKeyState::Pressed { continue; }
            let Some((_, action)) = self.ids.iter().find(|(id, _)| *id == event.id) else { continue; };
            let current = backend::frontmost_app();
            let mut state = shared.lock().unwrap();
            match action {
                1 | 2 => {
                    if let Some(path) = &current { state.config.cycle_app_cursor(path, *action == 1); }
                    else { state.config.cycle_cursor(*action == 1); }
                }
                3 => if let Some(path) = &current { state.config.set_app_cursor(path, "@original"); },
                4 => state.config.enabled = !state.config.enabled,
                _ => (),
            }
            if let Err(e) = model::save(config_path, &state.config) { state.status = format!("Could not save settings: {e}"); }
        }
    }
}
