//! Styles: paragraph, character, table and linked styles with `based on` chains, plus the
//! built-in style set (our own values in the spirit of a modern word processor's defaults).

use serde::{Deserialize, Serialize};

use crate::props::{Align, Border, BorderStyle, Borders, CharProps, LineSpacing, ParaProps, Rgb, TabAlign, TabLeader, TabStop, TextColor, Underline};

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
    pub band_fill: Option<Rgb>,
    pub first_col_chr: CharProps,
    pub total_chr: CharProps,
    pub total_border_top: Option<Border>,
}

impl TableStyleParts {
    /// Apply `patch` (a style further down a based-on chain) over these parts: what it sets wins,
    /// the rest is inherited; borders merge edge by edge.
    pub fn overlay(&mut self, patch: &TableStyleParts) {
        self.borders = match (self.borders, patch.borders) {
            (Some(b), Some(p)) => Some(Borders {
                top: p.top.or(b.top),
                left: p.left.or(b.left),
                bottom: p.bottom.or(b.bottom),
                right: p.right.or(b.right),
                between: p.between.or(b.between),
                inside_v: p.inside_v.or(b.inside_v),
            }),
            (b, p) => p.or(b),
        };
        self.fill = patch.fill.or(self.fill);
        self.cell_margins = patch.cell_margins.or(self.cell_margins);
        self.header_fill = patch.header_fill.or(self.header_fill);
        self.header_chr.overlay(&patch.header_chr);
        self.band_fill = patch.band_fill.or(self.band_fill);
        self.first_col_chr.overlay(&patch.first_col_chr);
        self.total_chr.overlay(&patch.total_chr);
        self.total_border_top = patch.total_border_top.or(self.total_border_top);
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
    pub styles: Vec<Style>,
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
        StyleSheet { default_chr: CharProps::default(), default_para: ParaProps::default(), styles: Vec::new() }
    }

    /// The built-in style set used for new documents.
    pub fn builtin() -> Self {
        let mut s = StyleSheet {
            default_chr: CharProps { font: Some(BODY_FONT.into()), size: Some(12.0), kern: Some(0.0), lang: Some("en-US".into()), ..chr() },
            default_para: ParaProps {
                space_after: Some(8.0),
                space_before: Some(0.0),
                line_spacing: Some(LineSpacing::Multiple(1.15)),
                widow_control: Some(true),
                ..para()
            },
            styles: Vec::new(),
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
    let mk = |id: &str, name: &str, parts: TableStyleParts| Style {
        id: id.into(),
        name: name.into(),
        kind: StyleKind::Table,
        based_on: Some("TableNormal".into()),
        para: ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..ParaProps::default() },
        priority: Some(60),
        builtin: true,
        table: Some(parts),
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
    fn chain_is_cycle_safe() {
        let mut s = StyleSheet::builtin();
        if let Some(n) = s.get_mut("Normal") {
            n.based_on = Some("Heading1".into());
        }
        let c = s.chain("Heading1");
        assert!(c.len() <= 2);
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
