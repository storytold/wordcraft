//! Tracked insertions and deletions: the revision-mark sprms of a run's CHPX (`sprmCFRMarkIns`
//! and `sprmCFRMarkDel`, each with its author index into `SttbfRMark` and its `DTTM`)
//! ([MS-DOC] §2.6.1, §2.9.290), as model revisions.

use std::cell::RefCell;
use std::collections::HashMap;

use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Revision, RevisionKind};

use crate::fib::Fib;
use crate::piece::PrmRef;
use crate::sprm;

const C_RMARK_DEL: u16 = 0x0800;
const C_RMARK_INS: u16 = 0x0801;
const C_IBST_RMARK: u16 = 0x4804;
const C_DTTM_RMARK: u16 = 0x6805;
const C_IBST_RMARK_DEL: u16 = 0x4863;
const C_DTTM_RMARK_DEL: u16 = 0x6864;
/// `fcSttbfRMark`: the revision authors.
const STTBF_RMARK: usize = 51;
/// Most distinct revisions (author, time and kind) one document gets.
const MAX_REVISIONS: usize = 100_000;
/// Longest author name kept.
const MAX_AUTHOR: usize = 255;

/// The document's revisions, made as the walk meets their marks: runs with the same author,
/// time and kind share one.
pub(crate) struct Revisions {
    authors: Vec<String>,
    list: RefCell<Vec<Revision>>,
    index: RefCell<HashMap<(bool, u16, u32), u32>>,
}

impl Revisions {
    pub(crate) fn new(table: &[u8], fib: &Fib) -> Self {
        let authors = fib
            .pair(STTBF_RMARK)
            .filter(|(_, lcb)| *lcb > 0)
            .and_then(|(fc, lcb)| crate::notes::sttb_strings_max(table, fc, lcb, MAX_AUTHOR))
            .unwrap_or_default();
        Revisions { authors, list: RefCell::new(Vec::new()), index: RefCell::new(HashMap::new()) }
    }

    /// Set `props.ins` / `props.del` from the revision marks of a run: those in its CHPX
    /// `grpprl`, then those of its piece's `prm`, which apply after the CHPX's.
    pub(crate) fn mark(&self, props: &mut CharProps, grpprl: &[u8], prm: PrmRef<'_>) {
        let mut m = Marks::default();
        for prl in sprm::iter(grpprl) {
            m.apply(prl.op, prl.operand);
        }
        match prm {
            PrmRef::Sprm0 { isprm, val } => {
                if let Some(op) = sprm::from_prm0(isprm) {
                    m.apply(op, &[val]);
                }
            }
            PrmRef::Grpprl(Some(g)) => {
                for prl in sprm::iter(g) {
                    m.apply(prl.op, prl.operand);
                }
            }
            PrmRef::Grpprl(None) | PrmRef::None => {}
        }
        if m.ins {
            props.ins = self.id(true, m.ibst_ins, m.dttm_ins);
        }
        if m.del {
            props.del = self.id(false, m.ibst_del, m.dttm_del);
        }
    }

    /// The index of the revision of this kind, author and time, made on first use.
    fn id(&self, insert: bool, ibst: u16, dttm: u32) -> Option<u32> {
        let key = (insert, ibst, dttm);
        if let Some(&i) = self.index.borrow().get(&key) {
            return Some(i);
        }
        let mut list = self.list.borrow_mut();
        if list.len() >= MAX_REVISIONS {
            return None;
        }
        let author = self.authors.get(usize::from(ibst)).cloned().unwrap_or_else(|| "Unknown".to_string());
        let kind = if insert { RevisionKind::Insert } else { RevisionKind::Delete };
        list.push(Revision::new(kind, author, dttm_iso(dttm)));
        let i = u32::try_from(list.len() - 1).ok()?;
        self.index.borrow_mut().insert(key, i);
        Some(i)
    }

    pub(crate) fn take(self) -> Vec<Revision> {
        self.list.into_inner()
    }
}

/// The revision-mark sprms seen so far for one run, a later one replacing an earlier one.
#[derive(Default)]
struct Marks {
    ins: bool,
    del: bool,
    ibst_ins: u16,
    dttm_ins: u32,
    ibst_del: u16,
    dttm_del: u32,
}

impl Marks {
    fn apply(&mut self, op: u16, operand: &[u8]) {
        let u16v = || operand.get(..2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        let u32v = || operand.get(..4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        match op {
            C_RMARK_INS => self.ins = operand.first().is_some_and(|&v| v == 1 || v == 0x81),
            C_RMARK_DEL => self.del = operand.first().is_some_and(|&v| v == 1 || v == 0x81),
            C_IBST_RMARK => self.ibst_ins = u16v(),
            C_DTTM_RMARK => self.dttm_ins = u32v(),
            C_IBST_RMARK_DEL => self.ibst_del = u16v(),
            C_DTTM_RMARK_DEL => self.dttm_del = u32v(),
            _ => {}
        }
    }
}

/// A `DTTM` ([MS-DOC] §2.9.75) as an ISO 8601 UTC time; empty for 0 or an impossible date.
pub(crate) fn dttm_iso(d: u32) -> String {
    if d == 0 {
        return String::new();
    }
    let minute = d & 0x3F;
    let hour = (d >> 6) & 0x1F;
    let day = (d >> 11) & 0x1F;
    let month = (d >> 16) & 0xF;
    let year = 1900 + ((d >> 20) & 0x1FF) as i32;
    match chrono::NaiveDate::from_ymd_opt(year, month, day).and_then(|date| date.and_hms_opt(hour, minute, 0)) {
        Some(t) => t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revisions() -> Revisions {
        Revisions { authors: vec!["Ada".into(), "Grace".into()], list: RefCell::new(Vec::new()), index: RefCell::new(HashMap::new()) }
    }

    fn marked(chpx: &[u8], prm: PrmRef<'_>) -> (CharProps, Vec<Revision>) {
        let r = revisions();
        let mut props = CharProps::default();
        r.mark(&mut props, chpx, prm);
        (props, r.take())
    }

    const INS: [u8; 3] = [0x01, 0x08, 1];
    const NOT_INS: [u8; 3] = [0x01, 0x08, 0];
    /// sprmCFRMarkDel on, by the author at index 1.
    const DEL_BY_GRACE: [u8; 7] = [0x00, 0x08, 1, 0x63, 0x48, 1, 0];

    #[test]
    fn a_piece_prm_marks_a_run() {
        // Prm1: a grpprl from the piece table's Prc.
        let (p, revs) = marked(&[], PrmRef::Grpprl(Some(&DEL_BY_GRACE)));
        assert_eq!(p.del, Some(0));
        assert_eq!((revs[0].kind, revs[0].author.as_str()), (RevisionKind::Delete, "Grace"));
        // Prm0: sprmCFRMarkIns by its isprm.
        let (p, revs) = marked(&[], PrmRef::Sprm0 { isprm: 0x42, val: 1 });
        assert_eq!(p.ins, Some(0));
        assert_eq!((revs[0].kind, revs[0].author.as_str()), (RevisionKind::Insert, "Ada"));
    }

    #[test]
    fn the_piece_prm_applies_after_the_chpx() {
        let (p, _) = marked(&INS, PrmRef::Grpprl(Some(&NOT_INS)));
        assert_eq!(p.ins, None, "the Prm turns the CHPX's insertion off");
        let (p, _) = marked(&NOT_INS, PrmRef::Sprm0 { isprm: 0x42, val: 1 });
        assert_eq!(p.ins, Some(0));
        let (p, _) = marked(&INS, PrmRef::None);
        assert_eq!(p.ins, Some(0));
    }
}
