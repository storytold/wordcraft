//! Motion in the ribbon's tab strip, so it doesn't jump from one look to the next.
//!
//! - **Hover:** a tab's hover fill fades in as the pointer arrives and out as it leaves, over
//!   egui's `animation_time` ([`hover`]).
//! - **Another tab:** the active tab's underline glides from the tab it was under to the new one
//!   in [`GLIDE_SECS`], changing width as it goes, fast at first and settling
//!   ([`crate::caret_glide::ease`]) ([`underline`]).
//!
//! What never glides: the underline of a tab that stays active while the strip moves under it (a
//! resized window, a contextual tab coming or going), going to File and back to the same tab, and
//! the first underline after the active tab was in the » menu. Our choice; Word wasn't measured.
//!
//! One rare glide is left in: Read Mode, Full Screen View and Focus hide the strip, but keytips
//! still work there, so Alt and a letter can choose another tab unseen. On leaving the mode, the
//! underline glides to that tab from where it last was. It's cosmetic and over in [`GLIDE_SECS`].
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

/// Each window has its own: they all draw in the same frame and share the context's memory, and
/// another window's tab must not read as a change of tab.
fn glide_id(ctx: &egui::Context) -> Id {
    Id::unique(("ribbon_tab_underline_glide", ctx.viewport_id()))
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
    let id = glide_id(ctx);
    let Some((tab, target)) = active else {
        ctx.data_mut(|d| d.remove::<Glide>(id));
        return None;
    };
    let now = ctx.input(|i| i.time);
    let secs = if ui.style().animation_time > 0.0 { GLIDE_SECS } else { 0.0 };
    let glide = match ctx.data(|d| d.get_temp::<Glide>(id)) {
        // The same tab in the same place: any glide carries on.
        Some(g) if g.tab == tab && g.to == target => g,
        // Another tab: glide from where the underline is now.
        Some(g) if g.tab != tab && secs > 0.0 => Glide { tab: tab.to_string(), from: g.at(now, secs), to: target, start: now },
        // The first frame, animations off, or the strip moved under the tab: there at once.
        _ => Glide { tab: tab.to_string(), from: target, to: target, start: now },
    };
    let at = glide.at(now, secs);
    if glide.progress(now, secs) < 1.0 {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, glide));
    Some(at)
}

/// Whether the stored underline glide is for `tab` and set off from another place (for tests).
#[cfg(test)]
pub(crate) fn glided(ctx: &egui::Context, tab: &str) -> bool {
    ctx.data(|d| d.get_temp::<Glide>(glide_id(ctx))).is_some_and(|g| g.tab == tab && g.from != g.to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};
    use egui_kittest::kittest::Queryable;

    /// The underline as painted in the last frame: the one accent bar of its height.
    fn painted_underline(h: &egui_kittest::Harness<'_, crate::WordApp>) -> Rect {
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        let bars: Vec<Rect> = h
            .output()
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == accent && (r.rect.height() - 3.0).abs() < 0.01 => Some(r.rect),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 1, "one underline: {bars:?}");
        bars[0]
    }

    /// A click on a tab glides the underline there from the active tab whichever side it is on,
    /// as painted: from under the old tab, through the space between, to under the one clicked.
    /// A click on a tab left of the active one used to leave no tab active for the frame of the
    /// click (the tabs after it saw the new tab), so the underline appeared under it at once.
    #[test]
    fn a_click_glides_the_underline_left_and_right() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.ui.animations = crate::theme::Animations::On;
        app.ui.tab = "View".into();
        // 60 frames a second: the harness's default step is longer than a whole glide.
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
            let start = painted_underline(&h);
            let clicked = h.get_by_label(to).rect();
            h.get_by_label(to).click();
            h.run_steps(2);
            assert_eq!(h.state().ui.tab, to);
            assert!(glided(&h.ctx, to), "{from} to {to} glides");
            h.run_steps(2);
            let mid = painted_underline(&h);
            h.run_steps(20);
            let end = painted_underline(&h);
            assert!((end.center().x - clicked.center().x).abs() < 1.0, "{from} to {to} ends under the tab: {end:?}, {clicked:?}");
            let (low, high) = (start.center().x.min(end.center().x), start.center().x.max(end.center().x));
            assert!(low + 1.0 < mid.center().x && mid.center().x < high - 1.0, "{from} to {to} passes between: {start:?}, {mid:?}, {end:?}");
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

    /// The active tab moving before its glide has arrived (the strip changed under it) ends the
    /// glide: the underline is under the tab at once, not somewhere between the tab's old place
    /// and its new one. Until then, the glide carries on.
    #[test]
    fn underline_goes_straight_to_a_tab_that_moves_during_its_glide() {
        let moved = Rect::from_min_size(pos2(200.0, 27.0), vec2(40.0, 3.0));
        let ctx = egui::Context::default();
        frame(&ctx, 0.0, true, |ui| underline(ui, Some(("Home", home()))));
        frame(&ctx, 1.0, true, |ui| underline(ui, Some(("Insert", insert()))));
        let mid = frame(&ctx, 1.0 + GLIDE_SECS / 4.0, true, |ui| underline(ui, Some(("Insert", insert())))).unwrap();
        assert!(mid.min.x > home().min.x && mid.min.x < insert().min.x, "an unmoved tab keeps gliding: {mid:?}");
        let now = 1.0 + GLIDE_SECS / 2.0;
        assert_eq!(frame(&ctx, now, true, |ui| underline(ui, Some(("Insert", moved)))), Some(moved), "under the tab at once");
        assert!(!glided(&ctx, "Insert"), "the glide is over");
        assert_eq!(frame(&ctx, now + 0.01, true, |ui| underline(ui, Some(("Insert", moved)))), Some(moved), "and stays there");
    }

    /// Every window draws in the same frame and shares the context's memory: two windows on
    /// different tabs keep their own underline, neither gliding towards the other's for ever.
    #[test]
    fn each_window_keeps_its_own_underline() {
        let time = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        crate::theme::apply_animations(&ctx, true);
        let clock = time.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| {
            let mut input = egui::RawInput { viewport_id: viewport.ids.this, time: Some(clock.get()), ..Default::default() };
            input.viewports.entry(viewport.ids.this).or_default().parent = Some(viewport.ids.parent);
            let mut cb = viewport.viewport_ui_cb;
            ctx.run_ui(input, |ui| cb(ui)).drop_without_applying_deltas();
        });
        let second = egui::ViewportId::from_hash_of("second");
        let mut last = None;
        for n in 0..30 {
            time.set(f64::from(n) / 60.0);
            let input = egui::RawInput { time: Some(time.get()), ..Default::default() };
            ctx.run_ui(input, |ui| {
                let root = underline(ui, Some(("Home", home())));
                let other =
                    ui.ctx().show_viewport_immediate(second, egui::ViewportBuilder::default(), |ui, _| underline(ui, Some(("Insert", insert()))));
                last = Some((root, other));
            })
            .drop_without_applying_deltas();
        }
        let (root, other) = last.unwrap();
        assert_eq!(root, Some(home()), "the first window");
        assert_eq!(other, Some(insert()), "the second window");
    }

    /// The hover fill fades all the way in and all the way out, step by step, in about egui's
    /// `animation_time` each way. With animations off it comes and goes at once.
    #[test]
    fn hover_fades_in_and_out_unless_animations_are_off() {
        const STEP: f64 = 1.0 / 60.0;
        let fill = Color32::from_rgb(200, 200, 200);
        let id = Id::unique("tab");
        let alpha = |c: Option<Color32>| c.map_or(0, |c| c.a());
        let ctx = egui::Context::default();
        let mut time = 0.0;
        assert_eq!(frame(&ctx, time, true, |ui| hover(ui, id, false, fill)), None, "not hovered");
        let most = (f64::from(ctx.global_style().animation_time) / STEP).ceil() as usize + 2;
        for (on, end) in [(true, fill.a()), (false, 0)] {
            let mut last = alpha(if on { None } else { Some(fill) });
            let mut frames = 0;
            while last != end {
                time += STEP;
                frames += 1;
                assert!(frames <= most, "on={on}: over {most} frames, alpha {last}");
                let now = alpha(frame(&ctx, time, true, |ui| hover(ui, id, on, fill)));
                assert!(if on { now > last } else { now < last }, "on={on}: step by step, {last} then {now}");
                last = now;
            }
            assert!(frames >= 2, "on={on}: a fade, not a jump");
        }
        let ctx = egui::Context::default();
        frame(&ctx, 0.0, false, |ui| hover(ui, id, false, fill));
        assert_eq!(frame(&ctx, 0.01, false, |ui| hover(ui, id, true, fill)), Some(fill), "off: there at once");
        assert_eq!(frame(&ctx, 0.02, false, |ui| hover(ui, id, false, fill)), None, "off: gone at once");
    }
}
