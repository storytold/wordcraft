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

mod glyphs;
mod lines;
mod objects;
mod options;
mod page;
mod paint;

pub(crate) use glyphs::author_order;
pub use glyphs::{CHECK_BOX_INSET, check_box, text_color, upright_in_vertical};
pub use lines::{mark_text_width, section_break_label};
pub(crate) use objects::inline_rect;
pub use objects::seg_path;
pub use options::{BalloonMode, ChangeBar, CommentDisplay, DeleteMark, DisplayOptions, FormattingMarks, InsertMark, MarkupOptions};
pub use page::{CHANGE_BAR, ItemDisplay, MOVE_COLOR, object_display, page_display, page_layers, revision_color};
pub use paint::{Paint, Stroke, paint_for};

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
    /// A shape's picture fill: picture `media` stretched over `rect` and clipped to the `kind`
    /// outline (`corner`: a rounded rectangle's), or to a freeform's closed paths `clip` when
    /// there are some. The shape's shadow and glow (`effects`, cast by the filled shape, grown
    /// by half the outline's `stroke_width` when it has one) are drawn first, under the
    /// picture. It comes just before the shape's [`Draw::Shape`] or paths, which draw the
    /// outline over it.
    ShapePicture {
        rect: Rect,
        kind: ShapeKind,
        corner: Option<u32>,
        media: String,
        clip: Vec<PathSeg>,
        effects: wordcraft_doc::effects::ShapeEffects,
        stroke_width: Option<f32>,
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
