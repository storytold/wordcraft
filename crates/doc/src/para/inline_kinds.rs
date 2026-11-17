use super::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ShapeKind {
    #[default]
    Rectangle,
    RoundedRectangle,
    Ellipse,
    /// Flowchart OR preset: an ellipse with horizontal and vertical spokes.
    FlowchartOr,
    Triangle,
    /// A right triangle (DrawingML's `rtTriangle`): the right angle at the bottom left.
    RightTriangle,
    Diamond,
    Line,
    /// A block arrow pointing right (`rightArrow`; also stands for the double-headed arrows).
    Arrow,
    /// A block arrow pointing left (`leftArrow`).
    ArrowLeft,
    /// A block arrow pointing up (`upArrow`).
    ArrowUp,
    /// A block arrow pointing down (`downArrow`).
    ArrowDown,
    Star,
    Heart,
    /// A text box (rectangle with a text story).
    TextBox,
    /// A shape drawn point by point, such as ink (its geometry is the shape's `freeform`).
    Freeform,
    /// Connectors: lines whose ends can be glued to other shapes (see [`crate::connector`]).
    StraightConnector,
    ElbowConnector,
    CurvedConnector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NoteKind {
    #[default]
    Footnote,
    Endnote,
}

/// Where a positional tab ([`InlineObject::PositionalTab`]) lands on its line, and how the text
/// after it lines up there (ECMA-376 `ST_PTabAlignment`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PTabAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl PTabAlign {
    pub const ALL: [PTabAlign; 3] = [PTabAlign::Left, PTabAlign::Center, PTabAlign::Right];

    /// The OOXML name (`left`, `center`, `right`).
    pub fn ooxml(self) -> &'static str {
        match self {
            PTabAlign::Left => "left",
            PTabAlign::Center => "center",
            PTabAlign::Right => "right",
        }
    }

    pub fn from_ooxml(v: &str) -> Option<PTabAlign> {
        PTabAlign::ALL.into_iter().find(|a| a.ooxml() == v)
    }
}

/// What a positional tab's place is measured against (ECMA-376 `ST_PTabRelativeTo`): the text
/// margins, or the paragraph's indents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PTabBase {
    #[default]
    Margin,
    Indent,
}

impl PTabBase {
    pub const ALL: [PTabBase; 2] = [PTabBase::Margin, PTabBase::Indent];

    /// The OOXML name (`margin`, `indent`).
    pub fn ooxml(self) -> &'static str {
        match self {
            PTabBase::Margin => "margin",
            PTabBase::Indent => "indent",
        }
    }

    pub fn from_ooxml(v: &str) -> Option<PTabBase> {
        PTabBase::ALL.into_iter().find(|a| a.ooxml() == v)
    }
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
