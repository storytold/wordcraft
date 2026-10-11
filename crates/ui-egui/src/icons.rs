//! WordCraft's icon set, drawn in code (original artwork; no external icon assets).
//!
//! `paint(painter, rect, name, ink, accent)` draws icon `name`. Every icon must say what its
//! command does **at a glance**: the ribbon is icon-dense, so an icon that needs its label to be
//! understood is a bug. These rules apply to every icon; follow them when adding or changing one.
//!
//! # Grid and drawing
//! - A 16 × 16 unit grid, drawn at 16 px (small buttons, menus, toolbars) or 32 px (large ribbon
//!   buttons). Don't paint at other sizes.
//! - Stroke width comes from [`stroke_px`] (about 1.25 px at 16, 2 px at 32); never set a width
//!   by hand. Bold lettering uses `Pen::heavy`.
//! - Every corner is rounded with `CORNER` (lines, outlines and fills alike); arrow heads, checks
//!   and letter apexes use `TIP`. Hand-drawn strokes and rope use `Pen::curve`, not polylines.
//! - Strokes stay between 1 and 15, so nothing is clipped. Draw only inside the given rect.
//!
//! # Colour
//! - Line work in the ink colour; **one** meaningful detail in the accent (the arrow, the new
//!   item, the changed part); an optional soft tint (`pen.t`) inside the main shape.
//! - Status colours only where the colour *is* the meaning, and only from the theme: green = add,
//!   accept, OK; red = remove, reject, error; orange = warning. Never hard-code colours or use
//!   black-alpha shadows (they vanish in the dark theme).
//! - Colour commands (font colour, highlight, shading) draw their bar in the accent; the split
//!   button passes the current colour as the accent.
//! - Disabled icons pass the same colour as ink and accent; then every colour, the status colours
//!   and the tint included, follows the ink.
//!
//! # Spacing
//! - Each part has one colour, and parts of different colours never touch.
//! - Separate parts keep at least 1.25 units of clear space (2.25 between stroke centre lines).
//! - Shapes never overlap or cross: show depth by a gap or by drawing only the visible part of the
//!   back shape (see `copy`), never by stacking.
//! - Balance the composition in the frame; don't crowd one corner.
//!
//! # Reused elements
//! Anything that appears in more than one icon is drawn by its `Pen` helper at fixed proportions,
//! never redrawn by hand: `page_h` / `page_landscape` (one ratio, `PAGE_RATIO`; text only through
//! `page_slot` / `page_rows`, and only on full-size pages), `bubble`, `window`, `table`,
//! `picture`, `lens`, `pencil`, `lock`, `person` / `person_filled`, `chain`, `brackets`,
//! `scribble`, `glyph_a` / `glyph_small_a`, `plus`, `check`, `cross`, `arrow` / `span` / `head`
//! (`HEAD`) and `cycle`.
//! - If an element is needed twice, add a helper; don't copy coordinates.
//! - Scale a helper, never stretch it. Windows may change aspect with the arrangement shown (side
//!   by side tall, stacked wide) but match within an arrangement.
//! - Free text lines use one pitch for paragraphs (3 units: 3.5, 6.5, 9.5, 12.5) and one for lists
//!   (5 units: 3, 8, 13).
//!
//! # Meaning
//! - Draw what the command does. Letters only where the command is about letters (Bold, Italic,
//!   Aa…); font letters must exist in Inter (`theme::tests::interface_symbols_have_glyphs`), other
//!   scripts are drawn with strokes.
//! - One command, one drawing: only true aliases share an arm ([`ALIASES`]), enforced by
//!   `tests::every_icon_is_distinct`.
//! - Related commands share a base and differ in one clear detail (Find, Zoom, Zoom In, Zoom Out;
//!   Comment, New, Delete, Resolve).
//! - Don't imitate another product's icon compositions or approximate third-party logos.
//!
//! # Adding or changing an icon
//! 1. Add the name to [`NAMES`] and draw it with the helpers, following the rules above.
//! 2. Look at it: at 16 and 32 px, on the light and dark ribbon, next to its neighbours in the same
//!    ribbon group (headless `ui_shot`, then read the PNG).
//! 3. Run `cargo test -p wordcraft-ui-egui` (distinct drawings, no fallback tiles, glyph coverage)
//!    and `cargo xtask ci`.

use egui::{Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

/// Stroke width in pixels for an icon drawn `side` pixels wide: about 1.25 px at 16, 2 px at 32.
pub fn stroke_px(side: f32) -> f32 {
    (0.5 + side * 0.047).clamp(1.1, 2.4)
}

/// Corner radius, in grid units, for every vertex of a line, outline or filled shape: the same
/// rounding as a rectangle's corner, so no icon mixes sharp and soft corners.
const CORNER: f32 = 1.0;
/// Smaller rounding for points that should stay crisp: arrow heads, checks, letter apexes.
const TIP: f32 = 0.4;
/// Arrow head arm length, the same on every arrow.
const HEAD: f32 = 2.0;
/// Width-to-height ratio of every page.
const PAGE_RATIO: f32 = 11.0 / 13.0;

/// Round each corner of a polyline (or polygon when `closed`) with radius `r`, limited to half of
/// the shorter neighbouring segment so short segments stay intact.
fn round_corners(pts: &[(f32, f32)], closed: bool, r: f32) -> Vec<(f32, f32)> {
    let n = pts.len();
    if n < 3 || r <= 0.0 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(n * 6);
    for i in 0..n {
        let interior = closed || (i > 0 && i + 1 < n);
        let p = pts[i];
        if !interior {
            out.push(p);
            continue;
        }
        let a = pts[(i + n - 1) % n];
        let b = pts[(i + 1) % n];
        let (ax, ay) = (a.0 - p.0, a.1 - p.1);
        let (bx, by) = (b.0 - p.0, b.1 - p.1);
        let (la, lb) = ((ax * ax + ay * ay).sqrt(), (bx * bx + by * by).sqrt());
        if la < 1e-4 || lb < 1e-4 {
            out.push(p);
            continue;
        }
        let k = r.min(la * 0.5).min(lb * 0.5);
        let p1 = (p.0 + ax / la * k, p.1 + ay / la * k);
        let p2 = (p.0 + bx / lb * k, p.1 + by / lb * k);
        for j in 0..=4 {
            let t = j as f32 / 4.0;
            let u = 1.0 - t;
            out.push((u * u * p1.0 + 2.0 * u * t * p.0 + t * t * p2.0, u * u * p1.1 + 2.0 * u * t * p.1 + t * t * p2.1));
        }
    }
    out
}

/// The outline of a page `w` × `h` with its top-right corner cut, from (0, 0).
fn page_pts(w: f32, h: f32) -> [(f32, f32); 5] {
    let k = w * 0.3;
    [(0.0, 0.0), (w - k, 0.0), (w, k), (w, h), (0.0, h)]
}

/// A smooth curve through `pts` (Catmull-Rom), for hand-drawn strokes and rope.
fn smooth(pts: &[(f32, f32)], closed: bool) -> Vec<(f32, f32)> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let at = |i: isize| -> (f32, f32) { if closed { pts[i.rem_euclid(n as isize) as usize] } else { pts[i.clamp(0, n as isize - 1) as usize] } };
    let segs = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(segs * 8 + 1);
    for i in 0..segs as isize {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        for j in 0..8 {
            let t = j as f32 / 8.0;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            out.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    if !closed {
        out.push(pts[n - 1]);
    }
    out
}

struct Pen<'a> {
    p: &'a Painter,
    o: Pos2,
    s: f32,
    w: f32,
    c: Color32,
    a: Color32,
    /// Soft fill inside a main shape.
    t: Color32,
}

impl Pen<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.o.x + x * self.s, self.o.y + y * self.s)
    }
    fn pts(&self, pts: &[(f32, f32)]) -> Vec<Pos2> {
        pts.iter().map(|&(x, y)| self.pt(x, y)).collect()
    }
    fn cap(&self, q: Pos2, col: Color32) {
        self.p.circle_filled(q, self.w * 0.5, col);
    }
    /// An open polyline in `col`, with round caps and rounded corners.
    fn path(&self, pts: &[(f32, f32)], col: Color32) {
        self.path_r(pts, col, CORNER);
    }
    /// [`Pen::path`] with its own corner radius.
    fn path_r(&self, pts: &[(f32, f32)], col: Color32, r: f32) {
        let v = self.pts(&round_corners(pts, false, r));
        if v.len() < 2 {
            return;
        }
        for q in &v {
            self.cap(*q, col);
        }
        self.p.add(Shape::line(v, Stroke::new(self.w, col)));
    }
    fn line(&self, pts: &[(f32, f32)]) {
        self.path(pts, self.c);
    }
    fn acc(&self, pts: &[(f32, f32)]) {
        self.path(pts, self.a);
    }
    /// An ink polyline at a heavier weight (bold lettering).
    fn heavy(&self, pts: &[(f32, f32)]) {
        let v = self.pts(pts);
        if v.len() < 2 {
            return;
        }
        let w = self.w * 1.9;
        let v =
            self.pts(&round_corners(&v.iter().map(|q| ((q.x - self.o.x) / self.s, (q.y - self.o.y) / self.s)).collect::<Vec<_>>(), false, CORNER));
        for q in &v {
            self.p.circle_filled(*q, w * 0.5, self.c);
        }
        self.p.add(Shape::line(v, Stroke::new(w, self.c)));
    }
    /// A closed outline in `col`, corners rounded.
    fn poly(&self, pts: &[(f32, f32)], col: Color32) {
        let v = self.pts(&round_corners(pts, true, CORNER));
        for q in &v {
            self.cap(*q, col);
        }
        self.p.add(Shape::closed_line(v, Stroke::new(self.w, col)));
    }
    /// A filled convex shape, corners rounded like the outlines.
    fn fill(&self, pts: &[(f32, f32)], col: Color32) {
        self.p.add(Shape::convex_polygon(self.pts(&round_corners(pts, true, CORNER)), col, Stroke::NONE));
    }
    /// A smooth stroke through `pts`.
    fn curve(&self, pts: &[(f32, f32)], closed: bool, col: Color32) {
        let v = smooth(pts, closed);
        if closed {
            let q = self.pts(&v);
            self.p.add(Shape::closed_line(q, Stroke::new(self.w, col)));
        } else {
            self.path_r(&v, col, 0.0);
        }
    }
    fn radius(&self, r: f32) -> CornerRadius {
        CornerRadius::same((r * self.s).round().clamp(0.0, 255.0) as u8)
    }
    fn frame(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.pt(x0, y0), self.pt(x1, y1))
    }
    /// A rounded-rectangle outline in `col`; `r` in grid units.
    fn rect_c(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, col: Color32) {
        self.p.rect_stroke(self.frame(x0, y0, x1, y1), self.radius(r), Stroke::new(self.w, col), egui::StrokeKind::Middle);
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) {
        self.rect_c(x0, y0, x1, y1, r, self.c);
    }
    /// A filled rounded rectangle.
    fn block(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, col: Color32) {
        self.p.rect_filled(self.frame(x0, y0, x1, y1), self.radius(r), col);
    }
    /// A tinted, outlined rounded rectangle: the usual container.
    fn panel(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) {
        self.block(x0, y0, x1, y1, r, self.t);
        self.rect(x0, y0, x1, y1, r);
    }
    fn circle_c(&self, x: f32, y: f32, r: f32, col: Color32) {
        self.p.circle_stroke(self.pt(x, y), r * self.s, Stroke::new(self.w, col));
    }
    fn circle(&self, x: f32, y: f32, r: f32) {
        self.circle_c(x, y, r, self.c);
    }
    /// A tinted, outlined circle.
    fn disc(&self, x: f32, y: f32, r: f32) {
        self.p.circle_filled(self.pt(x, y), r * self.s, self.t);
        self.circle(x, y, r);
    }
    fn dot(&self, x: f32, y: f32, r: f32, col: Color32) {
        self.p.circle_filled(self.pt(x, y), r * self.s, col);
    }
    /// Points along an arc from `a0` to `a1` degrees (0 = right, 90 = down).
    fn arc_pts(x: f32, y: f32, r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
        let n = (((a1 - a0).abs() / 12.0).ceil() as usize).max(2);
        (0..=n)
            .map(|i| {
                let t = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                (x + r * t.cos(), y + r * t.sin())
            })
            .collect()
    }
    fn arc(&self, x: f32, y: f32, r: f32, a0: f32, a1: f32, col: Color32) {
        self.path(&Self::arc_pts(x, y, r, a0, a1), col);
    }
    /// An open arrow head with its tip at (`x`, `y`) pointing along `deg`.
    fn head(&self, x: f32, y: f32, deg: f32, col: Color32) {
        let t = deg.to_radians();
        let (l, r) = (t + 2.4, t - 2.4);
        self.path_r(&[(x + HEAD * l.cos(), y + HEAD * l.sin()), (x, y), (x + HEAD * r.cos(), y + HEAD * r.sin())], col, TIP);
    }
    /// A straight arrow from (x0, y0) to (x1, y1).
    fn arrow(&self, x0: f32, y0: f32, x1: f32, y1: f32, col: Color32) {
        self.path(&[(x0, y0), (x1, y1)], col);
        let deg = (y1 - y0).atan2(x1 - x0).to_degrees();
        self.head(x1, y1, deg, col);
    }
    /// A double-headed arrow.
    fn span(&self, x0: f32, y0: f32, x1: f32, y1: f32, col: Color32) {
        self.path(&[(x0, y0), (x1, y1)], col);
        let deg = (y1 - y0).atan2(x1 - x0).to_degrees();
        self.head(x1, y1, deg, col);
        self.head(x0, y0, deg + 180.0, col);
    }
    /// A circular arrow (refresh / rotate) around (x, y).
    fn cycle(&self, x: f32, y: f32, r: f32, col: Color32) {
        self.arc(x, y, r, -60.0, 230.0, col);
        let (hx, hy) = (x + r * (-60f32).to_radians().cos(), y + r * (-60f32).to_radians().sin());
        self.head(hx, hy, -60.0 + 90.0 + 180.0 + 20.0, col);
    }
    fn text(&self, x: f32, y: f32, size: f32, t: &str, col: Color32, semibold: bool) {
        let px = size * self.s;
        let font = if semibold { crate::theme::semibold(px) } else { FontId::proportional(px) };
        self.p.text(self.pt(x, y), egui::Align2::CENTER_CENTER, t, font, col);
    }
    /// Text rows from x0 to x1 at each y.
    fn rows(&self, x0: f32, x1: f32, ys: &[f32]) {
        for y in ys {
            self.line(&[(x0, *y), (x1, *y)]);
        }
    }
    /// A capital A drawn as strokes, `h` tall with its baseline at `y` and centred on `x`.
    fn glyph_a(&self, x: f32, y: f32, h: f32, col: Color32, bar: Color32) {
        let w = h * 0.42;
        self.path_r(&[(x - w, y), (x, y - h), (x + w, y)], col, TIP);
        self.path(&[(x - w * 0.55, y - h * 0.38), (x + w * 0.55, y - h * 0.38)], bar);
    }
    /// A small a (bowl and stem) centred at x with baseline y and x-height h.
    fn glyph_small_a(&self, x: f32, y: f32, h: f32, col: Color32) {
        let r = h * 0.5;
        self.circle_c(x - r * 0.3, y - r, r, col);
        self.path(&[(x + r * 0.7 + 0.2, y - h), (x + r * 0.7 + 0.2, y)], col);
    }
    /// A page `h` tall with its top-left corner at (x0, y0), at the one page ratio
    /// ([`PAGE_RATIO`]); returns its right edge.
    fn page_h(&self, x0: f32, y0: f32, h: f32) -> f32 {
        let x1 = x0 + h * PAGE_RATIO;
        self.page_rect(x0, y0, x1, y0 + h);
        x1
    }
    /// A page with a cut top-right corner, tinted. The cut is a fixed share of the width, so every
    /// page has the same proportions.
    fn page_rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.page_outline(&page_pts(x1 - x0, y1 - y0).iter().map(|&(x, y)| (x0 + x, y0 + y)).collect::<Vec<_>>(), x1 - x0);
    }
    /// The page from [`Pen::page_h`] turned 90° clockwise: `h` wide (the portrait height) with
    /// its top-left at (x0, y0), the cut corner now at the bottom right.
    fn page_landscape(&self, x0: f32, y0: f32, h: f32) {
        let w = h * PAGE_RATIO;
        self.page_outline(&page_pts(w, h).iter().map(|&(x, y)| (x0 + h - y, y0 + x)).collect::<Vec<_>>(), w);
    }
    fn page_outline(&self, pts: &[(f32, f32)], w: f32) {
        // A tighter radius than other shapes, so the cut corner still reads on small pages.
        let r = w * 0.09;
        let outline = self.pts(&round_corners(pts, true, r));
        self.p.add(Shape::convex_polygon(outline.clone(), self.t, Stroke::NONE));
        for q in &outline {
            self.cap(*q, self.c);
        }
        self.p.add(Shape::closed_line(outline, Stroke::new(self.w, self.c)));
    }
    /// A speech bubble `w` wide at fixed proportions (13 × 9.5, tail scaled with it), tail at the
    /// bottom left, or bottom right when `flip`. Tinted, outlined in `col`.
    fn bubble(&self, x0: f32, y0: f32, w: f32, col: Color32, flip: bool) {
        let k = w / 13.0;
        let h = 9.5 * k;
        let shape = [(3.5, 9.5), (1.5, 12.0), (1.5, 9.5), (0.0, 9.5), (0.0, 0.0), (13.0, 0.0), (13.0, 9.5)];
        let pts: Vec<(f32, f32)> = shape.iter().map(|&(x, y)| (if flip { x0 + w - x * k } else { x0 + x * k }, y0 + y * k)).collect();
        self.block(x0, y0, x0 + w, y0 + h, 1.0, self.t);
        self.poly(&pts, col);
    }
    /// A window: frame and title bar, tinted.
    fn window(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.panel(x0, y0, x1, y1, 1.0);
        self.line(&[(x0, y0 + 2.5), (x1, y0 + 2.5)]);
    }
    /// A table grid: frame, header row line and the given inner lines.
    fn grid(&self, x0: f32, y0: f32, x1: f32, y1: f32, cols: &[f32], rows: &[f32]) {
        self.panel(x0, y0, x1, y1, 1.0);
        for x in cols {
            self.line(&[(*x, y0), (*x, y1)]);
        }
        for y in rows {
            self.line(&[(x0, *y), (x1, *y)]);
        }
    }
    /// The table from Table, `w` wide at 13:11: a tinted header row, three columns, three rows.
    fn table(&self, x0: f32, y0: f32, w: f32) {
        let (h, k) = (w * 11.0 / 13.0, w / 13.0);
        self.block(x0, y0, x0 + w, y0 + 3.5 * k, 1.0, self.a.gamma_multiply(0.5));
        self.grid(x0, y0, x0 + w, y0 + h, &[x0 + 4.5 * k, x0 + 8.5 * k], &[y0 + 3.5 * k, y0 + 7.25 * k]);
    }
    /// Where text line `i` sits on a page `h` tall at (x0, y0): (left, right, y). Lines are inset
    /// by the same share of the width on every page and spaced by the same share of its height; the
    /// first line is shorter so it clears the cut corner.
    fn page_slot(x0: f32, y0: f32, h: f32, i: usize) -> (f32, f32, f32) {
        let w = h * PAGE_RATIO;
        let right = if i == 0 { x0 + w * 0.58 } else { x0 + w * 0.77 };
        (x0 + w * 0.23, right, y0 + h * (0.27 + 0.18 * i as f32))
    }
    /// Ink text lines in the given slots of a page. Only full-size pages carry text: on a page
    /// under 10 units tall the shared inset would crowd the edge, so small pages stay plain.
    fn page_rows(&self, x0: f32, y0: f32, h: f32, slots: &[usize]) {
        for &i in slots {
            let (l, r, y) = Pen::page_slot(x0, y0, h, i);
            self.line(&[(l, y), (r, y)]);
        }
    }
    /// The person from [`Pen::person`], filled (the signed-in user).
    fn person_filled(&self, x: f32, y: f32, k: f32, col: Color32) {
        self.dot(x, y - 7.0 * k, 2.0 * k, col);
        self.fill(&Pen::arc_pts(x, y, 4.0 * k, 180.0, 360.0), col);
    }
    /// Field brackets from x0 to x1, arms a fixed share of the width.
    fn brackets(&self, x0: f32, x1: f32, y0: f32, y1: f32) {
        let arm = (x1 - x0) * 0.17;
        self.line(&[(x0 + arm, y0), (x0, y0), (x0, y1), (x0 + arm, y1)]);
        self.line(&[(x1 - arm, y0), (x1, y0), (x1, y1), (x1 - arm, y1)]);
    }
    /// A person: head and shoulders, centred on x with feet at y.
    fn person(&self, x: f32, y: f32, k: f32, col: Color32) {
        self.circle_c(x, y - 7.0 * k, 2.0 * k, col);
        self.arc(x, y, 4.0 * k, 180.0, 360.0, col);
    }
    /// A padlock `w` wide centred on `cx` with its body's bottom at `bottom`; the body is
    /// 0.75 w tall and the shackle 0.6 w across, at every size.
    fn lock(&self, cx: f32, bottom: f32, w: f32, col: Color32) {
        let (x0, x1, y1) = (cx - w / 2.0, cx + w / 2.0, bottom);
        let y0 = y1 - w * 0.75;
        let r = w * 0.3;
        self.block(x0, y0, x1, y1, 1.0, self.t);
        self.rect_c(x0, y0, x1, y1, 1.0, col);
        self.arc(cx, y0 - r * 0.6, r, 180.0, 360.0, col);
        self.path(&[(cx - r, y0 - r * 0.6), (cx - r, y0 - 0.4)], col);
        self.path(&[(cx + r, y0 - r * 0.6), (cx + r, y0 - 0.4)], col);
    }
    /// The picture element from Picture: a frame `w` wide (13:11) with a mountain and a sun, at the
    /// same proportions wherever it appears (Picture, Online Picture, Caption).
    fn picture(&self, x0: f32, y0: f32, w: f32) {
        let k = w / 13.0;
        let at = |x: f32, y: f32| (x0 + x * k, y0 + y * k);
        self.panel(x0, y0, x0 + w, y0 + 11.0 * k, 1.0);
        self.line(&[at(2.5, 8.5), at(5.5, 5.0), at(8.0, 7.5), at(9.25, 6.25), at(10.5, 7.5)]);
        let sun = at(9.25, 3.0);
        self.dot(sun.0, sun.1, 1.25 * k, self.a);
    }
    /// The pencil from Pencil, scaled by `k` with its 16-unit frame at (x0, y0): body outline in
    /// ink, a band across it, and the tip in `tip`, set apart from the body.
    fn pencil(&self, x0: f32, y0: f32, k: f32, tip: Color32) {
        let at = |pts: &[(f32, f32)]| pts.iter().map(|&(x, y)| (x0 + x * k, y0 + y * k)).collect::<Vec<_>>();
        let body = at(&[(3.9, 9.6), (10.5, 3.0), (13.0, 5.5), (6.4, 12.1)]);
        self.fill(&at(&[(1.5, 14.5), (2.5, 11.0), (5.0, 13.5)]), tip);
        self.fill(&body, self.t);
        self.poly(&body, self.c);
        self.line(&at(&[(9.2, 4.3), (11.7, 6.8)]));
    }
    /// A plus sign with 2-unit arms.
    fn plus(&self, x: f32, y: f32, col: Color32) {
        self.path(&[(x, y - 2.0), (x, y + 2.0)], col);
        self.path(&[(x - 2.0, y), (x + 2.0, y)], col);
    }
    /// A check mark centred on (x, y), `k` = 1 for 5 × 4 units.
    fn check(&self, x: f32, y: f32, k: f32, col: Color32) {
        self.path_r(&[(x - 2.5 * k, y + 0.1 * k), (x - 0.75 * k, y + 1.85 * k), (x + 2.5 * k, y - 2.15 * k)], col, TIP);
    }
    /// A cross with arms `k` units from the centre.
    fn cross(&self, x: f32, y: f32, k: f32, col: Color32) {
        self.path(&[(x - k, y - k), (x + k, y + k)], col);
        self.path(&[(x + k, y - k), (x - k, y + k)], col);
    }
    /// The chain link from Link: two halves, scaled by 0.85 about the centre and pulled apart by
    /// `sep` along the diagonal (0 = linked).
    fn chain(&self, sep: f32) {
        let half_a = [(7.0, 4.0), (8.5, 2.5), (10.0, 1.8), (11.8, 1.8), (13.5, 2.5), (14.2, 4.2), (14.2, 6.0), (13.5, 7.5), (12.0, 9.0)];
        let half_b = [(9.0, 12.0), (7.5, 13.5), (6.0, 14.2), (4.2, 14.2), (2.5, 13.5), (1.8, 11.8), (1.8, 10.0), (2.5, 8.5), (4.0, 7.0)];
        let t = |p: &(f32, f32), d: f32| (8.0 + (p.0 - 8.0) * 0.85 + d, 8.0 + (p.1 - 8.0) * 0.85 - d);
        self.line(&half_a.iter().map(|p| t(p, sep)).collect::<Vec<_>>());
        self.line(&half_b.iter().map(|p| t(p, -sep)).collect::<Vec<_>>());
    }
    /// The hand-drawn ink stroke shared by Ink to Shape and Ink to Math.
    fn scribble(&self, x: f32, y: f32) {
        self.curve(&[(x, y + 4.0), (x + 1.25, y), (x + 2.75, y + 4.5), (x + 4.0, y + 0.5)], false, self.a);
    }
    /// A lens with its handle, the handle in `h`.
    fn lens(&self, x: f32, y: f32, r: f32, h: Color32) {
        self.disc(x, y, r);
        // A handle in its own colour starts clear of the rim; an ink handle joins it.
        let d = (r + if h == self.c { 0.5 } else { 2.25 }) / std::f32::consts::SQRT_2;
        let e = r * 2.4 / std::f32::consts::SQRT_2;
        self.path(&[(x + d, y + d), (x + e, y + e)], h);
    }
}

/// Paint icon `name` into `r`. `c` is the line colour, `accent` the detail colour; status colours
/// come from the theme unless the icon is disabled (`c == accent`).
pub fn paint(p: &Painter, r: Rect, name: &str, c: Color32, accent: Color32) {
    let side = r.width().min(r.height());
    let s = side / 16.0;
    let disabled = c == accent || accent.a() == 0;
    let a = if accent.a() == 0 { c } else { accent };
    let tokens = crate::theme::Tokens::get(p.ctx());
    let tint = if disabled { Color32::TRANSPARENT } else { a.gamma_multiply(if tokens.dark { 0.28 } else { 0.16 }) };
    let pen = Pen { p, o: r.center() - vec2(8.0 * s, 8.0 * s), s, w: stroke_px(side), c, a, t: tint };
    let (green, red, orange) = if disabled { (c, c, c) } else { (tokens.green, tokens.red, tokens.orange) };
    draw(&pen, name, c, a, green, red, orange);
}

#[allow(clippy::too_many_lines)]
fn draw(pen: &Pen, name: &str, c: Color32, a: Color32, green: Color32, red: Color32, orange: Color32) {
    // Spacing rule: every part is one colour, and separate parts keep at least 1.25 units of clear
    // space (2.25 between stroke centre lines). Strokes stay between 1 and 15.
    match name {
        // Clipboard
        "paste" => {
            pen.block(2.5, 2.5, 13.5, 14.5, 1.0, pen.t);
            pen.line(&[
                (5.0, 2.5),
                (3.5, 2.5),
                (2.5, 3.5),
                (2.5, 13.5),
                (3.5, 14.5),
                (12.5, 14.5),
                (13.5, 13.5),
                (13.5, 3.5),
                (12.5, 2.5),
                (11.0, 2.5),
            ]);
            pen.rect(5.5, 1.25, 10.5, 3.75, 1.0);
            pen.arrow(8.0, 6.5, 8.0, 11.5, a);
        }
        "cut" => {
            pen.disc(4.5, 12.0, 2.0);
            pen.disc(11.5, 12.0, 2.0);
            pen.line(&[(5.6, 10.3), (11.5, 1.75)]);
            pen.line(&[(10.4, 10.3), (4.5, 1.75)]);
        }
        "copy" => {
            pen.line(&[(10.5, 3.0), (10.5, 2.5), (9.5, 1.5), (2.5, 1.5), (1.5, 2.5), (1.5, 9.5), (2.5, 10.5), (3.0, 10.5)]);
            pen.block(5.5, 5.5, 14.5, 14.5, 1.0, pen.t);
            pen.rect_c(5.5, 5.5, 14.5, 14.5, 1.0, a);
        }
        // Format painter as a roller: pick up the look, roll it on.
        "painter" => {
            pen.block(1.5, 2.0, 10.5, 6.0, 1.0, pen.t);
            pen.rect_c(1.5, 2.0, 10.5, 6.0, 1.0, a);
            pen.line(&[(12.75, 4.0), (14.25, 4.0), (14.25, 8.5), (8.0, 8.5), (8.0, 10.5)]);
            pen.rect(6.75, 10.5, 9.25, 14.5, 0.5);
        }
        // Font
        "bold" => {
            pen.heavy(&[(5.0, 8.0), (8.5, 8.0)]);
            pen.heavy(&Pen::arc_pts(8.5, 5.25, 2.75, -90.0, 90.0));
            pen.heavy(&Pen::arc_pts(9.0, 10.75, 2.75, -90.0, 90.0));
            pen.heavy(&[(8.5, 2.5), (4.75, 2.5), (4.75, 13.5), (9.0, 13.5)]);
        }
        "italic" => {
            pen.line(&[(7.0, 2.5), (12.0, 2.5)]);
            pen.line(&[(4.0, 13.5), (9.0, 13.5)]);
            pen.line(&[(9.5, 2.5), (6.5, 13.5)]);
        }
        "underline" => {
            pen.line(&[(4.5, 1.75), (4.5, 7.25)]);
            pen.arc(8.0, 7.25, 3.5, 180.0, 0.0, c);
            pen.line(&[(11.5, 7.25), (11.5, 1.75)]);
            pen.acc(&[(3.0, 14.0), (13.0, 14.0)]);
        }
        "strike" => {
            pen.arc(8.0, 5.25, 2.75, -20.0, -260.0, c);
            pen.arc(8.0, 10.75, 2.75, 160.0, -80.0, c);
            pen.line(&[(2.5, 8.0), (13.5, 8.0)]);
        }
        "charborder" => {
            pen.block(1.5, 1.5, 14.5, 14.5, 1.5, pen.t);
            pen.rect_c(1.5, 1.5, 14.5, 14.5, 1.5, a);
            pen.glyph_a(8.0, 11.5, 7.5, c, c);
        }
        // Sub- and superscript: a drawn x and a smooth digit from the font.
        "subscript" => {
            pen.line(&[(1.5, 3.0), (8.0, 11.0)]);
            pen.line(&[(8.0, 3.0), (1.5, 11.0)]);
            pen.text(12.25, 12.0, 7.5, "2", a, true);
        }
        "superscript" => {
            pen.line(&[(1.5, 5.0), (8.0, 13.0)]);
            pen.line(&[(8.0, 5.0), (1.5, 13.0)]);
            pen.text(12.25, 4.0, 7.5, "2", a, true);
        }
        "grow" => {
            pen.glyph_a(6.0, 14.0, 11.0, c, c);
            pen.arrow(13.0, 9.0, 13.0, 2.5, a);
        }
        "shrink" => {
            pen.glyph_a(6.0, 14.0, 8.0, c, c);
            pen.arrow(13.0, 2.5, 13.0, 9.0, a);
        }
        "case" => {
            pen.glyph_small_a(3.25, 13.0, 4.0, c);
            pen.glyph_a(11.0, 13.0, 8.5, c, c);
            pen.arrow(2.0, 4.5, 7.25, 4.5, a);
        }
        "clear" => {
            pen.glyph_a(5.0, 12.5, 9.0, c, c);
            pen.cross(12.75, 10.75, 1.75, red);
        }
        // Highlight, font colour, shading: the bar is the colour that will be applied. The split
        // button passes the current colour as the accent.
        "highlight" => {
            pen.fill(&[(6.0, 6.5), (10.5, 0.75), (13.5, 3.25), (9.0, 9.0)], pen.t);
            pen.poly(&[(6.0, 6.5), (10.5, 0.75), (13.5, 3.25), (9.0, 9.0)], c);
            pen.line(&[(6.0, 6.5), (4.5, 9.0), (6.5, 10.0), (9.0, 9.0)]);
            color_bar(pen, a);
        }
        "fontcolor" => {
            pen.glyph_a(8.0, 10.0, 8.5, c, c);
            color_bar(pen, a);
        }
        "effects" => {
            pen.glyph_a(6.0, 13.0, 10.0, c, c);
            pen.plus(12.75, 3.75, a);
            pen.dot(13.5, 10.0, 1.0, a);
        }
        "launcher" => {
            pen.line(&[(3.0, 6.5), (3.0, 13.0), (9.5, 13.0)]);
            pen.arrow(6.5, 9.5, 13.0, 3.0, a);
        }
        // Paragraph
        "bullets" => {
            for (i, y) in [3.0, 8.0, 13.0].iter().enumerate() {
                pen.dot(2.75, *y, 1.25, if i == 0 { a } else { c });
                pen.line(&[(6.5, *y), (14.5, *y)]);
            }
        }
        "numbering" => {
            for (i, y) in [3.0, 8.0, 13.0].iter().enumerate() {
                pen.text(2.75, *y, 5.0, ["1", "2", "3"][i], if i == 0 { a } else { c }, true);
                pen.line(&[(7.0, *y), (14.5, *y)]);
            }
        }
        "multilevel" => {
            for (i, (x, y)) in [(2.5, 3.0), (5.0, 8.0), (7.5, 13.0)].iter().enumerate() {
                pen.dot(*x, *y, 1.1, if i == 0 { a } else { c });
                pen.line(&[(x + 3.5, *y), (14.5, *y)]);
            }
        }
        "indent" => {
            pen.rows(1.5, 14.5, &[3.5, 12.5]);
            pen.rows(8.0, 14.5, &[6.5, 9.5]);
            pen.arrow(1.5, 8.0, 5.75, 8.0, a);
        }
        "outdent" => {
            pen.rows(1.5, 14.5, &[3.5, 12.5]);
            pen.rows(8.0, 14.5, &[6.5, 9.5]);
            pen.arrow(5.75, 8.0, 1.5, 8.0, a);
        }
        "sort" => {
            for (i, y) in [3.5, 6.5, 9.5, 12.5].iter().enumerate() {
                pen.line(&[(1.5, *y), (8.5 - 2.25 * i as f32, *y)]);
            }
            pen.arrow(12.5, 2.0, 12.5, 14.0, a);
        }
        "pilcrow" => {
            pen.block(4.0, 2.0, 8.5, 8.5, 3.0, pen.t);
            pen.arc(7.5, 5.25, 3.25, 90.0, 270.0, c);
            pen.line(&[(7.5, 8.5), (7.5, 2.0), (12.0, 2.0)]);
            pen.line(&[(8.5, 2.5), (8.5, 14.0)]);
            pen.line(&[(11.0, 2.5), (11.0, 14.0)]);
        }
        // Alignment: the edge the text holds to, in accent.
        "alignLeft" => {
            pen.acc(&[(1.5, 2.0), (1.5, 14.0)]);
            pen.rows(4.0, 14.5, &[3.0, 9.0]);
            pen.rows(4.0, 10.5, &[6.0, 12.0]);
        }
        "alignCenter" => {
            pen.rows(1.5, 14.5, &[3.0, 9.0]);
            for y in [6.0, 12.0] {
                pen.acc(&[(4.5, y), (11.5, y)]);
            }
        }
        "alignRight" => {
            pen.acc(&[(14.5, 2.0), (14.5, 14.0)]);
            pen.rows(1.5, 12.0, &[3.0, 9.0]);
            pen.rows(5.5, 12.0, &[6.0, 12.0]);
        }
        "justify" => {
            pen.acc(&[(1.5, 2.0), (1.5, 14.0)]);
            pen.acc(&[(14.5, 2.0), (14.5, 14.0)]);
            pen.rows(4.0, 12.0, &[3.0, 6.0, 9.0, 12.0]);
        }
        "textLtr" => {
            pen.rows(2.0, 14.0, &[3.5]);
            pen.rows(2.0, 10.0, &[6.5]);
            pen.arrow(2.0, 11.5, 14.0, 11.5, a);
        }
        "textRtl" => {
            pen.rows(2.0, 14.0, &[3.5]);
            pen.rows(6.0, 14.0, &[6.5]);
            pen.arrow(14.0, 11.5, 2.0, 11.5, a);
        }
        "lineSpacing" => {
            pen.rows(7.0, 14.5, &[3.0, 8.0, 13.0]);
            pen.span(3.0, 2.0, 3.0, 14.0, a);
        }
        "shading" => {
            pen.block(2.0, 1.5, 14.0, 10.0, 1.0, pen.t);
            pen.rows(4.5, 11.5, &[4.25, 7.25]);
            color_bar(pen, a);
        }
        "borders" => {
            for (x, y) in [
                (5.0, 2.0),
                (8.0, 2.0),
                (11.0, 2.0),
                (2.0, 5.0),
                (2.0, 8.0),
                (2.0, 11.0),
                (14.0, 5.0),
                (14.0, 8.0),
                (14.0, 11.0),
                (8.0, 5.0),
                (8.0, 11.0),
                (5.0, 8.0),
                (11.0, 8.0),
            ] {
                pen.dot(x, y, 0.65, c);
            }
            pen.acc(&[(2.0, 14.0), (14.0, 14.0)]);
        }
        // Editing
        "find" | "search" => pen.lens(6.5, 6.5, 4.5, a),
        "replace" => {
            pen.lens(4.0, 4.0, 2.5, c);
            pen.line(&[(9.5, 3.0), (14.5, 3.0)]);
            pen.acc(&[(5.5, 13.5), (14.5, 13.5)]);
            pen.curve(&[(11.5, 5.5), (12.25, 8.0), (11.0, 10.25)], false, a);
            pen.head(10.75, 10.75, 120.0, a);
        }
        "select" => {
            pen.line(&[(1.5, 4.0), (1.5, 1.5), (4.0, 1.5)]);
            pen.line(&[(7.0, 1.5), (9.0, 1.5)]);
            pen.line(&[(1.5, 7.0), (1.5, 9.0)]);
            pen.fill(&[(6.0, 6.0), (14.5, 9.5), (10.25, 10.25)], a);
            pen.fill(&[(6.0, 6.0), (10.25, 10.25), (9.5, 14.5)], a);
        }
        "dictate" => {
            pen.panel(6.0, 1.5, 10.0, 9.0, 2.0);
            pen.acc(&Pen::arc_pts(8.0, 7.75, 4.5, 0.0, 180.0));
            pen.acc(&[(8.0, 12.25), (8.0, 14.5)]);
        }
        "editor" => {
            pen.rows(1.5, 9.5, &[3.5, 6.5]);
            pen.rows(1.5, 5.5, &[9.5]);
            pen.check(11.5, 11.5, 1.0, green);
        }
        "spelling" => {
            pen.text(7.0, 4.25, 7.0, "abc", c, true);
            pen.path(&[(1.75, 11.75), (3.25, 10.75), (4.75, 11.75), (6.25, 10.75), (7.75, 11.75)], red);
            pen.check(12.0, 11.5, 0.9, green);
        }
        "styles" => {
            pen.panel(1.5, 1.5, 11.0, 11.0, 1.0);
            pen.glyph_a(6.25, 8.75, 4.5, c, c);
            pen.acc(&[(13.5, 4.5), (13.5, 13.5), (4.5, 13.5)]);
        }
        "stylesPane" => {
            pen.window(1.5, 1.5, 14.5, 14.5);
            pen.line(&[(9.25, 4.0), (9.25, 14.5)]);
            pen.rows(4.0, 6.75, &[7.0, 10.0]);
            pen.dot(12.0, 7.0, 0.65, a);
            pen.dot(12.0, 9.5, 0.65, c);
            pen.dot(12.0, 12.0, 0.65, c);
        }
        // Insert
        "coverPage" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.page_rows(2.5, 1.5, 13.0, &[0]);
            let (l, r, y1) = Pen::page_slot(2.5, 1.5, 13.0, 1);
            let (_, _, y2) = Pen::page_slot(2.5, 1.5, 13.0, 2);
            pen.block(l, y1 - 0.75, r, y2 + 0.75, 0.5, a);
            pen.page_rows(2.5, 1.5, 13.0, &[3]);
        }
        "blankPage" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.plus(8.0, 8.0, a);
        }
        "pageBreak" => {
            pen.line(&[(2.5, 1.0), (2.5, 5.0), (13.5, 5.0), (13.5, 1.0)]);
            pen.line(&[(2.5, 15.0), (2.5, 11.0), (13.5, 11.0), (13.5, 15.0)]);
            dashes(pen, 8.0);
        }
        // Table: the header row is tinted, never a second line colour on the grid.
        "table" => pen.table(1.5, 2.5, 13.0),
        "picture" => pen.picture(1.5, 2.5, 13.0),
        "onlinePicture" => {
            pen.picture(1.0, 1.0, 9.0);
            pen.circle_c(12.5, 12.5, 2.25, a);
            pen.acc(&[(10.25, 12.5), (14.75, 12.5)]);
            pen.acc(&[(12.5, 10.25), (11.7, 12.5), (12.5, 14.75)]);
            pen.acc(&[(12.5, 10.25), (13.3, 12.5), (12.5, 14.75)]);
        }
        "shapes" => {
            pen.panel(1.5, 1.5, 6.5, 6.5, 1.0);
            pen.circle_c(11.75, 4.25, 2.75, a);
            pen.fill(&[(8.0, 9.0), (12.5, 14.5), (3.5, 14.5)], pen.t);
            pen.poly(&[(8.0, 9.0), (12.5, 14.5), (3.5, 14.5)], c);
        }
        "shapeEffects" => {
            pen.panel(1.5, 1.5, 10.0, 10.0, 1.5);
            pen.acc(&[(12.5, 4.0), (12.5, 11.0), (11.0, 12.5), (4.0, 12.5)]);
        }
        "icons" => {
            pen.disc(4.5, 4.5, 2.5);
            pen.poly(&[(11.5, 2.0), (14.0, 6.75), (9.0, 6.75)], c);
            pen.block(2.0, 9.25, 7.0, 14.0, 1.0, a);
            pen.circle(11.5, 11.5, 2.5);
        }
        "models3d" => {
            pen.fill(&[(8.0, 1.5), (14.0, 4.5), (8.0, 7.5), (2.0, 4.5)], pen.a.gamma_multiply(0.5));
            pen.poly(&[(8.0, 1.5), (14.0, 4.5), (14.0, 11.5), (8.0, 14.5), (2.0, 11.5), (2.0, 4.5)], c);
            pen.line(&[(2.0, 4.5), (8.0, 7.5), (14.0, 4.5)]);
            pen.line(&[(8.0, 7.5), (8.0, 14.5)]);
        }
        "smartArt" => {
            pen.block(5.5, 1.5, 10.5, 5.0, 1.0, a);
            pen.line(&[(8.0, 7.25), (8.0, 8.5)]);
            pen.line(&[(4.0, 10.5), (4.0, 8.5), (12.0, 8.5), (12.0, 10.5)]);
            pen.panel(1.5, 10.5, 6.5, 14.5, 1.0);
            pen.panel(9.5, 10.5, 14.5, 14.5, 1.0);
        }
        "chart" => {
            pen.line(&[(1.5, 14.5), (14.5, 14.5)]);
            pen.panel(1.75, 8.0, 4.25, 12.0, 0.75);
            // Outlined like its neighbours, so all three bars share one baseline.
            pen.block(6.75, 2.5, 9.25, 12.0, 0.75, a);
            pen.rect_c(6.75, 2.5, 9.25, 12.0, 0.75, a);
            pen.panel(11.75, 5.5, 14.25, 12.0, 0.75);
        }
        "screenshot" => {
            pen.line(&[(1.5, 5.0), (1.5, 1.5), (5.0, 1.5)]);
            pen.line(&[(11.0, 1.5), (14.5, 1.5), (14.5, 5.0)]);
            pen.line(&[(14.5, 11.0), (14.5, 14.5), (11.0, 14.5)]);
            pen.line(&[(5.0, 14.5), (1.5, 14.5), (1.5, 11.0)]);
            pen.p.circle_filled(pen.pt(8.0, 8.0), 2.25 * pen.s, a);
        }
        "video" => {
            pen.panel(1.5, 3.5, 10.0, 12.5, 1.5);
            pen.poly(&[(12.5, 7.0), (14.5, 5.0), (14.5, 11.0), (12.5, 9.0)], a);
        }
        "link" => {
            pen.chain(0.0);
            pen.acc(&[(6.5, 9.5), (9.5, 6.5)]);
        }
        "bookmark" => {
            pen.fill(&[(3.5, 1.5), (12.5, 1.5), (12.5, 14.5), (8.0, 11.0)], pen.t);
            pen.fill(&[(3.5, 1.5), (8.0, 11.0), (3.5, 14.5)], pen.t);
            pen.poly(&[(3.5, 1.5), (12.5, 1.5), (12.5, 14.5), (8.0, 11.0), (3.5, 14.5)], c);
            pen.acc(&[(6.0, 5.0), (10.0, 5.0)]);
        }
        "crossRef" => {
            pen.page_h(1.5, 1.5, 8.0);
            pen.acc(&[(5.0, 12.5), (5.0, 14.0), (13.5, 14.0), (13.5, 6.0)]);
            pen.head(13.5, 5.5, -90.0, a);
        }
        "comment" => {
            pen.bubble(1.5, 2.0, 13.0, c, false);
            pen.rows(4.5, 11.5, &[5.0, 8.0]);
        }
        "newComment" => {
            pen.bubble(1.5, 5.0, 9.5, c, false);
            pen.plus(13.0, 2.75, a);
        }
        "header" => {
            pen.page_h(2.5, 1.5, 13.0);
            let (l, r, y) = Pen::page_slot(2.5, 1.5, 13.0, 0);
            pen.acc(&[(l, y), (r, y)]);
            pen.page_rows(2.5, 1.5, 13.0, &[2, 3]);
        }
        "footer" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.page_rows(2.5, 1.5, 13.0, &[0, 1]);
            let (l, r, y) = Pen::page_slot(2.5, 1.5, 13.0, 3);
            pen.acc(&[(l, y), (r, y)]);
        }
        "pageNumber" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.page_rows(2.5, 1.5, 13.0, &[0, 1]);
            let (_, _, y2) = Pen::page_slot(2.5, 1.5, 13.0, 2);
            let (_, _, y3) = Pen::page_slot(2.5, 1.5, 13.0, 3);
            pen.text(8.0, (y2 + y3) / 2.0, 4.5, "1", a, true);
        }
        "textBox" => {
            pen.line(&[(1.5, 4.0), (1.5, 1.5), (4.0, 1.5)]);
            pen.line(&[(12.0, 1.5), (14.5, 1.5), (14.5, 4.0)]);
            pen.line(&[(14.5, 12.0), (14.5, 14.5), (12.0, 14.5)]);
            pen.line(&[(4.0, 14.5), (1.5, 14.5), (1.5, 12.0)]);
            pen.line(&[(6.5, 1.5), (9.5, 1.5)]);
            pen.line(&[(6.5, 14.5), (9.5, 14.5)]);
            pen.line(&[(1.5, 6.5), (1.5, 9.5)]);
            pen.line(&[(14.5, 6.5), (14.5, 9.5)]);
            pen.acc(&[(5.5, 5.0), (10.5, 5.0)]);
            pen.acc(&[(8.0, 5.0), (8.0, 11.5)]);
        }
        "quickParts" => {
            pen.panel(1.5, 1.5, 6.5, 6.5, 1.0);
            pen.block(9.5, 1.5, 14.5, 6.5, 1.0, a);
            pen.panel(1.5, 9.5, 6.5, 14.5, 1.0);
            pen.panel(9.5, 9.5, 14.5, 14.5, 1.0);
        }
        "wordArt" => {
            pen.glyph_a(8.0, 10.0, 8.5, c, c);
            pen.arc(8.0, 3.5, 11.0, 58.0, 122.0, a);
        }
        "dropCap" => {
            pen.panel(1.0, 1.0, 8.5, 8.0, 1.0);
            pen.glyph_a(4.75, 6.0, 3.0, a, a);
            pen.rows(11.0, 14.5, &[2.0, 5.0]);
            pen.rows(1.5, 14.5, &[11.0, 14.0]);
        }
        "signature" => {
            pen.acc(&[(2.0, 10.0), (4.0, 5.0), (5.5, 9.5), (8.0, 4.0), (9.5, 9.0), (12.0, 7.0), (14.0, 8.0)]);
            pen.line(&[(1.5, 13.5), (14.5, 13.5)]);
        }
        "dateTime" => {
            pen.panel(1.5, 2.5, 14.5, 14.5, 1.5);
            pen.line(&[(1.5, 6.0), (14.5, 6.0)]);
            pen.line(&[(5.0, 1.0), (5.0, 3.5)]);
            pen.line(&[(11.0, 1.0), (11.0, 3.5)]);
            pen.dot(10.5, 10.5, 1.4, a);
            pen.dot(5.5, 10.5, 1.0, c);
        }
        "object" => {
            pen.panel(1.5, 1.5, 14.5, 14.5, 1.5);
            pen.block(5.0, 5.0, 11.0, 11.0, 1.0, a);
        }
        "equation" => {
            pen.line(&[(1.5, 9.0), (3.5, 8.0), (6.0, 14.0), (9.0, 2.0), (14.5, 2.0)]);
            pen.acc(&[(10.5, 6.5), (14.5, 11.5)]);
            pen.acc(&[(14.5, 6.5), (10.5, 11.5)]);
        }
        "symbol" => {
            pen.arc(8.0, 7.0, 5.0, 130.0, 410.0, c);
            pen.line(&[(2.0, 13.5), (5.0, 13.5), (4.8, 10.8)]);
            pen.line(&[(14.0, 13.5), (11.0, 13.5), (11.2, 10.8)]);
            pen.dot(8.0, 7.0, 1.0, a);
        }
        // Draw: a nib or tip set apart from the body by a clear gap.
        "pen" => {
            pen.fill(&[(1.5, 14.5), (2.5, 11.0), (5.0, 13.5)], a);
            pen.poly(&[(3.9, 9.6), (10.5, 3.0), (13.0, 5.5), (6.4, 12.1)], c);
            pen.line(&[(9.2, 4.3), (11.7, 6.8)]);
        }
        "pencil" => pen.pencil(0.0, 0.0, 1.0, orange),
        "eraser" => {
            pen.fill(&[(1.5, 9.5), (4.75, 6.25), (10.25, 11.75), (8.5, 13.5), (5.5, 13.5)], pen.a.gamma_multiply(0.5));
            pen.poly(&[(1.5, 9.5), (8.0, 3.0), (13.5, 8.5), (8.5, 13.5), (5.5, 13.5)], c);
            pen.line(&[(4.75, 6.25), (10.75, 12.25)]);
            pen.line(&[(11.5, 14.5), (14.5, 14.5)]);
        }
        // Ink thickness: three strokes, thin to thick; the thickest in the accent.
        "thickness" => {
            pen.line(&[(2.5, 3.0), (13.5, 3.0)]);
            pen.block(2.0, 5.75, 14.0, 8.0, 0.75, c);
            pen.block(2.0, 10.5, 14.0, 13.75, 1.0, a);
        }
        "lasso" => {
            // One rope: a loose loop that runs on into a trailing end.
            let rope = [(8.5, 1.75), (13.25, 2.75), (14.25, 5.5), (12.0, 8.25), (7.5, 9.0), (3.0, 8.0), (1.75, 5.0), (4.25, 2.5)];
            pen.p.add(Shape::convex_polygon(pen.pts(&smooth(&rope, true)), pen.t, Stroke::NONE));
            pen.curve(&rope, true, c);
            pen.curve(&[(3.0, 8.0), (4.75, 10.0), (3.5, 12.5), (5.0, 14.5), (7.5, 14.0)], false, c);
        }
        "inkToShape" => {
            pen.scribble(1.0, 5.75);
            pen.arrow(6.75, 8.0, 8.75, 8.0, c);
            pen.panel(11.0, 5.25, 14.5, 10.75, 1.0);
        }
        "inkToMath" => {
            pen.scribble(1.0, 5.75);
            pen.arrow(6.75, 8.0, 8.75, 8.0, c);
            pen.line(&[(11.0, 8.75), (11.75, 8.25), (12.5, 11.0), (13.75, 5.25), (14.75, 5.25)]);
        }
        "canvas" => {
            pen.panel(1.5, 2.5, 14.5, 13.5, 1.0);
            pen.acc(&[(4.0, 10.5), (6.5, 6.0), (9.0, 9.5), (12.0, 5.5)]);
        }
        "replay" => {
            pen.cycle(8.0, 8.0, 6.0, c);
            pen.fill(&[(6.5, 5.25), (11.0, 8.0), (6.5, 10.75)], a);
        }
        // Design
        "themes" => {
            pen.panel(1.5, 1.5, 14.5, 14.5, 1.5);
            pen.block(3.25, 3.25, 7.25, 7.25, 1.0, a);
            pen.block(8.75, 3.25, 12.75, 7.25, 1.0, orange);
            pen.block(3.25, 8.75, 7.25, 12.75, 1.0, green);
            pen.line(&[(9.5, 10.75), (12.0, 10.75)]);
        }
        "colors" => {
            pen.p.circle_filled(pen.pt(4.25, 5.0), 2.75 * pen.s, red);
            pen.p.circle_filled(pen.pt(11.75, 5.0), 2.75 * pen.s, a);
            pen.p.circle_filled(pen.pt(8.0, 11.75), 2.75 * pen.s, green);
        }
        "fonts" => {
            pen.glyph_a(4.75, 13.0, 9.0, c, c);
            pen.glyph_small_a(12.75, 13.0, 4.0, a);
        }
        "paraSpacing" => {
            pen.rows(6.0, 14.5, &[2.0, 5.0]);
            pen.span(2.75, 5.25, 2.75, 10.75, a);
            pen.rows(6.0, 14.5, &[11.0, 14.0]);
        }
        "effectsDesign" => {
            pen.disc(6.5, 9.5, 5.0);
            pen.plus(12.75, 3.75, a);
            pen.dot(13.75, 12.25, 1.0, a);
        }
        "setDefault" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.check(8.0, 8.5, 1.0, green);
        }
        "watermark" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.acc(&[(5.0, 11.5), (10.0, 6.5)]);
        }
        "pageColor" => {
            let k = 11.0 * 0.3;
            pen.fill(&[(2.5, 1.5), (13.5 - k, 1.5), (13.5, 1.5 + k), (13.5, 14.5), (2.5, 14.5)], a.gamma_multiply(0.55));
            pen.page_h(2.5, 1.5, 13.0);
        }
        "pageBorders" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.rect_c(4.75, 5.25, 11.0, 12.25, 0.5, a);
        }
        // Layout
        "margins" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.acc(&[(5.0, 4.5), (5.0, 11.75)]);
            pen.acc(&[(11.0, 6.5), (11.0, 11.75)]);
        }
        "orientation" => {
            // The same page, upright and turned a quarter turn clockwise.
            pen.page_h(1.25, 1.25, 6.5);
            pen.page_landscape(8.25, 9.25, 6.5);
            pen.arc(9.0, 6.75, 4.25, -90.0, 0.0, a);
            pen.head(13.25, 6.75, 90.0, a);
        }
        "size" => {
            let x1 = pen.page_h(5.5, 1.5, 8.5);
            pen.span(2.25, 2.0, 2.25, 10.0, a);
            pen.span(5.75, 13.25, x1 - 0.25, 13.25, a);
        }
        "columns" => {
            pen.rows(1.5, 5.5, &[3.0, 6.0, 9.0, 12.0]);
            pen.rows(10.5, 14.5, &[3.0, 6.0, 9.0]);
            pen.acc(&[(8.0, 2.0), (8.0, 14.0)]);
        }
        "breaks" => {
            pen.line(&[(2.5, 1.0), (2.5, 5.0), (13.5, 5.0), (13.5, 1.0)]);
            pen.line(&[(2.5, 15.0), (2.5, 11.0), (10.5, 11.0), (13.5, 14.0), (13.5, 15.0)]);
            dashes(pen, 8.0);
        }
        "lineNumbers" => {
            for (i, y) in [3.0, 8.0, 13.0].iter().enumerate() {
                pen.text(2.5, *y, 5.0, ["1", "2", "3"][i], a, true);
                pen.line(&[(6.0, *y), (14.5, *y)]);
            }
        }
        "hyphenation" => {
            pen.rows(1.5, 8.5, &[3.5]);
            pen.acc(&[(11.0, 3.5), (13.0, 3.5)]);
            pen.rows(1.5, 14.5, &[6.5, 9.5]);
            pen.rows(1.5, 10.0, &[12.5]);
        }
        "position" => {
            pen.panel(1.5, 1.5, 14.5, 14.5, 1.5);
            for x in [4.75, 8.0] {
                for y in [4.75, 8.0, 11.25] {
                    pen.dot(x, y, 0.7, c);
                }
            }
            pen.dot(11.25, 8.0, 0.7, c);
            pen.dot(11.25, 11.25, 0.7, c);
            pen.block(10.0, 3.5, 12.5, 6.0, 0.5, a);
        }
        "wrapText" => {
            pen.rows(1.5, 14.5, &[3.5, 12.5]);
            pen.rows(1.5, 4.5, &[6.5, 9.5]);
            pen.rows(11.5, 14.5, &[6.5, 9.5]);
            pen.block(6.25, 5.75, 9.75, 10.25, 1.0, a);
        }
        "bringForward" => {
            pen.line(&[(9.5, 3.0), (9.5, 2.5), (8.5, 1.5), (2.5, 1.5), (1.5, 2.5), (1.5, 8.5), (2.5, 9.5), (3.0, 9.5)]);
            pen.block(5.5, 5.5, 14.5, 14.5, 1.0, a);
        }
        "sendBackward" => {
            pen.acc(&[(9.5, 3.0), (9.5, 2.5), (8.5, 1.5), (2.5, 1.5), (1.5, 2.5), (1.5, 8.5), (2.5, 9.5), (3.0, 9.5)]);
            pen.panel(5.5, 5.5, 14.5, 14.5, 1.0);
        }
        "selectionPane" => {
            pen.window(1.5, 1.5, 14.5, 14.5);
            for (i, y) in [6.75, 9.25, 11.75].iter().enumerate() {
                pen.dot(4.25, *y, 0.75, if i == 0 { a } else { c });
                pen.line(&[(7.0, *y), (12.0, *y)]);
            }
        }
        "align" => {
            pen.acc(&[(1.5, 1.5), (1.5, 14.5)]);
            pen.panel(4.0, 2.5, 14.0, 6.5, 1.0);
            pen.panel(4.0, 9.5, 10.0, 13.5, 1.0);
        }
        "group" => {
            for pts in [
                [(1.5, 4.0), (1.5, 1.5), (4.0, 1.5)],
                [(12.0, 1.5), (14.5, 1.5), (14.5, 4.0)],
                [(14.5, 12.0), (14.5, 14.5), (12.0, 14.5)],
                [(4.0, 14.5), (1.5, 14.5), (1.5, 12.0)],
            ] {
                pen.acc(&pts);
            }
            pen.panel(4.25, 4.25, 8.0, 8.0, 1.0);
            pen.disc(10.75, 10.75, 1.75);
        }
        "rotate" => {
            pen.panel(5.5, 8.0, 10.5, 13.0, 1.0);
            pen.arc(8.0, 10.0, 6.5, 200.0, 330.0, a);
            pen.head(13.63, 6.75, 60.0, a);
        }
        "indentLeft" => {
            pen.acc(&[(1.5, 4.5), (1.5, 11.5)]);
            pen.arrow(1.5, 8.0, 5.75, 8.0, a);
            pen.rows(8.0, 14.5, &[3.5, 6.5, 9.5, 12.5]);
        }
        "indentRight" => {
            pen.acc(&[(14.5, 4.5), (14.5, 11.5)]);
            pen.arrow(14.5, 8.0, 10.25, 8.0, a);
            pen.rows(1.5, 8.0, &[3.5, 6.5, 9.5, 12.5]);
        }
        "spaceBefore" => {
            pen.rows(6.0, 14.5, &[2.0]);
            pen.span(2.75, 2.25, 2.75, 9.25, a);
            pen.rows(6.0, 14.5, &[9.5, 12.5]);
        }
        "spaceAfter" => {
            pen.rows(6.0, 14.5, &[3.5, 6.5]);
            pen.span(2.75, 6.75, 2.75, 13.75, a);
            pen.rows(6.0, 14.5, &[14.0]);
        }
        // References
        "toc" => {
            pen.page_h(2.5, 1.5, 13.0);
            for i in 0..3 {
                let (l, _, y) = Pen::page_slot(2.5, 1.5, 13.0, i + 1);
                pen.line(&[(l, y), (l + 3.0, y)]);
                pen.dot(l + 5.5, y, 0.55, a);
            }
            pen.page_rows(2.5, 1.5, 13.0, &[0]);
        }
        "addText" => {
            pen.rows(1.5, 14.5, &[3.5, 6.5]);
            pen.rows(1.5, 8.0, &[9.5]);
            pen.plus(12.5, 12.5, a);
        }
        "update" => pen.cycle(8.0, 8.0, 5.5, a),
        "updateTable" => {
            pen.table(1.0, 1.0, 8.0);
            pen.cycle(11.75, 11.75, 2.5, a);
        }
        "footnote" => {
            pen.rows(1.5, 9.0, &[3.5, 6.5]);
            pen.text(12.0, 3.0, 5.0, "1", a, true);
            pen.line(&[(1.5, 9.5), (6.0, 9.5)]);
            pen.acc(&[(1.5, 12.5), (12.0, 12.5)]);
        }
        "endnote" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.page_rows(2.5, 1.5, 13.0, &[0]);
            let (l, _, y) = Pen::page_slot(2.5, 1.5, 13.0, 1);
            pen.line(&[(l, y), (l + 2.75, y)]);
            pen.text(l + 5.5, y - 1.0, 4.0, "1", a, true);
            let (l, r, y) = Pen::page_slot(2.5, 1.5, 13.0, 3);
            pen.acc(&[(l, y), (r, y)]);
        }
        "nextFootnote" => {
            pen.rows(1.5, 8.5, &[3.5]);
            pen.text(12.0, 3.0, 5.0, "1", c, true);
            pen.arrow(2.0, 9.5, 13.0, 9.5, a);
        }
        "showNotes" => {
            pen.rows(1.5, 9.0, &[3.5, 6.5]);
            pen.text(12.0, 3.0, 5.0, "1", a, true);
            pen.line(&[(1.5, 9.5), (6.0, 9.5)]);
            pen.block(1.5, 11.75, 14.5, 14.25, 1.0, a);
        }
        "researcher" => {
            pen.page_h(1.5, 1.5, 8.0);
            pen.lens(11.5, 11.5, 1.75, a);
        }
        // Zotero
        "docPrefs" => {
            pen.page_h(1.5, 1.5, 8.0);
            pen.circle_c(12.25, 12.25, 1.4, a);
            for k in 0..6 {
                let t = k as f32 * std::f32::consts::PI / 3.0;
                let (dx, dy) = (t.cos(), t.sin());
                pen.acc(&[(12.25 + 2.1 * dx, 12.25 + 2.1 * dy), (12.25 + 2.7 * dx, 12.25 + 2.7 * dy)]);
            }
        }
        "unlinkCitations" => {
            pen.chain(1.0);
            pen.acc(&[(4.25, 4.25), (5.75, 5.75)]);
            pen.acc(&[(10.25, 10.25), (11.75, 11.75)]);
        }
        "addNote" => {
            pen.fill(&[(1.5, 1.5), (14.5, 1.5), (14.5, 10.5), (10.5, 14.5), (1.5, 14.5)], pen.t);
            pen.poly(&[(1.5, 1.5), (14.5, 1.5), (14.5, 10.5), (10.5, 14.5), (1.5, 14.5)], c);
            pen.line(&[(10.5, 14.5), (10.5, 10.5), (14.5, 10.5)]);
            pen.plus(7.0, 7.0, a);
        }
        "citation" => {
            for x in [3.0, 7.5] {
                pen.dot(x, 4.0, 1.5, a);
                pen.acc(&[(x + 1.25, 4.5), (x + 0.25, 7.25)]);
            }
            pen.rows(11.0, 14.5, &[3.5, 6.5]);
            pen.rows(1.5, 14.5, &[9.5, 12.5]);
        }
        "sources" => {
            pen.panel(1.5, 2.5, 4.0, 12.25, 0.5);
            pen.panel(6.25, 2.5, 8.75, 12.25, 0.5);
            pen.poly(&[(11.0, 3.75), (12.75, 3.25), (14.5, 11.75), (12.75, 12.25)], a);
            pen.line(&[(1.0, 14.5), (15.0, 14.5)]);
        }
        "bibliography" => {
            pen.page_h(2.5, 1.5, 13.0);
            for i in [0, 2] {
                let (l, r, y) = Pen::page_slot(2.5, 1.5, 13.0, i);
                pen.acc(&[(l, y), (r, y)]);
                let (_, r, y) = Pen::page_slot(2.5, 1.5, 13.0, i + 1);
                pen.line(&[(l + 1.75, y), (r, y)]);
            }
        }
        "caption" => {
            pen.picture(2.5, 1.0, 11.0);
            pen.acc(&[(2.5, 13.5), (13.5, 13.5)]);
        }
        "tableOfFigures" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.page_rows(2.5, 1.5, 13.0, &[0]);
            for i in [1, 3] {
                let (l, r, y) = Pen::page_slot(2.5, 1.5, 13.0, i);
                pen.block(l, y - 0.85, l + 2.25, y + 0.85, 0.5, a);
                pen.line(&[(l + 4.0, y), (r, y)]);
            }
        }
        "markEntry" => {
            pen.fill(&[(1.5, 3.5), (10.0, 3.5), (14.0, 8.0), (10.0, 12.5), (1.5, 12.5)], pen.t);
            pen.poly(&[(1.5, 3.5), (10.0, 3.5), (14.0, 8.0), (10.0, 12.5), (1.5, 12.5)], c);
            pen.acc(&[(4.0, 8.0), (8.5, 8.0)]);
        }
        "index" => {
            pen.page_h(2.5, 1.5, 13.0);
            let (l, _, y) = Pen::page_slot(2.5, 1.5, 13.0, 1);
            pen.glyph_a(l + 1.25, y, 3.5, a, a);
            pen.page_rows(2.5, 1.5, 13.0, &[2, 3]);
        }
        "markCitation" => {
            pen.fill(&[(1.5, 3.5), (10.0, 3.5), (14.0, 8.0), (10.0, 12.5), (1.5, 12.5)], pen.t);
            pen.poly(&[(1.5, 3.5), (10.0, 3.5), (14.0, 8.0), (10.0, 12.5), (1.5, 12.5)], c);
            pen.text(6.25, 8.0, 5.5, "\u{a7}", a, true);
        }
        "tableOfAuthorities" => {
            pen.page_h(2.5, 1.5, 13.0);
            let (l, _, y) = Pen::page_slot(2.5, 1.5, 13.0, 0);
            pen.text(l + 1.0, y, 5.0, "§", a, true);
            pen.page_rows(2.5, 1.5, 13.0, &[2, 3]);
        }
        // Mailings
        "envelope" => {
            pen.panel(1.5, 3.0, 14.5, 13.0, 1.5);
            pen.acc(&[(4.0, 5.5), (8.0, 8.75), (12.0, 5.5)]);
        }
        "labels" => {
            pen.panel(1.5, 1.5, 14.5, 14.5, 1.5);
            pen.block(4.0, 3.5, 12.0, 7.0, 0.75, a);
            pen.rect(4.0, 9.75, 12.0, 12.25, 0.75);
        }
        // Mail merge: one letter, many copies out.
        "mailMerge" => {
            pen.page_h(1.5, 1.5, 8.0);
            pen.arrow(11.25, 3.5, 14.5, 3.5, a);
            pen.arrow(11.25, 8.5, 14.5, 8.5, a);
            pen.arrow(5.0, 12.0, 5.0, 14.75, a);
        }
        "recipients" => {
            pen.person(5.0, 14.5, 0.9, c);
            pen.person(12.0, 10.0, 0.7, a);
        }
        "editRecipients" => {
            pen.person(4.5, 9.5, 0.7, c);
            pen.pencil(6.75, 6.25, 0.58, a);
        }
        "mergeField" => {
            brackets(pen, 3.0, 13.0);
            pen.acc(&[(6.25, 8.0), (9.75, 8.0)]);
        }
        "addressBlock" => {
            brackets(pen, 2.0, 14.0);
            pen.acc(&[(6.25, 5.0), (9.75, 5.0)]);
            pen.rows(6.25, 9.75, &[8.0]);
            pen.rows(6.25, 8.5, &[11.0]);
        }
        "greetingLine" => {
            brackets(pen, 3.0, 13.0);
            pen.text(8.0, 8.0, 7.0, "Hi", a, true);
        }
        "rules" => {
            pen.brackets(1.5, 7.5, 1.5, 6.0);
            pen.acc(&[(4.5, 8.5), (4.5, 13.5), (9.5, 13.5)]);
            pen.acc(&[(4.5, 10.0), (9.5, 10.0)]);
            pen.dot(12.25, 10.0, 1.0, c);
            pen.dot(12.25, 13.5, 1.0, c);
        }
        "matchFields" => {
            for y in [3.0, 8.0, 13.0] {
                pen.line(&[(1.5, y), (4.5, y)]);
                pen.line(&[(11.5, y), (14.5, y)]);
            }
            pen.acc(&[(6.75, 3.0), (9.25, 8.0)]);
            pen.acc(&[(6.75, 8.0), (9.25, 3.0)]);
            pen.acc(&[(6.75, 13.0), (9.25, 13.0)]);
        }
        "highlightFields" => {
            pen.block(5.25, 6.0, 10.75, 10.0, 1.0, a.gamma_multiply(0.45));
            brackets(pen, 3.0, 13.0);
            pen.acc(&[(6.5, 8.0), (9.5, 8.0)]);
        }
        "preview" => {
            pen.line(&Pen::arc_pts(8.0, 13.0, 8.5, 210.0, 330.0));
            pen.line(&Pen::arc_pts(8.0, 3.0, 8.5, 30.0, 150.0));
            pen.disc(8.0, 8.0, 2.5);
            pen.dot(8.0, 8.0, 1.0, a);
        }
        "next" => pen.acc(&[(6.0, 3.0), (11.0, 8.0), (6.0, 13.0)]),
        "previous" => pen.acc(&[(10.0, 3.0), (5.0, 8.0), (10.0, 13.0)]),
        "first" => {
            pen.line(&[(3.5, 3.0), (3.5, 13.0)]);
            pen.acc(&[(11.5, 3.0), (6.5, 8.0), (11.5, 13.0)]);
        }
        "last" => {
            pen.line(&[(12.5, 3.0), (12.5, 13.0)]);
            pen.acc(&[(4.5, 3.0), (9.5, 8.0), (4.5, 13.0)]);
        }
        "finish" => {
            pen.page_h(1.5, 1.5, 8.0);
            pen.check(12.25, 12.25, 0.9, green);
        }
        "checkErrors" => {
            pen.page_h(1.5, 1.5, 8.0);
            pen.circle_c(12.25, 12.25, 2.25, red);
            pen.path(&[(12.25, 11.0), (12.25, 12.4)], red);
            pen.dot(12.25, 13.5, 0.45, red);
        }
        // Review
        "thesaurus" => {
            pen.panel(2.5, 1.5, 13.5, 14.5, 1.0);
            pen.line(&[(5.0, 1.5), (5.0, 14.5)]);
            pen.acc(&[(7.5, 5.0), (11.0, 5.0)]);
            pen.rows(7.5, 11.0, &[8.0, 11.0]);
        }
        "wordCount" => {
            pen.text(8.0, 6.0, 7.0, "123", c, true);
            pen.acc(&[(2.5, 12.5), (13.5, 12.5)]);
        }
        "readAloud" => {
            pen.glyph_a(4.5, 12.5, 9.0, c, c);
            pen.arc(8.5, 8.0, 3.0, -50.0, 50.0, a);
            pen.arc(8.5, 8.0, 6.0, -50.0, 50.0, a);
        }
        "accessibility" => {
            pen.dot(8.0, 2.5, 1.4, a);
            pen.line(&[(2.5, 5.5), (13.5, 5.5)]);
            pen.line(&[(8.0, 5.5), (8.0, 9.5), (5.0, 14.5)]);
            pen.line(&[(8.0, 9.5), (11.0, 14.5)]);
        }
        "translate" => {
            pen.glyph_a(4.25, 9.0, 7.5, c, c);
            pen.arrow(8.5, 4.0, 12.5, 4.0, a);
            pen.dot(12.1, 8.0, 0.75, a);
            pen.acc(&[(9.5, 9.75), (14.75, 9.75)]);
            pen.acc(&[(13.5, 9.75), (12.25, 12.25), (9.75, 14.75)]);
            pen.acc(&[(10.5, 9.75), (11.75, 12.25), (14.5, 14.75)]);
        }
        "language" => {
            pen.disc(8.0, 8.0, 6.5);
            pen.line(&[(1.5, 8.0), (14.5, 8.0)]);
            pen.line(&[(2.75, 4.75), (13.25, 4.75)]);
            pen.line(&[(2.75, 11.25), (13.25, 11.25)]);
            let meridian =
                |a0: f32, a1: f32| Pen::arc_pts(8.0, 8.0, 6.5, a0, a1).iter().map(|&(x, y)| (8.0 + (x - 8.0) * 0.45, y)).collect::<Vec<_>>();
            pen.line(&meridian(-90.0, 90.0));
            pen.line(&meridian(90.0, 270.0));
        }
        "deleteComment" => {
            pen.bubble(1.5, 2.0, 13.0, c, false);
            pen.cross(8.0, 6.75, 2.0, red);
        }
        "prevComment" => {
            pen.bubble(1.5, 2.0, 13.0, c, false);
            pen.acc(&[(9.5, 4.5), (6.75, 6.75), (9.5, 9.0)]);
        }
        "nextComment" => {
            pen.bubble(1.5, 2.0, 13.0, c, false);
            pen.acc(&[(6.5, 4.5), (9.25, 6.75), (6.5, 9.0)]);
        }
        // Show comments: text with a comment balloon in the margin.
        "showComments" => {
            pen.rows(1.5, 7.5, &[3.5, 6.5, 9.5, 12.5]);
            pen.bubble(10.0, 2.0, 4.5, a, false);
        }
        "resolve" => {
            pen.bubble(1.5, 2.0, 13.0, c, false);
            pen.check(8.0, 6.75, 1.0, green);
        }
        // Track changes as a diff: a removed line in red, an added line in green, each with its
        // sign in the margin.
        "trackChanges" => {
            pen.rows(5.75, 14.5, &[3.5]);
            pen.path(&[(1.25, 6.5), (3.25, 6.5)], red);
            pen.path(&[(5.75, 6.5), (12.0, 6.5)], red);
            pen.path(&[(1.25, 9.5), (3.25, 9.5)], green);
            pen.path(&[(2.25, 8.6), (2.25, 10.4)], green);
            pen.path(&[(5.75, 9.5), (13.5, 9.5)], green);
            pen.rows(5.75, 11.0, &[12.5]);
        }
        "markup" => {
            pen.rows(1.5, 8.25, &[3.5, 6.5, 9.5, 12.5]);
            pen.block(10.5, 2.0, 14.5, 8.0, 1.0, a);
        }
        "reviewingPane" => {
            pen.window(1.5, 1.5, 14.5, 14.5);
            pen.line(&[(4.0, 6.75), (12.0, 6.75)]);
            pen.line(&[(1.5, 9.5), (14.5, 9.5)]);
            pen.acc(&[(4.0, 12.0), (9.0, 12.0)]);
        }
        "accept" => {
            pen.disc(8.0, 8.0, 6.5);
            pen.check(8.0, 8.25, 1.15, green);
        }
        "reject" => {
            pen.disc(8.0, 8.0, 6.5);
            pen.cross(8.0, 8.0, 2.4, red);
        }
        "prevChange" => {
            pen.rows(7.0, 14.5, &[3.0, 8.0, 13.0]);
            pen.acc(&[(4.5, 3.0), (1.5, 8.0), (4.5, 13.0)]);
        }
        "nextChange" => {
            pen.rows(1.5, 9.0, &[3.0, 8.0, 13.0]);
            pen.acc(&[(11.5, 3.0), (14.5, 8.0), (11.5, 13.0)]);
        }
        "compare" => {
            let x1 = pen.page_h(1.25, 4.5, 6.75);
            pen.page_h(x1 + 2.25, 4.5, 6.75);
            pen.dot(1.25 + 2.85, 8.5, 0.75, c);
            pen.dot(x1 + 2.25 + 2.85, 8.5, 0.75, a);
        }
        "combine" => {
            let w = 5.5 * PAGE_RATIO;
            pen.page_h(1.0, 1.0, 5.5);
            pen.page_h(15.0 - w, 1.0, 5.5);
            let (l, r) = (1.0 + w / 2.0, 15.0 - w / 2.0);
            pen.acc(&[(l, 8.75), (l, 10.5), (r, 10.5), (r, 8.75)]);
            pen.arrow(8.0, 10.5, 8.0, 14.75, a);
        }
        "protect" => pen.lock(8.0, 14.5, 9.0, c),
        "restrict" => {
            pen.page_h(1.5, 1.5, 8.0);
            pen.lock(12.25, 14.5, 4.5, a);
        }
        "blockAuthors" => {
            pen.person(4.0, 14.5, 0.85, c);
            pen.lock(12.25, 14.5, 4.5, a);
        }
        "hideInk" => {
            pen.acc(&[(2.0, 11.0), (4.5, 6.0), (7.0, 11.0), (9.5, 6.0), (12.0, 11.0)]);
            pen.acc(&[(2.0, 2.0), (14.0, 14.0)]);
        }
        // View
        "readMode" => {
            pen.fill(&[(1.5, 3.0), (6.75, 4.0), (6.75, 14.0), (1.5, 13.0)], pen.t);
            pen.poly(&[(1.5, 3.0), (6.75, 4.0), (6.75, 14.0), (1.5, 13.0)], c);
            pen.poly(&[(14.5, 3.0), (9.25, 4.0), (9.25, 14.0), (14.5, 13.0)], a);
        }
        "printLayout" => {
            pen.page_h(2.5, 1.5, 13.0);
            pen.page_rows(2.5, 1.5, 13.0, &[0, 1, 2, 3]);
        }
        // Web layout: a browser window with a wide banner and a line of text.
        "webLayout" => {
            pen.window(1.5, 1.5, 14.5, 14.5);
            pen.block(4.0, 6.5, 12.0, 9.0, 0.5, a);
            pen.line(&[(4.0, 11.75), (12.0, 11.75)]);
        }
        "outline" => {
            for (x, y) in [(2.5, 3.0), (5.5, 8.0), (5.5, 13.0)] {
                pen.dot(x, y, 1.1, a);
                pen.line(&[(x + 3.25, y), (14.5, y)]);
            }
        }
        "draft" => {
            pen.rows(1.5, 14.5, &[3.5, 6.5, 9.5]);
            pen.rows(1.5, 8.5, &[12.5]);
            pen.acc(&[(11.0, 12.5), (14.5, 12.5)]);
        }
        "focus" => {
            pen.line(&[(1.5, 5.0), (1.5, 1.5), (5.0, 1.5)]);
            pen.line(&[(11.0, 1.5), (14.5, 1.5), (14.5, 5.0)]);
            pen.line(&[(14.5, 11.0), (14.5, 14.5), (11.0, 14.5)]);
            pen.line(&[(5.0, 14.5), (1.5, 14.5), (1.5, 11.0)]);
            pen.acc(&[(5.0, 6.5), (11.0, 6.5)]);
            pen.rows(5.0, 9.5, &[9.5]);
        }
        "immersive" => {
            pen.fill(&[(1.5, 3.0), (6.75, 4.0), (6.75, 14.0), (1.5, 13.0)], pen.t);
            pen.poly(&[(1.5, 3.0), (6.75, 4.0), (6.75, 14.0), (1.5, 13.0)], c);
            pen.poly(&[(14.5, 3.0), (9.25, 4.0), (9.25, 14.0), (14.5, 13.0)], c);
            pen.dot(11.875, 8.75, 0.9, a);
        }
        "vertical" => {
            pen.page_h(2.75, 1.125, 5.75);
            pen.page_h(2.75, 9.125, 5.75);
            pen.arrow(12.25, 2.0, 12.25, 14.0, a);
        }
        "sideToSide" => {
            let x1 = pen.page_h(1.25, 1.5, 6.75);
            pen.page_h(x1 + 2.25, 1.5, 6.75);
            pen.span(3.0, 12.0, 13.0, 12.0, a);
        }
        "ruler" => {
            pen.fill(&[(4.0, 1.5), (7.0, 1.5), (5.5, 3.5)], a);
            pen.panel(1.5, 5.5, 14.5, 11.5, 1.0);
            for (i, x) in [4.0, 6.5, 9.0, 11.5].iter().enumerate() {
                pen.line(&[(*x, 5.5), (*x, if i % 2 == 0 { 8.75 } else { 7.75 })]);
            }
        }
        "gridlines" => {
            pen.block(5.75, 5.75, 10.25, 10.25, 0.0, pen.a.gamma_multiply(0.5));
            pen.rect(1.5, 1.5, 14.5, 14.5, 1.0);
            for k in [5.75, 10.25] {
                pen.line(&[(k, 1.5), (k, 14.5)]);
                pen.line(&[(1.5, k), (14.5, k)]);
            }
        }
        "navPane" => {
            pen.window(1.5, 1.5, 14.5, 14.5);
            pen.line(&[(6.75, 4.0), (6.75, 14.5)]);
            pen.dot(4.0, 7.0, 0.7, a);
            pen.dot(4.0, 9.5, 0.7, c);
            pen.dot(4.0, 12.0, 0.7, c);
            pen.rows(9.25, 12.0, &[7.0, 9.5, 12.0]);
        }
        "zoom" => {
            pen.lens(6.5, 6.5, 4.5, c);
            pen.dot(6.5, 6.5, 1.5, a);
        }
        "zoomIn" => {
            pen.lens(6.5, 6.5, 4.5, c);
            pen.plus(6.5, 6.5, a);
        }
        "zoomOut" => {
            pen.lens(6.5, 6.5, 4.5, c);
            pen.acc(&[(4.5, 6.5), (8.5, 6.5)]);
        }
        "zoom100" => {
            pen.text(8.0, 6.5, 5.5, "100", a, true);
            pen.line(&[(2.5, 11.5), (13.5, 11.5)]);
        }
        "onePage" => {
            pen.page_h(4.4, 3.75, 8.5);
            for pts in [
                [(1.5, 3.75), (1.5, 1.5), (3.75, 1.5)],
                [(12.25, 1.5), (14.5, 1.5), (14.5, 3.75)],
                [(14.5, 12.25), (14.5, 14.5), (12.25, 14.5)],
                [(3.75, 14.5), (1.5, 14.5), (1.5, 12.25)],
            ] {
                pen.acc(&pts);
            }
        }
        "multiplePages" => {
            for (x, y) in [(2.0, 1.125), (9.12, 1.125), (2.0, 9.125), (9.12, 9.125)] {
                pen.page_h(x, y, 5.75);
            }
            pen.dot(4.4, 4.0, 0.8, a);
        }
        "pageWidth" => {
            let x1 = pen.page_h(4.4, 1.5, 8.5);
            pen.span(4.4, 13.75, x1, 13.75, a);
        }
        "darkMode" => {
            pen.circle(8.0, 8.0, 6.0);
            pen.fill(&Pen::arc_pts(8.0, 8.0, 6.0, 90.0, 270.0), c);
        }
        "interfaceTheme" => {
            pen.window(1.5, 1.5, 14.5, 14.5);
            pen.block(8.0, 4.0, 14.5, 14.5, 1.0, c);
            pen.acc(&[(3.75, 7.0), (5.75, 7.0)]);
        }
        "newWindow" => {
            pen.window(1.5, 5.5, 10.5, 14.5);
            pen.plus(12.5, 3.0, a);
        }
        "arrangeAll" => {
            pen.window(1.5, 1.5, 14.5, 6.75);
            pen.window(1.5, 9.25, 14.5, 14.5);
        }
        "split" => {
            pen.window(1.5, 1.5, 14.5, 14.5);
            pen.block(3.5, 8.25, 12.5, 9.75, 0.75, a);
        }
        "sideBySide" => {
            pen.window(1.5, 2.5, 6.75, 13.5);
            pen.window(9.25, 2.5, 14.5, 13.5);
        }
        // Synchronous scrolling: two windows whose scroll positions match.
        "syncScroll" => {
            pen.window(1.5, 2.5, 6.75, 13.5);
            pen.window(9.25, 2.5, 14.5, 13.5);
            pen.acc(&[(4.25, 7.25), (4.25, 10.0)]);
            pen.acc(&[(11.75, 7.25), (11.75, 10.0)]);
        }
        "switchWindows" => {
            pen.window(1.5, 1.5, 9.0, 7.0);
            pen.window(7.0, 9.25, 14.5, 14.75);
            pen.acc(&[(11.5, 2.5), (13.0, 2.5), (13.0, 6.25)]);
            pen.head(13.0, 7.0, 90.0, a);
            pen.acc(&[(4.5, 13.75), (3.0, 13.75), (3.0, 10.25)]);
            pen.head(3.0, 9.5, -90.0, a);
        }
        "macros" => {
            pen.panel(1.5, 1.5, 14.5, 14.5, 1.5);
            pen.fill(&[(6.0, 5.0), (11.0, 8.0), (6.0, 11.0)], a);
        }
        "info" => {
            pen.disc(8.0, 8.0, 6.5);
            pen.acc(&[(8.0, 7.75), (8.0, 11.5)]);
            pen.dot(8.0, 4.75, 1.0, a);
        }
        "properties" => {
            pen.line(&[(1.5, 5.0), (14.5, 5.0)]);
            pen.line(&[(1.5, 11.0), (14.5, 11.0)]);
            pen.disc(5.0, 5.0, 2.0);
            pen.disc(11.0, 11.0, 2.0);
        }
        // Table tools
        "insertAbove" => {
            pen.grid(1.5, 7.5, 14.5, 14.5, &[8.0], &[]);
            pen.plus(8.0, 3.25, a);
        }
        "insertBelow" => {
            pen.grid(1.5, 1.5, 14.5, 8.5, &[8.0], &[]);
            pen.plus(8.0, 12.75, a);
        }
        "insertLeft" => {
            pen.grid(7.5, 1.5, 14.5, 14.5, &[], &[8.0]);
            pen.plus(3.25, 8.0, a);
        }
        "insertRight" => {
            pen.grid(1.5, 1.5, 8.5, 14.5, &[], &[8.0]);
            pen.plus(12.75, 8.0, a);
        }
        "deleteTable" => {
            pen.table(1.5, 1.5, 8.0);
            pen.cross(12.5, 12.5, 1.75, red);
        }
        "merge" => {
            pen.panel(1.5, 3.5, 14.5, 12.5, 1.0);
            pen.arrow(3.75, 8.0, 6.75, 8.0, a);
            pen.arrow(12.25, 8.0, 9.25, 8.0, a);
        }
        "splitCells" => {
            pen.panel(1.5, 3.5, 14.5, 12.5, 1.0);
            pen.acc(&[(8.0, 5.75), (8.0, 10.25)]);
            pen.head(4.0, 8.0, 180.0, c);
            pen.head(12.0, 8.0, 0.0, c);
        }
        "splitTable" => {
            pen.grid(1.5, 1.5, 14.5, 6.0, &[8.0], &[]);
            pen.grid(1.5, 10.0, 14.5, 14.5, &[8.0], &[]);
            pen.acc(&[(3.5, 8.0), (12.5, 8.0)]);
        }
        "autofit" => {
            pen.grid(1.5, 2.5, 14.5, 13.5, &[8.0], &[8.0]);
            for (x, y) in [(3.75, 5.25), (10.25, 5.25), (3.75, 10.75), (10.25, 10.75)] {
                pen.acc(&[(x, y), (x + 2.0, y)]);
            }
        }
        "distributeRows" => {
            pen.grid(6.0, 1.5, 14.5, 14.5, &[], &[5.83, 10.17]);
            pen.span(2.75, 2.0, 2.75, 14.0, a);
        }
        "distributeCols" => {
            pen.grid(1.5, 6.0, 14.5, 14.5, &[5.83, 10.17], &[]);
            pen.span(2.0, 2.75, 14.0, 2.75, a);
        }
        "rowHeight" => {
            pen.panel(6.0, 4.5, 14.5, 11.5, 1.0);
            pen.span(2.75, 4.5, 2.75, 11.5, a);
        }
        "colWidth" => {
            pen.panel(4.5, 6.0, 11.5, 14.5, 1.0);
            pen.span(4.5, 2.75, 11.5, 2.75, a);
        }
        "cellAlign" => {
            pen.panel(1.5, 1.5, 14.5, 14.5, 1.0);
            pen.acc(&[(5.0, 6.75), (11.0, 6.75)]);
            pen.acc(&[(6.25, 9.25), (9.75, 9.25)]);
            pen.line(&[(8.0, 1.5), (8.0, 3.0)]);
            pen.line(&[(8.0, 13.0), (8.0, 14.5)]);
            pen.line(&[(1.5, 8.0), (3.0, 8.0)]);
            pen.line(&[(13.0, 8.0), (14.5, 8.0)]);
        }
        "textDirection" => {
            pen.glyph_a(5.5, 13.0, 10.0, c, c);
            pen.arrow(13.0, 2.5, 13.0, 13.5, a);
        }
        "cellMargins" => {
            pen.panel(1.5, 1.5, 14.5, 14.5, 1.0);
            for (x0, y0, x1, y1) in [(6.0, 4.5, 10.0, 4.5), (6.0, 11.5, 10.0, 11.5), (4.5, 7.0, 4.5, 9.0), (11.5, 7.0, 11.5, 9.0)] {
                pen.acc(&[(x0, y0), (x1, y1)]);
            }
        }
        "repeatHeader" => {
            pen.block(1.5, 1.5, 14.5, 5.5, 1.0, a);
            pen.rect(1.5, 7.5, 14.5, 14.5, 1.0);
            pen.line(&[(1.5, 11.0), (14.5, 11.0)]);
            pen.line(&[(8.0, 7.5), (8.0, 14.5)]);
        }
        "toText" => {
            pen.table(1.5, 1.5, 7.0);
            pen.arrow(10.0, 9.5, 11.75, 11.25, a);
            pen.rows(1.5, 14.5, &[13.75]);
        }
        "formula" => {
            pen.arc(8.5, 4.0, 2.0, 0.0, -180.0, c);
            pen.line(&[(6.5, 4.0), (6.5, 12.0), (5.5, 13.5), (3.5, 13.5)]);
            pen.line(&[(4.5, 7.0), (8.5, 7.0)]);
            pen.acc(&[(10.5, 8.0), (14.5, 13.0)]);
            pen.acc(&[(14.5, 8.0), (10.5, 13.0)]);
        }
        "selectTable" => {
            pen.table(1.5, 1.5, 8.0);
            pen.fill(&[(11.0, 11.0), (15.0, 12.75), (12.75, 13.25)], a);
            pen.fill(&[(11.0, 11.0), (12.75, 13.25), (12.25, 15.0)], a);
        }
        // Draw Table: the Table icon's table with a pencil below its corner, clear of it (the same
        // pairing as Edit Recipients).
        "draw" | "drawTable" => {
            pen.table(1.0, 1.0, 9.5);
            pen.pencil(8.25, 8.0, 0.48, a);
        }
        "borderPainter" => {
            pen.acc(&[(1.5, 1.5), (8.5, 1.5)]);
            pen.line(&[(1.5, 4.0), (1.5, 7.0)]);
            pen.pencil(2.25, 2.0, 0.8, a);
        }
        // Chrome
        // Save: into a tray, not a floppy disk.
        "save" => {
            pen.line(&[(1.5, 9.5), (1.5, 13.5), (2.5, 14.5), (13.5, 14.5), (14.5, 13.5), (14.5, 9.5)]);
            pen.arrow(8.0, 1.5, 8.0, 10.5, a);
        }
        "undo" => {
            pen.line(&[(3.0, 6.5), (10.0, 6.5)]);
            pen.arc(10.0, 10.0, 3.5, -90.0, 90.0, c);
            pen.line(&[(10.0, 13.5), (6.0, 13.5)]);
            pen.head(2.5, 6.5, 180.0, c);
        }
        "redo" => {
            pen.line(&[(13.0, 6.5), (6.0, 6.5)]);
            pen.arc(6.0, 10.0, 3.5, -90.0, -270.0, c);
            pen.line(&[(6.0, 13.5), (10.0, 13.5)]);
            pen.head(13.5, 6.5, 0.0, c);
        }
        "share" => {
            pen.line(&[
                (5.0, 6.5),
                (3.0, 6.5),
                (2.0, 7.5),
                (2.0, 13.5),
                (3.0, 14.5),
                (13.0, 14.5),
                (14.0, 13.5),
                (14.0, 7.5),
                (13.0, 6.5),
                (11.0, 6.5),
            ]);
            pen.arrow(8.0, 10.5, 8.0, 1.5, a);
        }
        "user" => pen.person_filled(8.0, 15.0, 1.5, a),
        "close" => {
            pen.line(&[(3.5, 3.5), (12.5, 12.5)]);
            pen.line(&[(12.5, 3.5), (3.5, 12.5)]);
        }
        "dropdown" => pen.line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
        "more" => {
            for x in [3.0, 8.0, 13.0] {
                pen.dot(x, 8.0, 1.25, c);
            }
        }
        "pin" => {
            pen.fill(&[(5.5, 1.5), (10.5, 1.5), (10.5, 8.5), (5.5, 8.5)], pen.t);
            pen.line(&[(5.5, 8.5), (5.5, 1.5), (10.5, 1.5), (10.5, 8.5)]);
            pen.line(&[(3.0, 9.0), (13.0, 9.0)]);
            pen.line(&[(8.0, 9.0), (8.0, 15.0)]);
        }
        "plus" => {
            pen.line(&[(8.0, 3.0), (8.0, 13.0)]);
            pen.line(&[(3.0, 8.0), (13.0, 8.0)]);
        }
        "minus" => pen.line(&[(3.0, 8.0), (13.0, 8.0)]),
        "check" => pen.line(&[(3.0, 8.5), (6.5, 12.0), (13.0, 4.0)]),
        "chevronLeft" => pen.line(&[(10.0, 3.0), (5.0, 8.0), (10.0, 13.0)]),
        "chevronRight" => pen.line(&[(6.0, 3.0), (11.0, 8.0), (6.0, 13.0)]),
        "chevronUp" => pen.line(&[(3.0, 10.5), (8.0, 5.5), (13.0, 10.5)]),
        "chevronDown" => pen.line(&[(3.0, 5.5), (8.0, 10.5), (13.0, 5.5)]),
        "chevronDoubleRight" => {
            pen.line(&[(3.0, 3.0), (8.0, 8.0), (3.0, 13.0)]);
            pen.line(&[(8.5, 3.0), (13.5, 8.0), (8.5, 13.0)]);
        }
        // Read Aloud player
        "mediaPlay" => pen.fill(&[(4.5, 2.5), (13.0, 8.0), (4.5, 13.5)], c),
        "mediaPause" => {
            pen.block(3.5, 2.5, 6.5, 13.5, 0.75, c);
            pen.block(9.5, 2.5, 12.5, 13.5, 0.75, c);
        }
        "mediaStop" => pen.block(3.0, 3.0, 13.0, 13.0, 1.0, c),
        "mediaPrev" => {
            pen.block(2.5, 3.0, 4.5, 13.0, 0.5, c);
            pen.fill(&[(13.0, 3.0), (6.0, 8.0), (13.0, 13.0)], c);
        }
        "mediaNext" => {
            pen.block(11.5, 3.0, 13.5, 13.0, 0.5, c);
            pen.fill(&[(3.0, 3.0), (10.0, 8.0), (3.0, 13.0)], c);
        }
        "pastCitation" => {
            pen.line(&[(3.0, 7.0), (1.5, 7.0), (1.5, 14.0), (3.0, 14.0)]);
            pen.line(&[(8.0, 7.0), (9.5, 7.0), (9.5, 14.0), (8.0, 14.0)]);
            pen.rows(4.25, 6.75, &[10.5]);
            pen.arc(7.5, 6.75, 5.0, 210.0, 320.0, a);
            pen.head(11.3, 3.5, 75.0, a);
            pen.acc(&[(13.5, 8.5), (13.5, 14.5)]);
        }
        "addins" => {
            pen.panel(1.5, 1.5, 6.5, 6.5, 1.0);
            pen.panel(9.5, 1.5, 14.5, 6.5, 1.0);
            pen.panel(1.5, 9.5, 6.5, 14.5, 1.0);
            pen.plus(12.0, 12.0, a);
        }
        "discord" => {
            pen.bubble(1.25, 1.25, 6.5, c, false);
            pen.bubble(8.25, 8.25, 6.5, a, true);
        }
        "help" => {
            pen.disc(8.0, 8.0, 6.5);
            pen.arc(8.0, 6.3, 2.0, 190.0, 400.0, a);
            pen.acc(&[(9.3, 7.8), (8.0, 8.9), (8.0, 9.25)]);
            pen.dot(8.0, 11.9, 0.9, a);
        }
        _ => {
            for (x0, y0, x1, y1) in [(2.0, 2.0, 5.0, 2.0), (11.0, 2.0, 14.0, 2.0), (2.0, 14.0, 5.0, 14.0), (11.0, 14.0, 14.0, 14.0)] {
                pen.line(&[(x0, y0), (x1, y1)]);
            }
            pen.line(&[(2.0, 2.0), (2.0, 14.0)]);
            pen.line(&[(14.0, 2.0), (14.0, 14.0)]);
        }
    }
}

/// A dashed accent line across the middle (a break between two pages).
fn dashes(pen: &Pen, y: f32) {
    for (x0, x1) in [(1.5, 3.5), (7.0, 9.0), (12.5, 14.5)] {
        pen.acc(&[(x0, y), (x1, y)]);
    }
}

/// Field brackets across the icon from `y0` to `y1` (mail merge fields).
fn brackets(pen: &Pen, y0: f32, y1: f32) {
    pen.brackets(1.5, 14.5, y0, y1);
}

/// The colour bar under the colour commands, in the colour that will be applied. A very light
/// colour gets an ink outline so it stays visible on light grounds.
fn color_bar(pen: &Pen, col: Color32) {
    pen.block(1.5, 12.5, 14.5, 15.0, 0.75, col);
    let [r, g, b, _] = col.to_srgba_unmultiplied();
    let light = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b) > 215.0;
    if light {
        pen.p.rect_stroke(pen.frame(1.5, 12.5, 14.5, 15.0), pen.radius(0.75), Stroke::new(1.0, pen.c.gamma_multiply(0.6)), egui::StrokeKind::Outside);
    }
}

/// Every icon name the interface draws, for tests and the icon catalogue.
pub const NAMES: &[&str] = &[
    "paste",
    "cut",
    "copy",
    "painter",
    "bold",
    "italic",
    "underline",
    "strike",
    "charborder",
    "subscript",
    "superscript",
    "grow",
    "shrink",
    "case",
    "clear",
    "highlight",
    "fontcolor",
    "effects",
    "launcher",
    "bullets",
    "numbering",
    "multilevel",
    "indent",
    "outdent",
    "sort",
    "pilcrow",
    "alignLeft",
    "alignCenter",
    "alignRight",
    "justify",
    "textLtr",
    "textRtl",
    "lineSpacing",
    "shading",
    "borders",
    "find",
    "search",
    "replace",
    "select",
    "dictate",
    "editor",
    "spelling",
    "styles",
    "stylesPane",
    "coverPage",
    "blankPage",
    "pageBreak",
    "table",
    "picture",
    "onlinePicture",
    "shapes",
    "shapeEffects",
    "icons",
    "models3d",
    "smartArt",
    "chart",
    "screenshot",
    "video",
    "link",
    "bookmark",
    "crossRef",
    "comment",
    "newComment",
    "header",
    "footer",
    "pageNumber",
    "textBox",
    "quickParts",
    "wordArt",
    "dropCap",
    "signature",
    "dateTime",
    "object",
    "equation",
    "symbol",
    "pen",
    "pencil",
    "eraser",
    "thickness",
    "lasso",
    "inkToShape",
    "inkToMath",
    "canvas",
    "replay",
    "themes",
    "colors",
    "fonts",
    "paraSpacing",
    "effectsDesign",
    "setDefault",
    "watermark",
    "pageColor",
    "pageBorders",
    "margins",
    "orientation",
    "size",
    "columns",
    "breaks",
    "lineNumbers",
    "hyphenation",
    "position",
    "wrapText",
    "bringForward",
    "sendBackward",
    "selectionPane",
    "align",
    "group",
    "rotate",
    "indentLeft",
    "indentRight",
    "spaceBefore",
    "spaceAfter",
    "toc",
    "addText",
    "update",
    "updateTable",
    "footnote",
    "endnote",
    "nextFootnote",
    "showNotes",
    "researcher",
    "docPrefs",
    "unlinkCitations",
    "addNote",
    "citation",
    "sources",
    "bibliography",
    "caption",
    "tableOfFigures",
    "markEntry",
    "index",
    "markCitation",
    "tableOfAuthorities",
    "envelope",
    "labels",
    "mailMerge",
    "recipients",
    "editRecipients",
    "mergeField",
    "addressBlock",
    "greetingLine",
    "rules",
    "matchFields",
    "highlightFields",
    "preview",
    "next",
    "previous",
    "first",
    "last",
    "finish",
    "checkErrors",
    "thesaurus",
    "wordCount",
    "readAloud",
    "accessibility",
    "translate",
    "language",
    "deleteComment",
    "prevComment",
    "nextComment",
    "showComments",
    "resolve",
    "trackChanges",
    "markup",
    "reviewingPane",
    "accept",
    "reject",
    "prevChange",
    "nextChange",
    "compare",
    "combine",
    "protect",
    "restrict",
    "blockAuthors",
    "hideInk",
    "readMode",
    "printLayout",
    "webLayout",
    "outline",
    "draft",
    "focus",
    "immersive",
    "vertical",
    "sideToSide",
    "ruler",
    "gridlines",
    "navPane",
    "zoom",
    "zoomIn",
    "zoomOut",
    "zoom100",
    "onePage",
    "multiplePages",
    "pageWidth",
    "darkMode",
    "interfaceTheme",
    "newWindow",
    "arrangeAll",
    "split",
    "sideBySide",
    "syncScroll",
    "switchWindows",
    "macros",
    "info",
    "properties",
    "insertAbove",
    "insertBelow",
    "insertLeft",
    "insertRight",
    "deleteTable",
    "merge",
    "splitCells",
    "splitTable",
    "autofit",
    "distributeRows",
    "distributeCols",
    "rowHeight",
    "colWidth",
    "cellAlign",
    "textDirection",
    "cellMargins",
    "repeatHeader",
    "toText",
    "formula",
    "selectTable",
    "draw",
    "drawTable",
    "borderPainter",
    "save",
    "undo",
    "redo",
    "share",
    "user",
    "close",
    "dropdown",
    "more",
    "pin",
    "plus",
    "minus",
    "check",
    "chevronLeft",
    "chevronRight",
    "chevronUp",
    "chevronDown",
    "chevronDoubleRight",
    "mediaPlay",
    "mediaPause",
    "mediaStop",
    "mediaPrev",
    "mediaNext",
    "pastCitation",
    "addins",
    "discord",
    "help",
];

/// Names that are deliberately the same drawing as another (aliases of one command or one idea).
pub const ALIASES: &[(&str, &str)] = &[("search", "find"), ("drawTable", "draw")];

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes_of(ctx: &egui::Context, name: &str) -> String {
        let mut out = String::new();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(64.0, 64.0))), ..Default::default() };
        let mut full = ctx.run_ui(input, |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("icon")));
            paint(&painter, Rect::from_min_size(Pos2::ZERO, vec2(32.0, 32.0)), name, Color32::from_gray(66), Color32::from_rgb(59, 91, 219));
        });
        for s in std::mem::take(&mut full.shapes) {
            out.push_str(&format!("{:?}", s.shape));
        }
        full.drop_without_applying_deltas();
        out
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(false, false));
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx
    }

    /// Every command has its own drawing: two names render identically only if listed in
    /// [`ALIASES`].
    #[test]
    fn every_icon_is_distinct() {
        let ctx = context();
        let mut seen: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
        let mut dup = Vec::new();
        for &n in NAMES {
            let key = shapes_of(&ctx, n);
            if let Some(prev) = seen.get(&key) {
                if !ALIASES.iter().any(|&(x, y)| (x == n && y == *prev) || (x == *prev && y == n)) {
                    dup.push(format!("{n} = {prev}"));
                }
            } else {
                seen.insert(key, n);
            }
        }
        assert!(dup.is_empty(), "icons sharing a drawing: {dup:?}");
    }

    /// No name the interface uses falls through to the fallback tile.
    #[test]
    fn every_name_has_a_drawing() {
        let ctx = context();
        let fallback = shapes_of(&ctx, "\u{0}no such icon");
        let missing: Vec<&str> = NAMES.iter().copied().filter(|n| shapes_of(&ctx, n) == fallback).collect();
        assert!(missing.is_empty(), "icons drawn as the fallback: {missing:?}");
    }
}
