//! Style resolution: document defaults → paragraph style chain → character style chain → direct
//! formatting, into concrete values for layout.

use serde::{Deserialize, Serialize};

use crate::props::{
    Align, Border, Borders, CharProps, Highlight, LineSpacing, NumRef, ParaProps, Rgb, TabAlign, TabStop, TextColor, Underline, VertAlign,
};
use crate::styles::{StyleKind, StyleSheet};

/// Concrete character formatting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedChar {
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub underline_color: Option<Rgb>,
    pub strike: bool,
    pub double_strike: bool,
    pub color: TextColor,
    pub highlight: Option<Rgb>,
    pub shading: Option<Rgb>,
    /// Visible character border only (`nil`/`none` resolve to `None`).
    pub border: Option<Border>,
    pub vert_align: VertAlign,
    pub caps: bool,
    pub small_caps: bool,
    pub hidden: bool,
    pub spacing: f32,
    pub scale: f32,
    pub position: f32,
    pub outline: bool,
    pub shadow: bool,
    pub emboss: bool,
    pub engrave: bool,
    pub link: Option<String>,
    pub ins: Option<u32>,
    pub del: Option<u32>,
    pub no_proof: bool,
    pub lang: Option<String>,
    /// Right-to-left run (`w:rtl`).
    pub rtl: bool,
    /// The whole run uses the complex-script properties (`w:cs`).
    pub cs: bool,
    /// Complex-script font, size, bold and italic (Arabic, Persian, Hebrew… text).
    pub font_cs: String,
    pub size_cs: f32,
    pub bold_cs: bool,
    pub italic_cs: bool,
}

impl ResolvedChar {
    fn from(c: &CharProps) -> ResolvedChar {
        ResolvedChar {
            font: c.font.clone().unwrap_or_else(|| crate::styles::BODY_FONT.to_string()),
            size: c.size.unwrap_or(11.0).clamp(1.0, 1638.0),
            bold: c.bold.unwrap_or(false),
            italic: c.italic.unwrap_or(false),
            underline: c.underline.unwrap_or_default(),
            underline_color: c.underline_color,
            strike: c.strike.unwrap_or(false),
            double_strike: c.double_strike.unwrap_or(false),
            color: c.color.unwrap_or_default(),
            highlight: c.highlight.and_then(Highlight::rgb),
            shading: c.shading,
            border: c.border.filter(Border::is_visible),
            vert_align: c.vert_align.unwrap_or_default(),
            caps: c.caps.unwrap_or(false),
            small_caps: c.small_caps.unwrap_or(false),
            hidden: c.hidden.unwrap_or(false),
            spacing: c.spacing.unwrap_or(0.0).clamp(-100.0, 100.0),
            scale: c.scale.unwrap_or(100.0).clamp(1.0, 600.0),
            position: c.position.unwrap_or(0.0).clamp(-800.0, 800.0),
            outline: c.outline.unwrap_or(false),
            shadow: c.shadow.unwrap_or(false),
            emboss: c.emboss.unwrap_or(false),
            engrave: c.engrave.unwrap_or(false),
            link: c.link.clone(),
            ins: c.ins,
            del: c.del,
            no_proof: c.no_proof.unwrap_or(false),
            lang: c.lang.clone(),
            rtl: c.rtl.unwrap_or(false),
            cs: c.cs.unwrap_or(false),
            font_cs: c.font_cs.clone().or_else(|| c.font.clone()).unwrap_or_else(|| crate::styles::BODY_FONT.to_string()),
            size_cs: c.size_cs.or(c.size).unwrap_or(11.0).clamp(1.0, 1638.0),
            bold_cs: c.bold_cs.or(c.bold).unwrap_or(false),
            italic_cs: c.italic_cs.or(c.italic).unwrap_or(false),
        }
    }
    /// The formatting complex-script text in this run is drawn with: the complex-script font,
    /// size, bold and italic in place of the others (Word formats Arabic, Persian and Hebrew
    /// characters, and every character of an `rtl`/`cs` run, this way).
    pub fn complex(&self) -> ResolvedChar {
        ResolvedChar { font: self.font_cs.clone(), size: self.size_cs, bold: self.bold_cs, italic: self.italic_cs, ..self.clone() }
    }
    /// Does a character of this run use the complex-script properties?
    pub fn uses_complex(&self, c: char) -> bool {
        self.rtl || self.cs || crate::bidi::is_complex_script(c)
    }
    /// The size glyphs are drawn at (super/subscript shrink to ~2/3).
    pub fn draw_size(&self) -> f32 {
        if self.vert_align == VertAlign::Baseline { self.size } else { self.size * 0.65 }
    }
    /// Baseline shift, points (positive = up).
    pub fn baseline_shift(&self) -> f32 {
        self.position
            + match self.vert_align {
                VertAlign::Baseline => 0.0,
                VertAlign::Superscript => self.size * 0.33,
                VertAlign::Subscript => -self.size * 0.14,
            }
    }
}

/// Concrete paragraph formatting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPara {
    pub style: String,
    pub align: Align,
    pub indent_left: f32,
    pub indent_right: f32,
    pub indent_first: f32,
    pub space_before: f32,
    pub space_after: f32,
    pub line_spacing: LineSpacing,
    pub contextual_spacing: bool,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub page_break_before: bool,
    pub widow_control: bool,
    pub outline_level: Option<u8>,
    pub numbering: Option<NumRef>,
    pub tabs: Vec<TabStop>,
    pub shading: Option<Rgb>,
    pub borders: Option<Borders>,
    pub bidi: bool,
    pub drop_cap: u8,
    pub suppress_hyphens: bool,
    /// Asian typography (see `ParaProps`): East Asian line-breaking rules.
    pub kinsoku: bool,
    /// Latin words wrap whole (`false`: they may break at any character).
    pub word_wrap: bool,
    pub overflow_punct: bool,
    pub top_line_punct: bool,
    pub auto_space_de: bool,
    pub auto_space_dn: bool,
}

const MAX_INDENT: f32 = 1584.0; // 22"

impl ResolvedPara {
    /// Visual (left, right) indents. Indents, like alignment, are logical: `indent_left` is the
    /// start (leading) edge, which is the right edge of a right-to-left paragraph (ISO 29500
    /// `w:ind/@w:start`, read from `w:left` too).
    pub fn visual_indents(&self) -> (f32, f32) {
        if self.bidi { (self.indent_right, self.indent_left) } else { (self.indent_left, self.indent_right) }
    }
}

impl ResolvedPara {
    fn from(p: &ParaProps, style: &str) -> ResolvedPara {
        let clampi = |v: Option<f32>| v.unwrap_or(0.0).clamp(-MAX_INDENT, MAX_INDENT);
        let mut tabs: Vec<TabStop> =
            p.tabs.clone().unwrap_or_default().into_iter().filter(|t| t.align != TabAlign::Clear && t.pos.is_finite()).collect();
        tabs.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        ResolvedPara {
            style: style.to_string(),
            align: p.align.unwrap_or_default(),
            indent_left: clampi(p.indent_left),
            indent_right: clampi(p.indent_right),
            indent_first: clampi(p.indent_first),
            space_before: p.space_before.unwrap_or(0.0).clamp(0.0, 1584.0),
            space_after: p.space_after.unwrap_or(0.0).clamp(0.0, 1584.0),
            line_spacing: match p.line_spacing.unwrap_or_default() {
                LineSpacing::Multiple(m) => LineSpacing::Multiple(if m.is_finite() { m.clamp(0.06, 132.0) } else { 1.0 }),
                LineSpacing::AtLeast(v) => LineSpacing::AtLeast(if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 }),
                LineSpacing::Exactly(v) => LineSpacing::Exactly(if v.is_finite() { v.clamp(0.7, 1584.0) } else { 12.0 }),
            },
            contextual_spacing: p.contextual_spacing.unwrap_or(false),
            keep_next: p.keep_next.unwrap_or(false),
            keep_lines: p.keep_lines.unwrap_or(false),
            page_break_before: p.page_break_before.unwrap_or(false),
            widow_control: p.widow_control.unwrap_or(true),
            outline_level: p.outline_level.filter(|l| *l < 9),
            numbering: p.numbering.filter(|n| n.num != 0),
            tabs,
            shading: p.shading,
            borders: p.borders.filter(Borders::any_visible),
            bidi: p.bidi.unwrap_or(false),
            drop_cap: p.drop_cap.unwrap_or(0).min(10),
            suppress_hyphens: p.suppress_hyphens.unwrap_or(false),
            kinsoku: p.kinsoku.unwrap_or(true),
            word_wrap: p.word_wrap.unwrap_or(true),
            overflow_punct: p.overflow_punct.unwrap_or(true),
            top_line_punct: p.top_line_punct.unwrap_or(false),
            auto_space_de: p.auto_space_de.unwrap_or(true),
            auto_space_dn: p.auto_space_dn.unwrap_or(true),
        }
    }
}

impl StyleSheet {
    /// Paragraph properties after defaults and the style chain (without direct formatting).
    pub fn para_style_props(&self, style: &str) -> (ParaProps, CharProps) {
        self.para_style_props_over(style, self.default_para.clone(), self.default_chr.clone())
    }

    /// The paragraph style chain of `style` applied over `pp` / `cp`.
    fn para_style_props_over(&self, style: &str, mut pp: ParaProps, mut cp: CharProps) -> (ParaProps, CharProps) {
        for s in self.chain(style) {
            if s.kind == StyleKind::Paragraph {
                let mut sp = s.para.clone();
                // Tabs accumulate along the chain (with Clear removing inherited stops).
                if let (Some(base), Some(add)) = (pp.tabs.clone(), sp.tabs.clone()) {
                    let mut merged: Vec<TabStop> = base.into_iter().filter(|t| !add.iter().any(|a| (a.pos - t.pos).abs() < 0.5)).collect();
                    merged.extend(add);
                    sp.tabs = Some(merged);
                }
                pp.overlay(&sp);
                cp.overlay(&s.chr);
            }
        }
        pp.style = None;
        (pp, cp)
    }

    /// Resolve a paragraph's properties.
    pub fn resolve_para(&self, p: &ParaProps) -> ResolvedPara {
        self.resolve_para_in(p, None)
    }

    /// Resolve a paragraph's properties; `table` is the paragraph formatting of the table style of
    /// the table the paragraph is in. The order is that of ECMA-376 §17.7.2: document defaults, the
    /// table style, the paragraph style chain, direct formatting. So the table style replaces
    /// document defaults (Table Grid's single spacing beats the defaults' 8 pt after), but any
    /// property the paragraph style chain sets, Normal included, beats the table style.
    pub fn resolve_para_in(&self, p: &ParaProps, table: Option<&ParaProps>) -> ResolvedPara {
        let style = p.style.clone().unwrap_or_else(|| "Normal".into());
        let mut defaults = self.default_para.clone();
        if let Some(t) = table {
            defaults.overlay(&ParaProps { style: None, ..t.clone() });
        }
        let (mut base, _) = self.para_style_props_over(&style, defaults, CharProps::default());
        let mut direct = p.clone();
        if let (Some(b), Some(d)) = (base.tabs.clone(), direct.tabs.clone()) {
            let mut merged: Vec<TabStop> = b.into_iter().filter(|t| !d.iter().any(|a| (a.pos - t.pos).abs() < 0.5)).collect();
            merged.extend(d);
            direct.tabs = Some(merged);
        }
        base.overlay(&direct);
        ResolvedPara::from(&base, &style)
    }

    /// Resolve character formatting for a run in a paragraph of style `para_style`.
    pub fn resolve_char(&self, para_style: Option<&str>, c: &CharProps) -> ResolvedChar {
        self.resolve_char_in(para_style, None, c)
    }

    /// Resolve character formatting for a run in a paragraph of style `para_style`; `table` is the
    /// run formatting of the table style (with the cell's conditional formatting) of the table the
    /// paragraph is in. Order: document defaults, the table style, the paragraph style chain, the
    /// character style chain, direct formatting (ECMA-376 §17.7.2).
    pub fn resolve_char_in(&self, para_style: Option<&str>, table: Option<&CharProps>, c: &CharProps) -> ResolvedChar {
        let mut defaults = self.default_chr.clone();
        if let Some(t) = table {
            defaults.overlay(&CharProps { style: None, ..t.clone() });
        }
        let (_, mut cp) = self.para_style_props_over(para_style.unwrap_or("Normal"), ParaProps::default(), defaults);
        if let Some(cs) = c.style.as_deref() {
            for s in self.chain(cs) {
                if s.kind == StyleKind::Character || s.kind == StyleKind::Paragraph {
                    cp.overlay(&s.chr);
                }
            }
        }
        let mut direct = c.clone();
        direct.style = None;
        cp.overlay(&direct);
        ResolvedChar::from(&cp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_resolves() {
        let s = StyleSheet::builtin();
        let p = s.resolve_para(&ParaProps { style: Some("Heading1".into()), ..Default::default() });
        assert!(p.keep_next);
        assert_eq!(p.outline_level, Some(0));
        assert_eq!(p.space_after, 4.0);
        let c = s.resolve_char(Some("Heading1"), &CharProps::default());
        assert_eq!(c.size, 20.0);
        assert_eq!(c.font, crate::styles::HEADING_FONT);
        let c2 = s.resolve_char(Some("Heading1"), &CharProps { size: Some(30.0), ..Default::default() });
        assert_eq!(c2.size, 30.0);
    }

    #[test]
    fn char_style_then_direct() {
        let s = StyleSheet::builtin();
        let c = s.resolve_char(None, &CharProps { style: Some("Strong".into()), italic: Some(true), ..Default::default() });
        assert!(c.bold && c.italic);
        assert_eq!(c.size, 12.0);
        let unknown = s.resolve_char(Some("NoSuchStyle"), &CharProps::default());
        assert_eq!(unknown.size, 12.0);
    }

    #[test]
    fn hostile_values_clamped() {
        let s = StyleSheet::builtin();
        let p = s.resolve_para(&ParaProps {
            indent_left: Some(f32::INFINITY),
            line_spacing: Some(LineSpacing::Multiple(f32::NAN)),
            ..Default::default()
        });
        assert_eq!(p.indent_left, MAX_INDENT);
        assert_eq!(p.line_spacing, LineSpacing::Multiple(1.0));
        let c = s.resolve_char(None, &CharProps { size: Some(-4.0), ..Default::default() });
        assert_eq!(c.size, 1.0);
    }

    /// ECMA-376 §17.7.2 for text in a table: document defaults, then the table style, then the
    /// paragraph style chain (Normal included), then direct formatting.
    #[test]
    fn table_style_sits_between_defaults_and_paragraph_style() {
        let mut s = StyleSheet::builtin();
        s.styles.push(crate::styles::Style {
            id: "Spaced".into(),
            based_on: Some("Normal".into()),
            para: ParaProps { space_after: Some(12.0), ..Default::default() },
            chr: CharProps { color: Some(TextColor::Rgb(Rgb(0, 0, 0xFF))), ..Default::default() },
            ..Default::default()
        });
        let table_p = ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..Default::default() };
        let red = TextColor::Rgb(Rgb(0xC0, 0, 0));
        let table_c = CharProps { bold: Some(true), color: Some(red), size: Some(9.0), ..Default::default() };
        // Document defaults alone (outside a table).
        let out = s.resolve_para(&ParaProps::default());
        assert_eq!((out.space_after, out.line_spacing), (8.0, LineSpacing::Multiple(1.15)));
        // The table style replaces the defaults.
        let p = s.resolve_para_in(&ParaProps::default(), Some(&table_p));
        assert_eq!((p.space_after, p.line_spacing), (0.0, LineSpacing::Multiple(1.0)));
        let c = s.resolve_char_in(None, Some(&table_c), &CharProps::default());
        assert!(c.bold);
        assert_eq!((c.color, c.size), (red, 9.0));
        // A paragraph style beats it, for what the style sets only.
        let spaced = ParaProps { style: Some("Spaced".into()), ..Default::default() };
        let p = s.resolve_para_in(&spaced, Some(&table_p));
        assert_eq!((p.space_after, p.line_spacing), (12.0, LineSpacing::Multiple(1.0)));
        let c = s.resolve_char_in(Some("Spaced"), Some(&table_c), &CharProps::default());
        assert!(c.bold);
        assert_eq!((c.color, c.size), (TextColor::Rgb(Rgb(0, 0, 0xFF)), 9.0));
        // A character style and direct formatting beat both.
        let c =
            s.resolve_char_in(Some("Spaced"), Some(&table_c), &CharProps { style: Some("Emphasis".into()), bold: Some(false), ..Default::default() });
        assert!(!c.bold && c.italic);
        let p = s.resolve_para_in(&ParaProps { space_after: Some(3.0), ..spaced }, Some(&table_p));
        assert_eq!(p.space_after, 3.0);
        // Normal setting a property itself beats the table style too (Word draws such rows tall).
        if let Some(n) = s.get_mut("Normal") {
            n.para.space_after = Some(10.0);
            n.chr.size = Some(11.0);
        }
        assert_eq!(s.resolve_para_in(&ParaProps::default(), Some(&table_p)).space_after, 10.0);
        assert_eq!(s.resolve_char_in(None, Some(&table_c), &CharProps::default()).size, 11.0);
    }

    #[test]
    fn header_tabs_inherit() {
        let s = StyleSheet::builtin();
        let p = s.resolve_para(&ParaProps {
            style: Some("Header".into()),
            tabs: Some(vec![TabStop { pos: 100.0, align: TabAlign::Left, leader: Default::default() }]),
            ..Default::default()
        });
        assert_eq!(p.tabs.len(), 3);
        assert_eq!(p.tabs[0].pos, 100.0);
    }
}
