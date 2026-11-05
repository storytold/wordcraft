//! The document's own ribbon: tabs, groups and buttons that run the document's macros (the
//! ribbon customisation part of an Office package, `customUI/customUI14.xml`). Buttons the user
//! adds from the Developer tab live here and travel with the file; Word shows them too.
//!
//! Only ribbons made of tabs, groups and macro buttons are modelled. Richer customisations
//! (pictures of their own, menus, callbacks other than a button's `onAction`) stay in the file
//! unchanged (kept as package parts by the DOCX reader); their buttons still show and run,
//! read-only.

use serde::{Deserialize, Serialize};

/// Most tabs, groups per tab and buttons per group a document's ribbon may hold.
pub const MAX_RIBBON_ITEMS: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ribbon {
    pub tabs: Vec<RibbonTab>,
}

/// A tab: one of the built-in tabs (`idMso`, `TabHome`) gaining groups, or a tab of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RibbonTab {
    /// The control id of a custom tab (`id`); empty for a built-in tab.
    #[serde(default)]
    pub id: String,
    /// The built-in tab (`idMso`: `TabHome`, `TabDeveloper`), when this is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub groups: Vec<RibbonGroup>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RibbonGroup {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub buttons: Vec<RibbonButton>,
}

/// A button that runs a macro: `on_action` is the macro's name (`Module1.Hello`), which Word
/// calls with the button (`Sub Hello(control As IRibbonControl)`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RibbonButton {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub on_action: String,
    /// A built-in picture by name (`imageMso`): WordCraft draws its own icon for the names it
    /// knows; the name is kept for Word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub large: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screentip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supertip: Option<String>,
    /// Free text handed to the macro (`control.Tag`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl Ribbon {
    pub fn is_empty(&self) -> bool {
        self.tabs.iter().all(|t| t.groups.iter().all(|g| g.buttons.is_empty()))
    }

    /// Every control id in use (tabs, groups, buttons).
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.tabs.iter().flat_map(|t| {
            std::iter::once(t.id.as_str())
                .chain(t.groups.iter().flat_map(|g| std::iter::once(g.id.as_str()).chain(g.buttons.iter().map(|b| b.id.as_str()))))
        })
    }

    /// A control id not in use yet: `prefix1`, `prefix2`…
    pub fn fresh_id(&self, prefix: &str) -> String {
        let used: std::collections::HashSet<&str> = self.ids().collect();
        let mut n = 1usize;
        loop {
            let id = format!("{prefix}{n}");
            if !used.contains(id.as_str()) {
                return id;
            }
            n = n.saturating_add(1);
        }
    }

    pub fn button(&self, id: &str) -> Option<&RibbonButton> {
        self.tabs.iter().flat_map(|t| &t.groups).flat_map(|g| &g.buttons).find(|b| b.id == id)
    }

    pub fn button_mut(&mut self, id: &str) -> Option<&mut RibbonButton> {
        self.tabs.iter_mut().flat_map(|t| &mut t.groups).flat_map(|g| &mut g.buttons).find(|b| b.id == id)
    }

    /// Removes the button `id`, and its group and tab when they are left empty. Whether it was there.
    pub fn remove_button(&mut self, id: &str) -> bool {
        let mut found = false;
        for t in &mut self.tabs {
            for g in &mut t.groups {
                let before = g.buttons.len();
                g.buttons.retain(|b| b.id != id);
                found |= g.buttons.len() != before;
            }
            t.groups.retain(|g| !g.buttons.is_empty());
        }
        self.tabs.retain(|t| !t.groups.is_empty());
        found
    }
}

/// Word's built-in tab ids (`idMso`) and the WordCraft tab each one is. The first id of a tab is
/// the one written; the others are read as the same tab.
pub const BUILTIN_TABS: &[(&str, &str)] = &[
    ("TabHome", "Home"),
    ("TabInsert", "Insert"),
    ("TabDesign", "Design"),
    ("TabWordDesign", "Design"),
    ("TabPageLayoutWord", "Layout"),
    ("TabReferences", "References"),
    ("TabMailings", "Mailings"),
    ("TabReviewWord", "Review"),
    ("TabReview", "Review"),
    ("TabView", "View"),
    ("TabDeveloper", "Developer"),
];

/// The tab WordCraft shows a built-in tab id as (`TabReferences` → References).
pub fn builtin_tab_name(id_mso: &str) -> Option<&'static str> {
    BUILTIN_TABS.iter().find(|(k, _)| k.eq_ignore_ascii_case(id_mso)).map(|(_, v)| *v)
}

/// The built-in tab id of a WordCraft tab name (References → `TabReferences`).
pub fn builtin_tab_id(name: &str) -> Option<&'static str> {
    BUILTIN_TABS.iter().find(|(_, v)| v.eq_ignore_ascii_case(name)).map(|(k, _)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(id: &str) -> RibbonButton {
        RibbonButton { id: id.into(), label: id.into(), on_action: "Module1.Go".into(), ..Default::default() }
    }

    #[test]
    fn ids_are_fresh_and_removal_tidies_up() {
        let mut r = Ribbon {
            tabs: vec![RibbonTab {
                id: "craftTab1".into(),
                label: "Mine".into(),
                groups: vec![RibbonGroup { id: "craftGroup1".into(), label: "G".into(), buttons: vec![button("craftButton1"), button("b")] }],
                ..Default::default()
            }],
        };
        assert_eq!(r.fresh_id("craftButton"), "craftButton2");
        assert_eq!(r.fresh_id("craftTab"), "craftTab2");
        assert!(r.button("b").is_some());
        assert!(r.remove_button("b"));
        assert!(!r.remove_button("b"));
        assert!(!r.is_empty());
        assert!(r.remove_button("craftButton1"));
        assert!(r.tabs.is_empty() && r.is_empty(), "an empty group and tab go too");
    }

    #[test]
    fn builtin_tabs_map_both_ways() {
        assert_eq!(builtin_tab_name("tabreferences"), Some("References"));
        assert_eq!(builtin_tab_name("TabWordDesign"), Some("Design"));
        assert_eq!(builtin_tab_id("layout"), Some("TabPageLayoutWord"));
        assert_eq!(builtin_tab_id("Review"), Some("TabReviewWord"));
        assert_eq!(builtin_tab_id("Macros"), None);
    }
}
