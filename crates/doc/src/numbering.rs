//! Lists: abstract numbering definitions (9 levels each), numbering instances, and the counter
//! state used to compute list labels.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::props::{Align, CharProps};
use crate::section::NumFormat;
use crate::{DocError, free_id};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LevelSuffix {
    #[default]
    Tab,
    Space,
    Nothing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Level {
    pub start: u32,
    pub format: NumFormat,
    /// Label template: `%1.`, `%1.%2.`, or the bullet character.
    pub text: String,
    pub align: Align,
    /// Left indent of the text, points.
    pub indent: f32,
    /// Hanging indent (label sits this far left of `indent`), points.
    pub hanging: f32,
    pub suffix: LevelSuffix,
    /// Label formatting (bullet font etc.).
    pub chr: CharProps,
    /// Restart after a higher level (true = after any higher level, the default).
    pub restart: bool,
    /// Restart only after this level (1-based, Word's `w:lvlRestart`): a level 3 that restarts
    /// after level 1 keeps counting across level 2 items. `None` restarts after the level just
    /// above. Ignored when `restart` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_after: Option<u8>,
    /// The tab stop that follows the number (Word's "Add tab stop at"), points from the
    /// paragraph's left edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab: Option<f32>,
    /// Show all levels as decimal (legal numbering).
    pub legal: bool,
    /// Paragraph style linked to this level (e.g. Heading 1 for outline numbering).
    pub style: Option<String>,
}

impl Default for Level {
    fn default() -> Self {
        Level {
            start: 1,
            format: NumFormat::Decimal,
            text: "%1.".into(),
            align: Align::Left,
            indent: 36.0,
            hanging: 18.0,
            suffix: LevelSuffix::Tab,
            chr: CharProps::default(),
            restart: true,
            restart_after: None,
            tab: None,
            legal: false,
            style: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AbstractNum {
    pub id: u32,
    pub name: Option<String>,
    pub levels: Vec<Level>,
    /// Word's list identity (`w:nsid`, 8 hex digits), which it uses to match lists across
    /// documents (paste, compare); kept from the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nsid: Option<String>,
    /// The list template the definition came from (`w:tmpl`); kept from the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmpl: Option<String>,
    /// `w:multiLevelType` from the file (`singleLevel`, `multilevel`, `hybridMultilevel`);
    /// `None`: derived from the levels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_level_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Num {
    pub id: u32,
    pub abstract_id: u32,
    /// Per-level start overrides (level, start).
    pub start_overrides: Vec<(u8, u32)>,
    /// Levels this list defines itself instead of taking them from its abstract list (level,
    /// definition): Word's `w:lvlOverride/w:lvl`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub level_overrides: Vec<(u8, Level)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Numbering {
    pub abstracts: Vec<AbstractNum>,
    pub nums: Vec<Num>,
}

/// Built-in list kinds for the Bullets / Numbering / Multilevel galleries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListKind {
    /// • ○ ▪ repeating.
    Bullet,
    /// A specific bullet character.
    BulletChar(char),
    /// 1. a. i.
    Numbered,
    /// 1) a) i)
    NumberedParen,
    /// I. A. 1.
    Outline,
    /// A. B. C.
    UpperLetter,
    /// a) b) c)
    LowerLetter,
    /// a. b. c.
    LowerLetterDot,
    /// i. ii. iii.
    LowerRoman,
    /// 1. 1.1. 1.1.1.
    Legal,
}

impl ListKind {
    pub fn parse(s: &str) -> Option<ListKind> {
        Some(match s {
            "bullet" => ListKind::Bullet,
            "numbered" | "number" => ListKind::Numbered,
            "numberedParen" => ListKind::NumberedParen,
            "outline" => ListKind::Outline,
            "upperLetter" => ListKind::UpperLetter,
            "lowerLetter" => ListKind::LowerLetter,
            "lowerLetterDot" => ListKind::LowerLetterDot,
            "lowerRoman" => ListKind::LowerRoman,
            "legal" | "multilevel" => ListKind::Legal,
            s if s.chars().count() == 1 => ListKind::BulletChar(s.chars().next()?),
            _ => return None,
        })
    }
    pub fn is_bullet(self) -> bool {
        matches!(self, ListKind::Bullet | ListKind::BulletChar(_))
    }
}

/// The bullet a level of a plain bulleted list shows: • ○ ▪ repeating (`level` is zero-based).
pub fn default_bullet(level: usize) -> char {
    ['•', '○', '▪'][level % 3]
}

fn bullet_level(i: usize, c: char) -> Level {
    Level {
        format: NumFormat::Bullet,
        text: c.to_string(),
        indent: 36.0 * (i as f32 + 1.0),
        hanging: 18.0,
        chr: CharProps {
            font: match c {
                '•' | '▪' | '○' => Some("Arial".into()),
                // Plain text characters (a dash bullet) use the paragraph's font.
                c if c.is_ascii() || c == '–' => None,
                _ => Some("Segoe UI Symbol".into()),
            },
            ..CharProps::default()
        },
        ..Level::default()
    }
}

/// Nine levels for a list kind.
pub fn levels_for(kind: ListKind) -> Vec<Level> {
    (0..9)
        .map(|i| {
            let ind = 36.0 * (i as f32 + 1.0);
            let cyc = |a: NumFormat, b: NumFormat, c: NumFormat| match i % 3 {
                0 => a,
                1 => b,
                _ => c,
            };
            let lvl = |format: NumFormat, text: String| Level { format, text, indent: ind, hanging: 18.0, ..Level::default() };
            let n = i + 1;
            match kind {
                ListKind::Bullet => bullet_level(i, default_bullet(i)),
                ListKind::BulletChar(c) => bullet_level(i, if i == 0 { c } else { default_bullet(i) }),
                ListKind::Numbered => lvl(cyc(NumFormat::Decimal, NumFormat::LowerLetter, NumFormat::LowerRoman), format!("%{n}.")),
                ListKind::NumberedParen => lvl(cyc(NumFormat::Decimal, NumFormat::LowerLetter, NumFormat::LowerRoman), format!("%{n})")),
                ListKind::UpperLetter => lvl(cyc(NumFormat::UpperLetter, NumFormat::LowerLetter, NumFormat::LowerRoman), format!("%{n}.")),
                ListKind::LowerLetter => lvl(cyc(NumFormat::LowerLetter, NumFormat::LowerRoman, NumFormat::Decimal), format!("%{n})")),
                ListKind::LowerLetterDot => lvl(cyc(NumFormat::LowerLetter, NumFormat::LowerRoman, NumFormat::Decimal), format!("%{n}.")),
                ListKind::LowerRoman => lvl(cyc(NumFormat::LowerRoman, NumFormat::LowerLetter, NumFormat::Decimal), format!("%{n}.")),
                ListKind::Outline => {
                    let f = [NumFormat::UpperRoman, NumFormat::UpperLetter, NumFormat::Decimal, NumFormat::LowerLetter, NumFormat::LowerRoman]
                        .get(i)
                        .copied()
                        .unwrap_or(NumFormat::Decimal);
                    lvl(f, format!("%{n}."))
                }
                ListKind::Legal => {
                    let text: String = (1..=n).map(|k| format!("%{k}.")).collect();
                    Level { format: NumFormat::Decimal, text, indent: 18.0 + 25.0 * i as f32, hanging: 18.0 + 7.0 * i as f32, ..Level::default() }
                }
            }
        })
        .collect()
}

impl Numbering {
    pub fn num(&self, id: u32) -> Option<&Num> {
        self.nums.iter().find(|n| n.id == id)
    }
    pub fn abstract_of(&self, num: u32) -> Option<&AbstractNum> {
        let n = self.num(num)?;
        self.abstracts.iter().find(|a| a.id == n.abstract_id)
    }
    /// Whether a paragraph in list `num` at `level` gets a label ([`Counters::next_label`] gives
    /// one), without counting it.
    pub fn shows_label(&self, num: u32, level: u8) -> bool {
        self.abstract_of(num).is_some() && self.level(num, level.min(8)).is_some()
    }
    pub fn level(&self, num: u32, level: u8) -> Option<&Level> {
        if let Some((_, l)) = self.num(num)?.level_overrides.iter().find(|(i, _)| *i == level) {
            return Some(l);
        }
        self.abstract_of(num)?.levels.get(level as usize)
    }
    /// Create a new list of `kind`; returns the `num` id. List ids run out only past `u32::MAX`
    /// lists, more than memory holds; then nothing is added and the id is 0, "no list".
    pub fn add_list(&mut self, kind: ListKind) -> u32 {
        self.try_add_list(kind).unwrap_or(0)
    }
    /// [`Numbering::add_list`], or an error, adding nothing, when list ids are all taken.
    pub fn try_add_list(&mut self, kind: ListKind) -> crate::Result<u32> {
        self.try_add_levels(None, levels_for(kind))
    }
    /// A new list definition (`name`, `levels`) and a `num` for it; returns the `num` id. Both ids
    /// are [`free_id`]s, so the largest ids a file can hold don't make them collide with its lists;
    /// an error, adding nothing, when they are all taken.
    pub fn try_add_levels(&mut self, name: Option<String>, levels: Vec<Level>) -> crate::Result<u32> {
        let aid = free_id(self.abstracts.iter().map(|a| a.id).max(), self.abstracts.iter().map(|a| a.id), 0);
        let (Some(aid), Some(nid)) = (aid, self.free_num_id()) else { return Err(DocError::Invalid("no free list id".into())) };
        self.abstracts.push(AbstractNum { id: aid, name, levels, ..Default::default() });
        self.nums.push(Num { id: nid, abstract_id: aid, ..Default::default() });
        Ok(nid)
    }
    /// A `num` id no list has (`num` 0 means "no list"), `None` when they are all taken.
    fn free_num_id(&self) -> Option<u32> {
        free_id(self.nums.iter().map(|n| n.id).max(), self.nums.iter().map(|n| n.id), 1)
    }
    /// A new `num` that restarts numbering of an existing list (`None`: no such list, or list ids
    /// are all taken).
    pub fn restart(&mut self, num: u32) -> Option<u32> {
        let source = self.num(num)?;
        let a = source.abstract_id;
        let level_overrides = source.level_overrides.clone();
        let nid = self.free_num_id()?;
        // Every level starts again at its own effective start, so a level this list defines itself keeps its start.
        let levels = self.abstracts.iter().find(|x| x.id == a).map_or(0, |x| x.levels.len());
        let start_overrides = (0..levels).filter_map(|i| self.level(num, i as u8).map(|l| (i as u8, l.start))).collect();
        self.nums.push(Num { id: nid, abstract_id: a, start_overrides, level_overrides });
        Some(nid)
    }
    /// Find an existing list whose first level matches `kind`, so consecutive bullet clicks reuse it.
    pub fn find_kind(&self, kind: ListKind) -> Option<u32> {
        let want = levels_for(kind);
        let first = want.first()?;
        self.nums
            .iter()
            .find(|n| self.abstract_of(n.id).and_then(|a| a.levels.first()).is_some_and(|l| l.format == first.format && l.text == first.text))
            .map(|n| n.id)
    }
}

/// Running list counters while walking a document in order.
#[derive(Default, Debug, Clone)]
pub struct Counters {
    /// Per abstract list: current value per level.
    state: HashMap<u32, [u32; 9]>,
    /// Which levels of which nums have started: a num's start override for a level applies
    /// once, when that level of that num is first used (ECMA-376 Part 1, 17.9.27 startOverride).
    seen: HashMap<(u32, u8), bool>,
    /// Per abstract list: whether each level has counted yet (a value of 0 can be a real count when a
    /// level starts at 0).
    flags: HashMap<u32, [LevelFlags; 9]>,
}

/// Whether a level has counted yet, and the value its next item takes if a start override says so.
#[derive(Default, Debug, Clone, Copy)]
struct LevelFlags {
    started: bool,
    first: Option<u32>,
}

impl Counters {
    /// The current value of each level of `num`'s list (0 for a level not counted yet).
    pub fn values(&self, numbering: &Numbering, num: u32) -> Option<[u32; 9]> {
        let abs = numbering.abstract_of(num)?;
        Some(self.state.get(&abs.id).copied().unwrap_or([0; 9]))
    }

    /// Advance the counter for `num`/`level` and return the label (`"1."`, `"a)"`, `"•"`).
    pub fn next_label(&mut self, numbering: &Numbering, num: u32, level: u8) -> Option<(String, Level)> {
        let n = numbering.num(num)?;
        let abs = numbering.abstract_of(num)?;
        let lv = (level as usize).min(8);
        // This list's own definition of a level wins over its abstract list's.
        let level_def = |k: usize| numbering.level(num, k as u8);
        // A "none" level (Word's numFmt none) still counts and restarts the levels below it; only
        // its number reads as nothing, so its literal text ("RESPONSE:") shows (#4985).
        let def = level_def(lv)?.clone();
        let st = self.state.entry(abs.id).or_insert([0; 9]);
        let fl = self.flags.entry(abs.id).or_default();
        // Only this level's override, not the num's others: the counters are shared by every
        // num of the abstract list, so restarting its other levels here would renumber
        // paragraphs of the other nums. A heading style can use one num for levels 1-4 and
        // another, with start overrides on every level, for 5-9; its first level-5 heading
        // must not restart the chapter count.
        if let std::collections::hash_map::Entry::Vacant(e) = self.seen.entry((num, lv as u8)) {
            e.insert(true);
            if let Some((_, start)) = n.start_overrides.iter().find(|(l, _)| *l as usize == lv)
                && let (Some(s), Some(f)) = (st.get_mut(lv), fl.get_mut(lv))
            {
                *s = 0;
                *f = LevelFlags { started: false, first: Some(*start) };
            }
        }
        if let (Some(v), Some(f)) = (st.get_mut(lv), fl.get_mut(lv)) {
            *v = if f.started { v.saturating_add(1) } else { f.first.unwrap_or(def.start) };
            *f = LevelFlags { started: true, first: None };
        }
        // Deeper levels restart, unless they never restart or only restart after a level above
        // this one (Word's lvlRestart).
        for (d, (deeper, f)) in st.iter_mut().zip(fl.iter_mut()).enumerate().skip(lv + 1) {
            let restarts = level_def(d).is_none_or(|l| l.restart && l.restart_after.is_none_or(|k| lv < k as usize));
            if restarts {
                *deeper = 0;
                *f = LevelFlags::default();
            }
        }
        let vals = *st;
        let started = *fl;
        if def.format == NumFormat::Bullet {
            return Some((def.text.clone(), def));
        }
        let mut label = String::new();
        // Word's level text is short; .docx and the lists command already cap it at 64.
        let mut chars = def.text.chars().take(MAX_LEVEL_TEXT).peekable();
        while let Some(c) = chars.next() {
            if c == '%'
                && let Some(d) = chars.peek().and_then(|d| d.to_digit(10))
            {
                chars.next();
                let k = (d as usize).saturating_sub(1).min(8);
                let val = vals.get(k).copied().unwrap_or(0);
                let counted = started.get(k).is_some_and(|f| f.started);
                let val = if counted { val } else { level_def(k).map(|l| l.start).unwrap_or(1) };
                // Legal numbering (w:isLgl) shows every level in Arabic digits, whatever its own
                // format (二 of a Chinese outline becomes 2 in "2.1"); a "none" level stays empty.
                let fmt = match level_def(k).map(|l| l.format).unwrap_or(NumFormat::Decimal) {
                    f @ (NumFormat::DecimalZero | NumFormat::None) => f,
                    _ if def.legal => NumFormat::Decimal,
                    f => f,
                };
                label.push_str(&fmt.format(val));
                if label.len() > MAX_LABEL_BYTES {
                    break;
                }
            } else {
                label.push(c);
            }
        }
        Some((label, def))
    }
}

/// Longest level text (in characters) expanded into a list label.
const MAX_LEVEL_TEXT: usize = 64;
/// Labels stop growing once they pass this many bytes.
const MAX_LABEL_BYTES: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_huge_level_text_gives_a_bounded_label() {
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        let huge = Level { format: NumFormat::UpperRoman, text: "%1".repeat(30_000), start: u32::MAX, ..Level::default() };
        if let Some(num) = n.nums.iter_mut().find(|x| x.id == id) {
            num.level_overrides = vec![(0, huge)];
        }
        let mut c = Counters::default();
        let label = c.next_label(&n, id, 0).unwrap().0;
        assert!(label.len() <= 512, "label is {} bytes", label.len());
        assert!(label.starts_with("MMMM"));
    }

    #[test]
    fn a_list_s_own_level_definitions_win() {
        // A specification's article numbers: level 1 counts parts with no label of its own, level 2
        // shows "part.article" with a zero-padded article (Word's lvlOverride/lvl).
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        let lvl = |format, text: &str| Level { format, text: text.into(), ..Level::default() };
        if let Some(num) = n.nums.iter_mut().find(|x| x.id == id) {
            num.level_overrides = vec![(1, lvl(NumFormat::Decimal, "")), (2, lvl(NumFormat::DecimalZero, "%2.%3"))];
        }
        let mut c = Counters::default();
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "");
        assert_eq!(c.next_label(&n, id, 2).unwrap().0, "1.01");
        assert_eq!(c.next_label(&n, id, 2).unwrap().0, "1.02");
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "");
        assert_eq!(c.next_label(&n, id, 2).unwrap().0, "2.01");
        // Levels it doesn't define still come from the abstract list.
        assert_eq!(c.next_label(&n, id, 0).unwrap().0, "1.");
        assert_eq!(n.level(id, 2).map(|l| l.format), Some(NumFormat::DecimalZero));
    }

    #[test]
    fn a_none_level_shows_its_text_and_still_counts() {
        // As Word shows them (#4985): a "none" level's literal text is its label, its own number
        // reads as nothing (also in legal numbering), and it restarts the levels below it.
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        let lvl = |format, text: &str| Level { format, text: text.into(), ..Level::default() };
        let legal = Level { legal: true, ..lvl(NumFormat::Decimal, "%2.%3") };
        if let Some(num) = n.nums.iter_mut().find(|x| x.id == id) {
            num.level_overrides = vec![(0, lvl(NumFormat::None, "RESPONSE:")), (1, lvl(NumFormat::None, "[%2]")), (2, legal)];
        }
        assert!(n.shows_label(id, 0));
        let mut c = Counters::default();
        assert_eq!(c.next_label(&n, id, 0).unwrap().0, "RESPONSE:");
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "[]");
        assert_eq!(c.next_label(&n, id, 2).unwrap().0, ".1");
        assert_eq!(c.next_label(&n, id, 2).unwrap().0, ".2");
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "[]");
        assert_eq!(c.next_label(&n, id, 2).unwrap().0, ".1");
    }

    #[test]
    fn a_start_override_restarts_only_its_own_level_of_a_shared_list() {
        // One abstract list numbers the headings through two nums: one for levels 1-4, and one
        // restarting every level (startOverride 1 on 0..8) for levels 5-9, first used deep in
        // chapter 2. Chapter 3 is still chapter 3, and the deep heading counts from 1.
        let mut n = Numbering::default();
        let chapters = n.add_list(ListKind::Legal);
        let abs = n.abstract_of(chapters).map(|a| a.id).unwrap();
        let deep = chapters + 1;
        n.nums.push(Num { id: deep, abstract_id: abs, start_overrides: (0..9).map(|l| (l, 1)).collect(), ..Num::default() });
        let mut c = Counters::default();
        let mut l = |num, lv| c.next_label(&n, num, lv).unwrap().0;
        assert_eq!(l(chapters, 0), "1.");
        assert_eq!(l(chapters, 0), "2.");
        assert_eq!(l(chapters, 1), "2.1.");
        assert_eq!(l(chapters, 2), "2.1.1.");
        assert_eq!(l(chapters, 3), "2.1.1.1.");
        assert_eq!(l(deep, 4), "2.1.1.1.1.", "the deep num restarts only level 5");
        assert_eq!(l(deep, 4), "2.1.1.1.2.");
        assert_eq!(l(chapters, 0), "3.", "and not the chapter count");
        assert_eq!(l(chapters, 1), "3.1.");
    }

    #[test]
    fn levels_restart_only_after_their_restart_level() {
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Legal);
        if let Some(a) = n.abstracts.first_mut() {
            // Level 3 restarts only after level 1; level 2 never restarts.
            a.levels[2].restart_after = Some(1);
            a.levels[2].text = "(%3)".into();
            a.levels[1].restart = false;
        }
        let mut c = Counters::default();
        let mut l = |lv| c.next_label(&n, id, lv).unwrap().0;
        assert_eq!(l(0), "1.");
        assert_eq!(l(1), "1.1.");
        assert_eq!(l(2), "(1)");
        assert_eq!(l(1), "1.2.");
        assert_eq!(l(2), "(2)", "level 2 doesn't restart level 3");
        assert_eq!(l(0), "2.");
        assert_eq!(l(1), "2.3.", "level 2 never restarts");
        assert_eq!(l(2), "(1)", "level 1 restarts level 3");
    }

    #[test]
    fn a_level_starting_at_zero_counts_up() {
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        if let Some(a) = n.abstracts.first_mut() {
            a.levels[0].start = 0;
        }
        let mut c = Counters::default();
        let labels: Vec<String> = (0..4).map(|_| c.next_label(&n, id, 0).unwrap().0).collect();
        assert_eq!(labels, ["0.", "1.", "2.", "3."]);
    }

    #[test]
    fn a_start_override_of_one_or_zero_is_honoured() {
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        if let Some(a) = n.abstracts.first_mut() {
            a.levels[0].start = 5;
        }
        if let Some(num) = n.nums.iter_mut().find(|x| x.id == id) {
            num.start_overrides = vec![(0, 1)];
        }
        let mut c = Counters::default();
        let labels: Vec<String> = (0..2).map(|_| c.next_label(&n, id, 0).unwrap().0).collect();
        assert_eq!(labels, ["1.", "2."]);

        if let Some(num) = n.nums.iter_mut().find(|x| x.id == id) {
            num.start_overrides = vec![(0, 0)];
        }
        let mut c = Counters::default();
        let labels: Vec<String> = (0..3).map(|_| c.next_label(&n, id, 0).unwrap().0).collect();
        assert_eq!(labels, ["0.", "1.", "2."]);
    }

    #[test]
    fn numbered_labels() {
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        let mut c = Counters::default();
        assert_eq!(c.next_label(&n, id, 0).unwrap().0, "1.");
        assert_eq!(c.next_label(&n, id, 0).unwrap().0, "2.");
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "a.");
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "b.");
        assert_eq!(c.next_label(&n, id, 0).unwrap().0, "3.");
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "a.");
        assert!(c.next_label(&n, 999, 0).is_none());
    }

    #[test]
    fn east_asian_labels_and_legal_numbering() {
        // A Chinese outline: 第一章, then "一、" items, and a legal level showing 1.1.
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        if let Some(a) = n.abstracts.first_mut() {
            a.levels[0] = Level { format: NumFormat::ChineseCounting, text: "第%1章".into(), ..Level::default() };
            a.levels[1] = Level { format: NumFormat::ChineseCounting, text: "%2、".into(), ..Level::default() };
            a.levels[2] = Level { format: NumFormat::ChineseCounting, text: "%1.%2.%3".into(), legal: true, ..Level::default() };
            a.levels[3] = Level { format: NumFormat::ChineseCounting, text: "（%4）".into(), ..Level::default() };
        }
        let mut c = Counters::default();
        let mut l = |lv| c.next_label(&n, id, lv).unwrap().0;
        assert_eq!(l(0), "第一章");
        assert_eq!(l(1), "一、");
        assert_eq!(l(1), "二、");
        assert_eq!(l(1), "三、");
        assert_eq!(l(2), "1.3.1", "legal numbering is decimal on every level");
        assert_eq!(l(3), "（一）");
        for _ in 0..9 {
            l(3);
        }
        assert_eq!(l(3), "（十一）");
        assert_eq!(l(0), "第二章");
    }

    #[test]
    fn legal_labels() {
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Legal);
        let mut c = Counters::default();
        assert_eq!(c.next_label(&n, id, 0).unwrap().0, "1.");
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "1.1.");
        assert_eq!(c.next_label(&n, id, 2).unwrap().0, "1.1.1.");
        assert_eq!(c.next_label(&n, id, 1).unwrap().0, "1.2.");
    }

    #[test]
    fn restart_and_bullets() {
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        let mut c = Counters::default();
        c.next_label(&n, id, 0);
        c.next_label(&n, id, 0);
        let r = n.restart(id).unwrap();
        assert_eq!(c.next_label(&n, r, 0).unwrap().0, "1.");
        let b = n.add_list(ListKind::Bullet);
        assert_eq!(c.next_label(&n, b, 0).unwrap().0, "•");
        assert_eq!(n.find_kind(ListKind::Bullet), Some(b));
    }

    #[test]
    fn restart_keeps_the_list_s_own_level_definitions() {
        let mut n = Numbering::default();
        let id = n.add_list(ListKind::Numbered);
        let own = Level { format: NumFormat::UpperRoman, text: "Part %1".into(), indent: 90.0, hanging: 40.0, start: 5, ..Level::default() };
        if let Some(num) = n.nums.iter_mut().find(|x| x.id == id) {
            num.level_overrides.push((0, own.clone()));
        }
        let r = n.restart(id).unwrap();
        assert_eq!(n.num(r).unwrap().level_overrides, vec![(0, own.clone())]);
        assert_eq!(n.level(r, 0), Some(&own));
        let mut c = Counters::default();
        assert_eq!(c.next_label(&n, r, 0).unwrap().0, "Part V");
    }
}
