//! Where a picture's frame sits and how text wraps around it (ODF 1.3 §19.11 `text:anchor-type`,
//! §20.2xx `style:wrap`, `style:run-through`, `style:horizontal-pos`/`-rel`,
//! `style:vertical-pos`/`-rel`, `fo:margin-*` of `style:graphic-properties`), and WordCraft's
//! [`Float`] for it, both ways.

use super::writer::pt;
use super::xml::{Attrs, get, length};
use wordcraft_doc::para::{Anchor, Float, FloatAlign, Wrap, WrapText};

/// Largest offset or distance read, points (22 inches).
const MAX_LEN: f32 = 1584.0;

/// The placement a graphic style states; each part `None` where it says nothing (its parent's
/// then counts).
#[derive(Clone, Debug, Default)]
pub(super) struct Graphic {
    wrap: Option<String>,
    run_through: Option<String>,
    hpos: Option<String>,
    hrel: Option<String>,
    vpos: Option<String>,
    vrel: Option<String>,
    /// `fo:margin-left`, `-right`, `-top`, `-bottom`.
    margins: [Option<f32>; 4],
}

impl Graphic {
    /// A `style:graphic-properties` element.
    pub(super) fn read(a: &Attrs) -> Graphic {
        let s = |k: &str| get(a, k).map(|v| v.trim().to_string());
        let m = |k: &str| get(a, k).and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, MAX_LEN));
        Graphic {
            wrap: s("style:wrap"),
            run_through: s("style:run-through"),
            hpos: s("style:horizontal-pos"),
            hrel: s("style:horizontal-rel"),
            vpos: s("style:vertical-pos"),
            vrel: s("style:vertical-rel"),
            margins: [m("fo:margin-left"), m("fo:margin-right"), m("fo:margin-top"), m("fo:margin-bottom")],
        }
    }

    /// This placement with `parent`'s filling what it leaves unsaid.
    pub(super) fn or(mut self, parent: &Graphic) -> Graphic {
        let fill = |a: &mut Option<String>, b: &Option<String>| {
            if a.is_none() {
                a.clone_from(b);
            }
        };
        fill(&mut self.wrap, &parent.wrap);
        fill(&mut self.run_through, &parent.run_through);
        fill(&mut self.hpos, &parent.hpos);
        fill(&mut self.hrel, &parent.hrel);
        fill(&mut self.vpos, &parent.vpos);
        fill(&mut self.vrel, &parent.vrel);
        for (m, p) in self.margins.iter_mut().zip(parent.margins) {
            *m = m.or(p);
        }
        self
    }
}

/// The [`Float`] of a frame anchored as `anchor` (`text:anchor-type`) at `x`, `y` (`svg:x`,
/// `svg:y`) with graphic style placement `g`. A frame in line with the text (`as-char`, or no
/// anchor stated) doesn't float.
pub(super) fn float_of(anchor: Option<&str>, x: Option<&str>, y: Option<&str>, g: &Graphic) -> Float {
    let anchor = anchor.map(str::trim);
    if matches!(anchor, None | Some("as-char")) {
        return Float::default();
    }
    let page = anchor == Some("page");
    let (wrap, wrap_text) = match g.wrap.as_deref() {
        Some("left") => (Wrap::Square, WrapText::Left),
        Some("right") => (Wrap::Square, WrapText::Right),
        Some("parallel") => (Wrap::Square, WrapText::BothSides),
        Some("dynamic") => (Wrap::Square, WrapText::Largest),
        Some("run-through") if g.run_through.as_deref() == Some("background") => (Wrap::BehindText, WrapText::BothSides),
        Some("run-through") => (Wrap::InFrontOfText, WrapText::BothSides),
        _ => (Wrap::TopAndBottom, WrapText::BothSides),
    };
    let h_rel = match g.hrel.as_deref() {
        Some("page") => Anchor::Page,
        Some("page-start-margin") => Anchor::LeftMargin,
        Some("page-end-margin") => Anchor::RightMargin,
        Some("page-content") => Anchor::Margin,
        Some("paragraph-content") => Anchor::Column,
        Some("char") => Anchor::Character,
        Some(_) => Anchor::Paragraph,
        None if page => Anchor::Page,
        None => Anchor::Paragraph,
    };
    let v_rel = match g.vrel.as_deref() {
        Some("page") => Anchor::Page,
        Some("page-content") => Anchor::Margin,
        Some("line") | Some("char") | Some("text") | Some("baseline") => Anchor::Line,
        Some(_) => Anchor::Paragraph,
        None if page => Anchor::Page,
        None => Anchor::Paragraph,
    };
    let h_align = match g.hpos.as_deref() {
        Some("left") => Some(FloatAlign::Start),
        Some("center") => Some(FloatAlign::Center),
        Some("right") => Some(FloatAlign::End),
        Some("inside") => Some(FloatAlign::Inside),
        Some("outside") => Some(FloatAlign::Outside),
        _ => None,
    };
    let v_align = match g.vpos.as_deref() {
        Some("top") => Some(FloatAlign::Start),
        Some("middle") => Some(FloatAlign::Center),
        Some("bottom") => Some(FloatAlign::End),
        _ => None,
    };
    let off = |v: Option<&str>| v.and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(-MAX_LEN, MAX_LEN)).unwrap_or(0.0);
    let [left, right, top, bottom] = g.margins;
    Float {
        wrap,
        wrap_text,
        h_rel,
        v_rel,
        x: if h_align.is_none() { off(x) } else { 0.0 },
        y: if v_align.is_none() { off(y) } else { 0.0 },
        h_align,
        v_align,
        dist: left.unwrap_or(0.0).max(right.unwrap_or(0.0)),
        dist_top: top.unwrap_or(0.0),
        dist_bottom: bottom.unwrap_or(0.0),
        ..Float::default()
    }
}

/// How a floating picture is written: its `text:anchor-type`, its graphic style's
/// `style:graphic-properties` attributes, and the frame's `svg:x`/`svg:y` attributes.
pub(super) fn frame_of(f: &Float) -> (&'static str, String, String) {
    let anchor = if f.h_rel == Anchor::Page && f.v_rel == Anchor::Page {
        "page"
    } else if f.v_rel == Anchor::Line {
        "char"
    } else {
        "paragraph"
    };
    let wrap = match (f.wrap, f.wrap_text) {
        (Wrap::BehindText, _) => "run-through\" style:run-through=\"background",
        (Wrap::InFrontOfText, _) => "run-through\" style:run-through=\"foreground",
        (Wrap::TopAndBottom, _) => "none",
        (_, WrapText::Left) => "left",
        (_, WrapText::Right) => "right",
        (_, WrapText::Largest) => "dynamic",
        _ => "parallel",
    };
    let hrel = match f.h_rel {
        Anchor::Page => "page",
        Anchor::Margin | Anchor::InsideMargin | Anchor::OutsideMargin => "page-content",
        Anchor::LeftMargin => "page-start-margin",
        Anchor::RightMargin => "page-end-margin",
        Anchor::Column => "paragraph-content",
        Anchor::Character => "char",
        _ => "paragraph",
    };
    let vrel = match f.v_rel {
        Anchor::Page | Anchor::TopMargin | Anchor::BottomMargin => "page",
        Anchor::Margin => "page-content",
        Anchor::Line => "line",
        _ => "paragraph",
    };
    let hpos = match f.h_align {
        Some(FloatAlign::Start) => "left",
        Some(FloatAlign::Center) => "center",
        Some(FloatAlign::End) => "right",
        Some(FloatAlign::Inside) => "inside",
        Some(FloatAlign::Outside) => "outside",
        None => "from-left",
    };
    let vpos = match f.v_align {
        Some(FloatAlign::Start) | Some(FloatAlign::Inside) => "top",
        Some(FloatAlign::Center) => "middle",
        Some(FloatAlign::End) | Some(FloatAlign::Outside) => "bottom",
        None => "from-top",
    };
    let d = |v: f32| pt(if v.is_finite() { v.clamp(0.0, MAX_LEN) } else { 0.0 });
    let props = format!(
        " style:wrap=\"{wrap}\" style:horizontal-pos=\"{hpos}\" style:horizontal-rel=\"{hrel}\" style:vertical-pos=\"{vpos}\" style:vertical-rel=\"{vrel}\" fo:margin-left=\"{}\" fo:margin-right=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\"",
        d(f.dist),
        d(f.dist),
        d(f.dist_top),
        d(f.dist_bottom)
    );
    let mut xy = String::new();
    let p = |v: f32| pt(if v.is_finite() { v.clamp(-MAX_LEN, MAX_LEN) } else { 0.0 });
    if f.h_align.is_none() {
        xy.push_str(&format!(" svg:x=\"{}\"", p(f.x)));
    }
    if f.v_align.is_none() {
        xy.push_str(&format!(" svg:y=\"{}\"", p(f.y)));
    }
    (anchor, props, xy)
}
