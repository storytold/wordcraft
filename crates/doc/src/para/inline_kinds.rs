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
    /// A block arrow pointing right (`rightArrow`).
    Arrow,
    /// A block arrow pointing left (`leftArrow`).
    ArrowLeft,
    /// A block arrow pointing up (`upArrow`).
    ArrowUp,
    /// A block arrow pointing down (`downArrow`).
    ArrowDown,
    /// A block arrow with a head at each end, pointing left and right (`leftRightArrow`).
    ArrowLeftRight,
    /// A block arrow with a head at each end, pointing up and down (`upDownArrow`).
    ArrowUpDown,
    /// A five-point star (`star5`). It stays `star` in JSON, so commands and saved documents
    /// that name it keep working; `star5` is read as well.
    #[serde(alias = "star5")]
    Star,
    /// The other star presets (`star4`, `star6` … `star32`): see [`ShapeKind::star`].
    Star4,
    Star6,
    Star7,
    Star8,
    Star10,
    Star12,
    Star16,
    Star24,
    Star32,
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

impl ShapeKind {
    /// A star preset's points, and the radius of its notches as a fraction of its points' radius.
    /// `None` for every other kind.
    ///
    /// The notch radius is the preset's default `adj` doubled, since ECMA-376 Part 1 §20.1.10.56
    /// pins `a` to 0..50000 and takes the notch radius as `a / 50000` of the star's own. The
    /// numbers are Word's live defaults for a preset with an empty `a:avLst`, read back through
    /// `Shape.Adjustments`: 0.125 for `star4`, 0.19098 for `star5`, 0.28868 for `star6`, 0.34601
    /// for `star7`, 0.42533 for `star10` and 0.375 for the rest.
    pub fn star(self) -> Option<(u32, f32)> {
        Some(match self {
            ShapeKind::Star4 => (4, 0.25),
            ShapeKind::Star => (5, 0.381_96),
            ShapeKind::Star6 => (6, 0.577_36),
            ShapeKind::Star7 => (7, 0.692_02),
            ShapeKind::Star8 => (8, 0.75),
            ShapeKind::Star10 => (10, 0.850_66),
            ShapeKind::Star12 => (12, 0.75),
            ShapeKind::Star16 => (16, 0.75),
            ShapeKind::Star24 => (24, 0.75),
            ShapeKind::Star32 => (32, 0.75),
            _ => return None,
        })
    }

    /// The star preset of `prst` (`star4` … `star32`), or `None` for any other name.
    pub fn from_star_preset(prst: &str) -> Option<ShapeKind> {
        let kind = match prst {
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
            _ => return None,
        };
        Some(kind)
    }

    /// A star kind's DrawingML preset name (`star4` … `star32`), `None` for every other kind.
    pub fn star_preset(self) -> Option<&'static str> {
        let prst = match self {
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
            _ => return None,
        };
        Some(prst)
    }

    /// All the star presets, fewest points first.
    pub const STARS: [ShapeKind; 10] = [
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
    ];
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

#[cfg(test)]
mod tests {
    use super::ShapeKind;

    /// #6391: every star preset has its own kind, point count and notch radius; the radius is
    /// Word's live default `adj` doubled (`Shape.Adjustments` on a shape with an empty `a:avLst`).
    #[test]
    fn star_presets_keep_their_points_and_notches() {
        let want: [(&str, ShapeKind, u32, f32); 10] = [
            ("star4", ShapeKind::Star4, 4, 2.0 * 0.125),
            ("star5", ShapeKind::Star, 5, 2.0 * 0.190_98),
            ("star6", ShapeKind::Star6, 6, 2.0 * 0.288_68),
            ("star7", ShapeKind::Star7, 7, 2.0 * 0.346_01),
            ("star8", ShapeKind::Star8, 8, 2.0 * 0.375),
            ("star10", ShapeKind::Star10, 10, 2.0 * 0.425_33),
            ("star12", ShapeKind::Star12, 12, 2.0 * 0.375),
            ("star16", ShapeKind::Star16, 16, 2.0 * 0.375),
            ("star24", ShapeKind::Star24, 24, 2.0 * 0.375),
            ("star32", ShapeKind::Star32, 32, 2.0 * 0.375),
        ];
        assert_eq!(want.map(|w| w.1).to_vec(), ShapeKind::STARS.to_vec());
        for (prst, kind, points, notch) in want {
            assert_eq!(ShapeKind::from_star_preset(prst), Some(kind), "{prst} is read");
            assert_eq!(kind.star_preset(), Some(prst), "{prst} is written back");
            let (n, r) = kind.star().unwrap_or((0, 0.0));
            assert_eq!(n, points, "{prst} has {points} points");
            assert!((r - notch).abs() < 0.000_05, "{prst}: notch {r} is not {notch}");
        }
        // Nothing else is a star, and no other preset name reads as one.
        for kind in [ShapeKind::Rectangle, ShapeKind::Heart, ShapeKind::Arrow, ShapeKind::ArrowLeftRight, ShapeKind::TextBox] {
            assert_eq!(kind.star(), None, "{kind:?}");
            assert_eq!(kind.star_preset(), None, "{kind:?}");
        }
        for prst in ["", "star", "star0", "star2", "star9", "star64", "STAR5", "star5x", "heart"] {
            assert_eq!(ShapeKind::from_star_preset(prst), None, "{prst:?}");
        }
    }

    /// Commands, the control channel and saved documents name a kind in JSON, so `star` keeps
    /// meaning the five-point star and the new kinds get the preset's own name.
    #[test]
    fn shape_kinds_keep_their_json_names() {
        let name = |k: ShapeKind| serde_json::to_value(k).ok().and_then(|v| v.as_str().map(str::to_owned));
        let kind = |s: &str| serde_json::from_value::<ShapeKind>(serde_json::json!(s)).ok();
        assert_eq!(name(ShapeKind::Star).as_deref(), Some("star"));
        assert_eq!(kind("star"), Some(ShapeKind::Star));
        // `star5` is the preset's own name for the same thing.
        assert_eq!(kind("star5"), Some(ShapeKind::Star));
        assert_eq!(name(ShapeKind::Star8).as_deref(), Some("star8"));
        assert_eq!(kind("star8"), Some(ShapeKind::Star8));
        assert_eq!(kind("star32"), Some(ShapeKind::Star32));
        assert_eq!(name(ShapeKind::ArrowLeftRight).as_deref(), Some("arrowLeftRight"));
        assert_eq!(kind("arrowLeftRight"), Some(ShapeKind::ArrowLeftRight));
        assert_eq!(kind("arrowUpDown"), Some(ShapeKind::ArrowUpDown));
        // The kinds that were already there keep their names.
        assert_eq!(name(ShapeKind::Arrow).as_deref(), Some("arrow"));
        assert_eq!(name(ShapeKind::Rectangle).as_deref(), Some("rectangle"));
        assert_eq!(kind("star99"), None);
    }
}
