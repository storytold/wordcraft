use super::*;

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
        let v = v.trim();
        if v.eq_ignore_ascii_case("bothSides") || v.eq_ignore_ascii_case("both") {
            Some(WrapText::BothSides)
        } else if v.eq_ignore_ascii_case("left") {
            Some(WrapText::Left)
        } else if v.eq_ignore_ascii_case("right") {
            Some(WrapText::Right)
        } else if v.eq_ignore_ascii_case("largest") {
            Some(WrapText::Largest)
        } else {
            None
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
    /// Anchored in a table cell but placed on the page, not in the cell (`wp:anchor/@layoutInCell`
    /// off, any VML shape in a header or footer): a picture watermark in a header that starts with a table.
    #[serde(skip_serializing_if = "is_false")]
    pub outside_cell: bool,
    /// Stacking order among floating objects (`wp:anchor/@relativeHeight`): higher is drawn on
    /// top. 0 = unknown, drawn in document order (objects inserted here, or from other formats).
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub z: u32,
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

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
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
