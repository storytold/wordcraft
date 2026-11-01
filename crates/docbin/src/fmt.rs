//! Sprms → model properties, and style-chain resolution.

use wordcraft_doc::Rgb;
use wordcraft_doc::props::{
    Align, Border, BorderStyle, Borders, CharProps, Highlight, LineSpacing, ParaProps, TabAlign, TabLeader, TabStop, TextColor, Underline, VertAlign,
};
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
const P_DXA_FIRST80: u16 = 0x8411; // sprmPDxaLeft1_80
// Indents in hundredths of a character (sprmPDxcRight, sprmPDxcLeft, sprmPDxcLeft1).
const P_DXC_RIGHT: u16 = 0x4455;
const P_DXC_LEFT: u16 = 0x4456;
const P_DXC_FIRST: u16 = 0x4457;
const P_DYA_BEFORE: u16 = 0xA413;
const P_DYA_AFTER: u16 = 0xA414;
const P_DYA_LINE: u16 = 0x6412;
const P_KEEP: u16 = 0x2405; // keep lines together
const P_KEEP_FOLLOW: u16 = 0x2406; // keep with next
const P_PAGE_BREAK_BEFORE: u16 = 0x2407;
const P_OUTLVL: u16 = 0x2640;
// Paragraph borders ([MS-DOC] §2.6.2). The 80 forms carry a Brc80 (palette colour), the
// newer ones a BrcOperand: cb(1) + Brc(8, with a full COLORREF).
const P_BRC_TOP80: u16 = 0x6424;
const P_BRC_LEFT80: u16 = 0x6425;
const P_BRC_BOTTOM80: u16 = 0x6426;
const P_BRC_RIGHT80: u16 = 0x6427;
const P_BRC_BETWEEN80: u16 = 0x6428;
const P_BRC_TOP: u16 = 0xC64E;
const P_BRC_LEFT: u16 = 0xC64F;
const P_BRC_BOTTOM: u16 = 0xC650;
const P_BRC_RIGHT: u16 = 0xC651;
const P_BRC_BETWEEN: u16 = 0xC652;

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

/// Highlight palette. The `sprmCHighlight` operand is an `Ico` ([MS-DOC] §2.9.135), the same
/// index as the text-colour palette above (1 = black, 7 = yellow, 9 = dark blue, ...), not the
/// order of Word's highlighter row. 0 = no highlight.
const HL: [Highlight; 17] = [
    Highlight::None,
    Highlight::Black,
    Highlight::Blue,
    Highlight::Turquoise,
    Highlight::BrightGreen,
    Highlight::Pink,
    Highlight::Red,
    Highlight::Yellow,
    Highlight::White,
    Highlight::DarkBlue,
    Highlight::Teal,
    Highlight::Green,
    Highlight::Violet,
    Highlight::DarkRed,
    Highlight::DarkYellow,
    Highlight::Gray50,
    Highlight::Gray25,
];

/// The `Ico` palette colour (0 = automatic → `None`).
pub(crate) fn ico(i: u8) -> Option<Rgb> {
    if i == 0 {
        return None;
    }
    ICO.get(i as usize).copied()
}

/// The model border style a `brcType` stands for ([MS-DOC] §2.9.22 BrcType). Types the model
/// doesn't carry (the gap variants, 3-D, inset/outset) draw as single lines, as the DOCX
/// reader maps them.
fn brc_style(t: u8) -> BorderStyle {
    match t {
        0x00 => BorderStyle::None,
        0x03 => BorderStyle::Double,
        0x06 => BorderStyle::Dotted,
        0x07 | 0x16 => BorderStyle::Dashed,
        0x08 | 0x09 => BorderStyle::DotDash,
        0x0A => BorderStyle::Triple,
        0x14 | 0x15 => BorderStyle::Wave,
        _ => BorderStyle::Single,
    }
}

/// The border width in points: `dptLineWidth` in 1/8-point steps (values below 2 count as 2;
/// 1-point steps for page borders, which never reach a paragraph property).
fn brc_width(t: u8, dpt: u8) -> f32 {
    if t >= 0x40 { f32::from(dpt) } else { f32::from(dpt.max(2)) / 8.0 }
}

/// One border from a Brc80 operand ([MS-DOC] §2.9.18). All bits set means "no border"; a
/// truncated operand is ignored.
fn brc80(op: &[u8]) -> Option<Border> {
    let [dpt, ty, ico_index, tail, ..] = op else { return None };
    if [*dpt, *ty, *ico_index, *tail] == [0xFF; 4] {
        return Some(Border::default());
    }
    Some(Border { style: brc_style(*ty), width: brc_width(*ty, *dpt), color: ico(*ico_index), space: f32::from(tail & 0x1F) })
}

/// One border from a `BrcOperand` ([MS-DOC] §2.9.21): cb(1, MUST be 8) + Brc(8).
fn brc(op: &[u8]) -> Option<Border> {
    if op.first() != Some(&8) {
        return None;
    }
    let b = op.get(1..9)?;
    if b.iter().all(|x| *x == 0xFF) {
        return Some(Border::default());
    }
    let color = match b.get(..4) {
        Some([0, 0, 0, 0]) => None, // automatic
        Some(px) => Some(Rgb(px[0], px[1], px[2])),
        None => None,
    };
    Some(Border { style: brc_style(b[5]), width: brc_width(b[5], b[4]), color, space: f32::from(b[6] & 0x1F) })
}

/// Put a decoded border on the side its sprm names (`between` = the border between this
/// paragraph and the next).
fn set_border(p: &mut ParaProps, op: u16, b: Border) {
    let borders = p.borders.get_or_insert_with(Borders::default);
    let slot = match op {
        P_BRC_TOP80 | P_BRC_TOP => &mut borders.top,
        P_BRC_LEFT80 | P_BRC_LEFT => &mut borders.left,
        P_BRC_BOTTOM80 | P_BRC_BOTTOM => &mut borders.bottom,
        P_BRC_RIGHT80 | P_BRC_RIGHT => &mut borders.right,
        _ => &mut borders.between,
    };
    *slot = Some(b);
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

/// Custom tab stops from `sprmPChgTabsPapx` / `sprmPChgTabs` ([MS-DOC] PChgTabsPapxOperand,
/// PChgTabsOperand): the stops removed become `Clear` stops (they hide inherited ones) and the
/// stops added carry their alignment and leader (TBD: jc in bits 0–2, tlc in bits 3–5).
fn change_tabs(p: &mut ParaProps, prl: &Prl) {
    let op = prl.operand;
    let i16_at = |i: usize| op.get(i..i + 2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 20.0);
    // Both operands start with cb; sprmPChgTabs's removals also carry a "close" distance each.
    let close = prl.op == sprm::P_CHG_TABS;
    let mut at = 1;
    let Some(&del) = op.get(at) else { return };
    let del = usize::from(del.min(64));
    at += 1;
    let mut removed = Vec::with_capacity(del);
    for k in 0..del {
        if let Some(x) = i16_at(at + 2 * k) {
            removed.push(x);
        }
    }
    at += 2 * del * if close { 2 } else { 1 };
    let Some(&add) = op.get(at) else { return };
    let add = usize::from(add.min(64));
    at += 1;
    let mut tabs = p.tabs.take().unwrap_or_default();
    // Removed within 25 twips (1.25 pt), as the format says.
    tabs.retain(|t| !removed.iter().any(|x| (t.pos - x).abs() <= 1.25));
    for x in removed {
        tabs.push(TabStop { pos: x, align: TabAlign::Clear, leader: TabLeader::None });
    }
    for k in 0..add {
        let (Some(pos), Some(&tbd)) = (i16_at(at + 2 * k), op.get(at + 2 * add + k)) else { break };
        let align = match tbd & 7 {
            1 => TabAlign::Center,
            2 => TabAlign::Right,
            3 => TabAlign::Decimal,
            4 => TabAlign::Bar,
            _ => TabAlign::Left,
        };
        let leader = match (tbd >> 3) & 7 {
            1 => TabLeader::Dot,
            2 => TabLeader::Hyphen,
            3 | 4 => TabLeader::Underscore,
            5 => TabLeader::MiddleDot,
            _ => TabLeader::None,
        };
        tabs.retain(|t| (t.pos - pos).abs() > 0.05);
        tabs.push(TabStop { pos, align, leader });
    }
    tabs.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    p.tabs = Some(tabs);
}

/// Apply a paragraph-property sprm.
pub(crate) fn apply_para(p: &mut ParaProps, prl: &Prl) {
    let tw = |v: i32| (v as f32 / 20.0).clamp(-1000.0, 1000.0);
    let s16 = |prl: &Prl| -> Option<i16> { u16_of(prl).map(|v| v as i16) };
    match prl.op {
        sprm::P_CHG_TABS | sprm::P_CHG_TABS_PAPX => change_tabs(p, prl),
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
        // A two-byte operand, like the other indents (spra 4).
        P_DXA_FIRST | P_DXA_FIRST80 => {
            if let Some(v) = s16(prl) {
                p.indent_first = Some(tw(v as i32));
            }
        }
        P_DXC_RIGHT | P_DXC_LEFT | P_DXC_FIRST => {
            if let Some(v) = s16(prl) {
                let chars = Some(f32::from(v) / 100.0);
                match prl.op {
                    P_DXC_RIGHT => p.indent_right_chars = chars,
                    P_DXC_LEFT => p.indent_left_chars = chars,
                    _ => p.indent_first_chars = chars,
                }
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
        // LSPD ([MS-DOC] §2.9.150): a signed dyaLine, then fMultLinespace. In lines (240ths) when
        // fMultLinespace is 1; else twips, at least when positive and exactly when negative.
        P_DYA_LINE => {
            if let Some(v) = s16(prl) {
                let mult = prl.operand.get(2..4).is_some_and(|b| b == [1, 0]);
                let v = f32::from(v);
                p.line_spacing = Some(if mult {
                    LineSpacing::Multiple((v.abs() / 240.0).clamp(0.06, 132.0))
                } else if v < 0.0 {
                    LineSpacing::Exactly((-v / 20.0).clamp(0.7, 1584.0))
                } else {
                    LineSpacing::AtLeast((v / 20.0).clamp(0.0, 1584.0))
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
        P_BRC_TOP80 | P_BRC_LEFT80 | P_BRC_BOTTOM80 | P_BRC_RIGHT80 | P_BRC_BETWEEN80 => {
            if let Some(b) = brc80(prl.operand) {
                set_border(p, prl.op, b);
            }
        }
        P_BRC_TOP | P_BRC_LEFT | P_BRC_BOTTOM | P_BRC_RIGHT | P_BRC_BETWEEN => {
            if let Some(b) = brc(prl.operand) {
                set_border(p, prl.op, b);
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
