//! Sprm decoding ([MS-DOC] §2.6). A Prl is a 2-byte opcode followed by an operand whose
//! size comes from the opcode's `spra` field (with two variable-length exceptions).

/// `sprmPChgTabs`: custom tab stops ([MS-DOC] §2.6.29 — handled in a later pass).
const P_CHG_TABS: u16 = 0xC615;
/// `sprmTDefTable`: table cell borders and widths.
const T_DEF_TABLE: u16 = 0xD608;

/// One decoded Prl: an opcode plus its operand bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Prl<'a> {
    pub(crate) op: u16,
    pub(crate) operand: &'a [u8],
}

impl Prl<'_> {
    /// Which kind of property this sprm modifies: 1 paragraph, 2 character, 3 picture,
    /// 4 section, 5 table ([MS-DOC] Sprm.sgc).
    pub(crate) fn sgc(&self) -> u8 {
        ((self.op >> 10) & 7) as u8
    }
}

/// Iterate the Prls of a grpprl. Malformed tails are dropped.
pub(crate) fn iter(grpprl: &[u8]) -> impl Iterator<Item = Prl<'_>> {
    let mut rest = grpprl;
    std::iter::from_fn(move || {
        if rest.len() < 2 {
            return None;
        }
        let op = u16::from_le_bytes([rest[0], rest[1]]);
        let after = &rest[2..];
        // Operand size per spra, with the two exceptions whose size byte lies.
        let len = match (op >> 13, op) {
            (_, P_CHG_TABS) => after.get(..2).map(|b| 2 + u16::from_le_bytes([b[0], b[1]]) as usize),
            (_, T_DEF_TABLE) => after.get(..2).map(|b| 2 + u16::from_le_bytes([b[0], b[1]]) as usize),
            (0, _) | (1, _) => Some(1),
            (2, _) | (4, _) | (5, _) => Some(2),
            (3, _) => Some(4),
            (7, _) => Some(3),
            (6, _) => after.first().map(|&n| 1 + n as usize),
            _ => None,
        }?;
        let operand = after.get(..len)?;
        rest = &after[len.min(after.len())..];
        Some(Prl { op, operand })
    })
}

/// A ToggleOperand value: 0 = off, 1 = on, 0x80 = leave unchanged, 0x81 = invert.
pub(crate) enum Toggle {
    Off,
    On,
    Keep,
    Invert,
}

pub(crate) fn toggle(p: &Prl) -> Toggle {
    match p.operand.first().copied().unwrap_or(0) {
        0x81 => Toggle::Invert,
        0x80 => Toggle::Keep,
        0x00 => Toggle::Off,
        _ => Toggle::On,
    }
}

/// The isprm indices of Prm0 that map to sprms we understand ([MS-DOC] §2.9.215 table).
pub(crate) fn from_prm0(isprm: u8) -> Option<u16> {
    Some(match isprm {
        0x05 => 0x2461, // sprmPJc
        0x07 => 0x2405, // sprmPFKeep
        0x08 => 0x2406, // sprmPFKeepFollow
        0x09 => 0x2407, // sprmPFPageBreakBefore
        0x33 => 0x2435, // sprmPWidowControl (low byte of the operand is the value)
        0x78 => 0x2640, // sprmPOutLvl
        0x4D => 0x2A0C, // sprmCHighlight
        0x55 => 0x0835, // sprmCFBold
        0x56 => 0x0836, // sprmCFItalic
        0x57 => 0x0837, // sprmCFStrike
        0x58 => 0x0838, // sprmCFOutline
        0x59 => 0x0839, // sprmCFShadow
        0x5A => 0x083A, // sprmCFSmallCaps
        0x5B => 0x083B, // sprmCFCaps
        0x5C => 0x083C, // sprmCFVanish
        0x5E => 0x2A3E, // sprmCKul
        0x62 => 0x2A42, // sprmCIco
        0x74 => 0x0854, // sprmCFImprint
        0x75 => 0x0855, // sprmCFSpec
        _ => return None,
    })
}
