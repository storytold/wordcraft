//! Lists: abstract numbering definitions (9 levels each), numbering instances, and the counter
//! state used to compute list labels.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::props::{Align, CharProps};
use crate::section::NumFormat;

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Num {
    pub id: u32,
    pub abstract_id: u32,
    /// Per-level start overrides (level, start).
    pub start_overrides: Vec<(u8, u32)>,
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

fn bullet_level(i: usize, c: char) -> Level {
    Level {
        format: NumFormat::Bullet,
        text: c.to_string(),
        indent: 36.0 * (i as f32 + 1.0),
        hanging: 18.0,
        chr: CharProps {
            font: Some(if c == '•' || c == '▪' || c == '○' { "Arial".into() } else { "Segoe UI Symbol".into() }),
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
                ListKind::Bullet => bullet_level(i, ['•', '○', '▪'][i % 3]),
                ListKind::BulletChar(c) => bullet_level(i, if i == 0 { c } else { ['•', '○', '▪'][i % 3] }),
                ListKind::Numbered => lvl(cyc(NumFormat::Decimal, NumFormat::LowerLetter, NumFormat::LowerRoman), format!("%{n}.")),
                ListKind::NumberedParen => lvl(cyc(NumFormat::Decimal, NumFormat::LowerLetter, NumFormat::LowerRoman), format!("%{n})")),
                ListKind::UpperLetter => lvl(cyc(NumFormat::UpperLetter, NumFormat::LowerLetter, NumFormat::LowerRoman), format!("%{n}.")),
                ListKind::LowerLetter => lvl(cyc(NumFormat::LowerLetter, NumFormat::LowerRoman, NumFormat::Decimal), format!("%{n})")),
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
    pub fn level(&self, num: u32, level: u8) -> Option<&Level> {
        self.abstract_of(num)?.levels.get(level as usize)
    }
    /// Create a new list of `kind`; returns the `num` id.
    pub fn add_list(&mut self, kind: ListKind) -> u32 {
        let aid = self.abstracts.iter().map(|a| a.id + 1).max().unwrap_or(0);
        self.abstracts.push(AbstractNum { id: aid, name: None, levels: levels_for(kind) });
        let nid = self.nums.iter().map(|n| n.id + 1).max().unwrap_or(1).max(1);
        self.nums.push(Num { id: nid, abstract_id: aid, start_overrides: Vec::new() });
        nid
    }
    /// A new `num` that restarts numbering of an existing list.
    pub fn restart(&mut self, num: u32) -> Option<u32> {
        let a = self.num(num)?.abstract_id;
        let nid = self.nums.iter().map(|n| n.id + 1).max().unwrap_or(1).max(1);
        let starts = self.abstracts.iter().find(|x| x.id == a).map(|x| x.levels.iter().enumerate().map(|(i, l)| (i as u8, l.start)).collect());
        self.nums.push(Num { id: nid, abstract_id: a, start_overrides: starts.unwrap_or_default() });
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
    /// Which nums have started (to apply start overrides once).
    seen: HashMap<u32, bool>,
}

impl Counters {
    /// Advance the counter for `num`/`level` and return the label (`"1."`, `"a)"`, `"•"`).
    pub fn next_label(&mut self, numbering: &Numbering, num: u32, level: u8) -> Option<(String, Level)> {
        let n = numbering.num(num)?;
        let abs = numbering.abstract_of(num)?;
        let lv = (level as usize).min(8);
        let def = abs.levels.get(lv)?.clone();
        // A "none" level shows no label at all (Word's numFmt none) — not even its text.
        if def.format == NumFormat::None {
            return None;
        }
        let st = self.state.entry(abs.id).or_insert([0; 9]);
        if let std::collections::hash_map::Entry::Vacant(e) = self.seen.entry(num) {
            e.insert(true);
            for (l, start) in &n.start_overrides {
                if let Some(s) = st.get_mut(*l as usize) {
                    *s = start.saturating_sub(1);
                }
            }
        }
        if let Some(v) = st.get_mut(lv) {
            *v = if *v == 0 { def.start.max(if def.start == 0 { 0 } else { 1 }) } else { v.saturating_add(1) };
            if *v == 0 {
                *v = def.start;
            }
        }
        for deeper in st.iter_mut().skip(lv + 1) {
            *deeper = 0;
        }
        let vals = *st;
        if def.format == NumFormat::Bullet {
            return Some((def.text.clone(), def));
        }
        let mut label = String::new();
        let mut chars = def.text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%'
                && let Some(d) = chars.peek().and_then(|d| d.to_digit(10))
            {
                chars.next();
                let k = (d as usize).saturating_sub(1).min(8);
                let val = vals.get(k).copied().unwrap_or(0);
                let val = if val == 0 { abs.levels.get(k).map(|l| l.start).unwrap_or(1) } else { val };
                let fmt = if def.legal && k != lv { NumFormat::Decimal } else { abs.levels.get(k).map(|l| l.format).unwrap_or(NumFormat::Decimal) };
                label.push_str(&fmt.format(val));
            } else {
                label.push(c);
            }
        }
        Some((label, def))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
