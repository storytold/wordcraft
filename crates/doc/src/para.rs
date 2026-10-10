//! Paragraphs: text with formatting runs and inline objects.
//!
//! A paragraph's text is one `String`. Formatting is a list of [`Run`]s whose byte lengths sum to
//! the text length. Inline objects (pictures, fields, note references, bookmarks, comment
//! anchors…) sit in the text as U+FFFC; the k-th U+FFFC is `objects[k]`. Special characters:
//! `\t` tab, `\n` manual line break, U+000C page break, U+000E column break.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::props::{CharProps, ParaProps};
use crate::section::SectionProps;
use crate::{DocError, Result};

/// Object replacement character: marks an inline object in paragraph text.
pub const OBJ: char = '\u{FFFC}';
/// Manual line break (Shift+Enter).
pub const LINE_BREAK: char = '\n';
/// Page break (Ctrl+Enter).
pub const PAGE_BREAK: char = '\u{000C}';
/// Column break (Ctrl+Shift+Enter).
pub const COLUMN_BREAK: char = '\u{000E}';
/// Optional hyphen (Ctrl+-).
pub const SOFT_HYPHEN: char = '\u{00AD}';
/// Nonbreaking hyphen (Ctrl+Shift+-).
pub const NB_HYPHEN: char = '\u{2011}';
/// Nonbreaking space (Ctrl+Shift+Space).
pub const NBSP: char = '\u{00A0}';

static REV: AtomicU64 = AtomicU64::new(1);

/// A fresh revision number (process-unique), used by layout caches to detect edits.
pub fn next_rev() -> u64 {
    REV.fetch_add(1, Ordering::Relaxed)
}

/// A formatting run: `len` bytes of text sharing `props`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub len: usize,
    #[serde(default, skip_serializing_if = "CharProps::is_empty")]
    pub props: CharProps,
}

/// How a floating object is placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Wrap {
    /// In line with text (not floating).
    #[default]
    Inline,
    Square,
    Tight,
    Through,
    TopAndBottom,
    BehindText,
    InFrontOfText,
}

/// What a floating object's offset is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    #[default]
    Column,
    Margin,
    Page,
    Paragraph,
    /// The page's left margin area (from the page's left edge to the text).
    LeftMargin,
    /// The page's right margin area (from the text to the page's right edge).
    RightMargin,
    /// The page's top margin area (from the page's top edge to the text).
    TopMargin,
    /// The page's bottom margin area (from the text to the page's bottom edge).
    BottomMargin,
    /// The inside margin (left on odd pages, right on even pages when mirrored).
    InsideMargin,
    /// The outside margin.
    OutsideMargin,
    /// The anchor character (horizontal only).
    Character,
    /// The anchor line (vertical only).
    Line,
}

/// Alignment of a floating object within its [`Anchor`] area, instead of an offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FloatAlign {
    /// Left or top.
    Start,
    Center,
    /// Right or bottom.
    End,
    Inside,
    Outside,
}

impl FloatAlign {
    /// From an OOXML alignment (`wp:align`, `w:tblpXSpec`, `w:tblpYSpec`).
    pub fn from_ooxml(v: &str) -> Option<FloatAlign> {
        match v {
            "left" | "top" => Some(FloatAlign::Start),
            "center" => Some(FloatAlign::Center),
            "right" | "bottom" => Some(FloatAlign::End),
            "inside" => Some(FloatAlign::Inside),
            "outside" => Some(FloatAlign::Outside),
            _ => None,
        }
    }

    /// The OOXML name on the horizontal (`left`, `right`) or vertical (`top`, `bottom`) axis.
    pub fn ooxml(self, horizontal: bool) -> &'static str {
        match (self, horizontal) {
            (FloatAlign::Start, true) => "left",
            (FloatAlign::Start, false) => "top",
            (FloatAlign::Center, _) => "center",
            (FloatAlign::End, true) => "right",
            (FloatAlign::End, false) => "bottom",
            (FloatAlign::Inside, _) => "inside",
            (FloatAlign::Outside, _) => "outside",
        }
    }
}

/// Floating placement (ignored when `wrap` is `Inline`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Float {
    pub wrap: Wrap,
    pub h_rel: Anchor,
    pub v_rel: Anchor,
    /// Offsets, points (used when the matching alignment is `None`).
    pub x: f32,
    pub y: f32,
    /// Alignments within the anchor area; they take precedence over the offsets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_align: Option<FloatAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v_align: Option<FloatAlign>,
    /// Distance from surrounding text at the left and right (points).
    pub dist: f32,
    /// Distance from surrounding text above and below (points).
    pub dist_top: f32,
    pub dist_bottom: f32,
    /// Room around the object for effects such as shadows (left, top, right, bottom, points),
    /// inline or floating: lines and surrounding text keep clear of it. Rotation's own overhang
    /// is not in here: it follows from [`Float::rot`] (see [`Float::spin_pad`]).
    pub effect: [f32; 4],
    /// Rotation clockwise about the object's centre, degrees (DrawingML `a:xfrm/@rot`). Offsets
    /// and the size stay those of the unrotated frame, as in Word; see [`Float::spin`].
    #[serde(skip_serializing_if = "is_zero")]
    pub rot: f32,
    /// Mirrored left to right / top to bottom (`a:xfrm/@flipH`, `@flipV`).
    #[serde(skip_serializing_if = "is_false")]
    pub flip_h: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub flip_v: bool,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn is_false(v: &bool) -> bool {
    !*v
}

impl Float {
    /// The rotation and flips, the angle normalised to `0..360` (a NaN angle is no rotation).
    pub fn spin(&self) -> wordcraft_geom::Spin {
        wordcraft_geom::Spin::new(self.rot, self.flip_h, self.flip_v)
    }
    /// Set the rotation and flips (the angle normalised).
    pub fn set_spin(&mut self, s: wordcraft_geom::Spin) {
        self.rot = wordcraft_geom::normalize_degrees(s.deg);
        self.flip_h = s.flip_h;
        self.flip_v = s.flip_v;
    }
    /// How far a `w` × `h` frame's rotated bounds reach past it on each side (x, y): the room a
    /// rotated object takes beyond its frame (zero unrotated; negative where the turned object
    /// is narrower than its frame, as a wide picture turned 90° is).
    pub fn spin_pad(&self, w: f32, h: f32) -> (f32, f32) {
        let (w, h) = (wordcraft_geom::finite(w).clamp(0.0, 4000.0), wordcraft_geom::finite(h).clamp(0.0, 4000.0));
        let (bw, bh) = self.spin().extent(w, h);
        ((bw - w) / 2.0, (bh - h) / 2.0)
    }
    /// The effect extents, finite and clamped (left, top, right, bottom).
    pub fn effect_extent(&self) -> [f32; 4] {
        self.effect.map(|v| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ShapeKind {
    #[default]
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Triangle,
    Diamond,
    Line,
    Arrow,
    Star,
    Heart,
    /// A text box (rectangle with a text story).
    TextBox,
    /// A shape drawn point by point, such as ink (its geometry is the shape's `freeform`).
    Freeform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NoteKind {
    #[default]
    Footnote,
    Endnote,
}

/// An inline object anchored at a U+FFFC in the paragraph text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InlineObject {
    Image {
        /// Key in `Document::media`.
        media: String,
        /// Display size, points.
        w: f32,
        h: f32,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        float: Float,
        /// Crop (left, top, right, bottom) fractions 0..1.
        #[serde(default)]
        crop: [f32; 4],
        /// The OLE object (embedded or linked file) this picture shows, as read from a file:
        /// saving writes the object back, not only its picture. See [`crate::graphic::Embedded`].
        #[serde(skip)]
        ole: Option<std::sync::Arc<crate::graphic::Embedded>>,
    },
    /// A chart or SmartArt diagram (see [`crate::graphic`]).
    Graphic {
        w: f32,
        h: f32,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        float: Float,
        graphic: std::sync::Arc<crate::graphic::Graphic>,
    },
    Shape {
        kind: ShapeKind,
        w: f32,
        h: f32,
        fill: Option<crate::props::Rgb>,
        stroke: Option<crate::props::Rgb>,
        #[serde(default)]
        stroke_width: f32,
        #[serde(default)]
        float: Float,
        /// Text box content: `Document::parts` id.
        #[serde(default)]
        story: Option<u32>,
        /// A freeform's geometry (`ShapeKind::Freeform`): its paths, or an ink stroke.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        freeform: Option<std::sync::Arc<crate::freeform::Freeform>>,
        /// Shadow, glow and soft edges.
        #[serde(default, skip_serializing_if = "crate::effects::ShapeEffects::is_empty")]
        effects: crate::effects::ShapeEffects,
    },
    /// Pictures, shapes and text boxes grouped into one object (Layout › Arrange › Group): it
    /// moves, wraps and resizes as one. Its members are laid out in the group's own coordinate
    /// space, `ch_w` × `ch_h`, which is stretched over the group's `w` × `h` (DrawingML's
    /// `a:chExt` and `a:ext`), so resizing the group scales them.
    Group {
        w: f32,
        h: f32,
        #[serde(default)]
        float: Float,
        /// Size of the members' coordinate space (their offsets and sizes are in it).
        ch_w: f32,
        ch_h: f32,
        children: Vec<GroupChild>,
    },
    /// A field: `instr` is the field code (`PAGE`, `NUMPAGES`, `DATE \@ "M/d/yyyy"`, `TOC \o "1-3"`…);
    /// `result` the cached display text.
    Field {
        instr: String,
        #[serde(default)]
        result: String,
        #[serde(default)]
        locked: bool,
    },
    /// The start of a field whose result is ordinary content — formatted, possibly spanning
    /// paragraphs — up to the matching [`InlineObject::FieldEnd`]. Citation managers' `ADDIN`
    /// fields (Zotero, Mendeley, EndNote) are kept this way; `instr` is the field code. See
    /// [`crate::fields`].
    FieldStart {
        instr: String,
        #[serde(default)]
        locked: bool,
    },
    /// The end of the innermost open [`InlineObject::FieldStart`].
    FieldEnd,
    NoteRef {
        kind: NoteKind,
        /// `Document::parts` id of the note's story.
        id: u32,
        /// Custom mark (empty = automatic number).
        #[serde(default)]
        custom: String,
    },
    BookmarkStart {
        name: String,
    },
    BookmarkEnd {
        name: String,
    },
    CommentStart {
        id: u32,
    },
    CommentEnd {
        id: u32,
    },
    /// An equation: its linear format (`x=(-b±√(b^2-4ac))/2a`, for plain text) and structure.
    /// A display equation sits on a line of its own.
    Equation {
        linear: String,
        #[serde(default)]
        display: bool,
        /// The structure; empty = parse `linear`.
        #[serde(default, skip_serializing_if = "crate::math::Math::is_empty")]
        math: crate::math::Math,
    },
    /// Something we don't model, kept for round-trip (raw XML of the source format).
    Opaque {
        format: String,
        xml: String,
        #[serde(default)]
        text: String,
    },
}

/// One member of an [`InlineObject::Group`]: a picture or shape (its own `w`, `h` and story), at
/// `x`, `y` in the group's coordinate space. Its `float` is unused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupChild {
    pub x: f32,
    pub y: f32,
    pub obj: InlineObject,
}

/// Most members a group keeps (hostile files).
pub const MAX_GROUP_CHILDREN: usize = 1000;

impl InlineObject {
    /// A picture, chart or diagram, shape, text box or group: something drawn with a frame, sized and wrapped.
    pub fn is_drawing(&self) -> bool {
        matches!(self, InlineObject::Image { .. } | InlineObject::Graphic { .. } | InlineObject::Shape { .. } | InlineObject::Group { .. })
    }
    /// A drawing's size and placement.
    pub fn frame(&self) -> Option<(f32, f32, &Float)> {
        match self {
            InlineObject::Image { w, h, float, .. }
            | InlineObject::Graphic { w, h, float, .. }
            | InlineObject::Shape { w, h, float, .. }
            | InlineObject::Group { w, h, float, .. } => Some((*w, *h, float)),
            _ => None,
        }
    }
    /// A drawing's placement, to change.
    pub fn float_mut(&mut self) -> Option<&mut Float> {
        match self {
            InlineObject::Image { float, .. }
            | InlineObject::Graphic { float, .. }
            | InlineObject::Shape { float, .. }
            | InlineObject::Group { float, .. } => Some(float),
            _ => None,
        }
    }
    /// Resize a drawing (a group's members scale with it).
    pub fn set_size(&mut self, nw: f32, nh: f32) {
        if let InlineObject::Image { w, h, .. }
        | InlineObject::Graphic { w, h, .. }
        | InlineObject::Shape { w, h, .. }
        | InlineObject::Group { w, h, .. } = self
        {
            *w = nw;
            *h = nh;
        }
    }
    /// The text box story this object shows, if it is a text box.
    pub fn text_box(&self) -> Option<u32> {
        match self {
            InlineObject::Shape { story, .. } => *story,
            _ => None,
        }
    }
    /// The text box stories this object shows: a text box's, or its group members'.
    pub fn text_boxes(&self) -> Vec<u32> {
        match self {
            InlineObject::Shape { story: Some(id), .. } => vec![*id],
            InlineObject::Group { children, .. } => children.iter().filter_map(|c| c.obj.text_box()).collect(),
            _ => Vec::new(),
        }
    }
    /// The text box story slots of this object and its group members, to repoint.
    pub fn text_box_slots(&mut self) -> Vec<&mut Option<u32>> {
        match self {
            InlineObject::Shape { story, .. } => vec![story],
            InlineObject::Group { children, .. } => children
                .iter_mut()
                .filter_map(|c| match &mut c.obj {
                    InlineObject::Shape { story, .. } => Some(story),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    /// A group's members placed in a `w` × `h` box at (`x`, `y`): each member's rectangle
    /// (x, y, w, h) and the member. Empty for anything else.
    pub fn group_rects(&self, x: f32, y: f32, w: f32, h: f32) -> Vec<([f32; 4], &InlineObject)> {
        let InlineObject::Group { ch_w, ch_h, children, .. } = self else { return Vec::new() };
        let fin = |v: f32| if v.is_finite() { v } else { 0.0 };
        let sx = if fin(*ch_w) > 0.0 { fin(w) / ch_w } else { 1.0 };
        let sy = if fin(*ch_h) > 0.0 { fin(h) / ch_h } else { 1.0 };
        children
            .iter()
            .take(MAX_GROUP_CHILDREN)
            .filter_map(|c| {
                let (cw, chh, _) = c.obj.frame()?;
                let r = [x + fin(c.x) * sx, y + fin(c.y) * sy, (fin(cw) * sx).clamp(0.0, 4000.0), (fin(chh) * sy).clamp(0.0, 4000.0)];
                (!matches!(c.obj, InlineObject::Group { .. })).then_some((r, &c.obj))
            })
            .collect()
    }
    /// Zero-width markers don't take part in layout.
    pub fn is_marker(&self) -> bool {
        matches!(
            self,
            InlineObject::BookmarkStart { .. }
                | InlineObject::BookmarkEnd { .. }
                | InlineObject::CommentStart { .. }
                | InlineObject::CommentEnd { .. }
                | InlineObject::FieldStart { .. }
                | InlineObject::FieldEnd
        )
    }
    pub fn is_floating(&self) -> bool {
        match self {
            InlineObject::Image { float, .. }
            | InlineObject::Graphic { float, .. }
            | InlineObject::Shape { float, .. }
            | InlineObject::Group { float, .. } => float.wrap != Wrap::Inline,
            _ => false,
        }
    }
    /// The geometry of an ink stroke (a freeform shape drawn with a pen), if this is one.
    pub fn ink(&self) -> Option<&crate::freeform::Freeform> {
        match self {
            InlineObject::Shape { freeform: Some(f), .. } if f.is_ink() => Some(f),
            _ => None,
        }
    }
    /// The text this object contributes to plain-text extraction.
    pub fn plain_text(&self) -> &str {
        match self {
            InlineObject::Field { result, .. } => result,
            InlineObject::Equation { linear, .. } => linear,
            InlineObject::Opaque { text, .. } => text,
            _ => "",
        }
    }
}

/// A paragraph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paragraph {
    pub text: String,
    pub runs: Vec<Run>,
    #[serde(default, skip_serializing_if = "ParaProps::is_empty")]
    pub props: ParaProps,
    /// Formatting of the paragraph mark (used when typing into an empty paragraph).
    #[serde(default, skip_serializing_if = "CharProps::is_empty")]
    pub mark: CharProps,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<InlineObject>,
    /// Set when this paragraph ends a section (OOXML keeps `w:sectPr` on the last paragraph).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<Box<SectionProps>>,
    /// Change counter for caches (not part of equality).
    #[serde(skip, default = "next_rev")]
    pub rev: u64,
}

impl PartialEq for Paragraph {
    fn eq(&self, o: &Self) -> bool {
        self.text == o.text
            && self.runs == o.runs
            && self.props == o.props
            && self.mark == o.mark
            && self.objects == o.objects
            && self.section == o.section
    }
}

impl Default for Paragraph {
    fn default() -> Self {
        Paragraph::new()
    }
}

impl Paragraph {
    pub fn new() -> Self {
        Paragraph {
            text: String::new(),
            runs: Vec::new(),
            props: ParaProps::default(),
            mark: CharProps::default(),
            objects: Vec::new(),
            section: None,
            rev: next_rev(),
        }
    }
    /// A paragraph of plain text in one run.
    pub fn with_text(text: &str, props: CharProps) -> Self {
        let mut p = Paragraph::new();
        let clean: String = text.chars().filter(|c| *c != OBJ).collect();
        if !clean.is_empty() {
            p.runs.push(Run { len: clean.len(), props: props.clone() });
        }
        p.text = clean;
        p.mark = props;
        p
    }
    pub fn styled(mut self, style: &str) -> Self {
        self.props.style = Some(style.to_string());
        self
    }
    pub fn touch(&mut self) {
        self.rev = next_rev();
    }
    pub fn len(&self) -> usize {
        self.text.len()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    /// The text with objects removed or replaced by their text (fields → result).
    pub fn plain_text(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        let mut k = 0;
        for c in self.text.chars() {
            if c == OBJ {
                if let Some(o) = self.objects.get(k) {
                    out.push_str(o.plain_text());
                }
                k += 1;
            } else {
                out.push(c);
            }
        }
        out
    }
    /// [`Self::plain_text`] without tracked deletions: the text as it reads with every change accepted.
    pub fn final_text(&self) -> String {
        self.text_without(&self.deleted_ranges())
    }
    /// The byte ranges of the tracked deletions.
    pub fn deleted_ranges(&self) -> Vec<std::ops::Range<usize>> {
        self.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| r).collect()
    }
    /// [`Self::plain_text`] without the text in the byte ranges `dropped` (in any order, overlapping
    /// or not).
    pub fn text_without(&self, dropped: &[std::ops::Range<usize>]) -> String {
        let mut sorted = dropped.to_vec();
        sorted.sort_unstable_by_key(|r| r.start);
        let mut next = sorted.iter().peekable();
        let mut out = String::with_capacity(self.text.len());
        let mut k = 0;
        for (i, c) in self.text.char_indices() {
            // By start: once the ranges that end by `i` are skipped, `i` is dropped if the next has begun.
            while next.next_if(|r| r.end <= i).is_some() {}
            let keep = !next.peek().is_some_and(|r| r.start <= i);
            if c == OBJ {
                if keep && let Some(o) = self.objects.get(k) {
                    out.push_str(o.plain_text());
                }
                k += 1;
            } else if keep {
                out.push(c);
            }
        }
        out
    }

    fn check(&self, off: usize) -> Result<()> {
        if off > self.text.len() || !self.text.is_char_boundary(off) {
            return Err(DocError::BadOffset(off));
        }
        Ok(())
    }

    /// Clamp `off` to the text and back to a char boundary.
    pub fn clamp(&self, off: usize) -> usize {
        let mut o = off.min(self.text.len());
        while o > 0 && !self.text.is_char_boundary(o) {
            o -= 1;
        }
        o
    }

    /// Index of the run containing byte `off` and the offset where that run starts. At a run
    /// boundary the earlier run wins (typing continues the formatting to the left).
    fn run_at(&self, off: usize) -> Option<(usize, usize)> {
        let mut start = 0;
        for (i, r) in self.runs.iter().enumerate() {
            let end = start + r.len;
            if off < end || (off == end && off > start) {
                return Some((i, start));
            }
            start = end;
        }
        None
    }

    /// Formatting in effect at `off` (what typing there would get): the run to the left, or the
    /// first run at offset 0, or the paragraph mark when empty.
    pub fn props_at(&self, off: usize) -> &CharProps {
        if off == 0 {
            return self.runs.first().map(|r| &r.props).unwrap_or(&self.mark);
        }
        self.run_at(off).and_then(|(i, _)| self.runs.get(i)).map(|r| &r.props).unwrap_or(&self.mark)
    }

    /// Formatting of the character starting at `off`.
    pub fn props_of_char(&self, off: usize) -> &CharProps {
        let mut start = 0;
        for r in &self.runs {
            if off < start + r.len {
                return &r.props;
            }
            start += r.len;
        }
        &self.mark
    }

    /// Runs as (byte range, props).
    pub fn run_ranges(&self) -> impl Iterator<Item = (std::ops::Range<usize>, &CharProps)> {
        let mut s = 0;
        self.runs.iter().map(move |r| {
            let a = s;
            s += r.len;
            (a..s, &r.props)
        })
    }

    /// Number of objects in `text[..off]`.
    fn objects_before(&self, off: usize) -> usize {
        self.text.get(..off).map(|t| t.chars().filter(|c| *c == OBJ).count()).unwrap_or(0)
    }

    /// The object at byte `off` (which must hold U+FFFC).
    pub fn object_at(&self, off: usize) -> Option<&InlineObject> {
        if !self.text.get(off..)?.starts_with(OBJ) {
            return None;
        }
        self.objects.get(self.objects_before(off))
    }
    pub fn object_at_mut(&mut self, off: usize) -> Option<&mut InlineObject> {
        if !self.text.get(off..)?.starts_with(OBJ) {
            return None;
        }
        let k = self.objects_before(off);
        self.objects.get_mut(k)
    }
    /// Byte offsets of all objects, in order.
    pub fn object_offsets(&self) -> Vec<usize> {
        self.text.char_indices().filter(|(_, c)| *c == OBJ).map(|(i, _)| i).collect()
    }

    /// Insert plain text (U+FFFC is dropped) at `off` with `props`. Returns the inserted length.
    pub fn insert_text(&mut self, off: usize, s: &str, props: &CharProps) -> Result<usize> {
        self.check(off)?;
        let clean: std::borrow::Cow<str> = if s.contains(OBJ) { s.chars().filter(|c| *c != OBJ).collect::<String>().into() } else { s.into() };
        if clean.is_empty() {
            return Ok(0);
        }
        self.text.insert_str(off, &clean);
        self.insert_run(off, clean.len(), props.clone());
        self.touch();
        Ok(clean.len())
    }

    /// Insert an object at `off`.
    pub fn insert_object(&mut self, off: usize, obj: InlineObject, props: &CharProps) -> Result<()> {
        self.check(off)?;
        let k = self.objects_before(off);
        self.text.insert(off, OBJ);
        self.objects.insert(k.min(self.objects.len()), obj);
        self.insert_run(off, OBJ.len_utf8(), props.clone());
        self.touch();
        Ok(())
    }

    fn insert_run(&mut self, off: usize, len: usize, props: CharProps) {
        // Split the run at `off`, insert, merge neighbours.
        let mut start = 0;
        let mut idx = self.runs.len();
        for (i, r) in self.runs.iter().enumerate() {
            if off <= start {
                idx = i;
                break;
            }
            if off < start + r.len {
                // split
                let left = off - start;
                let right = r.len - left;
                let p = r.props.clone();
                if let Some(r) = self.runs.get_mut(i) {
                    r.len = left;
                }
                self.runs.insert(i + 1, Run { len: right, props: p });
                idx = i + 1;
                break;
            }
            start += r.len;
        }
        self.runs.insert(idx.min(self.runs.len()), Run { len, props });
        self.normalize();
    }

    /// Merge adjacent equal runs and drop empty ones.
    pub fn normalize(&mut self) {
        let mut out: Vec<Run> = Vec::with_capacity(self.runs.len());
        for r in self.runs.drain(..) {
            if r.len == 0 {
                continue;
            }
            match out.last_mut() {
                Some(l) if l.props == r.props => l.len += r.len,
                _ => out.push(r),
            }
        }
        self.runs = out;
        // Repair a run list that doesn't cover the text exactly (never expected, but files lie).
        let total: usize = self.runs.iter().map(|r| r.len).sum();
        if total != self.text.len() {
            if total < self.text.len() {
                let p = self.runs.last().map(|r| r.props.clone()).unwrap_or_else(|| self.mark.clone());
                self.runs.push(Run { len: self.text.len() - total, props: p });
            } else {
                let mut keep = self.text.len();
                for r in &mut self.runs {
                    r.len = r.len.min(keep);
                    keep -= r.len;
                }
                self.runs.retain(|r| r.len > 0);
            }
        }
    }

    /// Delete `a..b` (bytes). Objects inside are removed.
    pub fn delete(&mut self, a: usize, b: usize) -> Result<()> {
        let (a, b) = (a.min(b), a.max(b));
        self.check(a)?;
        self.check(b)?;
        if a == b {
            return Ok(());
        }
        if self.runs.is_empty() || a == 0 {
            // Keep the formatting of what's deleted for the mark when emptying.
            if b == self.text.len() {
                self.mark = self.props_of_char(a).clone();
            }
        }
        let k0 = self.objects_before(a);
        let k1 = self.objects_before(b);
        if k1 > k0 && k1 <= self.objects.len() {
            self.objects.drain(k0..k1);
        }
        self.text.replace_range(a..b, "");
        let mut start = 0;
        for r in &mut self.runs {
            let (rs, re) = (start, start + r.len);
            start = re;
            let ov = re.min(b).saturating_sub(rs.max(a));
            r.len -= ov.min(r.len);
        }
        self.normalize();
        self.touch();
        Ok(())
    }

    /// Apply `f` to the formatting of `a..b`.
    pub fn format(&mut self, a: usize, b: usize, f: &dyn Fn(&mut CharProps)) -> Result<()> {
        let (a, b) = (a.min(b), a.max(b));
        self.check(a)?;
        self.check(b)?;
        if a == b {
            return Ok(());
        }
        let mut out = Vec::with_capacity(self.runs.len() + 2);
        let mut start = 0;
        for r in self.runs.drain(..) {
            let (rs, re) = (start, start + r.len);
            start = re;
            let cut = [rs, a.clamp(rs, re), b.clamp(rs, re), re];
            for w in cut.windows(2) {
                let (x, y) = (w[0], w[1]);
                if y <= x {
                    continue;
                }
                let mut p = r.props.clone();
                if x >= a && y <= b {
                    f(&mut p);
                }
                out.push(Run { len: y - x, props: p });
            }
        }
        self.runs = out;
        self.normalize();
        self.touch();
        Ok(())
    }

    /// Split at `off`: `self` keeps `..off`, the returned paragraph gets `off..` with the same
    /// paragraph properties (the section break, if any, moves to the second half).
    pub fn split_off(&mut self, off: usize) -> Result<Paragraph> {
        self.check(off)?;
        let k = self.objects_before(off);
        let tail_text = self.text.split_off(off);
        let tail_objs = if k <= self.objects.len() { self.objects.split_off(k) } else { Vec::new() };
        let mut tail_runs = Vec::new();
        let mut start = 0;
        let mut keep = Vec::new();
        for r in self.runs.drain(..) {
            let (rs, re) = (start, start + r.len);
            start = re;
            if re <= off {
                keep.push(r);
            } else if rs >= off {
                tail_runs.push(r);
            } else {
                keep.push(Run { len: off - rs, props: r.props.clone() });
                tail_runs.push(Run { len: re - off, props: r.props });
            }
        }
        self.runs = keep;
        let carry = self.props_at(off).clone();
        let mut tail = Paragraph {
            text: tail_text,
            runs: tail_runs,
            props: self.props.clone(),
            mark: if self.text.is_empty() && off == 0 { self.mark.clone() } else { carry },
            objects: tail_objs,
            section: self.section.take(),
            rev: next_rev(),
        };
        if tail.text.is_empty() {
            tail.mark = self.props_at(off).clone();
        }
        self.normalize();
        tail.normalize();
        self.touch();
        Ok(tail)
    }

    /// Append `other`'s content (its paragraph properties are dropped; its section break kept).
    pub fn append(&mut self, other: Paragraph) {
        let Paragraph { text, runs, objects, section, .. } = other;
        self.text.push_str(&text);
        self.runs.extend(runs);
        self.objects.extend(objects);
        if section.is_some() {
            self.section = section;
        }
        self.normalize();
        self.touch();
    }

    /// Previous grapheme boundary before `off` (0 at start).
    pub fn prev_boundary(&self, off: usize) -> usize {
        use unicode_segmentation::GraphemeCursor;
        let off = self.clamp(off);
        let mut c = GraphemeCursor::new(off, self.text.len(), true);
        c.prev_boundary(&self.text, 0).ok().flatten().unwrap_or(0)
    }
    /// Next grapheme boundary after `off` (len at end).
    pub fn next_boundary(&self, off: usize) -> usize {
        use unicode_segmentation::GraphemeCursor;
        let off = self.clamp(off);
        let mut c = GraphemeCursor::new(off, self.text.len(), true);
        c.next_boundary(&self.text, 0).ok().flatten().unwrap_or(self.text.len())
    }
    /// Start of the word at or before `off` (Ctrl+Left).
    pub fn word_start(&self, off: usize) -> usize {
        let off = self.clamp(off);
        let Some(before) = self.text.get(..off) else { return 0 };
        let mut idx = off;
        let mut seen_word = false;
        for (i, c) in before.char_indices().rev() {
            let w = crate::bidi::is_word_char(c);
            if w {
                seen_word = true;
                idx = i;
            } else if seen_word {
                break;
            } else {
                idx = i;
            }
        }
        idx
    }
    /// End of the word after `off`, including trailing spaces (Ctrl+Right, like Word).
    pub fn word_end(&self, off: usize) -> usize {
        let off = self.clamp(off);
        let Some(after) = self.text.get(off..) else { return self.text.len() };
        let it = after.char_indices().peekable();
        let mut end = self.text.len();
        let first_word = after.chars().next().is_some_and(|c| crate::bidi::is_word_char(c) && c != '\'');
        let mut in_space = false;
        for (i, c) in it {
            let w = crate::bidi::is_word_char(c);
            if c == ' ' || c == NBSP {
                in_space = true;
                continue;
            }
            if in_space || (first_word && !w) || (!first_word && i > 0) {
                end = off + i;
                break;
            }
        }
        end
    }
    /// The word around `off` (double-click): (start, end) without trailing space.
    pub fn word_at(&self, off: usize) -> (usize, usize) {
        let off = self.clamp(off);
        let is_w = |c: char| crate::bidi::is_word_char(c);
        let mut a = off;
        for (i, c) in self.text.get(..off).unwrap_or("").char_indices().rev() {
            if !is_w(c) {
                break;
            }
            a = i;
        }
        let mut b = off;
        for (i, c) in self.text.get(off..).unwrap_or("").char_indices() {
            if !is_w(c) {
                if i == 0 && a == off {
                    b = off + c.len_utf8();
                }
                break;
            }
            b = off + i + c.len_utf8();
        }
        // Word also selects the trailing spaces.
        let mut e = b;
        for (i, c) in self.text.get(b..).unwrap_or("").char_indices() {
            if c != ' ' {
                break;
            }
            e = b + i + 1;
        }
        (a, e)
    }
    /// Sentence around `off` (Ctrl+click): ends after `.`, `!`, `?` and following spaces.
    pub fn sentence_at(&self, off: usize) -> (usize, usize) {
        let off = self.clamp(off);
        let t = &self.text;
        let mut a = 0;
        let mut prev_end = false;
        for (i, c) in t.char_indices() {
            if i >= off {
                break;
            }
            if prev_end && c != ' ' {
                a = i;
            }
            if matches!(c, '.' | '!' | '?') {
                prev_end = true;
            } else if c != ' ' {
                prev_end = false;
            }
        }
        let mut b = t.len();
        let mut ended = false;
        for (i, c) in t.get(off..).unwrap_or("").char_indices() {
            if ended && c != ' ' {
                b = off + i;
                break;
            }
            if matches!(c, '.' | '!' | '?') {
                ended = true;
            }
        }
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> CharProps {
        CharProps { bold: Some(true), ..Default::default() }
    }

    #[test]
    fn text_without_takes_ranges_in_any_order() {
        // Multi-byte chars at range edges, an object, and ranges unsorted, overlapping, nested and empty.
        let mut p = Paragraph::with_text("añb€c", CharProps::default());
        p.insert_object(3, InlineObject::Field { instr: "PAGE".into(), result: "7".into(), locked: false }, &CharProps::default()).unwrap();
        let text = p.text.clone();
        let naive = |dropped: &[std::ops::Range<usize>]| {
            let mut out = String::new();
            let mut k = 0;
            for (i, c) in text.char_indices() {
                let keep = !dropped.iter().any(|r| r.contains(&i));
                if c == OBJ {
                    if keep && let Some(o) = p.objects.get(k) {
                        out.push_str(o.plain_text());
                    }
                    k += 1;
                } else if keep {
                    out.push(c);
                }
            }
            out
        };
        let n = text.len();
        let cases: Vec<Vec<std::ops::Range<usize>>> =
            vec![vec![], vec![1..3], vec![5..n, 0..1], vec![0..4, 2..3, 3..6], vec![2..9, 1..2, 4..4, 0..n], vec![6..6, 9..n, 0..0], vec![3..n + 10]];
        for dropped in cases {
            assert_eq!(p.text_without(&dropped), naive(&dropped), "{dropped:?} of {text:?}");
        }
        assert_eq!(p.text_without(std::slice::from_ref(&(1..3))), "a7b€c");
    }

    #[test]
    fn insert_and_runs() {
        let mut p = Paragraph::with_text("Hello world", CharProps::default());
        p.insert_text(5, ",", &CharProps::default()).unwrap();
        assert_eq!(p.text, "Hello, world");
        assert_eq!(p.runs.len(), 1);
        p.insert_text(0, "Oh ", &bold()).unwrap();
        assert_eq!(p.runs.len(), 2);
        assert_eq!(p.runs[0].len, 3);
        assert!(p.insert_text(100, "x", &bold()).is_err());
    }

    #[test]
    fn insert_rejects_mid_char() {
        let mut p = Paragraph::with_text("é", CharProps::default());
        assert!(p.insert_text(1, "x", &bold()).is_err());
    }

    #[test]
    fn format_splits_runs() {
        let mut p = Paragraph::with_text("abcdef", CharProps::default());
        p.format(2, 4, &|c| c.bold = Some(true)).unwrap();
        assert_eq!(p.runs.iter().map(|r| r.len).collect::<Vec<_>>(), vec![2, 2, 2]);
        assert_eq!(p.props_of_char(2).bold, Some(true));
        p.format(0, 6, &|c| c.bold = Some(true)).unwrap();
        assert_eq!(p.runs.len(), 1);
    }

    #[test]
    fn delete_merges() {
        let mut p = Paragraph::with_text("abcdef", CharProps::default());
        p.format(2, 4, &|c| c.bold = Some(true)).unwrap();
        p.delete(2, 4).unwrap();
        assert_eq!(p.text, "abef");
        assert_eq!(p.runs.len(), 1);
        p.delete(0, 4).unwrap();
        assert!(p.runs.is_empty());
    }

    #[test]
    fn objects_track_text() {
        let mut p = Paragraph::with_text("ab", CharProps::default());
        p.insert_object(1, InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false }, &CharProps::default()).unwrap();
        p.insert_object(0, InlineObject::BookmarkStart { name: "x".into() }, &CharProps::default()).unwrap();
        assert_eq!(p.objects.len(), 2);
        assert!(matches!(p.object_at(0), Some(InlineObject::BookmarkStart { .. })));
        let off = p.object_offsets()[1];
        assert!(matches!(p.object_at(off), Some(InlineObject::Field { .. })));
        assert_eq!(p.plain_text(), "a1b");
        p.delete(0, 3).unwrap();
        assert_eq!(p.objects.len(), 1);
        let tail = p.split_off(p.object_offsets()[0]).unwrap();
        assert!(p.objects.is_empty());
        assert_eq!(tail.objects.len(), 1);
    }

    #[test]
    fn split_and_append() {
        let mut p = Paragraph::with_text("Hello world", bold());
        let t = p.split_off(5).unwrap();
        assert_eq!(p.text, "Hello");
        assert_eq!(t.text, " world");
        assert_eq!(t.runs[0].props.bold, Some(true));
        p.append(t);
        assert_eq!(p.text, "Hello world");
        assert_eq!(p.runs.len(), 1);
        let t = p.split_off(p.len()).unwrap();
        assert!(t.is_empty());
        assert_eq!(t.mark.bold, Some(true));
    }

    #[test]
    fn words() {
        let p = Paragraph::with_text("The quick  brown fox.", CharProps::default());
        assert_eq!(p.word_end(0), 4);
        assert_eq!(p.word_end(4), 11);
        assert_eq!(p.word_start(10), 4);
        assert_eq!(p.word_start(11), 4);
        assert_eq!(p.word_at(5), (4, 11));
        assert_eq!(p.sentence_at(3), (0, 21));
        assert_eq!(p.next_boundary(0), 1);
        assert_eq!(p.prev_boundary(0), 0);
    }

    proptest::proptest! {
        #[test]
        fn edits_keep_runs_consistent(ops in proptest::collection::vec((0u8..4, 0usize..40, 0usize..40, "[a-zé ]{0,5}"), 0..40)) {
            let mut p = Paragraph::with_text("seed text", CharProps::default());
            for (k, a, b, s) in ops {
                let a = p.clamp(a);
                let b = p.clamp(b);
                let _ = match k {
                    0 => p.insert_text(a, &s, &bold()).map(|_| ()),
                    1 => p.delete(a, b),
                    2 => p.format(a, b, &|c| c.italic = Some(true)),
                    _ => p.split_off(a).map(|t| p.append(t)),
                };
                let total: usize = p.runs.iter().map(|r| r.len).sum();
                proptest::prop_assert_eq!(total, p.text.len());
                proptest::prop_assert!(p.runs.iter().all(|r| r.len > 0));
                proptest::prop_assert_eq!(p.text.matches(OBJ).count(), p.objects.len());
            }
        }
    }
}
