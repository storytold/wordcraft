use craft::automation::control::ControlChannel;
use craft::automation::screenshot::DocumentId;
use craft::ui::runtime::motion::now;
use craft::ui::runtime::windows::WindowHost;

use super::WordApp;
use crate::control;

impl WordApp {
    /// Answers the requests that arrive on `channel`: the control server's, or an example's or a test's own.
    pub fn with_control(mut self, channel: ControlChannel) -> Self {
        self.control = Some(channel);
        self
    }

    /// The active document as a screenshot sees it: its generation changes on every switch.
    fn capture_document(&self) -> Option<DocumentId> {
        Some(DocumentId::new(self.session.document_id(), self.session.rev()))
    }

    /// The control channel's logic phase: finishes captures and input waiters, then, once the
    /// fonts are warm, runs the queued requests and answers them.
    pub(crate) fn control_logic(&mut self, ctx: &egui::Context) {
        let Some(mut channel) = self.control.take() else { return };
        let host = WindowHost::of(ctx);
        let document = self.capture_document();
        channel.captures.note_windows(&host, document);
        let finished = channel.captures.drain(now(), document);
        channel.finish_captures(finished, &mut |path, bytes| control::write_automation_file(path, bytes));
        channel.poll_input(&host);
        channel.poll_access_trees(ctx);
        // A request run before the fonts are warm would see a layout with no usable atlas.
        if self.fonts_frames >= 2 {
            while let Some(req) = channel.next_request() {
                let reply = control::handle(self, ctx, &req);
                let document = self.capture_document();
                channel.settle(req, reply, document, f64::from(ctx.global_style().animation_time) * 1000.0);
            }
        }
        // The library does not wake the window for a queued capture or an access-tree wait; both
        // need passes to settle (`Captures` leaves the repainting to the app).
        if channel.waiting() > 0 || channel.captures.any_pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.control = Some(channel);
    }

    /// The control channel's UI phase: issues the due screenshots from a pass that drew the
    /// layout. A discarded pass is a layout nobody sees, so it neither counts nor issues.
    pub(crate) fn control_ui(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(mut channel) = self.control.take() else { return };
        let host = WindowHost::of(ctx);
        let document = self.capture_document();
        channel.captures.note_pass();
        let finished = channel.captures.issue(&host, now(), document);
        channel.finish_captures(finished, &mut |path, bytes| control::write_automation_file(path, bytes));
        self.control = Some(channel);
    }
}
