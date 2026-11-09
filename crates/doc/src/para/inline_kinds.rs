use super::{Deserialize, Serialize};

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
    #[serde(alias = "leftArrow")]
    ArrowLeft,
    #[serde(alias = "upArrow")]
    ArrowUp,
    #[serde(alias = "downArrow")]
    ArrowDown,
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
    /// Every built-in shape preset.
    pub const ALL: &[ShapeKind] = &[
        ShapeKind::Rectangle,
        ShapeKind::RoundedRectangle,
        ShapeKind::SingleCornerSnipped,
        ShapeKind::TopCornersSnipped,
        ShapeKind::DiagonalCornersSnipped,
        ShapeKind::SnippedAndRounded,
        ShapeKind::RoundSingleCorner,
        ShapeKind::RoundSameSideCorner,
        ShapeKind::RoundDiagonalCorner,
        ShapeKind::Ellipse,
        ShapeKind::Triangle,
        ShapeKind::RightTriangle,
        ShapeKind::Parallelogram,
        ShapeKind::Trapezoid,
        ShapeKind::Diamond,
        ShapeKind::Pentagon,
        ShapeKind::Hexagon,
        ShapeKind::Heptagon,
        ShapeKind::Octagon,
        ShapeKind::Decagon,
        ShapeKind::Dodecagon,
        ShapeKind::Pie,
        ShapeKind::Chord,
        ShapeKind::Teardrop,
        ShapeKind::Frame,
        ShapeKind::HalfFrame,
        ShapeKind::LShape,
        ShapeKind::DiagonalStripe,
        ShapeKind::Cross,
        ShapeKind::Plaque,
        ShapeKind::Cylinder,
        ShapeKind::Cube,
        ShapeKind::Bevel,
        ShapeKind::Donut,
        ShapeKind::NoSymbol,
        ShapeKind::BlockArc,
        ShapeKind::FoldedCorner,
        ShapeKind::SmileyFace,
        ShapeKind::Heart,
        ShapeKind::LightningBolt,
        ShapeKind::Sun,
        ShapeKind::Moon,
        ShapeKind::Cloud,
        ShapeKind::Arc,
        ShapeKind::DoubleBracket,
        ShapeKind::DoubleBrace,
        ShapeKind::LeftBracket,
        ShapeKind::RightBracket,
        ShapeKind::LeftBrace,
        ShapeKind::RightBrace,
        ShapeKind::TextBox,
        ShapeKind::Line,
        ShapeKind::Arrow,
        ShapeKind::DoubleArrow,
        ShapeKind::StraightConnector,
        ShapeKind::ElbowConnector,
        ShapeKind::ElbowArrow,
        ShapeKind::ElbowDoubleArrow,
        ShapeKind::CurvedConnector,
        ShapeKind::CurvedArrow,
        ShapeKind::CurvedDoubleArrow,
        ShapeKind::Curve,
        ShapeKind::Freeform,
        ShapeKind::Scribble,
        ShapeKind::ArrowLeft,
        ShapeKind::ArrowUp,
        ShapeKind::ArrowDown,
        ShapeKind::LeftRightArrow,
        ShapeKind::UpDownArrow,
        ShapeKind::QuadArrow,
        ShapeKind::LeftRightUpArrow,
        ShapeKind::BentArrow,
        ShapeKind::UTurnArrow,
        ShapeKind::LeftUpArrow,
        ShapeKind::BentUpArrow,
        ShapeKind::StripedRightArrow,
        ShapeKind::NotchedRightArrow,
        ShapeKind::PentagonArrow,
        ShapeKind::Chevron,
        ShapeKind::RightArrowCallout,
        ShapeKind::DownArrowCallout,
        ShapeKind::LeftArrowCallout,
        ShapeKind::UpArrowCallout,
        ShapeKind::LeftRightArrowCallout,
        ShapeKind::QuadArrowCallout,
        ShapeKind::CircularArrow,
        ShapeKind::MathPlus,
        ShapeKind::MathMinus,
        ShapeKind::MathMultiply,
        ShapeKind::MathDivide,
        ShapeKind::MathEqual,
        ShapeKind::MathNotEqual,
        ShapeKind::FlowchartProcess,
        ShapeKind::FlowchartAlternateProcess,
        ShapeKind::FlowchartDecision,
        ShapeKind::FlowchartData,
        ShapeKind::FlowchartPredefinedProcess,
        ShapeKind::FlowchartInternalStorage,
        ShapeKind::FlowchartDocument,
        ShapeKind::FlowchartMultidocument,
        ShapeKind::FlowchartTerminator,
        ShapeKind::FlowchartPreparation,
        ShapeKind::FlowchartManualInput,
        ShapeKind::FlowchartManualOperation,
        ShapeKind::FlowchartConnector,
        ShapeKind::FlowchartOffpageConnector,
        ShapeKind::FlowchartCard,
        ShapeKind::FlowchartPunchedTape,
        ShapeKind::FlowchartSummingJunction,
        ShapeKind::FlowchartOr,
        ShapeKind::FlowchartCollate,
        ShapeKind::FlowchartSort,
        ShapeKind::FlowchartExtract,
        ShapeKind::FlowchartMerge,
        ShapeKind::FlowchartStoredData,
        ShapeKind::FlowchartDelay,
        ShapeKind::FlowchartSequentialData,
        ShapeKind::FlowchartMagneticDisk,
        ShapeKind::FlowchartDirectAccessStorage,
        ShapeKind::FlowchartDisplay,
        ShapeKind::Explosion1,
        ShapeKind::Explosion2,
        ShapeKind::Star4,
        ShapeKind::Star,
        ShapeKind::Star6,
        ShapeKind::Star7,
        ShapeKind::Star8,
        ShapeKind::Star10,
        ShapeKind::Star12,
        ShapeKind::Star16,
        ShapeKind::Star24,
        ShapeKind::Star32,
        ShapeKind::UpRibbon,
        ShapeKind::DownRibbon,
        ShapeKind::CurvedUpRibbon,
        ShapeKind::CurvedDownRibbon,
        ShapeKind::VerticalScroll,
        ShapeKind::HorizontalScroll,
        ShapeKind::Wave,
        ShapeKind::DoubleWave,
        ShapeKind::RectangularCallout,
        ShapeKind::RoundedRectangularCallout,
        ShapeKind::OvalCallout,
        ShapeKind::CloudCallout,
        ShapeKind::LineCallout1,
        ShapeKind::LineCallout2,
        ShapeKind::LineCallout3,
        ShapeKind::LineCallout4,
        ShapeKind::LineCallout1AccentBar,
        ShapeKind::LineCallout2AccentBar,
        ShapeKind::LineCallout3AccentBar,
        ShapeKind::LineCallout4AccentBar,
        ShapeKind::DrawingCanvas,
    ];

    /// True if the shape is fundamentally a 1D stroke/line rather than a 2D filled polygon.
    pub fn is_line_like(&self) -> bool {
        matches!(
            self,
            ShapeKind::Line
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

            ShapeKind::ArrowLeft => "Left Arrow",
            ShapeKind::ArrowUp => "Up Arrow",
            ShapeKind::ArrowDown => "Down Arrow",
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
            // A line with a head at each end (Office's Lines › Double Arrow), not the block
            // `leftRightArrow` preset: the arrowheads are the `a:headEnd`/`a:tailEnd` we write.
            ShapeKind::DoubleArrow => "line",
            ShapeKind::StraightConnector => "straightConnector1",
            ShapeKind::ElbowConnector => "bentConnector2",
            ShapeKind::ElbowArrow => "bentConnector3",
            ShapeKind::ElbowDoubleArrow => "bentConnector4",
            ShapeKind::CurvedConnector => "curvedConnector2",
            ShapeKind::CurvedArrow => "curvedConnector3",
            ShapeKind::CurvedDoubleArrow => "curvedConnector4",
            ShapeKind::Curve => "curvedConnector5",
            // No DrawingML preset exists for these: with custom geometry they are written as
            // `a:custGeom`, and without it a curve keeps the shape visible and the file readable
            // (a `prst="freeform"` makes Word report unreadable content).
            ShapeKind::Freeform | ShapeKind::Scribble => "curvedConnector3",

            ShapeKind::ArrowLeft => "leftArrow",
            ShapeKind::ArrowUp => "upArrow",
            ShapeKind::ArrowDown => "downArrow",
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

            ShapeKind::Explosion1 => "irregularSeal1",
            ShapeKind::Explosion2 => "irregularSeal2",
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

            "line" | "lineInv" => ShapeKind::Line,
            "rightArrow" => ShapeKind::Arrow,
            "straightConnector1" => ShapeKind::StraightConnector,
            "bentConnector2" | "bentConnector3" | "bentConnector4" | "bentConnector5" => ShapeKind::ElbowConnector,
            // `curvedConnector5` is what we write for a Curve, so it reads back as one and a
            // curve keeps its kind through a save and reopen.
            "curvedConnector2" | "curvedConnector3" | "curvedConnector4" => ShapeKind::CurvedConnector,
            "curvedConnector5" => ShapeKind::Curve,
            "freeform" => ShapeKind::Freeform,
            "scribble" => ShapeKind::Scribble,

            "leftArrow" => ShapeKind::ArrowLeft,
            "upArrow" => ShapeKind::ArrowUp,
            "downArrow" => ShapeKind::ArrowDown,
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

            "irregularSeal1" => ShapeKind::Explosion1,
            "irregularSeal2" => ShapeKind::Explosion2,
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

    /// This kind with the arrowheads a file gives it (`a:headEnd` / `a:tailEnd`, ECMA-376
    /// §20.1.8.38 / §20.1.8.57): a line with a head at each end is a [`ShapeKind::DoubleArrow`],
    /// and an elbow or curved connector becomes its arrow variant, so the kind survives a save
    /// and reopen (the arrowheads alone don't tell the kinds apart otherwise).
    pub fn with_arrowheads(self, start: bool, end: bool) -> ShapeKind {
        match self {
            ShapeKind::Line | ShapeKind::DoubleArrow => {
                if start && end {
                    ShapeKind::DoubleArrow
                } else {
                    ShapeKind::Line
                }
            }
            ShapeKind::ElbowConnector | ShapeKind::ElbowArrow | ShapeKind::ElbowDoubleArrow => match (start, end) {
                (false, false) => ShapeKind::ElbowConnector,
                (false, true) => ShapeKind::ElbowArrow,
                _ => ShapeKind::ElbowDoubleArrow,
            },
            ShapeKind::CurvedConnector | ShapeKind::CurvedArrow | ShapeKind::CurvedDoubleArrow => match (start, end) {
                (false, false) => ShapeKind::CurvedConnector,
                (false, true) => ShapeKind::CurvedArrow,
                _ => ShapeKind::CurvedDoubleArrow,
            },
            other => other,
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
