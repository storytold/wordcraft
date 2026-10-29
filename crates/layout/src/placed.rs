use super::{Arc, Border, ParaLayout, Path, Rect, Rgb, Spin, StoryRef, TextDirection, Wrap};

/// Something placed on a page (page coordinates, points, y down).
#[derive(Clone, Debug)]
pub enum Placed {
    /// Lines `l0..l1` of a paragraph; `y` is the top of line `l0`; `x` the column's left edge.
    /// Turned lines (a table cell's text direction) run as `turn` says from the page point
    /// (`x`, `y`): see [`turn_point`].
    Lines {
        story: StoryRef,
        path: Path,
        para: Arc<ParaLayout>,
        l0: usize,
        l1: usize,
        x: f32,
        y: f32,
        turn: TextDirection,
    },
    Fill {
        rect: Rect,
        color: Rgb,
    },
    Rule {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        border: Border,
    },
    /// A picture in `rect`, turned about the rect's centre by `spin` (as are shapes and charts).
    Image {
        rect: Rect,
        media: String,
        crop: [f32; 4],
        story: StoryRef,
        path: Path,
        off: usize,
        spin: Spin,
    },
    Shape {
        rect: Rect,
        kind: wordcraft_doc::para::ShapeKind,
        fill: Option<wordcraft_doc::fill::Fill>,
        stroke: Option<Rgb>,
        stroke_width: f32,
        /// A freeform's paths (ink strokes among them).
        freeform: Option<Arc<wordcraft_doc::freeform::Freeform>>,
        effects: wordcraft_doc::effects::ShapeEffects,
        spin: Spin,
        /// Arrowheads and text warp (see [`wordcraft_doc::connector::ShapeExtra`]).
        extra: wordcraft_doc::connector::ShapeExtra,
        /// The floating object it draws (a Drawing Canvas's members: the canvas's), for leaving
        /// exactly one object out of a page (see [`crate::display::DisplayOptions::hide_object`]).
        object: Option<wordcraft_doc::Pos>,
        /// The description it is tagged with as a figure in tagged PDF (see
        /// [`shape_figure_alt`]); empty for a shape that is an artifact.
        alt: String,
    },
    /// A floating chart or diagram, drawn from its items inside `rect`. The object is the U+FFFC at
    /// byte `off` of paragraph `path` (its alt text).
    Graphic {
        rect: Rect,
        graphic: Arc<wordcraft_doc::graphic::Graphic>,
        story: StoryRef,
        path: Path,
        off: usize,
        spin: Spin,
    },
    /// `items` clipped to `rect`: a floating Drawing Canvas's members, kept inside its frame.
    /// `rect` is the unturned frame and `items` sit in it; `spin` turns the clipped whole about
    /// the frame's centre.
    Clip {
        rect: Rect,
        spin: Spin,
        items: Vec<Placed>,
    },
    /// Opens (`open`) or closes the items of a turned text box: its frame's shape and its
    /// text, laid out in the unturned frame `rect` and drawn turned by `spin` about the frame's
    /// centre, as Word does. While story `part` (the text it shows) is being edited, they are
    /// drawn straight instead (see [`display::DisplayOptions::straight`]). The items between
    /// stay in the page's list, so carets, selections and searches find the text as ever.
    Spun {
        rect: Rect,
        spin: Spin,
        part: u32,
        open: bool,
    },
    /// A table cell's area (for hit testing and cell selection). A vertically merged cell is
    /// one `Cell` as tall as all its rows; its continuation rows get a `covered` `Cell` (their
    /// own row's height) so the rows keep a box each, but it isn't a cell of its own on screen.
    Cell {
        rect: Rect,
        covered: bool,
        table: Path,
        row: usize,
        cell: usize,
        story: StoryRef,
        /// Its row is exactly as tall as it says and the content is taller: what runs past the
        /// cell is cut off when drawn, as in Word.
        clip: bool,
    },
    /// The area of a picture, shape or text box, for hit testing, selection handles and dragging
    /// (drawn by `Image`/`Shape`/`Lines`). The object is the U+FFFC at byte `off` of paragraph
    /// `path` in `story`. `rect` is its unrotated frame; `spin` turns it about the centre.
    Object {
        rect: Rect,
        spin: Spin,
        story: StoryRef,
        path: Path,
        off: usize,
        /// The text box story it shows (`Document::parts` id).
        text_box: Option<u32>,
        wrap: Wrap,
        /// Its paragraph's column left and top: where offsets relative to the column and the
        /// paragraph (`Anchor::Column` / `Anchor::Paragraph`) start.
        origin: wordcraft_geom::Point,
    },
}

impl Placed {
    pub(crate) fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Placed::Lines { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Placed::Fill { rect, .. }
            | Placed::Image { rect, .. }
            | Placed::Shape { rect, .. }
            | Placed::Graphic { rect, .. }
            | Placed::Spun { rect, .. }
            | Placed::Cell { rect, .. } => {
                rect.x += dx;
                rect.y += dy;
            }
            Placed::Object { rect, origin, .. } => {
                rect.x += dx;
                rect.y += dy;
                origin.x += dx;
                origin.y += dy;
            }
            Placed::Clip { rect, items, .. } => {
                rect.x += dx;
                rect.y += dy;
                for it in items {
                    it.translate(dx, dy);
                }
            }
            Placed::Rule { x0, y0, x1, y1, .. } => {
                *x0 += dx;
                *x1 += dx;
                *y0 += dy;
                *y1 += dy;
            }
        }
    }

    /// The page area of turned lines (`None` for anything else).
    pub fn turned_bounds(&self) -> Option<Rect> {
        let Placed::Lines { para, l0, l1, x, y, turn, .. } = self else { return None };
        if !turn.is_turned() {
            return None;
        }
        let first = para.lines.get(*l0)?;
        let last = para.lines.get(l1.checked_sub(1)?)?;
        let len = para.lines.get(*l0..*l1).unwrap_or(&[]).iter().map(|l| l.right).fold(0.0f32, f32::max);
        Some(turn_rect(*turn, *x, *y, Rect::new(0.0, 0.0, len, last.top + last.height - first.top)))
    }

    /// Move an item laid out in a turned frame (a table cell's text running `turn`) onto the
    /// page, the frame's origin landing on page point (`x`, `y`).
    pub(crate) fn turn(&mut self, turn: TextDirection, x: f32, y: f32) {
        if !turn.is_turned() {
            self.translate(x, y);
            return;
        }
        match self {
            Placed::Lines { x: lx, y: ly, turn: t, .. } => {
                (*lx, *ly) = turn_point(turn, x, y, *lx, *ly);
                // Turned text inside turned text keeps its own direction (no 180° text).
                if !t.is_turned() {
                    *t = turn;
                }
            }
            Placed::Fill { rect, .. }
            | Placed::Image { rect, .. }
            | Placed::Shape { rect, .. }
            | Placed::Graphic { rect, .. }
            | Placed::Spun { rect, .. }
            | Placed::Cell { rect, .. } => {
                *rect = turn_rect(turn, x, y, *rect);
            }
            Placed::Object { rect, origin, .. } => {
                *rect = turn_rect(turn, x, y, *rect);
                (origin.x, origin.y) = turn_point(turn, x, y, origin.x, origin.y);
            }
            Placed::Clip { rect, items, .. } => {
                *rect = turn_rect(turn, x, y, *rect);
                for it in items {
                    it.turn(turn, x, y);
                }
            }
            Placed::Rule { x0, y0, x1, y1, .. } => {
                (*x0, *y0) = turn_point(turn, x, y, *x0, *y0);
                (*x1, *y1) = turn_point(turn, x, y, *x1, *y1);
            }
        }
    }
}

/// Where point (`u`, `v`) of a frame turned `turn` lands on the page, the frame's origin being
/// page point (`x`, `y`): `u` runs along the lines, `v` across them (down for horizontal text).
pub fn turn_point(turn: TextDirection, x: f32, y: f32, u: f32, v: f32) -> (f32, f32) {
    match turn {
        TextDirection::Horizontal => (x + u, y + v),
        TextDirection::Down => (x - v, y + u),
        TextDirection::Up => (x + v, y - u),
    }
}

/// The inverse of [`turn_point`]: the frame point at page point (`px`, `py`).
pub fn unturn_point(turn: TextDirection, x: f32, y: f32, px: f32, py: f32) -> (f32, f32) {
    match turn {
        TextDirection::Horizontal => (px - x, py - y),
        TextDirection::Down => (py - y, x - px),
        TextDirection::Up => (y - py, px - x),
    }
}

/// The page rectangle a frame rectangle covers (see [`turn_point`]).
pub fn turn_rect(turn: TextDirection, x: f32, y: f32, r: Rect) -> Rect {
    match turn {
        TextDirection::Horizontal => Rect::new(x + r.x, y + r.y, r.w, r.h),
        TextDirection::Down => Rect::new(x - r.y - r.h, y + r.x, r.h, r.w),
        TextDirection::Up => Rect::new(x + r.y, y - r.x - r.w, r.h, r.w),
    }
}

/// Which header/footer of its section a page shows ([`Page::hf_slot`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HfSlot {
    #[default]
    Default,
    /// The first page of a section with "Different First Page".
    First,
    /// An even page with "Different Odd & Even Pages".
    Even,
}
