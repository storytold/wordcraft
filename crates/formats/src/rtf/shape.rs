//! Drawing shapes and text boxes (`\shp`, #6212). Word describes a shape in `\shpinst`: its
//! place (`\shpleft`… in twips, against `\shpbx…`/`\shpby…` or the `posrelh`/`posrelv`
//! properties), its wrapping (`\shpwr`, `\shpwrk`, `\shpfblwtxt`), its `\sp` name/value
//! properties and a text box's text (`\shptxt`); `\shprslt` holds a copy for readers without
//! shapes. A shape whose type is known here is read from `\shpinst` and its `\shprslt` skipped;
//! any other (pictures, lines, shapes in a group) keeps the copy, as before.

use wordcraft_doc::fill::Fill;
use wordcraft_doc::para::{Anchor, Float, FloatAlign, ShapeKind, Wrap, WrapText};

use super::reader::Reader;
use super::state::{Dest, MAX_NOTE_BLOCKS, Output};
use super::{FBlock, Inline, Rgb, model};

/// Most characters kept of a property's name or value (hostile input).
const MAX_PROP: usize = 64;

/// Twips in a point, and EMUs in a point.
const TWIPS: f32 = 20.0;
const EMUS: f32 = 12_700.0;

/// A `\shp` group being read.
#[derive(Default)]
pub(super) struct ShapeSt {
    /// The depths of the `\shp` group, of its `\shpinst` and of the `\shptxt` being read.
    shp_depth: usize,
    inst_depth: Option<usize>,
    txt_depth: Option<usize>,
    /// The output set aside while `\shptxt` is read, and the text read.
    saved: Option<Box<Output>>,
    blocks: Vec<FBlock>,
    /// The shape was read from `\shpinst`: its `\shprslt` copy is skipped.
    read: bool,
    /// `\shpleft`, `\shptop`, `\shpright`, `\shpbottom` (twips).
    rect: [i64; 4],
    bx: Option<Anchor>,
    by: Option<Anchor>,
    bx_ignore: bool,
    by_ignore: bool,
    wr: Option<i32>,
    wrk: i32,
    behind: bool,
    /// The `\sn` and `\sv` of the property being read.
    sn: String,
    sv: String,
    props: Props,
}

/// The `\sp` properties read.
#[derive(Default)]
struct Props {
    shape_type: Option<i64>,
    fill: Option<Rgb>,
    filled: Option<bool>,
    opacity: Option<i64>,
    line_color: Option<Rgb>,
    line: Option<bool>,
    line_width: Option<i64>,
    posrelh: Option<i64>,
    posrelv: Option<i64>,
    posh: Option<i64>,
    posv: Option<i64>,
    pseudo_inline: bool,
    fit: bool,
    hidden: bool,
    flip_h: bool,
    flip_v: bool,
    rotation: Option<i64>,
    behind: Option<bool>,
    /// `dxTextLeft`, `dyTextTop`, `dxTextRight`, `dyTextBottom` (EMUs).
    inset: [Option<i64>; 4],
    /// `dxWrapDistLeft`, `dyWrapDistTop`, `dyWrapDistBottom` (EMUs).
    dist: [Option<i64>; 3],
}

/// A colour property (a COLORREF, `0x00BBGGRR`); `None` for a scheme or system colour.
fn colorref(v: i64) -> Option<Rgb> {
    (0..=0xFF_FFFF).contains(&v).then_some(Rgb((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8))
}

/// The shapes read from `\shpinst`; the others keep their `\shprslt` copy.
fn kind(shape_type: i64) -> Option<ShapeKind> {
    Some(match shape_type {
        1 => ShapeKind::Rectangle,
        2 => ShapeKind::RoundedRectangle,
        3 => ShapeKind::Ellipse,
        4 => ShapeKind::Diamond,
        5 => ShapeKind::Triangle,
        6 => ShapeKind::RightTriangle,
        13 => ShapeKind::Arrow,
        66 => ShapeKind::ArrowLeft,
        67 => ShapeKind::ArrowDown,
        68 => ShapeKind::ArrowUp,
        202 => ShapeKind::TextBox,
        _ => return None,
    })
}

/// `posh`/`posv`: 0 is an offset, then left/top, centre, right/bottom, inside, outside.
fn align(v: Option<i64>) -> Option<FloatAlign> {
    match v? {
        1 => Some(FloatAlign::Start),
        2 => Some(FloatAlign::Center),
        3 => Some(FloatAlign::End),
        4 => Some(FloatAlign::Inside),
        5 => Some(FloatAlign::Outside),
        _ => None,
    }
}

/// A length in points from `v` in `per` units to the point, held to a page's reach.
fn points(v: i64, per: f32) -> f32 {
    (v.clamp(-31_680 * 12_700, 31_680 * 12_700) as f32 / per).clamp(-1584.0, 1584.0)
}

impl Reader {
    /// A shape word, read here when it is one: true then.
    pub(super) fn shape_word(&mut self, w: &str, p: Option<i32>, first_in_group: bool) -> bool {
        match self.st.dest {
            Dest::ShpInst => {
                self.shape_inst_word(w, p, first_in_group);
                true
            }
            // A group in a name or value (a picture's `pib`) is not read.
            Dest::ShpSn | Dest::ShpSv => {
                if first_in_group {
                    self.st.dest = Dest::Skip;
                }
                true
            }
            Dest::Body => self.shape_body_word(w),
            _ => false,
        }
    }

    fn shape_body_word(&mut self, w: &str) -> bool {
        let depth = self.depth();
        match w {
            // A group's shapes are placed in the group's own coordinates: its copy is read.
            "shpgrp" => {
                self.st.in_shpgrp = true;
                false
            }
            "shp" if self.boxes_on && self.shape.is_none() && self.sub.is_none() && !self.st.in_shpgrp && self.overflow == 0 => {
                self.shape = Some(Box::new(ShapeSt { shp_depth: depth, ..Default::default() }));
                true
            }
            "shpinst" => match self.shape.as_deref_mut() {
                Some(s) if s.inst_depth.is_none() && depth == s.shp_depth + 1 => {
                    s.inst_depth = Some(depth);
                    self.st.dest = Dest::ShpInst;
                    true
                }
                _ => false,
            },
            "shprslt" if self.shape.as_deref().is_some_and(|s| s.read && depth == s.shp_depth + 1) => {
                self.st.dest = Dest::Skip;
                true
            }
            _ => false,
        }
    }

    fn shape_inst_word(&mut self, w: &str, p: Option<i32>, first_in_group: bool) {
        let depth = self.depth();
        let pv = p.unwrap_or(0);
        let Some(s) = self.shape.as_deref_mut() else {
            self.st.dest = Dest::Skip;
            return;
        };
        match w {
            "shpleft" => s.rect[0] = i64::from(pv),
            "shptop" => s.rect[1] = i64::from(pv),
            "shpright" => s.rect[2] = i64::from(pv),
            "shpbottom" => s.rect[3] = i64::from(pv),
            "shpbxpage" => s.bx = Some(Anchor::Page),
            "shpbxmargin" => s.bx = Some(Anchor::Margin),
            "shpbxcolumn" => s.bx = Some(Anchor::Column),
            "shpbxignore" => s.bx_ignore = true,
            "shpbypage" => s.by = Some(Anchor::Page),
            "shpbymargin" => s.by = Some(Anchor::Margin),
            "shpbypara" => s.by = Some(Anchor::Paragraph),
            "shpbyignore" => s.by_ignore = true,
            "shpwr" => s.wr = Some(pv),
            "shpwrk" => s.wrk = pv,
            "shpfblwtxt" => s.behind = p != Some(0),
            "sp" => {}
            "sn" => {
                s.sn.clear();
                self.st.dest = Dest::ShpSn;
            }
            "sv" => {
                s.sv.clear();
                self.st.dest = Dest::ShpSv;
            }
            "shptxt" if s.saved.is_none() && s.txt_depth.is_none() => {
                s.txt_depth = Some(depth);
                let saved = self.swap_output(Output::default());
                if let Some(s) = self.shape.as_deref_mut() {
                    s.saved = Some(Box::new(saved));
                }
                self.st.dest = Dest::Body;
                self.st.intbl = false;
            }
            _ if first_in_group => self.st.dest = Dest::Skip,
            _ => {}
        }
    }

    /// `\defshp` in a picture's `\picprop` (which is otherwise skipped): the picture is the one
    /// Word writes of a shape for readers without shapes.
    pub(super) fn note_defshp(&mut self, w: &str) {
        if w == "defshp" && self.pict_depth.is_some() {
            self.pict_defshp = true;
        }
    }

    /// Text of a property's name or value.
    pub(super) fn shape_text(&mut self, t: &str) {
        let name = self.st.dest == Dest::ShpSn;
        if let Some(s) = self.shape.as_deref_mut() {
            let into = if name { &mut s.sn } else { &mut s.sv };
            for c in t.chars() {
                if into.len() >= MAX_PROP {
                    break;
                }
                into.push(c);
            }
        }
    }

    /// Whether a text box's text is being read.
    pub(super) fn in_shape_text(&self) -> bool {
        self.shape.as_deref().is_some_and(|s| s.saved.is_some())
    }

    /// Whether the group at the current depth is the `\shptxt` being read.
    pub(super) fn closes_shape_text(&self) -> bool {
        self.shape.as_deref().is_some_and(|s| s.txt_depth == Some(self.depth()))
    }

    /// Whether the group at the current depth is the shape's `\shpinst`.
    pub(super) fn closes_shape_inst(&self) -> bool {
        self.shape.as_deref().is_some_and(|s| s.inst_depth == Some(self.depth()))
    }

    /// After a group closed: the shape whose `\shp` group it was is done.
    pub(super) fn after_shape_group(&mut self) {
        if self.shape_read_at.is_some_and(|d| self.depth() < d) {
            self.shape_read_at = None;
        }
        if self.shape.as_deref().is_some_and(|s| self.depth() < s.shp_depth) {
            self.end_shape_text();
            // Only a shape read has its picture skipped: one that wasn't keeps the next picture.
            let read = self.shape.take().is_some_and(|s| s.read);
            self.shape_read_at = read.then_some(self.depth());
        }
    }

    /// The `\shptxt` group closed (or the file ended in it): keep its text for the shape and go
    /// back to the output it was read from.
    pub(super) fn end_shape_text(&mut self) {
        let Some(saved) = self.shape.as_deref_mut().and_then(|s| {
            s.txt_depth = None;
            s.saved.take()
        }) else {
            return;
        };
        if self.para_has || !self.para.inlines.is_empty() {
            self.end_para(false);
        }
        self.close_table();
        let mut inner = self.swap_output(*saved);
        inner.body.truncate(MAX_NOTE_BLOCKS);
        if let Some(s) = self.shape.as_deref_mut() {
            s.blocks = inner.body;
        }
    }

    /// A `\sv` group closed: the property it gives.
    pub(super) fn commit_shape_prop(&mut self) {
        let Some(s) = self.shape.as_deref_mut() else { return };
        let v = s.sv.trim().parse::<i64>().ok();
        let on = v.map(|v| v != 0);
        let p = &mut s.props;
        match s.sn.trim() {
            "shapeType" => p.shape_type = v,
            "fillColor" => p.fill = v.and_then(colorref),
            "fFilled" => p.filled = on,
            "fillOpacity" => p.opacity = v,
            "lineColor" => p.line_color = v.and_then(colorref),
            "fLine" => p.line = on,
            "lineWidth" => p.line_width = v,
            "posrelh" => p.posrelh = v,
            "posrelv" => p.posrelv = v,
            "posh" => p.posh = v,
            "posv" => p.posv = v,
            "fPseudoInline" => p.pseudo_inline = on == Some(true),
            "fFitShapeToText" => p.fit = on == Some(true),
            "fHidden" => p.hidden = on == Some(true),
            "fFlipH" => p.flip_h = on == Some(true),
            "fFlipV" => p.flip_v = on == Some(true),
            "rotation" => p.rotation = v,
            "fBehindDocument" => p.behind = on,
            "dxTextLeft" => p.inset[0] = v,
            "dyTextTop" => p.inset[1] = v,
            "dxTextRight" => p.inset[2] = v,
            "dyTextBottom" => p.inset[3] = v,
            "dxWrapDistLeft" => p.dist[0] = v,
            "dyWrapDistTop" => p.dist[1] = v,
            "dyWrapDistBottom" => p.dist[2] = v,
            _ => {}
        }
        s.sn.clear();
        s.sv.clear();
    }

    /// The `\shpinst` group closed: a shape of a known type goes in the paragraph, and its
    /// `\shprslt` copy will be skipped.
    pub(super) fn finish_shape(&mut self) {
        self.end_shape_text();
        let Some(s) = self.shape.as_deref_mut() else { return };
        let Some(kind) = s.props.shape_type.and_then(kind) else { return };
        let p = &s.props;
        let [l, t, r, b] = s.rect;
        let (w, h) = (points(r.saturating_sub(l), TWIPS), points(b.saturating_sub(t), TWIPS));
        let mut look = model::BoxLook::default();
        if p.filled == Some(false) {
            look.fill = None;
        } else {
            let transparency = p.opacity.map_or(0.0, |o| 1.0 - (o.clamp(0, 65_536) as f32 / 65_536.0));
            look.fill = Some(Fill::Solid { color: p.fill.unwrap_or(Rgb::WHITE), transparency });
        }
        look.line = if p.line == Some(false) {
            None
        } else {
            Some((p.line_color.unwrap_or(Rgb::BLACK), p.line_width.map_or(0.75, |w| points(w, EMUS).max(0.0))))
        };
        if p.inset.iter().any(Option::is_some) {
            let [il, it, ir, ib] = p.inset;
            let get = |v: Option<i64>, d: f32| v.map_or(d, |v| points(v, EMUS).max(0.0));
            look.inset = Some([get(il, 7.2), get(it, 3.6), get(ir, 7.2), get(ib, 3.6)]);
        }
        look.auto_fit = p.fit;
        let mut f = Float {
            flip_h: p.flip_h,
            flip_v: p.flip_v,
            rot: p.rotation.map_or(0.0, |r| (r.clamp(-360 * 65_536, 360 * 65_536) as f32 / 65_536.0).rem_euclid(360.0)),
            hidden: p.hidden,
            ..Float::default()
        };
        if !p.pseudo_inline {
            let behind = p.behind.unwrap_or(s.behind);
            f.wrap = match s.wr {
                Some(1) => Wrap::TopAndBottom,
                Some(2) => Wrap::Square,
                Some(4) => Wrap::Tight,
                Some(5) => Wrap::Through,
                _ if behind => Wrap::BehindText,
                _ => Wrap::InFrontOfText,
            };
            f.wrap_text = match s.wrk {
                1 => WrapText::Left,
                2 => WrapText::Right,
                3 => WrapText::Largest,
                _ => WrapText::BothSides,
            };
            let rel_h = p.posrelh.and_then(|v| match v {
                0 => Some(Anchor::Margin),
                1 => Some(Anchor::Page),
                2 => Some(Anchor::Column),
                3 => Some(Anchor::Character),
                _ => None,
            });
            let rel_v = p.posrelv.and_then(|v| match v {
                0 => Some(Anchor::Margin),
                1 => Some(Anchor::Page),
                2 => Some(Anchor::Paragraph),
                3 => Some(Anchor::Line),
                _ => None,
            });
            // `\shpbxignore` and `\shpbyignore` hand the anchor to the properties.
            f.h_rel = if s.bx_ignore { rel_h.or(s.bx) } else { s.bx.or(rel_h) }.unwrap_or(Anchor::Column);
            f.v_rel = if s.by_ignore { rel_v.or(s.by) } else { s.by.or(rel_v) }.unwrap_or(Anchor::Paragraph);
            f.x = points(l, TWIPS);
            f.y = points(t, TWIPS);
            f.h_align = align(p.posh);
            f.v_align = align(p.posv);
            let dist = |v: Option<i64>, d: f32| v.map_or(d, |v| points(v, EMUS).max(0.0));
            f.dist = dist(p.dist[0], 9.0);
            f.dist_top = dist(p.dist[1], 0.0);
            f.dist_bottom = dist(p.dist[2], 0.0);
        }
        let blocks = std::mem::take(&mut s.blocks);
        s.read = true;
        self.para.inlines.push(Inline::Shape { kind, w, h, float: Some(Box::new(f)), blocks, look, path: None });
        self.para_has = true;
    }
}
