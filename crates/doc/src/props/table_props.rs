use super::char_props::PropChange;
use super::color::Rgb;
use super::para_props::{Align, Borders};
use super::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

/// Which way a table cell's text runs (`w:textDirection`, ECMA-376 §17.4.72).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextDirection {
    /// Left to right, lines stacking downwards.
    #[default]
    Horizontal,
    /// Turned 90° clockwise: lines read top to bottom and stack from right to left (`tbRl`).
    Down,
    /// Turned 90° counter-clockwise: lines read bottom to top and stack from left to right (`btLr`).
    Up,
}

impl TextDirection {
    /// The direction an OOXML `ST_TextDirection` value names (transitional and strict names).
    /// The East Asian vertical layouts turn like `Down`; unknown values are horizontal.
    pub fn from_ooxml(v: &str) -> TextDirection {
        match v {
            "tbRl" | "tbRlV" | "tbLrV" | "rl" | "rlV" | "lrV" => TextDirection::Down,
            "btLr" | "lr" => TextDirection::Up,
            _ => TextDirection::Horizontal,
        }
    }
    /// The transitional OOXML name.
    pub fn ooxml(self) -> &'static str {
        match self {
            TextDirection::Horizontal => "lrTb",
            TextDirection::Down => "tbRl",
            TextDirection::Up => "btLr",
        }
    }
    /// Word's Text Direction button: horizontal → down → up → horizontal.
    pub fn next(self) -> TextDirection {
        match self {
            TextDirection::Horizontal => TextDirection::Down,
            TextDirection::Down => TextDirection::Up,
            TextDirection::Up => TextDirection::Horizontal,
        }
    }
    pub fn is_turned(self) -> bool {
        self != TextDirection::Horizontal
    }
}

/// Which way a text box's text runs (Shape Format › Text › Text Direction; DrawingML
/// `a:bodyPr/@vert`, ECMA-376 §21.1.10.83 `ST_TextVerticalType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextVert {
    /// Upright, lines stacking downwards (`horz`).
    #[default]
    Horz,
    /// Turned 90° clockwise (`vert`).
    Vert,
    /// Turned 270° (`vert270`).
    Vert270,
    /// Upright letters stacked one above the other (`wordArtVert`).
    Stacked,
}

impl TextVert {
    /// The direction an `ST_TextVerticalType` value names; the East Asian and Mongolian vertical
    /// layouts turn like `vert`, unknown values are horizontal.
    pub fn from_ooxml(v: &str) -> TextVert {
        match v {
            "vert" | "eaVert" | "mongolianVert" => TextVert::Vert,
            "vert270" => TextVert::Vert270,
            "wordArtVert" | "wordArtVertRtl" => TextVert::Stacked,
            _ => TextVert::Horz,
        }
    }
    pub fn ooxml(self) -> &'static str {
        match self {
            TextVert::Horz => "horz",
            TextVert::Vert => "vert",
            TextVert::Vert270 => "vert270",
            TextVert::Stacked => "wordArtVert",
        }
    }
    /// How its lines turn on the page (stacked text stays upright).
    pub fn turn(self) -> TextDirection {
        match self {
            TextVert::Vert => TextDirection::Down,
            TextVert::Vert270 => TextDirection::Up,
            TextVert::Horz | TextVert::Stacked => TextDirection::Horizontal,
        }
    }
}

/// Where a text box's text sits between its top and bottom (Align Text; `a:bodyPr/@anchor`,
/// `ST_TextAnchoringType`): reuses [`VAlign`]. Justified and distributed anchoring read as top
/// and middle.
pub fn valign_from_anchor(v: &str) -> VAlign {
    match v {
        "ctr" | "dist" => VAlign::Center,
        "b" => VAlign::Bottom,
        _ => VAlign::Top,
    }
}

/// The `ST_TextAnchoringType` value for `a`.
pub fn anchor_ooxml(a: VAlign) -> &'static str {
    match a {
        VAlign::Top => "t",
        VAlign::Center => "ctr",
        VAlign::Bottom => "b",
    }
}

/// Table-wide properties.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TableProps {
    pub style: Option<String>,
    /// Preferred width: points (`Some`) or automatic (`None`).
    pub width: Option<f32>,
    /// Preferred width as a percentage of the text column (overrides `width`).
    pub width_pct: Option<f32>,
    pub align: Option<Align>,
    /// Indent from the left margin, points.
    pub indent: Option<f32>,
    pub borders: Option<Borders>,
    /// Default cell margins (top, left, bottom, right), points.
    pub cell_margins: Option<[f32; 4]>,
    /// Rows per band (`w:tblStyleRowBandSize`) set directly on this table or row exception.
    pub band_size: Option<u32>,
    /// Columns per band (`w:tblStyleColBandSize`) set directly on this table or row exception.
    pub col_band_size: Option<u32>,
    /// Fixed column widths (no auto-fit).
    pub fixed: bool,
    /// Table style options ("look"): header row, total row, banded rows, first column, last
    /// column, banded columns.
    pub look: TableLook,
    pub shading: Option<Rgb>,
    /// Alternative text: the title (`w:tblCaption`).
    pub caption: Option<String>,
    /// Alternative text: the description (`w:tblDescription`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Floating placement (`w:tblpPr`); `None` for a table in the text flow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub float: Option<TableFloat>,
    /// Tracked formatting change (`w:tblPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<TableProps>>>,
    /// Right-to-left table (`w:bidiVisual`): the first logical column stands at the right edge,
    /// cell margins and left/right borders mirror, and the alignment and indent are measured from
    /// the right margin.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bidi_visual: bool,
}

/// Where a floating table sits; the text after it wraps around it.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TableFloat {
    /// What `x` is measured from: the column (`Column`), the margin or the page.
    pub h_rel: crate::para::Anchor,
    /// What `y` is measured from: the text where the table stands (`Paragraph`), the margin or
    /// the page.
    pub v_rel: crate::para::Anchor,
    /// Offsets, points (used when the matching alignment is `None`).
    pub x: f32,
    pub y: f32,
    pub h_align: Option<crate::para::FloatAlign>,
    pub v_align: Option<crate::para::FloatAlign>,
    /// Distance from surrounding text: left, top, right, bottom (points).
    pub dist: [f32; 4],
    /// Whether it may overlap other floating tables (`w:tblOverlap`); Word's default is yes.
    pub overlap: bool,
}

impl TableFloat {
    /// The distances from text, finite and clamped (left, top, right, bottom).
    pub fn dist_from_text(&self) -> [f32; 4] {
        self.dist.map(|v| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TableLook {
    pub header_row: bool,
    pub total_row: bool,
    pub banded_rows: bool,
    pub first_column: bool,
    pub last_column: bool,
    pub banded_columns: bool,
}

impl Default for TableLook {
    fn default() -> Self {
        TableLook { header_row: true, total_row: false, banded_rows: true, first_column: true, last_column: false, banded_columns: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HeightRule {
    #[default]
    Auto,
    AtLeast,
    Exact,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RowProps {
    /// Points (meaning depends on `height_rule`).
    pub height: Option<f32>,
    pub height_rule: HeightRule,
    /// Repeat as a header row at the top of each page.
    pub header: bool,
    /// Don't let the row break across pages.
    pub cant_split: bool,
    /// Table properties this row overrides (`w:tblPrEx`, ECMA-376 §17.4.61): width, alignment,
    /// indent, borders, shading, layout and cell margins. `Some` when the file has the element.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exceptions: Option<Box<TableProps>>,
    /// Grid columns skipped before the row's first cell (`w:gridBefore`), and the preferred
    /// width of that gap in points (`w:wBefore`).
    #[serde(skip_serializing_if = "is_zero")]
    pub grid_before: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w_before: Option<f32>,
    /// Grid columns left after the row's last cell (`w:gridAfter`), and their width in points
    /// (`w:wAfter`).
    #[serde(skip_serializing_if = "is_zero")]
    pub grid_after: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w_after: Option<f32>,
    /// Tracked formatting change (`w:trPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<RowProps>>>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VMerge {
    #[default]
    None,
    /// First cell of a vertically merged range.
    Restart,
    /// Continues the merge from the cell above.
    Continue,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CellProps {
    /// Preferred width, points.
    pub width: Option<f32>,
    /// Preferred width as a percentage of the table's (`w:tcW w:type="pct"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_pct: Option<f32>,
    /// Number of grid columns spanned (1 = no horizontal merge).
    pub span: u32,
    pub vmerge: VMerge,
    pub shading: Option<Rgb>,
    pub borders: Option<Borders>,
    /// Vertical alignment (`w:vAlign`); `None` follows the table style, else top. An explicit
    /// `Top` beats the style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valign: Option<VAlign>,
    /// Cell margins override (top, left, bottom, right), points.
    pub margins: Option<[f32; 4]>,
    /// Per-side overrides when `margins` is absent; omitted sides inherit table defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_margins: Option<[Option<f32>; 4]>,
    /// Which way the cell's text runs.
    pub text_direction: TextDirection,
    /// The file states the horizontal direction (`lrTb`) outright; saving writes it back.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub text_direction_set: bool,
    pub no_wrap: bool,
    /// The cell's last paragraph takes no room when it is empty (`w:hideMark`): a row of such
    /// empty cells shrinks to its borders.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hide_mark: bool,
    /// Tracked formatting change (`w:tcPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<CellProps>>>,
}

impl CellProps {
    /// Resolve top, left, bottom and right margins against the current table defaults.
    /// A complete override takes precedence over per-side overrides.
    pub fn resolved_margins(&self, defaults: [f32; 4]) -> [f32; 4] {
        if let Some(margins) = self.margins {
            return margins;
        }
        self.partial_margins.map_or(defaults, |sides| std::array::from_fn(|i| sides[i].unwrap_or(defaults[i])))
    }
}
