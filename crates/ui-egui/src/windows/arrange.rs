//! View › Window across the app's windows (#322): Arrange All, View Side by Side and Synchronous
//! Scrolling. The engine's `view.*` window commands decide and record what to do
//! (`Session::windows`, as the drawn window sees it) and push a request; [`request`] carries it
//! out by placing the windows' viewports, and the pairing lives here. Without native windows (the
//! web app, embedded viewports) the commands are disabled.
//!
//! Synchronous Scrolling hands scrolling from one window to its partner as document points: the
//! canvas adds up what the window scrolled itself (`CanvasState::sync_out`); [`forward_scroll`]
//! files it for the partner, and [`poll`] gives the partner what is waiting for it
//! (`CanvasState::sync_in`) before it draws.

use std::collections::HashMap;

use egui::{Rect, Vec2, ViewportCommand, ViewportId, pos2, vec2};
use serde_json::Value;
use wordcraft_engine::cmd::view::OtherWindow;

use crate::WordApp;

/// Positions farther from the origin than this, and larger sizes, are refused.
const MAX_COORD: f32 = 32768.0;
/// Larger scroll steps are refused.
const MAX_SCROLL: f32 = 1.0e6;

/// View Side by Side and Synchronous Scrolling for the app's windows.
#[derive(Default)]
pub(crate) struct State {
    /// The two windows (their keys) viewed side by side.
    pair: Option<(u64, u64)>,
    /// They scroll together.
    sync: bool,
    /// Scrolling (document points) the partner did, waiting for the window with this key.
    waiting: HashMap<u64, f32>,
}

impl State {
    /// The window that `key` is viewed beside.
    fn partner(&self, key: u64) -> Option<u64> {
        match self.pair? {
            (a, b) if a == key => Some(b),
            (a, b) if b == key => Some(a),
            _ => None,
        }
    }

    fn clear(&mut self) {
        self.pair = None;
        self.sync = false;
        self.waiting.clear();
    }
}

/// As the drawn window starts its frame: tell the session which other windows there are and
/// whether this one is viewed beside one, and give the window the scrolling its partner did.
pub(crate) fn poll(app: &mut WordApp, _ctx: &egui::Context) {
    let key = app.windows.shown;
    let open: Vec<u64> = app.windows.open.iter().map(|w| w.key).collect();
    let st = &mut app.windows.arranged;
    if st.pair.is_some_and(|(a, b)| !open.contains(&a) || !open.contains(&b)) {
        // One of the two closed.
        st.clear();
    }
    let partner = st.partner(key);
    let sync = partner.is_some() && st.sync;
    let waiting = st.waiting.remove(&key).unwrap_or(0.0);
    let native = app.windows.native && app.windows.driven;
    let others: Vec<OtherWindow> = if native {
        app.switch_windows_entries().into_iter().filter(|(k, ..)| *k != key).map(|(id, title, _)| OtherWindow { id, title }).collect()
    } else {
        Vec::new()
    };
    let w = &mut app.session.windows;
    w.available = native;
    w.others = others;
    w.side_by_side = partner;
    w.sync_scroll = sync;
    if sync && waiting.is_finite() {
        app.canvas.sync_in += waiting.clamp(-MAX_SCROLL, MAX_SCROLL);
    }
}

/// After the drawn window drew: what it scrolled itself goes to its partner.
pub(crate) fn forward_scroll(app: &mut WordApp, ctx: &egui::Context) {
    let key = app.windows.shown;
    let dy = std::mem::take(&mut app.canvas.sync_out);
    let st = &mut app.windows.arranged;
    let Some(partner) = st.partner(key).filter(|_| st.sync && dy.is_finite() && dy != 0.0) else { return };
    let total = st.waiting.entry(partner).or_default();
    *total = (*total + dy).clamp(-MAX_SCROLL, MAX_SCROLL);
    ctx.request_repaint_of(app.windows.viewport(partner));
}

/// Carry out a window request a `view.*` command made (`{"windows": …}`).
pub(crate) fn request(app: &mut WordApp, req: &Value) {
    let Some(what) = req.get("windows").and_then(Value::as_str) else { return };
    let key = app.windows.shown;
    let id = |k: &str| req.get(k).and_then(Value::as_u64);
    match what {
        "sideBySide" => {
            if id("off").is_some() {
                app.windows.arranged.clear();
            }
            if let Some(with) = id("with").filter(|w| *w != key && app.windows.get(*w).is_some()) {
                let st = &mut app.windows.arranged;
                st.waiting.clear();
                st.pair = Some((key, with));
                st.sync = true;
                if let Some(area) = monitor_of(app, key) {
                    let [left, right] = halves(area);
                    place(app, key, left);
                    place(app, with, right);
                }
            }
        }
        "syncScroll" => {
            if let Some(on) = req.get("value").and_then(Value::as_bool) {
                app.windows.arranged.sync = on;
            }
        }
        "arrange" => {
            let Some(area) = monitor_of(app, key) else { return };
            let keys: Vec<u64> = app.windows.open.iter().map(|w| w.key).collect();
            for (k, r) in keys.into_iter().zip(tile(app.windows.open.len(), area)) {
                place(app, k, r);
            }
        }
        _ => {}
    }
    if let Some(ctx) = app.ctx.as_ref() {
        ctx.request_repaint();
    }
}

/// The monitor window `key` is on.
fn monitor_of(app: &WordApp, key: u64) -> Option<Rect> {
    if !app.windows.native || !app.windows.driven {
        return None;
    }
    let ctx = app.ctx.as_ref()?;
    ctx.input_for(app.windows.viewport(key), |i| monitor_area(i.viewport()))
}

/// Move window `key`'s frame to `r`: un-maximized, its content `r` less the frame's borders, and
/// allowed below the usual minimum size so two halves fit a small screen. Platforms that don't
/// let apps place windows (Wayland) ignore the position.
fn place(app: &WordApp, key: u64, r: Rect) {
    let Some(ctx) = app.ctx.as_ref() else { return };
    let viewport: ViewportId = app.windows.viewport(key);
    let border = ctx.input_for(viewport, |i| {
        let v = i.viewport();
        match (v.outer_rect, v.inner_rect) {
            (Some(o), Some(inner)) => (o.size() - inner.size()).max(Vec2::ZERO),
            _ => Vec2::ZERO,
        }
    });
    let inner = (r.size() - border).max(vec2(200.0, 150.0));
    for cmd in [
        ViewportCommand::Fullscreen(false),
        ViewportCommand::Minimized(false),
        ViewportCommand::Maximized(false),
        ViewportCommand::MinInnerSize(inner.min(crate::window_geometry::MIN_SIZE)),
        ViewportCommand::InnerSize(inner),
        ViewportCommand::OuterPosition(r.min),
    ] {
        ctx.send_viewport_cmd_to(viewport, cmd);
    }
}

/// The monitor a window is on, in points. egui reports the monitor's size but not where it
/// is, so this assumes monitors of that size laid out in a grid from the primary monitor at the
/// origin, and picks the cell under the window's centre; with monitors of different sizes it can
/// pick the wrong one.
pub(crate) fn monitor_area(v: &egui::ViewportInfo) -> Option<Rect> {
    let size = v.monitor_size.filter(|s| s.is_finite() && s.x >= 1.0 && s.y >= 1.0)?;
    let centre = v.outer_rect.or(v.inner_rect).map(|r| r.center()).filter(|c| c.is_finite()).unwrap_or(egui::Pos2::ZERO);
    let cell = |c: f32, s: f32| ((c / s).floor() * s).clamp(-MAX_COORD, MAX_COORD);
    Some(Rect::from_min_size(pos2(cell(centre.x, size.x), cell(centre.y, size.y)), size))
}

/// Arrange All: `n` windows tiled over `area`. One or two windows stack top to bottom, like
/// Word; more fill a grid of ⌈√n⌉ columns, the last row's windows sharing its full width.
pub(crate) fn tile(n: usize, area: Rect) -> Vec<Rect> {
    let n = n.min(64);
    if n == 0 {
        return Vec::new();
    }
    let cols = if n <= 2 { 1 } else { (n as f32).sqrt().ceil() as usize };
    let rows = n.div_ceil(cols);
    let h = area.height() / rows as f32;
    (0..n)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            let in_row = if row + 1 == rows { n - row * cols } else { cols };
            let w = area.width() / in_row as f32;
            Rect::from_min_size(pos2(area.min.x + col as f32 * w, area.min.y + row as f32 * h), vec2(w, h))
        })
        .collect()
}

/// View Side by Side: the left and right halves of `area`.
pub(crate) fn halves(area: Rect) -> [Rect; 2] {
    let mid = area.center().x;
    [Rect::from_min_max(area.min, pos2(mid, area.max.y)), Rect::from_min_max(pos2(mid, area.min.y), area.max)]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Services, WordApp};

    #[test]
    fn tiles_cover_the_monitor_without_overlap() {
        let area = Rect::from_min_size(pos2(1920.0, 0.0), vec2(1920.0, 1080.0));
        assert!(tile(0, area).is_empty());
        assert_eq!(tile(1, area), vec![area]);
        // Two windows stack, like Word's Arrange All.
        assert_eq!(
            tile(2, area),
            vec![Rect::from_min_size(pos2(1920.0, 0.0), vec2(1920.0, 540.0)), Rect::from_min_size(pos2(1920.0, 540.0), vec2(1920.0, 540.0))]
        );
        for n in 3..=10 {
            let tiles = tile(n, area);
            assert_eq!(tiles.len(), n);
            let covered: f32 = tiles.iter().map(|r| r.area()).sum();
            assert!((covered - area.area()).abs() < 1.0, "{n} windows cover the monitor");
            for (i, a) in tiles.iter().enumerate() {
                assert!(area.expand(0.01).contains_rect(*a), "{n}: {a:?}");
                for b in &tiles[i + 1..] {
                    assert!(a.intersect(*b).area() < 0.01, "{n}: {a:?} overlaps {b:?}");
                }
            }
        }
        assert_eq!(tile(10_000, area).len(), 64, "hostile counts are capped");
        let [l, r] = halves(area);
        assert_eq!(
            (l, r),
            (Rect::from_min_size(pos2(1920.0, 0.0), vec2(960.0, 1080.0)), Rect::from_min_size(pos2(2880.0, 0.0), vec2(960.0, 1080.0)))
        );
        // The monitor is the one under the window's centre.
        let info = |x: f32| egui::ViewportInfo {
            monitor_size: Some(vec2(1920.0, 1080.0)),
            outer_rect: Some(Rect::from_min_size(pos2(x, 100.0), vec2(800.0, 600.0))),
            ..Default::default()
        };
        assert_eq!(monitor_area(&info(100.0)), Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0))));
        assert_eq!(monitor_area(&info(2000.0)), Some(area));
        assert_eq!(monitor_area(&egui::ViewportInfo::default()), None);
        assert_eq!(monitor_area(&egui::ViewportInfo { monitor_size: Some(vec2(f32::NAN, 1.0)), ..Default::default() }), None);
    }

    #[test]
    fn side_by_side_scrolls_with_its_partner_only() {
        let ctx = egui::Context::default();
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        app.ctx = Some(ctx.clone());
        app.new_window();
        let keys: Vec<u64> = app.windows.open.iter().map(|w| w.key).collect();
        assert_eq!(keys.len(), 2);
        let (first, second) = (keys[0], keys[1]);
        // Headless: no native viewports, so the commands stay disabled.
        poll(&mut app, &ctx);
        assert!(!app.session.windows.available);
        assert!(app.run("view.sideBySide", json!({})).is_err());
        // Pair the two windows as the request would, then scroll the drawn one.
        let shown = app.windows.shown;
        let other = if shown == first { second } else { first };
        app.windows.arranged.pair = Some((shown, other));
        app.windows.arranged.sync = true;
        poll(&mut app, &ctx);
        assert_eq!((app.session.windows.side_by_side, app.session.windows.sync_scroll), (Some(other), true));
        app.canvas.sync_out = 40.0;
        forward_scroll(&mut app, &ctx);
        assert_eq!(app.canvas.sync_out, 0.0);
        assert_eq!(app.windows.arranged.waiting.get(&other), Some(&40.0));
        assert!(!app.windows.arranged.waiting.contains_key(&shown));
        // Nothing is passed on once Synchronous Scrolling is off.
        app.windows.arranged.sync = false;
        app.canvas.sync_out = 5.0;
        forward_scroll(&mut app, &ctx);
        assert_eq!(app.windows.arranged.waiting.get(&other), Some(&40.0));
        // Scrolling that is out of range is clamped, not trusted.
        app.windows.arranged.sync = true;
        app.canvas.sync_out = 1.0e30;
        forward_scroll(&mut app, &ctx);
        assert_eq!(app.windows.arranged.waiting.get(&other), Some(&MAX_SCROLL));
        // The partner closing ends the pairing.
        app.windows.open.retain(|w| w.key == shown);
        poll(&mut app, &ctx);
        assert_eq!((app.session.windows.side_by_side, app.session.windows.sync_scroll), (None, false));
    }
}
