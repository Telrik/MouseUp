use std::sync::{mpsc::{self, Receiver, Sender}, Arc, OnceLock, atomic::AtomicBool};
use std::thread;

pub enum Event { Ready, Unavailable, Open, Quit }
static CHANNEL: OnceLock<(Sender<Event>, eframe::egui::Context)> = OnceLock::new();

pub fn emit(event: Event) {
    if let Some((sender, ctx)) = CHANNEL.get() { let _ = sender.send(event); ctx.request_repaint(); }
}

pub fn start(ctx: eframe::egui::Context, _: Arc<AtomicBool>) -> (Receiver<Event>, thread::JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let _ = CHANNEL.set((sender, ctx));
    (receiver, thread::spawn(|| {}))
}
