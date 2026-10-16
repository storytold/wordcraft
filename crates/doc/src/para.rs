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

/// Which sides of a Square, Tight or Through object text may flow on (`@wrapText`, ECMA-376
/// Part 1, §20.4.3.7 `ST_WrapText`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WrapText {
    /// Both sides of the object.
    #[default]
    BothSides,
    /// Only the left side: the band to the object's right stays empty.
    Left,
    /// Only the right side.
    Right,
    /// Only whichever side has more room, decided per line.
    Largest,
}

impl WrapText {
    /// From the OOXML value (`bothSides`, `left`, `right`, `largest`).
    pub fn from_ooxml(v: &str) -> Option<WrapText> {
        match v {
            "bothSides" => Some(WrapText::BothSides),
            "left" => Some(WrapText::Left),
            "right" => Some(WrapText::Right),
            "largest" => Some(WrapText::Largest),
            _ => None,
        }
    }

    /// The OOXML value.
    pub fn ooxml(self) -> &'static str {
        match self {
            WrapText::BothSides => "bothSides",
            WrapText::Left => "left",
            WrapText::Right => "right",
            WrapText::Largest => "largest",
        }
    }

    pub fn is_both(&self) -> bool {
        *self == WrapText::BothSides
    }
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Float {
    pub wrap: Wrap,
    /// The sides text wraps on (Square, Tight and Through).
    #[serde(skip_serializing_if = "WrapText::is_both")]
    pub wrap_text: WrapText,
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
    /// The outline Tight and Through wrapping follow (`wp:wrapPolygon`), in the object's own
    /// (unrotated) frame; `None` = derived from the object (see [`crate::wrap`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap_polygon: Option<std::sync::Arc<crate::wrap::WrapPolygon>>,
    /// What the drawing is, for screen readers and exports (`wp:docPr` `@descr`, `@title` and the
    /// decorative mark). See [`InlineObject::alt_text`].
    #[serde(skip_serializing_if = "AltText::is_empty")]
    pub alt: AltText,
    /// Hidden (`wp:docPr/@hidden`, VML `visibility:hidden`): kept and saved, but neither drawn nor
    /// laid out, as Word for Windows does (Word for Mac draws hidden VML shapes).
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
}

/// Longest alternative text kept (characters): hostile files and commands are capped.
pub const MAX_ALT_TEXT: usize = 10_000;

/// A drawing's alternative text (Picture/Shape Format › Accessibility › Alt Text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AltText {
    /// The description of a shape, text box or group. Pictures and charts keep theirs in their
    /// own `alt`; read it with [`InlineObject::alt_text`], which knows where it is.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// The short title (`wp:docPr/@title`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Decorative: it adds nothing to the content, so screen readers skip it and the
    /// accessibility checker doesn't ask for a description.
    #[serde(skip_serializing_if = "is_false")]
    pub decorative: bool,
}

impl AltText {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.title.is_empty() && !self.decorative
    }
}

/// `s` cut to [`MAX_ALT_TEXT`] characters.
pub fn cap_alt(s: &str) -> String {
    s.chars().take(MAX_ALT_TEXT).collect()
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
    #[serde(alias = "rect")]
    Rectangle,
    #[serde(alias = "roundRect")]
    RoundedRectangle,
    SingleCornerSnipped,
    TopCornersSnipped,
    DiagonalCornersSnipped,
    SnippedAndRounded,
    RoundSingleCorner,
    RoundSameSideCorner,
    RoundDiagonalCorner,

    // Basic shapes
    #[serde(alias = "oval")]
    Ellipse,
    #[serde(alias = "isoscelesTriangle")]
    Triangle,
    /// A right triangle (DrawingML's `rtTriangle`): the right angle at the bottom left.
    RightTriangle,
    Parallelogram,
    Trapezoid,
    Diamond,
    Pentagon,
    Hexagon,
    Heptagon,
    Octagon,
    Decagon,
    Dodecagon,
    Pie,
    Chord,
    Teardrop,
    Frame,
    HalfFrame,
    LShape,
    DiagonalStripe,
    Cross,
    Plaque,
    Cylinder,
    Cube,
    Bevel,
    Donut,
    NoSymbol,
    BlockArc,
    FoldedCorner,
    SmileyFace,
    Heart,
    LightningBolt,
    Sun,
    Moon,
    Cloud,
    Arc,
    DoubleBracket,
    DoubleBrace,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    /// A text box (rectangle with a text story).
    TextBox,

    // Lines
    Line,
    #[serde(alias = "rightArrow")]
    Arrow,
    DoubleArrow,
    /// Connectors: lines whose ends can be glued to other shapes (see [`crate::connector`]).
    StraightConnector,
    ElbowConnector,
    ElbowArrow,
    ElbowDoubleArrow,
    CurvedConnector,
    CurvedArrow,
    CurvedDoubleArrow,
    Curve,
    /// A shape drawn point by point, such as ink (its geometry is the shape's `freeform`).
    Freeform,
    Scribble,

    // Block arrows
    LeftArrow,
    UpArrow,
    DownArrow,
    LeftRightArrow,
    UpDownArrow,
    QuadArrow,
    LeftRightUpArrow,
    BentArrow,
    UTurnArrow,
    LeftUpArrow,
    BentUpArrow,
    StripedRightArrow,
    NotchedRightArrow,
    PentagonArrow,
    Chevron,
    RightArrowCallout,
    DownArrowCallout,
    LeftArrowCallout,
    UpArrowCallout,
    LeftRightArrowCallout,
    QuadArrowCallout,
    CircularArrow,

    // Equation shapes
    MathPlus,
    MathMinus,
    MathMultiply,
    MathDivide,
    MathEqual,
    MathNotEqual,

    // Flowchart
    FlowchartProcess,
    FlowchartAlternateProcess,
    FlowchartDecision,
    FlowchartData,
    FlowchartPredefinedProcess,
    FlowchartInternalStorage,
    FlowchartDocument,
    FlowchartMultidocument,
    FlowchartTerminator,
    FlowchartPreparation,
    FlowchartManualInput,
    FlowchartManualOperation,
    FlowchartConnector,
    FlowchartOffpageConnector,
    FlowchartCard,
    FlowchartPunchedTape,
    FlowchartSummingJunction,
    /// Flowchart OR preset: an ellipse with horizontal and vertical spokes.
    FlowchartOr,
    FlowchartCollate,
    FlowchartSort,
    FlowchartExtract,
    FlowchartMerge,
    FlowchartStoredData,
    FlowchartDelay,
    FlowchartSequentialData,
    FlowchartMagneticDisk,
    FlowchartDirectAccessStorage,
    FlowchartDisplay,

    // Stars and Banners
    Explosion1,
    Explosion2,
    Star4,
    #[serde(alias = "star5")]
    Star,
    Star6,
    Star7,
    Star8,
    Star10,
    Star12,
    Star16,
    Star24,
    Star32,
    UpRibbon,
    DownRibbon,
    CurvedUpRibbon,
    CurvedDownRibbon,
    VerticalScroll,
    HorizontalScroll,
    Wave,
    DoubleWave,

    // Callouts
    RectangularCallout,
    RoundedRectangularCallout,
    OvalCallout,
    CloudCallout,
    LineCallout1,
    LineCallout2,
    LineCallout3,
    LineCallout4,
    LineCallout1AccentBar,
    LineCallout2AccentBar,
    LineCallout3AccentBar,
    LineCallout4AccentBar,

    // Drawing Canvas
    DrawingCanvas,
}

impl ShapeKind {
    /// True if the shape is fundamentally a 1D stroke/line rather than a 2D filled polygon.
    pub fn is_line_like(&self) -> bool {
        matches!(
            self,
            ShapeKind::Line
                | ShapeKind::DoubleArrow
                | ShapeKind::StraightConnector
                | ShapeKind::ElbowConnector
                | ShapeKind::ElbowArrow
                | ShapeKind::ElbowDoubleArrow
                | ShapeKind::CurvedConnector
                | ShapeKind::CurvedArrow
                | ShapeKind::CurvedDoubleArrow
                | ShapeKind::Curve
                | ShapeKind::Freeform
                | ShapeKind::Scribble
                | ShapeKind::Arc
                | ShapeKind::LeftBracket
                | ShapeKind::RightBracket
                | ShapeKind::LeftBrace
                | ShapeKind::RightBrace
                | ShapeKind::DoubleBracket
                | ShapeKind::DoubleBrace
                | ShapeKind::LineCallout1
                | ShapeKind::LineCallout2
                | ShapeKind::LineCallout3
                | ShapeKind::LineCallout4
                | ShapeKind::LineCallout1AccentBar
                | ShapeKind::LineCallout2AccentBar
                | ShapeKind::LineCallout3AccentBar
                | ShapeKind::LineCallout4AccentBar
        )
    }

    /// English display name.
    pub fn label(&self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "Rectangle",
            ShapeKind::RoundedRectangle => "Rounded Rectangle",
            ShapeKind::SingleCornerSnipped => "Single Corner Snipped",
            ShapeKind::TopCornersSnipped => "Top Corners Snipped",
            ShapeKind::DiagonalCornersSnipped => "Diagonal Corners Snipped",
            ShapeKind::SnippedAndRounded => "Snipped and Single Corner Rounded",
            ShapeKind::RoundSingleCorner => "Round Single Corner",
            ShapeKind::RoundSameSideCorner => "Round Same Side Corner",
            ShapeKind::RoundDiagonalCorner => "Round Diagonal Corner",

            ShapeKind::Ellipse => "Oval",
            ShapeKind::Triangle => "Isosceles Triangle",
            ShapeKind::RightTriangle => "Right Triangle",
            ShapeKind::Parallelogram => "Parallelogram",
            ShapeKind::Trapezoid => "Trapezoid",
            ShapeKind::Diamond => "Diamond",
            ShapeKind::Pentagon => "Regular Pentagon",
            ShapeKind::Hexagon => "Hexagon",
            ShapeKind::Heptagon => "Heptagon",
            ShapeKind::Octagon => "Octagon",
            ShapeKind::Decagon => "Decagon",
            ShapeKind::Dodecagon => "Dodecagon",
            ShapeKind::Pie => "Pie",
            ShapeKind::Chord => "Chord",
            ShapeKind::Teardrop => "Teardrop",
            ShapeKind::Frame => "Frame",
            ShapeKind::HalfFrame => "Half Frame",
            ShapeKind::LShape => "L-Shape",
            ShapeKind::DiagonalStripe => "Diagonal Stripe",
            ShapeKind::Cross => "Cross",
            ShapeKind::Plaque => "Plaque",
            ShapeKind::Cylinder => "Can",
            ShapeKind::Cube => "Cube",
            ShapeKind::Bevel => "Bevel",
            ShapeKind::Donut => "Donut",
            ShapeKind::NoSymbol => "\"No\" Symbol",
            ShapeKind::BlockArc => "Block Arc",
            ShapeKind::FoldedCorner => "Folded Corner",
            ShapeKind::SmileyFace => "Smiley Face",
            ShapeKind::Heart => "Heart",
            ShapeKind::LightningBolt => "Lightning Bolt",
            ShapeKind::Sun => "Sun",
            ShapeKind::Moon => "Moon",
            ShapeKind::Cloud => "Cloud",
            ShapeKind::Arc => "Arc",
            ShapeKind::DoubleBracket => "Double Bracket",
            ShapeKind::DoubleBrace => "Double Brace",
            ShapeKind::LeftBracket => "Left Bracket",
            ShapeKind::RightBracket => "Right Bracket",
            ShapeKind::LeftBrace => "Left Brace",
            ShapeKind::RightBrace => "Right Brace",
            ShapeKind::TextBox => "Text Box",

            ShapeKind::Line => "Line",
            ShapeKind::Arrow => "Arrow",
            ShapeKind::DoubleArrow => "Double Arrow",
            ShapeKind::StraightConnector => "Straight Connector",
            ShapeKind::ElbowConnector => "Elbow Connector",
            ShapeKind::ElbowArrow => "Elbow Arrow",
            ShapeKind::ElbowDoubleArrow => "Elbow Double Arrow",
            ShapeKind::CurvedConnector => "Curved Connector",
            ShapeKind::CurvedArrow => "Curved Arrow",
            ShapeKind::CurvedDoubleArrow => "Curved Double Arrow",
            ShapeKind::Curve => "Curve",
            ShapeKind::Freeform => "Freeform: Shape",
            ShapeKind::Scribble => "Freeform: Scribble",

            ShapeKind::LeftArrow => "Left Arrow",
            ShapeKind::UpArrow => "Up Arrow",
            ShapeKind::DownArrow => "Down Arrow",
            ShapeKind::LeftRightArrow => "Left-Right Arrow",
            ShapeKind::UpDownArrow => "Up-Down Arrow",
            ShapeKind::QuadArrow => "Quad Arrow",
            ShapeKind::LeftRightUpArrow => "Left-Right-Up Arrow",
            ShapeKind::BentArrow => "Bent Arrow",
            ShapeKind::UTurnArrow => "U-Turn Arrow",
            ShapeKind::LeftUpArrow => "Left-Up Arrow",
            ShapeKind::BentUpArrow => "Bent-Up Arrow",
            ShapeKind::StripedRightArrow => "Striped Right Arrow",
            ShapeKind::NotchedRightArrow => "Notched Right Arrow",
            ShapeKind::PentagonArrow => "Pentagon",
            ShapeKind::Chevron => "Chevron",
            ShapeKind::RightArrowCallout => "Right Arrow Callout",
            ShapeKind::DownArrowCallout => "Down Arrow Callout",
            ShapeKind::LeftArrowCallout => "Left Arrow Callout",
            ShapeKind::UpArrowCallout => "Up Arrow Callout",
            ShapeKind::LeftRightArrowCallout => "Left-Right Arrow Callout",
            ShapeKind::QuadArrowCallout => "Quad Arrow Callout",
            ShapeKind::CircularArrow => "Circular Arrow",

            ShapeKind::MathPlus => "Equation: Plus",
            ShapeKind::MathMinus => "Equation: Minus",
            ShapeKind::MathMultiply => "Equation: Multiply",
            ShapeKind::MathDivide => "Equation: Divide",
            ShapeKind::MathEqual => "Equation: Equal",
            ShapeKind::MathNotEqual => "Equation: Not Equal",

            ShapeKind::FlowchartProcess => "Process",
            ShapeKind::FlowchartAlternateProcess => "Alternate Process",
            ShapeKind::FlowchartDecision => "Decision",
            ShapeKind::FlowchartData => "Data",
            ShapeKind::FlowchartPredefinedProcess => "Predefined Process",
            ShapeKind::FlowchartInternalStorage => "Internal Storage",
            ShapeKind::FlowchartDocument => "Document",
            ShapeKind::FlowchartMultidocument => "Multidocument",
            ShapeKind::FlowchartTerminator => "Terminator",
            ShapeKind::FlowchartPreparation => "Preparation",
            ShapeKind::FlowchartManualInput => "Manual Input",
            ShapeKind::FlowchartManualOperation => "Manual Operation",
            ShapeKind::FlowchartConnector => "Connector",
            ShapeKind::FlowchartOffpageConnector => "Off-page Connector",
            ShapeKind::FlowchartCard => "Card",
            ShapeKind::FlowchartPunchedTape => "Punched Tape",
            ShapeKind::FlowchartSummingJunction => "Summing Junction",
            ShapeKind::FlowchartOr => "Or",
            ShapeKind::FlowchartCollate => "Collate",
            ShapeKind::FlowchartSort => "Sort",
            ShapeKind::FlowchartExtract => "Extract",
            ShapeKind::FlowchartMerge => "Merge",
            ShapeKind::FlowchartStoredData => "Stored Data",
            ShapeKind::FlowchartDelay => "Delay",
            ShapeKind::FlowchartSequentialData => "Sequential Data",
            ShapeKind::FlowchartMagneticDisk => "Magnetic Disk",
            ShapeKind::FlowchartDirectAccessStorage => "Direct Access Storage",
            ShapeKind::FlowchartDisplay => "Display",

            ShapeKind::Explosion1 => "Explosion 1",
            ShapeKind::Explosion2 => "Explosion 2",
            ShapeKind::Star4 => "4-Point Star",
            ShapeKind::Star => "5-Point Star",
            ShapeKind::Star6 => "6-Point Star",
            ShapeKind::Star7 => "7-Point Star",
            ShapeKind::Star8 => "8-Point Star",
            ShapeKind::Star10 => "10-Point Star",
            ShapeKind::Star12 => "12-Point Star",
            ShapeKind::Star16 => "16-Point Star",
            ShapeKind::Star24 => "24-Point Star",
            ShapeKind::Star32 => "32-Point Star",
            ShapeKind::UpRibbon => "Up Ribbon",
            ShapeKind::DownRibbon => "Down Ribbon",
            ShapeKind::CurvedUpRibbon => "Curved Up Ribbon",
            ShapeKind::CurvedDownRibbon => "Curved Down Ribbon",
            ShapeKind::VerticalScroll => "Vertical Scroll",
            ShapeKind::HorizontalScroll => "Horizontal Scroll",
            ShapeKind::Wave => "Wave",
            ShapeKind::DoubleWave => "Double Wave",

            ShapeKind::RectangularCallout => "Rectangular Callout",
            ShapeKind::RoundedRectangularCallout => "Rounded Rectangular Callout",
            ShapeKind::OvalCallout => "Oval Callout",
            ShapeKind::CloudCallout => "Cloud Callout",
            ShapeKind::LineCallout1 => "Line Callout 1",
            ShapeKind::LineCallout2 => "Line Callout 2",
            ShapeKind::LineCallout3 => "Line Callout 3",
            ShapeKind::LineCallout4 => "Line Callout 4",
            ShapeKind::LineCallout1AccentBar => "Line Callout 1 (Accent Bar)",
            ShapeKind::LineCallout2AccentBar => "Line Callout 2 (Accent Bar)",
            ShapeKind::LineCallout3AccentBar => "Line Callout 3 (Accent Bar)",
            ShapeKind::LineCallout4AccentBar => "Line Callout 4 (Accent Bar)",

            ShapeKind::DrawingCanvas => "New Drawing Canvas",
        }
    }

    /// Preset geometry identifier in DrawingML (ECMA-376 / ISO 29500 `a:prstGeom prst="..."`).
    pub fn ooxml_prst(&self) -> &'static str {
        match self {
            ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::DrawingCanvas => "rect",
            ShapeKind::RoundedRectangle => "roundRect",
            ShapeKind::SingleCornerSnipped => "snip1Rect",
            ShapeKind::TopCornersSnipped => "snip2SameRect",
            ShapeKind::DiagonalCornersSnipped => "snip2DiagRect",
            ShapeKind::SnippedAndRounded => "snipRoundRect",
            ShapeKind::RoundSingleCorner => "round1Rect",
            ShapeKind::RoundSameSideCorner => "round2SameRect",
            ShapeKind::RoundDiagonalCorner => "round2DiagRect",

            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Triangle => "triangle",
            ShapeKind::RightTriangle => "rtTriangle",
            ShapeKind::Parallelogram => "parallelogram",
            ShapeKind::Trapezoid => "trapezoid",
            ShapeKind::Diamond => "diamond",
            ShapeKind::Pentagon => "pentagon",
            ShapeKind::Hexagon => "hexagon",
            ShapeKind::Heptagon => "heptagon",
            ShapeKind::Octagon => "octagon",
            ShapeKind::Decagon => "decagon",
            ShapeKind::Dodecagon => "dodecagon",
            ShapeKind::Pie => "pie",
            ShapeKind::Chord => "chord",
            ShapeKind::Teardrop => "teardrop",
            ShapeKind::Frame => "frame",
            ShapeKind::HalfFrame => "halfFrame",
            ShapeKind::LShape => "lShape",
            ShapeKind::DiagonalStripe => "diagStripe",
            ShapeKind::Cross => "cross",
            ShapeKind::Plaque => "plaque",
            ShapeKind::Cylinder => "can",
            ShapeKind::Cube => "cube",
            ShapeKind::Bevel => "bevel",
            ShapeKind::Donut => "donut",
            ShapeKind::NoSymbol => "noSmoking",
            ShapeKind::BlockArc => "blockArc",
            ShapeKind::FoldedCorner => "foldedCorner",
            ShapeKind::SmileyFace => "smileyFace",
            ShapeKind::Heart => "heart",
            ShapeKind::LightningBolt => "lightningBolt",
            ShapeKind::Sun => "sun",
            ShapeKind::Moon => "moon",
            ShapeKind::Cloud => "cloud",
            ShapeKind::Arc => "arc",
            ShapeKind::DoubleBracket => "bracketPair",
            ShapeKind::DoubleBrace => "bracePair",
            ShapeKind::LeftBracket => "leftBracket",
            ShapeKind::RightBracket => "rightBracket",
            ShapeKind::LeftBrace => "leftBrace",
            ShapeKind::RightBrace => "rightBrace",

            ShapeKind::Line => "line",
            ShapeKind::Arrow => "rightArrow",
            ShapeKind::DoubleArrow => "leftRightArrow",
            ShapeKind::StraightConnector => "straightConnector1",
            ShapeKind::ElbowConnector => "bentConnector2",
            ShapeKind::ElbowArrow => "bentConnector3",
            ShapeKind::ElbowDoubleArrow => "bentConnector4",
            ShapeKind::CurvedConnector => "curvedConnector2",
            ShapeKind::CurvedArrow => "curvedConnector3",
            ShapeKind::CurvedDoubleArrow => "curvedConnector4",
            ShapeKind::Curve => "curvedConnector5",
            ShapeKind::Freeform => "freeform",
            ShapeKind::Scribble => "scribble",

            ShapeKind::LeftArrow => "leftArrow",
            ShapeKind::UpArrow => "upArrow",
            ShapeKind::DownArrow => "downArrow",
            ShapeKind::LeftRightArrow => "leftRightArrow",
            ShapeKind::UpDownArrow => "upDownArrow",
            ShapeKind::QuadArrow => "quadArrow",
            ShapeKind::LeftRightUpArrow => "leftRightUpArrow",
            ShapeKind::BentArrow => "bentArrow",
            ShapeKind::UTurnArrow => "uturnArrow",
            ShapeKind::LeftUpArrow => "leftUpArrow",
            ShapeKind::BentUpArrow => "bentUpArrow",
            ShapeKind::StripedRightArrow => "stripedRightArrow",
            ShapeKind::NotchedRightArrow => "notchedRightArrow",
            ShapeKind::PentagonArrow => "homePlate",
            ShapeKind::Chevron => "chevron",
            ShapeKind::RightArrowCallout => "rightArrowCallout",
            ShapeKind::DownArrowCallout => "downArrowCallout",
            ShapeKind::LeftArrowCallout => "leftArrowCallout",
            ShapeKind::UpArrowCallout => "upArrowCallout",
            ShapeKind::LeftRightArrowCallout => "leftRightArrowCallout",
            ShapeKind::QuadArrowCallout => "quadArrowCallout",
            ShapeKind::CircularArrow => "circularArrow",

            ShapeKind::MathPlus => "mathPlus",
            ShapeKind::MathMinus => "mathMinus",
            ShapeKind::MathMultiply => "mathMultiply",
            ShapeKind::MathDivide => "mathDivide",
            ShapeKind::MathEqual => "mathEqual",
            ShapeKind::MathNotEqual => "mathNotEqual",

            ShapeKind::FlowchartProcess => "flowChartProcess",
            ShapeKind::FlowchartAlternateProcess => "flowChartAlternateProcess",
            ShapeKind::FlowchartDecision => "flowChartDecision",
            ShapeKind::FlowchartData => "flowChartInputOutput",
            ShapeKind::FlowchartPredefinedProcess => "flowChartPredefinedProcess",
            ShapeKind::FlowchartInternalStorage => "flowChartInternalStorage",
            ShapeKind::FlowchartDocument => "flowChartDocument",
            ShapeKind::FlowchartMultidocument => "flowChartMultidocument",
            ShapeKind::FlowchartTerminator => "flowChartTerminator",
            ShapeKind::FlowchartPreparation => "flowChartPreparation",
            ShapeKind::FlowchartManualInput => "flowChartManualInput",
            ShapeKind::FlowchartManualOperation => "flowChartManualOperation",
            ShapeKind::FlowchartConnector => "flowChartConnector",
            ShapeKind::FlowchartOffpageConnector => "flowChartOffpageConnector",
            ShapeKind::FlowchartCard => "flowChartPunchedCard",
            ShapeKind::FlowchartPunchedTape => "flowChartPunchedTape",
            ShapeKind::FlowchartSummingJunction => "flowChartSummingJunction",
            ShapeKind::FlowchartOr => "flowChartOr",
            ShapeKind::FlowchartCollate => "flowChartCollate",
            ShapeKind::FlowchartSort => "flowChartSort",
            ShapeKind::FlowchartExtract => "flowChartExtract",
            ShapeKind::FlowchartMerge => "flowChartMerge",
            ShapeKind::FlowchartStoredData => "flowChartOnlineStorage",
            ShapeKind::FlowchartDelay => "flowChartDelay",
            ShapeKind::FlowchartSequentialData => "flowChartMagneticTape",
            ShapeKind::FlowchartMagneticDisk => "flowChartMagneticDisk",
            ShapeKind::FlowchartDirectAccessStorage => "flowChartMagneticDrum",
            ShapeKind::FlowchartDisplay => "flowChartDisplay",

            ShapeKind::Explosion1 => "star8",
            ShapeKind::Explosion2 => "star14",
            ShapeKind::Star4 => "star4",
            ShapeKind::Star => "star5",
            ShapeKind::Star6 => "star6",
            ShapeKind::Star7 => "star7",
            ShapeKind::Star8 => "star8",
            ShapeKind::Star10 => "star10",
            ShapeKind::Star12 => "star12",
            ShapeKind::Star16 => "star16",
            ShapeKind::Star24 => "star24",
            ShapeKind::Star32 => "star32",
            ShapeKind::UpRibbon => "ribbon",
            ShapeKind::DownRibbon => "ribbon2",
            ShapeKind::CurvedUpRibbon => "ellipseRibbon",
            ShapeKind::CurvedDownRibbon => "ellipseRibbon2",
            ShapeKind::VerticalScroll => "verticalScroll",
            ShapeKind::HorizontalScroll => "horizontalScroll",
            ShapeKind::Wave => "wave",
            ShapeKind::DoubleWave => "doubleWave",

            ShapeKind::RectangularCallout => "wedgeRectCallout",
            ShapeKind::RoundedRectangularCallout => "wedgeRoundRectCallout",
            ShapeKind::OvalCallout => "wedgeEllipseCallout",
            ShapeKind::CloudCallout => "cloudCallout",
            ShapeKind::LineCallout1 => "lineCallout1",
            ShapeKind::LineCallout2 => "lineCallout2",
            ShapeKind::LineCallout3 => "lineCallout3",
            ShapeKind::LineCallout4 => "lineCallout4",
            ShapeKind::LineCallout1AccentBar => "lineCallout1AccentBar",
            ShapeKind::LineCallout2AccentBar => "lineCallout2AccentBar",
            ShapeKind::LineCallout3AccentBar => "lineCallout3AccentBar",
            ShapeKind::LineCallout4AccentBar => "lineCallout4AccentBar",
        }
    }

    /// Resolve an OOXML DrawingML preset shape name to `ShapeKind`.
    pub fn from_ooxml_prst(prst: &str) -> Self {
        match prst {
            "roundRect" => ShapeKind::RoundedRectangle,
            "snip1Rect" => ShapeKind::SingleCornerSnipped,
            "snip2SameRect" => ShapeKind::TopCornersSnipped,
            "snip2DiagRect" => ShapeKind::DiagonalCornersSnipped,
            "snipRoundRect" => ShapeKind::SnippedAndRounded,
            "round1Rect" => ShapeKind::RoundSingleCorner,
            "round2SameRect" => ShapeKind::RoundSameSideCorner,
            "round2DiagRect" => ShapeKind::RoundDiagonalCorner,

            "ellipse" => ShapeKind::Ellipse,
            "triangle" => ShapeKind::Triangle,
            "rtTriangle" => ShapeKind::RightTriangle,
            "parallelogram" => ShapeKind::Parallelogram,
            "trapezoid" => ShapeKind::Trapezoid,
            "diamond" => ShapeKind::Diamond,
            "pentagon" => ShapeKind::Pentagon,
            "hexagon" => ShapeKind::Hexagon,
            "heptagon" => ShapeKind::Heptagon,
            "octagon" => ShapeKind::Octagon,
            "decagon" => ShapeKind::Decagon,
            "dodecagon" => ShapeKind::Dodecagon,
            "pie" => ShapeKind::Pie,
            "chord" => ShapeKind::Chord,
            "teardrop" => ShapeKind::Teardrop,
            "frame" => ShapeKind::Frame,
            "halfFrame" => ShapeKind::HalfFrame,
            "lShape" => ShapeKind::LShape,
            "diagStripe" => ShapeKind::DiagonalStripe,
            "cross" => ShapeKind::Cross,
            "plaque" => ShapeKind::Plaque,
            "can" => ShapeKind::Cylinder,
            "cube" => ShapeKind::Cube,
            "bevel" => ShapeKind::Bevel,
            "donut" => ShapeKind::Donut,
            "noSmoking" => ShapeKind::NoSymbol,
            "blockArc" => ShapeKind::BlockArc,
            "foldedCorner" => ShapeKind::FoldedCorner,
            "smileyFace" => ShapeKind::SmileyFace,
            "heart" => ShapeKind::Heart,
            "lightningBolt" => ShapeKind::LightningBolt,
            "sun" => ShapeKind::Sun,
            "moon" => ShapeKind::Moon,
            "cloud" => ShapeKind::Cloud,
            "arc" => ShapeKind::Arc,
            "bracketPair" => ShapeKind::DoubleBracket,
            "bracePair" => ShapeKind::DoubleBrace,
            "leftBracket" => ShapeKind::LeftBracket,
            "rightBracket" => ShapeKind::RightBracket,
            "leftBrace" => ShapeKind::LeftBrace,
            "rightBrace" => ShapeKind::RightBrace,

            "line" => ShapeKind::Line,
            "rightArrow" => ShapeKind::Arrow,
            "straightConnector1" => ShapeKind::StraightConnector,
            "bentConnector2" | "bentConnector3" | "bentConnector4" | "bentConnector5" => ShapeKind::ElbowConnector,
            "curvedConnector2" | "curvedConnector3" | "curvedConnector4" | "curvedConnector5" => ShapeKind::CurvedConnector,
            "freeform" => ShapeKind::Freeform,
            "scribble" => ShapeKind::Scribble,

            "leftArrow" => ShapeKind::LeftArrow,
            "upArrow" => ShapeKind::UpArrow,
            "downArrow" => ShapeKind::DownArrow,
            "leftRightArrow" => ShapeKind::LeftRightArrow,
            "upDownArrow" => ShapeKind::UpDownArrow,
            "quadArrow" => ShapeKind::QuadArrow,
            "leftRightUpArrow" => ShapeKind::LeftRightUpArrow,
            "bentArrow" => ShapeKind::BentArrow,
            "uturnArrow" => ShapeKind::UTurnArrow,
            "leftUpArrow" => ShapeKind::LeftUpArrow,
            "bentUpArrow" => ShapeKind::BentUpArrow,
            "stripedRightArrow" => ShapeKind::StripedRightArrow,
            "notchedRightArrow" => ShapeKind::NotchedRightArrow,
            "homePlate" => ShapeKind::PentagonArrow,
            "chevron" => ShapeKind::Chevron,
            "rightArrowCallout" => ShapeKind::RightArrowCallout,
            "downArrowCallout" => ShapeKind::DownArrowCallout,
            "leftArrowCallout" => ShapeKind::LeftArrowCallout,
            "upArrowCallout" => ShapeKind::UpArrowCallout,
            "leftRightArrowCallout" => ShapeKind::LeftRightArrowCallout,
            "quadArrowCallout" => ShapeKind::QuadArrowCallout,
            "circularArrow" => ShapeKind::CircularArrow,

            "mathPlus" => ShapeKind::MathPlus,
            "mathMinus" => ShapeKind::MathMinus,
            "mathMultiply" => ShapeKind::MathMultiply,
            "mathDivide" => ShapeKind::MathDivide,
            "mathEqual" => ShapeKind::MathEqual,
            "mathNotEqual" => ShapeKind::MathNotEqual,

            "flowChartProcess" => ShapeKind::FlowchartProcess,
            "flowChartAlternateProcess" => ShapeKind::FlowchartAlternateProcess,
            "flowChartDecision" => ShapeKind::FlowchartDecision,
            "flowChartInputOutput" => ShapeKind::FlowchartData,
            "flowChartPredefinedProcess" => ShapeKind::FlowchartPredefinedProcess,
            "flowChartInternalStorage" => ShapeKind::FlowchartInternalStorage,
            "flowChartDocument" => ShapeKind::FlowchartDocument,
            "flowChartMultidocument" => ShapeKind::FlowchartMultidocument,
            "flowChartTerminator" => ShapeKind::FlowchartTerminator,
            "flowChartPreparation" => ShapeKind::FlowchartPreparation,
            "flowChartManualInput" => ShapeKind::FlowchartManualInput,
            "flowChartManualOperation" => ShapeKind::FlowchartManualOperation,
            "flowChartConnector" => ShapeKind::FlowchartConnector,
            "flowChartOffpageConnector" => ShapeKind::FlowchartOffpageConnector,
            "flowChartPunchedCard" => ShapeKind::FlowchartCard,
            "flowChartPunchedTape" => ShapeKind::FlowchartPunchedTape,
            "flowChartSummingJunction" => ShapeKind::FlowchartSummingJunction,
            "flowChartOr" => ShapeKind::FlowchartOr,
            "flowChartCollate" => ShapeKind::FlowchartCollate,
            "flowChartSort" => ShapeKind::FlowchartSort,
            "flowChartExtract" => ShapeKind::FlowchartExtract,
            "flowChartMerge" => ShapeKind::FlowchartMerge,
            "flowChartOnlineStorage" => ShapeKind::FlowchartStoredData,
            "flowChartDelay" => ShapeKind::FlowchartDelay,
            "flowChartMagneticTape" => ShapeKind::FlowchartSequentialData,
            "flowChartMagneticDisk" => ShapeKind::FlowchartMagneticDisk,
            "flowChartMagneticDrum" => ShapeKind::FlowchartDirectAccessStorage,
            "flowChartDisplay" => ShapeKind::FlowchartDisplay,

            "star4" => ShapeKind::Star4,
            "star5" => ShapeKind::Star,
            "star6" => ShapeKind::Star6,
            "star7" => ShapeKind::Star7,
            "star8" => ShapeKind::Star8,
            "star10" => ShapeKind::Star10,
            "star12" => ShapeKind::Star12,
            "star16" => ShapeKind::Star16,
            "star24" => ShapeKind::Star24,
            "star32" => ShapeKind::Star32,
            "ribbon" => ShapeKind::UpRibbon,
            "ribbon2" => ShapeKind::DownRibbon,
            "ellipseRibbon" => ShapeKind::CurvedUpRibbon,
            "ellipseRibbon2" => ShapeKind::CurvedDownRibbon,
            "verticalScroll" => ShapeKind::VerticalScroll,
            "horizontalScroll" => ShapeKind::HorizontalScroll,
            "wave" => ShapeKind::Wave,
            "doubleWave" => ShapeKind::DoubleWave,

            "wedgeRectCallout" => ShapeKind::RectangularCallout,
            "wedgeRoundRectCallout" => ShapeKind::RoundedRectangularCallout,
            "wedgeEllipseCallout" => ShapeKind::OvalCallout,
            "cloudCallout" => ShapeKind::CloudCallout,
            "lineCallout1" => ShapeKind::LineCallout1,
            "lineCallout2" => ShapeKind::LineCallout2,
            "lineCallout3" => ShapeKind::LineCallout3,
            "lineCallout4" => ShapeKind::LineCallout4,
            "lineCallout1AccentBar" => ShapeKind::LineCallout1AccentBar,
            "lineCallout2AccentBar" => ShapeKind::LineCallout2AccentBar,
            "lineCallout3AccentBar" => ShapeKind::LineCallout3AccentBar,
            "lineCallout4AccentBar" => ShapeKind::LineCallout4AccentBar,

            _ => ShapeKind::Rectangle,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NoteKind {
    #[default]
    Footnote,
    Endnote,
}

/// How ruby (phonetic guide) text lines up over its base text (ECMA-376 `ST_RubyAlign`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RubyAlign {
    #[default]
    Center,
    /// Letters spread evenly from edge to edge.
    DistributeLetter,
    /// Letters spread evenly with half a gap at each edge.
    DistributeSpace,
    Left,
    Right,
    /// Right-aligned beside vertical text (laid out like `Right` in horizontal text).
    RightVertical,
}

impl RubyAlign {
    pub const ALL: [RubyAlign; 6] =
        [RubyAlign::Center, RubyAlign::DistributeLetter, RubyAlign::DistributeSpace, RubyAlign::Left, RubyAlign::Right, RubyAlign::RightVertical];

    /// The OOXML name (`center`, `distributeLetter`…), also used by commands.
    pub fn ooxml(self) -> &'static str {
        match self {
            RubyAlign::Center => "center",
            RubyAlign::DistributeLetter => "distributeLetter",
            RubyAlign::DistributeSpace => "distributeSpace",
            RubyAlign::Left => "left",
            RubyAlign::Right => "right",
            RubyAlign::RightVertical => "rightVertical",
        }
    }

    pub fn from_ooxml(v: &str) -> Option<RubyAlign> {
        RubyAlign::ALL.into_iter().find(|a| a.ooxml() == v)
    }
}

/// Phonetic Guide's Offset for a ruby `raise` (baseline to baseline, points): the gap from the top
/// of the base text to the bottom of the ruby text, taking letters as rising 0.8 em above their
/// baseline and dropping 0.2 em below it.
pub fn ruby_offset(raise: f32, base_size: f32, size: f32) -> f32 {
    raise - 0.8 * base_size - 0.2 * size
}

/// The ruby raise (baseline to baseline, points) for a Phonetic Guide Offset (see [`ruby_offset`]).
pub fn ruby_raise(offset: f32, base_size: f32, size: f32) -> f32 {
    offset + 0.8 * base_size + 0.2 * size
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
        /// Drawing id, text warp, connector ends and arrowheads.
        #[serde(default, skip_serializing_if = "crate::connector::ShapeExtra::is_empty")]
        extra: crate::connector::ShapeExtra,
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
        /// A Drawing Canvas (Insert › Shapes › New Drawing Canvas, DrawingML's `wpc:wpc`) rather
        /// than a group: it has its own background and outline, and its frame is independent of
        /// its members: resizing it doesn't scale them (`ch_w` × `ch_h` follow `w` × `h`), and
        /// what lies outside the frame is clipped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        canvas: Option<CanvasStyle>,
    },
    /// A field: `instr` is the field code (`PAGE`, `NUMPAGES`, `DATE \@ "M/d/yyyy"`, `TOC \o "1-3"`…);
    /// `result` the cached display text. A `locked` field keeps its result when fields update.
    Field {
        instr: String,
        #[serde(default)]
        result: String,
        #[serde(default)]
        locked: bool,
        /// `w:dirty` (ECMA-376 §17.16.5.35): the cached result is stale — another program that
        /// couldn't compute it set the flag so Word refreshes the field on open. Kept through
        /// saves; cleared when WordCraft computes a result, like Word's F9.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dirty: bool,
        /// Shows its code (`{ PAGE }`) instead of its result, the other way round from the
        /// document-wide View › Field Codes setting (`fields.toggleCode`, Shift+F9). A view
        /// toggle: never written to files.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        code: bool,
        /// A legacy form field's definition (`FORMCHECKBOX`, `FORMTEXT`, `FORMDROPDOWN`): the
        /// source's `w:ffData`, raw OOXML kept for round trip. Empty for other fields.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        form: String,
    },
    /// The start of a field whose result is ordinary content — formatted, possibly spanning
    /// paragraphs — up to the matching [`InlineObject::FieldEnd`]. Imported TOCs and citation
    /// managers' `ADDIN` fields are kept this way; `instr` is the field code. See
    /// [`crate::fields`].
    FieldStart {
        instr: String,
        #[serde(default)]
        locked: bool,
        /// `w:dirty` on the begin marker, as [`InlineObject::Field::dirty`].
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dirty: bool,
    },
    /// The end of the innermost open [`InlineObject::FieldStart`].
    FieldEnd,
    /// The start of a content control whose content is what follows, up to the matching
    /// [`InlineObject::ControlEnd`]. See [`crate::control`].
    ControlStart {
        control: Box<crate::control::ContentControl>,
    },
    /// The end of the innermost open [`InlineObject::ControlStart`].
    ControlEnd,
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
    /// The start of a range that stays editable while the document is protected (Restrict
    /// Editing exceptions, `w:permStart`): for everyone (`group` = `everyone`), another editing
    /// group, or one editor (`editor`, a user name or address).
    PermStart {
        id: u32,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        group: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        editor: String,
    },
    /// The end of the editable range [`InlineObject::PermStart`] with the same `id` opened.
    PermEnd {
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
    /// Ruby (Phonetic Guide): small `ruby` text (furigana, pinyin…) set over `base` text. The base
    /// text takes the formatting of the object's run; it reads as the paragraph's text.
    Ruby {
        base: String,
        ruby: String,
        #[serde(default)]
        align: RubyAlign,
        /// Ruby text size, points (`w:hps`).
        size: f32,
        /// From the base text's baseline up to the ruby text's baseline, points (`w:hpsRaise`).
        raise: f32,
        /// The base text size the ruby was set for, points (`w:hpsBaseText`).
        #[serde(default)]
        base_size: f32,
        /// Language of the ruby (`w:lid`, e.g. `ja-JP`); empty = unspecified.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        lang: String,
        /// Formatting of the ruby text over the run's (font, colour…); its size is `size`.
        #[serde(default, skip_serializing_if = "CharProps::is_empty")]
        props: Box<CharProps>,
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

/// The background fill and outline of a Drawing Canvas (an [`InlineObject::Group`] with `canvas`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CanvasStyle {
    pub fill: Option<crate::props::Rgb>,
    pub stroke: Option<crate::props::Rgb>,
    /// Outline width, points.
    pub stroke_width: f32,
}

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
    /// Resize a drawing (a group's members scale with it; a canvas's keep their size and place).
    pub fn set_size(&mut self, nw: f32, nh: f32) {
        if let InlineObject::Image { w, h, .. }
        | InlineObject::Graphic { w, h, .. }
        | InlineObject::Shape { w, h, .. }
        | InlineObject::Group { w, h, .. } = self
        {
            *w = nw;
            *h = nh;
        }
        if let InlineObject::Group { ch_w, ch_h, canvas: Some(_), .. } = self {
            *ch_w = nw;
            *ch_h = nh;
        }
    }
    /// A Drawing Canvas's background and outline (`None` for anything else).
    pub fn canvas_style(&self) -> Option<&CanvasStyle> {
        match self {
            InlineObject::Group { canvas, .. } => canvas.as_ref(),
            _ => None,
        }
    }
    /// Whether this is a Drawing Canvas.
    pub fn is_canvas(&self) -> bool {
        self.canvas_style().is_some()
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
                | InlineObject::PermStart { .. }
                | InlineObject::PermEnd { .. }
                | InlineObject::FieldStart { .. }
                | InlineObject::FieldEnd
                | InlineObject::ControlStart { .. }
                | InlineObject::ControlEnd
        )
    }
    /// A drawing's description (alternative text), wherever it is kept.
    pub fn alt_text(&self) -> &str {
        match self {
            InlineObject::Image { alt, .. } | InlineObject::Graphic { alt, .. } => alt,
            InlineObject::Shape { float, .. } | InlineObject::Group { float, .. } => &float.alt.text,
            _ => "",
        }
    }
    /// Set a drawing's description (capped at [`MAX_ALT_TEXT`] characters); `false` if this is no drawing.
    pub fn set_alt_text(&mut self, text: &str) -> bool {
        let text = cap_alt(text);
        match self {
            InlineObject::Image { alt, .. } | InlineObject::Graphic { alt, .. } => *alt = text,
            InlineObject::Shape { float, .. } | InlineObject::Group { float, .. } => float.alt.text = text,
            _ => return false,
        }
        true
    }
    /// A drawing marked decorative.
    pub fn is_decorative(&self) -> bool {
        self.frame().is_some_and(|(_, _, f)| f.alt.decorative)
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
    /// The line a footnote or endnote separator draws (ECMA-376 §17.11: the `w:separator` and
    /// `w:continuationSeparator` run elements): a short line, or with `continuation` one across
    /// the column. Kept as the run element itself.
    pub fn note_separator(continuation: bool) -> InlineObject {
        let el = if continuation { "w:continuationSeparator" } else { "w:separator" };
        InlineObject::Opaque { format: "docx".into(), xml: format!("<w:r><{el}/></w:r>"), text: String::new() }
    }
    /// For a [`InlineObject::note_separator`]: whether it is the continuation separator.
    pub fn as_note_separator(&self) -> Option<bool> {
        match self {
            InlineObject::Opaque { format, xml, .. } if format == "docx" => match xml.as_str() {
                "<w:r><w:separator/></w:r>" => Some(false),
                "<w:r><w:continuationSeparator/></w:r>" => Some(true),
                _ => None,
            },
            _ => None,
        }
    }
    /// The text this object contributes to plain-text extraction.
    pub fn plain_text(&self) -> &str {
        match self {
            InlineObject::Field { result, .. } => result,
            InlineObject::Equation { linear, .. } => linear,
            InlineObject::Ruby { base, .. } => base,
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
    /// [`Self::plain_text`] as it reads on screen: characters in runs set in a symbol font
    /// (Symbol, Wingdings…) become their standard Unicode equivalents
    /// ([`crate::symbol_fonts`]). For copying as text and plain-text export; the stored text is
    /// unchanged.
    pub fn shown_text(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        let mut k = 0;
        let mut runs = self.run_ranges().peekable();
        for (i, c) in self.text.char_indices() {
            while runs.peek().is_some_and(|(r, _)| r.end <= i) {
                runs.next();
            }
            if c == OBJ {
                if let Some(o) = self.objects.get(k) {
                    out.push_str(o.plain_text());
                }
                k += 1;
                continue;
            }
            let font = runs.peek().filter(|(r, _)| r.contains(&i)).and_then(|(_, p)| p.font.as_deref());
            out.push(font.and_then(|f| crate::symbol_fonts::symbol_char(f, c)).unwrap_or(c));
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
            let keep = next.peek().is_none_or(|r| r.start > i);
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
        // Runs must end on character boundaries and cover the text exactly (never expected, but
        // files lie): clamp each end to the text, push it past a split character, pad the tail.
        let n = self.text.len();
        let mut end = 0usize;
        let mut out: Vec<Run> = Vec::with_capacity(self.runs.len());
        for r in self.runs.drain(..) {
            let mut e = end.saturating_add(r.len).min(n);
            while !self.text.is_char_boundary(e) {
                e += 1;
            }
            if e == end {
                continue;
            }
            match out.last_mut() {
                Some(l) if l.props == r.props => l.len += e - end,
                _ => out.push(Run { len: e - end, props: r.props }),
            }
            end = e;
        }
        if end < n {
            match out.last_mut() {
                Some(l) => l.len += n - end,
                None => out.push(Run { len: n, props: self.mark.clone() }),
            }
        }
        self.runs = out;
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
                // Formatting only: the mark is a tracked change of its own or it is not, whatever
                // revision the deleted text was part of.
                let revisions = (self.mark.ins, self.mark.del, self.mark.fmt_change.clone());
                let mut mark = self.props_of_char(a).clone();
                (mark.ins, mark.del, mark.fmt_change) = revisions;
                self.mark = mark;
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

    /// Delete `a..b` except the one-character objects at the offsets in `keep` (control
    /// markers a deletion leaves in place).
    pub fn delete_except(&mut self, a: usize, b: usize, keep: &[usize]) -> Result<()> {
        let (a, b) = (a.min(b), a.max(b));
        let mut keep: Vec<usize> =
            keep.iter().copied().filter(|k| *k >= a && k + OBJ.len_utf8() <= b && self.text.get(*k..).is_some_and(|t| t.starts_with(OBJ))).collect();
        if keep.is_empty() {
            return self.delete(a, b);
        }
        keep.sort_unstable();
        keep.dedup();
        // Pieces between the kept objects, last first so earlier offsets stay valid.
        let mut pieces = Vec::with_capacity(keep.len() + 1);
        let mut from = a;
        for k in &keep {
            pieces.push((from, *k));
            from = k + OBJ.len_utf8();
        }
        pieces.push((from, b));
        for (x, y) in pieces.into_iter().rev() {
            if y > x {
                self.delete(x, y)?;
            }
        }
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
        // The new paragraph's mark takes the formatting at the split, but never a hyperlink.
        let carry = self.props_at(off).unlinked();
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
            tail.mark = self.props_at(off).unlinked();
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

    /// Previous caret stop before `off` (0 at start): a grapheme boundary that doesn't split a
    /// complex-script syllable ([`crate::cluster`]).
    pub fn prev_boundary(&self, off: usize) -> usize {
        crate::cluster::prev_cluster(&self.text, self.clamp(off))
    }
    /// Next caret stop after `off` (len at end), see [`Self::prev_boundary`].
    pub fn next_boundary(&self, off: usize) -> usize {
        crate::cluster::next_cluster(&self.text, self.clamp(off))
    }
    /// Where Backspace at `off` deletes from: the previous character in the Brahmic scripts
    /// (Myanmar, Khmer, Devanagari, Thai…), where it removes one vowel sign or stacked consonant
    /// at a time, otherwise the previous caret stop.
    pub fn backspace_from(&self, off: usize) -> usize {
        let off = self.clamp(off);
        match self.text.get(..off).and_then(|t| t.char_indices().next_back()) {
            Some((i, c)) if crate::cluster::backspaces_by_char(c) => i,
            _ => self.prev_boundary(off),
        }
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
        p.insert_object(
            3,
            InlineObject::Field { instr: "PAGE".into(), result: "7".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
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
        p.insert_object(
            1,
            InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
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
    fn emptying_a_paragraph_keeps_the_formatting_but_not_the_revisions_of_its_text() {
        let mut props = bold();
        props.ins = Some(3);
        props.del = Some(4);
        let mut p = Paragraph::with_text("Hello", props);
        p.mark = bold();
        p.delete(0, 5).unwrap();
        assert!(p.is_empty());
        assert_eq!(p.mark.bold, Some(true));
        assert_eq!((p.mark.ins, p.mark.del), (None, None));
        // A mark that is itself a tracked change keeps its own revision.
        let mut p = Paragraph::with_text("Hello", bold());
        p.mark.del = Some(7);
        p.delete(0, 5).unwrap();
        assert_eq!((p.mark.ins, p.mark.del), (None, Some(7)));
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
