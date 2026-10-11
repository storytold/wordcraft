//! Sprms → model properties, and style-chain resolution.

use wordcraft_doc::Rgb;
use wordcraft_doc::props::{Align, CharProps, Highlight, LineSpacing, ParaProps, TextColor, Underline, VertAlign};
use wordcraft_doc::styles::{Style, StyleKind, StyleSheet};

use crate::sprm::{self, Prl, Toggle};

// Character sprms ([MS-DOC] §2.6.1).
const C_BOLD: u16 = 0x0835;
const C_ITALIC: u16 = 0x0836;
const C_STRIKE: u16 = 0x0837;
const C_SMALL_CAPS: u16 = 0x083A;
const C_CAPS: u16 = 0x083B;
const C_HIDDEN: u16 = 0x083C; // sprmCFVanish
const C_KUL: u16 = 0x2A3E; // underline style (Kul)
const C_ICO: u16 = 0x2A42; // palette colour
const C_CR: u16 = 0x6870; // explicit COLORREF
const C_HPS: u16 = 0x4A43; // half-points
const C_HPS_POS: u16 = 0x4845; // super/subscript by half-points
const C_FTC0: u16 = 0x4A4F; // font index
const C_HIGHLIGHT: u16 = 0x2A0C;
const C_ISTD: u16 = 0x4A30; // character style

// Paragraph sprms ([MS-DOC] §2.6.2).
const P_JC: u16 = 0x2461;
const P_JC80: u16 = 0x2403;
const P_DXA_LEFT: u16 = 0x845E;
const P_DXA_LEFT80: u16 = 0x840F;
const P_DXA_RIGHT: u16 = 0x845D;
const P_DXA_RIGHT80: u16 = 0x840E;
const P_DXA_FIRST: u16 = 0x8460; // sprmPDxaLeft1
const P_DYA_BEFORE: u16 = 0xA413;
const P_DYA_AFTER: u16 = 0xA414;
const P_DYA_LINE: u16 = 0x6412;
const P_KEEP: u16 = 0x2405; // keep lines together
const P_KEEP_FOLLOW: u16 = 0x2406; // keep with next
const P_PAGE_BREAK_BEFORE: u16 = 0x2407;
const P_OUTLVL: u16 = 0x2640;

/// Word's 16-colour `Ico` palette ([MS-DOC] §2.9.135), 0 = automatic.
const ICO: [Rgb; 17] = [
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 255),
    Rgb(0, 255, 255),
    Rgb(0, 255, 0),
    Rgb(255, 0, 255),
    Rgb(255, 0, 0),
    Rgb(255, 255, 0),
    Rgb(255, 255, 255),
    Rgb(0, 0, 128),
    Rgb(0, 128, 128),
    Rgb(0, 128, 0),
    Rgb(128, 0, 128),
    Rgb(128, 0, 0),
    Rgb(128, 128, 0),
    Rgb(128, 128, 128),
    Rgb(192, 192, 192),
];

/// Highlight palette ([MS-DOC] §2.9.135 Highlight, order of Word's highlighter row).
const HL: [Highlight; 17] = [
    Highlight::None,
    Highlight::Yellow,
    Highlight::BrightGreen,
    Highlight::Turquoise,
    Highlight::Pink,
    Highlight::Blue,
    Highlight::Red,
    Highlight::DarkBlue,
    Highlight::Teal,
    Highlight::Green,
    Highlight::Violet,
    Highlight::DarkRed,
    Highlight::DarkYellow,
    Highlight::Gray50,
    Highlight::Gray25,
    Highlight::None,
    Highlight::None,
];

/// The `Ico` palette colour (0 = automatic → `None`).
pub(crate) fn ico(i: u8) -> Option<Rgb> {
    if i == 0 {
        return None;
    }
    ICO.get(i as usize).copied()
}

fn u16_of(p: &Prl) -> Option<u16> {
    match p.operand {
        [b0, b1, ..] => Some(u16::from_le_bytes([*b0, *b1])),
        _ => None,
    }
}

fn s16_of(p: &Prl) -> Option<i16> {
    u16_of(p).map(|v| v as i16)
}

fn set(props: &mut Option<bool>, t: Toggle, base: bool) {
    *props = Some(match t {
        Toggle::On => true,
        Toggle::Off => false,
        Toggle::Invert => !base,
        Toggle::Keep => return,
    });
}

/// Apply a character-property sprm. `base` carries the style-inherited values so toggle
/// operands can invert them; `fonts` maps sprmCRgFtc0 indices to names.
pub(crate) fn apply_char(p: &mut CharProps, prl: &Prl, base: &CharProps, fonts: &[String]) {
    let op = prl.op;
    match op {
        C_BOLD => set(&mut p.bold, sprm::toggle(prl), base.bold.unwrap_or(false)),
        C_ITALIC => set(&mut p.italic, sprm::toggle(prl), base.italic.unwrap_or(false)),
        C_STRIKE => set(&mut p.strike, sprm::toggle(prl), base.strike.unwrap_or(false)),
        C_SMALL_CAPS => set(&mut p.small_caps, sprm::toggle(prl), base.small_caps.unwrap_or(false)),
        C_CAPS => set(&mut p.caps, sprm::toggle(prl), base.caps.unwrap_or(false)),
        C_HIDDEN => set(&mut p.hidden, sprm::toggle(prl), base.hidden.unwrap_or(false)),
        C_KUL => {
            p.underline = match prl.operand.first().copied().unwrap_or(0) {
                0x00 => Some(Underline::None),
                0x01 => Some(Underline::Single),
                0x02 => Some(Underline::Words),
                0x03 => Some(Underline::Double),
                0x04 => Some(Underline::Dotted),
                0x06 => Some(Underline::Thick),
                0x07 => Some(Underline::Dash),
                0x09 => Some(Underline::DotDash),
                0x0A => Some(Underline::DotDotDash),
                0x0B => Some(Underline::Wave),
                0x14 => Some(Underline::Dotted),
                0x17 => Some(Underline::Thick),
                0x18 => Some(Underline::Dash),
                0x19 => Some(Underline::DotDash),
                0x1A => Some(Underline::DotDotDash),
                0x1B => Some(Underline::DoubleWave),
                _ => Some(Underline::Single),
            };
        }
        C_ICO => {
            if let Some(rgb) = ICO.get(prl.operand.first().copied().unwrap_or(0) as usize) {
                p.color = Some(if rgb == &Rgb::BLACK && prl.operand.first() == Some(&0) { TextColor::Auto } else { TextColor::Rgb(*rgb) });
            }
        }
        C_CR => {
            // COLORREF 0x00BBGGRR, stored little-endian: operand bytes are R, G, B.
            if let [r, g, b, ..] = prl.operand
                && let (r, g, b) = (*r, *g, *b)
            {
                p.color = Some(TextColor::Rgb(Rgb(r, g, b)));
            }
        }
        C_HPS => {
            if let Some(hps) = u16_of(prl) {
                p.size = Some((hps as f32 / 2.0).clamp(1.0, 1638.0));
            }
        }
        C_HPS_POS => {
            if let Some(pos) = s16_of(prl) {
                p.vert_align = Some(match pos {
                    pos if pos > 0 => VertAlign::Superscript,
                    pos if pos < 0 => VertAlign::Subscript,
                    _ => VertAlign::Baseline,
                });
            }
        }
        C_FTC0 => {
            if let Some(name) = u16_of(prl).and_then(|i| fonts.get(i as usize)) {
                p.font = Some(name.clone());
            }
        }
        C_HIGHLIGHT => {
            if let Some(h) = HL.get(prl.operand.first().copied().unwrap_or(0) as usize) {
                p.highlight = Some(*h);
            }
        }
        C_ISTD => {} // resolved separately through the style table
        _ => {}
    }
}

/// Apply a paragraph-property sprm.
pub(crate) fn apply_para(p: &mut ParaProps, prl: &Prl) {
    let tw = |v: i32| (v as f32 / 20.0).clamp(-1000.0, 1000.0);
    let s16 = |prl: &Prl| -> Option<i16> { u16_of(prl).map(|v| v as i16) };
    let i32of = |prl: &Prl| -> Option<i32> {
        match prl.operand {
            [b0, b1, b2, b3, ..] => Some(i32::from_le_bytes([*b0, *b1, *b2, *b3])),
            _ => None,
        }
    };
    match prl.op {
        P_JC | P_JC80 => {
            p.align = match prl.operand.first().copied().unwrap_or(0) {
                1 => Some(Align::Center),
                2 => Some(Align::Right),
                3 => Some(Align::Justify),
                4 => Some(Align::Distribute),
                _ => Some(Align::Left),
            };
        }
        P_DXA_LEFT | P_DXA_LEFT80 => {
            if let Some(v) = s16(prl) {
                p.indent_left = Some(tw(v as i32));
            }
        }
        P_DXA_RIGHT | P_DXA_RIGHT80 => {
            if let Some(v) = s16(prl) {
                p.indent_right = Some(tw(v as i32));
            }
        }
        P_DXA_FIRST => {
            if let Some(v) = i32of(prl) {
                p.indent_first = Some(tw(v));
            }
        }
        P_DYA_BEFORE => {
            if let Some(v) = s16(prl) {
                p.space_before = Some((v as f32 / 20.0).clamp(0.0, 3168.0));
            }
        }
        P_DYA_AFTER => {
            if let Some(v) = s16(prl) {
                p.space_after = Some((v as f32 / 20.0).clamp(0.0, 3168.0));
            }
        }
        P_DYA_LINE => {
            if let Some(v) = u16_of(prl) {
                let mult = (v & 0x3FFF) as f32;
                p.line_spacing = Some(match v & 0xC000 {
                    0x0000 => LineSpacing::Multiple((mult / 240.0).clamp(0.1, 20.0)),
                    0x8000 => LineSpacing::AtLeast((mult / 20.0).clamp(0.0, 1584.0)),
                    _ => LineSpacing::Exactly((mult / 20.0).clamp(0.0, 1584.0)),
                });
            }
        }
        P_KEEP => {
            if matches!(sprm::toggle(prl), Toggle::On) {
                p.keep_lines = Some(true);
            }
        }
        P_KEEP_FOLLOW => {
            if matches!(sprm::toggle(prl), Toggle::On) {
                p.keep_next = Some(true);
            }
        }
        P_PAGE_BREAK_BEFORE => {
            if matches!(sprm::toggle(prl), Toggle::On) {
                p.page_break_before = Some(true);
            }
        }
        P_OUTLVL => {
            if let Some(&l) = prl.operand.first() {
                p.outline_level = Some(l.min(8));
            }
        }
        _ => {}
    }
}

/// Build the model stylesheet: resolve each raw style's chain (base → this) and the ids we
/// reference from paragraphs.
pub(crate) fn stylesheet(raw: &[crate::stsh::RawStyle], fonts: &[String]) -> StyleSheet {
    let mut sheet = StyleSheet::empty();
    // Word's base character properties: font index 0 of the table, 10 pt ([MS-DOC] sprmCHps).
    sheet.default_chr.font = fonts.first().cloned();
    if sheet.default_chr.size.is_none() {
        sheet.default_chr.size = Some(10.0);
    }
    for (istd, st) in raw.iter().enumerate() {
        if st.name.is_empty() {
            continue;
        }
        let id = style_id(&st.name);
        let kind = match st.stk {
            2 => StyleKind::Character,
            3 => StyleKind::Table,
            4 => StyleKind::Numbering,
            _ => StyleKind::Paragraph,
        };
        let base = base_of(raw, istd).and_then(|i| raw.get(i)).map(|b| style_id(&b.name));
        // Inherit: properties come from the base chain, root first, then our own grpprls.
        let mut chr = CharProps::default();
        let mut para = ParaProps::default();
        for s in style_chain(raw, istd).into_iter().filter_map(|i| raw.get(i)) {
            apply_chain(&mut chr, &s.chpx, fonts);
            apply_chain_para(&mut para, &s.papx);
        }
        if para.outline_level.is_none() && kind == StyleKind::Paragraph {
            let h = heading_level(&st.name);
            if h > 0 {
                para.outline_level = Some(h - 1);
            }
        }
        sheet.styles.push(Style { id, name: display_name(&st.name), kind, based_on: base, chr, para, ..Default::default() });
    }
    sheet
}

/// Most `istdBase` links followed; a longer chain (or a cycle) in a corrupt file stops there.
const MAX_STYLE_DEPTH: usize = 16;

/// The parent of style `istd`: its zero-based `istdBase` ([MS-DOC] §2.9.260), when that names
/// another existing, named style (0x0FFF means none).
fn base_of(raw: &[crate::stsh::RawStyle], istd: usize) -> Option<usize> {
    let b = raw.get(istd)?.istd_base;
    let b = usize::from(b);
    (b != 0x0FFF && b != istd && raw.get(b).is_some_and(|s| !s.name.is_empty())).then_some(b)
}

/// Style `istd` and its ancestors, root first and `istd` last. Cycles and chains deeper than
/// [`MAX_STYLE_DEPTH`] are cut.
fn style_chain(raw: &[crate::stsh::RawStyle], istd: usize) -> Vec<usize> {
    let mut chain = vec![istd];
    let mut cur = istd;
    while chain.len() < MAX_STYLE_DEPTH {
        let Some(b) = base_of(raw, cur) else { break };
        if chain.contains(&b) {
            break;
        }
        chain.push(b);
        cur = b;
    }
    chain.reverse();
    chain
}

/// The style-id form of a stored (lower-case) built-in name, matching the docx reader's ids.
pub(crate) fn style_id(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "normal" => "Normal".into(),
        "title" => "Title".into(),
        "subtitle" => "Subtitle".into(),
        "quote" | "body text" | "body text 2" | "body text 3" | "body text indent" | "body text indent 2" | "body text indent 3" => {
            let mut c = lower.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
        "default paragraph font" => "Default Paragraph Font".into(),
        _ => {
            let mut out = String::with_capacity(name.len());
            let mut up = true;
            for ch in name.trim().chars() {
                if ch == ' ' {
                    up = true;
                    out.push(ch);
                } else if up {
                    out.extend(ch.to_uppercase());
                    up = false;
                } else {
                    out.push(ch);
                }
            }
            out
        }
    }
}

/// "heading 3" → 3; 0 when not a heading.
fn heading_level(name: &str) -> u8 {
    let lower = name.trim().to_ascii_lowercase();
    lower.strip_prefix("heading ").and_then(|d| d.parse().ok()).filter(|&l: &u8| (1..=9).contains(&l)).unwrap_or(0)
}

fn display_name(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    let h = heading_level(&lower);
    if h > 0 {
        return format!("Heading {h}");
    }
    match lower.as_str() {
        "body text" => "Body Text".into(),
        _ => style_id(name),
    }
}

/// Apply a grpprl of character sprms with a toggle base.
fn apply_chain(chr: &mut CharProps, grpprl: &[u8], fonts: &[String]) {
    let base = chr.clone();
    for prl in sprm::iter(grpprl) {
        if prl.sgc() == 2 {
            apply_char(chr, &prl, &base, fonts);
        }
    }
}

/// Apply a grpprl of paragraph sprms.
fn apply_chain_para(para: &mut ParaProps, grpprl: &[u8]) {
    for prl in sprm::iter(grpprl) {
        if prl.sgc() == 1 {
            apply_para(para, &prl);
        }
    }
}
