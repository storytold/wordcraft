//! Sections and headers/footers: `PlcfSed` → `Sed` → `Sepx` (grpprls of section sprms) and
//! the `Plcfhdd` stories of the header subdocument ([MS-DOC] §2.8.25, §2.9.244).

use wordcraft_doc::section::{SectionProps, SectionStart};

use crate::fib::{Fib, pair};
use crate::sprm::{self, Prl};

/// Most sections we read.
const MAX_SECTIONS: usize = 1024;

// Section sprms ([MS-DOC] §2.6.4).
const S_BKC: u16 = 0x3009;
const S_TITLE_PAGE: u16 = 0x300A; // sprmSFTitlePage
const S_CCOLUMNS: u16 = 0x500B;
const S_PGN_RESTART: u16 = 0x3011; // sprmSFPgnRestart
const S_PGN_START: u16 = 0x501C; // sprmSPgnStart97
const S_PGN_START32: u16 = 0x7044; // sprmSPgnStart, the 32-bit form
const S_NFC_PGN: u16 = 0x300E;
const S_DXA_COLUMNS: u16 = 0x900C;
const S_VJC: u16 = 0x301A;
const S_ORIENTATION: u16 = 0x301D; // sprmSBOrientation
const S_HDR_TOP: u16 = 0xB017;
const S_HDR_BOTTOM: u16 = 0xB018;
const S_XA_PAGE: u16 = 0xB01F;
const S_YA_PAGE: u16 = 0xB020;
const S_DXA_LEFT: u16 = 0xB021;
const S_DXA_RIGHT: u16 = 0xB022;
const S_DYA_TOP: u16 = 0x9023;
const S_DYA_BOTTOM: u16 = 0x9024;
const S_DXA_GUTTER: u16 = 0xB025;

/// One section: covers paragraphs up to (excluding) `end_cp`, with its properties.
pub(crate) struct Section {
    pub(crate) end_cp: u32,
    pub(crate) props: SectionProps,
}

/// Parse `PlcfSed` and each section's `Sepx` grpprl.
pub(crate) fn parse(word: &[u8], table: &[u8], fib: &Fib) -> Vec<Section> {
    let Some((fc, lcb)) = fib.pair(pair::PLCF_SED) else { return Vec::new() };
    let Some(plc) = table.get(fc as usize..(fc as usize).saturating_add(lcb as usize)) else { return Vec::new() };
    // PlcSed: aCp[n+1] u32s + aSed[n] (Sed = fn 2, fcSepx 4, fnMpr 2, fcMpr 4).
    if plc.len() < 4 || (plc.len() - 4) % 16 != 0 {
        return Vec::new();
    }
    let n = (plc.len() - 4) / 16;
    let cp_at = |i: usize| -> Option<u32> { plc.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
    let mut out = Vec::with_capacity(n.min(MAX_SECTIONS));
    for i in 0..n.min(MAX_SECTIONS) {
        let Some(end_cp) = cp_at(i + 1) else { break };
        let sed_at = (n + 1) * 4 + i * 12;
        let fc_sepx = plc.get(sed_at + 2..sed_at + 6).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0xFFFF_FFFF);
        // Sepx: cb u16, then a grpprl of section sprms.
        let grpprl = word.get(fc_sepx as usize..).and_then(|sepx| sepx.get(..2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)).and_then(|cb| {
            let at = (fc_sepx as usize).checked_add(2)?;
            word.get(at..at.checked_add(cb)?)
        });
        out.push(Section { end_cp, props: sepx_props(grpprl.unwrap_or_default()) });
    }
    out
}

/// A section's properties from its Sepx grpprl.
fn sepx_props(grpprl: &[u8]) -> SectionProps {
    let mut props = SectionProps::default();
    let (mut explicit_orient, mut restart) = (false, false);
    for prl in sprm::iter(grpprl) {
        // Only a defined operand counts as explicit; anything else leaves the
        // orientation to the page shape.
        if prl.op == S_ORIENTATION && matches!(prl.operand.first(), Some(1 | 2)) {
            explicit_orient = true;
        }
        if prl.op == S_PGN_RESTART {
            restart = prl.operand.first().is_some_and(|&v| v != 0);
        }
        apply_section(&mut props, &prl);
    }
    // The start number counts only when the section restarts its page numbers
    // ([MS-DOC] sprmSPgnStart97), whichever of the two comes first. A restart without a
    // start number restarts at 0, the sprms' default.
    if !restart {
        props.page_num_start = None;
    } else if props.page_num_start.is_none() {
        props.page_num_start = Some(0);
    }
    // Without an explicit orientation sprm, infer it from the page shape.
    if !explicit_orient {
        if props.page_w < props.page_h {
            props.landscape = false;
        } else if props.page_w > props.page_h {
            props.landscape = true;
        }
    }
    props
}

/// Apply a section sprm to the model's section properties. All the measurement sprms here
/// carry 16-bit twip operands ([MS-DOC] §2.6.4).
fn apply_section(s: &mut SectionProps, prl: &Prl) {
    let tw = |v: i32| (v as f32 / 20.0).clamp(0.0, 1584.0);
    let s16 = |prl: &Prl| -> Option<i16> {
        match prl.operand {
            [b0, b1, ..] => Some(i16::from_le_bytes([*b0, *b1])),
            _ => None,
        }
    };
    match prl.op {
        S_BKC => {
            s.start = match prl.operand.first().copied().unwrap_or(2) {
                // SBkcOperand ([MS-DOC] sprmSBkc): bkcContinuous, bkcNewColumn, bkcNewPage,
                // bkcEvenPage, bkcOddPage; anything else falls back to the default (new page).
                0 => SectionStart::Continuous,
                1 => SectionStart::NextColumn,
                3 => SectionStart::EvenPage,
                4 => SectionStart::OddPage,
                _ => SectionStart::NextPage,
            };
        }
        S_TITLE_PAGE => s.title_page = prl.operand.first().is_some_and(|&v| v != 0),
        S_CCOLUMNS => {
            if let Some(c) = s16(prl) {
                s.columns.count = (c as u32).saturating_add(1).min(44);
            }
        }
        S_PGN_START => {
            if let Some(v) = s16(prl) {
                s.page_num_start = Some(v.max(0) as u32);
            }
        }
        S_PGN_START32 => {
            if let [b0, b1, b2, b3, ..] = prl.operand {
                s.page_num_start = Some(u32::from_le_bytes([*b0, *b1, *b2, *b3]).min(2_147_483_646));
            }
        }
        S_NFC_PGN => {
            if let Some(&nfc) = prl.operand.first() {
                s.page_num_format = crate::list::num_format(nfc);
            }
        }
        S_DXA_COLUMNS => {
            if let Some(v) = s16(prl) {
                s.columns.space = tw(v as i32);
            }
        }
        S_VJC => {
            s.valign = match prl.operand.first() {
                Some(1) => wordcraft_doc::section::PageVAlign::Center,
                Some(2) => wordcraft_doc::section::PageVAlign::Justify,
                Some(3) => wordcraft_doc::section::PageVAlign::Bottom,
                _ => wordcraft_doc::section::PageVAlign::Top,
            };
        }
        // SBOrientationOperand ([MS-DOC] §2.9.236): 1 portrait, 2 landscape.
        S_ORIENTATION => match prl.operand.first() {
            Some(1) => s.landscape = false,
            Some(2) => s.landscape = true,
            _ => {}
        },
        S_HDR_TOP => {
            if let Some(v) = s16(prl) {
                s.header = tw(v as i32);
            }
        }
        S_HDR_BOTTOM => {
            if let Some(v) = s16(prl) {
                s.footer = tw(v as i32);
            }
        }
        S_XA_PAGE => {
            if let Some(v) = s16(prl) {
                s.page_w = tw(v as i32).max(7.2);
            }
        }
        S_YA_PAGE => {
            if let Some(v) = s16(prl) {
                s.page_h = tw(v as i32).max(7.2);
            }
        }
        S_DXA_LEFT => {
            if let Some(v) = s16(prl) {
                s.margin_left = tw(v as i32);
            }
        }
        S_DXA_RIGHT => {
            if let Some(v) = s16(prl) {
                s.margin_right = tw(v as i32);
            }
        }
        S_DYA_TOP => {
            if let Some(v) = s16(prl) {
                s.margin_top = tw(v as i32);
            }
        }
        S_DYA_BOTTOM => {
            if let Some(v) = s16(prl) {
                s.margin_bottom = tw(v as i32);
            }
        }
        S_DXA_GUTTER => {
            if let Some(v) = s16(prl) {
                s.gutter = tw(v as i32);
            }
        }
        _ => {}
    }
}

/// The header stories' CP ranges from `Plcfhdd` (a PLC of bare CPs): story i spans
/// `[cps[i], cps[i+1])`. Stories 0-5 are footnote/endnote separators; every section then owns
/// six stories in the order even header, odd (default) header, even footer, odd (default)
/// footer, first header, first footer. The CPs are relative to the header subdocument.
///
/// The CPs of a well-formed PLC ascend, so the stories are disjoint. A story starting before
/// the end of an earlier one (a hostile table pointing many sections at the same text, each
/// read into its own part) is left empty, like a slot linked to the previous section's:
/// every subdocument CP becomes at most one story.
pub(crate) fn header_stories(table: &[u8], fib: &Fib) -> Vec<(u32, u32)> {
    let Some((fc, lcb)) = fib.pair(pair::PLCF_HDD) else { return Vec::new() };
    let Some(plc) = table.get(fc as usize..(fc as usize).saturating_add(lcb as usize)) else { return Vec::new() };
    // Only the six separator stories and six per section (at most 1024) are ever used.
    let n = (lcb as usize / 4).min(6 + 6 * MAX_SECTIONS + 1);
    let cp_at = |i: usize| -> Option<u32> { plc.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
    let mut stories = Vec::with_capacity(n);
    let mut kept_to = 0u32;
    for i in 0..n.saturating_sub(1) {
        if let (Some(a), Some(b)) = (cp_at(i), cp_at(i + 1)) {
            let b = if a < kept_to { a } else { b.max(a) };
            kept_to = kept_to.max(b);
            stories.push((a, b));
        }
    }
    stories
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sprmSPgnStart97 alone doesn't restart the numbering: only with sprmSFPgnRestart on, in
    /// either order.
    #[test]
    fn a_page_number_start_counts_only_with_a_restart() {
        let start = [0x1C, 0x50, 5, 0];
        let restart = [0x11, 0x30, 1];
        let off = [0x11, 0x30, 0];
        assert_eq!(sepx_props(&start).page_num_start, None);
        assert_eq!(sepx_props(&[&off[..], &start].concat()).page_num_start, None);
        assert_eq!(sepx_props(&[&restart[..], &start].concat()).page_num_start, Some(5));
        assert_eq!(sepx_props(&[&start[..], &restart].concat()).page_num_start, Some(5));
    }

    /// A restart without a start number restarts at 0 ([MS-DOC] sprmSPgnStart97's default),
    /// and the 32-bit sprmSPgnStart gives the start number as well.
    #[test]
    fn a_page_number_restart_without_a_start_restarts_at_zero() {
        let restart = [0x11, 0x30, 1];
        assert_eq!(sepx_props(&restart).page_num_start, Some(0));
        let start32 = [0x44, 0x70, 0x40, 0x42, 0x0F, 0x00];
        assert_eq!(sepx_props(&[&restart[..], &start32].concat()).page_num_start, Some(1_000_000));
        assert_eq!(sepx_props(&start32).page_num_start, None);
    }
}
