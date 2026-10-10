//! WordCraft's icon set, drawn in code (original artwork; no external icon assets).
//!
//! Icons are designed on a 20×20 grid in two colours: line work (`c`) and an accent (`a`).

use egui::{Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    c: Color32,
    a: Color32,
    w: f32,
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
        Stroke::new(self.w * self.s(), c)
    }
    fn line(&self, pts: &[(f32, f32)]) {
        self.line_c(pts, self.c);
    }
    fn line_c(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(Shape::line(v, self.stroke(c)));
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
        self.p.rect_stroke(Rect::from_min_max(self.pt(x0, y0), self.pt(x1, y1)), 1.0, self.stroke(c), egui::StrokeKind::Middle);
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

/// Paint icon `name` into `r`. `c` is the line colour; the accent comes from the theme.
pub fn paint(p: &Painter, r: Rect, name: &str, c: Color32, accent: Color32) {
    let pen = Pen { p, r, c, a: accent, w: 1.25 };
    let a = accent;
    let red = Color32::from_rgb(0xD1, 0x3B, 0x3B);
    let yellow = Color32::from_rgb(0xF2, 0xC8, 0x11);
    let green = Color32::from_rgb(0x2E, 0x8B, 0x57);
    let orange = Color32::from_rgb(0xE0, 0x7B, 0x1F);
    match name {
        // Clipboard
        "paste" => {
            pen.rect(4.0, 3.5, 14.0, 17.0, c);
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
            pen.rect(3.5, 3.0, 12.5, 14.0, c);
            pen.frect(7.5, 6.5, 16.5, 17.5, Color32::TRANSPARENT);
            pen.rect(7.5, 6.5, 16.5, 17.5, a);
        }
        "painter" => {
            pen.frect(4.0, 3.0, 16.0, 8.0, yellow);
            pen.rect(4.0, 3.0, 16.0, 8.0, c);
            pen.line(&[(16.0, 5.5), (17.5, 5.5), (17.5, 10.0), (10.0, 10.0), (10.0, 12.0)]);
            pen.frect(8.8, 12.0, 11.2, 18.0, c);
        }
        // Font
        "bold" => pen.text(10.0, 10.5, 15.0, "B", c, true),
        "italic" => pen.text(10.0, 10.5, 15.0, "I", c, false),
        "underline" => {
            pen.text(10.0, 9.0, 14.0, "U", c, false);
            pen.line(&[(5.0, 17.0), (15.0, 17.0)]);
        }
        "strike" => {
            pen.text(10.0, 10.0, 12.0, "ab", c, false);
            pen.line_c(&[(3.0, 10.5), (17.0, 10.5)], c);
        }
        "charborder" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, a);
            pen.text(10.0, 6.0, 9.0, "A", c, false);
        }
        "subscript" => {
            pen.text(8.0, 9.0, 13.0, "x", c, false);
            pen.text(15.0, 14.5, 8.0, "2", a, true);
        }
        "superscript" => {
            pen.text(8.0, 11.0, 13.0, "x", c, false);
            pen.text(15.0, 5.0, 8.0, "2", a, true);
        }
        "grow" => {
            pen.text(8.0, 11.0, 15.0, "A", c, false);
            pen.fill(&[(14.5, 6.0), (18.5, 6.0), (16.5, 3.0)], a);
        }
        "shrink" => {
            pen.text(8.0, 12.0, 11.0, "A", c, false);
            pen.fill(&[(14.5, 4.0), (18.5, 4.0), (16.5, 7.0)], a);
        }
        "case" => pen.text(10.0, 10.5, 12.0, "Aa", c, false),
        "clear" => {
            pen.text(8.0, 9.0, 13.0, "A", c, false);
            pen.fill(&[(10.0, 15.0), (14.0, 11.0), (18.0, 15.0), (14.0, 19.0)], Color32::from_rgb(0xE8, 0x6F, 0xA0));
        }
        "highlight" => {
            pen.fill(&[(5.0, 12.0), (11.0, 4.0), (14.0, 7.0), (8.0, 15.0)], c);
            pen.line(&[(5.0, 12.0), (3.5, 15.0), (8.0, 15.0)]);
            pen.frect(2.0, 16.5, 18.0, 19.0, yellow);
        }
        "fontcolor" => {
            pen.text(10.0, 9.0, 14.0, "A", c, false);
            pen.frect(2.0, 16.5, 18.0, 19.0, red);
        }
        "effects" => {
            pen.text(10.0, 10.0, 15.0, "A", a, true);
            pen.text(10.5, 10.5, 15.0, "A", Color32::from_rgba_unmultiplied(0, 0, 0, 40), true);
        }
        "fontdialog" => pen.line(&[(13.0, 13.0), (17.0, 17.0), (17.0, 13.0)]),
        // Paragraph
        "bullets" => {
            for (i, y) in [5.0, 10.0, 15.0].iter().enumerate() {
                pen.fcircle(4.0, *y, 1.4, if i == 0 { a } else { c });
                pen.line(&[(7.5, *y), (17.0, *y)]);
            }
        }
        "numbering" => {
            for (i, y) in [5.0, 10.0, 15.0].iter().enumerate() {
                pen.text(4.0, *y, 6.0, &(i + 1).to_string(), if i == 0 { a } else { c }, true);
                pen.line(&[(7.5, *y), (17.0, *y)]);
            }
        }
        "multilevel" => {
            pen.text(3.5, 4.5, 5.5, "1", a, true);
            pen.line(&[(6.5, 4.5), (17.0, 4.5)]);
            pen.text(6.5, 10.0, 5.5, "a", c, true);
            pen.line(&[(9.5, 10.0), (17.0, 10.0)]);
            pen.text(9.5, 15.5, 5.5, "i", c, true);
            pen.line(&[(12.0, 15.5), (17.0, 15.5)]);
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
            pen.text(6.0, 5.5, 7.0, "A", c, true);
            pen.text(6.0, 14.5, 7.0, "Z", c, true);
            pen.line_c(&[(14.0, 3.0), (14.0, 17.0)], a);
            pen.line_c(&[(11.5, 14.5), (14.0, 17.0), (16.5, 14.5)], a);
        }
        "pilcrow" => pen.text(10.0, 10.5, 15.0, "¶", c, false),
        "alignLeft" => {
            pen.lines(3.0, 17.0, &[4.0, 10.0, 16.0]);
            ().then_lines(&pen, 3.0, 12.0, &[7.0, 13.0])
        }
        "alignCenter" => {
            pen.lines(3.0, 17.0, &[4.0, 10.0, 16.0]);
            ().then_lines(&pen, 6.0, 14.0, &[7.0, 13.0])
        }
        "alignRight" => {
            pen.lines(3.0, 17.0, &[4.0, 10.0, 16.0]);
            ().then_lines(&pen, 8.0, 17.0, &[7.0, 13.0])
        }
        "justify" => pen.lines(3.0, 17.0, &[4.0, 7.0, 10.0, 13.0, 16.0]),
        "lineSpacing" => {
            pen.lines(9.0, 17.0, &[5.0, 10.0, 15.0]);
            pen.line_c(&[(4.5, 3.0), (4.5, 17.0)], a);
            pen.line_c(&[(2.5, 5.0), (4.5, 3.0), (6.5, 5.0)], a);
            pen.line_c(&[(2.5, 15.0), (4.5, 17.0), (6.5, 15.0)], a);
        }
        "shading" => {
            pen.fill(&[(4.0, 9.0), (10.0, 3.0), (16.0, 9.0), (10.0, 15.0)], Color32::TRANSPARENT);
            pen.closed(&[(4.0, 9.0), (10.0, 3.0), (16.0, 9.0), (10.0, 15.0)], c);
            pen.fcircle(16.5, 13.5, 1.6, a);
            pen.frect(2.0, 16.5, 18.0, 19.0, yellow);
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
            pen.fill(&[(5.0, 3.0), (5.0, 16.0), (8.0, 13.0), (10.5, 18.0), (12.5, 17.0), (10.0, 12.0), (14.0, 12.0)], c);
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
        "chart" => {
            pen.frect(3.5, 11.0, 6.5, 17.0, a);
            pen.frect(8.5, 6.0, 11.5, 17.0, orange);
            pen.frect(13.5, 9.0, 16.5, 17.0, green);
            pen.line(&[(2.0, 17.5), (18.0, 17.5)]);
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
        "sideBySide" | "syncScroll" | "switchWindows" => {
            pen.rect(2.0, 4.0, 9.5, 16.0, c);
            pen.rect(10.5, 4.0, 18.0, 16.0, a);
        }
        "macros" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.fill(&[(8.0, 6.5), (8.0, 13.5), (14.0, 10.0)], a);
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
        "dropdown" => pen.arrow_down(10.0, 10.0, c),
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
        "help" => {
            pen.circle(10.0, 10.0, 7.0, c);
            pen.text(10.0, 10.0, 10.0, "?", a, true);
        }
        _ => {
            // Fallback: a rounded tile with the first letter.
            p.rect_stroke(r.shrink(r.width() * 0.12), 3.0, Stroke::new(1.0, c), egui::StrokeKind::Middle);
            let ch = name.chars().next().map(|x| x.to_ascii_uppercase().to_string()).unwrap_or_default();
            p.text(r.center(), egui::Align2::CENTER_CENTER, ch, FontId::proportional(r.height() * 0.45), a);
        }
    }
}

trait ThenLines {
    fn then_lines(self, pen: &Pen, x0: f32, x1: f32, ys: &[f32]);
}

impl ThenLines for () {
    fn then_lines(self, pen: &Pen, x0: f32, x1: f32, ys: &[f32]) {
        pen.lines(x0, x1, ys);
    }
}
