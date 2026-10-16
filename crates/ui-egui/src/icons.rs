//! WordCraft's icon set, drawn in code (original artwork; no external icon assets).
//!
//! Icons are designed on a 20×20 grid in two colours: line work (`c`) and an accent (`a`).

use egui::{Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

/// The Home ribbon family is drawn with one stroke weight and pixel-snapped axis-aligned edges:
/// its line work then matches the weight of the letter glyphs beside it and stays crisp at 1x, 1.5x
/// and 2x. Icons outside the family keep the set's original weight and unsnapped geometry.
const HOME_STROKE: f32 = 1.4;

/// The names refined as one family (2026-10-13): the Home ribbon's own icons. Shared copies of these
/// names elsewhere inherit the same art; every other name is untouched.
/// The names that get the family's uniform stroke and the optical calibration. Paste is deliberately
/// absent: it keeps its earlier original drawing (see the legacy case in `paint`).
const HOME_FAMILY: &[&str] = &[
    "cut",
    "copy",
    "painter",
    "bold",
    "italic",
    "underline",
    "strike",
    "charborder",
    "enclose",
    "subscript",
    "superscript",
    "grow",
    "shrink",
    "case",
    "clear",
    "phonetic",
    "effects",
    "highlight",
    "fontcolor",
    "bullets",
    "numbering",
    "multilevel",
    "outdent",
    "indent",
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
    "replace",
    "select",
    "dictate",
    "editor",
    "spelling",
];

/// Optical calibration for the Home family: a uniform scale and a viewport offset that bring each
/// icon's **visible ink** to the same apparent size without stretching any of them. Measured from our
/// own art at 17 pt / 2x (`optical-baseline.tsv`, `.local/ribbon-size-padding/`); the target is a
/// 13.5-point longest ink extent in the 17-point tile, so the art keeps its own aspect ratio and a
/// narrow `I`, a two-glyph `Aa` or a small script digit stays proportionally smaller than a silhouette.
/// Paste is deliberately absent: it keeps its earlier original drawing, stroke and unsnapped
/// coordinates (see the legacy case in `paint`).
const HOME_CALIBRATION: &[(&str, f32, f32, f32)] = &[
    ("alignCenter", 1.0465, -0.000, -0.523),
    ("alignLeft", 1.0465, -0.000, -0.523),
    ("alignRight", 1.0465, -0.000, -0.523),
    ("bold", 1.5000, -0.071, -0.375),
    ("borders", 1.0417, -0.000, -0.021),
    ("bullets", 1.0597, -0.085, -0.000),
    ("case", 1.1250, 0.422, -0.281),
    ("charborder", 1.0385, -0.000, -0.000),
    ("clear", 1.0680, -0.432, -1.709),
    ("copy", 1.0385, 0.012, -0.519),
    ("cut", 1.0131, -0.000, -0.215),
    ("dictate", 0.9441, -0.236, -0.142),
    ("editor", 0.9566, 1.393, -0.108),
    ("effects", 1.4324, -0.103, -0.662),
    ("enclose", 1.0465, -0.000, -0.000),
    ("find", 1.0465, -0.000, -0.000),
    ("fontcolor", 0.9926, -0.000, -1.266),
    ("grow", 1.0358, 0.147, 0.440),
    ("highlight", 1.0526, -0.000, -1.303),
    ("indent", 1.0465, -0.000, -0.000),
    ("italic", 1.3043, -0.000, -0.000),
    ("lineSpacing", 1.0131, 0.215, -0.000),
    ("multilevel", 1.0417, 0.031, -0.000),
    ("numbering", 1.0247, 0.140, -0.000),
    ("outdent", 1.0465, -0.000, -0.000),
    ("painter", 1.0150, -0.761, -0.152),
    ("phonetic", 1.0019, 0.062, -0.238),
    ("pilcrow", 1.5000, 0.141, -1.013),
    ("replace", 0.9468, 0.241, -0.166),
    ("select", 1.0588, 0.450, -0.450),
    ("shading", 1.0181, -0.260, -1.044),
    ("shrink", 1.1117, -0.337, 0.083),
    ("sort", 1.0385, -0.236, 0.260),
    ("spelling", 0.9566, 1.393, -0.108),
    ("strike", 1.0465, -0.000, -0.262),
    ("subscript", 1.2453, -0.349, -2.849),
    ("superscript", 1.1321, -0.317, 0.976),
    ("textLtr", 1.0822, 0.229, -1.244),
    ("textRtl", 1.0822, -0.231, -1.244),
    ("underline", 1.1714, -0.000, -0.279),
];

/// The tile the calibration was measured in; the scale is a ratio, the offsets are in these points.
const CALIBRATION_TILE: f32 = 17.0;

/// The calibrated art viewport for `name`: the supplied paint box scaled and recentred around its own
/// centre. Names outside the table draw in the supplied box exactly as before.
fn art_viewport(r: Rect, name: &str) -> Rect {
    let Some((_, scale, dx, dy)) = HOME_CALIBRATION.iter().find(|(n, ..)| *n == name) else {
        return r;
    };
    let k = r.width() / CALIBRATION_TILE;
    Rect::from_center_size(r.center() + vec2(dx * k, dy * k), r.size() * *scale)
}

struct Pen<'a> {
    p: &'a Painter,
    /// The art viewport: grid coordinates 0..20 map into this rect (calibrated for the Home family).
    r: Rect,
    /// The scale the stroke is measured in — the *supplied* paint box, never the calibrated viewport,
    /// so normalising an icon's size cannot thicken or thin its lines.
    w_scale: f32,
    c: Color32,
    a: Color32,
    w: f32,
    /// Snap axis-aligned edges to the device pixel grid and quantise the stroke (Home family).
    snap: bool,
}

impl Pen<'_> {
    fn s(&self) -> f32 {
        self.r.width().min(self.r.height()) / 20.0
    }
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        let s = self.s();
        let o = self.r.center() - vec2(10.0 * s, 10.0 * s);
        pos2(o.x + x * s, o.y + y * s)
    }
    fn stroke(&self, c: Color32) -> Stroke {
        Stroke::new(if self.snap { self.snapped_width() } else { self.w * self.w_scale }, c)
    }
    /// The stroke width in points, quantised to at least one device pixel so a thin line never
    /// disappears into anti-aliasing.
    fn snapped_width(&self) -> f32 {
        let ppp = self.p.ctx().pixels_per_point().max(1.0);
        ((self.w * self.w_scale * ppp).round().max(1.0)) / ppp
    }
    fn line(&self, pts: &[(f32, f32)]) {
        self.line_c(pts, self.c);
    }
    fn line_c(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        let stroke = self.stroke(c);
        let v = if self.snap { snapped_polyline(&v, stroke.width, self.p.ctx().pixels_per_point()) } else { v };
        self.p.add(Shape::line(v, stroke));
    }
    fn closed(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(Shape::closed_line(v, self.stroke(c)));
    }
    fn fill(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(Shape::convex_polygon(v, c, Stroke::NONE));
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        let r = Rect::from_min_max(self.pt(x0, y0), self.pt(x1, y1));
        let stroke = self.stroke(c);
        let r = if self.snap { snapped_rect(r, stroke.width, self.p.ctx().pixels_per_point()) } else { r };
        self.p.rect_stroke(r, 1.0, stroke, egui::StrokeKind::Middle);
    }
    fn frect(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        self.p.rect_filled(Rect::from_min_max(self.pt(x0, y0), self.pt(x1, y1)), 0.5, c);
    }
    fn circle(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_stroke(self.pt(x, y), r * self.s(), self.stroke(c));
    }
    fn fcircle(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_filled(self.pt(x, y), r * self.s(), c);
    }
    fn text(&self, x: f32, y: f32, size: f32, t: &str, c: Color32, bold: bool) {
        let font = if bold { crate::theme::semibold(size * self.s()) } else { FontId::proportional(size * self.s()) };
        self.p.text(self.pt(x, y), egui::Align2::CENTER_CENTER, t, font, c);
    }
    /// Horizontal text lines (a paragraph).
    fn lines(&self, x0: f32, x1: f32, ys: &[f32]) {
        for y in ys {
            self.line(&[(x0, *y), (x1, *y)]);
        }
    }
    /// A page outline with a folded corner.
    fn page(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.closed(&[(x0, y0), (x1 - 4.0, y0), (x1, y0 + 4.0), (x1, y1), (x0, y1)], self.c);
        self.line(&[(x1 - 4.0, y0), (x1 - 4.0, y0 + 4.0), (x1, y0 + 4.0)]);
    }
    fn arrow_down(&self, x: f32, y: f32, c: Color32) {
        self.fill(&[(x - 2.5, y - 1.2), (x + 2.5, y - 1.2), (x, y + 1.5)], c);
    }
}

/// Moves a coordinate onto the device pixel grid, or onto a pixel centre when the stroke's device
/// width is odd (a half-pixel offset is what makes a 1-pixel line cover whole pixels).
fn snap_to_pixel(v: f32, width: f32, ppp: f32) -> f32 {
    let ppp = ppp.max(1.0);
    if ((width * ppp).round() as i32) % 2 != 0 { ((v * ppp).floor() + 0.5) / ppp } else { (v * ppp).round() / ppp }
}

/// Snaps the horizontal and vertical segments of a polyline to the pixel grid; diagonal segments
/// keep their own anti-aliasing.
fn snapped_polyline(points: &[Pos2], width: f32, ppp: f32) -> Vec<Pos2> {
    let mut out = points.to_vec();
    for i in 0..out.len().saturating_sub(1) {
        let (a, b) = (out[i], out[i + 1]);
        if (a.y - b.y).abs() < 0.01 {
            let y = snap_to_pixel(a.y, width, ppp);
            out[i].y = y;
            out[i + 1].y = y;
        } else if (a.x - b.x).abs() < 0.01 {
            let x = snap_to_pixel(a.x, width, ppp);
            out[i].x = x;
            out[i + 1].x = x;
        }
    }
    out
}

/// Snaps an axis-aligned rectangle's edges the same way.
fn snapped_rect(r: Rect, width: f32, ppp: f32) -> Rect {
    Rect::from_min_max(
        pos2(snap_to_pixel(r.min.x, width, ppp), snap_to_pixel(r.min.y, width, ppp)),
        pos2(snap_to_pixel(r.max.x, width, ppp), snap_to_pixel(r.max.y, width, ppp)),
    )
}

/// Paint icon `name` into `r`. `c` is the line colour; the accent comes from the theme.
pub fn paint(p: &Painter, r: Rect, name: &str, c: Color32, accent: Color32) {
    // Paste keeps its earlier original drawing: the set's own stroke weight, no snapping, no
    // calibration — its large-button role stays distinct from the small ribbon controls.
    let legacy = name == "paste";
    let home = HOME_FAMILY.contains(&name);
    let pen = Pen {
        p,
        r: art_viewport(r, name),
        w_scale: r.width().min(r.height()) / 20.0,
        c,
        a: accent,
        w: if home { HOME_STROKE } else { 1.25 },
        snap: home && !legacy,
    };
    let a = accent;
    let red = Color32::from_rgb(0xD1, 0x3B, 0x3B);
    let yellow = Color32::from_rgb(0xF2, 0xC8, 0x11);
    let green = Color32::from_rgb(0x2E, 0x8B, 0x57);
    let orange = Color32::from_rgb(0xE0, 0x7B, 0x1F);
    match name {
        // Clipboard
        // The earlier original drawing (our own HEAD art, restored on request): a board with its clip
        // and a tinted sheet overlapping it to the lower right. It keeps the set's own 1.25-unit
        // stroke and unsnapped coordinates, and is exempt from the optical calibration.
        "paste" => {
            // Only the board's *visible* edges are drawn: the rear right and bottom edges stop where
            // the sheet begins, so no white outline shows through the translucent page (the sheet is
            // tinted, not opaque, so a full rectangle would be visible inside it).
            pen.line(&[(4.0, 3.5), (14.0, 3.5), (14.0, 7.25)]);
            pen.line(&[(4.0, 3.5), (4.0, 17.0), (8.25, 17.0)]);
            pen.frect(7.0, 2.0, 11.0, 5.0, a);
            pen.frect(9.0, 8.0, 17.5, 18.5, pen.a.linear_multiply(0.18));
            pen.rect(9.0, 8.0, 17.5, 18.5, a);
            pen.lines(10.8, 15.8, &[11.0, 13.5, 16.0]);
        }
        "cut" => {
            pen.circle(6.0, 15.0, 2.5, c);
            pen.circle(14.0, 15.0, 2.5, c);
            pen.line(&[(7.5, 13.0), (14.0, 3.0)]);
            pen.line(&[(12.5, 13.0), (6.0, 3.0)]);
        }
        "copy" => {
            // The back sheet stops where the front one starts, so no edge crosses another.
            pen.line(&[(3.5, 14.5), (3.5, 3.5), (12.5, 3.5)]);
            pen.line(&[(3.5, 14.5), (8.0, 14.5)]);
            pen.rect(8.0, 6.5, 16.5, 17.5, a);
        }
        "painter" => {
            pen.frect(4.0, 3.0, 16.0, 8.0, yellow);
            pen.rect(4.0, 3.0, 16.0, 8.0, c);
            pen.line(&[(16.0, 5.75), (17.5, 5.75), (17.5, 10.0), (10.0, 10.0), (10.0, 12.5)]);
            pen.frect(8.6, 12.5, 11.4, 18.0, c);
        }
        // Font
        "bold" => pen.text(10.0, 10.0, 15.0, "B", c, true),
        // A slanted stem with its own short bars: not an upright I.
        "italic" => {
            pen.line(&[(9.5, 4.5), (14.0, 4.5)]);
            pen.line(&[(13.0, 4.5), (8.5, 15.5)]);
            pen.line(&[(6.0, 15.5), (10.5, 15.5)]);
        }
        "underline" => {
            pen.text(10.0, 8.5, 14.0, "U", c, false);
            pen.line(&[(5.5, 16.5), (14.5, 16.5)]);
        }
        "strike" => {
            pen.text(10.0, 10.0, 12.0, "ab", c, false);
            pen.line_c(&[(3.0, 10.0), (17.0, 10.0)], c);
        }
        "charborder" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, a);
            // The letter sits in the middle of its box, not at the top.
            pen.text(10.0, 10.0, 10.0, "A", c, false);
        }
        "enclose" => {
            pen.circle(10.0, 10.0, 7.0, a);
            pen.text(10.0, 10.0, 10.0, "A", c, false);
        }
        "subscript" => {
            pen.text(7.5, 9.5, 13.0, "x", c, false);
            pen.text(14.5, 15.0, 8.0, "2", a, true);
        }
        "superscript" => {
            pen.text(7.5, 11.0, 13.0, "x", c, false);
            pen.text(14.5, 5.5, 8.0, "2", a, true);
        }
        "grow" => {
            pen.text(7.0, 11.5, 14.0, "A", c, false);
            pen.line_c(&[(15.5, 10.0), (15.5, 4.5)], a);
            pen.fill(&[(13.5, 5.5), (17.5, 5.5), (15.5, 2.5)], a);
        }
        "shrink" => {
            pen.text(7.0, 12.5, 11.0, "A", c, false);
            pen.line_c(&[(15.5, 3.5), (15.5, 9.0)], a);
            pen.fill(&[(13.5, 8.0), (17.5, 8.0), (15.5, 11.0)], a);
        }
        "case" => pen.text(10.0, 10.0, 12.0, "Aa", c, false),
        // The letter and a tilted eraser beside it, inside the grid's safe inset.
        "clear" => {
            pen.text(7.5, 8.5, 12.0, "A", c, false);
            pen.closed(&[(8.5, 13.5), (12.0, 10.0), (17.0, 15.0), (13.5, 18.5)], Color32::from_rgb(0xE8, 0x6F, 0xA0));
        }
        // Small letters over a large one: text with a reading above it.
        "phonetic" => {
            pen.text(10.0, 5.0, 7.5, "abc", a, false);
            pen.text(10.0, 13.5, 12.5, "A", c, false);
        }
        "highlight" => {
            // A marker with an outlined nib above its yellow band.
            pen.closed(&[(5.5, 11.5), (11.0, 4.5), (14.5, 8.0), (9.0, 15.0)], c);
            pen.line(&[(5.5, 11.5), (3.5, 15.0), (9.0, 15.0)]);
            pen.frect(2.5, 16.5, 17.5, 19.0, yellow);
        }
        "fontcolor" => {
            pen.text(10.0, 9.0, 14.0, "A", c, false);
            pen.frect(2.0, 16.5, 18.0, 19.0, red);
        }
        "effects" => {
            // The shadow is painted *behind* the accent letter, so the letter itself stays clean.
            pen.text(10.5, 10.5, 15.0, "A", c.linear_multiply(0.45), true);
            pen.text(10.0, 10.0, 15.0, "A", a, true);
        }
        "fontdialog" => pen.line(&[(13.0, 13.0), (17.0, 17.0), (17.0, 13.0)]),
        // Paragraph
        "bullets" => {
            // One pitch for every line, one x for every dot: the tiers stay level.
            for (i, y) in [5.0, 10.0, 15.0].iter().enumerate() {
                pen.fcircle(4.0, *y, 1.4, if i == 0 { a } else { c });
                pen.line(&[(7.0, *y), (17.0, *y)]);
            }
        }
        "numbering" => {
            for (i, y) in [5.0, 10.0, 15.0].iter().enumerate() {
                pen.text(4.0, *y, 6.0, &(i + 1).to_string(), if i == 0 { a } else { c }, true);
                pen.line(&[(7.5, *y), (17.0, *y)]);
            }
        }
        "multilevel" => {
            // Three levels on one pitch, each line starting further right than the one above.
            pen.text(3.5, 5.0, 5.5, "1", a, true);
            pen.line(&[(6.5, 5.0), (17.0, 5.0)]);
            pen.text(6.5, 10.0, 5.5, "a", c, true);
            pen.line(&[(9.5, 10.0), (17.0, 10.0)]);
            pen.text(9.5, 15.0, 5.5, "i", c, true);
            pen.line(&[(12.0, 15.0), (17.0, 15.0)]);
        }
        "outdent" | "indent" => {
            pen.lines(9.0, 17.0, &[7.0, 10.0, 13.0]);
            pen.lines(3.0, 17.0, &[3.5, 16.5]);
            if name == "indent" {
                pen.fill(&[(3.0, 7.0), (3.0, 13.0), (6.5, 10.0)], a);
            } else {
                pen.fill(&[(6.5, 7.0), (6.5, 13.0), (3.0, 10.0)], a);
            }
        }
        "sort" => {
            // The A and Z share one column; the arrow is straight and inside the safe inset.
            pen.text(5.5, 5.0, 7.0, "A", c, true);
            pen.text(5.5, 15.0, 7.0, "Z", c, true);
            pen.line_c(&[(14.5, 3.5), (14.5, 16.5)], a);
            pen.line_c(&[(12.0, 14.0), (14.5, 16.5), (17.0, 14.0)], a);
        }
        "pilcrow" => pen.text(10.0, 10.5, 15.0, "¶", c, false),
        // The four alignments share one pitch and endpoints; only the short lines' span moves, so
        // the set reads as a family and centre/right stay symmetric about the middle.
        "alignLeft" | "alignCenter" | "alignRight" | "justify" => {
            let (s0, s1) = match name {
                "alignCenter" => (6.0, 14.0),
                "alignRight" => (8.0, 17.0),
                "justify" => (3.0, 17.0),
                _ => (3.0, 12.0),
            };
            // Two long lines and two short ones: only the short lines' span follows the alignment.
            pen.lines(3.0, 17.0, &[4.5, 12.5]);
            pen.lines(s0, s1, &[8.5, 16.5]);
        }
        // Text direction: lines set from the start edge, an arrow in the reading direction.
        "textLtr" => {
            // Left-aligned lines over an arrow pointing right, all inside the safe inset.
            pen.lines(3.0, 14.0, &[4.5, 8.5]);
            pen.lines(3.0, 9.0, &[12.5]);
            pen.line_c(&[(3.0, 15.5), (16.5, 15.5)], a);
            pen.line_c(&[(13.5, 13.0), (16.5, 15.5), (13.5, 18.0)], a);
        }
        "textRtl" => {
            // The mirror image: right-aligned lines over an arrow pointing left.
            pen.lines(6.0, 17.0, &[4.5, 8.5]);
            pen.lines(11.0, 17.0, &[12.5]);
            pen.line_c(&[(17.0, 15.5), (3.5, 15.5)], a);
            pen.line_c(&[(6.5, 13.0), (3.5, 15.5), (6.5, 18.0)], a);
        }
        "lineSpacing" => {
            // A double-headed arrow beside three lines: level heads, level lines.
            pen.lines(9.0, 17.0, &[5.0, 10.0, 15.0]);
            pen.line_c(&[(4.5, 4.0), (4.5, 16.0)], a);
            pen.line_c(&[(2.5, 6.0), (4.5, 4.0), (6.5, 6.0)], a);
            pen.line_c(&[(2.5, 14.0), (4.5, 16.0), (6.5, 14.0)], a);
        }
        "shading" => {
            // A tilted paint container with its level line, drop and colour band.
            pen.closed(&[(4.5, 9.5), (10.0, 4.0), (15.5, 9.5), (10.0, 15.0)], c);
            pen.line(&[(4.5, 9.5), (15.5, 9.5)]);
            pen.fcircle(16.5, 12.5, 1.6, a);
            pen.frect(2.5, 16.5, 17.5, 19.0, yellow);
        }
        "borders" => {
            for y in [3.0, 10.0, 17.0] {
                for x in [3.0, 6.5, 10.0, 13.5, 17.0] {
                    pen.fcircle(x, y, 0.6, c);
                }
            }
            for x in [3.0, 10.0, 17.0] {
                for y in [6.5, 13.5] {
                    pen.fcircle(x, y, 0.6, c);
                }
            }
            pen.line_c(&[(3.0, 17.0), (17.0, 17.0)], a);
        }
        // Editing
        "find" => {
            pen.circle(8.0, 8.0, 5.0, c);
            pen.line_c(&[(11.5, 11.5), (17.0, 17.0)], a);
        }
        "replace" => {
            pen.text(6.0, 6.0, 8.0, "ab", c, false);
            pen.text(13.0, 15.0, 8.0, "ac", a, false);
            pen.line(&[(13.0, 3.5), (16.0, 3.5), (16.0, 8.5)]);
            pen.line(&[(14.5, 7.0), (16.0, 8.5), (17.5, 7.0)]);
        }
        "select" => {
            // Overlapping convex pieces keep the concave arrow intact without antialiasing seams.
            pen.fill(&[(5.0, 3.0), (5.0, 16.0), (14.0, 12.0)], c);
            pen.fill(&[(8.0, 13.0), (10.5, 18.0), (12.5, 17.0), (10.0, 12.0)], c);
        }
        "dictate" => {
            pen.rect(7.5, 2.5, 12.5, 12.0, c);
            pen.line(&[(5.0, 9.5), (5.0, 11.0), (10.0, 15.0), (15.0, 11.0), (15.0, 9.5)]);
            pen.line(&[(10.0, 15.0), (10.0, 18.0)]);
        }
        "editor" | "spelling" => {
            pen.text(8.0, 7.0, 9.0, "abc", c, true);
            pen.line_c(&[(5.0, 13.0), (8.5, 16.5), (16.0, 9.0)], green);
        }
        "styles" => {
            pen.text(10.0, 10.0, 14.0, "A", a, false);
            pen.line(&[(13.0, 15.0), (17.0, 11.0)]);
        }
        "stylesPane" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.line(&[(11.0, 3.0), (11.0, 17.0)]);
            pen.text(7.0, 10.0, 8.0, "A", a, false);
        }
        // Insert
        "coverPage" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.frect(5.5, 7.0, 14.5, 10.0, a);
        }
        "blankPage" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.line_c(&[(10.0, 7.0), (10.0, 13.0)], a);
            pen.line_c(&[(7.0, 10.0), (13.0, 10.0)], a);
        }
        "pageBreak" => {
            pen.line(&[(4.0, 2.0), (4.0, 7.0), (16.0, 7.0), (16.0, 2.0)]);
            pen.line(&[(4.0, 18.0), (4.0, 13.0), (16.0, 13.0), (16.0, 18.0)]);
            pen.line_c(&[(2.0, 10.0), (18.0, 10.0)], a);
        }
        "drawTable" => {
            pen.rect(2.5, 3.0, 14.0, 14.5, c);
            pen.line(&[(2.5, 8.5), (14.0, 8.5)]);
            pen.line(&[(8.0, 3.0), (8.0, 14.5)]);
            pen.line(&[(9.5, 18.0), (17.0, 10.5), (18.5, 12.0), (11.0, 19.5), (9.0, 19.8), (9.5, 18.0)]);
            pen.fill(&[(17.0, 10.5), (18.0, 9.5), (19.5, 11.0), (18.5, 12.0)], a);
        }
        "table" => {
            pen.frect(3.0, 3.0, 17.0, 7.0, a);
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.line(&[(3.0, 7.0), (17.0, 7.0)]);
            pen.line(&[(3.0, 12.0), (17.0, 12.0)]);
            pen.line(&[(8.0, 3.0), (8.0, 17.0)]);
            pen.line(&[(12.5, 3.0), (12.5, 17.0)]);
        }
        "picture" => {
            pen.rect(2.5, 4.0, 17.5, 16.0, c);
            pen.fill(&[(4.0, 14.5), (8.5, 8.0), (12.0, 14.5)], a);
            pen.fill(&[(9.5, 14.5), (13.0, 10.0), (16.0, 14.5)], green);
            pen.fcircle(13.5, 7.0, 1.5, yellow);
        }
        "onlinePicture" => {
            paint(p, Rect::from_min_size(r.min, r.size() * 0.8), "picture", c, accent);
            pen.circle(15.0, 15.0, 3.5, a);
        }
        "shapes" => {
            pen.rect(3.0, 3.0, 11.0, 11.0, c);
            pen.fcircle(13.0, 13.0, 4.5, a);
        }
        // Shape Effects: a square casting a soft shadow down and to the right.
        "shapeEffects" => {
            for (k, alpha) in [(2.0, 30), (1.0, 50), (0.0, 80)] {
                pen.frect(7.0 - k, 7.0 - k, 18.0 + k, 18.0 + k, Color32::from_rgba_unmultiplied(0, 0, 0, alpha));
            }
            pen.frect(3.0, 3.0, 14.0, 14.0, a);
            pen.rect(3.0, 3.0, 14.0, 14.0, c);
        }
        "icons" => {
            pen.circle(10.0, 10.0, 7.5, c);
            pen.fcircle(7.5, 8.0, 1.0, c);
            pen.fcircle(12.5, 8.0, 1.0, c);
            pen.line_c(&[(6.5, 12.0), (10.0, 14.5), (13.5, 12.0)], a);
        }
        "models3d" => {
            pen.closed(&[(10.0, 2.5), (17.0, 6.0), (17.0, 14.0), (10.0, 17.5), (3.0, 14.0), (3.0, 6.0)], c);
            pen.line_c(&[(3.0, 6.0), (10.0, 9.5), (17.0, 6.0)], a);
            pen.line_c(&[(10.0, 9.5), (10.0, 17.5)], a);
        }
        "smartArt" => {
            pen.frect(7.0, 2.5, 13.0, 6.5, a);
            pen.rect(2.5, 13.0, 8.0, 17.0, c);
            pen.rect(12.0, 13.0, 17.5, 17.0, c);
            pen.line(&[(10.0, 6.5), (10.0, 10.0), (5.0, 10.0), (5.0, 13.0)]);
            pen.line(&[(10.0, 10.0), (15.0, 10.0), (15.0, 13.0)]);
        }
        "addShape" => {
            pen.frect(2.5, 3.0, 9.5, 8.0, a);
            pen.rect(2.5, 12.0, 9.5, 17.0, c);
            pen.line(&[(6.0, 8.0), (6.0, 12.0)]);
            pen.line_c(&[(14.5, 10.5), (14.5, 17.5)], green);
            pen.line_c(&[(11.0, 14.0), (18.0, 14.0)], green);
        }
        "promote" => {
            pen.lines(9.0, 17.5, &[5.0, 10.0, 15.0]);
            pen.line_c(&[(6.5, 7.5), (3.0, 10.0), (6.5, 12.5)], a);
        }
        "demote" => {
            pen.lines(9.0, 17.5, &[5.0, 10.0, 15.0]);
            pen.line_c(&[(3.0, 7.5), (6.5, 10.0), (3.0, 12.5)], a);
        }
        "textPane" => {
            pen.rect(2.5, 3.0, 17.5, 17.0, c);
            pen.line(&[(9.0, 3.0), (9.0, 17.0)]);
            pen.lines(4.0, 7.5, &[6.5, 10.0, 13.5]);
            pen.frect(11.0, 6.0, 16.0, 9.0, a);
            pen.frect(11.0, 11.5, 16.0, 14.5, a);
        }
        "smartArtColors" => {
            pen.fcircle(7.0, 7.5, 4.0, a);
            pen.fcircle(13.0, 7.5, 4.0, orange);
            pen.fcircle(10.0, 13.0, 4.0, green);
            pen.circle(10.0, 10.0, 8.0, c);
        }
        "resetGraphic" => {
            pen.rect(7.5, 6.5, 16.5, 15.5, c);
            pen.frect(9.5, 8.5, 14.5, 13.5, a);
            pen.line(&[(3.5, 13.0), (3.5, 4.0), (11.0, 4.0)]);
            pen.line(&[(1.5, 6.0), (3.5, 4.0), (5.5, 6.0)]);
        }
        "chart" => {
            pen.frect(3.5, 11.0, 6.5, 17.0, a);
            pen.frect(8.5, 6.0, 11.5, 17.0, orange);
            pen.frect(13.5, 9.0, 16.5, 17.0, green);
            pen.line(&[(2.0, 17.5), (18.0, 17.5)]);
        }
        "changeChartType" => {
            pen.frect(3.0, 11.0, 5.5, 17.0, a);
            pen.frect(7.0, 7.0, 9.5, 17.0, orange);
            pen.line(&[(2.0, 17.5), (11.0, 17.5)]);
            pen.fill(&[(15.0, 10.0), (15.0, 5.0), (19.0, 8.5)], green);
            pen.circle(15.0, 10.0, 4.0, c);
            pen.line_c(&[(4.0, 5.0), (9.0, 2.5), (12.0, 4.5)], a);
        }
        "chartData" => {
            pen.rect(2.5, 3.0, 17.5, 17.0, c);
            pen.frect(2.5, 3.0, 17.5, 6.5, a);
            pen.line(&[(2.5, 10.0), (17.5, 10.0)]);
            pen.line(&[(2.5, 13.5), (17.5, 13.5)]);
            pen.line(&[(7.5, 3.0), (7.5, 17.0)]);
            pen.line(&[(12.5, 3.0), (12.5, 17.0)]);
            pen.frect(9.0, 14.5, 11.0, 16.0, orange);
        }
        "chartTitle" => {
            pen.frect(5.0, 2.5, 15.0, 4.5, a);
            pen.frect(4.0, 11.0, 7.0, 17.0, c);
            pen.frect(8.5, 8.0, 11.5, 17.0, c);
            pen.frect(13.0, 12.5, 16.0, 17.0, c);
            pen.line(&[(2.5, 17.5), (17.5, 17.5)]);
        }
        "legend" => {
            for (k, col) in [a, orange, green].into_iter().enumerate() {
                let y = 5.0 + 5.0 * k as f32;
                pen.frect(3.0, y - 1.5, 6.0, y + 1.5, col);
                pen.line(&[(8.0, y), (17.0, y)]);
            }
        }
        "dataLabels" => {
            pen.frect(3.5, 11.0, 7.0, 17.5, a);
            pen.frect(12.0, 8.0, 15.5, 17.5, orange);
            pen.rect(2.5, 5.5, 8.0, 9.0, c);
            pen.rect(11.0, 2.5, 16.5, 6.0, c);
        }
        "screenshot" => {
            pen.line(&[(3.0, 7.0), (3.0, 3.0), (7.0, 3.0)]);
            pen.line(&[(13.0, 3.0), (17.0, 3.0), (17.0, 7.0)]);
            pen.line(&[(17.0, 13.0), (17.0, 17.0), (13.0, 17.0)]);
            pen.line(&[(7.0, 17.0), (3.0, 17.0), (3.0, 13.0)]);
            pen.fcircle(10.0, 10.0, 3.0, a);
        }
        "video" => {
            pen.rect(2.5, 5.0, 14.0, 15.0, c);
            pen.fill(&[(14.0, 10.0), (18.0, 6.5), (18.0, 13.5)], a);
        }
        "link" => {
            pen.closed(
                &[(3.0, 10.0), (7.0, 6.0), (9.5, 6.0), (9.5, 8.0), (7.5, 8.0), (5.0, 10.5), (7.5, 13.0), (9.5, 13.0), (9.5, 15.0), (7.0, 15.0)],
                c,
            );
            pen.line_c(&[(7.5, 10.5), (12.5, 10.5)], a);
            pen.closed(
                &[
                    (17.0, 10.5),
                    (13.0, 14.5),
                    (10.5, 14.5),
                    (10.5, 12.5),
                    (12.5, 12.5),
                    (15.0, 10.0),
                    (12.5, 7.5),
                    (10.5, 7.5),
                    (10.5, 5.5),
                    (13.0, 5.5),
                ],
                c,
            );
        }
        "bookmark" => {
            pen.closed(&[(5.5, 2.5), (14.5, 2.5), (14.5, 17.5), (10.0, 13.5), (5.5, 17.5)], c);
            pen.fill(&[(7.0, 4.0), (13.0, 4.0), (13.0, 8.0), (7.0, 8.0)], a);
        }
        "crossRef" => {
            pen.page(3.0, 2.0, 12.0, 14.0);
            pen.line_c(&[(9.0, 12.0), (17.0, 17.0)], a);
            pen.line_c(&[(14.0, 17.5), (17.0, 17.0), (16.5, 14.0)], a);
        }
        "comment" | "newComment" => {
            pen.closed(&[(2.5, 3.5), (17.5, 3.5), (17.5, 13.5), (9.0, 13.5), (5.0, 17.0), (5.0, 13.5), (2.5, 13.5)], c);
            if name == "newComment" {
                pen.line_c(&[(10.0, 6.0), (10.0, 11.0)], a);
                pen.line_c(&[(7.5, 8.5), (12.5, 8.5)], a);
            } else {
                pen.lines(5.5, 14.5, &[7.0, 10.0]);
            }
        }
        "header" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.frect(5.5, 5.0, 14.5, 7.5, a);
            pen.lines(6.0, 14.0, &[11.0, 13.5]);
        }
        "footer" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.lines(6.0, 14.0, &[6.5, 9.0]);
            pen.frect(5.5, 13.0, 14.5, 15.5, a);
        }
        "pageNumber" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.text(10.0, 13.0, 7.0, "#", a, true);
        }
        "textBox" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.text(10.0, 10.0, 10.0, "A", a, false);
        }
        "quickParts" => {
            pen.rect(3.0, 3.0, 9.0, 9.0, c);
            pen.frect(11.0, 3.0, 17.0, 9.0, a);
            pen.rect(3.0, 11.0, 9.0, 17.0, c);
            pen.rect(11.0, 11.0, 17.0, 17.0, c);
        }
        "wordArt" => {
            pen.text(10.0, 10.5, 15.0, "A", a, true);
            pen.line_c(&[(3.0, 17.5), (17.0, 17.5)], orange);
        }
        "textFill" => {
            pen.text(10.0, 8.5, 13.0, "A", c, true);
            pen.frect(3.0, 15.0, 17.0, 18.5, a);
        }
        "textOutline" => {
            pen.text(10.0, 8.5, 13.0, "A", a, false);
            pen.rect(3.0, 15.0, 17.0, 18.5, c);
        }
        "transform" => {
            // Letters along an arch.
            let arc: Vec<(f32, f32)> = (0..=12)
                .map(|i| {
                    let t = std::f32::consts::PI * (1.0 - i as f32 / 12.0);
                    (10.0 + 7.5 * t.cos(), 15.0 - 9.0 * t.sin())
                })
                .collect();
            pen.line_c(&arc, a);
            pen.text(10.0, 13.0, 8.0, "A", c, true);
            pen.line(&[(2.5, 17.5), (17.5, 17.5)]);
        }
        "dropCap" => {
            pen.text(6.0, 8.0, 12.0, "A", a, true);
            pen.lines(11.0, 17.0, &[4.0, 8.0, 12.0]);
            pen.lines(3.0, 17.0, &[16.0]);
        }
        "signature" => {
            pen.line_c(&[(3.0, 13.0), (6.0, 7.0), (8.0, 12.0), (11.0, 6.0), (13.0, 12.0)], a);
            pen.line(&[(3.0, 16.5), (17.0, 16.5)]);
        }
        "dateTime" => {
            pen.rect(3.0, 4.0, 17.0, 17.0, c);
            pen.frect(3.0, 4.0, 17.0, 7.5, a);
            pen.lines(5.0, 15.0, &[11.0, 14.0]);
        }
        "object" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.fcircle(10.0, 10.0, 3.5, a);
        }
        "equation" => pen.text(10.0, 10.0, 17.0, "π", a, false),
        "symbol" => pen.text(10.0, 10.0, 17.0, "Ω", c, false),
        // Draw
        "pen" => {
            pen.line(&[(4.0, 16.0), (14.0, 6.0), (16.0, 8.0), (6.0, 18.0), (3.5, 18.5), (4.0, 16.0)]);
            pen.fill(&[(14.0, 6.0), (15.5, 4.5), (17.5, 6.5), (16.0, 8.0)], a);
        }
        "pencil" => {
            pen.line(&[(4.0, 16.0), (14.0, 6.0), (16.0, 8.0), (6.0, 18.0), (3.5, 18.5), (4.0, 16.0)]);
            pen.fill(&[(14.0, 6.0), (15.5, 4.5), (17.5, 6.5), (16.0, 8.0)], orange);
        }
        "eraser" => {
            pen.closed(&[(3.0, 13.0), (11.0, 5.0), (17.0, 11.0), (11.0, 17.0), (7.0, 17.0)], c);
            pen.fill(&[(3.0, 13.0), (7.0, 9.0), (13.0, 15.0), (11.0, 17.0), (7.0, 17.0)], Color32::from_rgb(0xE8, 0x6F, 0xA0));
        }
        // Ink thickness: three strokes, thin to thick.
        "thickness" => {
            pen.frect(3.0, 4.5, 17.0, 5.5, c);
            pen.frect(3.0, 9.0, 17.0, 11.0, c);
            pen.frect(3.0, 14.0, 17.0, 17.0, a);
        }
        "lasso" => {
            pen.closed(&[(4.0, 7.0), (9.0, 3.0), (16.0, 5.0), (16.0, 11.0), (10.0, 13.0), (4.0, 11.0)], c);
            pen.line_c(&[(6.0, 12.5), (5.0, 17.5)], a);
        }
        "inkToShape" => {
            pen.line_c(&[(3.0, 10.0), (5.0, 6.0), (8.0, 12.0), (10.0, 7.0)], a);
            pen.rect(11.0, 9.0, 17.0, 15.0, c);
        }
        "inkToMath" => {
            pen.line_c(&[(3.0, 12.0), (5.0, 7.0), (7.0, 12.0)], a);
            pen.text(13.5, 10.0, 11.0, "√", c, false);
        }
        "canvas" => {
            pen.rect(2.5, 4.0, 17.5, 16.0, c);
            pen.line_c(&[(5.0, 13.0), (9.0, 8.0), (12.0, 11.0), (15.0, 7.0)], a);
        }
        "replay" => {
            pen.circle(10.0, 10.0, 6.5, c);
            pen.fill(&[(8.5, 7.0), (8.5, 13.0), (13.0, 10.0)], a);
        }
        // Add Pen: a pen with a plus.
        "addPen" => {
            pen.line(&[(3.0, 17.0), (11.0, 9.0), (13.0, 11.0), (5.0, 19.0), (2.5, 19.5), (3.0, 17.0)]);
            pen.fill(&[(11.0, 9.0), (12.5, 7.5), (14.5, 9.5), (13.0, 11.0)], a);
            pen.line_c(&[(15.0, 2.0), (15.0, 8.0)], a);
            pen.line_c(&[(12.0, 5.0), (18.0, 5.0)], a);
        }
        // Ink Replay's controls.
        "replayRewind" => {
            pen.frect(4.0, 5.0, 5.5, 15.0, c);
            pen.fill(&[(16.0, 5.0), (16.0, 15.0), (6.5, 10.0)], c);
        }
        "replayPlay" => pen.fill(&[(6.0, 4.0), (6.0, 16.0), (16.0, 10.0)], a),
        "replayPause" => {
            pen.frect(5.5, 4.5, 8.5, 15.5, a);
            pen.frect(11.5, 4.5, 14.5, 15.5, a);
        }
        "replayForward" => {
            pen.fill(&[(4.0, 5.0), (4.0, 15.0), (13.5, 10.0)], c);
            pen.frect(14.5, 5.0, 16.0, 15.0, c);
        }
        // Design
        "themes" => {
            pen.frect(3.0, 3.0, 10.0, 10.0, a);
            pen.frect(10.0, 3.0, 17.0, 10.0, orange);
            pen.frect(3.0, 10.0, 10.0, 17.0, green);
            pen.frect(10.0, 10.0, 17.0, 17.0, yellow);
        }
        "colors" => {
            pen.fcircle(7.0, 7.5, 3.5, red);
            pen.fcircle(13.0, 7.5, 3.5, a);
            pen.fcircle(10.0, 13.0, 3.5, green);
        }
        "fonts" => pen.text(10.0, 10.0, 13.0, "Aa", a, false),
        "paraSpacing" => {
            pen.lines(8.0, 17.0, &[4.0, 7.0, 13.0, 16.0]);
            pen.line_c(&[(4.0, 8.0), (4.0, 12.0)], a);
            pen.line_c(&[(2.5, 9.5), (4.0, 8.0), (5.5, 9.5)], a);
            pen.line_c(&[(2.5, 10.5), (4.0, 12.0), (5.5, 10.5)], a);
        }
        "effectsDesign" => {
            pen.fcircle(10.0, 10.0, 6.0, a.linear_multiply(0.5));
            pen.circle(10.0, 10.0, 6.0, c);
        }
        "setDefault" => {
            pen.line_c(&[(4.0, 10.5), (8.5, 15.0), (16.5, 5.0)], green);
        }
        "watermark" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.text(10.0, 11.0, 7.0, "W", a.linear_multiply(0.6), true);
        }
        "pageColor" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.frect(5.0, 9.0, 15.0, 17.0, a.linear_multiply(0.5));
        }
        "pageBorders" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.rect(6.0, 6.5, 14.0, 15.5, a);
        }
        // Layout
        "margins" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.frect(6.0, 4.5, 14.0, 15.5, a.linear_multiply(0.3));
        }
        "orientation" => {
            pen.page(3.0, 3.0, 11.0, 15.0);
            pen.rect(9.0, 10.0, 18.0, 17.0, a);
        }
        "size" => {
            pen.page(5.0, 2.0, 15.0, 18.0);
            pen.line_c(&[(2.5, 2.5), (2.5, 17.5)], a);
            pen.line_c(&[(5.0, 19.0), (15.0, 19.0)], a);
        }
        "columns" => {
            pen.lines(3.0, 9.0, &[4.0, 7.0, 10.0, 13.0, 16.0]);
            pen.lines(11.0, 17.0, &[4.0, 7.0, 10.0, 13.0]);
        }
        "breaks" => {
            pen.page(4.0, 1.5, 16.0, 8.5);
            pen.page(4.0, 11.5, 16.0, 18.5);
            pen.line_c(&[(2.0, 10.0), (18.0, 10.0)], a);
        }
        "lineNumbers" => {
            for (i, y) in [4.0, 8.0, 12.0, 16.0].iter().enumerate() {
                pen.text(4.0, *y, 5.0, &(i + 1).to_string(), a, true);
                pen.line(&[(7.0, *y), (17.0, *y)]);
            }
        }
        "hyphenation" => {
            pen.text(7.0, 7.0, 8.0, "ab-", c, false);
            pen.text(9.0, 14.5, 8.0, "cd", a, false);
        }
        "position" | "wrapText" => {
            pen.lines(3.0, 17.0, &[3.0, 17.0]);
            pen.lines(3.0, 6.0, &[7.0, 10.0, 13.0]);
            pen.frect(7.5, 6.0, 12.5, 14.0, a);
            pen.lines(14.0, 17.0, &[7.0, 10.0, 13.0]);
        }
        "bringForward" => {
            pen.rect(3.0, 3.0, 12.0, 12.0, c);
            pen.frect(8.0, 8.0, 17.0, 17.0, a);
        }
        "sendBackward" => {
            pen.frect(3.0, 3.0, 12.0, 12.0, a);
            pen.rect(8.0, 8.0, 17.0, 17.0, c);
        }
        "selectionPane" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.lines(6.0, 14.0, &[7.0, 10.0, 13.0]);
        }
        "align" => {
            pen.line_c(&[(3.0, 2.0), (3.0, 18.0)], a);
            pen.rect(4.5, 4.0, 15.0, 8.0, c);
            pen.rect(4.5, 11.0, 11.0, 15.0, c);
        }
        "group" => {
            pen.rect(2.5, 2.5, 17.5, 17.5, a);
            pen.rect(5.0, 5.0, 11.0, 11.0, c);
            pen.fcircle(13.0, 13.0, 3.0, c);
        }
        "rotate" => {
            pen.circle(10.0, 10.0, 6.0, c);
            pen.fill(&[(14.5, 3.0), (17.5, 6.0), (13.5, 7.0)], a);
        }
        "indentLeft" | "indentRight" | "spaceBefore" | "spaceAfter" => {
            pen.lines(3.0, 17.0, &[6.0, 10.0, 14.0]);
            pen.fcircle(if name.contains("Right") { 15.0 } else { 5.0 }, 10.0, 1.6, a);
        }
        // References
        "toc" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.lines(6.0, 11.0, &[7.0, 10.0, 13.0]);
            for y in [7.0, 10.0, 13.0] {
                pen.fcircle(13.5, y, 0.8, a);
            }
        }
        "addText" => {
            pen.lines(3.0, 13.0, &[5.0, 9.0, 13.0]);
            pen.line_c(&[(15.0, 11.0), (15.0, 17.0)], a);
            pen.line_c(&[(12.0, 14.0), (18.0, 14.0)], a);
        }
        "updateTable" | "update" => {
            pen.circle(10.0, 10.0, 6.0, c);
            pen.fill(&[(14.5, 3.0), (17.5, 6.0), (13.5, 7.0)], green);
        }
        "footnote" => {
            pen.text(6.5, 7.0, 10.0, "ab", c, false);
            pen.text(13.5, 4.5, 6.0, "1", a, true);
            pen.line(&[(3.0, 13.0), (9.0, 13.0)]);
            pen.lines(3.0, 17.0, &[16.0]);
        }
        "endnote" => {
            pen.text(6.5, 7.0, 10.0, "ab", c, false);
            pen.text(13.5, 4.5, 6.0, "i", a, true);
            pen.lines(3.0, 17.0, &[13.0, 16.0]);
        }
        "nextFootnote" => {
            pen.text(7.0, 7.0, 8.0, "ab", c, false);
            pen.text(13.0, 5.0, 6.0, "1", a, true);
            pen.line_c(&[(6.0, 15.0), (14.0, 15.0)], a);
            pen.line_c(&[(11.5, 12.5), (14.0, 15.0), (11.5, 17.5)], a);
        }
        "showNotes" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.frect(5.5, 13.0, 14.5, 16.5, a.linear_multiply(0.5));
        }
        "researcher" => {
            pen.circle(9.0, 9.0, 5.0, c);
            pen.line_c(&[(12.5, 12.5), (17.0, 17.0)], a);
            pen.lines(6.5, 11.5, &[8.0, 10.0]);
        }
        // Zotero
        "docPrefs" => {
            pen.page(3.0, 2.5, 13.0, 17.5);
            pen.lines(5.0, 10.0, &[7.5, 10.0]);
            // A small gear: ring and teeth.
            pen.circle(14.0, 14.0, 2.2, a);
            for k in 0..6 {
                let t = k as f32 * std::f32::consts::PI / 3.0;
                let (dx, dy) = (t.cos(), t.sin());
                pen.line_c(&[(14.0 + 2.8 * dx, 14.0 + 2.8 * dy), (14.0 + 4.2 * dx, 14.0 + 4.2 * dy)], a);
            }
        }
        "unlinkCitations" => {
            // Two chain links pulled apart, with a break between them.
            pen.closed(
                &[(2.5, 12.5), (6.0, 9.0), (8.5, 9.0), (8.5, 10.8), (6.8, 10.8), (4.6, 13.0), (6.8, 15.2), (8.5, 15.2), (8.5, 17.0), (6.0, 17.0)],
                c,
            );
            pen.closed(
                &[(17.5, 7.5), (14.0, 11.0), (11.5, 11.0), (11.5, 9.2), (13.2, 9.2), (15.4, 7.0), (13.2, 4.8), (11.5, 4.8), (11.5, 3.0), (14.0, 3.0)],
                c,
            );
            pen.line_c(&[(9.0, 6.5), (7.5, 5.0)], a);
            pen.line_c(&[(11.0, 13.5), (12.5, 15.0)], a);
            pen.line_c(&[(10.0, 5.5), (10.0, 3.5)], a);
            pen.line_c(&[(10.0, 14.5), (10.0, 16.5)], a);
        }
        "addNote" => {
            pen.closed(&[(3.0, 3.0), (17.0, 3.0), (17.0, 12.5), (12.5, 17.0), (3.0, 17.0)], c);
            pen.line(&[(12.5, 17.0), (12.5, 12.5), (17.0, 12.5)]);
            pen.line_c(&[(6.0, 8.0), (11.0, 8.0)], a);
            pen.line_c(&[(8.5, 5.5), (8.5, 10.5)], a);
            pen.lines(6.0, 11.0, &[13.5]);
        }
        "citation" => {
            pen.text(6.0, 9.0, 14.0, "“", a, true);
            pen.lines(9.0, 17.0, &[6.0, 10.0, 14.0]);
        }
        "sources" | "bibliography" => {
            pen.rect(4.0, 3.0, 9.0, 17.0, c);
            pen.rect(9.0, 3.0, 13.0, 17.0, a);
            pen.closed(&[(13.5, 4.0), (16.5, 3.5), (18.0, 16.5), (15.0, 17.0)], c);
        }
        "caption" => {
            pen.rect(3.0, 2.5, 17.0, 11.5, c);
            pen.fill(&[(4.5, 10.0), (8.0, 5.5), (11.0, 10.0)], a);
            pen.lines(3.0, 13.0, &[15.0, 17.5]);
        }
        "tableOfFigures" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.frect(6.0, 5.0, 10.0, 8.0, a);
            pen.lines(6.0, 14.0, &[11.0, 14.0]);
        }
        "markEntry" | "index" | "markCitation" | "tableOfAuthorities" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.text(10.0, 10.0, 8.0, if name == "index" || name == "markEntry" { "A" } else { "§" }, a, true);
        }
        // Mailings
        "envelope" => {
            pen.rect(2.5, 5.0, 17.5, 15.5, c);
            pen.line_c(&[(2.5, 5.0), (10.0, 11.0), (17.5, 5.0)], a);
        }
        "labels" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.frect(5.0, 5.0, 9.5, 9.0, a);
            pen.frect(10.5, 5.0, 15.0, 9.0, a);
            pen.frect(5.0, 11.0, 9.5, 15.0, a);
            pen.frect(10.5, 11.0, 15.0, 15.0, a);
        }
        "mailMerge" | "recipients" | "editRecipients" => {
            pen.fcircle(7.0, 7.0, 2.5, a);
            pen.line(&[(3.0, 15.0), (3.0, 13.0), (7.0, 10.5), (11.0, 13.0), (11.0, 15.0)]);
            pen.fcircle(13.5, 6.0, 2.0, c);
            pen.line(&[(11.5, 10.0), (13.5, 9.0), (17.0, 11.0), (17.0, 14.0)]);
        }
        "mergeField" | "addressBlock" | "greetingLine" | "rules" | "matchFields" | "highlightFields" => {
            pen.rect(2.5, 6.0, 17.5, 14.0, c);
            pen.text(10.0, 10.0, 7.0, "«»", a, true);
        }
        "preview" => {
            pen.text(10.0, 10.0, 8.0, "ABC", a, true);
            pen.rect(2.5, 5.0, 17.5, 15.0, c);
        }
        "next" | "previous" | "first" | "last" => {
            let pts: &[(f32, f32)] =
                if name == "next" || name == "last" { &[(7.0, 4.0), (13.0, 10.0), (7.0, 16.0)] } else { &[(13.0, 4.0), (7.0, 10.0), (13.0, 16.0)] };
            pen.line_c(pts, a);
        }
        "finish" => {
            pen.page(3.0, 2.0, 13.0, 15.0);
            pen.line_c(&[(10.0, 15.0), (13.0, 18.0), (18.0, 11.0)], green);
        }
        "checkErrors" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.fcircle(14.0, 14.0, 3.5, red);
        }
        // Review
        "thesaurus" => {
            pen.rect(3.5, 2.5, 16.5, 17.5, c);
            pen.text(10.0, 9.5, 8.0, "Aa", a, true);
        }
        "wordCount" => pen.text(10.0, 10.0, 9.0, "123", a, true),
        "readAloud" => {
            pen.text(6.0, 10.0, 10.0, "A", c, false);
            pen.line_c(&[(12.0, 7.0), (13.5, 10.0), (12.0, 13.0)], a);
            pen.line_c(&[(14.5, 5.0), (17.0, 10.0), (14.5, 15.0)], a);
        }
        "accessibility" => {
            pen.fcircle(10.0, 4.0, 1.6, a);
            pen.line(&[(4.0, 7.0), (16.0, 7.0)]);
            pen.line(&[(10.0, 7.0), (10.0, 12.0), (7.0, 17.5)]);
            pen.line(&[(10.0, 12.0), (13.0, 17.5)]);
        }
        "altText" => {
            // A small picture with lines of text beside and below it.
            pen.rect(2.5, 3.5, 11.5, 11.0, c);
            pen.line_c(&[(3.5, 10.0), (6.0, 6.5), (8.0, 8.5), (9.0, 7.5), (10.5, 10.0)], a);
            for (x0, x1, y) in [(13.5, 17.5, 5.0), (13.5, 17.5, 8.0), (2.5, 17.5, 14.0), (2.5, 13.0, 17.0)] {
                pen.line_c(&[(x0, y), (x1, y)], c);
            }
        }
        "translate" | "language" => {
            pen.text(6.5, 7.0, 9.0, "A", c, true);
            pen.text(13.5, 13.5, 9.0, "あ", a, false);
        }
        "deleteComment" => {
            pen.closed(&[(2.5, 3.5), (17.5, 3.5), (17.5, 13.5), (9.0, 13.5), (5.0, 17.0), (5.0, 13.5), (2.5, 13.5)], c);
            pen.line_c(&[(7.5, 6.0), (12.5, 11.0)], red);
            pen.line_c(&[(12.5, 6.0), (7.5, 11.0)], red);
        }
        "prevComment" | "nextComment" => {
            pen.closed(&[(2.5, 3.5), (17.5, 3.5), (17.5, 13.5), (9.0, 13.5), (5.0, 17.0), (5.0, 13.5), (2.5, 13.5)], c);
            let pts: &[(f32, f32)] =
                if name == "nextComment" { &[(8.0, 6.0), (12.0, 8.5), (8.0, 11.0)] } else { &[(12.0, 6.0), (8.0, 8.5), (12.0, 11.0)] };
            pen.line_c(pts, a);
        }
        "showComments" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.frect(12.0, 3.0, 17.0, 17.0, a.linear_multiply(0.4));
        }
        "resolve" => {
            pen.closed(&[(2.5, 3.5), (17.5, 3.5), (17.5, 13.5), (9.0, 13.5), (5.0, 17.0), (5.0, 13.5), (2.5, 13.5)], c);
            pen.line_c(&[(6.5, 8.5), (9.0, 11.0), (13.5, 6.0)], green);
        }
        "trackChanges" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.line_c(&[(6.0, 7.0), (14.0, 7.0)], red);
            pen.line_c(&[(6.0, 11.0), (14.0, 11.0)], a);
            pen.lines(6.0, 11.0, &[15.0]);
        }
        "markup" | "reviewingPane" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.line_c(&[(6.0, 8.0), (13.0, 8.0)], red);
            pen.lines(6.0, 13.0, &[12.0]);
        }
        "accept" => {
            pen.page(4.0, 2.0, 15.0, 17.0);
            pen.line_c(&[(10.0, 14.5), (12.5, 17.0), (18.0, 11.0)], green);
        }
        "reject" => {
            pen.page(4.0, 2.0, 15.0, 17.0);
            pen.line_c(&[(11.0, 12.0), (17.0, 18.0)], red);
            pen.line_c(&[(17.0, 12.0), (11.0, 18.0)], red);
        }
        "prevChange" | "nextChange" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            let pts: &[(f32, f32)] =
                if name == "nextChange" { &[(8.5, 7.0), (11.5, 10.0), (8.5, 13.0)] } else { &[(11.5, 7.0), (8.5, 10.0), (11.5, 13.0)] };
            pen.line_c(pts, a);
        }
        "compare" | "combine" => {
            pen.page(2.0, 3.0, 10.0, 15.0);
            pen.page(10.0, 5.0, 18.0, 17.0);
            pen.line_c(&[(6.0, 18.5), (14.0, 18.5)], a);
        }
        "protect" | "restrict" | "blockAuthors" => {
            pen.rect(5.0, 9.0, 15.0, 17.5, c);
            pen.line_c(&[(7.0, 9.0), (7.0, 6.0), (10.0, 3.0), (13.0, 6.0), (13.0, 9.0)], a);
        }
        "hideInk" => {
            pen.line_c(&[(3.0, 13.0), (6.0, 8.0), (9.0, 13.0), (12.0, 8.0)], a);
            pen.line(&[(3.0, 3.0), (17.0, 17.0)]);
        }
        // View
        "readMode" => {
            pen.closed(&[(2.0, 4.0), (9.5, 5.0), (9.5, 17.0), (2.0, 16.0)], c);
            pen.closed(&[(18.0, 4.0), (10.5, 5.0), (10.5, 17.0), (18.0, 16.0)], a);
        }
        "printLayout" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.lines(6.0, 14.0, &[7.0, 10.0, 13.0]);
        }
        "webLayout" => {
            pen.circle(10.0, 10.0, 7.0, c);
            pen.line_c(&[(3.0, 10.0), (17.0, 10.0)], a);
            pen.line_c(&[(10.0, 3.0), (7.0, 10.0), (10.0, 17.0)], a);
            pen.line_c(&[(10.0, 3.0), (13.0, 10.0), (10.0, 17.0)], a);
        }
        "outline" => {
            pen.fcircle(4.0, 4.5, 1.2, a);
            pen.line(&[(6.5, 4.5), (17.0, 4.5)]);
            pen.fcircle(7.0, 10.0, 1.2, a);
            pen.line(&[(9.5, 10.0), (17.0, 10.0)]);
            pen.fcircle(10.0, 15.5, 1.2, a);
            pen.line(&[(12.5, 15.5), (17.0, 15.5)]);
        }
        "draft" => pen.lines(3.0, 17.0, &[4.0, 7.0, 10.0, 13.0, 16.0]),
        "focus" => {
            pen.line(&[(3.0, 7.0), (3.0, 3.0), (7.0, 3.0)]);
            pen.line(&[(13.0, 3.0), (17.0, 3.0), (17.0, 7.0)]);
            pen.line(&[(17.0, 13.0), (17.0, 17.0), (13.0, 17.0)]);
            pen.line(&[(7.0, 17.0), (3.0, 17.0), (3.0, 13.0)]);
            pen.lines(7.0, 13.0, &[8.5, 11.5]);
        }
        "immersive" => {
            pen.closed(&[(2.0, 4.0), (9.5, 5.0), (9.5, 17.0), (2.0, 16.0)], c);
            pen.closed(&[(18.0, 4.0), (10.5, 5.0), (10.5, 17.0), (18.0, 16.0)], c);
            pen.fcircle(14.0, 10.0, 2.0, a);
        }
        "vertical" => {
            pen.page(5.0, 1.0, 15.0, 9.0);
            pen.page(5.0, 11.0, 15.0, 19.0);
        }
        "sideToSide" => {
            pen.page(1.5, 4.0, 9.5, 16.0);
            pen.page(10.5, 4.0, 18.5, 16.0);
        }
        "ruler" => {
            pen.rect(2.0, 7.0, 18.0, 13.0, c);
            for x in [4.5, 7.0, 9.5, 12.0, 14.5] {
                pen.line_c(&[(x, 7.0), (x, 10.0)], a);
            }
        }
        "gridlines" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            for k in [6.5, 10.0, 13.5] {
                pen.line_c(&[(k, 3.0), (k, 17.0)], a.linear_multiply(0.6));
                pen.line_c(&[(3.0, k), (17.0, k)], a.linear_multiply(0.6));
            }
        }
        "navPane" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.frect(3.0, 3.0, 8.0, 17.0, a.linear_multiply(0.5));
        }
        "zoom" => {
            pen.circle(8.5, 8.5, 5.5, c);
            pen.line_c(&[(12.5, 12.5), (17.5, 17.5)], a);
            pen.line(&[(6.0, 8.5), (11.0, 8.5)]);
            pen.line(&[(8.5, 6.0), (8.5, 11.0)]);
        }
        "zoomIn" | "zoomOut" => {
            pen.circle(8.5, 8.5, 5.5, c);
            pen.line_c(&[(12.5, 12.5), (17.5, 17.5)], a);
            pen.line(&[(6.0, 8.5), (11.0, 8.5)]);
            if name == "zoomIn" {
                pen.line(&[(8.5, 6.0), (8.5, 11.0)]);
            }
        }
        "zoom100" => pen.text(10.0, 10.0, 7.5, "100", a, true),
        "onePage" => pen.page(5.0, 2.0, 15.0, 18.0),
        "multiplePages" => {
            pen.page(1.5, 4.0, 9.5, 16.0);
            pen.page(10.5, 4.0, 18.5, 16.0);
        }
        "pageWidth" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.line_c(&[(1.5, 10.0), (18.5, 10.0)], a);
        }
        "darkMode" => {
            pen.fcircle(10.0, 10.0, 7.0, c);
            pen.fcircle(13.0, 8.0, 6.0, Color32::from_rgba_unmultiplied(255, 255, 255, 230));
        }
        "interfaceTheme" => {
            // A window, its right half dark: light, dark or both (follow the system).
            pen.frect(10.0, 3.5, 17.5, 16.5, c);
            pen.rect(2.5, 3.5, 17.5, 16.5, c);
            pen.line_c(&[(2.5, 6.5), (10.0, 6.5)], a);
        }
        "newWindow" => {
            pen.rect(2.5, 5.0, 13.0, 15.0, c);
            pen.rect(7.0, 2.5, 17.5, 12.5, a);
        }
        "arrangeAll" => {
            pen.rect(3.0, 2.5, 17.0, 9.0, c);
            pen.rect(3.0, 11.0, 17.0, 17.5, a);
        }
        "split" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.line_c(&[(3.0, 10.0), (17.0, 10.0)], a);
        }
        "fullScreen" => {
            // A screen with arrows out to its four corners.
            pen.rect(2.0, 3.5, 18.0, 16.5, c);
            pen.line_c(&[(5.0, 8.5), (5.0, 6.0), (7.5, 6.0)], a);
            pen.line_c(&[(12.5, 6.0), (15.0, 6.0), (15.0, 8.5)], a);
            pen.line_c(&[(15.0, 11.5), (15.0, 14.0), (12.5, 14.0)], a);
            pen.line_c(&[(7.5, 14.0), (5.0, 14.0), (5.0, 11.5)], a);
            pen.line_c(&[(5.0, 6.0), (8.0, 8.5)], a);
            pen.line_c(&[(15.0, 6.0), (12.0, 8.5)], a);
            pen.line_c(&[(15.0, 14.0), (12.0, 11.5)], a);
            pen.line_c(&[(5.0, 14.0), (8.0, 11.5)], a);
        }
        "sideBySide" | "syncScroll" | "switchWindows" => {
            pen.rect(2.0, 4.0, 9.5, 16.0, c);
            pen.rect(10.5, 4.0, 18.0, 16.0, a);
        }
        "macros" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.fill(&[(8.0, 6.5), (8.0, 13.5), (14.0, 10.0)], a);
        }
        // A page with a filled band at the top and a style mark: the document's template.
        "documentTemplate" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.frect(6.0, 5.0, 11.5, 7.0, a);
            pen.text(10.0, 13.0, 8.0, "A", a, false);
        }
        "properties" | "info" => {
            pen.circle(10.0, 10.0, 7.0, c);
            pen.line_c(&[(10.0, 9.0), (10.0, 14.5)], a);
            pen.fcircle(10.0, 6.3, 1.0, a);
        }
        // Table tools
        "insertAbove" | "insertBelow" | "insertLeft" | "insertRight" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            match name {
                "insertAbove" => pen.frect(3.0, 3.0, 17.0, 8.0, a),
                "insertBelow" => pen.frect(3.0, 12.0, 17.0, 17.0, a),
                "insertLeft" => pen.frect(3.0, 3.0, 8.0, 17.0, a),
                _ => pen.frect(12.0, 3.0, 17.0, 17.0, a),
            }
        }
        "deleteTable" => {
            pen.rect(3.0, 3.0, 15.0, 15.0, c);
            pen.line(&[(3.0, 9.0), (15.0, 9.0)]);
            pen.line(&[(9.0, 3.0), (9.0, 15.0)]);
            pen.line_c(&[(12.0, 12.0), (18.0, 18.0)], red);
            pen.line_c(&[(18.0, 12.0), (12.0, 18.0)], red);
        }
        "merge" => {
            pen.rect(3.0, 5.0, 17.0, 15.0, c);
            pen.line_c(&[(5.0, 10.0), (15.0, 10.0)], a);
            pen.line_c(&[(7.0, 8.0), (5.0, 10.0), (7.0, 12.0)], a);
            pen.line_c(&[(13.0, 8.0), (15.0, 10.0), (13.0, 12.0)], a);
        }
        "splitCells" | "splitTable" => {
            pen.rect(3.0, 5.0, 17.0, 15.0, c);
            pen.line_c(&[(10.0, 5.0), (10.0, 15.0)], a);
        }
        "autofit" | "distributeRows" | "distributeCols" | "rowHeight" | "colWidth" => {
            pen.rect(3.0, 4.0, 17.0, 16.0, c);
            pen.line_c(&[(5.0, 10.0), (15.0, 10.0)], a);
            pen.line_c(&[(7.0, 8.0), (5.0, 10.0), (7.0, 12.0)], a);
            pen.line_c(&[(13.0, 8.0), (15.0, 10.0), (13.0, 12.0)], a);
        }
        "cellAlign" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.lines(6.0, 14.0, &[8.5, 11.5]);
        }
        "textDirection" => {
            pen.text(9.0, 10.0, 10.0, "A", c, false);
            pen.line_c(&[(15.5, 4.0), (15.5, 16.0)], a);
            pen.line_c(&[(13.5, 14.0), (15.5, 16.0), (17.5, 14.0)], a);
        }
        "cellMargins" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.rect(6.0, 6.0, 14.0, 14.0, a);
        }
        "repeatHeader" => {
            pen.frect(3.0, 3.0, 17.0, 7.0, a);
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.line(&[(3.0, 12.0), (17.0, 12.0)]);
        }
        "toText" => {
            pen.rect(3.0, 3.0, 11.0, 11.0, c);
            pen.lines(9.0, 17.0, &[14.0, 17.0]);
        }
        "formula" => pen.text(10.0, 10.0, 12.0, "fx", a, false),
        "selectTable" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.fill(&[(8.0, 7.0), (8.0, 15.0), (10.0, 13.0), (11.5, 16.0), (12.5, 15.5), (11.0, 12.5), (13.5, 12.5)], a);
        }
        "draw" => {
            pen.rect(3.0, 3.0, 13.0, 13.0, c);
            pen.line_c(&[(10.0, 17.0), (17.0, 10.0)], a);
        }
        "borderPainter" => {
            pen.line(&[(4.0, 16.0), (14.0, 6.0), (16.0, 8.0), (6.0, 18.0), (3.5, 18.5), (4.0, 16.0)]);
            pen.line_c(&[(3.0, 3.0), (11.0, 3.0)], a);
        }
        // Chrome
        "save" => {
            pen.closed(&[(3.0, 3.0), (14.0, 3.0), (17.0, 6.0), (17.0, 17.0), (3.0, 17.0)], c);
            pen.frect(6.0, 3.0, 13.0, 7.5, a);
            pen.rect(6.0, 11.0, 14.0, 17.0, c);
        }
        "undo" => {
            pen.line(&[(5.0, 8.0), (13.0, 8.0)]);
            pen.p.add(Shape::line(
                (0..=12)
                    .map(|i| {
                        let t = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
                        pen.pt(13.0 + 4.5 * t.cos(), 12.5 + 4.5 * t.sin())
                    })
                    .collect(),
                pen.stroke(c),
            ));
            pen.line(&[(13.0, 17.0), (8.0, 17.0)]);
            pen.fill(&[(2.5, 8.0), (6.5, 4.5), (6.5, 11.5)], a);
        }
        "redo" => {
            pen.line(&[(15.0, 8.0), (7.0, 8.0)]);
            pen.p.add(Shape::line(
                (0..=12)
                    .map(|i| {
                        let t = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
                        pen.pt(7.0 + 4.5 * t.cos(), 12.5 - 4.5 * t.sin())
                    })
                    .collect(),
                pen.stroke(c),
            ));
            pen.line(&[(7.0, 17.0), (12.0, 17.0)]);
            pen.fill(&[(17.5, 8.0), (13.5, 4.5), (13.5, 11.5)], a);
        }
        "search" => {
            pen.circle(8.5, 8.5, 5.0, c);
            pen.line(&[(12.0, 12.0), (16.5, 16.5)]);
        }
        "share" => {
            pen.line(&[(10.0, 3.0), (10.0, 12.0)]);
            pen.line(&[(6.5, 6.5), (10.0, 3.0), (13.5, 6.5)]);
            pen.line(&[(5.0, 9.0), (3.5, 9.0), (3.5, 17.0), (16.5, 17.0), (16.5, 9.0), (15.0, 9.0)]);
        }
        "user" => {
            pen.fcircle(10.0, 7.0, 3.5, a);
            pen.p.add(Shape::convex_polygon(
                (0..=16)
                    .map(|i| {
                        let t = std::f32::consts::PI * i as f32 / 16.0;
                        pen.pt(10.0 - 6.5 * t.cos(), 18.0 - 6.0 * t.sin())
                    })
                    .collect(),
                a,
                Stroke::NONE,
            ));
        }
        "close" => {
            pen.line(&[(5.0, 5.0), (15.0, 15.0)]);
            pen.line(&[(15.0, 5.0), (5.0, 15.0)]);
        }
        // A triangle filling most of its box: it is drawn at 9–12 pt beside combos and split
        // buttons, where a smaller one shrank to a dot (#535).
        "dropdown" => pen.fill(&[(3.0, 7.0), (17.0, 7.0), (10.0, 14.0)], c),
        "more" => {
            for x in [5.0, 10.0, 15.0] {
                pen.fcircle(x, 10.0, 1.3, c);
            }
        }
        "launcher" => {
            pen.line(&[(5.0, 5.0), (5.0, 15.0), (15.0, 15.0)]);
            pen.line(&[(8.0, 8.0), (15.0, 15.0)]);
            pen.line(&[(11.0, 15.0), (15.0, 15.0), (15.0, 11.0)]);
        }
        "pin" => {
            pen.line(&[(10.0, 12.0), (10.0, 18.0)]);
            pen.rect(7.0, 2.0, 13.0, 9.0, c);
            pen.line(&[(5.0, 12.0), (15.0, 12.0)]);
        }
        "plus" => {
            pen.line(&[(10.0, 4.0), (10.0, 16.0)]);
            pen.line(&[(4.0, 10.0), (16.0, 10.0)]);
        }
        "minus" => pen.line(&[(4.0, 10.0), (16.0, 10.0)]),
        "check" => pen.line_c(&[(4.0, 10.5), (8.5, 15.0), (16.5, 5.0)], c),
        "chevronLeft" => pen.line(&[(13.0, 4.0), (7.0, 10.0), (13.0, 16.0)]),
        "chevronRight" => pen.line(&[(7.0, 4.0), (13.0, 10.0), (7.0, 16.0)]),
        "chevronUp" => pen.line(&[(4.0, 13.0), (10.0, 7.0), (16.0, 13.0)]),
        "chevronDown" => pen.line(&[(4.0, 7.0), (10.0, 13.0), (16.0, 7.0)]),
        // Read Aloud player
        "mediaPlay" => pen.fill(&[(6.0, 4.0), (16.0, 10.0), (6.0, 16.0)], c),
        "mediaPause" => {
            pen.frect(5.5, 4.0, 8.5, 16.0, c);
            pen.frect(11.5, 4.0, 14.5, 16.0, c);
        }
        "mediaStop" => pen.frect(5.0, 5.0, 15.0, 15.0, c),
        "mediaPrev" => {
            pen.frect(4.0, 5.0, 6.0, 15.0, c);
            pen.fill(&[(16.0, 5.0), (7.0, 10.0), (16.0, 15.0)], c);
        }
        "mediaNext" => {
            pen.frect(14.0, 5.0, 16.0, 15.0, c);
            pen.fill(&[(4.0, 5.0), (13.0, 10.0), (4.0, 15.0)], c);
        }
        // Move past citation: a bracketed citation with an arrow hopping over it to the caret.
        "pastCitation" => {
            pen.line_c(&[(4.5, 9.0), (3.0, 9.0), (3.0, 17.0), (4.5, 17.0)], c);
            pen.line_c(&[(11.5, 9.0), (13.0, 9.0), (13.0, 17.0), (11.5, 17.0)], c);
            pen.lines(5.5, 10.5, &[12.0, 14.5]);
            pen.line_c(&[(4.0, 6.0), (8.0, 2.5), (14.0, 2.5), (16.5, 7.0)], a);
            pen.line_c(&[(14.0, 6.0), (16.5, 7.0), (17.5, 4.5)], a);
            pen.line_c(&[(17.0, 9.0), (17.0, 17.5)], a);
        }
        "chevronDoubleRight" => {
            pen.line(&[(4.0, 4.0), (9.0, 10.0), (4.0, 16.0)]);
            pen.line(&[(11.0, 4.0), (16.0, 10.0), (11.0, 16.0)]);
        }
        "addins" => {
            pen.rect(3.0, 3.0, 9.0, 9.0, c);
            pen.rect(11.0, 3.0, 17.0, 9.0, c);
            pen.rect(3.0, 11.0, 9.0, 17.0, c);
            pen.line_c(&[(14.0, 11.0), (14.0, 17.0)], a);
            pen.line_c(&[(11.0, 14.0), (17.0, 14.0)], a);
        }
        "discord" => {
            pen.fill(&[(3.0, 6.0), (7.0, 3.5), (13.0, 3.5), (17.0, 6.0), (17.5, 14.0), (13.5, 16.5), (6.5, 16.5), (2.5, 14.0)], a);
            pen.fcircle(7.5, 10.5, 1.5, Color32::WHITE);
            pen.fcircle(12.5, 10.5, 1.5, Color32::WHITE);
        }
        // Open: a folder with its flap lifted.
        "open" => {
            pen.closed(&[(2.5, 5.0), (7.5, 5.0), (9.0, 6.5), (15.5, 6.5), (15.5, 9.0)], c);
            pen.line(&[(2.5, 5.0), (2.5, 16.0)]);
            pen.closed(&[(2.5, 16.0), (5.0, 9.0), (18.0, 9.0), (15.5, 16.0)], a);
        }
        // Print: a printer with a sheet coming out.
        "print" => {
            pen.rect(6.0, 2.5, 14.0, 7.0, c);
            pen.rect(2.5, 7.0, 17.5, 14.0, c);
            pen.closed(&[(6.0, 11.5), (14.0, 11.5), (14.0, 17.5), (6.0, 17.5)], a);
            pen.fcircle(15.0, 9.5, 0.8, c);
        }
        "alignText" => {
            // A box with its text lines in the middle, arrows to the top and bottom edges.
            pen.rect(3.0, 2.5, 17.0, 17.5, c);
            pen.lines(5.5, 11.0, &[9.0, 11.5]);
            pen.line_c(&[(14.0, 5.0), (14.0, 15.0)], a);
            pen.line_c(&[(12.5, 6.5), (14.0, 5.0), (15.5, 6.5)], a);
            pen.line_c(&[(12.5, 13.5), (14.0, 15.0), (15.5, 13.5)], a);
        }
        "crop" => {
            // Two right angles overlapping: the crop marks.
            pen.line_c(&[(6.0, 2.0), (6.0, 14.0), (18.0, 14.0)], c);
            pen.line_c(&[(2.0, 6.0), (14.0, 6.0), (14.0, 18.0)], a);
        }
        "textBoxLink" | "textBoxUnlink" => {
            // Two boxes, the text running from the first into the second (broken: cut through).
            pen.rect(2.0, 3.0, 8.5, 11.0, c);
            pen.rect(11.5, 9.0, 18.0, 17.0, c);
            pen.lines(3.5, 7.0, &[5.5, 8.0]);
            pen.line_c(&[(5.0, 11.0), (5.0, 14.0), (11.5, 14.0)], a);
            pen.line_c(&[(9.5, 12.5), (11.5, 14.0), (9.5, 15.5)], a);
            if name == "textBoxUnlink" {
                pen.line_c(&[(6.5, 16.5), (9.0, 11.5)], c);
            }
        }

        // The built-in file browser (#576): a folder, a file and the home folder.
        "folder" => {
            pen.frect(2.5, 4.5, 8.5, 7.0, yellow);
            pen.frect(2.5, 6.5, 17.5, 16.0, yellow);
            pen.closed(&[(2.5, 4.5), (8.0, 4.5), (9.5, 6.5), (17.5, 6.5), (17.5, 16.0), (2.5, 16.0)], c);
        }
        "file" => {
            pen.page(4.5, 2.0, 15.5, 18.0);
            pen.lines(7.0, 13.0, &[9.0, 12.0, 15.0]);
        }
        "homeFolder" => {
            pen.closed(&[(3.0, 9.5), (10.0, 3.5), (17.0, 9.5)], c);
            pen.closed(&[(5.0, 8.5), (5.0, 17.0), (15.0, 17.0), (15.0, 8.5)], c);
            pen.frect(8.5, 12.0, 11.5, 17.0, a);
        }

        // Outlining tab: arrows moving a paragraph between outline levels and along the outline.
        "outlinePromote" | "outlineDemote" => {
            pen.lines(9.0, 17.0, &[6.0, 10.0, 14.0]);
            if name == "outlinePromote" {
                pen.line_c(&[(7.0, 10.0), (2.5, 10.0)], a);
                pen.line_c(&[(5.0, 7.5), (2.5, 10.0), (5.0, 12.5)], a);
            } else {
                pen.line_c(&[(2.5, 10.0), (7.0, 10.0)], a);
                pen.line_c(&[(4.5, 7.5), (7.0, 10.0), (4.5, 12.5)], a);
            }
        }
        "outlineTop" | "outlineBody" => {
            pen.lines(11.0, 17.0, &[6.0, 10.0, 14.0]);
            let (x, d) = if name == "outlineTop" { (3.0, 1.0) } else { (9.0, -1.0) };
            for k in [0.0, 3.5] {
                pen.line_c(&[(x + d * k + d * 2.5, 7.0), (x + d * k, 10.0), (x + d * k + d * 2.5, 13.0)], a);
            }
        }
        "outlineUp" | "outlineDown" => {
            pen.lines(3.0, 11.0, &[6.0, 10.0, 14.0]);
            let up = name == "outlineUp";
            let (tip, tail) = if up { (3.0, 17.0) } else { (17.0, 3.0) };
            pen.line_c(&[(15.0, tail), (15.0, tip)], a);
            let wing = if up { tip + 3.0 } else { tip - 3.0 };
            pen.line_c(&[(12.0, wing), (15.0, tip), (18.0, wing)], a);
        }
        "outlineExpand" | "outlineCollapse" => {
            pen.fcircle(10.0, 10.0, 6.5, c);
            pen.line_c(&[(6.5, 10.0), (13.5, 10.0)], Color32::WHITE);
            if name == "outlineExpand" {
                pen.line_c(&[(10.0, 6.5), (10.0, 13.5)], Color32::WHITE);
            }
        }
        "closeOutline" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.line_c(&[(7.0, 8.0), (13.0, 14.0)], red);
            pen.line_c(&[(13.0, 8.0), (7.0, 14.0)], red);
        }
        "help" => {
            pen.circle(10.0, 10.0, 7.0, c);
            pen.text(10.0, 10.0, 10.0, "?", a, true);
        }
        // Developer › Controls: a field's frame with what it holds.
        "ccRichText" => {
            pen.rect(2.5, 5.0, 17.5, 15.0, a);
            pen.text(7.5, 10.0, 8.0, "A", c, true);
            pen.text(13.0, 10.5, 7.0, "a", c, false);
        }
        "ccPlainText" => {
            pen.rect(2.5, 5.0, 17.5, 15.0, a);
            pen.text(8.0, 10.0, 8.0, "A", c, false);
            pen.line(&[(12.0, 7.5), (12.0, 12.5)]);
        }
        "ccPicture" => {
            pen.rect(2.5, 4.0, 17.5, 16.0, a);
            pen.line(&[(4.5, 14.0), (8.5, 9.5), (11.5, 12.5), (13.5, 10.5), (15.5, 14.0)]);
            pen.fcircle(13.5, 7.0, 1.2, c);
        }
        "ccGallery" => {
            pen.rect(2.5, 3.0, 17.5, 17.0, a);
            pen.frect(5.0, 5.5, 9.0, 9.5, c);
            pen.frect(11.0, 5.5, 15.0, 9.5, c);
            pen.lines(5.0, 15.0, &[12.5, 15.0]);
        }
        "ccCheckBox" => {
            pen.rect(4.0, 4.0, 16.0, 16.0, c);
            pen.line_c(&[(6.5, 10.0), (9.0, 13.0), (14.0, 6.5)], a);
        }
        "ccComboBox" => {
            pen.rect(2.5, 6.0, 17.5, 14.0, c);
            pen.line(&[(13.0, 6.0), (13.0, 14.0)]);
            pen.lines(4.5, 10.5, &[10.0]);
            pen.arrow_down(15.25, 10.0, a);
        }
        "ccDropDown" => {
            pen.rect(2.5, 3.0, 17.5, 9.0, c);
            pen.line(&[(13.0, 3.0), (13.0, 9.0)]);
            pen.arrow_down(15.25, 6.0, a);
            pen.rect(2.5, 9.0, 13.0, 17.0, c);
            pen.lines(4.5, 11.0, &[11.5, 14.5]);
        }
        "ccDate" => {
            pen.rect(3.0, 4.5, 17.0, 17.0, c);
            pen.frect(3.0, 4.5, 17.0, 8.0, a);
            pen.line(&[(6.5, 3.0), (6.5, 6.0)]);
            pen.line(&[(13.5, 3.0), (13.5, 6.0)]);
            for (x, y) in [(6.5, 11.0), (10.0, 11.0), (13.5, 11.0), (6.5, 14.5), (10.0, 14.5)] {
                pen.fcircle(x, y, 0.9, c);
            }
        }
        "ccRepeat" => {
            pen.rect(2.5, 3.0, 17.5, 17.0, a);
            pen.rect(5.0, 5.5, 15.0, 9.5, c);
            pen.rect(5.0, 11.0, 15.0, 15.0, c);
            pen.line_c(&[(10.0, 15.0), (10.0, 18.5)], a);
        }
        "designMode" => {
            pen.line(&[(4.0, 16.0), (16.0, 16.0), (4.0, 4.0), (4.0, 16.0)]);
            pen.line_c(&[(4.0, 12.0), (8.0, 16.0)], a);
            pen.line(&[(7.0, 13.0), (7.0, 12.0)]);
        }
        _ => {
            // Fallback: a rounded tile with the first letter.
            p.rect_stroke(r.shrink(r.width() * 0.12), 3.0, Stroke::new(1.0, c), egui::StrokeKind::Middle);
            let ch = name.chars().next().map(|x| x.to_ascii_uppercase().to_string()).unwrap_or_default();
            p.text(r.center(), egui::Align2::CENTER_CENTER, ch, FontId::proportional(r.height() * 0.45), a);
        }
    }
}

/// Draw an Insert › Icons library icon into `r` in `c`, from the same path data the inserted
/// picture is rendered from (`wordcraft_render::icon_lib`).
pub fn library(p: &Painter, r: Rect, icon: &wordcraft_render::icon_lib::Icon, c: Color32) {
    use wordcraft_render::icon_lib::{GRID, STROKE};
    let s = r.width().min(r.height()) / GRID;
    let o = r.center() - vec2(GRID / 2.0 * s, GRID / 2.0 * s);
    let stroke = Stroke::new(STROKE * s, c);
    for piece in icon.pieces() {
        let pts: Vec<Pos2> = piece.points.iter().map(|q| pos2(o.x + q[0] * s, o.y + q[1] * s)).collect();
        if piece.fill {
            p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
        } else if piece.closed {
            p.add(Shape::closed_line(pts, stroke));
        } else {
            // Round caps, as on the inserted picture.
            for end in [pts.first(), pts.last()].into_iter().flatten() {
                p.circle_filled(*end, stroke.width / 2.0, c);
            }
            p.add(Shape::line(pts, stroke));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paints one icon into a square box on a fresh context and returns its shapes and the box.
    fn shapes(name: &str, box_size: f32, ppp: f32) -> (Vec<Shape>, Rect) {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(false, false));
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.set_pixels_per_point(ppp);
        // One priming frame: fonts and tokens are in place from the next frame on.
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let r = Rect::from_min_size(egui::Pos2::ZERO, vec2(box_size, box_size));
        let out = ctx.run_ui(egui::RawInput::default(), |ui| paint(ui.painter(), r, name, Color32::WHITE, Color32::RED));
        let shapes: Vec<Shape> = out.shapes.iter().map(|s| s.shape.clone()).collect();
        out.drop_without_applying_deltas();
        (shapes, r)
    }

    /// Every refined icon draws something, stays inside its paint box and has finite geometry — at
    /// the 17 pt small box, the 32 pt large box, and 1x, 1.5x and 2x device pixels.
    #[test]
    fn home_icons_draw_inside_their_box_at_every_scale() {
        for name in HOME_FAMILY {
            for size in [17.0, 32.0] {
                for ppp in [1.0, 1.5, 2.0] {
                    let (shapes, r) = shapes(name, size, ppp);
                    assert!(!shapes.is_empty(), "{name} at {size}px/{ppp}x drew nothing");
                    let mut bounds = Rect::NOTHING;
                    for s in &shapes {
                        let b = s.visual_bounding_rect();
                        assert!(
                            b.min.x.is_finite() && b.min.y.is_finite() && b.max.x.is_finite() && b.max.y.is_finite(),
                            "{name} at {size}px/{ppp}x has a non-finite shape: {b:?}"
                        );
                        bounds = bounds.union(b);
                    }
                    assert!(r.expand(1.5).contains_rect(bounds), "{name} at {size}px/{ppp}x drew outside its box: {bounds:?} in {r:?}");
                    assert!(bounds.width() > 1.0 && bounds.height() > 1.0, "{name} at {size}px/{ppp}x is degenerate: {bounds:?}");
                }
            }
        }
    }

    /// The stroke the Home family draws is at least one device pixel wide at every scale, so a thin
    /// line never fades out; other icons keep the set's own weight.
    #[test]
    fn home_strokes_stay_at_least_one_device_pixel() {
        for ppp in [1.0, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            let r = Rect::from_min_size(egui::Pos2::ZERO, vec2(17.0, 17.0));
            let home = Pen { p: &ctx.debug_painter(), r, w_scale: 17.0 / 20.0, c: Color32::WHITE, a: Color32::RED, w: HOME_STROKE, snap: true };
            let plain = Pen { p: &ctx.debug_painter(), r, w_scale: 17.0 / 20.0, c: Color32::WHITE, a: Color32::RED, w: 1.25, snap: false };
            let (hw, pw) = (home.stroke(Color32::WHITE).width, plain.stroke(Color32::WHITE).width);
            assert!((hw * ppp).round() >= 1.0, "the home stroke is {:.2}pt = {:.2} device pixels at {ppp}x", hw, hw * ppp);
            // The quantised weight may round down to a single crisp pixel; the *intended* weight is
            // the family's own, which is at least the set's.
            assert!(home.w * home.s() >= plain.w * plain.s(), "the family's weight is lighter than the set's");
            assert!((pw * ppp).round() >= 1.0, "the set's own stroke disappears at {ppp}x");
        }
    }

    /// Paste is the earlier original drawing with its hidden rear edges omitted: the board is drawn as
    /// two open paths that stop where the overlapping tinted sheet begins (so nothing shows through
    /// it), the clip, sheet and paper lines are unchanged, and the whole icon keeps the set's own
    /// 1.25-unit stroke and unsnapped coordinates at both paint sizes.
    #[test]
    fn paste_keeps_the_earlier_original_drawing() {
        for (size, ppp) in [(17.0f32, 2.0f32), (32.0, 2.0)] {
            let (shapes, r) = shapes("paste", size, ppp);
            let s = size / 20.0;
            let at = |x: f32, y: f32| egui::pos2(r.min.x + x * s, r.min.y + y * s);
            let want = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
            let same = |a: Rect, b: Rect| (a.min - b.min).length() < 0.01 && (a.max - b.max).length() < 0.01;
            let stroke = 1.25 * s;
            let rects: Vec<(Rect, Color32, Color32, f32)> = shapes
                .iter()
                .filter_map(|sh| match sh {
                    Shape::Rect(rs) => Some((rs.rect, rs.fill, rs.stroke.color, rs.stroke.width)),
                    _ => None,
                })
                .collect();
            let mut paths: Vec<(Vec<egui::Pos2>, f32)> = Vec::new();
            for sh in &shapes {
                match sh {
                    Shape::Path(p) if !p.closed => paths.push((p.points.clone(), p.stroke.width)),
                    Shape::LineSegment { points, stroke } => paths.push((points.to_vec(), stroke.width)),
                    _ => {}
                }
            }
            // The board: two open paths, the rear edges stopping before the sheet.
            let front = vec![at(4.0, 3.5), at(14.0, 3.5), at(14.0, 7.25)];
            let left = vec![at(4.0, 3.5), at(4.0, 17.0), at(8.25, 17.0)];
            let close = |a: &[egui::Pos2], b: &[egui::Pos2]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (*x - *y).length() < 0.01);
            assert!(
                paths.iter().any(|(p, w)| close(p, &front) && (*w - stroke).abs() < 0.01),
                "{size}pt: the board's top-right path changed: {paths:?}"
            );
            assert!(
                paths.iter().any(|(p, w)| close(p, &left) && (*w - stroke).abs() < 0.01),
                "{size}pt: the board's left-bottom path changed: {paths:?}"
            );
            // The old full rectangle must be gone.
            assert!(!rects.iter().any(|(b, ..)| same(*b, want(4.0, 3.5, 14.0, 17.0))), "{size}pt: the board is still a closed rectangle");
            // The clip, the tinted sheet and its outline are unchanged.
            assert!(
                rects.iter().any(|(b, f, _, _)| same(*b, want(7.0, 2.0, 11.0, 5.0)) && *f != Color32::TRANSPARENT),
                "{size}pt: the clip is missing: {rects:?}"
            );
            assert!(
                rects.iter().any(|(b, f, _, _)| same(*b, want(9.0, 8.0, 17.5, 18.5)) && *f != Color32::TRANSPARENT),
                "{size}pt: the tinted sheet is missing: {rects:?}"
            );
            assert!(
                rects.iter().any(|(b, _, c, w)| same(*b, want(9.0, 8.0, 17.5, 18.5)) && *c != Color32::TRANSPARENT && (*w - stroke).abs() < 0.01),
                "{size}pt: the sheet outline is missing: {rects:?}"
            );
            // Three text lines, unsnapped, spanning x 10.8 .. 15.8.
            for y in [11.0, 13.5, 16.0] {
                let wy = at(0.0, y).y;
                assert!(
                    paths.iter().any(|(p, _)| p.len() == 2 && (p[0].y - wy).abs() < 0.01 && (p[1].y - wy).abs() < 0.01),
                    "{size}pt: the line at y {y} is missing or was snapped: {paths:?}"
                );
            }
            assert!(paths.iter().all(|(_, w)| (*w - stroke).abs() < 0.01), "{size}pt: a path lost the set's stroke: {paths:?}");
            let (lx0, lx1) = (at(10.8, 0.0).x, at(15.8, 0.0).x);
            assert!(
                paths.iter().any(|(p, _)| p.len() == 2 && (p[0].x - lx0).abs() < 0.01 && (p[1].x - lx1).abs() < 0.01),
                "{size}pt: the line span changed: {paths:?}"
            );
        }
    }

    /// The board's *hidden* rear edges must not be drawn through the translucent sheet that overlaps
    /// them: every board edge is checked against the sheet's interior, while the three paper lines are
    /// the authorised strokes inside it. Expected to fail while the board is still a full rectangle.
    #[test]
    fn paste_hides_the_board_edges_behind_its_sheet() {
        for (size, ppp) in [(17.0f32, 2.0f32), (32.0, 2.0)] {
            let (shapes, r) = shapes("paste", size, ppp);
            let s = size / 20.0;
            let at = |x: f32, y: f32| (r.min.x + x * s, r.min.y + y * s);
            let rect_of = |x0: f32, y0: f32, x1: f32, y1: f32| {
                let (ax, ay) = at(x0, y0);
                let (bx, by) = at(x1, y1);
                Rect::from_min_max(egui::pos2(ax, ay), egui::pos2(bx, by))
            };
            let sheet = rect_of(9.0, 8.0, 17.5, 18.5);
            let board = rect_of(4.0, 3.5, 14.0, 17.0);
            let inner = sheet.shrink(0.5);
            let same = |a: Rect, b: Rect| (a.min - b.min).length() < 0.01 && (a.max - b.max).length() < 0.01;
            // Every stroke that belongs to the board: a full rectangle before the fix, two open
            // paths after it.
            let mut edges: Vec<(egui::Pos2, egui::Pos2)> = Vec::new();
            for sh in &shapes {
                match sh {
                    Shape::Rect(rs) if same(rs.rect, board) => {
                        let (a, b) = (rs.rect.min, rs.rect.max);
                        edges.push((a, egui::pos2(b.x, a.y)));
                        edges.push((egui::pos2(b.x, a.y), b));
                        edges.push((b, egui::pos2(a.x, b.y)));
                        edges.push((egui::pos2(a.x, b.y), a));
                    }
                    Shape::Path(p) if p.points.len() > 2 => {
                        for w in p.points.windows(2) {
                            if p.points.iter().all(|q| board.expand(0.01).contains(*q)) {
                                edges.push((w[0], w[1]));
                            }
                        }
                    }
                    _ => {}
                }
            }
            assert!(!edges.is_empty(), "{size}pt: the board's outline was not drawn at all");
            for (a, b) in &edges {
                // An axis-aligned edge shows through the sheet when it lies strictly inside it and
                // its span overlaps the interior; a diagonal edge is checked by its bounding box.
                let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
                let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
                let vertical = x1 - x0 < 0.01;
                let horizontal = y1 - y0 < 0.01;
                let visible = if vertical {
                    x0 > inner.min.x + 0.01 && x0 < inner.max.x - 0.01 && y1.min(inner.max.y) - y0.max(inner.min.y) > 0.01
                } else if horizontal {
                    y0 > inner.min.y + 0.01 && y0 < inner.max.y - 0.01 && x1.min(inner.max.x) - x0.max(inner.min.x) > 0.01
                } else {
                    x1.min(inner.max.x) - x0.max(inner.min.x) > 0.01 && y1.min(inner.max.y) - y0.max(inner.min.y) > 0.01
                };
                assert!(!visible, "{size}pt: a board edge {a:?}-{b:?} is drawn through the sheet's interior {inner:?}");
            }
            // The three paper lines are the authorised strokes inside the sheet.
            let mut papers = 0;
            for sh in &shapes {
                if let Shape::Path(p) = sh
                    && p.points.len() == 2
                    && p.points.iter().all(|q| inner.contains(*q))
                {
                    papers += 1;
                }
            }
            assert_eq!(papers, 3, "{size}pt: expected the three paper lines inside the sheet, found {papers}");
        }
    }

    /// The visible ink of every refined Home icon is the same apparent size in its tile: a 13.5-point
    /// longest extent at 17 points (device pixels 1x, 1.5x and 2x, where snapping rounds differently),
    /// centred on the paint box within a point and never escaping it. Paste is excluded — it keeps its
    /// earlier original drawing, which the Paste tests check.
    #[test]
    fn home_icons_are_optically_balanced_in_their_tile() {
        for name in HOME_FAMILY.iter().filter(|n| **n != "paste") {
            for ppp in [1.0, 1.5, 2.0] {
                let (shapes, r) = shapes(name, 17.0, ppp);
                let mut ink = Rect::NOTHING;
                for s in &shapes {
                    let b = s.visual_bounding_rect();
                    assert!(
                        b.min.x.is_finite() && b.min.y.is_finite() && b.max.x.is_finite() && b.max.y.is_finite(),
                        "{name} at {ppp}x has non-finite geometry"
                    );
                    ink = ink.union(b);
                }
                assert!(ink.width() > 1.0 && ink.height() > 1.0, "{name} at {ppp}x draws almost nothing: {ink:?}");
                let longest = ink.width().max(ink.height());
                assert!((longest - 13.5).abs() <= 1.0, "{name} at {ppp}x: longest visible ink is {longest:.2} points, not 13.5 +/- 1");
                let (dx, dy) = ((ink.center().x - r.center().x).abs(), (ink.center().y - r.center().y).abs());
                assert!(dx <= 1.0 && dy <= 1.0, "{name} at {ppp}x: ink centre {:?} is {dx:.2}/{dy:.2} points off the tile centre", ink.center());
                assert!(r.expand(1.5).contains_rect(ink), "{name} at {ppp}x: the ink escapes its tile: {ink:?}");
            }
        }
    }

    /// Normalising the size must not normalise the *shape*: a narrow `I`, a two-glyph `Aa`, the wide
    /// `ab`, a cursor and a pilcrow keep their own proportions instead of being stretched into squares.
    #[test]
    fn optical_sizing_keeps_each_icon_aspect_ratio() {
        let expected = [
            ("italic", 0.75),
            ("case", 1.59),
            ("strike", 1.68),
            ("select", 0.60),
            ("pilcrow", 0.64),
            ("dictate", 0.68),
            ("subscript", 1.14),
            ("charborder", 1.00),
        ];
        for (name, want) in expected {
            let (shapes, _) = shapes(name, 17.0, 2.0);
            let mut ink = Rect::NOTHING;
            for s in &shapes {
                ink = ink.union(s.visual_bounding_rect());
            }
            let got = ink.width() / ink.height();
            assert!((got - want).abs() < 0.15, "{name}: ink aspect {got:.2} does not match its own proportions {want:.2}");
        }
    }

    /// The italic I is a slanted stem with its own short bars, not the upright letter.
    #[test]
    fn italic_is_slanted_with_its_own_bars() {
        let (shapes, _) = shapes("italic", 32.0, 2.0);
        let segs: Vec<(egui::Pos2, egui::Pos2)> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) if p.points.len() == 2 => Some((p.points[0], p.points[1])),
                Shape::LineSegment { points, .. } => Some((points[0], points[1])),
                _ => None,
            })
            .collect();
        let flat = segs.iter().filter(|(a, b)| (a.y - b.y).abs() < 0.01).count();
        let slanted = segs.iter().filter(|(a, b)| (a.y - b.y).abs() > 4.0 && (a.x - b.x).abs() > 1.0).count();
        assert_eq!(flat, 2, "the italic I has a top and a bottom bar: {segs:?}");
        assert_eq!(slanted, 1, "the italic stem is slanted: {segs:?}");
        assert!(shapes.iter().all(|s| !matches!(s, Shape::Text(_))), "the italic I is drawn, not typed");
    }

    /// The letter inside the character border and the enclosure sits in the middle of its shape.
    #[test]
    fn bordered_and_enclosed_letters_are_centred() {
        for name in ["charborder", "enclose"] {
            let (shapes, r) = shapes(name, 32.0, 2.0);
            let letters: Vec<Rect> = shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Text(t) => Some(t.visual_bounding_rect()),
                    _ => None,
                })
                .collect();
            assert_eq!(letters.len(), 1, "{name} draws one letter: {shapes:#?}");
            let c = letters[0].center();
            assert!(
                (c.x - r.center().x).abs() < 1.5 && (c.y - r.center().y).abs() < 1.5,
                "{name}: the letter sits at {c:?}, not at the centre of {r:?}"
            );
        }
    }

    /// The text-effect letter keeps its accent colour: the shadow is painted behind it, not over it.
    #[test]
    fn effects_paints_its_accent_letter_last() {
        let (shapes, _) = shapes("effects", 32.0, 2.0);
        let colors: Vec<Color32> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Text(t) => Some(t.fallback_color),
                _ => None,
            })
            .collect();
        assert_eq!(colors.len(), 2, "effects draws a shadow and the letter: {shapes:#?}");
        assert_ne!(colors[0], colors[1], "the shadow and the letter are different colours");
        assert_eq!(colors[1], Color32::RED, "the accent letter is painted on top: {colors:?}");
    }
}
