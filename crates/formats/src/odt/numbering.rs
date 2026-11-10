use super::model;
use super::xml::{Attrs, get, length};
use std::collections::HashMap;
use wordcraft_doc::numbering::{Level, LevelSuffix};
use wordcraft_doc::section::NumFormat;

pub(super) fn format(value: &str) -> Option<NumFormat> {
    matches!(value.trim(), "" | "1" | "a" | "A" | "i" | "I").then(|| super::styles::num_format(value))
}

pub(super) fn definition(kind: &str, a: &Attrs, level: usize) -> Level {
    let bullet = kind != "list-level-style-number";
    let character = get(a, "text:bullet-char").and_then(|s| s.chars().find(|c| !c.is_control() && !c.is_whitespace())).unwrap_or('•');
    let bullet_level =
        wordcraft_doc::numbering::levels_for(wordcraft_doc::numbering::ListKind::BulletChar(character)).into_iter().next().unwrap_or_default();
    let format = if bullet {
        NumFormat::Bullet
    } else {
        format(get(a, "style:num-format").unwrap_or("1")).unwrap_or_else(|| {
            model::dropped(model::Dropped::OdtNumbering);
            NumFormat::Decimal
        })
    };
    let display = get(a, "text:display-levels").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).clamp(1, level + 1);
    let number = (level + 1 - display..=level).map(|i| format!("%{}", i + 1)).collect::<Vec<_>>().join(".");
    Level {
        start: get(a, "text:start-value").and_then(|v| v.parse().ok()).unwrap_or(1).min(32_767),
        format,
        sequential_letters: get(a, "style:num-letter-sync") != Some("true"),
        text: if bullet {
            character.to_string()
        } else {
            format!(
                "{}{}{}",
                get(a, "style:num-prefix").unwrap_or("").chars().take(16).collect::<String>(),
                number,
                get(a, "style:num-suffix").unwrap_or("").chars().take(16).collect::<String>()
            )
        },
        chr: if bullet { bullet_level.chr } else { Default::default() },
        indent: 36.0 * (level.min(8) as f32 + 1.0),
        ..Default::default()
    }
}

pub(super) fn spacing(level: &mut Level, local: &str, a: &Attrs) {
    // In label-alignment mode, an omitted separator is a list tab, not nothing.
    if local == "list-level-label-alignment" || get(a, "text:list-level-position-and-space-mode") == Some("label-alignment") {
        level.suffix = LevelSuffix::Tab;
    }
    // Match the paragraph geometry limit; overflowing lengths must not reach layout.
    let len = |key| get(a, key).and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(-1584.0, 1584.0));
    if let Some(align) = get(a, "fo:text-align") {
        level.align = match align {
            "end" | "right" => wordcraft_doc::Align::Right,
            "center" => wordcraft_doc::Align::Center,
            _ => wordcraft_doc::Align::Left,
        };
    }
    let before = len("text:space-before");
    let width = len("text:min-label-width");
    if before.is_some() || width.is_some() {
        // Stated legacy geometry uses zero for an omitted component, not the fallback indent.
        let width = width.unwrap_or(0.0).max(0.0);
        level.indent = before.unwrap_or(0.0) + width;
        level.hanging = width;
        level.suffix = if width > 0.0 { LevelSuffix::Tab } else { LevelSuffix::Nothing };
    }
    if let Some(left) = len("fo:margin-left") {
        level.indent = left;
    }
    if let Some(first) = len("fo:text-indent") {
        level.hanging = -first;
    }
    if let Some(tab) = len("text:list-tab-stop-position") {
        level.tab = Some(tab);
    }
    // Match the document model's twip precision for list positions.
    level.indent = (level.indent.clamp(-1584.0, 1584.0) * 20.0).round() / 20.0;
    level.hanging = (level.hanging.clamp(-1584.0, 1584.0) * 20.0).round() / 20.0;
    if let Some(follow) = get(a, "text:label-followed-by") {
        level.suffix = match follow {
            "listtab" => LevelSuffix::Tab,
            "space" => LevelSuffix::Space,
            _ => LevelSuffix::Nothing,
        };
    }
}

/// State of one open ODT list and its continuation chain.
#[derive(Clone, Default)]
pub(super) struct OpenList {
    /// Named list style, inherited by nested lists when omitted.
    pub style: Option<String>,
    /// Shared definitions for the open nesting levels.
    pub definitions: std::sync::Arc<Vec<(u8, Level)>>,
    /// Whether the current item is numbered rather than a list header.
    pub item_open: bool,
    /// Document numbering instance shared by nested and continued lists.
    pub group: u32,
    /// Continuation chain whose next counter is tracked independently.
    chain: u32,
    /// Whether the current item has emitted a paragraph.
    pub emitted: bool,
    /// Source XML ID used by explicit continuation references.
    id: Option<String>,
    /// Next item value, bounded when applied to a document level.
    next: u32,
    /// Pending explicit or initial restart for the next paragraph.
    pub start: Option<u32>,
    /// Whether no numbered item has opened yet.
    first: bool,
    /// Whether this list resumes an earlier chain.
    continued: bool,
}

impl OpenList {
    pub fn item(&mut self, a: &Attrs, header: bool) {
        self.item_open = !header;
        if header {
            return;
        }
        let explicit = get(a, "text:start-value").and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.min(32_767));
        self.start = explicit.or_else(|| ((self.first && !self.continued) || (!self.first && !self.emitted)).then_some(self.next));
        self.next = explicit.unwrap_or(self.next).saturating_add(1);
        self.first = false;
        self.emitted = false;
    }
}

/// Bounded continuation lookup shared by the lists in one document.
#[derive(Default)]
pub(super) struct Lists {
    /// Monotonic identity allocator for independent lists.
    next_group: u32,
    /// Closed lists indexed by source XML ID.
    named: HashMap<String, OpenList>,
    /// Most recent closed list at each nesting depth.
    previous: HashMap<usize, OpenList>,
    /// Next counter for each continuation chain.
    next: HashMap<u32, u32>,
}

impl Lists {
    pub fn open(&mut self, a: &Attrs, parents: &[OpenList], start: u32) -> OpenList {
        let style = get(a, "text:style-name").map(String::from).or_else(|| parents.last().and_then(|l| l.style.clone()));
        let previous = if let Some(id) = get(a, "text:continue-list") {
            let found = self.named.get(id);
            if found.is_none() {
                model::dropped(model::Dropped::OdtNumbering);
            }
            found
        } else if get(a, "text:continue-numbering") == Some("true") {
            self.previous.get(&parents.len()).filter(|l| l.style == style)
        } else {
            None
        };
        let continued = previous.map(|l| (l.group, self.next.get(&l.chain).copied().unwrap_or(l.next), l.chain));
        self.next_group = self.next_group.saturating_add(1);
        OpenList {
            style,
            definitions: Default::default(),
            item_open: false,
            group: parents.last().map(|l| l.group).or_else(|| continued.map(|v| v.0)).unwrap_or(self.next_group),
            chain: continued.map(|v| v.2).unwrap_or(self.next_group),
            emitted: false,
            id: get(a, "xml:id").filter(|s| s.len() <= 256).map(String::from),
            next: continued.map(|v| v.1).unwrap_or(start),
            start: None,
            first: true,
            continued: continued.is_some(),
        }
    }
    pub fn close(&mut self, list: OpenList, level: usize) {
        if self.next.len() < 16384 || self.next.contains_key(&list.chain) {
            self.next.insert(list.chain, list.next);
        } else {
            model::dropped(model::Dropped::OdtNumbering);
        }
        if self.named.len() < 16384 {
            if let Some(id) = &list.id {
                self.named.insert(id.clone(), list.clone());
            }
        } else {
            model::dropped(model::Dropped::OdtNumbering);
        }
        self.previous.insert(level, list);
    }
}
