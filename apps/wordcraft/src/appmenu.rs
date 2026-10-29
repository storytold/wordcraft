//! Host glue only; transport and platform association live in craft-desktop.
use wordcraft_ui_egui::{WordApp, native_menu};

pub struct Bridge(craft_desktop::appmenu::Bridge);

impl Bridge {
    pub fn start(cc: &eframe::CreationContext<'_>, app: &WordApp) -> Option<Self> {
        let ctx = cc.egui_ctx.clone();
        craft_desktop::appmenu::Bridge::start(
            cc.winit_window()?.clone(),
            "ai.storyteller.wordcraft",
            "WORDCRAFT",
            native_menu::snapshot(app),
            move || ctx.request_repaint(),
        )
        .map(Self)
    }
    pub fn publish(&self, app: &WordApp) {
        self.0.publish(native_menu::snapshot(app));
    }
    pub fn poll(&mut self, app: &mut WordApp, ctx: &egui::Context) {
        for click in self.0.take_events() {
            // Fresh viewport focus can arrive before the app swaps in that window's state.
            if ctx.input(|i| i.raw.viewports.iter().any(|(id, v)| *id != egui::ViewportId::ROOT && v.focused == Some(true))) {
                continue;
            }
            if let Err(e) = native_menu::activate(app, click.document, &click.action) {
                log::debug!("global menu: {e}");
            }
        }
    }
}
