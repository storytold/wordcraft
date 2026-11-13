//! Requirements between add-ins (manifest 1.3): which app versions, plugin protocol, host features
//! and other add-ins (in which versions) an add-in needs, and resolving them over every installed
//! add-in together.
//!
//! [`resolve`] is pure: it takes a snapshot of the installed add-ins ([`Node`]) and the host
//! ([`HostInfo`]) and returns which add-ins are blocked and why, the start order (required add-ins
//! first) and who needs whom. The registry ([`crate::refresh`]) applies it.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::version::{Range, Version};

/// Most entries in `requires.addins`.
pub const MAX_ADDINS: usize = 32;
/// Longest `why` (characters).
pub const MAX_WHY: usize = 200;
/// Longest chain of required add-ins below an add-in (links).
pub const MAX_DEPTH: usize = 8;
/// Most entries in `requires.apps` and `requires.features`.
const MAX_LIST: usize = 64;

/// A manifest's `requires`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requires {
    /// Per app id, the app versions the add-in works with; an app missing here isn't constrained.
    #[serde(default)]
    pub apps: BTreeMap<String, String>,
    /// The plugin protocol versions it needs (`>=1.2`); empty: any.
    #[serde(default)]
    pub protocol: String,
    /// Names from `host.features` that must all be present.
    #[serde(default)]
    pub features: Vec<String>,
    /// Other add-ins.
    #[serde(default)]
    pub addins: Vec<AddinRequirement>,
}

/// One entry of `requires.addins`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddinRequirement {
    pub id: String,
    /// A version range; empty: any (`*`).
    #[serde(default)]
    pub version: String,
    /// `owner/repo` to install it from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// Never blocks; the add-in learns whether it's there from `initialize`'s `addins`.
    #[serde(default)]
    pub optional: bool,
    /// Shown to the person (at most 200 characters).
    #[serde(default)]
    pub why: String,
}

impl Requires {
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty() && self.protocol.trim().is_empty() && self.features.is_empty() && self.addins.is_empty()
    }

    /// The manifest's limits. Ranges aren't checked here: one that doesn't parse blocks the
    /// add-in instead ([`resolve`]).
    pub(crate) fn validate(&self, own_id: &str) -> Result<(), String> {
        let m = crate::MANIFEST;
        if self.addins.len() > MAX_ADDINS {
            return Err(format!("{m}: at most {MAX_ADDINS} entries in `requires.addins`"));
        }
        if self.apps.len() > MAX_LIST || self.features.len() > MAX_LIST {
            return Err(format!("{m}: at most {MAX_LIST} entries in `requires.apps` and `requires.features`"));
        }
        if let Some(a) = self.apps.keys().find(|a| !crate::valid_id(a)) {
            return Err(format!("{m}: `requires.apps` names `{a}`, which isn't an app id"));
        }
        if self.features.iter().any(|f| f.trim().is_empty() || f.len() > 64) {
            return Err(format!("{m}: `requires.features` needs names of 1–64 characters"));
        }
        let mut seen = BTreeSet::new();
        for r in &self.addins {
            if !crate::valid_id(&r.id) {
                return Err(format!("{m}: `requires.addins` id `{}` must be 1–128 letters, digits, `.`, `-` or `_`", r.id));
            }
            if r.id == own_id {
                return Err(format!("{m}: add-in {own_id} requires itself"));
            }
            if !seen.insert(r.id.as_str()) {
                return Err(format!("{m}: `requires.addins` names {} twice", r.id));
            }
            if r.why.chars().count() > MAX_WHY {
                return Err(format!("{m}: `why` for {} is longer than {MAX_WHY} characters", r.id));
            }
            if let Some(g) = &r.github
                && !valid_github(g)
            {
                return Err(format!("{m}: `github` for {} must be `owner/repo`", r.id));
            }
        }
        Ok(())
    }

    /// The required (not optional) add-ins.
    pub fn required(&self) -> impl Iterator<Item = &AddinRequirement> {
        self.addins.iter().filter(|r| !r.optional)
    }
}

fn valid_github(g: &str) -> bool {
    matches!(crate::install::parse_github_url(g), Ok(repo) if repo.version == crate::install::Version::Latest)
}

impl AddinRequirement {
    /// Its version range (`*` when empty).
    pub fn range(&self) -> Result<Range, String> {
        Range::parse(&self.version)
    }

    /// The range as written, `*` when empty.
    pub fn need(&self) -> String {
        let v = self.version.trim();
        if v.is_empty() { "*".into() } else { v.to_string() }
    }

    /// What to call it when it isn't installed: the repository's name, else the id.
    pub fn label(&self) -> String {
        self.github.as_deref().and_then(|g| crate::install::parse_github_url(g).ok()).map(|r| r.name).unwrap_or_else(|| self.id.clone())
    }
}

// ---------------------------------------------------------------- resolution

/// Why an add-in is blocked (`plugins.list`'s `blockedBy[].kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// `requires.apps`: this app's version.
    App,
    /// `requires.protocol`.
    Protocol,
    /// `requires.features`, or an Office add-in's requirement sets and methods.
    Feature,
    /// A required add-in: missing, off, failed, blocked or out of range.
    Addin,
    /// A requirement cycle.
    Cycle,
    /// A range in the manifest that doesn't parse.
    Range,
    /// Required add-ins more than [`MAX_DEPTH`] deep.
    Depth,
}

/// One reason an add-in is blocked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    /// The add-in (or app, API set) this is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub kind: Kind,
    /// What's needed (a range, a feature name).
    pub need: String,
    /// What there is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub have: Option<String>,
    /// For people: "Needs craft-vba ^0.1.4 (installed: 0.1.3)".
    pub message: String,
}

impl Blocker {
    fn new(id: Option<&str>, kind: Kind, need: impl Into<String>, have: Option<String>, message: String) -> Blocker {
        Blocker { id: id.map(str::to_string), kind, need: need.into(), have, message }
    }
}

/// The app add-ins are resolved against.
#[derive(Clone, Debug)]
pub struct HostInfo {
    /// `wordcraft`.
    pub id: String,
    /// `WordCraft`.
    pub name: String,
    pub version: Version,
    /// The plugin protocol it speaks (`initialize`'s `host.protocol`).
    pub protocol: Version,
    pub features: Vec<String>,
    /// Office.js requirement sets and the highest version of each.
    pub api_sets: Vec<(String, Version)>,
    /// Office.js Common API methods (`Document.getSelectedDataAsync`).
    pub methods: Vec<String>,
}

/// An Office add-in's `<Requirements>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OfficeNeeds {
    /// (set name, minimum version).
    pub sets: Vec<(String, String)>,
    pub methods: Vec<String>,
}

/// One installed add-in, as resolution sees it.
#[derive(Clone, Debug, Default)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Turned on.
    pub enabled: bool,
    /// Stopped after failing too often.
    pub failed: bool,
    pub requires: Requires,
    pub office: Option<OfficeNeeds>,
}

/// What [`resolve`] decides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolution {
    /// The blocked add-ins and why (an add-in absent here isn't blocked).
    pub blocked: BTreeMap<String, Vec<Blocker>>,
    /// Every add-in, required ones before those that need them (load order otherwise); add-ins in
    /// a cycle come last.
    pub order: Vec<String>,
    /// Per add-in, the installed add-ins that require it (not optionally), in load order.
    pub required_by: BTreeMap<String, Vec<String>>,
}

impl Resolution {
    pub fn is_blocked(&self, id: &str) -> bool {
        self.blocked.contains_key(id)
    }
}

/// Checks the app, protocol, features and Office requirements of one add-in: the reasons that
/// don't depend on other add-ins.
fn own_blockers(n: &Node, host: &HostInfo) -> Vec<Blocker> {
    let mut out = Vec::new();
    let app = &host.name;
    for (app_id, text) in &n.requires.apps {
        if !app_id.eq_ignore_ascii_case(&host.id) {
            continue;
        }
        match Range::parse(text) {
            Ok(r) if r.matches(&host.version) => {}
            Ok(r) => out.push(Blocker::new(
                Some(&host.id),
                Kind::App,
                r.as_str(),
                Some(host.version.to_string()),
                format!("Needs {app} {} (this is {})", r.describe(), host.version),
            )),
            Err(_) => out.push(Blocker::new(
                Some(&host.id),
                Kind::Range,
                text.trim(),
                None,
                format!("Its manifest's requirement for {app} (`{}`) isn't a version range", short(text)),
            )),
        }
    }
    if !n.requires.protocol.trim().is_empty() {
        match Range::parse(&n.requires.protocol) {
            Ok(r) if r.matches(&host.protocol) => {}
            Ok(r) => out.push(Blocker::new(
                None,
                Kind::Protocol,
                r.as_str(),
                Some(protocol_text(&host.protocol)),
                format!("Needs plugin protocol {} (this {app} has {})", r.describe(), protocol_text(&host.protocol)),
            )),
            Err(_) => out.push(Blocker::new(
                None,
                Kind::Range,
                n.requires.protocol.trim(),
                None,
                format!("Its manifest's requirement for the plugin protocol (`{}`) isn't a version range", short(&n.requires.protocol)),
            )),
        }
    }
    for f in &n.requires.features {
        if !host.features.iter().any(|h| h == f.trim()) {
            out.push(Blocker::new(Some(f.trim()), Kind::Feature, f.trim(), None, format!("Needs `{}`, which this {app} doesn't have", f.trim())));
        }
    }
    for r in &n.requires.addins {
        if r.range().is_err() {
            out.push(Blocker::new(
                Some(&r.id),
                Kind::Range,
                r.version.trim(),
                None,
                format!("Its manifest's requirement for {} (`{}`) isn't a version range", r.label(), short(&r.version)),
            ));
        }
    }
    if let Some(o) = &n.office {
        out.extend(office_blockers(o, host));
    }
    out
}

/// An Office add-in's requirement sets and methods against what the Office.js library reports.
pub fn office_blockers(o: &OfficeNeeds, host: &HostInfo) -> Vec<Blocker> {
    let app = &host.name;
    let mut out = Vec::new();
    for (set, min) in &o.sets {
        let have = host.api_sets.iter().find(|(n, _)| n.eq_ignore_ascii_case(set)).map(|(_, v)| v);
        let want = Version::parse(if min.trim().is_empty() { "1.1" } else { min });
        match (have, want) {
            (_, Err(_)) => out.push(Blocker::new(
                Some(set),
                Kind::Range,
                min.trim(),
                None,
                format!("Its manifest's requirement {set} {} isn't a version", short(min)),
            )),
            (None, Ok(_)) => out.push(Blocker::new(
                Some(set),
                Kind::Feature,
                format!("{set} {}", min.trim()),
                None,
                format!("Needs {set} {}, which {app} doesn't support", min.trim()),
            )),
            (Some(h), Ok(w)) if w > *h => out.push(Blocker::new(
                Some(set),
                Kind::Feature,
                format!("{set} {}", min.trim()),
                Some(format!("{set} {}", set_text(h))),
                format!("Needs {set} {}; {app} supports up to {}", min.trim(), set_text(h)),
            )),
            _ => {}
        }
    }
    for m in &o.methods {
        if !host.methods.iter().any(|h| h.eq_ignore_ascii_case(m.trim())) {
            out.push(Blocker::new(Some(m.trim()), Kind::Feature, m.trim(), None, format!("Needs {}, which {app} doesn't support", m.trim())));
        }
    }
    out
}

/// `1.4` for 1.4.0 (requirement sets are written MAJOR.MINOR).
fn set_text(v: &Version) -> String {
    if v.patch == 0 && v.pre.is_empty() { format!("{}.{}", v.major, v.minor) } else { v.to_string() }
}

/// `2.0` for 2.0.0.
pub fn protocol_text(v: &Version) -> String {
    set_text(v)
}

fn short(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 40 { format!("{}…", s.chars().take(40).collect::<String>()) } else { s.to_string() }
}

/// Resolves every installed add-in together (see the module docs).
pub fn resolve(nodes: &[Node], host: &HostInfo) -> Resolution {
    let n = nodes.len();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        index.entry(node.id.as_str()).or_insert(i);
    }
    // Required edges to installed add-ins: i needs j.
    let edges: Vec<Vec<usize>> = nodes
        .iter()
        .map(|node| {
            let mut e: Vec<usize> = node.requires.required().filter_map(|r| index.get(r.id.as_str()).copied()).collect();
            e.dedup();
            e
        })
        .collect();
    let mut required_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (i, e) in edges.iter().enumerate() {
        for &j in e {
            if let (Some(dep), Some(user)) = (nodes.get(j), nodes.get(i)) {
                required_by.entry(dep.id.clone()).or_default().push(user.id.clone());
            }
        }
    }
    // Cycles: the add-ins that reach themselves, each with the path back.
    let cycle: Vec<Option<Vec<usize>>> = (0..n).map(|i| cycle_through(i, &edges)).collect();
    let in_cycle = |i: usize| cycle.get(i).is_some_and(Option::is_some);
    // Kahn over the add-ins outside cycles: required ones first, load order between equals.
    let mut waiting: Vec<usize> = (0..n).map(|i| edges.get(i).map_or(0, |e| e.iter().filter(|&&j| !in_cycle(j)).count())).collect();
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| !in_cycle(i) && waiting.get(i) == Some(&0)).collect();
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, e) in edges.iter().enumerate() {
        for &j in e {
            if let Some(u) = users.get_mut(j) {
                u.push(i);
            }
        }
    }
    let mut topo = Vec::with_capacity(n);
    while let Some(i) = ready.pop_first() {
        topo.push(i);
        for &u in users.get(i).map(Vec::as_slice).unwrap_or(&[]) {
            if in_cycle(u) {
                continue;
            }
            if let Some(w) = waiting.get_mut(u) {
                *w = w.saturating_sub(1);
                if *w == 0 {
                    ready.insert(u);
                }
            }
        }
    }
    // Decide in that order, so each add-in sees whether its required ones are usable.
    let mut blocked: Vec<Vec<Blocker>> = vec![Vec::new(); n];
    let mut depth: Vec<usize> = vec![0; n];
    let usable =
        |j: usize, blocked: &[Vec<Blocker>]| nodes.get(j).is_some_and(|d| d.enabled && !d.failed) && blocked.get(j).is_some_and(Vec::is_empty);
    for i in (0..n).filter(|&i| in_cycle(i)) {
        let Some(node) = nodes.get(i) else { continue };
        let mut b = own_blockers(node, host);
        if let Some(Some(path)) = cycle.get(i) {
            let names: Vec<&str> = path.iter().filter_map(|&k| nodes.get(k)).map(|k| k.name.as_str()).collect();
            let ids: Vec<&str> = path.iter().filter_map(|&k| nodes.get(k)).map(|k| k.id.as_str()).collect();
            b.push(Blocker::new(ids.get(1).copied(), Kind::Cycle, ids.join(" → "), None, format!("Requirement cycle: {}", names.join(" → "))));
        }
        if let Some(slot) = blocked.get_mut(i) {
            *slot = b;
        }
    }
    for &i in &topo {
        let Some(node) = nodes.get(i) else { continue };
        let mut b = own_blockers(node, host);
        let d = edges.get(i).map_or(0, |e| e.iter().filter(|&&j| !in_cycle(j)).filter_map(|&j| depth.get(j)).map(|x| x + 1).max().unwrap_or(0));
        if let Some(slot) = depth.get_mut(i) {
            *slot = d;
        }
        if d > MAX_DEPTH {
            b.push(Blocker::new(
                None,
                Kind::Depth,
                format!("at most {MAX_DEPTH}"),
                Some(d.to_string()),
                format!("Its required add-ins go {d} deep (at most {MAX_DEPTH})"),
            ));
        }
        for r in node.requires.required() {
            let Ok(range) = r.range() else { continue }; // Reported by own_blockers.
            let Some(&j) = index.get(r.id.as_str()) else {
                b.push(Blocker::new(Some(&r.id), Kind::Addin, r.need(), None, format!("Needs {}, which isn't installed", r.label())));
                continue;
            };
            let Some(dep) = nodes.get(j) else { continue };
            let name = &dep.name;
            let in_range = Version::parse(&dep.version).map(|v| range.matches(&v)).unwrap_or(range.is_any());
            let have = Some(if dep.version.trim().is_empty() { "?".to_string() } else { dep.version.trim().to_string() });
            let (msg, have) = if !in_range {
                (format!("Needs {name} {} (installed: {})", r.need(), have.as_deref().unwrap_or("?")), have)
            } else if !dep.enabled {
                (format!("Needs {name}, which is turned off"), have)
            } else if dep.failed {
                (format!("Needs {name}, which has failed"), have)
            } else if !usable(j, &blocked) {
                let why = blocked.get(j).and_then(|x| x.first()).map(|x| x.message.clone()).unwrap_or_default();
                (format!("Needs {name}, which is blocked ({why})"), have)
            } else {
                continue;
            };
            b.push(Blocker::new(Some(&r.id), Kind::Addin, r.need(), have, msg));
        }
        if let Some(slot) = blocked.get_mut(i) {
            *slot = b;
        }
    }
    let mut order: Vec<String> = topo.iter().filter_map(|&i| nodes.get(i)).map(|x| x.id.clone()).collect();
    order.extend((0..n).filter(|&i| in_cycle(i)).filter_map(|i| nodes.get(i)).map(|x| x.id.clone()));
    let blocked = nodes.iter().zip(blocked).filter(|(_, b)| !b.is_empty()).map(|(x, b)| (x.id.clone(), b)).collect();
    Resolution { blocked, order, required_by }
}

/// A path `i → … → i` along required edges, if there is one (shortest, breadth first).
fn cycle_through(i: usize, edges: &[Vec<usize>]) -> Option<Vec<usize>> {
    let mut parent: HashMap<usize, usize> = HashMap::new();
    let mut queue = VecDeque::from([i]);
    while let Some(k) = queue.pop_front() {
        for &j in edges.get(k).map(Vec::as_slice).unwrap_or(&[]) {
            if j == i {
                // Walk back from k to i.
                let mut back = Vec::new();
                let mut at = k;
                while at != i {
                    back.push(at);
                    at = *parent.get(&at)?;
                    if back.len() > edges.len() {
                        return None;
                    }
                }
                back.reverse();
                let mut path = vec![i];
                path.extend(back);
                path.push(i);
                return Some(path);
            }
            if let std::collections::hash_map::Entry::Vacant(e) = parent.entry(j) {
                e.insert(k);
                queue.push_back(j);
            }
        }
    }
    None
}

// ---------------------------------------------------------------- installing

/// Installing `id` at `version` would leave an add-in that requires it out of range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// The add-in that requires it.
    pub dependent: String,
    pub need: String,
    pub version: String,
    /// "Reports needs craft-vba ^0.1.4; 0.2.0 doesn't match".
    pub message: String,
}

/// The installed, turned-on add-ins whose (required) range for `id` `version` doesn't match.
pub fn conflicts(nodes: &[Node], id: &str, name: &str, version: &str) -> Vec<Conflict> {
    let v = Version::parse(version);
    nodes
        .iter()
        .filter(|n| n.enabled && n.id != id)
        .filter_map(|n| {
            let r = n.requires.required().find(|r| r.id == id)?;
            let range = r.range().ok()?;
            let fits = match &v {
                Ok(v) => range.matches(v),
                Err(_) => range.is_any(),
            };
            (!fits).then(|| Conflict {
                dependent: n.id.clone(),
                need: r.need(),
                version: version.to_string(),
                message: format!(
                    "{} needs {name} {}; {} doesn't match",
                    n.name,
                    r.need(),
                    if version.trim().is_empty() { "?" } else { version.trim() }
                ),
            })
        })
        .collect()
}

/// One of an add-in's requirements that isn't met by what's installed: missing or out of range.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Missing {
    pub id: String,
    /// The repository's name, else the id.
    pub name: String,
    /// The range (`*` for any).
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    pub optional: bool,
    pub why: String,
    /// The installed version when it's out of range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed: Option<String>,
}

/// `node`'s requirements (optional ones too) that are missing or out of range among `nodes`.
/// Ranges that don't parse are left out: installing can't fix them.
pub fn missing(node: &Node, nodes: &[Node]) -> Vec<Missing> {
    node.requires
        .addins
        .iter()
        .filter_map(|r| {
            let range = r.range().ok()?;
            let installed = nodes.iter().find(|n| n.id == r.id);
            let fits = installed.is_some_and(|d| Version::parse(&d.version).map(|v| range.matches(&v)).unwrap_or(range.is_any()));
            (!fits).then(|| Missing {
                id: r.id.clone(),
                name: r.label(),
                version: r.need(),
                github: r.github.clone(),
                optional: r.optional,
                why: r.why.clone(),
                installed: installed.map(|d| d.version.clone()),
            })
        })
        .collect()
}

/// The release to install for `range` from a GitHub release list (`GET /repos/o/r/releases`):
/// the newest one whose tag is a version in range (`v0.1.5` or `0.1.5`). Drafts are skipped,
/// and so are releases GitHub marks as pre-releases unless the range names a pre-release; for
/// `*`, pre-release versions are skipped too. Returns its index in `releases` and its version.
pub fn pick_release(releases: &[Json], range: &Range) -> Option<(usize, Version)> {
    let mut best: Option<(usize, Version)> = None;
    for (i, r) in releases.iter().enumerate() {
        if r.get("draft").and_then(Json::as_bool) == Some(true) {
            continue;
        }
        let flagged = r.get("prerelease").and_then(Json::as_bool) == Some(true);
        let Some(v) = r.get("tag_name").and_then(Json::as_str).and_then(Version::from_tag) else { continue };
        let pre_ok = !range.is_any() && range.allows_prereleases();
        if (flagged || v.is_prerelease()) && !pre_ok {
            continue;
        }
        if !range.matches(&v) {
            continue;
        }
        if best.as_ref().is_none_or(|(_, b)| v > *b) {
            best = Some((i, v));
        }
    }
    best
}

#[cfg(test)]
mod tests;
