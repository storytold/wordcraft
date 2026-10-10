//! View › Window across WordCraft windows (#322): Switch Windows, Arrange All, View Side by Side
//! and Synchronous Scrolling.
//!
//! Every desktop window is its own process with one document. The host (the desktop app) gives
//! the UI a [`WindowHost`]: the other running windows (a registry in the settings folder) and a
//! small authenticated channel to send them a [`PeerMessage`]. The engine's `view.*` window
//! commands only decide and record what to do (`Session::windows`); [`request`] carries it out
//! and [`poll`] applies what other windows ask of this one. Without a host (web, tests) the
//! commands are disabled.

use egui::{Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wordcraft_engine::cmd::view::OtherWindow;

use crate::WordApp;

/// What one window asks of another. Coordinates and sizes are egui points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PeerMessage {
    /// Are you there? (The registry's liveness check; never reaches the UI.)
    Ping,
    /// Come to the front.
    Focus,
    /// Move and resize the window's frame to this rect.
    Place { x: f32, y: f32, width: f32, height: f32 },
    /// View Side by Side with the sender turned on or off.
    SideBySide { on: bool },
    /// The sender turned Synchronous Scrolling on or off.
    SyncScroll { on: bool },
    /// The sender scrolled by this many document points (positive: down).
    Scroll { dy: f32 },
}

/// Positions farther from the origin than this, and larger sizes, are refused.
const MAX_COORD: f32 = 32768.0;
/// Larger scroll steps are refused.
const MAX_SCROLL: f32 = 1.0e6;

impl PeerMessage {
    /// The message if its numbers are usable: finite, positions within ±32768 points, sizes
    /// positive and at most 32768, scroll steps at most 10⁶ points. Messages come from other
    /// processes, so they are checked like any input.
    pub fn sanitized(self) -> Option<Self> {
        let ok = match &self {
            PeerMessage::Place { x, y, width, height } => {
                [*x, *y].iter().all(|c| c.is_finite() && c.abs() <= MAX_COORD)
                    && [*width, *height].iter().all(|c| c.is_finite() && *c >= 1.0 && *c <= MAX_COORD)
            }
            PeerMessage::Scroll { dy } => dy.is_finite() && dy.abs() <= MAX_SCROLL,
            _ => true,
        };
        ok.then_some(self)
    }
}

/// A message and the window it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    pub from: u64,
    pub msg: PeerMessage,
}

/// The host's side of View › Window: the other windows and a channel to them. Nothing here
/// blocks the UI thread: the list is refreshed and messages are sent in the background.
pub trait WindowHost {
    /// This window's id in the registry.
    fn id(&self) -> u64;
    /// The other open windows as last seen, in a stable order.
    fn others(&self) -> Vec<OtherWindow>;
    /// Send `msg` to window `to` (queued; a window that has gone away is skipped).
    fn send(&self, to: u64, msg: PeerMessage);
    /// This window's title, for the other windows' Switch Windows lists.
    fn publish(&self, title: &str);
    /// Messages other windows sent since the last call.
    fn incoming(&self) -> Vec<Envelope>;
}

/// Each frame: keep `Session::windows` current, publish the title and apply messages from other
/// windows.
pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    let Some(host) = app.services.windows.as_ref() else { return };
    let others = host.others();
    let title = app.title_stem();
    host.publish(&title);
    let incoming = host.incoming();
    let w = &mut app.session.windows;
    w.available = true;
    w.others = others;
    if let Some(p) = w.side_by_side
        && !w.others.iter().any(|o| o.id == p)
    {
        // The other window closed.
        w.side_by_side = None;
        w.sync_scroll = false;
    }
    for env in incoming {
        apply(app, ctx, env);
    }
    // This window scrolled (the canvas adds it up): the partner follows.
    let dy = std::mem::take(&mut app.canvas.sync_out);
    if let (Some(p), true) = (app.session.windows.side_by_side, app.session.windows.sync_scroll)
        && dy != 0.0
        && let Some(host) = app.services.windows.as_ref()
    {
        host.send(p, PeerMessage::Scroll { dy });
    }
}

/// What another window asked of this one.
fn apply(app: &mut WordApp, ctx: &egui::Context, env: Envelope) {
    let w = &mut app.session.windows;
    let partner = w.side_by_side == Some(env.from);
    match env.msg {
        PeerMessage::Ping => {}
        PeerMessage::Focus => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            app.canvas.want_focus = true;
        }
        PeerMessage::Place { x, y, width, height } => place(ctx, Rect::from_min_size(pos2(x, y), vec2(width, height))),
        PeerMessage::SideBySide { on: true } => {
            // A new partner replaces the old one, which is told.
            if let Some(old) = w.side_by_side.filter(|old| *old != env.from)
                && let Some(host) = app.services.windows.as_ref()
            {
                host.send(old, PeerMessage::SideBySide { on: false });
            }
            w.side_by_side = Some(env.from);
            w.sync_scroll = true;
        }
        PeerMessage::SideBySide { on: false } if partner => {
            w.side_by_side = None;
            w.sync_scroll = false;
        }
        PeerMessage::SyncScroll { on } if partner => w.sync_scroll = on,
        PeerMessage::Scroll { dy } if partner && w.sync_scroll => {
            app.canvas.sync_in += dy;
            ctx.request_repaint();
        }
        _ => {}
    }
    ctx.request_repaint();
}

/// Carry out a window request a `view.*` command made (`{"windows": …}`).
pub fn request(app: &mut WordApp, req: &Value) {
    let Some(what) = req.get("windows").and_then(Value::as_str) else { return };
    let (Some(host), Some(ctx)) = (app.services.windows.as_ref(), app.ctx.as_ref()) else { return };
    let id = |k: &str| req.get(k).and_then(Value::as_u64);
    let area = ctx.input(|i| monitor_area(i.viewport()));
    match what {
        "focus" => {
            if let Some(to) = id("id") {
                host.send(to, PeerMessage::Focus);
            }
        }
        "arrange" => {
            let Some(area) = area else { return };
            let mut ids: Vec<u64> = app.session.windows.others.iter().map(|o| o.id).chain([host.id()]).collect();
            ids.sort_unstable();
            for (id, r) in ids.iter().zip(tile(ids.len(), area)) {
                if *id == host.id() {
                    place(ctx, r);
                } else {
                    host.send(*id, place_msg(r));
                }
            }
        }
        "sideBySide" => {
            if let Some(off) = id("off") {
                host.send(off, PeerMessage::SideBySide { on: false });
            }
            if let Some(with) = id("with") {
                if let Some(area) = area {
                    let [left, right] = halves(area);
                    place(ctx, left);
                    host.send(with, place_msg(right));
                }
                host.send(with, PeerMessage::SideBySide { on: true });
            }
        }
        "syncScroll" => {
            if let (Some(with), Some(on)) = (id("with"), req.get("value").and_then(Value::as_bool)) {
                host.send(with, PeerMessage::SyncScroll { on });
            }
        }
        _ => {}
    }
}

fn place_msg(r: Rect) -> PeerMessage {
    PeerMessage::Place { x: r.min.x, y: r.min.y, width: r.width(), height: r.height() }
}

/// Move this window's frame to `r`: un-maximized, its content `r` less the frame's borders, and
/// allowed below the usual minimum size so two halves fit a small screen. Platforms that don't
/// let apps place windows (Wayland) ignore the position.
fn place(ctx: &egui::Context, r: Rect) {
    let border = ctx.input(|i| {
        let v = i.viewport();
        match (v.outer_rect, v.inner_rect) {
            (Some(o), Some(inner)) => (o.size() - inner.size()).max(Vec2::ZERO),
            _ => Vec2::ZERO,
        }
    });
    let inner = (r.size() - border).max(vec2(200.0, 150.0));
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(inner.min(crate::window_geometry::MIN_SIZE)));
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(inner));
    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(r.min));
}

/// The monitor this window is on, in points. egui reports the monitor's size but not where it
/// is, so this assumes monitors of that size laid out in a grid from the primary monitor at the
/// origin, and picks the cell under the window's centre.
pub fn monitor_area(v: &egui::ViewportInfo) -> Option<Rect> {
    let size = v.monitor_size.filter(|s| s.is_finite() && s.x >= 1.0 && s.y >= 1.0)?;
    let centre = v.outer_rect.or(v.inner_rect).map(|r| r.center()).filter(|c| c.is_finite()).unwrap_or(egui::Pos2::ZERO);
    let cell = |c: f32, s: f32| ((c / s).floor() * s).clamp(-MAX_COORD, MAX_COORD);
    Some(Rect::from_min_size(pos2(cell(centre.x, size.x), cell(centre.y, size.y)), size))
}

/// Arrange All: `n` windows tiled over `area`. One or two windows stack top to bottom, like
/// Word; more fill a grid of ⌈√n⌉ columns, the last row's windows sharing its full width.
pub fn tile(n: usize, area: Rect) -> Vec<Rect> {
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
pub fn halves(area: Rect) -> [Rect; 2] {
    let mid = area.center().x;
    [Rect::from_min_max(area.min, pos2(mid, area.max.y)), Rect::from_min_max(pos2(mid, area.min.y), area.max)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;

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
    fn hostile_messages_are_refused() {
        let place = |x: f32, w: f32| PeerMessage::Place { x, y: 0.0, width: w, height: 600.0 }.sanitized();
        assert!(place(10.0, 800.0).is_some());
        assert!(place(f32::NAN, 800.0).is_none());
        assert!(place(1e9, 800.0).is_none());
        assert!(place(0.0, -5.0).is_none());
        assert!(place(0.0, f32::INFINITY).is_none());
        assert!(PeerMessage::Scroll { dy: f32::NAN }.sanitized().is_none());
        assert!(PeerMessage::Scroll { dy: 1e30 }.sanitized().is_none());
        assert_eq!(PeerMessage::Scroll { dy: -12.5 }.sanitized(), Some(PeerMessage::Scroll { dy: -12.5 }));
        let parsed: PeerMessage = serde_json::from_value(json!({"type": "sideBySide", "on": true})).unwrap();
        assert_eq!(parsed, PeerMessage::SideBySide { on: true });
        assert!(serde_json::from_value::<PeerMessage>(json!({"type": "engine.execute"})).is_err());
    }

    /// A host that records what was sent and hands out queued messages.
    #[derive(Clone, Default)]
    struct Fake {
        sent: Rc<RefCell<Vec<(u64, PeerMessage)>>>,
        inbox: Rc<RefCell<Vec<Envelope>>>,
    }

    impl WindowHost for Fake {
        fn id(&self) -> u64 {
            1
        }
        fn others(&self) -> Vec<OtherWindow> {
            vec![OtherWindow { id: 2, title: "Notes".into() }, OtherWindow { id: 3, title: "Draft".into() }]
        }
        fn send(&self, to: u64, msg: PeerMessage) {
            self.sent.borrow_mut().push((to, msg));
        }
        fn publish(&self, _title: &str) {}
        fn incoming(&self) -> Vec<Envelope> {
            self.inbox.borrow_mut().drain(..).collect()
        }
    }

    #[test]
    fn side_by_side_scrolls_with_its_partner_only() {
        let fake = Fake::default();
        let services = crate::Services { windows: Some(Box::new(fake.clone())), ..Default::default() };
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        let ctx = egui::Context::default();
        app.ctx = Some(ctx.clone());
        poll(&mut app, &ctx);
        assert!(app.session.windows.available);
        assert_eq!(app.session.windows.others.len(), 2);
        app.run("view.sideBySide", json!({"window": 3})).unwrap();
        // No monitor size headless: nothing is placed, but the partner hears about it.
        assert_eq!(fake.sent.borrow_mut().drain(..).collect::<Vec<_>>(), [(3, PeerMessage::SideBySide { on: true })]);
        app.run("view.switchWindows", json!({"window": 2})).unwrap();
        assert_eq!(fake.sent.borrow_mut().pop(), Some((2, PeerMessage::Focus)));
        // Scrolls from the partner move this window; anyone else's are ignored.
        fake.inbox
            .borrow_mut()
            .extend([Envelope { from: 3, msg: PeerMessage::Scroll { dy: 40.0 } }, Envelope { from: 2, msg: PeerMessage::Scroll { dy: 1000.0 } }]);
        poll(&mut app, &ctx);
        assert_eq!(app.canvas.sync_in, 40.0);
        // This window's own scrolling goes to the partner.
        app.canvas.sync_out = -15.0;
        poll(&mut app, &ctx);
        assert_eq!(fake.sent.borrow_mut().pop(), Some((3, PeerMessage::Scroll { dy: -15.0 })));
        // Turned off on the partner's side: no more syncing.
        fake.inbox.borrow_mut().push(Envelope { from: 3, msg: PeerMessage::SideBySide { on: false } });
        poll(&mut app, &ctx);
        assert_eq!((app.session.windows.side_by_side, app.session.windows.sync_scroll), (None, false));
        app.canvas.sync_out = 5.0;
        poll(&mut app, &ctx);
        assert!(fake.sent.borrow().is_empty());
        assert!(app.run("view.syncScroll", json!({})).is_err());
    }
}
