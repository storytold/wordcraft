//! Styles: paragraph, character, table and linked styles with `based on` chains, plus the
//! built-in style set (our own values in the spirit of a modern word processor's defaults).

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::props::{
    Align, Border, BorderStyle, Borders, CharProps, LineSpacing, ParaProps, Rgb, TabAlign, TabLeader, TabStop, TableLook, TextColor, Underline,
    VAlign,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StyleKind {
    #[default]
    Paragraph,
    Character,
    Table,
    Numbering,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Style {
    /// Stable id (`Heading1`), referenced by `ParaProps::style` / `CharProps::style`.
    pub id: String,
    /// Display name (`Heading 1`).
    pub name: String,
    pub kind: StyleKind,
    pub based_on: Option<String>,
    /// Style for the paragraph after this one (Enter at the end).
    pub next: Option<String>,
    /// Linked character/paragraph style id.
    pub linked: Option<String>,
    pub para: ParaProps,
    pub chr: CharProps,
    /// Gallery order (lower first).
    pub priority: Option<u32>,
    /// Shown in the Styles gallery on the Home tab.
    pub quick: bool,
    pub hidden: bool,
    /// Built in (cannot be deleted).
    pub builtin: bool,
    /// Can't be applied while Restrict Editing limits formatting to a selection of styles
    /// (`w:locked`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// The default style of its kind (`w:default`), whatever its id: localized documents call
    /// the default character, table and list styles something other than the English ids.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
    /// Table style: conditional formatting for header row, banding etc.
    pub table: Option<TableStyleParts>,
}

/// Table style conditional formats (shading + character formatting per region).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TableStyleParts {
    pub borders: Option<Borders>,
    /// Shading of every cell (`w:tblPr/w:shd`, or the `wholeTable` region's cell shading).
    pub fill: Option<Rgb>,
    /// Cell margins, points: [top, left, bottom, right] (`w:tblCellMar`).
    pub cell_margins: Option<[f32; 4]>,
    pub header_fill: Option<Rgb>,
    pub header_chr: CharProps,
    /// Cell borders of the header row (`firstRow` region's `w:tcBorders`): `between` is the
    /// border between header rows, `inside_v` between its cells.
    pub header_borders: Option<Borders>,
    pub band_fill: Option<Rgb>,
    /// Character formatting of the odd row bands (`band1Horz` region).
    pub band_chr: CharProps,
    /// Cell borders of the odd row bands (`band1Horz` region's `w:tcBorders`).
    pub band_borders: Option<Borders>,
    /// Rows per band (`w:tblStyleRowBandSize`). Unset (in the whole based-on chain) or zero, the
    /// row bands are off: Word reads a missing size as zero, where ECMA-376 says one (MS-OE376
    /// §2.7.5.7, #3415). Our own banded styles set one.
    pub band_size: Option<u32>,
    /// Columns per band (`w:tblStyleColBandSize`); unset or zero, the column bands are off, as
    /// in Word (MS-OI29500 §17.7.6.5).
    pub col_band_size: Option<u32>,
    pub first_col_chr: CharProps,
    pub total_chr: CharProps,
    pub total_border_top: Option<Border>,
    /// Shading and cell borders of the first column (`firstCol` region).
    pub first_col_fill: Option<Rgb>,
    pub first_col_borders: Option<Borders>,
    /// The last column (`lastCol` region).
    pub last_col_fill: Option<Rgb>,
    pub last_col_chr: CharProps,
    pub last_col_borders: Option<Borders>,
    /// Shading and cell borders of the total (last) row (`lastRow` region); a top edge left
    /// unset falls back to `total_border_top`.
    pub total_fill: Option<Rgb>,
    pub total_borders: Option<Borders>,
    /// The odd column bands (`band1Vert` region).
    pub col_band_fill: Option<Rgb>,
    pub col_band_chr: CharProps,
    pub col_band_borders: Option<Borders>,
    /// Per-part formatting the named fields above don't carry: every field of the parts without
    /// one (even bands, corner cells), and the cell text alignment of every part. Read and write
    /// parts through [`TableStyleParts::part`] / [`TableStyleParts::set_part`].
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub regions: BTreeMap<TablePart, PartFormat>,
}

/// A conditional formatting part of a table style (`w:tblStylePr/@w:type`, ECMA-376 §17.7.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TablePart {
    WholeTable,
    FirstRow,
    LastRow,
    FirstCol,
    LastCol,
    Band1Vert,
    Band2Vert,
    Band1Horz,
    Band2Horz,
    NeCell,
    NwCell,
    SeCell,
    SwCell,
}

impl TablePart {
    /// Every part, lowest precedence first (ECMA-376 §17.7.6: whole table, bands, first/last
    /// column, first/last row, corner cells).
    pub const ALL: [TablePart; 13] = [
        TablePart::WholeTable,
        TablePart::Band1Vert,
        TablePart::Band2Vert,
        TablePart::Band1Horz,
        TablePart::Band2Horz,
        TablePart::FirstCol,
        TablePart::LastCol,
        TablePart::FirstRow,
        TablePart::LastRow,
        TablePart::NeCell,
        TablePart::NwCell,
        TablePart::SeCell,
        TablePart::SwCell,
    ];
    /// The `w:type` value.
    pub fn ooxml(self) -> &'static str {
        match self {
            TablePart::WholeTable => "wholeTable",
            TablePart::FirstRow => "firstRow",
            TablePart::LastRow => "lastRow",
            TablePart::FirstCol => "firstCol",
            TablePart::LastCol => "lastCol",
            TablePart::Band1Vert => "band1Vert",
            TablePart::Band2Vert => "band2Vert",
            TablePart::Band1Horz => "band1Horz",
            TablePart::Band2Horz => "band2Horz",
            TablePart::NeCell => "neCell",
            TablePart::NwCell => "nwCell",
            TablePart::SeCell => "seCell",
            TablePart::SwCell => "swCell",
        }
    }
    pub fn from_ooxml(s: &str) -> Option<TablePart> {
        TablePart::ALL.into_iter().find(|p| p.ooxml() == s)
    }
    /// A part spanning whole rows (its left/right edges are the table's sides, `inside_v` the
    /// edges between its cells).
    pub fn is_row(self) -> bool {
        matches!(self, TablePart::FirstRow | TablePart::LastRow | TablePart::Band1Horz | TablePart::Band2Horz)
    }
    /// A part spanning whole columns (its top/bottom edges are the table's, `between` the edges
    /// between its cells).
    pub fn is_column(self) -> bool {
        matches!(self, TablePart::FirstCol | TablePart::LastCol | TablePart::Band1Vert | TablePart::Band2Vert)
    }
    fn bit(self) -> u16 {
        1 << (TablePart::ALL.iter().position(|p| *p == self).unwrap_or(0) as u16)
    }
}

/// The table style parts one cell is in (always the whole table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PartSet(u16);

impl PartSet {
    /// The parts of the cell at row `ri` of `nrows`, grid column `g` spanning `span` of `ncols`,
    /// in a table with options `look`, row bands of `row_size` rows and column bands of
    /// `col_size` columns (a size of zero turns those bands off, as in Word).
    pub fn of_cell(
        look: &TableLook,
        (row_size, col_size): (u32, u32),
        (ri, nrows): (usize, usize),
        (g, span, ncols): (usize, usize, usize),
    ) -> PartSet {
        let header = look.header_row && ri == 0;
        let total = look.total_row && ri + 1 == nrows && nrows > 1;
        let first_col = look.first_column && g == 0;
        let last_col = look.last_column && g.saturating_add(span) >= ncols && ncols > 1;
        let rows = row_size.min(1000) as usize;
        let cols = col_size.min(1000) as usize;
        let row_band = look.banded_rows && !header && rows > 0;
        let odd_row = (ri.saturating_sub(usize::from(look.header_row)) / rows.max(1)).is_multiple_of(2);
        let col_band = look.banded_columns && !first_col && cols > 0;
        let odd_col = (g.saturating_sub(usize::from(look.first_column)) / cols.max(1)).is_multiple_of(2);
        let mut bits = TablePart::WholeTable.bit();
        for (on, p) in [
            (col_band && odd_col, TablePart::Band1Vert),
            (col_band && !odd_col, TablePart::Band2Vert),
            (row_band && odd_row, TablePart::Band1Horz),
            (row_band && !odd_row, TablePart::Band2Horz),
            (first_col, TablePart::FirstCol),
            (last_col, TablePart::LastCol),
            (header, TablePart::FirstRow),
            (total, TablePart::LastRow),
            (header && last_col, TablePart::NeCell),
            (header && first_col, TablePart::NwCell),
            (total && last_col, TablePart::SeCell),
            (total && first_col, TablePart::SwCell),
        ] {
            if on {
                bits |= p.bit();
            }
        }
        PartSet(bits)
    }
    pub fn contains(self, p: TablePart) -> bool {
        self.0 & p.bit() != 0
    }
    /// The parts, lowest precedence first.
    pub fn iter(self) -> impl Iterator<Item = TablePart> {
        TablePart::ALL.into_iter().filter(move |p| self.contains(*p))
    }
}

/// What one table style part gives its cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PartFormat {
    pub chr: CharProps,
    /// Cell shading.
    pub fill: Option<Rgb>,
    /// Cell borders (the whole table's are the table borders).
    pub borders: Option<Borders>,
    /// Horizontal alignment of the cells' paragraphs (`w:pPr/w:jc`).
    pub align: Option<Align>,
    /// Vertical alignment in the cells (`w:tcPr/w:vAlign`).
    pub v_align: Option<VAlign>,
    /// Cell-level Fit Text (`w:tcFitText`); unset inherits.
    pub fit_text: Option<bool>,
}

impl PartFormat {
    pub fn is_empty(&self) -> bool {
        *self == PartFormat::default()
    }
    /// What `patch` sets wins; borders merge edge by edge.
    pub fn overlay(&mut self, patch: &PartFormat) {
        self.chr.overlay(&patch.chr);
        self.fill = patch.fill.or(self.fill);
        self.borders = match (self.borders, patch.borders) {
            (Some(mut b), Some(p)) => {
                b.overlay(&p);
                Some(b)
            }
            (b, p) => p.or(b),
        };
        self.align = patch.align.or(self.align);
        self.v_align = patch.v_align.or(self.v_align);
        self.fit_text = patch.fit_text.or(self.fit_text);
    }
}

impl TableStyleParts {
    /// Apply `patch` (a style further down a based-on chain) over these parts: what it sets wins,
    /// the rest is inherited; borders merge edge by edge.
    pub fn overlay(&mut self, patch: &TableStyleParts) {
        let merge = |b: Option<Borders>, p: Option<Borders>| match (b, p) {
            (Some(mut b), Some(p)) => {
                b.overlay(&p);
                Some(b)
            }
            (b, p) => p.or(b),
        };
        self.borders = merge(self.borders, patch.borders);
        self.header_borders = merge(self.header_borders, patch.header_borders);
        self.band_borders = merge(self.band_borders, patch.band_borders);
        self.band_size = patch.band_size.or(self.band_size);
        self.col_band_size = patch.col_band_size.or(self.col_band_size);
        self.band_chr.overlay(&patch.band_chr);
        self.fill = patch.fill.or(self.fill);
        self.cell_margins = patch.cell_margins.or(self.cell_margins);
        self.header_fill = patch.header_fill.or(self.header_fill);
        self.header_chr.overlay(&patch.header_chr);
        self.band_fill = patch.band_fill.or(self.band_fill);
        self.first_col_chr.overlay(&patch.first_col_chr);
        self.total_chr.overlay(&patch.total_chr);
        self.total_border_top = patch.total_border_top.or(self.total_border_top);
        self.first_col_fill = patch.first_col_fill.or(self.first_col_fill);
        self.first_col_borders = merge(self.first_col_borders, patch.first_col_borders);
        self.last_col_fill = patch.last_col_fill.or(self.last_col_fill);
        self.last_col_chr.overlay(&patch.last_col_chr);
        self.last_col_borders = merge(self.last_col_borders, patch.last_col_borders);
        self.total_fill = patch.total_fill.or(self.total_fill);
        self.total_borders = merge(self.total_borders, patch.total_borders);
        self.col_band_fill = patch.col_band_fill.or(self.col_band_fill);
        self.col_band_chr.overlay(&patch.col_band_chr);
        self.col_band_borders = merge(self.col_band_borders, patch.col_band_borders);
        for (k, f) in &patch.regions {
            self.regions.entry(*k).or_default().overlay(f);
        }
    }

    /// Part `p`'s formatting. The whole table's text formatting is the style's own
    /// ([`Style::table_part`] adds it); its borders are the table borders.
    pub fn part(&self, p: TablePart) -> PartFormat {
        let extra = self.regions.get(&p).cloned().unwrap_or_default();
        let named = |chr: &CharProps, fill: Option<Rgb>, borders: Option<Borders>| PartFormat {
            chr: chr.clone(),
            fill,
            borders,
            align: extra.align,
            v_align: extra.v_align,
            fit_text: extra.fit_text,
        };
        match p {
            TablePart::WholeTable => PartFormat {
                chr: CharProps::default(),
                fill: self.fill,
                borders: self.borders,
                align: None,
                v_align: extra.v_align,
                fit_text: extra.fit_text,
            },
            TablePart::FirstRow => named(&self.header_chr, self.header_fill, self.header_borders),
            TablePart::LastRow => {
                // The built-in top rule is the region's top edge unless its borders set one.
                let borders = match (self.total_borders, self.total_border_top) {
                    (Some(mut b), top) => {
                        b.top = b.top.or(top);
                        Some(b)
                    }
                    (None, top) => top.map(|t| Borders { top: Some(t), ..Borders::default() }),
                };
                named(&self.total_chr, self.total_fill, borders)
            }
            TablePart::FirstCol => named(&self.first_col_chr, self.first_col_fill, self.first_col_borders),
            TablePart::LastCol => named(&self.last_col_chr, self.last_col_fill, self.last_col_borders),
            TablePart::Band1Vert => named(&self.col_band_chr, self.col_band_fill, self.col_band_borders),
            TablePart::Band1Horz => named(&self.band_chr, self.band_fill, self.band_borders),
            _ => extra,
        }
    }

    /// Replace part `p`'s formatting (for the whole table only its fill, borders and vertical
    /// alignment: [`Style::set_table_part`] stores the rest).
    pub fn set_part(&mut self, p: TablePart, f: PartFormat) {
        let align_only = PartFormat { align: f.align, v_align: f.v_align, fit_text: f.fit_text, ..PartFormat::default() };
        let PartFormat { chr, fill, borders, .. } = f.clone();
        let rest = match p {
            TablePart::WholeTable => {
                self.fill = fill;
                self.borders = borders;
                PartFormat { v_align: f.v_align, fit_text: f.fit_text, ..PartFormat::default() }
            }
            TablePart::FirstRow => {
                (self.header_chr, self.header_fill, self.header_borders) = (chr, fill, borders);
                align_only
            }
            TablePart::LastRow => {
                (self.total_chr, self.total_fill) = (chr, fill);
                self.total_border_top = borders.and_then(|b| b.top);
                self.total_borders = borders.map(|b| Borders { top: None, ..b }).filter(|b| *b != Borders::default());
                align_only
            }
            TablePart::FirstCol => {
                (self.first_col_chr, self.first_col_fill, self.first_col_borders) = (chr, fill, borders);
                align_only
            }
            TablePart::LastCol => {
                (self.last_col_chr, self.last_col_fill, self.last_col_borders) = (chr, fill, borders);
                align_only
            }
            TablePart::Band1Vert => {
                (self.col_band_chr, self.col_band_fill, self.col_band_borders) = (chr, fill, borders);
                align_only
            }
            TablePart::Band1Horz => {
                (self.band_chr, self.band_fill, self.band_borders) = (chr, fill, borders);
                align_only
            }
            _ => f,
        };
        if rest.is_empty() {
            self.regions.remove(&p);
        } else {
            self.regions.insert(p, rest);
        }
    }
}

impl Style {
    /// Table style part `p` as this style alone sets it (the whole table's text formatting and
    /// paragraph alignment are the style's own).
    pub fn table_part(&self, p: TablePart) -> PartFormat {
        let mut f = self.table.as_ref().map(|t| t.part(p)).unwrap_or_default();
        if p == TablePart::WholeTable {
            f.chr = self.chr.clone();
            f.align = self.para.align;
        }
        f
    }
    /// Replace part `p` of this table style.
    pub fn set_table_part(&mut self, p: TablePart, f: PartFormat) {
        if p == TablePart::WholeTable {
            self.chr = f.chr.clone();
            self.para.align = f.align;
        }
        let mut t = self.table.take().unwrap_or_default();
        t.set_part(p, f);
        self.table = (t != TableStyleParts::default()).then_some(t);
    }
}

impl TableStyleProps {
    /// Part `p` with the based-on chain merged (see [`Style::table_part`]).
    pub fn part(&self, p: TablePart) -> PartFormat {
        let mut f = self.parts.part(p);
        if p == TablePart::WholeTable {
            f.chr = self.chr.clone();
            f.align = self.para.align;
        }
        f
    }
}

/// A table style with its based-on chain merged: what it gives the text in the table's cells
/// (`para`, `chr`) and the table itself (`parts`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TableStyleProps {
    pub para: ParaProps,
    pub chr: CharProps,
    pub parts: TableStyleParts,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StyleSheet {
    /// Document defaults (`w:docDefaults`).
    pub default_chr: CharProps,
    pub default_para: ParaProps,
    /// Preserve an explicit `w:pPrDefault`, even when empty. Omission leaves defaults to
    /// the application; nonempty `default_para` is always written after spacing edits.
    pub default_para_present: bool,
    pub styles: Vec<Style>,
    /// The file's `w:latentStyles` (behaviour of built-in styles not defined in the document:
    /// gallery, priority, visibility), as XML, written back as is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latent_styles: Option<String>,
}

impl Default for StyleSheet {
    fn default() -> Self {
        StyleSheet::builtin()
    }
}

/// The default heading colour.
pub const HEADING_BLUE: Rgb = Rgb(0x0F, 0x47, 0x61);
/// Default body/heading fonts (theme minor/major). Substituted by layout when not installed.
pub const BODY_FONT: &str = "Aptos";
pub const HEADING_FONT: &str = "Aptos Display";

fn chr() -> CharProps {
    CharProps::default()
}
fn para() -> ParaProps {
    ParaProps::default()
}
fn color(c: Rgb) -> Option<TextColor> {
    Some(TextColor::Rgb(c))
}

impl StyleSheet {
    pub fn empty() -> Self {
        StyleSheet {
            default_chr: CharProps::default(),
            default_para: ParaProps::default(),
            default_para_present: true,
            styles: Vec::new(),
            latent_styles: None,
        }
    }

    /// The built-in style set used for new documents.
    pub fn builtin() -> Self {
        let mut s = StyleSheet {
            default_chr: CharProps { font: Some(BODY_FONT.into()), size: Some(12.0), kern: Some(1.0), lang: Some("en-US".into()), ..chr() },
            default_para: ParaProps {
                space_after: Some(8.0),
                space_before: Some(0.0),
                line_spacing: Some(LineSpacing::Multiple(1.15)),
                widow_control: Some(true),
                ..para()
            },
            default_para_present: true,
            styles: Vec::new(),
            latent_styles: None,
        };
        let mut add = |st: Style| s.styles.push(st);
        add(Style { id: "Normal".into(), name: "Normal".into(), priority: Some(0), quick: true, ..p_style() });
        add(Style {
            id: "DefaultParagraphFont".into(),
            name: "Default Paragraph Font".into(),
            kind: StyleKind::Character,
            priority: Some(1),
            hidden: true,
            builtin: true,
            ..Default::default()
        });
        add(Style {
            id: "NoSpacing".into(),
            name: "No Spacing".into(),
            based_on: Some("Normal".into()),
            para: ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..para() },
            priority: Some(1),
            quick: true,
            ..p_style()
        });
        let heads: [(f32, f32, f32, Option<bool>, Rgb, &str); 9] = [
            (20.0, 18.0, 4.0, None, HEADING_BLUE, HEADING_FONT),
            (16.0, 8.0, 4.0, None, HEADING_BLUE, HEADING_FONT),
            (14.0, 8.0, 4.0, None, HEADING_BLUE, BODY_FONT),
            (12.0, 4.0, 2.0, Some(true), HEADING_BLUE, BODY_FONT),
            (12.0, 4.0, 2.0, None, HEADING_BLUE, BODY_FONT),
            (12.0, 2.0, 0.0, Some(true), Rgb(0x59, 0x59, 0x59), BODY_FONT),
            (12.0, 2.0, 0.0, None, Rgb(0x59, 0x59, 0x59), BODY_FONT),
            (12.0, 0.0, 0.0, Some(true), Rgb(0x27, 0x27, 0x27), BODY_FONT),
            (12.0, 0.0, 0.0, None, Rgb(0x27, 0x27, 0x27), BODY_FONT),
        ];
        for (i, (size, before, after, italic, c, font)) in heads.into_iter().enumerate() {
            let n = i + 1;
            add(Style {
                id: format!("Heading{n}"),
                name: format!("Heading {n}"),
                based_on: Some("Normal".into()),
                next: Some("Normal".into()),
                linked: Some(format!("Heading{n}Char")),
                para: ParaProps {
                    space_before: Some(before),
                    space_after: Some(after),
                    keep_next: Some(true),
                    keep_lines: Some(true),
                    outline_level: Some(i as u8),
                    ..para()
                },
                chr: CharProps { font: Some(font.into()), size: Some(size), italic, color: color(c), ..chr() },
                priority: Some(9 + n as u32),
                quick: n <= 3,
                ..p_style()
            });
        }
        add(Style {
            id: "Title".into(),
            name: "Title".into(),
            based_on: Some("Normal".into()),
            next: Some("Normal".into()),
            para: ParaProps { space_after: Some(4.0), contextual_spacing: Some(true), line_spacing: Some(LineSpacing::Multiple(1.0)), ..para() },
            chr: CharProps { font: Some(HEADING_FONT.into()), size: Some(28.0), spacing: Some(-0.5), kern: Some(14.0), ..chr() },
            priority: Some(10),
            quick: true,
            ..p_style()
        });
        add(Style {
            id: "Subtitle".into(),
            name: "Subtitle".into(),
            based_on: Some("Normal".into()),
            next: Some("Normal".into()),
            chr: CharProps { size: Some(14.0), spacing: Some(0.75), color: color(Rgb(0x59, 0x59, 0x59)), ..chr() },
            priority: Some(11),
            quick: true,
            ..p_style()
        });
        let c_style = |id: &str, name: &str, prio: u32, c: CharProps| Style {
            id: id.into(),
            name: name.into(),
            kind: StyleKind::Character,
            based_on: Some("DefaultParagraphFont".into()),
            chr: c,
            priority: Some(prio),
            quick: true,
            builtin: true,
            ..Default::default()
        };
        add(c_style("SubtleEmphasis", "Subtle Emphasis", 19, CharProps { italic: Some(true), color: color(Rgb(0x40, 0x40, 0x40)), ..chr() }));
        add(c_style("Emphasis", "Emphasis", 20, CharProps { italic: Some(true), ..chr() }));
        add(c_style("IntenseEmphasis", "Intense Emphasis", 21, CharProps { italic: Some(true), color: color(HEADING_BLUE), ..chr() }));
        add(c_style("Strong", "Strong", 22, CharProps { bold: Some(true), ..chr() }));
        add(Style {
            id: "Quote".into(),
            name: "Quote".into(),
            based_on: Some("Normal".into()),
            next: Some("Normal".into()),
            para: ParaProps { align: Some(Align::Center), space_before: Some(8.0), ..para() },
            chr: CharProps { italic: Some(true), color: color(Rgb(0x40, 0x40, 0x40)), ..chr() },
            priority: Some(29),
            quick: true,
            ..p_style()
        });
        let blue_line = Border { style: BorderStyle::Single, width: 0.5, color: Some(HEADING_BLUE), space: 10.0 };
        add(Style {
            id: "IntenseQuote".into(),
            name: "Intense Quote".into(),
            based_on: Some("Normal".into()),
            next: Some("Normal".into()),
            para: ParaProps {
                align: Some(Align::Center),
                space_before: Some(18.0),
                space_after: Some(18.0),
                indent_left: Some(43.2),
                indent_right: Some(43.2),
                borders: Some(Borders { top: Some(blue_line), bottom: Some(blue_line), ..Default::default() }),
                ..para()
            },
            chr: CharProps { italic: Some(true), color: color(HEADING_BLUE), ..chr() },
            priority: Some(30),
            quick: true,
            ..p_style()
        });
        add(c_style("SubtleReference", "Subtle Reference", 31, CharProps { small_caps: Some(true), color: color(Rgb(0x5A, 0x5A, 0x5A)), ..chr() }));
        add(c_style(
            "IntenseReference",
            "Intense Reference",
            32,
            CharProps { bold: Some(true), small_caps: Some(true), spacing: Some(0.25), color: color(HEADING_BLUE), ..chr() },
        ));
        add(c_style("BookTitle", "Book Title", 33, CharProps { bold: Some(true), italic: Some(true), spacing: Some(0.25), ..chr() }));
        add(Style {
            id: "ListParagraph".into(),
            name: "List Paragraph".into(),
            based_on: Some("Normal".into()),
            para: ParaProps { indent_left: Some(36.0), contextual_spacing: Some(true), ..para() },
            priority: Some(34),
            quick: true,
            ..p_style()
        });
        let mut hl =
            c_style("Hyperlink", "Hyperlink", 99, CharProps { color: color(Rgb(0x46, 0x78, 0x86)), underline: Some(Underline::Single), ..chr() });
        hl.quick = false;
        add(hl);
        add(Style {
            id: "Caption".into(),
            name: "Caption".into(),
            based_on: Some("Normal".into()),
            next: Some("Normal".into()),
            para: ParaProps { space_after: Some(10.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..para() },
            chr: CharProps { italic: Some(true), size: Some(9.0), color: color(Rgb(0x0E, 0x28, 0x41)), ..chr() },
            priority: Some(35),
            ..p_style()
        });
        for (id, name) in [("Header", "Header"), ("Footer", "Footer")] {
            add(Style {
                id: id.into(),
                name: name.into(),
                based_on: Some("Normal".into()),
                para: ParaProps {
                    space_after: Some(0.0),
                    line_spacing: Some(LineSpacing::Multiple(1.0)),
                    tabs: Some(vec![
                        TabStop { pos: 234.0, align: TabAlign::Center, leader: TabLeader::None },
                        TabStop { pos: 468.0, align: TabAlign::Right, leader: TabLeader::None },
                    ]),
                    ..para()
                },
                priority: Some(99),
                ..p_style()
            });
        }
        add(Style {
            id: "FootnoteText".into(),
            name: "Footnote Text".into(),
            based_on: Some("Normal".into()),
            para: ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..para() },
            chr: CharProps { size: Some(10.0), ..chr() },
            priority: Some(99),
            ..p_style()
        });
        let mut fr =
            c_style("FootnoteReference", "Footnote Reference", 99, CharProps { vert_align: Some(crate::props::VertAlign::Superscript), ..chr() });
        fr.quick = false;
        add(fr);
        add(Style {
            id: "EndnoteText".into(),
            name: "Endnote Text".into(),
            based_on: Some("Normal".into()),
            para: ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..para() },
            chr: CharProps { size: Some(10.0), ..chr() },
            priority: Some(99),
            ..p_style()
        });
        let mut er =
            c_style("EndnoteReference", "Endnote Reference", 99, CharProps { vert_align: Some(crate::props::VertAlign::Superscript), ..chr() });
        er.quick = false;
        add(er);
        add(Style {
            id: "TOCHeading".into(),
            name: "TOC Heading".into(),
            based_on: Some("Heading1".into()),
            next: Some("Normal".into()),
            para: ParaProps { outline_level: Some(9), space_before: Some(12.0), ..para() },
            priority: Some(39),
            ..p_style()
        });
        for n in 1..=9u32 {
            add(Style {
                id: format!("TOC{n}"),
                name: format!("TOC {n}"),
                based_on: Some("Normal".into()),
                next: Some("Normal".into()),
                para: ParaProps { space_after: Some(5.0), indent_left: Some(12.0 * (n - 1) as f32), ..para() },
                priority: Some(39),
                ..p_style()
            });
        }
        add(Style {
            id: "TableGrid".into(),
            name: "Table Grid".into(),
            kind: StyleKind::Table,
            para: ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..para() },
            priority: Some(59),
            builtin: true,
            table: Some(TableStyleParts { borders: Some(Borders::all(Border::single(0.5))), ..Default::default() }),
            ..Default::default()
        });
        s.styles.extend(table_styles());
        s
    }

    pub fn get(&self, id: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.id == id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Style> {
        self.styles.iter_mut().find(|s| s.id == id)
    }
    /// Find by display name (case-insensitive) or id.
    pub fn find(&self, name_or_id: &str) -> Option<&Style> {
        self.get(name_or_id).or_else(|| self.styles.iter().find(|s| s.name.eq_ignore_ascii_case(name_or_id)))
    }
    /// The Quick Styles gallery, in priority order.
    pub fn gallery(&self) -> Vec<&Style> {
        let mut v: Vec<&Style> = self.styles.iter().filter(|s| s.quick && !s.hidden && s.kind != StyleKind::Table).collect();
        v.sort_by_key(|s| (s.priority.unwrap_or(99), s.name.clone()));
        v
    }
    /// The `based_on` chain from the root down to `id` (bounded, cycle-safe).
    pub fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut cur = self.get(id);
        while let Some(s) = cur {
            if out.len() >= 32 || out.iter().any(|o: &&Style| o.id == s.id) {
                break;
            }
            out.push(s);
            cur = s.based_on.as_deref().and_then(|b| self.get(b));
        }
        out.reverse();
        out
    }
    /// Which indents (left, first line) of a paragraph that gets its list from style `id` the style
    /// chain sets over the list level's own. Word applies the level's indents on top of the most
    /// derived style that gives a list, so that style and the ones based on it win over the level,
    /// and the styles it is based on don't (Heading 3 with its own left indent on Heading 2's
    /// list). `(false, false)` when no style in the chain gives a list.
    pub fn indents_over_list(&self, id: &str) -> (bool, bool) {
        let chain = self.chain(id);
        let Some(k) = chain.iter().rposition(|s| s.para.numbering.is_some()) else { return (false, false) };
        let set = |v: Option<f32>, chars: Option<f32>| v.is_some() || chars.is_some_and(|c| c != 0.0);
        chain.get(k..).unwrap_or_default().iter().fold((false, false), |(l, f), s| {
            (l || set(s.para.indent_left, s.para.indent_left_chars), f || set(s.para.indent_first, s.para.indent_first_chars))
        })
    }
    /// Table style `id` with its based-on chain merged, base styles first (`None` when there is no
    /// such table style). Styles of other kinds in the chain are skipped.
    pub fn table_style(&self, id: &str) -> Option<TableStyleProps> {
        let chain: Vec<&Style> = self.chain(id).into_iter().filter(|s| s.kind == StyleKind::Table).collect();
        if chain.last().is_none_or(|s| s.id != id) {
            return None;
        }
        let mut out = TableStyleProps::default();
        for s in chain {
            out.para.overlay(&s.para);
            out.chr.overlay(&s.chr);
            if let Some(t) = &s.table {
                out.parts.overlay(t);
            }
        }
        out.para.style = None;
        out.chr.style = None;
        Some(out)
    }
    /// Add or replace a style.
    pub fn upsert(&mut self, st: Style) {
        match self.styles.iter_mut().find(|s| s.id == st.id) {
            Some(s) => *s = st,
            None => self.styles.push(st),
        }
    }
    /// For a file from a producer that never writes `w:qFormat` (LibreOffice): give it Word's
    /// usual Quick Styles gallery. Each built-in style whose id or name matches a quick style of
    /// [`StyleSheet::builtin`] becomes quick (taking the built-in's priority when it has none), and
    /// the quick built-ins the file lacks are added with their linked styles. Existing ids and
    /// names are never overwritten. Callers use it only when no style in the file is quick.
    pub fn add_default_gallery(&mut self) {
        let builtin = StyleSheet::builtin();
        for q in builtin.styles.iter().filter(|s| s.quick && s.kind != StyleKind::Table) {
            let found =
                self.styles.iter().position(|s| s.id == q.id).or_else(|| self.styles.iter().position(|s| s.name.eq_ignore_ascii_case(&q.name)));
            if let Some(st) = found.and_then(|i| self.styles.get_mut(i)) {
                if st.builtin && st.kind == q.kind {
                    st.quick = true;
                    st.priority = st.priority.or(q.priority);
                }
                continue;
            }
            for sid in std::iter::once(q.id.as_str()).chain(q.linked.as_deref()) {
                if let Some(b) = builtin.get(sid)
                    && self.get(sid).is_none()
                    && self.find(&b.name).is_none()
                {
                    self.styles.push(b.clone());
                }
            }
        }
    }
    /// Remove style `id` and return it. Styles based on it are rebased onto its own base, and
    /// `next` / `linked` references to it are dropped. Text that uses it is the document's to
    /// retarget ([`crate::Document::restyle`]).
    pub fn remove(&mut self, id: &str) -> Option<Style> {
        let i = self.styles.iter().position(|s| s.id == id)?;
        let gone = self.styles.remove(i);
        for s in &mut self.styles {
            if s.based_on.as_deref() == Some(id) {
                s.based_on = gone.based_on.clone().filter(|b| *b != s.id);
            }
            if s.next.as_deref() == Some(id) {
                s.next = None;
            }
            if s.linked.as_deref() == Some(id) {
                s.linked = None;
            }
        }
        Some(gone)
    }
    /// An id not used yet, derived from a display name.
    pub fn new_id(&self, name: &str) -> String {
        let base: String = name.chars().filter(|c| c.is_alphanumeric()).collect();
        let base = if base.is_empty() { "Style".to_string() } else { base };
        if self.get(&base).is_none() {
            return base;
        }
        (1..10_000).map(|i| format!("{base}{i}")).find(|id| self.get(id).is_none()).unwrap_or(base)
    }
}

fn p_style() -> Style {
    Style { kind: StyleKind::Paragraph, builtin: true, ..Default::default() }
}

/// A few table styles of our own (grid, plain, light/medium accents).
fn table_styles() -> Vec<Style> {
    let thin = |c: Rgb| Border { style: BorderStyle::Single, width: 0.5, color: Some(c), space: 0.0 };
    // Bands of one row and one column, written out so Word bands them too (#3415).
    let mk = |id: &str, name: &str, parts: TableStyleParts| Style {
        id: id.into(),
        name: name.into(),
        kind: StyleKind::Table,
        based_on: Some("TableNormal".into()),
        para: ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..ParaProps::default() },
        priority: Some(60),
        builtin: true,
        table: Some(TableStyleParts { band_size: Some(1), col_band_size: Some(1), ..parts }),
        ..Default::default()
    };
    let accents = [
        ("Blue", Rgb(0x15, 0x60, 0x82), Rgb(0xC1, 0xE4, 0xF5)),
        ("Orange", Rgb(0xE9, 0x71, 0x32), Rgb(0xFB, 0xE2, 0xD5)),
        ("Green", Rgb(0x19, 0x6B, 0x24), Rgb(0xC1, 0xF0, 0xC8)),
        ("Purple", Rgb(0x71, 0x2E, 0x8D), Rgb(0xE5, 0xD2, 0xEE)),
    ];
    let mut v = vec![
        Style { id: "TableNormal".into(), name: "Normal Table".into(), kind: StyleKind::Table, hidden: true, builtin: true, ..Default::default() },
        mk(
            "PlainTable1",
            "Plain Table 1",
            TableStyleParts {
                borders: Some(Borders::all(thin(Rgb(0xBF, 0xBF, 0xBF)))),
                header_chr: CharProps { bold: Some(true), ..CharProps::default() },
                band_fill: Some(Rgb(0xF2, 0xF2, 0xF2)),
                first_col_chr: CharProps { bold: Some(true), ..CharProps::default() },
                ..Default::default()
            },
        ),
        mk(
            "GridTable1Light",
            "Grid Table 1 Light",
            TableStyleParts {
                borders: Some(Borders::all(thin(Rgb(0x99, 0x99, 0x99)))),
                header_chr: CharProps { bold: Some(true), ..CharProps::default() },
                first_col_chr: CharProps { bold: Some(true), ..CharProps::default() },
                ..Default::default()
            },
        ),
    ];
    for (name, dark, light) in accents {
        v.push(mk(
            &format!("GridTable4Accent{name}"),
            &format!("Grid Table 4 – {name}"),
            TableStyleParts {
                borders: Some(Borders::all(thin(light))),
                header_fill: Some(dark),
                header_chr: CharProps { bold: Some(true), color: Some(TextColor::Rgb(Rgb::WHITE)), ..CharProps::default() },
                band_fill: Some(light),
                first_col_chr: CharProps { bold: Some(true), ..CharProps::default() },
                ..Default::default()
            },
        ));
        v.push(mk(
            &format!("ListTable3Accent{name}"),
            &format!("List Table 3 – {name}"),
            TableStyleParts {
                borders: Some(Borders::box_(thin(dark))),
                header_fill: Some(dark),
                header_chr: CharProps { bold: Some(true), color: Some(TextColor::Rgb(Rgb::WHITE)), ..CharProps::default() },
                total_border_top: Some(thin(dark)),
                ..Default::default()
            },
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_present() {
        let s = StyleSheet::builtin();
        for id in ["Normal", "Heading1", "Heading9", "Title", "Strong", "TableGrid", "TOC3", "Hyperlink", "ListParagraph"] {
            assert!(s.get(id).is_some(), "{id}");
        }
        assert_eq!(s.find("heading 2").map(|s| s.id.as_str()), Some("Heading2"));
        let g = s.gallery();
        assert_eq!(g.first().map(|s| s.id.as_str()), Some("Normal"));
        assert!(g.iter().any(|s| s.id == "Heading1"));
    }

    #[test]
    fn default_gallery_marks_and_adds_quick_builtins() {
        let b = StyleSheet::builtin();
        let mut s = StyleSheet::empty();
        let plain = |id: &str, name: &str| Style { id: id.into(), name: name.into(), builtin: true, ..Default::default() };
        s.styles.push(plain("Normal", "Normal"));
        s.styles.push(plain("Heading1", "heading 1"));
        s.styles.push(Style { priority: Some(7), ..plain("Heading2", "Heading 2") });
        s.styles.push(Style { builtin: false, ..plain("Heading3", "Heading 3") });
        s.styles.push(Style { para: ParaProps { space_after: Some(99.0), ..Default::default() }, ..plain("Custom1", "Subtitle") });
        s.add_default_gallery();
        let ids: Vec<&str> = s.gallery().iter().map(|x| x.id.as_str()).collect();
        for id in ["Normal", "NoSpacing", "Heading1", "Heading2", "Title"] {
            assert!(ids.contains(&id), "{id} in {ids:?}");
        }
        let mut bare = StyleSheet::empty();
        bare.add_default_gallery();
        assert!(bare.get("Heading1").is_some_and(|x| x.quick) && bare.get("Title").is_some(), "missing quick builtins added");
        assert_eq!(s.get("Heading2").and_then(|x| x.priority), Some(7), "file priority kept");
        assert_eq!(s.get("Heading1").and_then(|x| x.priority), b.get("Heading1").and_then(|x| x.priority));
        assert!(!s.get("Heading3").is_some_and(|x| x.quick), "custom style untouched");
        assert!(s.get("Subtitle").is_none(), "a style with that name exists: not duplicated");
        assert!(s.get("Custom1").is_some_and(|x| x.quick), "found by name only, built in: quick");
        assert_eq!(s.get("Custom1").and_then(|x| x.para.space_after), Some(99.0));
    }

    #[test]
    fn older_serialized_styles_keep_explicit_defaults() {
        let mut value = serde_json::to_value(StyleSheet::empty()).unwrap();
        value.as_object_mut().unwrap().remove("defaultParaPresent");
        let sheet: StyleSheet = serde_json::from_value(value).unwrap();
        assert!(sheet.default_para_present);
        let mut omitted = sheet;
        omitted.default_para_present = false;
        let again: StyleSheet = serde_json::from_str(&serde_json::to_string(&omitted).unwrap()).unwrap();
        assert_eq!(again, omitted);
    }

    #[test]
    fn chain_is_cycle_safe() {
        let mut s = StyleSheet::builtin();
        if let Some(n) = s.get_mut("Normal") {
            n.based_on = Some("Heading1".into());
        }
        let c = s.chain("Heading1");
        assert!(c.len() <= 2);
    }

    /// #3415: a band size of zero (Word's reading of a style without one) turns those bands off.
    #[test]
    fn band_size_zero_turns_bands_off() {
        let look = crate::props::TableLook { banded_columns: true, ..Default::default() };
        let parts = |sizes, ri, g| PartSet::of_cell(&look, sizes, (ri, 5), (g, 1, 4));
        assert!(parts((1, 1), 1, 1).contains(TablePart::Band1Horz) && parts((1, 1), 1, 1).contains(TablePart::Band1Vert));
        assert!(parts((1, 1), 2, 2).contains(TablePart::Band2Horz) && parts((1, 1), 2, 2).contains(TablePart::Band2Vert));
        for (ri, g) in [(1, 1), (2, 2)] {
            let p = parts((0, 0), ri, g);
            for band in [TablePart::Band1Horz, TablePart::Band2Horz, TablePart::Band1Vert, TablePart::Band2Vert] {
                assert!(!p.contains(band), "{band:?} at row {ri}, column {g}");
            }
        }
        // Our banded styles say so, so Word bands them too.
        let s = StyleSheet::builtin();
        let t = s.table_style("GridTable4AccentBlue").map(|t| t.parts);
        assert_eq!(t.map(|t| (t.band_size, t.col_band_size)), Some((Some(1), Some(1))));
    }

    /// A table style based on another inherits what it doesn't set itself; borders merge edge by
    /// edge.
    #[test]
    fn table_style_merges_based_on_chain() {
        let mut s = StyleSheet::builtin();
        let red = Rgb(0xC0, 0, 0);
        let thick = Border::single(2.0);
        s.upsert(Style {
            id: "RedGrid".into(),
            kind: StyleKind::Table,
            based_on: Some("TableGrid".into()),
            chr: CharProps { bold: Some(true), color: color(red), ..chr() },
            table: Some(TableStyleParts {
                borders: Some(Borders { top: Some(thick), ..Default::default() }),
                header_fill: Some(red),
                ..Default::default()
            }),
            ..Default::default()
        });
        let t = s.table_style("RedGrid").unwrap();
        assert_eq!((t.para.space_after, t.para.line_spacing), (Some(0.0), Some(LineSpacing::Multiple(1.0))), "from Table Grid");
        assert_eq!((t.chr.bold, t.chr.color), (Some(true), color(red)));
        let b = t.parts.borders.unwrap();
        assert_eq!(b.top, Some(thick));
        assert_eq!(b.left, Some(Border::single(0.5)), "Table Grid's other edges");
        assert_eq!(t.parts.header_fill, Some(red));
        // Not a table style, unknown, or a based-on loop: no panic.
        assert!(s.table_style("Normal").is_none());
        assert!(s.table_style("Nope").is_none());
        if let Some(g) = s.get_mut("TableGrid") {
            g.based_on = Some("RedGrid".into());
        }
        assert!(s.table_style("RedGrid").is_some());
    }

    #[test]
    fn new_ids_unique() {
        let s = StyleSheet::builtin();
        assert_eq!(s.new_id("My Style"), "MyStyle");
        assert_eq!(s.new_id("Normal"), "Normal1");
        assert_eq!(s.new_id("!!!"), "Style");
    }
}
