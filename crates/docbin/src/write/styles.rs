//! The stylesheet (`STSH`, [MS-DOC] §2.9.271) and the font table (`SttbfFfn`, §2.9.286).
//!
//! Istds 0–14 are fixed: 0 Normal, 1–9 heading 1–9, 10 Default Paragraph Font, 11 Normal
//! Table, 12 No List, 13–14 unused. Every other style follows from istd 15 in stylesheet
//! order. A style Word has built in keeps its `sti` and the name Word gives it, so Word treats
//! it as its own; the rest are user styles.

use std::collections::HashMap;

use wordcraft_doc::props::CharProps;
use wordcraft_doc::styles::{Style, StyleKind, StyleSheet};

use super::grpprl::{self, CharCtx, Grpprl, ParaCtx};

/// Istds reserved for built-in styles.
const FIXED: u16 = 15;
/// `sti` of a user-defined style.
const STI_USER: u16 = 0x0FFE;
/// `istdBase` / `istdNext` meaning "none".
const ISTD_NIL: u16 = 0x0FFF;
/// Most styles written (the format allows 0x0FFE).
const MAX_STYLES: usize = 4000;

/// Built-in styles: the name Word stores (lower-case where Word's is), and the `sti`.
const BUILTIN: &[(&str, u16)] = &[
    ("Normal", 0),
    ("heading 1", 1),
    ("heading 2", 2),
    ("heading 3", 3),
    ("heading 4", 4),
    ("heading 5", 5),
    ("heading 6", 6),
    ("heading 7", 7),
    ("heading 8", 8),
    ("heading 9", 9),
    ("toc 1", 19),
    ("toc 2", 20),
    ("toc 3", 21),
    ("toc 4", 22),
    ("toc 5", 23),
    ("toc 6", 24),
    ("toc 7", 25),
    ("toc 8", 26),
    ("toc 9", 27),
    ("Normal Indent", 28),
    ("footnote text", 29),
    ("annotation text", 30),
    ("header", 31),
    ("footer", 32),
    ("index heading", 33),
    ("caption", 34),
    ("table of figures", 35),
    ("footnote reference", 38),
    ("annotation reference", 39),
    ("line number", 40),
    ("page number", 41),
    ("endnote reference", 42),
    ("endnote text", 43),
    ("List", 47),
    ("List Bullet", 48),
    ("List Number", 49),
    ("Title", 62),
    ("Signature", 64),
    ("Default Paragraph Font", 65),
    ("Body Text", 66),
    ("Body Text Indent", 67),
    ("List Continue", 68),
    ("Subtitle", 74),
    ("Salutation", 75),
    ("Date", 76),
    ("Hyperlink", 85),
    ("FollowedHyperlink", 86),
    ("Strong", 87),
    ("Emphasis", 88),
    ("Plain Text", 91),
    ("Normal (Web)", 94),
    ("Normal Table", 105),
    ("No List", 107),
    ("Balloon Text", 153),
    ("Table Grid", 154),
    ("Placeholder Text", 156),
    ("No Spacing", 157),
    ("List Paragraph", 179),
    ("Quote", 180),
    ("Intense Quote", 181),
    ("Subtle Emphasis", 260),
    ("Intense Emphasis", 261),
    ("Subtle Reference", 262),
    ("Intense Reference", 263),
    ("Book Title", 264),
    ("Bibliography", 265),
    ("TOC Heading", 266),
];

/// The built-in style a model style is, by its name or id (`Heading 1`, `Heading1` and
/// `heading 1` are the same), with the name Word stores.
fn builtin(st: &Style) -> Option<(&'static str, u16)> {
    let norm = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    let (name, id) = (norm(&st.name), norm(&st.id));
    // The docx reader's ids for a few styles differ from their names.
    let alias = |s: &str| match s {
        "tablenormal" => "normaltable".to_string(),
        "commenttext" => "annotationtext".to_string(),
        "commentreference" => "annotationreference".to_string(),
        other => other.to_string(),
    };
    BUILTIN
        .iter()
        .find(|(n, _)| {
            let n = norm(n);
            n == alias(&name) || n == alias(&id)
        })
        .copied()
}

/// The stylesheet as written: each model style's istd.
pub(crate) struct Plan {
    /// Model style id → istd.
    pub(crate) istd: HashMap<String, u16>,
    /// Istd → model style (index into the stylesheet), `None` for an empty slot.
    slots: Vec<Option<usize>>,
    /// Istd → the name and sti it is written with.
    names: Vec<(String, u16)>,
}

/// Assign istds: the fixed built-ins to their slots, everything else after them.
pub(crate) fn plan(sheet: &StyleSheet) -> Plan {
    let mut slots: Vec<Option<usize>> = vec![None; FIXED as usize];
    let mut names: Vec<(String, u16)> = vec![(String::new(), 0); FIXED as usize];
    let mut istd = HashMap::new();
    let mut taken = std::collections::HashSet::new();
    for (i, st) in sheet.styles.iter().enumerate().take(MAX_STYLES) {
        if st.id.is_empty() || istd.contains_key(&st.id) {
            continue;
        }
        let bi = builtin(st);
        // The fixed slot a built-in style takes, when its kind is the one Word gives it.
        let fixed = bi.and_then(|(_, sti)| match (sti, st.kind) {
            (0..=9, StyleKind::Paragraph) => Some(sti),
            (65, StyleKind::Character) => Some(10),
            (105, StyleKind::Table) => Some(11),
            (107, StyleKind::Numbering) => Some(12),
            _ => None,
        });
        let slot = match fixed {
            Some(s) if !taken.contains(&s) => s,
            _ => {
                slots.push(None);
                names.push((String::new(), 0));
                (slots.len() - 1) as u16
            }
        };
        taken.insert(slot);
        if let Some(s) = slots.get_mut(slot as usize) {
            *s = Some(i);
        }
        // A built-in name is Word's own spelling; its sti only where the kind matches.
        let (name, sti) = match bi {
            Some((n, sti)) if builtin_kind(sti) == st.kind => (n.to_string(), sti),
            _ => (st.name.clone(), STI_USER),
        };
        if let Some(n) = names.get_mut(slot as usize) {
            *n = (name, sti);
        }
        istd.insert(st.id.clone(), slot);
    }
    // Normal and Default Paragraph Font are always there.
    if slots.first().is_some_and(Option::is_none)
        && let Some(n) = names.get_mut(0)
    {
        *n = ("Normal".into(), 0);
    }
    for (i, name, sti) in [(10usize, "Default Paragraph Font", 65u16), (11, "Normal Table", 105), (12, "No List", 107)] {
        if slots.get(i).is_some_and(Option::is_none)
            && let Some(n) = names.get_mut(i)
        {
            *n = (name.into(), sti);
        }
    }
    Plan { istd, slots, names }
}

/// The kind of style Word's built-in `sti` is.
fn builtin_kind(sti: u16) -> StyleKind {
    match sti {
        38 | 39 | 40 | 41 | 42 | 65 | 85 | 86 | 87 | 88 | 156 | 260..=264 => StyleKind::Character,
        105 | 154 => StyleKind::Table,
        107 => StyleKind::Numbering,
        _ => StyleKind::Paragraph,
    }
}

/// The STSH bytes. `default_font` is the font-table index of the document's default font
/// (the standard font of the stylesheet header).
pub(crate) fn stsh(sheet: &StyleSheet, plan: &Plan, ctx: &mut (impl CharCtx + ParaCtx), default_fonts: [u16; 3]) -> Vec<u8> {
    // LPStshi: cbStshi, then Stshi: Stshif (18 bytes), ftcBi, and the latent style data (an
    // empty LSD for every sti below stiMaxWhenSaved).
    let cstd = plan.slots.len().min(0x0FFE) as u16;
    let sti_max = plan.names.iter().map(|(_, sti)| *sti).filter(|&s| s != STI_USER).max().map_or(0x5B, |m| (m + 1).max(0x5B));
    let cb_stshi = 18 + 2 + 2 + 4 * usize::from(sti_max);
    let mut v = Vec::new();
    v.extend_from_slice(&(cb_stshi as u16).to_le_bytes());
    v.extend_from_slice(&cstd.to_le_bytes());
    v.extend_from_slice(&0x000Au16.to_le_bytes()); // cbSTDBaseInFile
    v.extend_from_slice(&1u16.to_le_bytes()); // fStdStylenamesWritten
    v.extend_from_slice(&sti_max.to_le_bytes()); // stiMaxWhenSaved
    v.extend_from_slice(&FIXED.to_le_bytes()); // istdMaxFixedWhenSaved
    v.extend_from_slice(&0u16.to_le_bytes()); // nVerBuiltInNamesWhenSaved
    for f in default_fonts {
        v.extend_from_slice(&f.to_le_bytes()); // rgftcStandardChpStsh
    }
    v.extend_from_slice(&default_fonts[2].to_le_bytes()); // ftcBi
    v.extend_from_slice(&4u16.to_le_bytes()); // StshiLsd.cbLSD
    v.extend(std::iter::repeat_n(0u8, 4 * usize::from(sti_max)));
    for (istd, slot) in plan.slots.iter().enumerate().take(cstd as usize) {
        let (name, sti) = plan.names.get(istd).cloned().unwrap_or_default();
        let std = match slot.and_then(|i| sheet.styles.get(i)) {
            Some(st) => std_bytes(st, &name, sti, sheet, plan, ctx, istd == 0),
            None if !name.is_empty() => empty_builtin(&name, sti, istd as u16),
            None => Vec::new(),
        };
        v.extend_from_slice(&(std.len() as u16).to_le_bytes());
        v.extend_from_slice(&std);
    }
    v
}

/// A built-in style the document doesn't define (Normal, Default Paragraph Font).
fn empty_builtin(name: &str, sti: u16, istd: u16) -> Vec<u8> {
    let own = istd.to_le_bytes().to_vec();
    let (stk, upx): (u16, Vec<Vec<u8>>) = match sti {
        65 => (2, vec![Vec::new()]),
        105 => (3, vec![Vec::new(), own, Vec::new()]),
        107 => (4, vec![own]),
        _ => (1, vec![own, Vec::new()]),
    };
    let next = if stk == 1 { istd } else { ISTD_NIL };
    assemble(sti, stk, ISTD_NIL, next, upx.len() as u16, 0, name, &upx)
}

/// One STD: `StdfBase`, the name, then the UPXs for its kind.
#[allow(clippy::too_many_arguments)]
fn std_bytes(st: &Style, name: &str, sti: u16, sheet: &StyleSheet, plan: &Plan, ctx: &mut (impl CharCtx + ParaCtx), normal: bool) -> Vec<u8> {
    let base = st.based_on.as_deref().and_then(|b| plan.istd.get(b).copied()).unwrap_or(ISTD_NIL);
    let istd_self = plan.istd.get(&st.id).copied().unwrap_or(0);
    let next = st.next.as_deref().and_then(|b| plan.istd.get(b).copied()).unwrap_or(istd_self);
    // Document defaults have nowhere else to go in this format: Normal (and any other root
    // paragraph or character style) starts from them.
    let root = base == ISTD_NIL;
    let mut chr = st.chr.clone();
    let mut para = st.para.clone();
    if root && matches!(st.kind, StyleKind::Paragraph) {
        chr = overlay(&sheet.default_chr, &chr);
        para = wordcraft_doc::props::ParaProps { style: None, ..overlay_para(&sheet.default_para, &para) };
    }
    if normal {
        chr.style = None;
    }
    para.style = None;
    para.numbering = para.numbering.filter(|n| n.num != 0);
    chr.style = None;
    let mut grfstd: u16 = 0;
    if st.hidden {
        grfstd |= 0x0002 | 0x0100; // fHidden, fSemiHidden
    }
    if st.locked {
        grfstd |= 0x0200;
    }
    if st.quick {
        grfstd |= 0x1000; // fQFormat
    }
    let (stk, cupx, upx): (u16, u16, Vec<Vec<u8>>) = match st.kind {
        StyleKind::Paragraph => {
            let mut papx = istd_self.to_le_bytes().to_vec();
            let mut g = grpprl::papx(&para, ctx);
            g.truncate_to(super::fkp::MAX_PAPX_GRPPRL);
            papx.extend_from_slice(&g.0);
            let mut c = grpprl::chpx(&chr, ctx);
            c.truncate_to(255);
            (1, 2, vec![papx, c.0])
        }
        StyleKind::Character => {
            let mut c = grpprl::chpx(&chr, ctx);
            c.truncate_to(255);
            (2, 1, vec![c.0])
        }
        // Table styles: their paragraph and character formatting; the conditional formats stay
        // with the model (tables carry their own borders and shading when written).
        StyleKind::Table => {
            let mut papx = istd_self.to_le_bytes().to_vec();
            papx.extend_from_slice(&grpprl::papx(&para, ctx).0);
            (3, 3, vec![Vec::new(), papx, grpprl::chpx(&chr, ctx).0])
        }
        StyleKind::Numbering => {
            let mut papx = istd_self.to_le_bytes().to_vec();
            papx.extend_from_slice(&Grpprl::default().0);
            (4, 1, vec![papx])
        }
    };
    assemble(sti, stk, base, next, cupx, grfstd, name, &upx)
}

/// `StdfBase` + `xstzName` + `GrLPUpxSw` (each UPX padded to an even size).
#[allow(clippy::too_many_arguments)]
fn assemble(sti: u16, stk: u16, base: u16, next: u16, cupx: u16, grfstd: u16, name: &str, upx: &[Vec<u8>]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(sti & 0x0FFF).to_le_bytes());
    v.extend_from_slice(&((stk & 0xF) | ((base & 0x0FFF) << 4)).to_le_bytes());
    v.extend_from_slice(&((cupx & 0xF) | ((next & 0x0FFF) << 4)).to_le_bytes());
    let bch_at = v.len();
    v.extend_from_slice(&0u16.to_le_bytes()); // bchUpe, filled below
    v.extend_from_slice(&grfstd.to_le_bytes());
    let units: Vec<u16> = name.encode_utf16().take(253).collect();
    v.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for u in &units {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v.extend_from_slice(&0u16.to_le_bytes());
    for u in upx {
        v.extend_from_slice(&(u.len() as u16).to_le_bytes());
        v.extend_from_slice(u);
        if u.len() % 2 == 1 {
            v.push(0);
        }
    }
    let len = v.len() as u16;
    if let Some(b) = v.get_mut(bch_at..bch_at + 2) {
        b.copy_from_slice(&len.to_le_bytes());
    }
    v
}

/// `over`'s character properties over `under`'s.
fn overlay(under: &CharProps, over: &CharProps) -> CharProps {
    let mut out = under.clone();
    out.overlay(over);
    out
}

fn overlay_para(under: &wordcraft_doc::props::ParaProps, over: &wordcraft_doc::props::ParaProps) -> wordcraft_doc::props::ParaProps {
    let mut out = under.clone();
    out.overlay(over);
    out
}

/// The `SttbfFfn`: a non-extended STTB whose entries are `FFN`s (a byte count, then the FFN).
pub(crate) fn font_table(names: &[String]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(names.len().min(0x7FFF) as u16).to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes()); // cbExtra
    for name in names.iter().take(0x7FFF) {
        let ffn = ffn(name);
        v.push(ffn.len() as u8);
        v.extend_from_slice(&ffn);
    }
    v
}

/// One `FFN` ([MS-DOC] §2.9.82): pitch/TrueType/family bits, `wWeight`, `chs`,
/// `ixchSzAlt`, `panose`, `fs`, then the name, NUL-terminated UTF-16.
fn ffn(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().take(31).collect();
    let mut v = Vec::with_capacity(40 + units.len() * 2 + 2);
    let symbol = matches!(name.to_ascii_lowercase().as_str(), "symbol" | "wingdings" | "wingdings 2" | "wingdings 3" | "webdings");
    let family: u8 = if symbol {
        5 // FF_DECORATIVE
    } else if name.to_ascii_lowercase().contains("mono") || name.eq_ignore_ascii_case("courier new") {
        3 // FF_MODERN
    } else if name.to_ascii_lowercase().contains("sans")
        || ["arial", "calibri", "aptos", "verdana", "tahoma", "segoe ui"].contains(&name.to_ascii_lowercase().as_str())
    {
        2 // FF_SWISS
    } else {
        1 // FF_ROMAN
    };
    // prq (2 = variable pitch), fTrueType, ff.
    v.push(2 | 0x04 | (family << 4));
    v.extend_from_slice(&400u16.to_le_bytes()); // wWeight
    v.push(if symbol { 2 } else { 0 }); // chs: SYMBOL_CHARSET or ANSI_CHARSET
    v.push(0); // ixchSzAlt: no alternate name
    v.extend_from_slice(&[0u8; 10]); // panose
    v.extend_from_slice(&[0u8; 24]); // fs
    for u in &units {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v.extend_from_slice(&0u16.to_le_bytes());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_table_reads_back() {
        let names = vec!["Times New Roman".to_string(), "Symbol".to_string(), "Noto Sans Ünï".to_string()];
        let t = font_table(&names);
        assert_eq!(crate::stsh::fonts(&t, 0, t.len() as u32), names);
    }
}
