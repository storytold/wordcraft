//! Lists: `PlfLst` (one LSTF per abstract list, its nine LVLs after the array) and `PlfLfo`
//! (one LFO per model num, with LFOLVL start and level overrides) ([MS-DOC] §2.9.147, §2.9.150).

use std::collections::HashMap;

use wordcraft_doc::numbering::{Level, LevelSuffix, Numbering};
use wordcraft_doc::props::{Align, ParaProps};
use wordcraft_doc::section::NumFormat;

use super::grpprl::{self, CharCtx, ParaCtx};

/// Levels of a multi-level list.
const LEVELS: usize = 9;
/// Most lists written (the format counts LFOs from 1 to 0x07FE).
const MAX_LISTS: usize = 0x07FE;

/// The written lists: `PlfLst` (with its LVLs appended) and how much of it `lcbPlfLst` covers,
/// `PlfLfo`, and each model num id's LFO index.
pub(crate) struct Lists {
    pub(crate) plf_lst: Vec<u8>,
    pub(crate) lcb_plf_lst: u32,
    pub(crate) plf_lfo: Vec<u8>,
}

/// Each model num's 1-based LFO index, in `Numbering::nums` order.
pub(crate) fn lfo_indexes(n: &Numbering) -> HashMap<u32, u16> {
    n.nums.iter().take(MAX_LISTS).enumerate().map(|(i, num)| (num.id, (i + 1) as u16)).collect()
}

/// The list id (`lsid`) of abstract list `i`: any unique value other than 0 and -1.
fn lsid(i: usize) -> i32 {
    0x1000_0000 + i as i32
}

pub(crate) fn write(n: &Numbering, ctx: &mut (impl CharCtx + ParaCtx)) -> Option<Lists> {
    let abstracts: Vec<_> = n.abstracts.iter().take(MAX_LISTS).collect();
    if abstracts.is_empty() || n.nums.is_empty() {
        return None;
    }
    let mut lst = Vec::new();
    lst.extend_from_slice(&(abstracts.len() as i16).to_le_bytes());
    for (i, _) in abstracts.iter().enumerate() {
        // LSTF: lsid, tplc, rgistdPara[9] (none), flags (multi-level), grfhic.
        lst.extend_from_slice(&lsid(i).to_le_bytes());
        lst.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..LEVELS {
            lst.extend_from_slice(&0x0FFFu16.to_le_bytes());
        }
        lst.push(0);
        lst.push(0);
    }
    let lcb_plf_lst = lst.len() as u32;
    for a in &abstracts {
        for l in 0..LEVELS {
            let level = a.levels.get(l).cloned().unwrap_or_else(|| default_level(l));
            lst.extend_from_slice(&lvl(&level, ctx));
        }
    }

    let nums: Vec<_> = n.nums.iter().take(MAX_LISTS).collect();
    let mut lfo = Vec::new();
    lfo.extend_from_slice(&(nums.len() as i32).to_le_bytes());
    let mut data = Vec::new();
    for num in &nums {
        let ai = abstracts.iter().position(|a| a.id == num.abstract_id).unwrap_or(0);
        // The overrides per level: a start value, a whole level, or both.
        let mut over: Vec<(u8, Option<u32>, Option<&Level>)> = Vec::new();
        for &(l, start) in &num.start_overrides {
            if usize::from(l) < LEVELS {
                over.push((l, Some(start), None));
            }
        }
        for (l, level) in &num.level_overrides {
            if usize::from(*l) >= LEVELS {
                continue;
            }
            match over.iter_mut().find(|o| o.0 == *l) {
                Some(o) => o.2 = Some(level),
                None => over.push((*l, None, Some(level))),
            }
        }
        over.sort_by_key(|o| o.0);
        // LFO: lsid, unused1, unused2, clfolvl, ibstFltAutoNum, grfhic, unused3.
        lfo.extend_from_slice(&lsid(ai).to_le_bytes());
        lfo.extend_from_slice(&[0u8; 8]);
        lfo.push(over.len() as u8);
        lfo.extend_from_slice(&[0u8; 3]);
        // LFOData: cp (unused, 0xFFFFFFFF), then the LFOLVLs.
        data.extend_from_slice(&u32::MAX.to_le_bytes());
        for (l, start, level) in over {
            let start_at = start.or(level.map(|lv| lv.start)).unwrap_or(1).min(0x7FFF);
            data.extend_from_slice(&(start_at as i32).to_le_bytes());
            let mut flags = l & 0x0F;
            if start.is_some() {
                flags |= 0x10; // fStartAt
            }
            if level.is_some() {
                flags |= 0x20; // fFormatting
            }
            data.push(flags);
            data.extend_from_slice(&[0u8; 3]);
            if let Some(level) = level {
                data.extend_from_slice(&lvl(level, ctx));
            }
        }
    }
    lfo.extend_from_slice(&data);
    Some(Lists { plf_lst: lst, lcb_plf_lst, plf_lfo: lfo })
}

/// A level the model's list doesn't have: decimal, `%n.`, indented a quarter inch a level.
fn default_level(l: usize) -> Level {
    Level { start: 1, format: NumFormat::Decimal, text: format!("%{}.", l + 1), indent: 18.0 * (l as f32 + 1.0), hanging: 18.0, ..Level::default() }
}

/// One LVL: LVLF, grpprlPapx (the indents), grpprlChpx (the number's formatting), number text.
fn lvl(level: &Level, ctx: &mut (impl CharCtx + ParaCtx)) -> Vec<u8> {
    // Number text: `%n` placeholders become the level index (n − 1), their 1-based positions
    // go in rgbxchNums.
    let mut xst: Vec<u16> = Vec::new();
    let mut nums: Vec<u8> = Vec::new();
    let mut chars = level.text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%'
            && let Some(d) = chars.peek().and_then(|d| d.to_digit(10)).filter(|d| (1..=9).contains(d))
        {
            chars.next();
            xst.push((d - 1) as u16);
            if nums.len() < 9 {
                nums.push(xst.len() as u8);
            }
            continue;
        }
        let mut buf = [0u16; 2];
        xst.extend_from_slice(c.encode_utf16(&mut buf));
    }
    xst.truncate(64);
    nums.retain(|&p| usize::from(p) <= xst.len());

    let mut papx = grpprl::papx(&ParaProps { indent_left: Some(level.indent), indent_first: Some(-level.hanging), ..Default::default() }, ctx);
    papx.truncate_to(255);
    let mut chpx = grpprl::chpx(&level.chr, ctx);
    chpx.truncate_to(255);

    let mut v = Vec::with_capacity(28 + papx.len() + chpx.len() + 2 + xst.len() * 2);
    v.extend_from_slice(&(level.start.min(0x7FFF) as i32).to_le_bytes());
    v.push(grpprl::nfc(level.format));
    let jc: u8 = match level.align {
        Align::Center => 1,
        Align::Right => 2,
        _ => 0,
    };
    let mut flags = jc;
    if level.legal {
        flags |= 0x04;
    }
    if !level.restart {
        flags |= 0x08;
    }
    v.push(flags);
    let mut rgbxch = [0u8; 9];
    for (slot, p) in rgbxch.iter_mut().zip(&nums) {
        *slot = *p;
    }
    v.extend_from_slice(&rgbxch);
    v.push(match level.suffix {
        LevelSuffix::Tab => 0,
        LevelSuffix::Space => 1,
        LevelSuffix::Nothing => 2,
    });
    v.extend_from_slice(&0i32.to_le_bytes()); // dxaIndentSav
    v.extend_from_slice(&0u32.to_le_bytes()); // unused2
    v.push(chpx.len() as u8);
    v.push(papx.len() as u8);
    v.push(level.restart_after.map_or(0, |r| r.saturating_add(1)).min(LEVELS as u8));
    v.push(0); // grfhic
    v.extend_from_slice(&papx.0);
    v.extend_from_slice(&chpx.0);
    v.extend_from_slice(&(xst.len() as u16).to_le_bytes());
    for u in &xst {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v
}
