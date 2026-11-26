//! The ribbon icons that show a letter or a symbol, drawn as text in the interface font.
//!
//! The SVG icon documents have no `<text>` element (the framework's icon parser takes shapes only), so a
//! letter in them is a hand-built path that differs from the interface font's letter. These icons are drawn
//! as a framework value glyph instead: the 20-unit grid, the pen width 1.25 and the text positions of the
//! code-drawn set the icons were exported from. Original artwork; the letters come from the theme's fonts.

use craft::ui::primitives::Glyph;
use craft::ui::style::{UiTokens, rgba_color};
use egui::{Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

/// Every icon `paint` knows.
pub(super) static ICONS: &[TextIcon] = &[
    TextIcon("addressBlock"),
    TextIcon("case"),
    TextIcon("ccPlainText"),
    TextIcon("ccRichText"),
    TextIcon("charborder"),
    TextIcon("citation"),
    TextIcon("clear"),
    TextIcon("documentTemplate"),
    TextIcon("dropCap"),
    TextIcon("editor"),
    TextIcon("effects"),
    TextIcon("enclose"),
    TextIcon("endnote"),
    TextIcon("equation"),
    TextIcon("fontcolor"),
    TextIcon("fonts"),
    TextIcon("footnote"),
    TextIcon("formula"),
    TextIcon("greetingLine"),
    TextIcon("grow"),
    TextIcon("help"),
    TextIcon("highlightFields"),
    TextIcon("hyphenation"),
    TextIcon("index"),
    TextIcon("inkToMath"),
    TextIcon("language"),
    TextIcon("lineNumbers"),
    TextIcon("markCitation"),
    TextIcon("markEntry"),
    TextIcon("matchFields"),
    TextIcon("mergeField"),
    TextIcon("multilevel"),
    TextIcon("nextFootnote"),
    TextIcon("numbering"),
    TextIcon("pageNumber"),
    TextIcon("phonetic"),
    TextIcon("pilcrow"),
    TextIcon("preview"),
    TextIcon("readAloud"),
    TextIcon("replace"),
    TextIcon("rules"),
    TextIcon("shrink"),
    TextIcon("sort"),
    TextIcon("spelling"),
    TextIcon("strike"),
    TextIcon("styles"),
    TextIcon("stylesPane"),
    TextIcon("subscript"),
    TextIcon("superscript"),
    TextIcon("symbol"),
    TextIcon("tableOfAuthorities"),
    TextIcon("textBox"),
    TextIcon("textDirection"),
    TextIcon("textFill"),
    TextIcon("textOutline"),
    TextIcon("thesaurus"),
    TextIcon("transform"),
    TextIcon("translate"),
    TextIcon("watermark"),
    TextIcon("wordArt"),
    TextIcon("wordCount"),
    TextIcon("zoom100"),
];

/// An icon drawn as text and lines; the name picks the drawing.
#[derive(craft::ui::contract::MemoProps)]
pub(super) struct TextIcon(pub(super) &'static str);

impl Glyph for TextIcon {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, _mirrored: bool) {
        // The framework passes `text.disabled` as `color` to a disabled button, and a glyph has no other way
        // to learn that; then both inks go grey (the accent ink would stay bright on a disabled button).
        let disabled = color == rgba_color(tokens.color.text.disabled);
        let accent = if disabled { color } else { rgba_color(tokens.color.accent.solid) };
        let pen = Pen { p: painter, r: rect, c: color, a: accent, w: 1.25, d: disabled };
        paint(&pen, self.0);
    }
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    c: Color32,
    a: Color32,
    w: f32,
    /// Disabled: the fixed colours (a red bar, a green tick) go grey too, as an SVG's fixed layers do.
    d: bool,
}

impl Pen<'_> {
    /// `colour`, or the disabled ink when the button is disabled.
    fn fixed(&self, colour: Color32) -> Color32 {
        if self.d { self.c } else { colour }
    }
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
}

/// Draws `name`; false when it is not one of [`ICONS`].
fn paint(pen: &Pen, name: &str) -> bool {
    let (c, a) = (pen.c, pen.a);
    let red = pen.fixed(super::ART_RED);
    let green = pen.fixed(super::ART_GREEN);
    let orange = pen.fixed(super::ART_ORANGE);
    match name {
        "strike" => {
            pen.text(10.0, 10.0, 12.0, "ab", c, false);
            pen.line_c(&[(3.0, 10.5), (17.0, 10.5)], c);
        }
        "charborder" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, a);
            pen.text(10.0, 6.0, 9.0, "A", c, false);
        }
        "enclose" => {
            pen.circle(10.0, 10.5, 7.5, a);
            pen.text(10.0, 10.5, 9.0, "A", c, false);
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
            pen.fill(&[(10.0, 15.0), (14.0, 11.0), (18.0, 15.0), (14.0, 19.0)], pen.fixed(super::ART_PINK));
        }
        "phonetic" => {
            pen.text(10.0, 5.0, 7.5, "abc", a, false);
            pen.text(10.0, 13.5, 12.5, "A", c, false);
        }
        "fontcolor" => {
            pen.text(10.0, 9.0, 14.0, "A", c, false);
            pen.frect(2.0, 16.5, 18.0, 19.0, red);
        }
        "effects" => {
            pen.text(10.0, 10.0, 15.0, "A", a, true);
            let shadow = if pen.d { Color32::TRANSPARENT } else { super::ART_SHADOW };
            pen.text(10.5, 10.5, 15.0, "A", shadow, true);
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
        "sort" => {
            pen.text(6.0, 5.5, 7.0, "A", c, true);
            pen.text(6.0, 14.5, 7.0, "Z", c, true);
            pen.line_c(&[(14.0, 3.0), (14.0, 17.0)], a);
            pen.line_c(&[(11.5, 14.5), (14.0, 17.0), (16.5, 14.5)], a);
        }
        "pilcrow" => pen.text(10.0, 10.5, 15.0, "¶", c, false),
        "replace" => {
            pen.text(6.0, 6.0, 8.0, "ab", c, false);
            pen.text(13.0, 15.0, 8.0, "ac", a, false);
            pen.line(&[(13.0, 3.5), (16.0, 3.5), (16.0, 8.5)]);
            pen.line(&[(14.5, 7.0), (16.0, 8.5), (17.5, 7.0)]);
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
        "pageNumber" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.text(10.0, 13.0, 7.0, "#", a, true);
        }
        "textBox" => {
            pen.rect(3.0, 3.0, 17.0, 17.0, c);
            pen.text(10.0, 10.0, 10.0, "A", a, false);
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
        "equation" => pen.text(10.0, 10.0, 17.0, "π", a, false),
        "symbol" => pen.text(10.0, 10.0, 17.0, "Ω", c, false),
        "inkToMath" => {
            pen.line_c(&[(3.0, 12.0), (5.0, 7.0), (7.0, 12.0)], a);
            pen.text(13.5, 10.0, 11.0, "√", c, false);
        }
        "fonts" => pen.text(10.0, 10.0, 13.0, "Aa", a, false),
        "watermark" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.text(10.0, 11.0, 7.0, "W", a.linear_multiply(0.6), true);
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
        "citation" => {
            pen.text(6.0, 9.0, 14.0, "“", a, true);
            pen.lines(9.0, 17.0, &[6.0, 10.0, 14.0]);
        }
        "markEntry" | "index" | "markCitation" | "tableOfAuthorities" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.text(10.0, 10.0, 8.0, if name == "index" || name == "markEntry" { "A" } else { "§" }, a, true);
        }
        "mergeField" | "addressBlock" | "greetingLine" | "rules" | "matchFields" | "highlightFields" => {
            pen.rect(2.5, 6.0, 17.5, 14.0, c);
            pen.text(10.0, 10.0, 7.0, "«»", a, true);
        }
        "preview" => {
            pen.text(10.0, 10.0, 8.0, "ABC", a, true);
            pen.rect(2.5, 5.0, 17.5, 15.0, c);
        }
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
        "translate" | "language" => {
            pen.text(6.5, 7.0, 9.0, "A", c, true);
            // あ drawn as strokes, like the rest of the icon set: the interface fonts need not
            // carry Japanese (craft-fonts is optional), and a letter without a glyph drew a box.
            pen.line_c(&[(11.6, 10.8), (15.4, 10.8)], a);
            pen.line_c(&[(13.5, 8.8), (13.5, 13.9)], a);
            pen.line_c(&[(11.0, 13.9), (16.2, 13.9)], a);
            pen.circle(13.5, 16.1, 1.8, a);
            pen.line_c(&[(15.0, 17.1), (15.8, 18.0)], a);
        }
        "zoom100" => pen.text(10.0, 10.0, 7.5, "100", a, true),
        "documentTemplate" => {
            pen.page(4.0, 2.0, 16.0, 18.0);
            pen.frect(6.0, 5.0, 11.5, 7.0, a);
            pen.text(10.0, 13.0, 8.0, "A", a, false);
        }
        "textDirection" => {
            pen.text(9.0, 10.0, 10.0, "A", c, false);
            pen.line_c(&[(15.5, 4.0), (15.5, 16.0)], a);
            pen.line_c(&[(13.5, 14.0), (15.5, 16.0), (17.5, 14.0)], a);
        }
        "formula" => pen.text(10.0, 10.0, 12.0, "fx", a, false),
        "help" => {
            pen.circle(10.0, 10.0, 7.0, c);
            pen.text(10.0, 10.0, 10.0, "?", a, true);
        }
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
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name in `ICONS` has a drawing, and a name outside it has none.
    #[test]
    fn every_text_icon_has_a_drawing() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter().clone();
            let pen =
                Pen { p: &painter, r: Rect::from_min_size(pos2(0.0, 0.0), vec2(20.0, 20.0)), c: Color32::BLACK, a: Color32::BLUE, w: 1.25, d: false };
            for icon in ICONS {
                assert!(paint(&pen, icon.0), "{} has no drawing", icon.0);
            }
            assert!(!paint(&pen, "notAnIcon"));
        });
        out.textures_delta.clear();
    }

    /// Every colour the shapes of `shape` paint with.
    fn ink_of(shape: &Shape, out: &mut Vec<Color32>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| ink_of(s, out)),
            Shape::LineSegment { stroke, .. } => out.push(stroke.color),
            Shape::Path(p) => {
                out.push(p.fill);
                if let egui::epaint::ColorMode::Solid(c) = p.stroke.color {
                    out.push(c);
                }
            }
            Shape::Circle(c) => out.extend([c.stroke.color, c.fill]),
            Shape::Rect(r) => out.extend([r.stroke.color, r.fill]),
            Shape::Text(t) => {
                out.push(t.fallback_color);
                out.extend(t.galley.job.sections.iter().map(|s| s.format.color));
            }
            _ => {}
        }
    }

    /// A disabled button hands its glyph `text.disabled` as the colour: every shape is then in that colour
    /// or invisible, with no accent and no fixed colour such as the font colour's red bar (production painted
    /// a disabled icon all grey). Enabled, the glyphs do use the accent.
    #[test]
    fn a_disabled_text_glyph_paints_only_the_disabled_ink() {
        use craft::theme::{Appearance, Contrast, Mode, Size, TextSize};
        let themes = craft::theme::builtins();
        let theme = themes.iter().find(|theme| theme.id.as_str() == craft::theme::SUITE_DEFAULT).unwrap();
        let appearance = Appearance {
            mode: Mode::Light,
            contrast: Contrast::Standard,
            size: Size::Compact,
            text_size: TextSize::default(),
            reduced_motion: false,
        };
        let tokens = UiTokens::new(std::sync::Arc::new(craft::theme::resolve(theme, appearance, None).tokens), Contrast::Standard, Size::Compact);
        let (accent, disabled, normal) =
            (rgba_color(tokens.color.accent.solid), rgba_color(tokens.color.text.disabled), rgba_color(tokens.color.text.normal));
        assert_ne!(accent, disabled);
        let inks = |color: Color32| {
            let mut out = Vec::new();
            let ctx = egui::Context::default();
            let mut run = ctx.run_ui(egui::RawInput::default(), |ui| {
                let painter = ui.painter().clone();
                for icon in ICONS {
                    icon.paint(&painter, &tokens, Rect::from_min_size(pos2(0.0, 0.0), vec2(20.0, 20.0)), color, false);
                }
            });
            run.textures_delta.clear();
            run.shapes.iter().for_each(|clipped| ink_of(&clipped.shape, &mut out));
            out
        };
        assert!(inks(normal).contains(&accent), "an enabled glyph uses the accent ink");
        // A shade of the disabled ink (a lighter "W" behind a letter) counts as that ink.
        let same_hue = |ink: Color32| {
            let (x, y) = (ink.to_opaque(), disabled.to_opaque());
            [x.r().abs_diff(y.r()), x.g().abs_diff(y.g()), x.b().abs_diff(y.b())].iter().all(|d| *d <= 2)
        };
        let stray: Vec<Color32> = inks(disabled).into_iter().filter(|ink| ink.a() > 0 && !same_hue(*ink)).collect();
        assert!(stray.is_empty(), "a disabled glyph painted in another colour: {stray:?}");
    }
}
