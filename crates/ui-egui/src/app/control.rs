use super::*;
use crate::control;

impl WordApp {
    /// One callback command per admitted request, and the synthetic-input queue's final pass.
    pub(super) fn flush_shot_and_synthetic(&mut self, ctx: &egui::Context) {
        // This is in `ui` so the captured frame is the layout just drawn; a discarded pass is not
        // a layout anyone will see. The shot's `issued` flag makes repeated final passes harmless.
        if !ctx.will_discard() {
            control::issue_screenshots(self, ctx);
        }
        // The raw input was consumed by this host frame's passes; only a final UI pass proves
        // the step is done. A discarded pass keeps the queue for the next UI frame.
        if self.synthetic_pending && !ctx.will_discard() {
            let n = self.synthetic_injected.min(self.synthetic.len());
            self.synthetic.drain(..n);
            self.synthetic_pending = false;
            self.synthetic_injected = 0;
            if !self.synthetic.is_empty() {
                ctx.request_repaint();
            }
        }
    }

    /// Make the capture geometry the active window's surface, viewport and document. The epoch is
    /// monotonic: resizing back to an earlier size is still a new epoch.
    pub(crate) fn refresh_capture_geometry(&mut self, ctx: &egui::Context) {
        let (viewport, surface) = self.active_surface_geometry(ctx);
        let geometry = surface.map(|surface| surface.with_target(viewport, self.session.document_id()));
        if self.capture_geometry != geometry {
            self.capture_geometry = geometry;
            self.capture_epoch = self.capture_epoch.wrapping_add(1);
        }
    }

    /// Drains queued control requests, dispatches each through `control::handle`, sends its reply or
    /// admits a screenshot, then puts the receiver back.
    pub(super) fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            match control::handle(self, ctx, &req) {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path } => control::admit_screenshot(self, ctx, path, reply),
            }
        }
        self.control_rx = Some(rx);
    }
}
