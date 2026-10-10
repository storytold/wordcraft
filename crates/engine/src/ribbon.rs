//! Customize Ribbon and the Quick Access Toolbar (`tools.customizeRibbon`): which ribbon tabs are
//! shown and in what order, custom tabs, custom groups (on built-in or custom tabs) holding any
//! commands, and the commands on the Quick Access Toolbar.
//!
//! Built-in groups aren't editable: a built-in tab only gains custom groups after its own. The
//! layout is saved with the front end's preferences; reading a saved layout never fails: junk,
//! duplicates and commands this version doesn't have are dropped.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Registry;

/// The ribbon's tabs in their default order. `File` opens Backstage: it is always first and
/// isn't part of the customizable list.
pub const TABS: [&str; 12] = ["File", "Home", "Insert", "Draw", "Design", "Layout", "References", "Mailings", "Review", "View", "Zotero", "Help"];
/// Tabs that come up with the selection; custom tabs can't take their names.
pub const CONTEXTUAL_TABS: [&str; 5] = ["Table Design", "Table Layout", "Picture Format", "Shape Format", "Equation"];
/// The Quick Access Toolbar's commands until the user changes them.
pub const DEFAULT_QAT: [&str; 3] = ["file.save", "edit.undo", "edit.redo"];

/// Limits on what a (possibly hand-edited) saved layout can hold.
const MAX_TABS: usize = 64;
const MAX_GROUPS: usize = 64;
const MAX_COMMANDS: usize = 200;
const MAX_NAME: usize = 64;
const MAX_ID: usize = 128;

/// A ribbon tab in the customized order: a built-in tab (which can be hidden and gain custom
/// groups) or a custom one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TabEntry {
    pub name: String,
    #[serde(skip_serializing_if = "is_false")]
    pub custom: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Custom groups, after a built-in tab's own groups.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<CustomGroup>,
}

/// A custom group: a name and the commands it shows, in order.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CustomGroup {
    pub name: String,
    pub commands: Vec<String>,
}

/// The user's ribbon and Quick Access Toolbar.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RibbonLayout {
    /// Every tab but File, in order, once the ribbon is customized; empty for the default ribbon.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<TabEntry>,
    /// The Quick Access Toolbar's commands; `None` for the default ones ([`DEFAULT_QAT`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qat: Option<Vec<String>>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A name the user typed: trimmed, without control characters, at most [`MAX_NAME`] characters.
fn clean_name(s: &str) -> Option<String> {
    let s: String = s.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn built_in(name: &str) -> bool {
    TABS.iter().skip(1).any(|t| *t == name)
}

/// Names a custom tab can't have.
fn reserved(name: &str) -> bool {
    TABS.iter().chain(CONTEXTUAL_TABS.iter()).any(|t| same(t, name))
}

fn default_tabs() -> Vec<TabEntry> {
    TABS.iter().skip(1).map(|t| TabEntry { name: (*t).to_string(), custom: false, hidden: false, groups: Vec::new() }).collect()
}

/// `base`, or `base 2`, `base 3`… whichever `taken` doesn't have yet.
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..1000).map(|n| format!("{base} {n}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

/// Move the item at `i` by `by` places (clamped to the list). Returns its new index.
fn shift<T>(v: &mut [T], i: usize, by: i64) -> usize {
    let last = v.len().saturating_sub(1) as i64;
    let to = (i as i64).saturating_add(by).clamp(0, last) as usize;
    if to < i {
        if let Some(s) = v.get_mut(to..=i) {
            s.rotate_right(1);
        }
    } else if let Some(s) = v.get_mut(i..=to) {
        s.rotate_left(1);
    }
    to
}

impl RibbonLayout {
    /// Read a saved layout, keeping only well-formed entries (never fails).
    pub fn from_value(v: &Value) -> RibbonLayout {
        let mut out = RibbonLayout::default();
        for t in v.get("tabs").and_then(Value::as_array).into_iter().flatten().take(MAX_TABS) {
            let Some(name) = t.get("name").and_then(Value::as_str).and_then(clean_name) else { continue };
            let custom = t.get("custom").and_then(Value::as_bool).unwrap_or(false);
            let hidden = t.get("hidden").and_then(Value::as_bool).unwrap_or(false);
            let mut groups: Vec<CustomGroup> = Vec::new();
            for g in t.get("groups").and_then(Value::as_array).into_iter().flatten().take(MAX_GROUPS) {
                let Some(gname) = g.get("name").and_then(Value::as_str).and_then(clean_name) else { continue };
                if groups.iter().any(|x| same(&x.name, &gname)) {
                    continue;
                }
                let mut commands: Vec<String> = Vec::new();
                for id in g.get("commands").and_then(Value::as_array).into_iter().flatten().take(MAX_COMMANDS).filter_map(Value::as_str) {
                    if !id.is_empty() && id.len() <= MAX_ID && !commands.iter().any(|c| c == id) {
                        commands.push(id.to_string());
                    }
                }
                groups.push(CustomGroup { name: gname, commands });
            }
            out.tabs.push(TabEntry { name, custom, hidden, groups });
        }
        if let Some(q) = v.get("qat").and_then(Value::as_array) {
            let mut ids: Vec<String> = Vec::new();
            for id in q.iter().take(MAX_COMMANDS).filter_map(Value::as_str) {
                if !id.is_empty() && id.len() <= MAX_ID && !ids.iter().any(|c| c == id) {
                    ids.push(id.to_string());
                }
            }
            out.qat = Some(ids);
        }
        out.normalize();
        out
    }

    /// Keep the tab list consistent: built-in tabs once each (missing ones back in their default
    /// place), custom tabs with unique, unreserved names; the default ribbon is stored as empty.
    fn normalize(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let mut tabs: Vec<TabEntry> = Vec::new();
        for t in std::mem::take(&mut self.tabs) {
            let ok = if t.custom { !reserved(&t.name) } else { built_in(&t.name) };
            if ok && !tabs.iter().any(|x| same(&x.name, &t.name)) {
                tabs.push(t);
            }
        }
        // Built-in tabs a saved layout lacks (a newer version's) go after the tab before them.
        for (i, name) in TABS.iter().enumerate().skip(1) {
            if tabs.iter().any(|t| !t.custom && t.name == *name) {
                continue;
            }
            let at = TABS.get(1..i).and_then(|before| before.iter().rev().find_map(|b| tabs.iter().position(|t| !t.custom && t.name == *b)));
            let entry = TabEntry { name: (*name).to_string(), custom: false, hidden: false, groups: Vec::new() };
            tabs.insert(at.map_or(0, |p| p + 1), entry);
        }
        self.tabs = if tabs == default_tabs() { Vec::new() } else { tabs };
    }

    /// The tabs in order (File excluded), customized or not.
    pub fn tabs(&self) -> Cow<'_, [TabEntry]> {
        if self.tabs.is_empty() { Cow::Owned(default_tabs()) } else { Cow::Borrowed(&self.tabs) }
    }

    /// Whether the ribbon is the default one.
    pub fn ribbon_is_default(&self) -> bool {
        self.tabs.is_empty()
    }

    /// The Quick Access Toolbar's commands.
    pub fn qat(&self) -> Vec<&str> {
        match &self.qat {
            Some(q) => q.iter().map(String::as_str).collect(),
            None => DEFAULT_QAT.to_vec(),
        }
    }

    /// Drop commands that don't exist (saved by another version, or junk).
    pub fn retain_known(&mut self, reg: &Registry) {
        for t in &mut self.tabs {
            for g in &mut t.groups {
                g.commands.retain(|id| reg.get(id).is_some());
            }
        }
        if let Some(q) = &mut self.qat {
            q.retain(|id| reg.get(id).is_some());
        }
    }

    /// Edit the tab list (customizing a default ribbon first), then tidy it up.
    fn edit<T>(&mut self, f: impl FnOnce(&mut Vec<TabEntry>) -> Result<T, String>) -> Result<T, String> {
        let mut tabs = self.tabs().into_owned();
        let r = f(&mut tabs)?;
        self.tabs = tabs;
        self.normalize();
        Ok(r)
    }

    fn tab_index(tabs: &[TabEntry], name: &str) -> Result<usize, String> {
        tabs.iter().position(|t| t.name == name).ok_or_else(|| format!("no tab `{name}`"))
    }

    fn group_index(tab: &TabEntry, name: &str) -> Result<usize, String> {
        tab.groups.iter().position(|g| g.name == name).ok_or_else(|| format!("tab `{}` has no custom group `{name}`", tab.name))
    }

    /// Show or hide a tab.
    pub fn set_hidden(&mut self, tab: &str, hidden: bool) -> Result<(), String> {
        self.edit(|tabs| {
            let i = Self::tab_index(tabs, tab)?;
            if let Some(t) = tabs.get_mut(i) {
                t.hidden = hidden;
            }
            Ok(())
        })
    }

    /// Add a custom tab (with one empty group) after `after`, or at the end. Returns its name.
    pub fn new_tab(&mut self, name: Option<&str>, after: Option<&str>) -> Result<String, String> {
        let name = match name {
            Some(n) => clean_name(n).ok_or("a tab needs a name")?,
            None => "New Tab".to_string(),
        };
        self.edit(|tabs| {
            if tabs.len() >= MAX_TABS {
                return Err("too many tabs".into());
            }
            let name = unique(&name, |n| reserved(n) || tabs.iter().any(|t| same(&t.name, n)));
            let at = match after {
                Some(a) => Self::tab_index(tabs, a)? + 1,
                None => tabs.len(),
            };
            let group = CustomGroup { name: "New Group".into(), commands: Vec::new() };
            tabs.insert(at, TabEntry { name: name.clone(), custom: true, hidden: false, groups: vec![group] });
            Ok(name)
        })
    }

    /// Add a custom group at the end of a tab. Returns its name.
    pub fn new_group(&mut self, tab: &str, name: Option<&str>) -> Result<String, String> {
        let name = match name {
            Some(n) => clean_name(n).ok_or("a group needs a name")?,
            None => "New Group".to_string(),
        };
        self.edit(|tabs| {
            let i = Self::tab_index(tabs, tab)?;
            let t = tabs.get_mut(i).ok_or("no such tab")?;
            if t.groups.len() >= MAX_GROUPS {
                return Err("too many groups on this tab".into());
            }
            let name = unique(&name, |n| t.groups.iter().any(|g| same(&g.name, n)));
            t.groups.push(CustomGroup { name: name.clone(), commands: Vec::new() });
            Ok(name)
        })
    }

    /// Add a command to a custom group (after `after`, or at the end). False when it's already there.
    pub fn add(&mut self, reg: &Registry, tab: &str, group: &str, id: &str, after: Option<&str>) -> Result<bool, String> {
        if reg.get(id).is_none() {
            return Err(format!("unknown command `{id}`"));
        }
        self.edit(|tabs| {
            let i = Self::tab_index(tabs, tab)?;
            let t = tabs.get_mut(i).ok_or("no such tab")?;
            let j = Self::group_index(t, group)?;
            let g = t.groups.get_mut(j).ok_or("no such group")?;
            if g.commands.iter().any(|c| c == id) {
                return Ok(false);
            }
            if g.commands.len() >= MAX_COMMANDS {
                return Err("too many commands in this group".into());
            }
            let at = after.and_then(|a| g.commands.iter().position(|c| c == a)).map_or(g.commands.len(), |p| p + 1);
            g.commands.insert(at, id.to_string());
            Ok(true)
        })
    }

    /// Remove a command from a custom group, a custom group, or a custom tab.
    pub fn remove(&mut self, tab: &str, group: Option<&str>, command: Option<&str>) -> Result<(), String> {
        self.edit(|tabs| {
            let i = Self::tab_index(tabs, tab)?;
            let Some(group) = group else {
                if !tabs.get(i).is_some_and(|t| t.custom) {
                    return Err(format!("`{tab}` is a built-in tab: hide it instead"));
                }
                tabs.remove(i);
                return Ok(());
            };
            let t = tabs.get_mut(i).ok_or("no such tab")?;
            let j = Self::group_index(t, group)?;
            match command {
                None => {
                    t.groups.remove(j);
                }
                Some(id) => {
                    let g = t.groups.get_mut(j).ok_or("no such group")?;
                    let k = g.commands.iter().position(|c| c == id).ok_or_else(|| format!("`{group}` doesn't have `{id}`"))?;
                    g.commands.remove(k);
                }
            }
            Ok(())
        })
    }

    /// Rename a custom tab or a custom group. Returns the new name.
    pub fn rename(&mut self, tab: &str, group: Option<&str>, to: &str) -> Result<String, String> {
        let to = clean_name(to).ok_or("the new name is empty")?;
        self.edit(|tabs| {
            let i = Self::tab_index(tabs, tab)?;
            match group {
                None => {
                    if !tabs.get(i).is_some_and(|t| t.custom) {
                        return Err(format!("`{tab}` is a built-in tab and keeps its name"));
                    }
                    if reserved(&to) || tabs.iter().enumerate().any(|(k, t)| k != i && same(&t.name, &to)) {
                        return Err(format!("there is already a tab named `{to}`"));
                    }
                    if let Some(t) = tabs.get_mut(i) {
                        t.name = to.clone();
                    }
                }
                Some(group) => {
                    let t = tabs.get_mut(i).ok_or("no such tab")?;
                    let j = Self::group_index(t, group)?;
                    if t.groups.iter().enumerate().any(|(k, g)| k != j && same(&g.name, &to)) {
                        return Err(format!("`{tab}` already has a group named `{to}`"));
                    }
                    if let Some(g) = t.groups.get_mut(j) {
                        g.name = to.clone();
                    }
                }
            }
            Ok(to)
        })
    }

    /// Move a tab, a custom group (within its tab) or a command (within its group) by `by` places.
    pub fn move_by(&mut self, tab: &str, group: Option<&str>, command: Option<&str>, by: i64) -> Result<(), String> {
        self.edit(|tabs| {
            let i = Self::tab_index(tabs, tab)?;
            let Some(group) = group else {
                shift(tabs, i, by);
                return Ok(());
            };
            let t = tabs.get_mut(i).ok_or("no such tab")?;
            let j = Self::group_index(t, group)?;
            match command {
                None => {
                    shift(&mut t.groups, j, by);
                }
                Some(id) => {
                    let g = t.groups.get_mut(j).ok_or("no such group")?;
                    let k = g.commands.iter().position(|c| c == id).ok_or_else(|| format!("`{group}` doesn't have `{id}`"))?;
                    shift(&mut g.commands, k, by);
                }
            }
            Ok(())
        })
    }

    /// Back to the default ribbon (the Quick Access Toolbar stays).
    pub fn reset_ribbon(&mut self) {
        self.tabs.clear();
    }

    /// Back to the default Quick Access Toolbar.
    pub fn reset_qat(&mut self) {
        self.qat = None;
    }

    /// Put a command on the Quick Access Toolbar (after `after`, or at the end). False when it's
    /// already there.
    pub fn qat_add(&mut self, reg: &Registry, id: &str, after: Option<&str>) -> Result<bool, String> {
        if reg.get(id).is_none() {
            return Err(format!("unknown command `{id}`"));
        }
        let mut q: Vec<String> = self.qat().into_iter().map(str::to_string).collect();
        if q.iter().any(|c| c == id) {
            return Ok(false);
        }
        if q.len() >= MAX_COMMANDS {
            return Err("the Quick Access Toolbar is full".into());
        }
        let at = after.and_then(|a| q.iter().position(|c| c == a)).map_or(q.len(), |p| p + 1);
        q.insert(at, id.to_string());
        self.qat = Some(q);
        Ok(true)
    }

    /// Take a command off the Quick Access Toolbar. False when it wasn't there.
    pub fn qat_remove(&mut self, id: &str) -> bool {
        let mut q: Vec<String> = self.qat().into_iter().map(str::to_string).collect();
        let Some(i) = q.iter().position(|c| c == id) else { return false };
        q.remove(i);
        self.qat = Some(q);
        true
    }

    /// Move a Quick Access Toolbar command by `by` places.
    pub fn qat_move(&mut self, id: &str, by: i64) -> Result<(), String> {
        let mut q: Vec<String> = self.qat().into_iter().map(str::to_string).collect();
        let i = q.iter().position(|c| c == id).ok_or_else(|| format!("`{id}` isn't on the Quick Access Toolbar"))?;
        shift(&mut q, i, by);
        self.qat = Some(q);
        Ok(())
    }
}

impl<'de> Deserialize<'de> for RibbonLayout {
    /// Lenient: any JSON reads (junk dropped), so a damaged layout never loses the other
    /// preferences saved next to it.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(RibbonLayout::from_value(&Value::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// #379: custom tabs and groups, hidden and moved tabs, and the Quick Access Toolbar, changed
    /// through the command (as agents do) without opening the dialog.
    #[test]
    fn layout_changes_through_the_command() {
        let mut s = crate::Session::new(wordcraft_doc::Document::new());
        let mut run = |v: Value| s.run("tools.customizeRibbon", &v);
        let r = run(json!({"newTab": {"name": "Mine", "after": "Home"}, "hide": "Draw"})).unwrap();
        assert_eq!(r["tab"], "Mine");
        run(json!({"add": {"tab": "Mine", "group": "New Group", "command": "format.bold"}})).unwrap();
        run(json!({"newGroup": {"tab": "Insert", "name": "Extras"}})).unwrap();
        run(json!({"add": {"tab": "Insert", "group": "Extras", "command": "edit.copy"}})).unwrap();
        run(json!({"rename": {"tab": "Mine", "to": "Writing"}})).unwrap();
        let r = run(json!({"move": {"tab": "Writing", "by": -5}, "qatAdd": "format.bold", "qatRemove": "file.save"})).unwrap();
        let tabs = r["tabs"].as_array().unwrap();
        assert_eq!(tabs[0]["name"], "Writing");
        assert_eq!(tabs[0]["groups"][0]["commands"], json!(["format.bold"]));
        assert!(tabs.iter().any(|t| t["name"] == "Draw" && t["hidden"] == true));
        assert_eq!(r["qat"], json!(["edit.undo", "edit.redo", "format.bold"]));
        // Built-in tabs keep their names and can only be hidden; reserved and unknown names fail.
        assert!(run(json!({"rename": {"tab": "Home", "to": "Start"}})).is_err());
        assert!(run(json!({"remove": {"tab": "Home"}})).is_err());
        assert!(run(json!({"rename": {"tab": "Writing", "to": "insert"}})).is_err());
        assert!(run(json!({"add": {"tab": "Writing", "group": "New Group", "command": "no.such"}})).is_err());
        assert!(run(json!({"move": {"tab": "Nope", "by": 1}})).is_err());
        drop(run);
        assert!(s.ui_requests.is_empty(), "programmatic calls never open dialogs");
        s.run("tools.customizeRibbon", &json!({"reset": true})).unwrap();
        assert_eq!(s.ribbon, RibbonLayout::default());
        s.run("tools.customizeRibbon", &json!({})).unwrap();
        assert_eq!(s.ui_requests.last(), Some(&json!({"open": "customizeRibbon"})));
    }

    #[test]
    fn saved_layouts_round_trip_and_junk_is_ignored() {
        let reg = crate::cmd::registry();
        let mut l = RibbonLayout::default();
        l.new_tab(None, None).unwrap();
        l.add(&reg, "New Tab", "New Group", "format.italic", None).unwrap();
        l.set_hidden("Mailings", true).unwrap();
        l.qat_add(&reg, "edit.repeat", None).unwrap();
        let back: RibbonLayout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back, l);

        let junk = json!({
            "tabs": [
                {"name": "Home", "groups": [{"name": "G", "commands": ["format.bold", "gone.command", 7, "format.bold"]}]},
                {"name": "Home"},
                {"name": "Bogus"},
                {"name": "Review", "custom": true},
                {"name": "  "},
                {"name": "Mine", "custom": true, "hidden": "yes", "groups": "x"},
                5
            ],
            "qat": ["edit.undo", "gone.command", 3, "edit.undo"]
        });
        let mut j: RibbonLayout = serde_json::from_value(junk).unwrap();
        j.retain_known(&reg);
        let tabs = j.tabs();
        assert_eq!(tabs.len(), TABS.len(), "11 built-in tabs and Mine: {tabs:?}");
        assert_eq!(tabs[0].groups[0].commands, ["format.bold"]);
        assert!(tabs.iter().any(|t| t.custom && t.name == "Mine" && !t.hidden));
        assert_eq!(j.qat(), ["edit.undo"]);
        for v in [json!(null), json!(5), json!("x"), json!({"tabs": {"a": 1}, "qat": "file.save"})] {
            assert_eq!(serde_json::from_value::<RibbonLayout>(v).unwrap(), RibbonLayout::default());
        }
    }
}
