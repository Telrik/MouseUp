use ksni::blocking::TrayMethods;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::Duration,
};

pub enum Event {
    Ready,
    Unavailable,
    Open,
    Quit,
}

struct Tray {
    sender: Sender<Event>,
    ctx: eframe::egui::Context,
}

impl Tray {
    fn emit(&self, event: Event) {
        let _ = self.sender.send(event);
        self.ctx.request_repaint();
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "mouseup".into()
    }
    fn title(&self) -> String {
        "MouseUp".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let Ok(image) = image::load_from_memory(include_bytes!("../../assets/brand.png")) else {
            return Vec::new();
        };
        let image = image.to_rgba8();
        let (width, height) = image.dimensions();
        let mut data = image.into_raw();
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width: width as i32,
            height: height as i32,
            data,
        }]
    }
    fn activate(&mut self, _: i32, _: i32) {
        self.emit(Event::Open);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem {
                label: "Open MouseUp".into(),
                activate: Box::new(|tray: &mut Self| tray.emit(Event::Open)),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: "Exit MouseUp".into(),
                activate: Box::new(|tray: &mut Self| tray.emit(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.emit(Event::Unavailable);
        true
    }
    fn watcher_online(&self) {
        self.emit(Event::Ready);
    }
}

pub fn start(
    ctx: eframe::egui::Context,
    alive: Arc<AtomicBool>,
) -> (Receiver<Event>, thread::JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let tray = Tray {
            sender: sender.clone(),
            ctx: ctx.clone(),
        };
        let handle = match tray.spawn() {
            Ok(handle) => {
                let _ = sender.send(Event::Ready);
                ctx.request_repaint();
                handle
            }
            Err(_) => {
                let _ = sender.send(Event::Unavailable);
                ctx.request_repaint();
                return;
            }
        };
        while alive.load(Ordering::Relaxed) && !handle.is_closed() {
            thread::sleep(Duration::from_millis(100));
        }
        handle.shutdown().wait();
    });
    (receiver, handle)
}
