//! WordCraft's icon set, drawn in code (original artwork; no external icon assets).
//!
//! The rules (see the design system's Iconography section):
//! - A 16×16 grid. Icons are drawn at 16 px (small buttons, menus, toolbars) or 32 px (large
//!   ribbon buttons); other sizes scale.
//! - One stroke weight per drawn size ([`stroke_px`]), round caps and joins.
//! - Every part has one colour, and separate parts keep at least 1.25 units of clear space
//!   (2.25 between stroke centre lines); parts of different colours never touch. Strokes stay
//!   between 1 and 15, so nothing is clipped.
//! - Line work in the ink colour `c`, one meaningful detail in the accent `a`, and an optional soft
//!   accent tint inside the main shape. Status colours (green, red, orange) come from the theme.
//! - Letters only where the command is about letters.
//! - Every command has its own drawing; only true aliases share an arm.
//!
//! Disabled icons pass the same colour as ink and accent; then every colour, the status colours
//! and the tint included, follows the ink.

use egui::{Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

/// Stroke width in pixels for an icon drawn `side` pixels wide: about 1.25 px at 16, 2 px at 32.
pub fn stroke_px(side: f32) -> f32 {
    (0.5 + side * 0.047).clamp(1.1, 2.4)
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
    /// An open polyline in `col`, with round caps and joins.
    fn path(&self, pts: &[(f32, f32)], col: Color32) {
        let v = self.pts(pts);
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
        for q in &v {
            self.p.circle_filled(*q, w * 0.5, self.c);
        }
        self.p.add(Shape::line(v, Stroke::new(w, self.c)));
    }
    /// A closed outline in `col`.
    fn poly(&self, pts: &[(f32, f32)], col: Color32) {
        let v = self.pts(pts);
        for q in &v {
            self.cap(*q, col);
        }
        self.p.add(Shape::closed_line(v, Stroke::new(self.w, col)));
    }
    /// A filled convex shape.
    fn fill(&self, pts: &[(f32, f32)], col: Color32) {
        self.p.add(Shape::convex_polygon(self.pts(pts), col, Stroke::NONE));
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
    fn head(&self, x: f32, y: f32, deg: f32, len: f32, col: Color32) {
        let t = deg.to_radians();
        let (l, r) = (t + 2.4, t - 2.4);
        self.path(&[(x + len * l.cos(), y + len * l.sin()), (x, y), (x + len * r.cos(), y + len * r.sin())], col);
    }
    /// A straight arrow from (x0, y0) to (x1, y1).
    fn arrow(&self, x0: f32, y0: f32, x1: f32, y1: f32, col: Color32) {
        self.path(&[(x0, y0), (x1, y1)], col);
        let deg = (y1 - y0).atan2(x1 - x0).to_degrees();
        self.head(x1, y1, deg, 2.4, col);
    }
    /// A double-headed arrow.
    fn span(&self, x0: f32, y0: f32, x1: f32, y1: f32, col: Color32) {
        self.path(&[(x0, y0), (x1, y1)], col);
        let deg = (y1 - y0).atan2(x1 - x0).to_degrees();
        self.head(x1, y1, deg, 2.0, col);
        self.head(x0, y0, deg + 180.0, 2.0, col);
    }
    /// A circular arrow (refresh / rotate) around (x, y).
    fn cycle(&self, x: f32, y: f32, r: f32, col: Color32) {
        self.arc(x, y, r, -60.0, 230.0, col);
        let (hx, hy) = (x + r * (-60f32).to_radians().cos(), y + r * (-60f32).to_radians().sin());
        self.head(hx, hy, -60.0 + 90.0 + 180.0 + 20.0, 2.2, col);
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
        self.path(&[(x - w, y), (x, y - h), (x + w, y)], col);
        self.path(&[(x - w * 0.55, y - h * 0.38), (x + w * 0.55, y - h * 0.38)], bar);
    }
    /// A small a (bowl and stem) centred at x with baseline y and x-height h.
    fn glyph_small_a(&self, x: f32, y: f32, h: f32, col: Color32) {
        let r = h * 0.5;
        self.circle_c(x - r * 0.3, y - r, r, col);
        self.path(&[(x + r * 0.7 + 0.2, y - h), (x + r * 0.7 + 0.2, y)], col);
    }
    /// A page with a cut top-right corner, tinted.
    fn page(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let k = ((x1 - x0) * 0.3).min(3.0);
        let pts = [(x0 + 1.0, y0), (x1 - k, y0), (x1, y0 + k), (x1, y1 - 1.0), (x1 - 1.0, y1), (x0 + 1.0, y1), (x0, y1 - 1.0), (x0, y0 + 1.0)];
        self.fill(&pts, self.t);
        self.poly(&pts, self.c);
    }
    /// A speech bubble with its tail at the bottom left, tinted.
    fn bubble(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let pts = [
            (x0 + 3.5, y1),
            (x0 + 1.5, y1 + 2.5),
            (x0 + 1.5, y1),
            (x0 + 1.0, y1),
            (x0, y1 - 1.0),
            (x0, y0 + 1.0),
            (x0 + 1.0, y0),
            (x1 - 1.0, y0),
            (x1, y0 + 1.0),
            (x1, y1 - 1.0),
            (x1 - 1.0, y1),
        ];
        self.block(x0, y0, x1, y1, 1.0, self.t);
        self.poly(&pts, self.c);
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
    /// A person: head and shoulders, centred on x with feet at y.
    fn person(&self, x: f32, y: f32, k: f32, col: Color32) {
        self.circle_c(x, y - 7.0 * k, 2.0 * k, col);
        self.arc(x, y, 4.0 * k, 180.0, 360.0, col);
    }
    /// A padlock with its body from (x0, y0) to (x1, y1).
    fn lock(&self, x0: f32, y0: f32, x1: f32, y1: f32, col: Color32) {
        let cx = (x0 + x1) / 2.0;
        let r = (x1 - x0) * 0.3;
        self.block(x0, y0, x1, y1, 1.0, self.t);
        self.rect_c(x0, y0, x1, y1, 1.0, col);
        self.arc(cx, y0 - r * 0.6, r, 180.0, 360.0, col);
        self.path(&[(cx - r, y0 - r * 0.6), (cx - r, y0 - 0.4)], col);
        self.path(&[(cx + r, y0 - r * 0.6), (cx + r, y0 - 0.4)], col);
    }
    /// A lens with its handle, the handle in `h`.
    fn lens(&self, x: f32, y: f32, r: f32, h: Color32) {
        self.disc(x, y, r);
        // A handle in its own colour starts clear of the rim; an ink handle joins it.
        let d = (r + if h == self.c { 0.5 } else { 2.25 }) / std::f32::consts::SQRT_2;
        self.path(&[(x + d, y + d), (14.5, 14.5)], h);
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
            pen.glyph_a(4.75, 13.0, 9.0, c, c);
            pen.glyph_small_a(12.75, 13.0, 4.0, a);
        }
        "clear" => {
            pen.glyph_a(5.0, 12.5, 9.0, c, c);
            pen.path(&[(11.25, 9.0), (14.5, 12.25)], red);
            pen.path(&[(14.5, 9.0), (11.25, 12.25)], red);
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
            pen.acc(&[(12.5, 1.75), (12.5, 6.25)]);
            pen.acc(&[(10.25, 4.0), (14.75, 4.0)]);
            pen.dot(13.5, 10.0, 1.0, a);
        }
        "fontdialog" | "launcher" => {
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
            pen.rows(1.5, 14.5, &[2.0, 14.0]);
            pen.rows(7.5, 14.5, &[6.0, 10.0]);
            pen.arrow(1.5, 8.0, 4.75, 8.0, a);
        }
        "outdent" => {
            pen.rows(1.5, 14.5, &[2.0, 14.0]);
            pen.rows(7.5, 14.5, &[6.0, 10.0]);
            pen.arrow(4.75, 8.0, 1.5, 8.0, a);
        }
        "sort" => {
            pen.rows(1.5, 8.5, &[3.0]);
            pen.rows(1.5, 6.5, &[6.5]);
            pen.rows(1.5, 4.5, &[10.0]);
            pen.rows(1.5, 2.5, &[13.5]);
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
            pen.rows(2.0, 14.0, &[2.5]);
            pen.rows(2.0, 10.0, &[6.0]);
            pen.arrow(2.0, 11.5, 14.0, 11.5, a);
        }
        "textRtl" => {
            pen.rows(2.0, 14.0, &[2.5]);
            pen.rows(6.0, 14.0, &[6.0]);
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
            pen.line(&[(1.5, 4.0), (8.5, 4.0)]);
            pen.acc(&[(7.5, 12.0), (14.5, 12.0)]);
            pen.line(&[(11.5, 4.0), (13.5, 4.0), (13.5, 7.5)]);
            pen.head(13.5, 8.25, 90.0, 1.8, c);
            pen.line(&[(4.5, 12.0), (2.5, 12.0), (2.5, 8.5)]);
            pen.head(2.5, 7.75, -90.0, 1.8, c);
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
            pen.rows(1.5, 9.5, &[3.0, 6.5]);
            pen.rows(1.5, 5.5, &[10.0]);
            pen.path(&[(8.5, 11.75), (10.5, 13.75), (14.5, 9.0)], green);
        }
        "spelling" => {
            pen.text(7.0, 4.25, 7.0, "abc", c, true);
            pen.path(&[(1.75, 11.75), (3.25, 10.75), (4.75, 11.75), (6.25, 10.75), (7.75, 11.75)], red);
            pen.path(&[(9.5, 11.5), (11.25, 13.25), (14.5, 9.25)], green);
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
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.block(5.0, 5.0, 11.0, 8.5, 0.5, a);
            pen.rows(5.0, 9.0, &[11.5]);
        }
        "blankPage" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.acc(&[(8.0, 5.5), (8.0, 10.5)]);
            pen.acc(&[(5.5, 8.0), (10.5, 8.0)]);
        }
        "pageBreak" => {
            pen.line(&[(2.5, 1.0), (2.5, 5.0), (13.5, 5.0), (13.5, 1.0)]);
            pen.line(&[(2.5, 15.0), (2.5, 11.0), (13.5, 11.0), (13.5, 15.0)]);
            dashes(pen, 8.0);
        }
        // Table: the header row is tinted, never a second line colour on the grid.
        "table" => {
            pen.block(1.5, 2.5, 14.5, 6.0, 1.0, pen.a.gamma_multiply(0.5));
            pen.grid(1.5, 2.5, 14.5, 13.5, &[6.0, 10.0], &[6.0, 9.75]);
        }
        "picture" => {
            pen.panel(1.5, 2.5, 14.5, 13.5, 1.0);
            pen.line(&[(4.0, 11.0), (7.0, 7.5), (9.5, 10.0), (10.75, 8.75), (12.0, 10.0)]);
            pen.dot(10.75, 5.5, 1.25, a);
        }
        "onlinePicture" => {
            pen.panel(1.0, 1.0, 9.0, 8.0, 1.0);
            pen.line(&[(3.5, 5.5), (5.0, 3.75), (6.5, 5.25)]);
            pen.circle_c(12.0, 12.25, 2.75, a);
            pen.acc(&[(9.25, 12.25), (14.75, 12.25)]);
            pen.acc(&[(12.0, 9.5), (11.0, 12.25), (12.0, 15.0)]);
            pen.acc(&[(12.0, 9.5), (13.0, 12.25), (12.0, 15.0)]);
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
            pen.block(6.75, 2.5, 9.25, 12.0, 0.75, a);
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
            pen.line(&[(7.0, 4.0), (8.5, 2.5), (10.0, 1.8), (11.8, 1.8), (13.5, 2.5), (14.2, 4.2), (14.2, 6.0), (13.5, 7.5), (12.0, 9.0)]);
            pen.line(&[(9.0, 12.0), (7.5, 13.5), (6.0, 14.2), (4.2, 14.2), (2.5, 13.5), (1.8, 11.8), (1.8, 10.0), (2.5, 8.5), (4.0, 7.0)]);
            pen.acc(&[(6.25, 9.75), (9.75, 6.25)]);
        }
        "bookmark" => {
            pen.fill(&[(3.5, 1.5), (12.5, 1.5), (12.5, 14.5), (8.0, 11.0)], pen.t);
            pen.fill(&[(3.5, 1.5), (8.0, 11.0), (3.5, 14.5)], pen.t);
            pen.poly(&[(3.5, 1.5), (12.5, 1.5), (12.5, 14.5), (8.0, 11.0), (3.5, 14.5)], c);
            pen.acc(&[(6.0, 5.0), (10.0, 5.0)]);
        }
        "crossRef" => {
            pen.page(1.5, 1.5, 8.5, 10.0);
            pen.acc(&[(5.0, 12.5), (5.0, 14.0), (13.5, 14.0), (13.5, 6.5)]);
            pen.head(13.5, 5.75, -90.0, 2.0, a);
        }
        "comment" => {
            pen.bubble(1.5, 2.0, 14.5, 11.5);
            pen.rows(4.5, 11.5, &[5.0, 8.0]);
        }
        "newComment" => {
            pen.block(1.5, 2.0, 8.0, 11.5, 1.0, pen.t);
            pen.line(&[(5.0, 11.5), (3.0, 14.0), (3.0, 11.5), (2.5, 11.5), (1.5, 10.5), (1.5, 3.0), (2.5, 2.0), (8.0, 2.0)]);
            pen.line(&[(14.5, 8.0), (14.5, 10.5), (13.5, 11.5), (5.0, 11.5)]);
            pen.acc(&[(12.5, 1.5), (12.5, 5.5)]);
            pen.acc(&[(10.5, 3.5), (14.5, 3.5)]);
        }
        "header" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.acc(&[(5.0, 4.5), (8.5, 4.5)]);
            pen.rows(5.0, 11.0, &[8.0, 10.5]);
        }
        "footer" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.rows(5.0, 9.5, &[5.0]);
            pen.rows(5.0, 11.0, &[7.5]);
            pen.acc(&[(5.0, 11.75), (11.0, 11.75)]);
        }
        "pageNumber" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.rows(5.0, 9.5, &[5.0]);
            pen.rows(5.0, 11.0, &[7.5]);
            pen.text(8.0, 11.0, 4.5, "1", a, true);
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
            pen.panel(1.0, 1.0, 8.5, 9.25, 1.0);
            pen.glyph_a(4.75, 7.0, 3.75, a, a);
            pen.rows(11.0, 14.5, &[2.0, 5.0, 8.0]);
            pen.rows(1.5, 14.5, &[11.75, 14.5]);
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
        "pencil" => {
            pen.fill(&[(1.5, 14.5), (2.5, 11.0), (5.0, 13.5)], orange);
            pen.fill(&[(3.9, 9.6), (10.5, 3.0), (13.0, 5.5), (6.4, 12.1)], pen.t);
            pen.poly(&[(3.9, 9.6), (10.5, 3.0), (13.0, 5.5), (6.4, 12.1)], c);
            pen.line(&[(5.3, 8.2), (7.8, 10.7)]);
        }
        "eraser" => {
            pen.fill(&[(1.5, 9.5), (4.75, 6.25), (10.25, 11.75), (8.5, 13.5), (5.5, 13.5)], pen.a.gamma_multiply(0.5));
            pen.poly(&[(1.5, 9.5), (8.0, 3.0), (13.5, 8.5), (8.5, 13.5), (5.5, 13.5)], c);
            pen.line(&[(4.75, 6.25), (10.75, 12.25)]);
            pen.line(&[(11.5, 14.5), (14.5, 14.5)]);
        }
        "lasso" => {
            pen.block(2.0, 2.0, 14.0, 9.5, 4.0, pen.t);
            pen.poly(&[(4.0, 2.5), (12.0, 2.0), (14.0, 5.5), (12.0, 9.0), (5.5, 9.5), (2.0, 6.5)], c);
            pen.acc(&[(4.5, 11.75), (3.75, 13.25), (5.25, 14.5)]);
        }
        "inkToShape" => {
            pen.acc(&[(1.0, 9.5), (2.25, 5.5), (3.75, 10.0), (5.0, 6.0)]);
            pen.arrow(6.75, 8.0, 9.0, 8.0, c);
            pen.panel(11.25, 5.25, 14.5, 10.75, 1.0);
        }
        "inkToMath" => {
            pen.acc(&[(1.0, 10.0), (2.0, 6.0), (3.0, 10.0), (4.0, 6.0)]);
            pen.arrow(5.5, 8.0, 7.75, 8.0, c);
            pen.line(&[(10.0, 8.5), (11.0, 8.0), (12.0, 11.0), (13.5, 5.0), (14.75, 5.0)]);
        }
        "canvas" => {
            pen.panel(1.5, 2.5, 14.5, 13.5, 1.0);
            pen.acc(&[(4.0, 10.5), (6.5, 6.0), (9.0, 9.5), (12.0, 5.5)]);
        }
        "replay" => {
            pen.arc(8.0, 8.0, 6.0, -150.0, 150.0, c);
            pen.head(2.8, 5.0, -100.0, 2.2, c);
            pen.fill(&[(6.5, 5.5), (11.0, 8.0), (6.5, 10.5)], a);
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
            pen.glyph_a(5.5, 13.5, 11.0, c, c);
            pen.text(12.25, 8.5, 9.0, "f", a, false);
        }
        "paraSpacing" => {
            pen.rows(7.0, 14.5, &[2.0, 4.75]);
            pen.rows(7.0, 14.5, &[11.25, 14.0]);
            pen.span(3.0, 5.5, 3.0, 10.5, a);
        }
        "effectsDesign" => {
            pen.disc(8.0, 8.0, 5.5);
            pen.arc(8.0, 8.0, 3.0, 200.0, 290.0, a);
        }
        "setDefault" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.path(&[(5.0, 8.5), (7.0, 10.5), (10.75, 6.25)], green);
        }
        "watermark" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.acc(&[(5.0, 11.5), (10.0, 6.5)]);
        }
        "pageColor" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.block(4.5, 8.5, 11.5, 12.0, 0.5, a);
        }
        "pageBorders" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.rect_c(4.75, 5.25, 11.0, 12.25, 0.5, a);
        }
        // Layout
        "margins" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.acc(&[(5.0, 4.5), (5.0, 11.75)]);
            pen.acc(&[(11.0, 6.5), (11.0, 11.75)]);
        }
        "orientation" => {
            pen.page(1.5, 1.25, 6.5, 8.25);
            pen.page(5.5, 10.5, 14.5, 14.75);
            pen.arc(9.0, 8.0, 5.0, -90.0, 0.0, a);
            pen.head(14.0, 8.25, 90.0, 1.8, a);
        }
        "size" => {
            pen.page(5.5, 1.5, 14.5, 10.0);
            pen.span(2.25, 2.0, 2.25, 10.0, a);
            pen.span(5.5, 13.25, 14.0, 13.25, a);
        }
        "columns" => {
            pen.rows(1.5, 5.5, &[3.0, 6.0, 9.0, 12.0]);
            pen.rows(10.5, 14.5, &[3.0, 6.0, 9.0]);
            pen.acc(&[(8.0, 2.0), (8.0, 14.0)]);
        }
        "breaks" => {
            pen.page(2.5, 1.0, 13.5, 5.5);
            pen.page(2.5, 10.5, 13.5, 15.0);
            dashes(pen, 8.0);
        }
        "lineNumbers" => {
            for (i, y) in [3.0, 8.0, 13.0].iter().enumerate() {
                pen.text(2.5, *y, 5.0, ["1", "2", "3"][i], a, true);
                pen.line(&[(6.0, *y), (14.5, *y)]);
            }
        }
        "hyphenation" => {
            pen.rows(1.5, 8.5, &[3.0]);
            pen.acc(&[(11.0, 3.0), (13.0, 3.0)]);
            pen.rows(1.5, 14.5, &[7.5]);
            pen.rows(1.5, 10.0, &[12.0]);
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
            pen.rows(1.5, 14.5, &[2.0, 14.0]);
            pen.rows(1.5, 4.5, &[6.0, 10.0]);
            pen.rows(11.5, 14.5, &[6.0, 10.0]);
            pen.block(6.25, 4.5, 9.75, 11.5, 1.0, a);
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
            pen.head(13.63, 6.75, 60.0, 2.0, a);
        }
        "indentLeft" => {
            pen.acc(&[(1.5, 5.5), (1.5, 10.5)]);
            pen.arrow(1.5, 8.0, 4.5, 8.0, a);
            pen.rows(7.0, 14.5, &[3.0, 8.0, 13.0]);
        }
        "indentRight" => {
            pen.acc(&[(14.5, 5.5), (14.5, 10.5)]);
            pen.arrow(14.5, 8.0, 11.5, 8.0, a);
            pen.rows(1.5, 9.0, &[3.0, 8.0, 13.0]);
        }
        "spaceBefore" => {
            pen.acc(&[(1.5, 1.5), (14.5, 1.5)]);
            pen.span(8.0, 4.0, 8.0, 7.75, a);
            pen.rows(1.5, 14.5, &[10.75, 13.75]);
        }
        "spaceAfter" => {
            pen.rows(1.5, 14.5, &[2.25, 5.25]);
            pen.span(8.0, 8.25, 8.0, 12.0, a);
            pen.acc(&[(1.5, 14.5), (14.5, 14.5)]);
        }
        // References
        "toc" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            for y in [5.5, 8.5, 11.5] {
                pen.line(&[(5.0, y), (8.0, y)]);
                pen.dot(10.75, y, 0.75, a);
            }
        }
        "addText" => {
            pen.rows(1.5, 14.5, &[3.0, 6.5]);
            pen.rows(1.5, 8.0, &[10.0]);
            pen.acc(&[(12.5, 10.5), (12.5, 14.5)]);
            pen.acc(&[(10.5, 12.5), (14.5, 12.5)]);
        }
        "update" => pen.cycle(8.0, 8.0, 5.5, a),
        "updateTable" => {
            pen.grid(1.0, 1.0, 7.5, 7.5, &[4.25], &[4.25]);
            pen.cycle(11.5, 11.5, 2.75, a);
        }
        "footnote" => {
            pen.rows(1.5, 9.0, &[3.0, 6.5]);
            pen.text(12.0, 3.0, 5.0, "1", a, true);
            pen.line(&[(1.5, 10.5), (6.0, 10.5)]);
            pen.acc(&[(1.5, 13.75), (12.0, 13.75)]);
        }
        "endnote" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.rows(5.0, 9.5, &[5.0]);
            pen.rows(5.0, 11.0, &[7.75]);
            pen.acc(&[(5.0, 11.75), (8.5, 11.75)]);
        }
        "nextFootnote" => {
            pen.rows(1.5, 8.5, &[3.0]);
            pen.text(12.0, 3.0, 5.0, "1", c, true);
            pen.arrow(2.0, 10.5, 13.0, 10.5, a);
        }
        "showNotes" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.line(&[(5.0, 6.0), (9.5, 6.0)]);
            pen.block(5.0, 9.5, 11.0, 12.0, 0.5, a);
        }
        "researcher" => {
            pen.page(1.5, 1.5, 7.5, 10.0);
            pen.circle_c(11.5, 11.5, 2.25, a);
            pen.acc(&[(13.25, 13.25), (14.5, 14.5)]);
        }
        // Zotero
        "docPrefs" => {
            pen.page(1.5, 1.5, 7.0, 12.0);
            pen.circle_c(12.0, 12.0, 1.4, a);
            for k in 0..6 {
                let t = k as f32 * std::f32::consts::PI / 3.0;
                let (dx, dy) = (t.cos(), t.sin());
                pen.acc(&[(12.0 + 2.1 * dx, 12.0 + 2.1 * dy), (12.0 + 2.7 * dx, 12.0 + 2.7 * dy)]);
            }
        }
        "unlinkCitations" => {
            pen.line(&[(5.5, 2.5), (7.0, 1.8), (8.8, 1.8), (10.5, 2.5), (11.2, 4.2), (11.2, 6.0), (10.5, 7.5), (9.0, 9.0)]);
            pen.line(&[(10.5, 13.5), (9.0, 14.2), (7.2, 14.2), (5.5, 13.5), (4.8, 11.8), (4.8, 10.0), (5.5, 8.5)]);
            pen.acc(&[(1.5, 9.5), (3.0, 9.0)]);
            pen.acc(&[(13.0, 7.0), (14.5, 6.5)]);
            pen.acc(&[(12.75, 10.25), (14.0, 11.25)]);
        }
        "addNote" => {
            pen.fill(&[(1.5, 1.5), (14.5, 1.5), (14.5, 10.5), (10.5, 14.5), (1.5, 14.5)], pen.t);
            pen.poly(&[(1.5, 1.5), (14.5, 1.5), (14.5, 10.5), (10.5, 14.5), (1.5, 14.5)], c);
            pen.line(&[(10.5, 14.5), (10.5, 10.5), (14.5, 10.5)]);
            pen.acc(&[(4.75, 7.0), (9.25, 7.0)]);
            pen.acc(&[(7.0, 4.75), (7.0, 9.25)]);
        }
        "citation" => {
            for x in [3.0, 7.5] {
                pen.dot(x, 4.0, 1.5, a);
                pen.acc(&[(x + 1.25, 4.5), (x + 0.25, 7.25)]);
            }
            pen.rows(11.0, 14.5, &[3.0, 6.5]);
            pen.rows(1.5, 14.5, &[10.5, 14.0]);
        }
        "sources" => {
            pen.panel(1.5, 2.5, 4.0, 12.25, 0.5);
            pen.panel(6.25, 2.5, 8.75, 12.25, 0.5);
            pen.poly(&[(11.0, 3.75), (12.75, 3.25), (14.5, 11.75), (12.75, 12.25)], a);
            pen.line(&[(1.0, 14.5), (15.0, 14.5)]);
        }
        "bibliography" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            for y in [4.75, 9.75] {
                pen.acc(&[(5.0, y), (10.0, y)]);
                pen.line(&[(6.75, y + 2.5), (11.0, y + 2.5)]);
            }
        }
        "caption" => {
            pen.panel(1.5, 1.5, 14.5, 10.0, 1.0);
            pen.line(&[(4.0, 7.5), (6.5, 4.5), (8.5, 6.5), (10.0, 5.0), (12.0, 7.5)]);
            pen.acc(&[(1.5, 13.5), (10.0, 13.5)]);
        }
        "tableOfFigures" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            for y in [5.25, 9.25] {
                pen.block(4.5, y - 1.25, 7.0, y + 1.25, 0.5, a);
                pen.line(&[(9.25, y), (11.0, y)]);
            }
            pen.line(&[(5.0, 12.25), (11.0, 12.25)]);
        }
        "markEntry" => {
            pen.fill(&[(1.5, 3.5), (10.0, 3.5), (14.0, 8.0), (10.0, 12.5), (1.5, 12.5)], pen.t);
            pen.poly(&[(1.5, 3.5), (10.0, 3.5), (14.0, 8.0), (10.0, 12.5), (1.5, 12.5)], c);
            pen.acc(&[(4.0, 8.0), (8.5, 8.0)]);
        }
        "index" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.glyph_a(6.25, 7.0, 3.5, a, a);
            pen.rows(5.0, 11.0, &[9.75]);
            pen.rows(6.75, 11.0, &[12.25]);
        }
        "markCitation" => {
            pen.fill(&[(1.5, 3.5), (10.0, 3.5), (14.0, 8.0), (10.0, 12.5), (1.5, 12.5)], pen.t);
            pen.poly(&[(1.5, 3.5), (10.0, 3.5), (14.0, 8.0), (10.0, 12.5), (1.5, 12.5)], c);
            pen.text(6.25, 8.0, 5.5, "\u{a7}", a, true);
        }
        "tableOfAuthorities" => {
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.text(6.25, 5.25, 5.0, "\u{a7}", a, true);
            pen.rows(5.0, 11.0, &[9.75, 12.25]);
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
            pen.page(1.5, 1.5, 8.0, 14.5);
            pen.acc(&[(4.0, 8.0), (5.5, 8.0)]);
            for y in [4.0, 8.0, 12.0] {
                pen.arrow(10.25, y, 14.0, y, a);
            }
        }
        "recipients" => {
            pen.person(5.0, 14.5, 0.9, c);
            pen.person(12.0, 10.0, 0.7, a);
        }
        "editRecipients" => {
            pen.person(5.0, 14.0, 1.0, c);
            pen.acc(&[(11.5, 14.5), (14.5, 11.5)]);
            pen.dot(11.0, 15.0, 0.5, a);
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
            pen.acc(&[(5.75, 8.0), (7.5, 8.0)]);
            pen.dot(10.0, 8.5, 0.6, a);
            pen.acc(&[(10.0, 8.5), (9.6, 9.75)]);
        }
        "rules" => {
            pen.line(&[(3.0, 1.5), (1.5, 1.5), (1.5, 6.0), (3.0, 6.0)]);
            pen.line(&[(6.0, 1.5), (7.5, 1.5), (7.5, 6.0), (6.0, 6.0)]);
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
            pen.page(1.5, 1.5, 7.0, 14.5);
            pen.path(&[(9.5, 8.5), (11.25, 10.25), (14.5, 6.25)], green);
        }
        "checkErrors" => {
            pen.page(1.5, 1.5, 7.0, 14.5);
            pen.circle_c(11.75, 8.0, 2.75, red);
            pen.path(&[(11.75, 6.6), (11.75, 8.2)], red);
            pen.dot(11.75, 9.4, 0.45, red);
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
            pen.glyph_a(4.5, 9.0, 7.5, c, c);
            pen.arrow(8.5, 4.0, 12.5, 4.0, a);
            pen.acc(&[(10.0, 9.0), (14.5, 9.0)]);
            pen.acc(&[(12.25, 9.0), (12.25, 14.5)]);
            pen.acc(&[(10.25, 11.75), (14.0, 14.25)]);
        }
        "language" => {
            pen.disc(8.0, 8.0, 6.5);
            pen.acc(&Pen::arc_pts(8.0, 8.0, 2.5, 0.0, 360.0).iter().map(|&(x, y)| (x, 8.0 + (y - 8.0) * 1.7)).collect::<Vec<_>>());
            pen.acc(&[(4.0, 8.0), (12.0, 8.0)]);
        }
        "deleteComment" => {
            pen.bubble(1.5, 2.0, 14.5, 11.5);
            pen.path(&[(6.0, 4.75), (10.0, 8.75)], red);
            pen.path(&[(10.0, 4.75), (6.0, 8.75)], red);
        }
        "prevComment" => {
            pen.bubble(1.5, 2.0, 14.5, 11.5);
            pen.acc(&[(9.5, 4.5), (6.75, 6.75), (9.5, 9.0)]);
        }
        "nextComment" => {
            pen.bubble(1.5, 2.0, 14.5, 11.5);
            pen.acc(&[(6.5, 4.5), (9.25, 6.75), (6.5, 9.0)]);
        }
        // Show comments: text with a comment balloon in the margin.
        "showComments" => {
            pen.rows(1.5, 7.5, &[3.0, 6.5, 10.0, 13.5]);
            pen.block(10.0, 2.0, 14.5, 7.0, 1.0, pen.t);
            pen.poly(
                &[(10.0, 3.0), (11.0, 2.0), (13.5, 2.0), (14.5, 3.0), (14.5, 6.0), (13.5, 7.0), (12.0, 7.0), (11.0, 8.5), (11.0, 7.0), (10.0, 6.0)],
                a,
            );
        }
        "resolve" => {
            pen.bubble(1.5, 2.0, 14.5, 11.5);
            pen.path(&[(5.25, 6.75), (7.25, 8.75), (10.75, 4.75)], green);
        }
        // Track changes as a diff: a removed line in red, an added line in green, each with its
        // sign in the margin.
        "trackChanges" => {
            pen.rows(5.75, 14.5, &[2.0]);
            pen.path(&[(1.25, 6.0), (3.25, 6.0)], red);
            pen.path(&[(5.75, 6.0), (12.0, 6.0)], red);
            pen.path(&[(1.25, 10.0), (3.25, 10.0)], green);
            pen.path(&[(2.25, 9.0), (2.25, 11.0)], green);
            pen.path(&[(5.75, 10.0), (13.5, 10.0)], green);
            pen.rows(5.75, 11.0, &[14.0]);
        }
        "markup" => {
            pen.rows(1.5, 8.25, &[3.0, 6.5, 10.0, 13.5]);
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
            pen.path(&[(5.0, 8.3), (7.2, 10.5), (11.2, 5.8)], green);
        }
        "reject" => {
            pen.disc(8.0, 8.0, 6.5);
            pen.path(&[(5.6, 5.6), (10.4, 10.4)], red);
            pen.path(&[(10.4, 5.6), (5.6, 10.4)], red);
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
            pen.page(1.5, 2.5, 6.5, 13.5);
            pen.page(9.5, 2.5, 14.5, 13.5);
            pen.dot(4.0, 9.0, 0.75, c);
            pen.dot(12.0, 9.0, 0.75, a);
        }
        "combine" => {
            pen.page(1.0, 1.0, 6.5, 6.5);
            pen.page(9.5, 1.0, 15.0, 6.5);
            pen.acc(&[(3.75, 8.75), (3.75, 10.5), (12.25, 10.5), (12.25, 8.75)]);
            pen.arrow(8.0, 10.5, 8.0, 15.0, a);
        }
        "protect" => pen.lock(3.0, 7.0, 13.0, 14.5, c),
        "restrict" => {
            pen.page(1.5, 1.5, 7.5, 14.5);
            pen.lock(10.0, 10.25, 14.5, 14.5, a);
        }
        "blockAuthors" => {
            pen.person(4.75, 14.5, 0.9, c);
            pen.lock(10.25, 10.25, 14.5, 14.5, a);
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
            pen.page(2.5, 1.5, 13.5, 14.5);
            pen.rows(5.0, 9.5, &[5.5]);
            pen.rows(5.0, 11.0, &[8.5, 11.5]);
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
            pen.rows(1.5, 14.5, &[2.5, 6.0, 9.5]);
            pen.rows(1.5, 8.5, &[13.0]);
            pen.acc(&[(11.0, 13.0), (14.5, 13.0)]);
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
            pen.page(3.5, 1.25, 12.5, 6.75);
            pen.page(3.5, 9.25, 12.5, 14.75);
        }
        "sideToSide" => {
            pen.page(1.5, 1.5, 6.75, 10.0);
            pen.page(9.25, 1.5, 14.5, 10.0);
            pen.span(3.0, 13.75, 13.0, 13.75, a);
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
            pen.acc(&[(4.5, 6.5), (8.5, 6.5)]);
            pen.acc(&[(6.5, 4.5), (6.5, 8.5)]);
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
            pen.page(3.5, 1.5, 12.5, 14.5);
            pen.rows(6.0, 9.5, &[5.5]);
            pen.rows(6.0, 10.0, &[8.5, 11.5]);
        }
        "multiplePages" => {
            for (x, y) in [(1.5, 1.5), (9.5, 1.5), (1.5, 9.5), (9.5, 9.5)] {
                pen.page(x, y, x + 5.0, y + 5.0);
            }
            pen.dot(4.0, 4.25, 0.9, a);
        }
        "pageWidth" => {
            pen.page(3.5, 1.5, 12.5, 10.0);
            pen.span(3.5, 13.75, 12.5, 13.75, a);
        }
        "darkMode" => {
            pen.circle(8.0, 8.0, 6.0);
            pen.fill(&Pen::arc_pts(8.0, 8.0, 6.0, 90.0, 270.0), c);
        }
        "interfaceTheme" => {
            pen.window(1.5, 2.0, 14.5, 14.0);
            pen.block(8.0, 4.5, 14.5, 14.0, 1.0, c);
            pen.acc(&[(3.75, 7.0), (5.75, 7.0)]);
        }
        "newWindow" => {
            pen.window(1.5, 5.0, 10.5, 14.5);
            pen.acc(&[(13.0, 1.5), (13.0, 4.5)]);
            pen.acc(&[(11.5, 3.0), (14.5, 3.0)]);
        }
        "arrangeAll" => {
            pen.window(1.5, 1.5, 14.5, 7.0);
            pen.window(1.5, 9.5, 14.5, 14.5);
        }
        "split" => {
            pen.panel(1.5, 1.5, 14.5, 14.5, 1.0);
            pen.acc(&[(4.0, 8.0), (12.0, 8.0)]);
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
            pen.window(1.5, 1.5, 9.0, 7.5);
            pen.window(7.0, 9.75, 14.5, 14.5);
            pen.acc(&[(11.5, 2.5), (13.0, 2.5), (13.0, 6.25)]);
            pen.head(13.0, 7.0, 90.0, 1.6, a);
            pen.acc(&[(4.5, 13.5), (3.0, 13.5), (3.0, 10.75)]);
            pen.head(3.0, 10.0, -90.0, 1.6, a);
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
            pen.acc(&[(8.0, 1.25), (8.0, 5.25)]);
            pen.acc(&[(6.0, 3.25), (10.0, 3.25)]);
        }
        "insertBelow" => {
            pen.grid(1.5, 1.5, 14.5, 8.5, &[8.0], &[]);
            pen.acc(&[(8.0, 10.75), (8.0, 14.75)]);
            pen.acc(&[(6.0, 12.75), (10.0, 12.75)]);
        }
        "insertLeft" => {
            pen.grid(7.5, 1.5, 14.5, 14.5, &[], &[8.0]);
            pen.acc(&[(1.25, 8.0), (5.25, 8.0)]);
            pen.acc(&[(3.25, 6.0), (3.25, 10.0)]);
        }
        "insertRight" => {
            pen.grid(1.5, 1.5, 8.5, 14.5, &[], &[8.0]);
            pen.acc(&[(10.75, 8.0), (14.75, 8.0)]);
            pen.acc(&[(12.75, 6.0), (12.75, 10.0)]);
        }
        "deleteTable" => {
            pen.grid(1.5, 1.5, 8.5, 8.5, &[5.0], &[5.0]);
            pen.path(&[(10.75, 10.75), (14.25, 14.25)], red);
            pen.path(&[(14.25, 10.75), (10.75, 14.25)], red);
        }
        "merge" => {
            pen.panel(1.5, 3.5, 14.5, 12.5, 1.0);
            pen.arrow(3.75, 8.0, 6.75, 8.0, a);
            pen.arrow(12.25, 8.0, 9.25, 8.0, a);
        }
        "splitCells" => {
            pen.panel(1.5, 3.5, 14.5, 12.5, 1.0);
            pen.acc(&[(8.0, 5.75), (8.0, 10.25)]);
            pen.head(4.0, 8.0, 180.0, 1.6, c);
            pen.head(12.0, 8.0, 0.0, 1.6, c);
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
            pen.acc(&[(5.0, 7.0), (11.0, 7.0)]);
            pen.rows(6.0, 10.0, &[10.0]);
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
            pen.grid(1.5, 1.5, 7.0, 7.0, &[4.25], &[4.25]);
            pen.arrow(8.75, 8.75, 10.75, 10.75, a);
            pen.rows(12.25, 14.5, &[12.25]);
            pen.rows(2.0, 14.5, &[14.75]);
        }
        "formula" => {
            pen.arc(8.5, 4.0, 2.0, 0.0, -180.0, c);
            pen.line(&[(6.5, 4.0), (6.5, 12.0), (5.5, 13.5), (3.5, 13.5)]);
            pen.line(&[(4.5, 7.0), (8.5, 7.0)]);
            pen.acc(&[(10.5, 8.0), (14.5, 13.0)]);
            pen.acc(&[(14.5, 8.0), (10.5, 13.0)]);
        }
        "selectTable" => {
            pen.grid(1.5, 1.5, 9.5, 9.5, &[5.5], &[5.5]);
            pen.fill(&[(11.0, 11.0), (15.0, 12.75), (12.75, 13.25)], a);
            pen.fill(&[(11.0, 11.0), (12.75, 13.25), (12.25, 15.0)], a);
        }
        // Draw table: a pencil drawing a grid.
        "draw" => {
            pen.grid(1.5, 1.5, 7.5, 7.5, &[4.5], &[4.5]);
            pen.fill(&[(9.25, 9.25), (11.94, 10.1), (10.1, 11.94)], a);
            pen.fill(&[(11.94, 10.1), (14.27, 12.71), (12.71, 14.27), (10.1, 11.94)], pen.t);
            pen.poly(&[(9.25, 9.25), (11.94, 10.1), (14.27, 12.71), (12.71, 14.27), (10.1, 11.94)], a);
        }
        "borderPainter" => {
            pen.acc(&[(1.5, 1.5), (9.0, 1.5)]);
            pen.line(&[(1.5, 4.0), (1.5, 7.0)]);
            pen.poly(&[(4.0, 14.5), (4.6, 12.0), (11.5, 5.1), (13.9, 7.5), (7.0, 14.4)], c);
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
            pen.head(2.5, 6.5, 180.0, 3.0, c);
        }
        "redo" => {
            pen.line(&[(13.0, 6.5), (6.0, 6.5)]);
            pen.arc(6.0, 10.0, 3.5, -90.0, -270.0, c);
            pen.line(&[(6.0, 13.5), (10.0, 13.5)]);
            pen.head(13.5, 6.5, 0.0, 3.0, c);
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
        "user" => {
            pen.p.circle_filled(pen.pt(8.0, 4.5), 2.75 * pen.s, a);
            pen.fill(&Pen::arc_pts(8.0, 15.0, 6.0, 180.0, 360.0), a);
        }
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
            pen.head(11.3, 3.5, 75.0, 1.8, a);
            pen.acc(&[(13.5, 8.5), (13.5, 14.5)]);
        }
        "addins" => {
            pen.panel(1.5, 1.5, 6.5, 6.5, 1.0);
            pen.panel(9.5, 1.5, 14.5, 6.5, 1.0);
            pen.panel(1.5, 9.5, 6.5, 14.5, 1.0);
            pen.acc(&[(12.0, 9.5), (12.0, 14.5)]);
            pen.acc(&[(9.5, 12.0), (14.5, 12.0)]);
        }
        "discord" => {
            pen.bubble(1.5, 1.5, 10.0, 8.5);
            pen.path(
                &[
                    (12.0, 5.0),
                    (13.5, 5.0),
                    (14.5, 6.0),
                    (14.5, 11.5),
                    (13.5, 12.5),
                    (13.0, 12.5),
                    (13.0, 14.5),
                    (11.0, 12.5),
                    (6.5, 12.5),
                    (5.5, 11.5),
                    (5.5, 11.0),
                ],
                a,
            );
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

/// Field brackets from `y0` to `y1` (mail merge fields).
fn brackets(pen: &Pen, y0: f32, y1: f32) {
    pen.line(&[(3.75, y0), (1.5, y0), (1.5, y1), (3.75, y1)]);
    pen.line(&[(12.25, y0), (14.5, y0), (14.5, y1), (12.25, y1)]);
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

/// Every icon name the interface draws, for tests and the design system's catalogue.
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
    "fontdialog",
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
pub const ALIASES: &[(&str, &str)] = &[("search", "find"), ("launcher", "fontdialog")];

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
