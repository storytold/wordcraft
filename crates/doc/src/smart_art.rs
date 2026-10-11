//! SmartArt graphics WordCraft makes and edits (Insert › SmartArt): a layout, an outline of items
//! (text and level) and a colour variant.
//!
//! A SmartArt object ([`crate::graphic::Graphic`] with `smart_art` set) keeps this model next to
//! the items it is drawn with. WordCraft lays it out with its own algorithms; the DOCX writer
//! saves it as diagram parts (data, layout, style, colours and the drawing of the shapes) and the
//! reader turns a diagram back into it only when its data part is exactly what WordCraft writes.
//! SmartArt from other programs stays as it was read.
//!
//! Every value from outside (commands, files) goes through [`SmartArtSpec::sanitize`]: item
//! count, text length and levels are capped.

use serde::{Deserialize, Serialize};

/// Most items in a SmartArt graphic.
pub const MAX_ITEMS: usize = 100;
/// Longest item text, characters.
pub const MAX_TEXT: usize = 255;
/// Deepest item level (0 is the top).
pub const MAX_LEVEL: u8 = 7;

/// How a SmartArt graphic arranges its items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SmartArtLayout {
    /// Blocks in rows.
    #[default]
    BasicList,
    /// Steps left to right with arrows between.
    Process,
    /// Steps around a circle with arrows between.
    Cycle,
    /// A tree: each item under the item one level up before it.
    Hierarchy,
    /// Stacked bands narrowing to the top.
    Pyramid,
    /// The first item in the middle, the items under it around it.
    Radial,
    /// Quadrants of a square.
    Matrix,
    /// Overlapping circles.
    Venn,
}

impl SmartArtLayout {
    pub const ALL: [SmartArtLayout; 8] = [
        SmartArtLayout::BasicList,
        SmartArtLayout::Process,
        SmartArtLayout::Cycle,
        SmartArtLayout::Hierarchy,
        SmartArtLayout::Pyramid,
        SmartArtLayout::Radial,
        SmartArtLayout::Matrix,
        SmartArtLayout::Venn,
    ];

    /// The id commands use (`basicList`, `process`…).
    pub fn id(self) -> &'static str {
        match self {
            SmartArtLayout::BasicList => "basicList",
            SmartArtLayout::Process => "process",
            SmartArtLayout::Cycle => "cycle",
            SmartArtLayout::Hierarchy => "hierarchy",
            SmartArtLayout::Pyramid => "pyramid",
            SmartArtLayout::Radial => "radial",
            SmartArtLayout::Matrix => "matrix",
            SmartArtLayout::Venn => "venn",
        }
    }

    /// The layout an id names (any ASCII case; `list` is the basic list).
    pub fn from_id(id: &str) -> Option<SmartArtLayout> {
        let id = id.trim();
        if id.eq_ignore_ascii_case("list") {
            return Some(SmartArtLayout::BasicList);
        }
        SmartArtLayout::ALL.into_iter().find(|l| l.id().eq_ignore_ascii_case(id))
    }

    /// The name shown for it.
    pub fn label(self) -> &'static str {
        match self {
            SmartArtLayout::BasicList => "Basic List",
            SmartArtLayout::Process => "Basic Process",
            SmartArtLayout::Cycle => "Basic Cycle",
            SmartArtLayout::Hierarchy => "Hierarchy",
            SmartArtLayout::Pyramid => "Basic Pyramid",
            SmartArtLayout::Radial => "Basic Radial",
            SmartArtLayout::Matrix => "Basic Matrix",
            SmartArtLayout::Venn => "Basic Venn",
        }
    }

    /// The gallery category it's shown under.
    pub fn category(self) -> &'static str {
        match self {
            SmartArtLayout::BasicList => "List",
            SmartArtLayout::Process => "Process",
            SmartArtLayout::Cycle => "Cycle",
            SmartArtLayout::Hierarchy => "Hierarchy",
            SmartArtLayout::Pyramid => "Pyramid",
            SmartArtLayout::Radial | SmartArtLayout::Venn => "Relationship",
            SmartArtLayout::Matrix => "Matrix",
        }
    }

    /// What it's for, in a sentence (the picker shows it).
    pub fn description(self) -> &'static str {
        match self {
            SmartArtLayout::BasicList => "Shows items of equal weight as blocks in rows.",
            SmartArtLayout::Process => "Shows steps in order, left to right, with arrows between them.",
            SmartArtLayout::Cycle => "Shows steps that repeat, around a circle.",
            SmartArtLayout::Hierarchy => "Shows a tree: each item below the item it belongs to.",
            SmartArtLayout::Pyramid => "Shows parts that build on each other, largest at the bottom.",
            SmartArtLayout::Radial => "Shows items around a central idea: the first item is the centre.",
            SmartArtLayout::Matrix => "Shows parts of a whole as quadrants.",
            SmartArtLayout::Venn => "Shows overlapping ideas as overlapping circles.",
        }
    }
}

/// The colours a SmartArt graphic takes from the document's theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SmartArtColors {
    /// Every shape in accent `n` (1–6).
    #[default]
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    /// Shapes take the accents in turn.
    Colorful,
}

impl SmartArtColors {
    pub const ALL: [SmartArtColors; 7] = [
        SmartArtColors::Accent1,
        SmartArtColors::Accent2,
        SmartArtColors::Accent3,
        SmartArtColors::Accent4,
        SmartArtColors::Accent5,
        SmartArtColors::Accent6,
        SmartArtColors::Colorful,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SmartArtColors::Accent1 => "accent1",
            SmartArtColors::Accent2 => "accent2",
            SmartArtColors::Accent3 => "accent3",
            SmartArtColors::Accent4 => "accent4",
            SmartArtColors::Accent5 => "accent5",
            SmartArtColors::Accent6 => "accent6",
            SmartArtColors::Colorful => "colorful",
        }
    }

    pub fn from_id(id: &str) -> Option<SmartArtColors> {
        SmartArtColors::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(id.trim()))
    }

    /// The name shown for it.
    pub fn label(self) -> &'static str {
        match self {
            SmartArtColors::Accent1 => "Accent 1",
            SmartArtColors::Accent2 => "Accent 2",
            SmartArtColors::Accent3 => "Accent 3",
            SmartArtColors::Accent4 => "Accent 4",
            SmartArtColors::Accent5 => "Accent 5",
            SmartArtColors::Accent6 => "Accent 6",
            SmartArtColors::Colorful => "Colorful",
        }
    }

    /// The theme accent (1–6) of shape `i`.
    pub fn accent(self, i: usize) -> usize {
        match self {
            SmartArtColors::Accent1 => 1,
            SmartArtColors::Accent2 => 2,
            SmartArtColors::Accent3 => 3,
            SmartArtColors::Accent4 => 4,
            SmartArtColors::Accent5 => 5,
            SmartArtColors::Accent6 => 6,
            SmartArtColors::Colorful => i % 6 + 1,
        }
    }
}

/// One item of the outline: its text and level (0 is the top).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartArtItem {
    pub text: String,
    #[serde(default)]
    pub level: u8,
}

impl SmartArtItem {
    pub fn new(text: impl Into<String>, level: u8) -> SmartArtItem {
        SmartArtItem { text: text.into(), level }
    }
}

/// A SmartArt graphic WordCraft can edit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartArtSpec {
    pub layout: SmartArtLayout,
    #[serde(default)]
    pub items: Vec<SmartArtItem>,
    #[serde(default)]
    pub colors: SmartArtColors,
}

impl SmartArtSpec {
    /// A new graphic of `layout` with WordCraft's sample items.
    pub fn sample(layout: SmartArtLayout) -> SmartArtSpec {
        let items: Vec<SmartArtItem> = match layout {
            SmartArtLayout::Hierarchy => vec![
                SmartArtItem::new("Lead", 0),
                SmartArtItem::new("Design", 1),
                SmartArtItem::new("Sketches", 2),
                SmartArtItem::new("Build", 1),
                SmartArtItem::new("Code", 2),
                SmartArtItem::new("Tests", 2),
            ],
            SmartArtLayout::Radial => {
                let mut v = vec![SmartArtItem::new("Goal", 0)];
                v.extend(["Plan", "Build", "Test", "Ship"].map(|t| SmartArtItem::new(t, 1)));
                v
            }
            SmartArtLayout::Process | SmartArtLayout::Cycle => ["Plan", "Build", "Test", "Ship"].map(|t| SmartArtItem::new(t, 0)).to_vec(),
            SmartArtLayout::Pyramid | SmartArtLayout::Venn => ["People", "Process", "Tools"].map(|t| SmartArtItem::new(t, 0)).to_vec(),
            SmartArtLayout::Matrix => ["Strengths", "Weaknesses", "Chances", "Risks"].map(|t| SmartArtItem::new(t, 0)).to_vec(),
            SmartArtLayout::BasicList => ["Ideas", "Notes", "Drafts", "Edits", "Final"].map(|t| SmartArtItem::new(t, 0)).to_vec(),
        };
        SmartArtSpec { layout, items, colors: SmartArtColors::default() }
    }

    /// Make the graphic safe to store and draw: at most [`MAX_ITEMS`] items, text cut to
    /// [`MAX_TEXT`] characters with control characters as spaces and the ends trimmed, and levels
    /// that follow on (the first item at level 0, each item at most one level below the one
    /// before it, never below [`MAX_LEVEL`]).
    pub fn sanitize(&mut self) {
        self.items.truncate(MAX_ITEMS);
        let mut prev: Option<u8> = None;
        for it in &mut self.items {
            it.text = clean_text(&it.text);
            let most = prev.map_or(0, |p| p.saturating_add(1)).min(MAX_LEVEL);
            it.level = it.level.min(most);
            prev = Some(it.level);
        }
    }

    /// The graphic after `sanitize`.
    pub fn sanitized(mut self) -> SmartArtSpec {
        self.sanitize();
        self
    }

    /// The index of the item each item belongs to (the nearest item before it one level up);
    /// `None` for the top-level items. Expects a sanitized graphic.
    pub fn parents(&self) -> Vec<Option<usize>> {
        let mut out = Vec::with_capacity(self.items.len());
        // The last item seen at each level.
        let mut last: Vec<usize> = Vec::new();
        for (i, it) in self.items.iter().enumerate() {
            let l = usize::from(it.level);
            last.truncate(l);
            out.push(if l == 0 { None } else { last.last().copied() });
            last.push(i);
        }
        out
    }

    /// The items that belong to item `i` (directly or deeper): the run after it at deeper levels.
    pub fn descendants(&self, i: usize) -> std::ops::Range<usize> {
        let Some(level) = self.items.get(i).map(|it| it.level) else { return 0..0 };
        let start = i + 1;
        let end = self.items.iter().skip(start).position(|it| it.level <= level).map_or(self.items.len(), |p| start + p);
        start..end
    }
}

/// `s` as item text: control characters as spaces, ends trimmed, at most [`MAX_TEXT`] characters.
pub fn clean_text(s: &str) -> String {
    let s: String = s.chars().take(MAX_TEXT * 4).map(|c| if c.is_control() { ' ' } else { c }).collect();
    s.trim().chars().take(MAX_TEXT).collect::<String>().trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_caps_items_text_and_levels() {
        let mut s = SmartArtSpec {
            items: (0..MAX_ITEMS + 20).map(|i| SmartArtItem::new(format!("{}\n{i}", "x".repeat(400)), if i == 0 { 5 } else { 200 })).collect(),
            ..Default::default()
        };
        s.sanitize();
        assert_eq!(s.items.len(), MAX_ITEMS);
        assert_eq!(s.items[0].level, 0, "the first item is at the top");
        assert_eq!(s.items[1].level, 1, "one level below the item before");
        assert!(s.items.iter().all(|it| it.level <= MAX_LEVEL));
        assert_eq!(s.items[0].text.chars().count(), MAX_TEXT);
        assert!(!s.items[0].text.contains('\n'));
    }

    #[test]
    fn parents_and_descendants_follow_levels() {
        let s = SmartArtSpec::sample(SmartArtLayout::Hierarchy).sanitized();
        assert_eq!(s.parents(), vec![None, Some(0), Some(1), Some(0), Some(3), Some(3)]);
        assert_eq!(s.descendants(0), 1..6);
        assert_eq!(s.descendants(1), 2..3);
        assert_eq!(s.descendants(5), 6..6);
        assert_eq!(s.descendants(99), 0..0);
        assert_eq!(SmartArtLayout::from_id("VENN"), Some(SmartArtLayout::Venn));
        assert_eq!(SmartArtColors::from_id("colorful"), Some(SmartArtColors::Colorful));
    }
}
