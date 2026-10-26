//! Page → draw items, shared by the raster renderer, the PDF exporter and thumbnails.

use std::collections::HashMap;

use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg, TextAlign};
use wordcraft_doc::para::{InlineObject, ShapeKind};
use wordcraft_doc::props::{Border, BorderStyle, Rgb, TextColor, TextDirection, Underline};
use wordcraft_doc::section::SectionStart;
use wordcraft_doc::{Block, Document, Path, StoryRef};
use wordcraft_fonts::{BezPath, FaceRef};
use wordcraft_geom::{Rect, Spin};

use crate::math::MItem;
use crate::para::{ClKind, LineEnd, ParaLayout};
use crate::{Page, Placed};

/// How an area is painted: one colour or a gradient (page points), opacities 0..1.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Rgb, f32),
    /// A linear gradient from `p0` to `p1`; stops are (offset 0..1, colour, opacity).
    Linear {
        p0: (f32, f32),
        p1: (f32, f32),
        stops: Vec<(f32, Rgb, f32)>,
    },
    /// A circular gradient out from `c` to radius `r`; stops as [`Paint::Linear`]'s.
    Radial {
        c: (f32, f32),
        r: f32,
        stops: Vec<(f32, Rgb, f32)>,
    },
}

impl Paint {
    /// Opaque `c`.
    pub fn solid(c: Rgb) -> Paint {
        Paint::Solid(c, 1.0)
    }
    /// This paint with every opacity times `a` (soft edges' bands).
    pub fn faded(&self, a: f32) -> Paint {
        let fade = |s: &Vec<(f32, Rgb, f32)>| s.iter().map(|(o, c, x)| (*o, *c, x * a)).collect();
        match self {
            Paint::Solid(c, x) => Paint::Solid(*c, x * a),
            Paint::Linear { p0, p1, stops } => Paint::Linear { p0: *p0, p1: *p1, stops: fade(stops) },
            Paint::Radial { c, r, stops } => Paint::Radial { c: *c, r: *r, stops: fade(stops) },
        }
    }
    /// The colour and opacity this paint gives the point (`x`, `y`): stops blended, padded past
    /// the ends.
    pub fn at(&self, x: f32, y: f32) -> (Rgb, f32) {
        let (stops, t) = match self {
            Paint::Solid(c, a) => return (*c, *a),
            Paint::Linear { p0, p1, stops } => {
                let d = (p1.0 - p0.0, p1.1 - p0.1);
                (stops, ((x - p0.0) * d.0 + (y - p0.1) * d.1) / (d.0 * d.0 + d.1 * d.1).max(1e-6))
            }
            Paint::Radial { c, r, stops } => (stops, (x - c.0).hypot(y - c.1) / r.max(1e-6)),
        };
        let (Some(first), Some(last)) = (stops.first(), stops.last()) else { return (Rgb::BLACK, 0.0) };
        let t = if t.is_finite() { t } else { 0.0 };
        let Some(i) = stops.iter().position(|s| s.0 >= t) else { return (last.1, last.2) };
        let Some(a) = i.checked_sub(1).and_then(|j| stops.get(j)) else { return (first.1, first.2) };
        let b = &stops[i];
        let f = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
        let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f).round().clamp(0.0, 255.0) as u8;
        (Rgb(mix(a.1.0, b.1.0), mix(a.1.1, b.1.1), mix(a.1.2, b.1.2)), a.2 + (b.2 - a.2) * f)
    }
}

/// How shape fill `fill` paints the shape's frame `frame` (#1500). A linear gradient runs across
/// the frame at its angle (`scaled`: the angle of the frame squashed to a square); a path
/// gradient runs from the centre of its focus rectangle out to the farthest corner, drawn
/// circular whatever its outline (rectangular and shape-following ones are approximated).
/// `tileRect` and `flip` are kept but not drawn. `None` for no fill.
pub fn paint_for(fill: &wordcraft_doc::fill::Fill, frame: Rect) -> Option<Paint> {
    use wordcraft_doc::fill::{Fill, GradientKind};
    let g = match fill.sanitized()? {
        Fill::Solid { color, transparency } => return Some(Paint::Solid(color, 1.0 - transparency / 100.0)),
        Fill::Gradient(g) => g,
    };
    let stops = g.stops.iter().map(|s| (s.pos / 100.0, s.color, 1.0 - s.transparency / 100.0)).collect();
    let (cx, cy) = (frame.x + frame.w / 2.0, frame.y + frame.h / 2.0);
    Some(match g.kind {
        GradientKind::Linear { angle, scaled } => {
            let (s, c) = angle.to_radians().sin_cos();
            // Scaled: the isolines of the square's gradient stretched with the frame.
            let (dx, dy) = if scaled && frame.w > 0.0 && frame.h > 0.0 { (c / frame.w, s / frame.h) } else { (c, s) };
            let len = dx.hypot(dy);
            let (dx, dy) = if len > 0.0 && len.is_finite() { (dx / len, dy / len) } else { (1.0, 0.0) };
            let half = (frame.w * dx.abs() + frame.h * dy.abs()) / 2.0;
            Paint::Linear { p0: (cx - dx * half, cy - dy * half), p1: (cx + dx * half, cy + dy * half), stops }
        }
        GradientKind::Path { focus: [l, t, r, b], .. } => {
            let x0 = frame.x + frame.w * l / 100.0;
            let x1 = frame.right() - frame.w * r / 100.0;
            let y0 = frame.y + frame.h * t / 100.0;
            let y1 = frame.bottom() - frame.h * b / 100.0;
            let c = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let corners = [(frame.x, frame.y), (frame.right(), frame.y), (frame.x, frame.bottom()), (frame.right(), frame.bottom())];
            let r = corners.iter().map(|(x, y)| (x - c.0).hypot(y - c.1)).fold(0.0f32, f32::max);
            Paint::Radial { c, r: r.max(0.01), stops }
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stroke {
    Solid,
    Dotted,
    Dashed,
    Double,
    /// Three thin lines side by side (a `triple` border).
    Triple,
    Wave,
    /// Word's border patterns (paragraph, cell and table borders): dots one width on, one off;
    /// dashes three on, one off; dash and dot.
    BorderDotted,
    BorderDashed,
    BorderDotDash,
}

impl Stroke {
    /// Dash and gap lengths, alternately, in line widths; `None` for the strokes that aren't
    /// dashed.
    pub fn dashes(self) -> Option<&'static [f32]> {
        match self {
            Stroke::Dotted => Some(&[1.0, 2.0]),
            Stroke::Dashed => Some(&[4.0, 2.0]),
            Stroke::BorderDotted => Some(&[1.0, 1.0]),
            Stroke::BorderDashed => Some(&[3.0, 1.0]),
            Stroke::BorderDotDash => Some(&[3.0, 1.0, 1.0, 1.0]),
            Stroke::Solid | Stroke::Double | Stroke::Triple | Stroke::Wave => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Draw {
    Glyphs {
        face: FaceRef,
        size: f32,
        /// Glyph id, x, baseline y (page coordinates).
        glyphs: Vec<(u32, f32, f32)>,
        color: Rgb,
        alpha: f32,
        synth_bold: bool,
        synth_italic: bool,
        /// The text these glyphs show (for PDF text extraction), and the hyperlink.
        text: String,
        link: Option<String>,
        /// Byte range of `text` each glyph shows (one per glyph; a ligature's range covers all
        /// its characters). Empty when unknown: the PDF writer then guesses from the font.
        ranges: Vec<std::ops::Range<usize>>,
        /// The text background (highlight or character shading) these glyphs are drawn on, if
        /// any: a dark page dims it, so it picks the text colour against the dimmed colour.
        text_bg: Option<Rgb>,
        /// Horizontal scale of the letters (`w:w`, 1 = 100 %): each glyph is stretched across
        /// from its pen position. The positions already have the scaled advances.
        x_scale: f32,
    },
    Fill {
        rect: Rect,
        color: Rgb,
        alpha: f32,
        /// A text background (highlight or character shading), which a dark page dims instead
        /// of inverting so the text on it stays readable. Other fills (paragraph, cell, page) don't.
        text_bg: bool,
    },
    Line {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: Rgb,
        stroke: Stroke,
        alpha: f32,
        /// The text background this line (an underline or strikethrough) is drawn on, as for
        /// [`Draw::Glyphs`]; `None` for every other line.
        text_bg: Option<Rgb>,
    },
    Image {
        rect: Rect,
        media: String,
        crop: [f32; 4],
        alpha: f32,
        /// The picture's alt text (empty for a floating or diagram picture, which tag themselves).
        alt: String,
        /// Marked decorative (Alt Text › Mark as decorative): an artifact in tagged PDF, which
        /// screen readers skip. Always `false` for a floating or diagram picture, as `alt`.
        decorative: bool,
    },
    Shape {
        rect: Rect,
        kind: ShapeKind,
        fill: Option<Paint>,
        stroke: Option<Rgb>,
        stroke_width: f32,
        /// Alternating dash/space lengths in page points; empty means solid.
        dash: Vec<f32>,
        cap: wordcraft_doc::connector::LineCap,
        /// Shadow, glow and soft edges (drawn with [`wordcraft_doc::effects::bands`]).
        effects: wordcraft_doc::effects::ShapeEffects,
        /// A rounded rectangle's corner ([`wordcraft_doc::connector::ShapeExtra::corner`]).
        corner: Option<u32>,
    },
    /// An ink stroke: a line through `pts` (page points) with round ends and joins, `width`
    /// wide, in `color` at opacity `alpha`.
    Ink { pts: Vec<(f32, f32)>, color: Rgb, width: f32, alpha: f32 },
    /// A vector path (a chart's or diagram's polygons, slices, lines): filled, then stroked.
    Path { segs: Vec<PathSeg>, fill: Option<Paint>, stroke: Option<Rgb>, stroke_width: f32, dash: Vec<f32>, cap: wordcraft_doc::connector::LineCap },
    /// An inline chart, diagram or described shape's draws, as one figure with its alt text (for
    /// tagged PDF). `kind` is the graphic's, `None` for a shape (see [`crate::shape_figure_alt`]).
    Figure { alt: String, kind: Option<GraphicKind>, draws: Vec<Draw> },
    /// A formatting mark (¶ · → ↵) in the UI's mark colour, or in `color` (a tracked
    /// paragraph mark in its reviser's colour).
    Mark { x: f32, baseline: f32, size: f32, ch: char, color: Option<Rgb> },
    /// Turned text (a table cell's text direction): `items` are drawn in a frame turned `turn`
    /// whose origin is page point (`x`, `y`) (see [`crate::turn_point`]).
    Turned { x: f32, y: f32, turn: TextDirection, items: Vec<Draw> },
    /// A rotated or flipped picture, shape, chart, group or Drawing Canvas: `items` drawn
    /// turned by `spin` about page point (`cx`, `cy`) (see [`Spin::matrix`]).
    Rotated { cx: f32, cy: f32, spin: Spin, items: Vec<Draw> },
    /// `items` clipped to `rect` (a Drawing Canvas's members to its frame). Inside a
    /// [`Draw::Rotated`], `rect` is in the turned frame's (unturned) coordinates.
    Clip { rect: Rect, items: Vec<Draw> },
    /// Text drawn from its outlines with WordArt effects (and bent, in a warped text box):
    /// `path` in page coordinates, painted as [`crate::wordart::art_layers`] says. `color` is the
    /// run's own colour, the fill when the effects set none. `text_bg`: as for [`Draw::Glyphs`].
    Art { path: BezPath, fx: Box<wordcraft_doc::wordart::TextEffects>, color: Rgb, alpha: f32, text_bg: Option<Rgb> },
    /// A formatting-mark label (a section break's name) in the UI's mark colour, left edge at
    /// `x`, drawn with the same face as [`Draw::Mark`]. Screen only, like every mark.
    MarkText { x: f32, baseline: f32, size: f32, text: String },
}

impl Draw {
    /// The affine map (a, b, c, d, e, f: x' = a·x + c·y + e, y' = b·x + d·y + f) from a frame
    /// turned `turn` with its origin at page point (`x`, `y`) to the page.
    pub fn turn_matrix(turn: TextDirection, x: f32, y: f32) -> [f32; 6] {
        match turn {
            TextDirection::Horizontal => [1.0, 0.0, 0.0, 1.0, x, y],
            TextDirection::Down => [0.0, 1.0, -1.0, 0.0, x, y],
            TextDirection::Up => [0.0, -1.0, 1.0, 0.0, x, y],
        }
    }
}

/// Formatting marks shown even while Show/Hide ¶ is off (File › Options › Display, "Always show
/// these formatting marks"). Show/Hide ¶ on shows them all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FormattingMarks {
    /// Tab characters (→).
    pub tabs: bool,
    /// Spaces (·).
    pub spaces: bool,
    /// Paragraph marks (¶), line breaks, page, column and section breaks.
    pub paragraphs: bool,
    /// Hidden text (shown with its dotted underline instead of left out).
    pub hidden: bool,
}

impl FormattingMarks {
    pub const ALL: FormattingMarks = FormattingMarks { tabs: true, spaces: true, paragraphs: true, hidden: true };

    /// The marks on screen: all of them with Show/Hide ¶ on, else the chosen ones.
    pub fn shown(self, show_all: bool) -> FormattingMarks {
        if show_all { FormattingMarks::ALL } else { self }
    }

    pub fn any(self) -> bool {
        self.tabs || self.spaces || self.paragraphs || self.hidden
    }
}

#[derive(Clone, Debug)]
pub struct DisplayOptions {
    /// Show formatting marks (¶): Show/Hide ¶, every mark.
    pub marks: bool,
    /// Marks shown while `marks` is off (File › Options › Display).
    pub always: FormattingMarks,
    /// Dim headers/footers (editing the body) or the body (editing a header/footer).
    pub dim_header: bool,
    pub dim_body: bool,
    /// Show tracked changes as markup (coloured, underlined/struck).
    pub markup: bool,
    /// Show on-screen-only marks: equation placeholders and prompts (never in print or PDF).
    pub placeholders: bool,
    /// Review › Hide Ink: leave ink strokes out (they stay in the document).
    pub hide_ink: bool,
    /// How tracked changes and comments are marked when `markup` is on (Track Changes Options).
    pub revisions: MarkupOptions,
    /// Leave this object out of the page: while it is dragged, the preview under the pointer
    /// stands in for it, so the page must not also show it where it was.
    pub hide_object: Option<wordcraft_doc::Pos>,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        DisplayOptions {
            marks: false,
            always: FormattingMarks::default(),
            dim_header: true,
            dim_body: false,
            markup: true,
            placeholders: false,
            hide_ink: false,
            revisions: MarkupOptions::default(),
            hide_object: None,
        }
    }
}

/// An object left out of a page's drawing ([`DisplayOptions::hide_object`]).
struct Hidden {
    object: wordcraft_doc::Pos,
    /// The text box stories it shows (its own and its group members', resolved to the story each
    /// one shows): a box's text is drawn as lines of its story, so it goes with the box.
    boxes: Vec<u32>,
}

impl Hidden {
    /// The object at `object` and the text box stories whose lines belong to it.
    fn new(doc: &Document, object: &wordcraft_doc::Pos) -> Hidden {
        let boxes = doc
            .para(object.story, &object.path)
            .and_then(|p| p.object_at(object.off))
            .map(|o| o.text_boxes().into_iter().map(|id| doc.text_box_chain(id).and_then(|c| c.first().copied()).unwrap_or(id)).collect())
            .unwrap_or_default();
        Hidden { object: object.clone(), boxes }
    }

    fn hides(&self, it: &Placed) -> bool {
        let me = &self.object;
        match it {
            Placed::Object { story, path, off, .. } | Placed::Image { story, path, off, .. } | Placed::Graphic { story, path, off, .. } => {
                *story == me.story && *path == me.path && *off == me.off
            }
            Placed::Shape { object: Some(pos), .. } => pos == me,
            Placed::Lines { story: StoryRef::Part(id), .. } => self.boxes.contains(id),
            _ => false,
        }
    }
}

/// An option list with stable names for commands and saved preferences.
macro_rules! named {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum $name { $($(#[$vm])* $v),+ }
        impl $name {
            /// Every choice with its name, in menu order.
            pub const ALL: &'static [($name, &'static str)] = &[$(($name::$v, $s)),+];
            pub fn name(self) -> &'static str {
                Self::ALL.iter().find(|(v, _)| *v == self).map(|(_, s)| *s).unwrap_or("")
            }
            pub fn parse(s: &str) -> Option<Self> {
                Self::ALL.iter().find(|(_, n)| *n == s).map(|(v, _)| *v)
            }
        }
    };
}

named! {
    /// How inserted text is marked.
    InsertMark {
        #[default]
        Underline = "underline",
        DoubleUnderline = "doubleUnderline",
        Bold = "bold",
        Italic = "italic",
        Strikethrough = "strikethrough",
        ColorOnly = "colorOnly",
        None = "none",
    }
}

named! {
    /// How deleted text is marked.
    DeleteMark {
        #[default]
        Strikethrough = "strikethrough",
        DoubleStrikethrough = "doubleStrikethrough",
        Hidden = "hidden",
        Caret = "caret",
        Hash = "hash",
        Underline = "underline",
        ColorOnly = "colorOnly",
        /// No mark (an ordinary deletion keeps its colour; moved-away text is plain, as in Word).
        None = "none",
        Bold = "bold",
        Italic = "italic",
        DoubleUnderline = "doubleUnderline",
    }
}

named! {
    /// Where the bar beside changed lines goes.
    ChangeBar {
        #[default]
        Outside = "outside",
        Left = "left",
        Right = "right",
        None = "none",
    }
}

named! {
    /// What the markup area beside the page shows.
    BalloonMode {
        /// Deletions, formatting and comments in balloons; insertions stay inline. Where the
        /// editor shows balloons, deleted body text leaves the layout (`deletions_in_balloons`)
        /// and the editor draws a "Deleted: …" balloon at its place; elsewhere (no markup area,
        /// PDF, and headers, footers, notes and text boxes, which get no balloons) it is drawn
        /// inline as in `CommentsAndFormatting`.
        Revisions = "revisions",
        /// Everything inline, no markup area.
        Inline = "inline",
        #[default]
        CommentsAndFormatting = "commentsAndFormatting",
    }
}

named! {
    /// How comments are shown beside the page.
    CommentDisplay {
        /// Every comment in the margin, as in Word.
        #[default]
        Margin = "margin",
        /// Only the comment of the text under the caret (or the clicked one) floats beside the page.
        Selected = "selected",
    }
}

/// Track Changes Options: what markup shows and how revisions are drawn. A per-user preference,
/// as in Word (saved with the interface settings, not the document).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarkupOptions {
    pub comments: bool,
    pub ink: bool,
    pub insertions_deletions: bool,
    pub formatting: bool,
    pub balloons: BalloonMode,
    pub comment_display: CommentDisplay,
    pub insert_mark: InsertMark,
    /// `None` = by author.
    pub insert_color: Option<Rgb>,
    pub delete_mark: DeleteMark,
    /// `None` = by author.
    pub delete_color: Option<Rgb>,
    pub changed_lines: ChangeBar,
    /// `None` = automatic (dark grey).
    pub changed_lines_color: Option<Rgb>,
    /// Record formatting changes while tracking.
    pub track_formatting: bool,
    /// How the place moved text was taken from is marked (Moves › Moved from).
    pub move_from_mark: DeleteMark,
    /// `None` = by author.
    pub move_from_color: Option<Rgb>,
    /// How moved text is marked where it went (Moves › Moved to).
    pub move_to_mark: InsertMark,
    /// `None` = by author.
    pub move_to_color: Option<Rgb>,
}

impl Default for MarkupOptions {
    fn default() -> Self {
        MarkupOptions {
            comments: true,
            ink: true,
            insertions_deletions: true,
            formatting: true,
            balloons: BalloonMode::default(),
            comment_display: CommentDisplay::default(),
            insert_mark: InsertMark::default(),
            insert_color: None,
            delete_mark: DeleteMark::default(),
            delete_color: None,
            changed_lines: ChangeBar::default(),
            changed_lines_color: None,
            track_formatting: true,
            move_from_mark: DeleteMark::DoubleStrikethrough,
            move_from_color: Some(MOVE_COLOR),
            move_to_mark: InsertMark::DoubleUnderline,
            move_to_color: Some(MOVE_COLOR),
        }
    }
}

impl MarkupOptions {
    /// Whether a tracked deletion takes no room with markup shown: insertions and deletions
    /// aren't shown, or its mark is Hidden. Text a move took away (`moved_from`) follows Moved
    /// from, other deletions follow Deletions, so Deletions = Hidden leaves moved-away text shown
    /// (Word for Mac's behaviour for that combination is unverified: this is the one place to
    /// change it).
    pub fn hides_deletion(&self, moved_from: bool) -> bool {
        let mark = if moved_from { self.move_from_mark } else { self.delete_mark };
        !self.insertions_deletions || mark == DeleteMark::Hidden
    }

    /// The deletions the layout leaves out with markup shown ([`crate::LayoutOptions::hide_marked`]).
    pub fn hidden_deletions(&self) -> crate::HideDeleted {
        crate::HideDeleted { plain: self.hides_deletion(false), moved_from: self.hides_deletion(true) }
    }
}

/// Colour of moved text (both ends of a tracked move).
pub const MOVE_COLOR: Rgb = Rgb(0x1A, 0x7F, 0x37);

/// Colour for revisions by author index.
pub fn revision_color(i: u32) -> Rgb {
    const C: [Rgb; 6] =
        [Rgb(0xB0, 0x1E, 0x8F), Rgb(0x1E, 0x6E, 0xB0), Rgb(0x2E, 0x8B, 0x3E), Rgb(0xC0, 0x5A, 0x10), Rgb(0x70, 0x3C, 0xB0), Rgb(0x0E, 0x7C, 0x86)];
    C.get(i as usize % C.len()).copied().unwrap_or(C[0])
}

fn stroke_of(u: Underline) -> Stroke {
    match u {
        Underline::Dotted => Stroke::Dotted,
        Underline::Dash | Underline::DotDash | Underline::DotDotDash => Stroke::Dashed,
        Underline::Double => Stroke::Double,
        Underline::Wave | Underline::DoubleWave => Stroke::Wave,
        _ => Stroke::Solid,
    }
}

/// Draw items for a page.
pub fn page_display(doc: &Document, page: &Page, opts: &DisplayOptions) -> Vec<Draw> {
    let mut out = Vec::new();
    let (ha, ba) = dimming(opts);
    let authors = Authors::new(doc);
    stories_display(doc, &[&page.header, &page.footer], opts, &authors, ha, &mut out);
    stories_display(doc, &[&page.items], opts, &authors, ba, &mut out);
    body_change_bars(doc, page, opts, ba, &mut out);
    out
}

/// A page's draw items as two layers, its headers and footers then its body, each at full
/// strength with the opacity to draw it at. A dimmed layer is faded as one picture, as Word dims
/// it: where its lines or fills overlap (a table's borders meeting) they don't darken each other.
pub fn page_layers(doc: &Document, page: &Page, opts: &DisplayOptions) -> [(f32, Vec<Draw>); 2] {
    let (ha, ba) = dimming(opts);
    let (mut hf, mut body) = (Vec::new(), Vec::new());
    let authors = Authors::new(doc);
    stories_display(doc, &[&page.header, &page.footer], opts, &authors, 1.0, &mut hf);
    stories_display(doc, &[&page.items], opts, &authors, 1.0, &mut body);
    body_change_bars(doc, page, opts, 1.0, &mut body);
    [(ha, hf), (ba, body)]
}

/// The opacity of the headers/footers and of the body.
fn dimming(opts: &DisplayOptions) -> (f32, f32) {
    let a = |dim: bool| if dim { 0.45 } else { 1.0 };
    (a(opts.dim_header), a(opts.dim_body))
}

fn body_change_bars(doc: &Document, page: &Page, opts: &DisplayOptions, alpha: f32, out: &mut Vec<Draw>) {
    if opts.markup && (opts.revisions.insertions_deletions || opts.revisions.formatting) && opts.revisions.changed_lines != ChangeBar::None {
        change_bars(doc, page, &opts.revisions, alpha, out);
    }
}

/// Draw items for the placed items of `stories`, faded by `a`.
fn stories_display(doc: &Document, stories: &[&Vec<Placed>], opts: &DisplayOptions, authors: &Authors, a: f32, out: &mut Vec<Draw>) {
    let hidden = opts.hide_object.as_ref().map(|p| Hidden::new(doc, p));
    for items in stories {
        let warps = warped_boxes(doc, items);
        let clips = clipped_cells(items);
        let mut done: Vec<u32> = Vec::new();
        for it in items.iter() {
            // The object being dragged is drawn by its preview instead: leave it out here.
            if hidden.as_ref().is_some_and(|h| h.hides(it)) {
                continue;
            }
            if let Placed::Lines { story, path, .. } = it
                && let Some(rect) = cell_clip(&clips, *story, &path.0)
            {
                let mut inner = Vec::new();
                item(doc, it, opts, authors, a, &mut inner);
                out.push(Draw::Clip { rect, items: inner });
                continue;
            }
            match it {
                Placed::Lines { story: StoryRef::Part(id), .. } if warps.iter().any(|(w, ..)| w == id) => {
                    // A warped text box's lines are drawn together, bent, where its first one is.
                    if done.contains(id) {
                        continue;
                    }
                    done.push(*id);
                    if let Some((_, rect, warp)) = warps.iter().find(|(w, ..)| w == id) {
                        let lines: Vec<&Placed> =
                            items.iter().filter(|i| matches!(i, Placed::Lines { story: StoryRef::Part(s), .. } if s == id)).collect();
                        warped_text(doc, &lines, *rect, warp, opts, authors, a, out);
                    }
                }
                _ => item(doc, it, opts, authors, a, out),
            }
        }
    }
}

/// One object drawn alone, as its page draws it: the picture, shape or group `o` in `rect` (its
/// unturned frame) and the text of the text box story `text_box` on `page`. A dragged object's
/// preview, which mustn't show what was behind it (#1010).
pub fn object_display(doc: &Document, page: &Page, o: &InlineObject, rect: Rect, text_box: Option<u32>, opts: &DisplayOptions) -> Vec<Draw> {
    let mut out = Vec::new();
    object_draws(o, rect, Spin::default(), 1.0, opts, &mut out);
    let Some(id) = text_box else { return out };
    let authors = Authors::new(doc);
    for items in [&page.items, &page.header, &page.footer] {
        let lines: Vec<&Placed> = items.iter().filter(|i| matches!(i, Placed::Lines { story: StoryRef::Part(s), .. } if *s == id)).collect();
        match warped_boxes(doc, items).into_iter().find(|(w, ..)| *w == id) {
            Some((_, rect, warp)) if !lines.is_empty() => warped_text(doc, &lines, rect, &warp, opts, &authors, 1.0, &mut out),
            _ => lines.into_iter().for_each(|it| item(doc, it, opts, &authors, 1.0, &mut out)),
        }
    }
    out
}

/// The cells among `items` whose content is cut off at their edge (see [`Placed::Cell`]'s
/// `clip`), by story and the path of their content (table, row, cell): their area, and the
/// order they come in (the first of two that hold a line clips it).
type Clips = HashMap<StoryRef, HashMap<Vec<u32>, (usize, Rect)>>;

fn clipped_cells(items: &[Placed]) -> Clips {
    let mut clips = Clips::new();
    let cells = items.iter().filter_map(|it| match it {
        Placed::Cell { rect, table, row, cell, story, clip: true, .. } => {
            let mut prefix = table.0.clone();
            prefix.extend([u32::try_from(*row).ok()?, u32::try_from(*cell).ok()?]);
            Some((*story, prefix, *rect))
        }
        _ => None,
    });
    for (i, (story, prefix, rect)) in cells.enumerate() {
        crate::work::steps(1);
        clips.entry(story).or_default().entry(prefix).or_insert((i, rect));
    }
    clips
}

/// The area to clip a line at `path` of `story` to: its cell's, when that cell is clipped. Only
/// the cells holding the line are looked up, not every clipped cell on the page.
fn cell_clip(clips: &Clips, story: StoryRef, path: &[u32]) -> Option<Rect> {
    let cells = clips.get(&story)?;
    (0..=path.len())
        .filter_map(|n| {
            crate::work::steps(1);
            cells.get(path.get(..n)?)
        })
        .min_by_key(|(i, _)| *i)
        .map(|(_, r)| *r)
}

/// The text boxes among `items` whose text is warped (Text Effects › Transform, drawn bent): their
/// story, frame and warp. Empty, without a walk of the document, when none is.
fn warped_boxes(doc: &Document, items: &[Placed]) -> Vec<(u32, Rect, wordcraft_doc::wordart::TextWarp)> {
    items
        .iter()
        .filter_map(|it| {
            let Placed::Object { rect, story, path, off, text_box: Some(id), .. } = it else { return None };
            match doc.para(*story, path)?.object_at(*off)? {
                InlineObject::Shape { extra, .. } => extra.warp.clone().filter(|w| !w.is_plain()).map(|w| (*id, *rect, w)),
                _ => None,
            }
        })
        .collect()
}

/// The text of a warped text box at `rect`: its lines' letters as outlines bent by `warp`, the
/// box's text stretched over the frame inside its margins (as WordArt is). Underlines, marks and
/// other decorations are left out; a warp that is drawn flat leaves the text as it is.
fn warped_text(
    doc: &Document,
    lines: &[&Placed],
    rect: Rect,
    warp: &wordcraft_doc::wordart::TextWarp,
    opts: &DisplayOptions,
    authors: &Authors,
    alpha: f32,
    out: &mut Vec<Draw>,
) {
    let mut raw = Vec::new();
    for it in lines {
        item(doc, it, opts, authors, alpha, &mut raw);
    }
    // The letters: runs with effects as they are, plain runs from their glyphs.
    let mut arts: Vec<(BezPath, Box<wordcraft_doc::wordart::TextEffects>, Rgb, f32)> = Vec::new();
    let mut hidden = Vec::new();
    for d in &raw {
        match d {
            Draw::Art { path, fx, color, alpha, .. } => arts.push((path.clone(), fx.clone(), *color, *alpha)),
            Draw::Glyphs { face, size, glyphs, color, alpha, synth_italic, .. } => {
                if *alpha > 0.0 {
                    arts.push((crate::wordart::glyph_path(face, *size, glyphs, *synth_italic), Box::default(), *color, *alpha));
                }
                let mut g = d.clone();
                if let Draw::Glyphs { alpha, .. } = &mut g {
                    *alpha = 0.0;
                }
                hidden.push(g);
            }
            _ => {}
        }
    }
    let src = arts.iter().filter_map(|(p, ..)| crate::wordart::path_box(p)).reduce(|a, b| a.union(b));
    let inset = Rect::new(rect.x + 7.2, rect.y + 3.6, (rect.w - 14.4).max(1.0), (rect.h - 7.2).max(1.0));
    let fitted = src.and_then(|b| crate::wordart::Warp::new(warp, inset, (b.height() / b.width().max(1e-3)) as f32).map(|w| (b, w)));
    let Some((src, w)) = fitted else {
        out.extend(raw);
        return;
    };
    for (path, fx, color, alpha) in arts {
        // No text background: the bent letters leave their highlight behind.
        out.push(Draw::Art { path: crate::wordart::warp_path(&path, src, &w), fx, color, alpha, text_bg: None });
    }
    out.extend(hidden);
}

/// The automatic colour of the bars beside changed lines.
pub const CHANGE_BAR: Rgb = Rgb(0x50, 0x50, 0x50);

/// Bars in the margin beside body lines with tracked insertions or deletions, or (when shown)
/// tracked formatting changes of their text, paragraph or paragraph mark.
fn change_bars(doc: &Document, page: &Page, m: &MarkupOptions, alpha: f32, out: &mut Vec<Draw>) {
    // Outside: the left margin, or the outer one of a right-hand page with mirrored margins.
    let right = match m.changed_lines {
        ChangeBar::Right => true,
        ChangeBar::Outside => doc.settings.mirror_margins && page.number % 2 == 1,
        _ => false,
    };
    let bx = if right { page.body.right() + 9.0 } else { page.body.x - 9.0 };
    let color = m.changed_lines_color.unwrap_or(CHANGE_BAR);
    for it in &page.items {
        let Placed::Lines { story, path, para: pl, l0, l1, y, turn, .. } = it else { continue };
        if turn.is_turned() {
            continue;
        }
        let Some(first) = pl.lines.get(*l0) else { continue };
        let (text, fmt) = (m.insertions_deletions, m.formatting);
        let para = doc.para(*story, path);
        let mark_rev = para.is_some_and(|p| (text && (p.mark.ins.is_some() || p.mark.del.is_some())) || (fmt && p.mark.fmt_change.is_some()));
        // A paragraph whose own formatting changed is marked on every line.
        let para_fmt = fmt && para.is_some_and(|p| p.props.fmt_change.is_some());
        for li in *l0..*l1 {
            let Some(line) = pl.lines.get(li) else { continue };
            let changed = para_fmt
                || (line.c0..line.c1)
                    .filter_map(|k| pl.clusters.get(k).and_then(|c| pl.styles.get(c.style as usize)))
                    .any(|st| (text && (st.rc.ins.is_some() || st.rc.del.is_some())) || (fmt && st.rc.fmt.is_some()))
                || (mark_rev && line.end == LineEnd::Para);
            if changed {
                let top = y + (line.top - first.top);
                out.push(Draw::Line {
                    x0: bx,
                    y0: top,
                    x1: bx,
                    y1: top + line.height,
                    width: 0.75,
                    color,
                    stroke: Stroke::Solid,
                    alpha,
                    text_bg: None,
                });
            }
        }
    }
}

fn border_stroke(s: BorderStyle) -> Stroke {
    match s {
        BorderStyle::Dotted => Stroke::BorderDotted,
        BorderStyle::Dashed => Stroke::BorderDashed,
        BorderStyle::DotDash => Stroke::BorderDotDash,
        BorderStyle::Double => Stroke::Double,
        BorderStyle::Triple => Stroke::Triple,
        BorderStyle::Wave => Stroke::Wave,
        _ => Stroke::Solid,
    }
}

/// A border line from `(x0, y0)` to `(x1, y1)` (paragraph rules and character borders).
fn rule(x0: f32, y0: f32, x1: f32, y1: f32, b: &Border, alpha: f32) -> Draw {
    Draw::Line {
        x0,
        y0,
        x1,
        y1,
        width: if b.style == BorderStyle::Thick { b.width.max(1.5) } else { b.width.max(0.25) },
        color: b.color.unwrap_or(Rgb::BLACK),
        stroke: border_stroke(b.style),
        alpha,
        text_bg: None,
    }
}

/// Closed box around one line segment of a character-border group, widened by `space`.
// Note: border drawn outside text advance, reserve width in line breaking if overlap shows.
fn char_box(b: &Border, x0: f32, x1: f32, top: f32, bottom: f32, alpha: f32, out: &mut Vec<Draw>) {
    let (l, r) = (x0 - b.space, x1 + b.space);
    out.push(rule(l, top, r, top, b, alpha));
    out.push(rule(l, bottom, r, bottom, b, alpha));
    out.push(rule(l, top, l, bottom, b, alpha));
    out.push(rule(r, top, r, bottom, b, alpha));
}

fn item(doc: &Document, it: &Placed, opts: &DisplayOptions, authors: &Authors, alpha: f32, out: &mut Vec<Draw>) {
    item_in(doc, it, opts, authors, alpha, Spin::default(), out)
}

/// [`item`] drawn inside the turn `outer` (a turned Drawing Canvas's members): a shape's
/// effects follow all the turns it is drawn inside.
fn item_in(doc: &Document, it: &Placed, opts: &DisplayOptions, authors: &Authors, alpha: f32, outer: Spin, out: &mut Vec<Draw>) {
    match it {
        Placed::Fill { rect, color } => out.push(Draw::Fill { rect: *rect, color: *color, alpha, text_bg: false }),
        Placed::Rule { x0, y0, x1, y1, border } => out.push(rule(*x0, *y0, *x1, *y1, border, alpha)),
        Placed::Image { rect, media, crop, spin, .. } => spun(
            *spin,
            *rect,
            vec![Draw::Image { rect: *rect, media: media.clone(), crop: *crop, alpha, alt: String::new(), decorative: false }],
            out,
        ),
        Placed::Shape { rect, kind, fill, stroke, stroke_width, effects, freeform, spin, extra, .. } => {
            let mut items = Vec::new();
            let fx = effects_in(*effects, spin.within(outer));
            shape_draws(*rect, *kind, fill.as_ref(), *stroke, *stroke_width, fx, freeform.as_deref(), extra, opts, &mut items);
            spun(*spin, *rect, items, out)
        }
        Placed::Graphic { rect, graphic, spin, .. } => spun(*spin, *rect, graphic_draws(doc, graphic, *rect, alpha), out),
        Placed::Clip { rect, spin, items } => {
            let mut inner = Vec::new();
            for it in items {
                item_in(doc, it, opts, authors, alpha, spin.within(outer), &mut inner);
            }
            spun(*spin, *rect, vec![Draw::Clip { rect: *rect, items: inner }], out)
        }
        Placed::Cell { .. } | Placed::Object { .. } => {}
        Placed::Lines { story, path, para, l0, l1, x, y, turn } if turn.is_turned() => {
            let mut items = Vec::new();
            lines(doc, *story, path, para, *l0, *l1, 0.0, 0.0, opts, authors, alpha, *turn == TextDirection::Down, &mut items);
            // Link areas are axis-aligned page rectangles: turned text gets none.
            for d in &mut items {
                if let Draw::Glyphs { link, .. } = d {
                    *link = None;
                }
            }
            if *turn == TextDirection::Down {
                items = upright_east_asian(items);
            }
            out.push(Draw::Turned { x: *x, y: *y, turn: *turn, items });
        }
        Placed::Lines { story, path, para, l0, l1, x, y, .. } => lines(doc, *story, path, para, *l0, *l1, *x, *y, opts, authors, alpha, false, out),
    }
}

/// Append display items for the half-open line range `l0..l1` of a laid-out paragraph.
/// `(x, y)` positions the paragraph's left origin and the first selected line's top; later lines
/// retain their layout offsets. Includes glyphs, inline objects, decorations and requested markup.
/// `down` keeps vertical East Asian glyphs upright before the caller turns the resulting draws.
fn lines(
    doc: &Document,
    story: StoryRef,
    path: &Path,
    pl: &ParaLayout,
    l0: usize,
    l1: usize,
    x: f32,
    y: f32,
    opts: &DisplayOptions,
    authors: &Authors,
    alpha: f32,
    down: bool,
    out: &mut Vec<Draw>,
) {
    let Some(first) = pl.lines.get(l0) else { return };
    let para = doc.para(story, path);
    // Inside a text box whose shape style gives its text a colour, that is what automatic means.
    let auto = match story {
        StoryRef::Part(id) => doc.parts.get(&id).and_then(|p| p.body.auto_color),
        StoryRef::Body => None,
    };
    let ink = |c: &TextColor, background: Option<Rgb>| match (c, auto) {
        (TextColor::Auto, Some(rgb)) => rgb,
        _ => text_color(c, background),
    };
    for li in l0..l1 {
        let Some(line) = pl.lines.get(li) else { continue };
        let top = y + (line.top - first.top);
        let base = top + (line.baseline - line.top);
        let bottom = top + line.height;
        // Commented text gets a soft shade (comment anchors are object markers).
        if opts.markup
            && opts.revisions.comments
            && let Some(p) = para
        {
            let mut open: Option<usize> = None;
            let mut ranges = Vec::new();
            for off in p.object_offsets() {
                match p.object_at(off) {
                    Some(InlineObject::CommentStart { .. }) => open = open.or(Some(off)),
                    Some(InlineObject::CommentEnd { .. }) => {
                        ranges.push((open.take().unwrap_or(0), off));
                    }
                    _ => {}
                }
            }
            if let Some(o) = open {
                ranges.push((o, p.len()));
            }
            for (a, b) in ranges {
                if b <= line.start || a >= line.stop {
                    continue;
                }
                for (x0, x1) in pl.x_spans(li, a.max(line.start), b.min(line.stop)) {
                    if x1 > x0 {
                        out.push(Draw::Fill {
                            rect: Rect::new(x + x0, top, x1 - x0, line.height),
                            color: Rgb(0xEF, 0xE3, 0xF7),
                            alpha,
                            text_bg: false,
                        });
                    }
                }
            }
        }
        // Backgrounds first: highlight and character shading; character borders group equal adjacent borders.
        let mut open: Option<(Border, f32, f32)> = None;
        for k in line.c0..line.c1 {
            let (Some(c), Some(cx), Some(nx)) = (pl.clusters.get(k), line.cl_left(k), line.cl_right(k)) else { continue };
            let Some(st) = pl.styles.get(c.style as usize) else { continue };
            if c.kind == ClKind::Marker {
                continue;
            }
            if let Some(h) = st.rc.highlight.or(st.rc.shading) {
                out.push(Draw::Fill { rect: Rect::new(x + cx, top, (nx - cx).max(0.0), line.height), color: h, alpha, text_bg: true });
            }
            let b = st.rc.border;
            if let Some((ob, x0, x1)) = open.as_mut()
                && Some(*ob) == b
            {
                // Right-to-left text grows leftwards.
                *x0 = x0.min(x + cx);
                *x1 = x1.max(x + nx);
                continue;
            }
            if let Some((ob, x0, x1)) = open.take() {
                char_box(&ob, x0, x1, top, bottom, alpha, out);
            }
            open = b.map(|b| (b, x + cx, x + nx));
        }
        if let Some((ob, x0, x1)) = open {
            char_box(&ob, x0, x1, top, bottom, alpha, out);
        }
        // Label.
        if li == pl.label_line()
            && let Some(lab) = &pl.label
        {
            let gx = x + lab.x;
            // One draw per font run: a glyph id only means something in its own face.
            for run in &lab.runs {
                let Some(st) = pl.styles.get(run.style as usize) else { continue };
                out.push(Draw::Glyphs {
                    face: st.face,
                    size: st.size,
                    glyphs: lab.glyphs.get(run.glyphs.clone()).into_iter().flatten().map(|g| (g.gid, gx + g.dx, base - st.shift - g.dy)).collect(),
                    color: ink(&st.rc.color, None),
                    alpha,
                    synth_bold: st.synth_bold,
                    synth_italic: st.synth_italic,
                    text: run.text.clone(),
                    link: None,
                    ranges: Vec::new(),
                    text_bg: None,
                    x_scale: 1.0,
                });
            }
        }
        // Hyphen at a hyphenated line end.
        if let Some((si, gid, adv)) = line.hyphen
            && let Some(st) = pl.styles.get(si as usize)
        {
            let hx = x + line.hyphen_x(adv);
            out.push(Draw::Glyphs {
                face: st.face,
                size: st.size,
                glyphs: vec![(gid, hx, base - st.shift)],
                color: ink(&st.rc.color, None),
                alpha,
                synth_bold: st.synth_bold,
                synth_italic: st.synth_italic,
                text: "-".into(),
                link: None,
                ranges: Vec::new(),
                text_bg: None,
                x_scale: st.rc.scale / 100.0,
            });
        }
        // Glyph runs grouped by style.
        let mut k = line.c0;
        while k < line.c1 {
            let Some(c) = pl.clusters.get(k) else { break };
            let si = c.style;
            let Some(st) = pl.styles.get(si as usize) else {
                k += 1;
                continue;
            };
            let mut glyphs = Vec::new();
            let mut text = String::new();
            let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
            let run_start = k;
            while k < line.c1 {
                let Some(c) = pl.clusters.get(k) else { break };
                if c.style != si {
                    break;
                }
                let cx = x + line.cl_left(k).unwrap_or(0.0);
                if matches!(c.kind, ClKind::Text | ClKind::Space) {
                    let a = text.len();
                    // An object's cluster (a field, a note number) stands for the text it shows.
                    let mut shown = None;
                    if let Ok(i) = pl.shown.binary_search_by_key(&k, |(i, _, _)| *i)
                        && let Some((_, s, r)) = pl.shown.get(i)
                    {
                        // All Caps field results read as the capitals drawn (#1473). `r` holds
                        // byte ranges into `s`, so a character whose capital has another UTF-8
                        // length keeps its own form and every range still lands where it did.
                        text.extend(s.chars().map(|ch| {
                            let up = crate::para::caps_char(&st.rc, ch);
                            if up.len_utf8() == ch.len_utf8() { up } else { ch }
                        }));
                        shown = Some(r);
                    } else if let Some(p) = para {
                        let t = p.text.get(c.start..c.end).unwrap_or("");
                        // Symbol-font characters (Wingdings…) read as the Unicode symbols drawn.
                        if wordcraft_doc::symbol_fonts::is_symbol_font(&st.rc.font) {
                            text.push_str(&wordcraft_doc::symbol_fonts::map_text(&st.rc.font, t));
                        } else if st.rc.caps || st.rc.small_caps {
                            // All Caps and Small Caps read as the capitals drawn (#1473), mapped
                            // as drawn, so a glyph never stands for two cases and ß reads as SS (#1581).
                            for ch in t.chars() {
                                crate::para::push_caps(&st.rc, ch, &mut text);
                            }
                        } else {
                            text.push_str(t);
                        }
                    }
                    let b = text.len();
                    let cg = pl.glyphs.get(c.g0 as usize..c.g1 as usize).unwrap_or(&[]);
                    if let Some(shown) = shown
                        && cg.len() > 1
                    {
                        // A whole field result is one cluster of many glyphs: each glyph gets the
                        // text it was shaped from (a ligature its letters), not the whole text as
                        // one ligature. Without that, a character each (the last takes any remainder).
                        let bounds: Vec<usize> = text.get(a..b).unwrap_or("").char_indices().map(|(i, _)| a + i).collect();
                        let known = shown.len() == cg.len();
                        for (n, g) in cg.iter().enumerate() {
                            let r = match shown.get(n).filter(|_| known) {
                                Some(r) => (a + r.start).min(b)..(a + r.end).min(b),
                                None => {
                                    let from = bounds.get(n).or(bounds.last()).copied().unwrap_or(a);
                                    let to = if n + 1 == cg.len() { b } else { bounds.get(n + 1).copied().unwrap_or(b) };
                                    from..to
                                }
                            };
                            glyphs.push((g.gid, cx + g.dx, base - st.shift - g.dy));
                            ranges.push(r.start..r.end.max(r.start));
                        }
                        k += 1;
                        continue;
                    }
                    if cg.is_empty() {
                        // No glyph of its own (the second letter of a ligature such as لا): its
                        // text belongs to the glyphs before it.
                        let from = ranges.last().map(|r| r.start);
                        for r in ranges.iter_mut().rev().take_while(|r| Some(r.start) == from) {
                            r.end = b;
                        }
                    }
                    for g in cg {
                        glyphs.push((g.gid, cx + g.dx, base - st.shift - g.dy));
                        ranges.push(a..b);
                    }
                }
                k += 1;
            }
            let run_end = k;
            let rc = &st.rc;
            let m = &opts.revisions;
            // Without markup a deletion is never drawn as ordinary text, even in a layout that
            // kept it; nor when its mark is Hidden.
            if rc.del.is_some() && (!opts.markup || m.hides_deletion(doc.move_name(rc.del).is_some())) {
                continue;
            }
            let shown = opts.markup && m.insertions_deletions;
            let del = rc.del.filter(|_| shown);
            let ins = rc.ins.filter(|_| shown && del.is_none());
            let author = |r: u32| authors.color(r);
            // Moved text: the Moves marks and colours (by default green, double strikethrough
            // where it was, double underline where it went).
            let moved_to = doc.move_name(ins).is_some();
            let moved_from = del.is_some() && doc.move_name(del).is_some();
            // Moved text marked (none) is plain text, its colour setting aside (Word for Mac).
            let plain_move = (moved_from && m.move_from_mark == DeleteMark::None) || (moved_to && m.move_to_mark == InsertMark::None);
            let color = match (ins, del) {
                _ if plain_move => text_color(&rc.color, rc.shading.or(rc.highlight)),
                (_, Some(r)) if moved_from => m.move_from_color.unwrap_or_else(|| author(r)),
                (Some(r), _) if moved_to => m.move_to_color.unwrap_or_else(|| author(r)),
                (Some(r), _) => m.insert_color.unwrap_or_else(|| author(r)),
                (None, Some(r)) => m.delete_color.unwrap_or_else(|| author(r)),
                _ => ink(&rc.color, rc.shading.or(rc.highlight)),
            };
            // The highlight or shading drawn under this run (as filled in the backgrounds pass).
            let text_bg = rc.highlight.or(rc.shading);
            let ins_mark = ins.map(|_| if moved_to { m.move_to_mark } else { m.insert_mark });
            let del_mark = del.map(|_| if moved_from { m.move_from_mark } else { m.delete_mark });
            if del_mark == Some(DeleteMark::Caret) {
                // A caret where the text was, instead of the text.
                if let Some(cx) = line.cl_left(run_start).map(|c| x + c) {
                    let h = st.size * 0.3;
                    let w = (st.size / 18.0).max(0.6);
                    out.push(Draw::Line {
                        x0: cx - h * 0.6,
                        y0: base + 1.0,
                        x1: cx,
                        y1: base + 1.0 - h,
                        width: w,
                        color,
                        stroke: Stroke::Solid,
                        alpha,
                        text_bg,
                    });
                    out.push(Draw::Line {
                        x0: cx,
                        y0: base + 1.0 - h,
                        x1: cx + h * 0.6,
                        y1: base + 1.0,
                        width: w,
                        color,
                        stroke: Stroke::Solid,
                        alpha,
                        text_bg,
                    });
                }
                continue;
            }
            if del_mark == Some(DeleteMark::Hash) {
                // Each deleted character shows as #.
                let gid = st.face.glyph_for('#');
                for g in &mut glyphs {
                    g.0 = gid;
                }
            }
            let synth_bold = st.synth_bold || ins_mark == Some(InsertMark::Bold) || del_mark == Some(DeleteMark::Bold);
            let synth_italic = st.synth_italic || ins_mark == Some(InsertMark::Italic) || del_mark == Some(DeleteMark::Italic);
            // Text effects draw the letters from their outlines; the glyphs stay, invisible, for
            // the PDF's text. Not for colour glyphs (no outlines), top-to-bottom text with upright
            // characters (their outlines would lie on their side), `#` deletion marks, or WordArt
            // in revision colours.
            let upright = down && text.chars().any(upright_in_vertical);
            let art = (!upright
                && !glyphs.is_empty()
                && del_mark != Some(DeleteMark::Hash)
                && (rc.text_effects.is_none() || ins.is_none() && del.is_none()))
            .then(|| crate::wordart::run_effects(rc, color))
            .flatten()
            .filter(|_| !st.face.has_color_glyphs());
            let has_art = art.is_some();
            if let Some(mut fx) = art {
                if synth_bold {
                    crate::wordart::embolden(&mut fx, color, st.size);
                }
                let path = crate::wordart::glyph_path(&st.face, st.size, &glyphs, synth_italic);
                out.push(Draw::Art { path, fx: Box::new(fx), color, alpha, text_bg });
            }
            if !glyphs.is_empty() {
                out.push(Draw::Glyphs {
                    face: st.face,
                    size: st.size,
                    glyphs,
                    color,
                    alpha: if has_art { 0.0 } else { alpha },
                    synth_bold,
                    synth_italic,
                    text,
                    link: rc.link.clone(),
                    ranges,
                    text_bg,
                    x_scale: rc.scale / 100.0,
                });
            }
            // Decorations across the run (underline skips trailing spaces of the line). Right-to-left
            // text inside a run can split it into several visual spans.
            let mut end_k = run_end;
            while end_k > run_start
                && pl.clusters.get(end_k - 1).is_some_and(|c| matches!(c.kind, ClKind::Space | ClKind::Marker | ClKind::LineBreak))
                && end_k == line.c1
            {
                end_k -= 1;
            }
            let thick = (st.size / 18.0).max(0.5);
            let underline = match (ins_mark, del_mark) {
                (Some(InsertMark::Underline), _) | (_, Some(DeleteMark::Underline)) => Underline::Single,
                (Some(InsertMark::DoubleUnderline), _) | (_, Some(DeleteMark::DoubleUnderline)) => Underline::Double,
                _ => rc.underline,
            };
            let double = rc.double_strike || del_mark == Some(DeleteMark::DoubleStrikethrough);
            let struck = rc.strike || double || ins_mark == Some(InsertMark::Strikethrough) || del_mark == Some(DeleteMark::Strikethrough);
            let spans = if underline != Underline::None || struck { line.spans(run_start, end_k) } else { Vec::new() };
            for (x0, x1) in spans.into_iter().map(|(a, b)| (x + a, x + b)).filter(|(a, b)| b > a) {
                if underline != Underline::None {
                    let uy = base - st.shift + st.size * 0.12;
                    let ucolor = rc.underline_color.unwrap_or(color);
                    if underline == Underline::Words {
                        for kk in run_start..end_k {
                            let Some(c) = pl.clusters.get(kk) else { continue };
                            if c.kind != ClKind::Text {
                                continue;
                            }
                            let a = x + line.cl_left(kk).unwrap_or(0.0);
                            let b = x + line.cl_right(kk).unwrap_or(a - x);
                            if a < x0 - 0.01 || b > x1 + 0.01 {
                                continue; // drawn with its own span
                            }
                            out.push(Draw::Line { x0: a, y0: uy, x1: b, y1: uy, width: thick, color: ucolor, stroke: Stroke::Solid, alpha, text_bg });
                        }
                    } else {
                        let w = if underline == Underline::Thick { thick * 2.0 } else { thick };
                        out.push(Draw::Line { x0, y0: uy, x1, y1: uy, width: w, color: ucolor, stroke: stroke_of(underline), alpha, text_bg });
                    }
                }
                if struck {
                    let sy = base - st.shift - st.size * 0.28;
                    let stroke = if double { Stroke::Double } else { Stroke::Solid };
                    out.push(Draw::Line { x0, y0: sy, x1, y1: sy, width: thick, color, stroke, alpha, text_bg });
                }
            }
        }
        // Proofing squiggles.
        for (a, b, grammar) in &pl.issues {
            if *b <= line.start || *a >= line.stop {
                continue;
            }
            for (x0, x1) in pl.x_spans(li, (*a).max(line.start), (*b).min(line.stop)) {
                if x1 - x0 < 1.0 {
                    continue;
                }
                let y = base + 2.5;
                let color = if *grammar { Rgb(0x2B, 0x57, 0xC0) } else { Rgb(0xE0, 0x24, 0x24) };
                out.push(Draw::Line {
                    x0: x + x0,
                    y0: y,
                    x1: x + x1,
                    y1: y,
                    width: 0.8,
                    color,
                    stroke: if *grammar { Stroke::Double } else { Stroke::Wave },
                    alpha,
                    text_bg: None,
                });
            }
        }
        // Tab leaders.
        for (k, leader) in &line.leaders {
            let (Some(c), Some(a), Some(b)) = (pl.clusters.get(*k), line.cl_left(*k), line.cl_right(*k)) else { continue };
            let Some(st) = pl.styles.get(c.style as usize) else { continue };
            let Some(ch) = leader.char() else { continue };
            let gid = st.face.glyph_for(ch);
            let adv = (st.face.advance(gid) as f32 * st.size / st.face.upem.max(1.0) as f32).max(1.0);
            let (mut gx, end) = (x + a + adv * 0.5, x + b - adv * 1.2);
            let mut glyphs = Vec::new();
            // Align leader dots to a grid so consecutive lines line up.
            gx = (gx / adv).ceil() * adv;
            while gx < end && glyphs.len() < 2000 {
                glyphs.push((gid, gx, base));
                gx += adv;
            }
            out.push(Draw::Glyphs {
                face: st.face,
                size: st.size,
                glyphs,
                color: ink(&st.rc.color, None),
                alpha,
                synth_bold: false,
                synth_italic: false,
                text: String::new(),
                link: None,
                ranges: Vec::new(),
                text_bg: None,
                x_scale: 1.0,
            });
        }
        // Inline objects.
        for k in line.c0..line.c1 {
            let Some(c) = pl.clusters.get(k) else { continue };
            let ClKind::Object(oi) = c.kind else { continue };
            if c.obj_h <= 0.0 {
                continue;
            }
            // A dragged inline object is left out too: its preview stands in for it.
            if opts.hide_object.as_ref().is_some_and(|p| p.story == story && p.path == *path && p.off == c.start) {
                continue;
            }
            let cx = x + line.cl_left(k).unwrap_or(0.0);
            let obj = para.and_then(|p| p.objects.get(oi));
            let rect = inline_rect(obj, cx, base, c.adv, c.obj_h);
            let spin = obj.and_then(InlineObject::frame).map(|(_, _, f)| f.spin()).unwrap_or_default();
            match obj {
                Some(o @ (InlineObject::Image { .. } | InlineObject::Shape { .. } | InlineObject::Group { .. })) => {
                    object_draws(o, rect, Spin::default(), alpha, opts, out)
                }
                Some(InlineObject::Graphic { graphic, alt, .. }) => {
                    let mut draws = Vec::new();
                    spun(spin, rect, graphic_draws(doc, graphic, rect, alpha), &mut draws);
                    out.push(Draw::Figure { alt: alt.clone(), kind: Some(graphic.kind), draws })
                }
                Some(InlineObject::Equation { .. }) => {
                    if let Some((_, ml)) = pl.maths.iter().find(|(k, _)| *k == oi) {
                        equation(&ml.items, cx, base, alpha, opts.placeholders, out);
                    }
                }
                Some(o @ InlineObject::Field { .. }) => {
                    if let Some(wordcraft_doc::legacy_form::LegacyForm::CheckBox { checked, .. }) = o.legacy_form() {
                        let color = pl.styles.get(c.style as usize).map_or(Rgb::BLACK, |st| text_color(&st.rc.color, None));
                        let frame = Rect::new(cx, base - c.obj_h, c.adv, c.obj_h + c.obj_d);
                        check_box(frame, checked, color, alpha, out);
                    }
                }
                _ => {}
            }
        }
        // Enclosed characters' shapes, in the text's colour.
        for (k, e) in &pl.enclosures {
            if *k < line.c0 || *k >= line.c1 {
                continue;
            }
            let (Some(c), Some(left)) = (pl.clusters.get(*k), line.cl_left(*k)) else { continue };
            let Some(st) = pl.styles.get(c.style as usize) else { continue };
            let rect = Rect::new(x + left + e.x, base - st.shift - e.top, e.w, e.h);
            out.push(Draw::Shape {
                rect,
                kind: e.kind,
                fill: None,
                stroke: Some(ink(&st.rc.color, None)),
                stroke_width: e.stroke,
                effects: Default::default(),
                corner: None,
                dash: Vec::new(),
                cap: Default::default(),
            });
        }
        // Ruby (Phonetic Guide) text over its base text, in runs of one style.
        for r in pl.rubies.iter().filter(|r| (line.c0..line.c1).contains(&r.cluster)) {
            let Some(bst) = pl.clusters.get(r.cluster).and_then(|c| pl.styles.get(c.style as usize)) else { continue };
            if bst.rc.del.is_some() && !opts.markup {
                continue;
            }
            let cx = x + line.cl_left(r.cluster).unwrap_or(0.0);
            let y = base - bst.shift - r.raise;
            let mut i = 0;
            while let Some(&(style, _)) = r.glyphs.get(i) {
                let n = r.glyphs.get(i..).map_or(0, |g| g.iter().take_while(|(s, _)| *s == style).count()).max(1);
                let run = r.glyphs.get(i..i + n).unwrap_or(&[]);
                let ranges = r.ranges.get(i..i + n).map(<[_]>::to_vec).unwrap_or_default();
                i += n;
                let Some(st) = pl.styles.get(style as usize) else { continue };
                out.push(Draw::Glyphs {
                    face: st.face,
                    size: st.size,
                    glyphs: run.iter().map(|(_, g)| (g.gid, cx + g.dx, y - g.dy)).collect(),
                    color: ink(&st.rc.color, st.rc.shading.or(st.rc.highlight)),
                    alpha,
                    synth_bold: st.synth_bold,
                    synth_italic: st.synth_italic,
                    text: r.text.clone(),
                    link: None,
                    ranges,
                    text_bg: st.rc.highlight.or(st.rc.shading),
                    x_scale: 1.0,
                });
            }
        }
        // Formatting marks.
        let marks = opts.always.shown(opts.marks);
        if marks.any() {
            let msize = pl.styles.first().map(|s| s.size).unwrap_or(11.0);
            for k in line.c0..line.c1 {
                let Some(c) = pl.clusters.get(k) else { continue };
                let cx = x + line.cl_left(k).unwrap_or(0.0);
                let nx = x + line.cl_right(k).unwrap_or(cx - x);
                let size = pl.styles.get(c.style as usize).map(|s| s.size).unwrap_or(msize);
                let arrow = if line.rtl { '←' } else { '→' };
                match c.kind {
                    ClKind::Space if marks.spaces => {
                        out.push(Draw::Mark { x: (cx + nx) / 2.0 - size * 0.12, baseline: base - size * 0.08, size, ch: '·', color: None })
                    }
                    ClKind::Tab if marks.tabs => {
                        out.push(Draw::Mark { x: cx + ((nx - cx) / 2.0 - size * 0.3).max(0.0), baseline: base, size, ch: arrow, color: None })
                    }
                    ClKind::LineBreak if marks.paragraphs => out.push(Draw::Mark { x: cx + 1.0, baseline: base, size, ch: '↵', color: None }),
                    ClKind::PageBreak | ClKind::ColumnBreak if marks.paragraphs => {
                        let label = if c.kind == ClKind::PageBreak { '⤓' } else { '⇥' };
                        out.push(Draw::Line {
                            x0: cx + 2.0,
                            y0: base - size * 0.3,
                            x1: x + line.right,
                            y1: base - size * 0.3,
                            width: 0.5,
                            color: Rgb(0x60, 0x60, 0x60),
                            stroke: Stroke::Dotted,
                            alpha,
                            text_bg: None,
                        });
                        out.push(Draw::Mark { x: cx + 1.0, baseline: base, size, ch: label, color: None });
                    }
                    _ => {}
                }
            }
            // A deleted paragraph mark that isn't shown (no markup, or its mark is Hidden) draws no ¶.
            let mark_gone =
                para.is_some_and(|p| p.mark.del.is_some() && (!opts.markup || opts.revisions.hides_deletion(doc.move_name(p.mark.del).is_some())));
            if line.end == LineEnd::Para && marks.paragraphs && !mark_gone {
                // The mark follows the line's visual end, past every glyph, even where the
                // logically last text runs against the paragraph's direction.
                let ex = x + line.visual_end_x();
                let size = pl.lines.first().map(|_| msize).unwrap_or(msize);
                // A right-to-left paragraph's mark sits at its end, on the left.
                let mx = if line.rtl { ex - 1.0 - size * 0.6 } else { ex + 1.0 };
                // A tracked (inserted or deleted) paragraph mark is drawn in its author's colour.
                let m = &opts.revisions;
                let by_author = |r: u32| authors.color(r);
                let color = para.filter(|_| opts.markup && m.insertions_deletions).and_then(|p| match (p.mark.ins, p.mark.del) {
                    (_, Some(r)) if doc.move_name(Some(r)).is_some() => Some(m.move_from_color.unwrap_or_else(|| by_author(r))),
                    (Some(r), None) if doc.move_name(Some(r)).is_some() => Some(m.move_to_color.unwrap_or_else(|| by_author(r))),
                    (_, Some(r)) => Some(m.delete_color.unwrap_or_else(|| by_author(r))),
                    (Some(r), None) => Some(m.insert_color.unwrap_or_else(|| by_author(r))),
                    _ => None,
                });
                // The last paragraph of a section ends in a section break instead of a plain ¶.
                if let Some(start) = section_break_after(doc, story, path) {
                    let (x0, x1) = if line.rtl { (x + line.left, ex - 2.0) } else { (ex + 2.0, x + line.right) };
                    section_break_mark(start, x0, x1, base, size, alpha, out);
                } else {
                    out.push(Draw::Mark { x: mx, baseline: base, size, ch: '¶', color });
                }
            }
        }
        let _ = bottom;
    }
}

/// The kind of section break that ends at body paragraph `path`: the start type of the section
/// that follows it. `None` for any other paragraph, and for the document's last section.
fn section_break_after(doc: &Document, story: StoryRef, path: &Path) -> Option<SectionStart> {
    if story != StoryRef::Body || path.0.len() != 1 {
        return None;
    }
    let i = *path.0.first()? as usize;
    match doc.body.get(i).map(|b| &**b) {
        Some(Block::Para(p)) if p.section.is_some() => {}
        _ => return None,
    }
    let secs = doc.sections();
    let k = secs.iter().position(|(end, _)| *end == i)?;
    secs.get(k + 1).map(|(_, s)| s.start)
}

/// On-screen name of a section break.
pub fn section_break_label(start: SectionStart) -> &'static str {
    match start {
        SectionStart::NextPage => "Section Break (Next Page)",
        SectionStart::Continuous => "Section Break (Continuous)",
        SectionStart::EvenPage => "Section Break (Even Page)",
        SectionStart::OddPage => "Section Break (Odd Page)",
        SectionStart::NextColumn => "Section Break (Next Column)",
    }
}

/// Width of a [`Draw::MarkText`] label.
pub fn mark_text_width(text: &str, size: f32) -> f32 {
    let face = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    let k = size / face.upem.max(1.0) as f32;
    text.chars().map(|c| face.advance(face.glyph_for(c)) as f32 * k).sum()
}

/// A section break mark between `x0` and `x1`: a dotted double rule with the break's name centred
/// in it (just the name when there is no room for the rule).
fn section_break_mark(start: SectionStart, x0: f32, x1: f32, base: f32, size: f32, alpha: f32, out: &mut Vec<Draw>) {
    let text = section_break_label(start);
    let lsize = (size * 0.8).clamp(6.0, 11.0);
    let w = mark_text_width(text, lsize);
    let gap = lsize * 0.4;
    let mid = base - size * 0.3;
    let color = Rgb(0x60, 0x60, 0x60);
    if x1 - x0 <= w + 4.0 * gap {
        out.push(Draw::MarkText { x: x0, baseline: base, size: lsize, text: text.into() });
        return;
    }
    let tx = (x0 + x1 - w) / 2.0;
    for (a, b) in [(x0, tx - gap), (tx + w + gap, x1)] {
        for y in [mid - 1.2, mid + 1.2] {
            out.push(Draw::Line { x0: a, y0: y, x1: b, y1: y, width: 0.5, color, stroke: Stroke::Dotted, alpha, text_bg: None });
        }
    }
    out.push(Draw::MarkText { x: tx, baseline: mid + lsize * 0.33, size: lsize, text: text.into() });
}

/// An equation's glyphs and rules with its origin at (`x`, `base`).
fn equation(items: &[MItem], x: f32, base: f32, alpha: f32, screen: bool, out: &mut Vec<Draw>) {
    for it in items {
        match it {
            MItem::Glyphs { face, size, color, synth_bold, synth_italic, glyphs, text } => out.push(Draw::Glyphs {
                face: *face,
                size: *size,
                glyphs: glyphs.iter().map(|(g, gx, gy)| (*g, x + gx, base - gy)).collect(),
                color: *color,
                alpha,
                synth_bold: *synth_bold,
                synth_italic: *synth_italic,
                text: text.clone(),
                link: None,
                // Equation text maps back through the font (no per-glyph ranges from the math layout).
                ranges: Vec::new(),
                text_bg: None,
                x_scale: 1.0,
            }),
            MItem::Rect { x: rx, y, w, h, color } => {
                out.push(Draw::Fill { rect: Rect::new(x + rx, base - y - h, *w, *h), color: *color, alpha, text_bg: false })
            }
            MItem::Line { x0, y0, x1, y1, width, color, dotted } => out.push(Draw::Line {
                x0: x + x0,
                y0: base - y0,
                x1: x + x1,
                y1: base - y1,
                width: *width,
                color: *color,
                stroke: if *dotted { Stroke::Dotted } else { Stroke::Solid },
                alpha,
                text_bg: None,
            }),
            MItem::ScreenOnly(inner) => {
                if screen {
                    equation(std::slice::from_ref(inner), x, base, alpha, screen, out);
                }
            }
        }
    }
}

/// The draws of a chart or diagram's items inside `rect` (page coordinates). The items were built
/// for the size `g.w` × `g.h` (0: `rect`'s own size), so they are scaled to `rect`; strokes and text
/// by the mean of the two scales.
fn graphic_draws(doc: &Document, g: &Graphic, rect: Rect, alpha: f32) -> Vec<Draw> {
    let (sx, sy) = (scale_of(rect.w, g.w), scale_of(rect.h, g.h));
    let k = (sx + sy) / 2.0;
    let inside = |r: &[f32; 4]| Rect::new(rect.x + r[0] * sx, rect.y + r[1] * sy, r[2] * sx, r[3] * sy);
    let mut out = Vec::with_capacity(g.items.len());
    for it in &g.items {
        match it {
            GraphicItem::Shape { rect: r, kind, fill, stroke, stroke_width } => out.push(Draw::Shape {
                rect: inside(r),
                kind: *kind,
                fill: fill.map(Paint::solid),
                stroke: *stroke,
                stroke_width: stroke_width * k,
                effects: Default::default(),
                corner: None,
                dash: Vec::new(),
                cap: Default::default(),
            }),
            GraphicItem::Path { segs, fill, stroke, stroke_width } => {
                let segs = page_segs(segs, |x, y| (rect.x + x * sx, rect.y + y * sy));
                out.push(Draw::Path {
                    segs,
                    fill: fill.map(Paint::solid),
                    stroke: *stroke,
                    stroke_width: stroke_width * k,
                    dash: Vec::new(),
                    cap: Default::default(),
                })
            }
            GraphicItem::Text { rect: r, .. } => out.extend(text_glyphs(doc, inside(r), it, alpha, k)),
            GraphicItem::Image { rect: r, media } => {
                out.push(Draw::Image { rect: inside(r), media: media.clone(), crop: [0.0; 4], alpha, alt: String::new(), decorative: false })
            }
        }
    }
    out
}

/// Scale from the size `built` points a graphic's items were made for to `len` points; 1 when
/// the built size is unknown (0).
fn scale_of(len: f32, built: f32) -> f32 {
    if built.is_finite() && built > 0.0 && len.is_finite() { (len / built).max(0.0) } else { 1.0 }
}

/// Path segments mapped to page coordinates by `map`, cleaned for the rasterizer and PDF: a segment
/// with a non-finite number is dropped, and so is anything before a move (or after a broken one).
fn page_segs(segs: &[PathSeg], map: impl Fn(f32, f32) -> (f32, f32)) -> Vec<PathSeg> {
    let fin = |p: (f32, f32)| p.0.is_finite() && p.1.is_finite();
    let mut out = Vec::with_capacity(segs.len());
    let mut open = false;
    for s in segs {
        match *s {
            PathSeg::Move(x, y) => {
                let p = map(x, y);
                open = fin(p);
                if open {
                    out.push(PathSeg::Move(p.0, p.1));
                }
            }
            PathSeg::Line(x, y) => {
                let p = map(x, y);
                if open && fin(p) {
                    out.push(PathSeg::Line(p.0, p.1));
                }
            }
            PathSeg::Cubic(a, b, c, d, e, f) => {
                let (p1, p2, p3) = (map(a, b), map(c, d), map(e, f));
                if open && fin(p1) && fin(p2) && fin(p3) {
                    out.push(PathSeg::Cubic(p1.0, p1.1, p2.0, p2.1, p3.0, p3.1));
                }
            }
            PathSeg::Close => {
                if open {
                    out.push(PathSeg::Close);
                }
                open = false;
            }
        }
    }
    out
}

/// A path of clean segments (see [`page_segs`]) as a kurbo path, for the rasteriser and PDF.
/// The draws of a shape at `rect`: a preset outline, or a freeform's paths (an ink stroke left out
/// under Review › Hide Ink).
#[allow(clippy::too_many_arguments)]
fn shape_draws(
    rect: Rect,
    kind: ShapeKind,
    fill: Option<&wordcraft_doc::fill::Fill>,
    stroke: Option<Rgb>,
    stroke_width: f32,
    effects: wordcraft_doc::effects::ShapeEffects,
    freeform: Option<&wordcraft_doc::freeform::Freeform>,
    extra: &wordcraft_doc::connector::ShapeExtra,
    opts: &DisplayOptions,
    out: &mut Vec<Draw>,
) {
    // Freeforms are painted across the shape's frame, not each path's bounds.
    let fill = fill.and_then(|f| paint_for(f, rect));
    if kind.is_open() {
        line_draws(rect, kind, stroke, stroke_width, effects, extra, out);
        return;
    }
    let Some(f) = freeform.filter(|_| kind == ShapeKind::Freeform) else {
        out.push(Draw::Shape {
            rect,
            kind,
            fill,
            stroke,
            stroke_width,
            effects,
            corner: extra.corner,
            dash: extra.dash.pattern(stroke_width),
            cap: extra.cap,
        });
        return;
    };
    if f.is_ink() && opts.hide_ink {
        return;
    }
    let width = if stroke_width.is_finite() { stroke_width.clamp(0.0, 200.0) } else { 0.75 };
    for (pts, closed) in f.placed(rect.x, rect.y, rect.w, rect.h) {
        if f.is_ink() || (!closed && fill.is_none()) {
            if let Some(color) = stroke {
                let mut pts: Vec<(f32, f32)> = pts.iter().map(|[x, y]| (*x, *y)).collect();
                if closed && let Some(first) = pts.first().copied() {
                    pts.push(first);
                }
                if !closed
                    && extra.arrow_end
                    && let Some(arrow) = path_arrowhead(&pts, color, width)
                {
                    out.push(arrow);
                }
                out.push(Draw::Ink { pts, color, width: width.max(0.25), alpha: f.alpha });
            }
            continue;
        }
        let mut segs: Vec<PathSeg> = Vec::with_capacity(pts.len() + 1);
        for (i, [x, y]) in pts.iter().enumerate() {
            segs.push(if i == 0 { PathSeg::Move(*x, *y) } else { PathSeg::Line(*x, *y) });
        }
        if closed {
            segs.push(PathSeg::Close);
        }
        let width = path_stroke_width(width);
        out.push(Draw::Path {
            segs,
            fill: if closed { fill.clone() } else { None },
            stroke,
            stroke_width: width,
            dash: extra.dash.pattern(width),
            cap: extra.cap,
        });
        if !closed
            && extra.arrow_end
            && let Some(color) = stroke
        {
            let points: Vec<(f32, f32)> = pts.iter().map(|[x, y]| (*x, *y)).collect();
            if let Some(arrow) = path_arrowhead(&points, color, width) {
                out.push(arrow);
            }
        }
    }
}

// Dash lengths use the effective width used by both path painters.
fn path_stroke_width(width: f32) -> f32 {
    if width.is_finite() && width > 0.0 { width.clamp(0.25, 200.0) } else { 0.75 }
}

/// Make a filled triangular arrowhead at the last point, in the points' coordinate space.
/// The last distinct segment supplies its direction and the bounded stroke width sets its size.
/// Returns `None` if there is no usable final segment. The triangle has no outline, so the path's
/// cap setting does not affect its appearance.
fn path_arrowhead(points: &[(f32, f32)], color: Rgb, stroke_width: f32) -> Option<Draw> {
    let tip = *points.last()?;
    let (prev_x, prev_y) = points.iter().rev().skip(1).copied().find(|p| *p != tip)?;
    let dx = tip.0 - prev_x;
    let dy = tip.1 - prev_y;
    let length = dx.hypot(dy);
    if !length.is_finite() || length <= f32::EPSILON {
        return None;
    }
    let (dx, dy) = (dx / length, dy / length);
    let width = if stroke_width.is_finite() { stroke_width.clamp(0.25, 200.0) } else { 0.75 };
    let len = (width * 3.5).max(7.0);
    let (bx, by) = (tip.0 - dx * len, tip.1 - dy * len);
    let (nx, ny) = (-dy * len * 0.45, dx * len * 0.45);
    let segs = vec![PathSeg::Move(tip.0, tip.1), PathSeg::Line(bx + nx, by + ny), PathSeg::Line(bx - nx, by - ny), PathSeg::Close];
    Some(Draw::Path { segs, fill: Some(Paint::solid(color)), stroke: None, stroke_width: 0.0, dash: Vec::new(), cap: Default::default() })
}

/// A line or connector at `rect` (its unflipped frame: flips are the frame's turn), with its
/// arrowheads.
fn line_draws(
    rect: Rect,
    kind: ShapeKind,
    stroke: Option<Rgb>,
    stroke_width: f32,
    effects: wordcraft_doc::effects::ShapeEffects,
    extra: &wordcraft_doc::connector::ShapeExtra,
    out: &mut Vec<Draw>,
) {
    use wordcraft_doc::connector::{connector_segs, end_direction};
    match connector_segs(kind, rect.w, rect.h) {
        Some(segs) => {
            let segs = segs
                .into_iter()
                .map(|s| match s {
                    PathSeg::Move(x, y) => PathSeg::Move(rect.x + x, rect.y + y),
                    PathSeg::Line(x, y) => PathSeg::Line(rect.x + x, rect.y + y),
                    PathSeg::Cubic(a, b, c, d, e, f) => PathSeg::Cubic(rect.x + a, rect.y + b, rect.x + c, rect.y + d, rect.x + e, rect.y + f),
                    PathSeg::Close => PathSeg::Close,
                })
                .collect();
            let width = path_stroke_width(stroke_width);
            out.push(Draw::Path { segs, fill: None, stroke, stroke_width: width, dash: extra.dash.pattern(width), cap: extra.cap });
        }
        None => out.push(Draw::Shape {
            rect,
            kind,
            fill: None,
            stroke,
            stroke_width,
            effects,
            corner: None,
            dash: extra.dash.pattern(stroke_width),
            cap: extra.cap,
        }),
    }
    let Some(color) = stroke else { return };
    // Our line runs from the bottom-left corner to the top-right; connectors from the top-left
    // to the bottom-right.
    let (start, end) =
        if kind == ShapeKind::Line { ((rect.x, rect.bottom()), (rect.right(), rect.y)) } else { ((rect.x, rect.y), (rect.right(), rect.bottom())) };
    let width = if stroke_width.is_finite() { stroke_width.clamp(0.25, 200.0) } else { 0.75 };
    let len = (width * 3.5).max(7.0);
    for (on, tip, at_end) in [(extra.arrow_start, start, false), (extra.arrow_end, end, true)] {
        if !on {
            continue;
        }
        let (dx, dy) = end_direction(kind, rect.w, rect.h, at_end);
        let (bx, by) = (tip.0 - dx * len, tip.1 - dy * len);
        let (nx, ny) = (-dy * len * 0.45, dx * len * 0.45);
        let segs = vec![PathSeg::Move(tip.0, tip.1), PathSeg::Line(bx + nx, by + ny), PathSeg::Line(bx - nx, by - ny), PathSeg::Close];
        out.push(Draw::Path { segs, fill: Some(Paint::solid(color)), stroke: None, stroke_width: 0.0, dash: Vec::new(), cap: Default::default() });
    }
}

pub fn seg_path(segs: &[PathSeg]) -> BezPath {
    let mut p = BezPath::new();
    for s in segs {
        match *s {
            PathSeg::Move(x, y) => p.move_to((x as f64, y as f64)),
            PathSeg::Line(x, y) => p.line_to((x as f64, y as f64)),
            PathSeg::Cubic(a, b, c, d, e, f) => p.curve_to((a as f64, b as f64), (c as f64, d as f64), (e as f64, f as f64)),
            PathSeg::Close => p.close_path(),
        }
    }
    p
}

/// One line of a `GraphicItem::Text` in `r`, shaped in its font (the document's body font when
/// unset) and aligned horizontally, centred vertically, at its size times `k`. `None` when there is
/// nothing to draw, or the item's rectangle or size isn't a finite number (a degenerate scale:
/// `r` itself is always finite, as [`Rect::new`] makes it).
fn text_glyphs(doc: &Document, r: Rect, it: &GraphicItem, alpha: f32, k: f32) -> Option<Draw> {
    let GraphicItem::Text { rect, text, size, color, bold, align, font } = it else { return None };
    let size = *size * k;
    if !rect.iter().chain([&size]).all(|v| v.is_finite()) {
        return None;
    }
    let family = font.as_deref().filter(|f| !f.is_empty()).unwrap_or(&doc.settings.minor_font);
    let size = size.clamp(1.0, 400.0);
    let resolved = wordcraft_fonts::word::resolve(family, *bold, false);
    let face = resolved.face.get();
    let shaped = wordcraft_fonts::shape(face, text, &[], |c| c);
    if shaped.is_empty() {
        return None;
    }
    let k = size / face.upem.max(1.0) as f32;
    let width: f32 = shaped.iter().map(|g| g.x_advance as f32 * k).sum();
    let (ascent, descent) = wordcraft_fonts::word::line_metrics(face);
    let baseline = r.y + r.h / 2.0 + (ascent - descent) as f32 * k / 2.0;
    let mut pen = match align {
        TextAlign::Left => r.x,
        TextAlign::Center => r.x + (r.w - width) / 2.0,
        TextAlign::Right => r.x + r.w - width,
    };
    let mut glyphs = Vec::with_capacity(shaped.len());
    for g in &shaped {
        glyphs.push((g.gid, pen + g.x_offset as f32 * k, baseline - g.y_offset as f32 * k));
        pen += g.x_advance as f32 * k;
    }
    Some(Draw::Glyphs {
        face: resolved.face,
        size,
        glyphs,
        color: *color,
        alpha,
        synth_bold: resolved.synth_bold,
        synth_italic: false,
        text: text.clone(),
        link: None,
        ranges: Vec::new(),
        text_bg: None,
        x_scale: 1.0,
    })
}

/// Each author's place among `authors` in order of first appearance (which colour marks their
/// changes and comments), found in one pass.
pub(crate) fn author_order<'a>(authors: impl Iterator<Item = &'a str>) -> HashMap<&'a str, u32> {
    let mut seen = HashMap::new();
    for a in authors {
        crate::work::steps(1);
        let n = u32::try_from(seen.len()).unwrap_or(u32::MAX);
        seen.entry(a).or_insert(n);
    }
    seen
}

/// The colours of the document's revision authors ([`author_order`]), worked out once for a
/// drawing, when its first tracked change is drawn, not once for every change.
struct Authors<'a> {
    doc: &'a Document,
    order: std::cell::OnceCell<HashMap<&'a str, u32>>,
}

impl<'a> Authors<'a> {
    fn new(doc: &'a Document) -> Self {
        Authors { doc, order: std::cell::OnceCell::new() }
    }

    /// The colour of revision `r`'s author.
    fn color(&self, r: u32) -> Rgb {
        let order = self.order.get_or_init(|| author_order(self.doc.revisions.iter().map(|v| v.author.as_str())));
        revision_color(self.doc.revisions.get(r as usize).and_then(|v| order.get(v.author.as_str()).copied()).unwrap_or(0))
    }
}

/// How far a legacy check box's square sits inside the room it takes on its line, on every side.
pub const CHECK_BOX_INSET: f32 = 1.16;

/// A legacy form check box (`FORMCHECKBOX`) in `frame`, the room it takes on its line (as wide
/// as the text's line is tall): a square inset [`CHECK_BOX_INSET`] with a 0.72 pt border, and,
/// checked, an X from corner to corner in 0.48 pt lines, in the text's colour. Measured on Word's
/// PDF export: a 9.96 pt square for 10 pt Calibri, 22.08 pt for 20 pt.
pub fn check_box(frame: Rect, checked: bool, color: Rgb, alpha: f32, out: &mut Vec<Draw>) {
    let i = CHECK_BOX_INSET.min(frame.w / 4.0).min(frame.h / 4.0).max(0.0);
    let sq = Rect::new(frame.x + i, frame.y + i, (frame.w - 2.0 * i).max(0.5), (frame.h - 2.0 * i).max(0.5));
    let line =
        |x0: f32, y0: f32, x1: f32, y1: f32, width: f32| Draw::Line { x0, y0, x1, y1, width, color, stroke: Stroke::Solid, alpha, text_bg: None };
    let (l, t, r, b) = (sq.x, sq.y, sq.x + sq.w, sq.y + sq.h);
    out.push(Draw::Shape {
        rect: sq,
        kind: ShapeKind::Rectangle,
        fill: None,
        stroke: Some(color),
        stroke_width: 0.72,
        effects: Default::default(),
        corner: None,
        dash: Vec::new(),
        cap: Default::default(),
    });
    if checked {
        out.extend([line(l, t, r, b, 0.48), line(r, t, l, b, 0.48)]);
    }
}

/// Automatic colour is black, or white on a dark background.
pub fn text_color(c: &TextColor, background: Option<Rgb>) -> Rgb {
    match c {
        TextColor::Rgb(r) => *r,
        TextColor::Auto => {
            if background.is_some_and(|b| b.luma() < 100.0) {
                Rgb::WHITE
            } else {
                Rgb::BLACK
            }
        }
    }
}

/// `items` turned by `spin` about the centre of `rect` (as they are, unturned).
fn spun(spin: Spin, rect: Rect, items: Vec<Draw>, out: &mut Vec<Draw>) {
    if spin.is_identity() {
        out.extend(items);
    } else {
        out.push(Draw::Rotated { cx: rect.x + rect.w / 2.0, cy: rect.y + rect.h / 2.0, spin, items });
    }
}

/// A picture, shape or group drawn in `rect` (its unturned frame), turned by its own spin about
/// the rect's centre; a group's members are turned inside it. `outer` is the spin it is already
/// drawn inside (its group's). Nothing for anything else.
fn object_draws(o: &InlineObject, rect: Rect, outer: Spin, alpha: f32, opts: &DisplayOptions, out: &mut Vec<Draw>) {
    let own = o.frame().map(|(_, _, f)| f.spin()).unwrap_or_default();
    match o {
        InlineObject::Image { media, crop, alt, .. } => {
            let decorative = o.is_decorative();
            spun(own, rect, vec![Draw::Image { rect, media: media.clone(), crop: *crop, alpha, alt: alt.clone(), decorative }], out)
        }
        InlineObject::Shape { kind, fill, stroke, stroke_width, effects, freeform, extra, .. } => {
            let effects = effects_in(*effects, own.within(outer));
            let mut items = Vec::new();
            shape_draws(rect, *kind, fill.as_ref(), *stroke, *stroke_width, effects, freeform.as_deref(), extra, opts, &mut items);
            match crate::shape_figure_alt(o) {
                // A described shape is one figure with its description (tagged PDF).
                Some(alt) => {
                    let mut draws = Vec::new();
                    spun(own, rect, items, &mut draws);
                    out.push(Draw::Figure { alt: alt.to_string(), kind: None, draws })
                }
                None => spun(own, rect, items, out),
            }
        }
        InlineObject::Group { .. } => {
            let mut members = Vec::new();
            for ([x, y, w, h], c) in o.group_rects(rect.x, rect.y, rect.w, rect.h) {
                if c.frame().is_some_and(|(_, _, f)| f.hidden) {
                    continue;
                }
                object_draws(c, Rect::new(x, y, w, h), own.within(outer), alpha, opts, &mut members);
            }
            match o.canvas_style() {
                // A Drawing Canvas: its background and outline, then its members clipped to its
                // frame, all inside its turn.
                Some(c) => spun(own, rect, vec![canvas_background(rect, c), Draw::Clip { rect, items: members }], out),
                None => spun(own, rect, members, out),
            }
        }
        // Native ink in a group (kept opaque): drawn as drawing-level ink is, from its pictures.
        InlineObject::Graphic { graphic, .. } if graphic.kind == GraphicKind::Ink => {
            let (sx, sy) = (scale_of(rect.w, graphic.w), scale_of(rect.h, graphic.h));
            let items = graphic
                .items
                .iter()
                .filter_map(|it| match it {
                    GraphicItem::Image { rect: r, media } => Some(Draw::Image {
                        rect: Rect::new(rect.x + r[0] * sx, rect.y + r[1] * sy, r[2] * sx, r[3] * sy),
                        media: media.clone(),
                        crop: [0.0; 4],
                        alpha,
                        alt: String::new(),
                        decorative: false,
                    }),
                    _ => None,
                })
                .collect();
            spun(own, rect, items, out)
        }
        _ => {}
    }
}

/// A shape's `effects` as drawn turned by `spin` (all the turns it is drawn inside): glows and
/// soft edges turn with the shape, and so does a shadow's offset when it rotates with the shape
/// (`rotWithShape`); one that doesn't keeps its direction on the page, so its angle is turned back.
pub(crate) fn effects_in(effects: wordcraft_doc::effects::ShapeEffects, spin: Spin) -> wordcraft_doc::effects::ShapeEffects {
    let mut e = effects;
    if let Some(s) = e.shadow.as_mut().filter(|s| !s.rot_with_shape && !spin.is_identity()) {
        let a = s.angle.to_radians();
        let (dx, dy) = spin.unapply(0.0, 0.0, a.cos(), a.sin());
        s.angle = wordcraft_geom::normalize_degrees(dy.atan2(dx).to_degrees());
    }
    e
}

/// Where inline object `obj` is drawn, given its cluster's box (`adv` × `obj_h` standing on the
/// baseline at `cx`): inside the room kept for its effects.
/// A rotated one's unrotated frame, centred in that room.
pub(crate) fn inline_rect(obj: Option<&InlineObject>, cx: f32, base: f32, adv: f32, obj_h: f32) -> Rect {
    let frame = obj.and_then(InlineObject::frame);
    let [l, t, r, b] = frame.map_or([0.0; 4], |(_, _, float)| float.effect_extent());
    let room = Rect::new(cx + l, base - obj_h + t, (adv - l - r).max(0.0), (obj_h - t - b).max(0.0));
    let Some((w, h, float)) = frame.filter(|(_, _, f)| f.rot != 0.0) else { return room };
    // The room is the rotated bounds of the frame: undo the rotation.
    let (w, h) = (wordcraft_geom::finite(w).clamp(1.0, 4000.0), wordcraft_geom::finite(h).clamp(1.0, 4000.0));
    let (bw, _) = float.spin().extent(w, h);
    let k = if bw > 0.0 { room.w / bw } else { 1.0 };
    let (fw, fh) = (w * k, h * k);
    Rect::new(room.x + (room.w - fw) / 2.0, room.y + (room.h - fh) / 2.0, fw, fh)
}

/// A Drawing Canvas's background and outline over its frame `rect`.
fn canvas_background(rect: Rect, c: &wordcraft_doc::para::CanvasStyle) -> Draw {
    Draw::Shape {
        rect,
        kind: ShapeKind::Rectangle,
        fill: c.fill.as_ref().and_then(|f| paint_for(f, rect)),
        stroke: c.stroke,
        stroke_width: c.stroke_width,
        effects: Default::default(),
        corner: None,
        dash: Vec::new(),
        cap: Default::default(),
    }
}

/// Whether `c` stands upright in top-to-bottom East Asian text (`tbRl`) rather than turning with
/// the line: ideographs, kana, Hangul, full-width letters and digits, and the ideographic comma
/// and full stop (an approximation of Unicode's vertical orientation, UAX #50). Brackets, dashes
/// and the prolonged sound mark turn with the line, as their vertical forms do.
pub fn upright_in_vertical(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{11FF}'
        | '\u{2E80}'..='\u{2FFF}'
        | '\u{3001}'..='\u{3007}'
        | '\u{3012}'..='\u{3013}'
        | '\u{3020}'..='\u{302F}'
        | '\u{3031}'..='\u{303F}'
        | '\u{3040}'..='\u{30FB}'
        | '\u{30FD}'..='\u{30FF}'
        | '\u{3100}'..='\u{31EF}'
        | '\u{31F0}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{A960}'..='\u{A97F}'
        | '\u{AC00}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF01}'..='\u{FF07}'
        | '\u{FF0A}'..='\u{FF0C}'
        | '\u{FF0E}'..='\u{FF3A}'
        | '\u{FF3C}'
        | '\u{FF3E}'..='\u{FF5A}'
        | '\u{FFE0}'..='\u{FFE6}'
        | '\u{20000}'..='\u{3FFFF}'
    )
}

/// Top-to-bottom text drawn turned 90° clockwise, with its East Asian characters turned back
/// upright, each about the centre of its em square (`tbRl`: ideographs stand, Latin lies).
/// Glyph runs are split where uprightness changes, so the text keeps its order.
fn upright_east_asian(items: Vec<Draw>) -> Vec<Draw> {
    let mut out = Vec::with_capacity(items.len());
    for d in items {
        let Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges, text_bg, x_scale } = d else {
            out.push(d);
            continue;
        };
        let up = |k: usize| ranges.get(k).and_then(|r| text.get(r.clone())).and_then(|t| t.chars().next()).is_some_and(upright_in_vertical);
        if ranges.len() != glyphs.len() || !(0..glyphs.len()).any(up) {
            out.push(Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges, text_bg, x_scale });
            continue;
        }
        let piece = |k0: usize, k1: usize| -> Draw {
            let gs = glyphs.get(k0..k1).unwrap_or(&[]).to_vec();
            let rs = ranges.get(k0..k1).unwrap_or(&[]);
            let (b0, b1) = (rs.iter().map(|r| r.start).min().unwrap_or(0), rs.iter().map(|r| r.end).max().unwrap_or(0));
            let t = text.get(b0..b1).unwrap_or("").to_string();
            let rs = rs.iter().map(|r| r.start.saturating_sub(b0)..r.end.saturating_sub(b0)).collect();
            Draw::Glyphs { face, size, glyphs: gs, color, alpha, synth_bold, synth_italic, text: t, link: link.clone(), ranges: rs, text_bg, x_scale }
        };
        let mut k0 = 0;
        while k0 < glyphs.len() {
            let u = up(k0);
            let k1 = (k0 + 1..glyphs.len()).find(|&k| up(k) != u).unwrap_or(glyphs.len());
            if !u {
                out.push(piece(k0, k1));
            } else {
                for k in k0..k1 {
                    let Some(&(_, gx, base)) = glyphs.get(k) else { continue };
                    // The em square: one em along the line (the advance to the next glyph when
                    // known), from about 0.88 em above the baseline to 0.12 em below it.
                    let adv = glyphs.get(k + 1).map(|g| g.1 - gx).filter(|a| *a > 0.0 && *a < size * 2.0).unwrap_or(size);
                    let (cx, cy) = (gx + adv / 2.0, base - size * 0.38);
                    let mut g = piece(k, k + 1);
                    // The ideographic comma and full stop sit in the upper right of their square in
                    // vertical text (lower left in horizontal): moved there before standing up.
                    if let Draw::Glyphs { text, glyphs, .. } = &mut g
                        && matches!(text.chars().next(), Some('\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}'))
                    {
                        for (_, x, y) in glyphs.iter_mut() {
                            *x += size * 0.55;
                            *y -= size * 0.55;
                        }
                    }
                    out.push(Draw::Rotated { cx, cy, spin: Spin::new(-90.0, false, false), items: vec![g] });
                }
            }
            k0 = k1;
        }
    }
    out
}
