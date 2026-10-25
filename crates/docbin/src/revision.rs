//! Tracked changes ([MS-DOC] §2.6.1): a run is an insertion or a deletion through the
//! revision-mark character sprms, which also carry the author (an index into `SttbfRMark`,
//! §2.9.276) and the date (a DTTM, §2.9.70). Paragraph marks carry them the same way, so a
//! deleted mark joins its paragraph with the next one when the change is accepted.
//!
//! The reader collects every distinct (kind, author, date) into [`Revisions`], which becomes
//! `Document::revisions`; runs point at it through `CharProps::ins` / `CharProps::del`, like
//! the DOCX reader's `w:ins` / `w:del`.

use std::collections::HashMap;

use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Revision, RevisionKind};

use crate::sprm::{self, Prl, Toggle};

/// `sprmCFRMarkDel` / `sprmCFRMarkIns`: the run is a tracked deletion / insertion.
const C_RMARK_DEL: u16 = 0x0800;
const C_RMARK_INS: u16 = 0x0801;
/// `sprmCIbstRMark` / `sprmCDttmRMark`: author index and date of the insertion.
const C_IBST_RMARK: u16 = 0x4804;
const C_DTTM_RMARK: u16 = 0x6805;
/// `sprmCIbstRMarkDel` / `sprmCDttmRMarkDel`: author index and date of the deletion.
const C_IBST_RMARK_DEL: u16 = 0x4863;
const C_DTTM_RMARK_DEL: u16 = 0x6864;

/// Most distinct revisions one document may create. Dates have minute precision, so a real
/// document stays far below this; a hostile one reuses an existing revision once it is full.
const MAX_REVISIONS: usize = 1 << 16;
/// Longest author name kept (characters), matching the DOCX reader.
const MAX_AUTHOR: usize = 256;

/// The revision-mark sprms of one character, as read from its CHPX and its piece's Prm.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Marks {
    ins: bool,
    del: bool,
    ibst: u16,
    dttm: u32,
    ibst_del: u16,
    dttm_del: u32,
}

impl Marks {
    /// Marks from a grpprl (later Prls override earlier ones).
    pub(crate) fn of(grpprl: &[u8]) -> Marks {
        let mut m = Marks::default();
        for prl in sprm::iter(grpprl) {
            m.apply(&prl);
        }
        m
    }

    /// Apply one Prl; anything but a revision-mark sprm is ignored.
    pub(crate) fn apply(&mut self, prl: &Prl) {
        let u16_op = || match prl.operand {
            [b0, b1, ..] => u16::from_le_bytes([*b0, *b1]),
            _ => 0,
        };
        let u32_op = || match prl.operand {
            [b0, b1, b2, b3, ..] => u32::from_le_bytes([*b0, *b1, *b2, *b3]),
            _ => 0,
        };
        match prl.op {
            C_RMARK_INS => toggle(&mut self.ins, prl),
            C_RMARK_DEL => toggle(&mut self.del, prl),
            C_IBST_RMARK => self.ibst = u16_op(),
            C_DTTM_RMARK => self.dttm = u32_op(),
            C_IBST_RMARK_DEL => self.ibst_del = u16_op(),
            C_DTTM_RMARK_DEL => self.dttm_del = u32_op(),
            _ => {}
        }
    }
}

/// A revision-mark ToggleOperand. Styles never carry revision marks, so the base is off.
fn toggle(flag: &mut bool, prl: &Prl) {
    match sprm::toggle(prl) {
        Toggle::On => *flag = true,
        Toggle::Off => *flag = false,
        Toggle::Invert => *flag = !*flag,
        Toggle::Keep => {}
    }
}

/// The document's revisions, built while walking its stories.
#[derive(Debug, Default)]
pub(crate) struct Revisions {
    authors: Vec<String>,
    list: Vec<Revision>,
    index: HashMap<(bool, String, String), u32>,
    /// The same by raw (kind, author index, DTTM), so a tracked character costs no allocation.
    raw: HashMap<(bool, u16, u32), u32>,
    /// The first insertion and deletion, which take new changes once the list is full.
    first: [Option<u32>; 2],
}

impl Revisions {
    /// Start with the author names of `SttbfRMark`.
    pub(crate) fn new(authors: Vec<String>) -> Revisions {
        Revisions { authors, ..Default::default() }
    }

    /// Point `props` at the revisions its marks describe (clearing what they don't).
    pub(crate) fn mark(&mut self, props: &mut CharProps, m: &Marks) {
        props.ins = if m.ins { self.intern(false, m.ibst, m.dttm) } else { None };
        props.del = if m.del { self.intern(true, m.ibst_del, m.dttm_del) } else { None };
    }

    /// The revision index for (kind, author index, DTTM), created on first use.
    fn intern(&mut self, delete: bool, ibst: u16, dttm: u32) -> Option<u32> {
        if let Some(i) = self.raw.get(&(delete, ibst, dttm)) {
            return Some(*i);
        }
        let i = self.intern_named(delete, ibst, dttm)?;
        if self.raw.len() < MAX_REVISIONS {
            self.raw.insert((delete, ibst, dttm), i);
        }
        Some(i)
    }

    /// [`Revisions::intern`] by author name and date: two author indices naming the same person,
    /// or two DTTMs giving the same date, are one revision.
    fn intern_named(&mut self, delete: bool, ibst: u16, dttm: u32) -> Option<u32> {
        let author: String = self.authors.get(usize::from(ibst)).map(|a| a.chars().take(MAX_AUTHOR).collect()).unwrap_or_default();
        let key = (delete, author, date(dttm));
        if let Some(i) = self.index.get(&key) {
            return Some(*i);
        }
        let first = &mut self.first[usize::from(delete)];
        if self.list.len() >= MAX_REVISIONS {
            // Full: the change still shows, under the first revision of its kind.
            return *first;
        }
        let i = u32::try_from(self.list.len()).ok()?;
        let kind = if delete { RevisionKind::Delete } else { RevisionKind::Insert };
        self.list.push(Revision::new(kind, key.1.clone(), key.2.clone()));
        first.get_or_insert(i);
        self.index.insert(key, i);
        Some(i)
    }

    /// The revisions, in order of first use: `Document::revisions`.
    pub(crate) fn into_list(self) -> Vec<Revision> {
        self.list
    }
}

/// A DTTM ([MS-DOC] §2.9.70) as an ISO 8601 UTC timestamp, the form `w:date` uses: bits
/// 0–5 minute, 6–10 hour, 11–15 day, 16–19 month, 20–28 years since 1900, 29–31 weekday.
/// Zero (no date) and impossible dates (30 February too) give an empty string.
pub(crate) fn date(dttm: u32) -> String {
    let minute = dttm & 0x3F;
    let hour = (dttm >> 6) & 0x1F;
    let day = (dttm >> 11) & 0x1F;
    let month = (dttm >> 16) & 0x0F;
    let year = 1900 + ((dttm >> 20) & 0x1FF);
    let real = i32::try_from(year).ok().and_then(|y| chrono::NaiveDate::from_ymd_opt(y, month, day)).is_some();
    if dttm == 0 || minute > 59 || hour > 23 || !real {
        return String::new();
    }
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:00Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dttm(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> u32 {
        minute | hour << 6 | day << 11 | month << 16 | (year - 1900) << 20
    }

    #[test]
    fn dttm_decodes_to_an_iso_date() {
        assert_eq!(date(dttm(2015, 5, 25, 13, 47)), "2015-05-25T13:47:00Z");
        assert_eq!(date(dttm(2016, 12, 31, 0, 0) | 6 << 29), "2016-12-31T00:00:00Z");
    }

    #[test]
    fn missing_or_impossible_dates_are_empty() {
        assert_eq!(date(0), "");
        assert_eq!(date(dttm(2015, 13, 1, 0, 0)), "");
        assert_eq!(date(dttm(2015, 1, 0, 0, 0)), "");
        assert_eq!(date(dttm(2015, 2, 29, 0, 0)), "", "2015 isn't a leap year");
        assert_eq!(date(dttm(2015, 4, 31, 0, 0)), "");
        assert_eq!(date(dttm(2016, 2, 29, 0, 0)), "2016-02-29T00:00:00Z");
        assert_eq!(date(u32::MAX), "");
    }

    #[test]
    fn same_author_and_date_share_one_revision() {
        let mut revs = Revisions::new(vec!["Alice".into(), "Bob".into()]);
        let ins = Marks { ins: true, ibst: 1, dttm: dttm(2020, 1, 2, 3, 4), ..Default::default() };
        let (mut a, mut b) = (CharProps::default(), CharProps::default());
        revs.mark(&mut a, &ins);
        revs.mark(&mut b, &ins);
        assert_eq!(a.ins, Some(0));
        assert_eq!(b.ins, Some(0));
        let del = Marks { del: true, ibst_del: 7, ..Default::default() }; // author out of range
        revs.mark(&mut b, &del);
        assert_eq!((b.ins, b.del), (None, Some(1)));
        let list = revs.into_list();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].kind, list[0].author.as_str(), list[0].date.as_str()), (RevisionKind::Insert, "Bob", "2020-01-02T03:04:00Z"));
        assert_eq!((list[1].kind, list[1].author.as_str(), list[1].date.as_str()), (RevisionKind::Delete, "", ""));
    }

    #[test]
    fn a_full_table_reuses_a_revision_of_the_same_kind() {
        let mut revs = Revisions::new(Vec::new());
        let mut p = CharProps::default();
        for i in 0..MAX_REVISIONS as u32 {
            let at = dttm(2000, 1 + i / 40_320 % 12, 1 + i / 1440 % 28, i / 60 % 24, i % 60);
            revs.mark(&mut p, &Marks { ins: true, dttm: at, ..Default::default() });
        }
        revs.mark(&mut p, &Marks { del: true, dttm: dttm(2020, 1, 1, 0, 0), ..Default::default() });
        assert_eq!(p.del, None, "no delete revision exists to reuse");
        revs.mark(&mut p, &Marks { ins: true, dttm: dttm(2021, 1, 1, 0, 0), ..Default::default() });
        assert_eq!(p.ins, Some(0));
        assert_eq!(revs.into_list().len(), MAX_REVISIONS);
    }
}
