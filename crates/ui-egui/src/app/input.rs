use super::WordApp;
use craft::ui::runtime::env::ShellRegions;

impl WordApp {
    /// A click on the window's chrome (ribbon tabs and empty ribbon space, galleries, check
    /// boxes, the status bar, panes) hands keyboard focus back to the document, as in Word, so the
    /// caret keeps blinking and typing continues where it was (#1093). egui drops the page's focus
    /// on any click elsewhere; only a click that left nothing focused counts, so a text field
    /// (font size, zoom, command search) keeps the focus it took, and clicks on a dialog, menu or
    /// popup, or while one is open, leave focus alone. Clicks on the page are the canvas's own,
    /// except those on a shell slot drawn over it (the ribbon dropped down in Focus): `regions`
    /// are this pass's slot rects.
    pub(super) fn refocus_after_chrome_click(&mut self, ctx: &egui::Context, regions: &ShellRegions) {
        if self.ui.backstage || !self.dialog_host.is_empty() {
            return;
        }
        let Some(pos) = ctx.input(|i| if i.pointer.any_click() { i.pointer.interact_pos() } else { None }) else { return };
        // The page handles its own clicks (comment balloons there take the keyboard themselves).
        let on_slot = regions.slots.iter().any(|(_, rect)| rect.contains(pos));
        if self.canvas.selected_balloon().is_some() || (!on_slot && self.canvas.contains(pos)) {
            return;
        }
        let on_chrome = ctx.layer_id_at(pos).is_none_or(|l| l.order == egui::Order::Background);
        if on_chrome && ctx.memory(|m| m.focused().is_none()) && !egui::Popup::is_any_open(ctx) {
            // The canvas claims the keyboard now, so the click doesn't swallow the first keystroke.
            self.canvas.focus_page(ctx);
            ctx.request_repaint();
        }
    }
}
