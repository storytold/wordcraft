//! Motion in the ribbon's tab strip, so it doesn't jump from one look to the next.
//!
//! - **Hover:** a tab's hover fill fades in as the pointer arrives and out as it leaves, over
//!   egui's `animation_time` ([`hover`]).
//! - **Another tab:** the active tab's underline glides from the tab it was under to the new one
//!   in [`GLIDE_SECS`], changing width as it goes, fast at first and settling
//!   ([`crate::caret_glide::ease`]) ([`underline`]).
//!
//! What never glides: the underline of a tab that stays active while the strip moves under it (a
//! resized window, a contextual tab coming or going), and an underline that wasn't drawn the frame
//! before (coming back from File, or the active tab in the » menu). Our choice; Word wasn't
//! measured.
//!
//! With animations off (File ▸ Options ▸ Accessibility, or the system's setting: egui's
//! `animation_time` at 0, as in the headless test harness) both happen at once.

use egui::{Color32, Id, Rect, Ui};

/// How long the underline takes to glide to another tab (seconds).
pub const GLIDE_SECS: f64 = 0.15;

/// The underline's glide: which tab it is under, where it set off from and where it is going.
#[derive(Clone, Debug, PartialEq)]
struct Glide {
    tab: String,
    from: Rect,
    to: Rect,
    start: f64,
}

impl Glide {
    /// Where the underline is drawn at `now`.
    fn at(&self, now: f64, secs: f64) -> Rect {
        let e = crate::caret_glide::ease(self.progress(now, secs));
        Rect::from_min_max(self.from.min.lerp(self.to.min, e), self.from.max.lerp(self.to.max, e))
    }

    /// How far along the glide is at `now`, from 0 to 1.
    fn progress(&self, now: f64, secs: f64) -> f32 {
        if secs <= 0.0 || self.from == self.to {
            return 1.0;
        }
        let s = (now - self.start) / secs;
        if s.is_finite() { s.clamp(0.0, 1.0) as f32 } else { 1.0 }
    }
}

fn glide_id() -> Id {
    Id::unique("ribbon_tab_underline_glide")
}

/// The hover fill for a tab this frame: `fill` faded in as far as the pointer has been over it
/// (`on`), or `None` when it has faded out entirely.
pub fn hover(ui: &Ui, id: Id, on: bool, fill: Color32) -> Option<Color32> {
    let f = ui.ctx().animate_bool(id.with("ribbon_tab_hover"), on);
    (f > 0.0).then(|| fill.gamma_multiply(f))
}

/// Where to draw the active tab's underline this frame. `active` is the active tab and where its
/// underline belongs, or `None` when no tab in the strip is active (File, or the active tab in the
/// » menu), and then nothing is drawn.
pub fn underline(ui: &Ui, active: Option<(&str, Rect)>) -> Option<Rect> {
    let ctx = ui.ctx();
    let id = glide_id();
    let Some((tab, target)) = active else {
        ctx.data_mut(|d| d.remove::<Glide>(id));
        return None;
    };
    let now = ctx.input(|i| i.time);
    let secs = if ui.style().animation_time > 0.0 { GLIDE_SECS } else { 0.0 };
    let glide = match ctx.data(|d| d.get_temp::<Glide>(id)) {
        // The same tab: it follows the strip at once, finishing any glide on its way there.
        Some(g) if g.tab == tab => Glide { to: target, ..g },
        // Another tab: glide from where the underline is now.
        Some(g) if secs > 0.0 => Glide { tab: tab.to_string(), from: g.at(now, secs), to: target, start: now },
        _ => Glide { tab: tab.to_string(), from: target, to: target, start: now },
    };
    let at = glide.at(now, secs);
    if glide.progress(now, secs) < 1.0 {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, glide));
    Some(at)
}

/// Whether the underline set off from another tab the last time it was drawn (for tests).
#[cfg(test)]
pub(crate) fn glided(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Glide>(glide_id())).is_some_and(|g| g.from != g.to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};
    use egui_kittest::kittest::Queryable;

    /// A click on a tab glides the underline there from the active tab whichever side it is on.
    /// A click on a tab left of the active one used to leave no tab active for the frame of the
    /// click (the tabs after it saw the new tab), so the underline appeared under it at once.
    #[test]
    fn a_click_glides_the_underline_left_and_right() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.ui.animations = crate::theme::Animations::On;
        app.ui.tab = "View".into();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut crate::WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        for (from, to) in [("View", "Home"), ("Home", "View")] {
            assert_eq!(h.state().ui.tab, from);
            h.get_by_label(to).click();
            h.run_steps(2);
            assert_eq!(h.state().ui.tab, to);
            assert!(glided(&h.ctx), "{from} to {to} glides");
            h.run_steps(20);
        }
    }

    /// Runs one frame at `time` with animations `on` and returns what `f` gives back.
    fn frame<R>(ctx: &egui::Context, time: f64, on: bool, mut f: impl FnMut(&Ui) -> R) -> R {
        crate::theme::apply_animations(ctx, on);
        let mut out = None;
        let input = egui::RawInput { time: Some(time), ..Default::default() };
        ctx.run_ui(input, |ui| out = Some(f(ui))).drop_without_applying_deltas();
        out.unwrap()
    }

    fn home() -> Rect {
        Rect::from_min_size(pos2(60.0, 27.0), vec2(30.0, 3.0))
    }

    fn insert() -> Rect {
        Rect::from_min_size(pos2(120.0, 27.0), vec2(40.0, 3.0))
    }

    /// Switching tabs glides the underline from the old tab to the new one in `GLIDE_SECS`,
    /// changing its width on the way.
    #[test]
    fn underline_glides_to_another_tab() {
        let ctx = egui::Context::default();
        assert_eq!(frame(&ctx, 0.0, true, |ui| underline(ui, Some(("Home", home())))), Some(home()), "first drawn in place");
        let start = frame(&ctx, 1.0, true, |ui| underline(ui, Some(("Insert", insert())))).unwrap();
        assert_eq!(start, home(), "sets off from Home");
        let mid = frame(&ctx, 1.0 + GLIDE_SECS / 2.0, true, |ui| underline(ui, Some(("Insert", insert())))).unwrap();
        assert!(mid.min.x > home().min.x && mid.min.x < insert().min.x, "on its way: {mid:?}");
        assert!(mid.width() > home().width() && mid.width() < insert().width(), "widening: {mid:?}");
        let end = frame(&ctx, 1.0 + GLIDE_SECS, true, |ui| underline(ui, Some(("Insert", insert())))).unwrap();
        assert_eq!(end, insert(), "arrived");
    }

    /// With animations off the underline is under the new tab in the frame the tab is chosen.
    #[test]
    fn underline_jumps_with_animations_off() {
        let ctx = egui::Context::default();
        frame(&ctx, 0.0, false, |ui| underline(ui, Some(("Home", home()))));
        assert_eq!(frame(&ctx, 1.0, false, |ui| underline(ui, Some(("Insert", insert())))), Some(insert()));
    }

    /// The same tab moving with the strip (a resized window) doesn't glide, and an underline that
    /// wasn't drawn (File) comes back in place.
    #[test]
    fn underline_doesnt_glide_without_a_tab_change() {
        let ctx = egui::Context::default();
        frame(&ctx, 0.0, true, |ui| underline(ui, Some(("Home", home()))));
        assert_eq!(frame(&ctx, 1.0, true, |ui| underline(ui, Some(("Home", insert())))), Some(insert()), "moved with the strip");
        assert_eq!(frame(&ctx, 2.0, true, |ui| underline(ui, None)), None, "File: no underline");
        assert_eq!(frame(&ctx, 3.0, true, |ui| underline(ui, Some(("Home", home())))), Some(home()), "back in place");
    }

    /// The hover fill fades in with animations on and is all there at once with them off.
    #[test]
    fn hover_fades_in_unless_animations_are_off() {
        let fill = Color32::from_rgb(200, 200, 200);
        let id = Id::unique("tab");
        let ctx = egui::Context::default();
        assert_eq!(frame(&ctx, 0.0, true, |ui| hover(ui, id, false, fill)), None, "not hovered");
        let early = frame(&ctx, 0.01, true, |ui| hover(ui, id, true, fill)).unwrap();
        assert!(early.a() < fill.a(), "fading in: {early:?}");
        let ctx = egui::Context::default();
        frame(&ctx, 0.0, false, |ui| hover(ui, id, false, fill));
        assert_eq!(frame(&ctx, 0.01, false, |ui| hover(ui, id, true, fill)), Some(fill));
    }
}
