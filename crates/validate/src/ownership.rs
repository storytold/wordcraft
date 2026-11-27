//! Explicit document scopes and the bounded part graph used by owner-local joins.
use crate::{Package, Resolution, package};
use std::collections::{BTreeMap, BTreeSet};

/// A normalized package identity; original entry names remain available for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PartId(String);
impl PartId {
    /// Package paths compare case-insensitively, after relationship target normalization.
    pub(crate) fn new(path: &str) -> Self {
        Self(path.trim_start_matches('/').to_ascii_lowercase())
    }
    /// The normalized identity, never a file-system path.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Index into one graph's explicit owner contexts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OwnerId(pub(crate) usize);

/// The source of an owner-local scope. Fallback is an explicit legacy supporting-part scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OwnerContext {
    Main(String),
    Glossary(String),
    Fallback,
}
impl OwnerContext {
    /// The established story root, or no root for the explicit fallback context.
    pub(crate) fn root(&self) -> Option<&str> {
        match self {
            Self::Main(name) | Self::Glossary(name) => Some(name),
            Self::Fallback => None,
        }
    }
}

/// Known memberships, established unowned status, and incomplete reachability are distinct.
#[derive(Debug)]
pub(crate) enum Membership {
    Owned(Vec<OwnerId>),
    Unowned,
    Unknown,
}

/// Root scopes do not depend on selecting a unique package glossary source.
pub(crate) fn owner_scopes(pkg: &Package) -> Vec<OwnerContext> {
    observation_contract! {
        rules: "-";
        collections: "field.content_types*1,field.entries*1,type.BTreeSet*1,type.Vec*2";
        scope: "explicit nearest-story-root graph; each glossary-typed part is its own root; shared membership retained; Unowned and Unknown distinct";
        loss: "missing/truncated manifest or relationships; unavailable entry identity; graph traversal/fanout budget";
        fallback: "never substitute main for unreached references; incomplete discovery yields Unknown; only named package-registry consumers may admit unowned declarations";
        package_loss: "independent";
    }

    let mut scopes = Vec::new();
    if let Resolution::One(main) = &pkg.main {
        scopes.push(OwnerContext::Main(main.clone()));
    }
    if let Resolution::One(glossary) = &pkg.glossary
        && !scopes.iter().any(|o| o.root().is_some_and(|r| r.eq_ignore_ascii_case(glossary)))
    {
        scopes.push(OwnerContext::Glossary(glossary.clone()));
    }
    let roots: BTreeSet<_> = pkg
        .entries
        .iter()
        .filter(|e| !e.is_dir && pkg.content_types.of(&e.name) == Some(package::GLOSSARY_CONTENT_TYPE))
        .map(|e| e.name.clone())
        .collect();
    for root in roots {
        if !scopes.iter().any(|o| o.root().is_some_and(|r| r.eq_ignore_ascii_case(&root))) {
            scopes.push(OwnerContext::Glossary(root));
        }
    }
    if scopes.is_empty() {
        scopes.push(OwnerContext::Fallback);
    }
    scopes
}

/// Membership data, with no default membership in main for unreached entries.
#[derive(Debug)]
pub(crate) struct PartGraph {
    memberships: BTreeMap<PartId, Membership>,
    budget_exhausted: bool,
    memberships_complete: bool,
}
impl PartGraph {
    /// Traverse each context independently, stopping at every other story root.
    pub(crate) fn new(pkg: &Package) -> Self {
        // Each glossary is independent: cap root-by-edge fanout as well as the package's input bytes.
        Self::with_budget(pkg, 200_000)
    }
    /// Bound traversal steps and queued edges; retain known memberships and mark unreached entries unknown on loss.
    fn with_budget(pkg: &Package, mut remaining: usize) -> Self {
        observation_contract! {
            rules: "-";
            collections: "field.entries*1,field.rels*2,type.BTreeMap*3,type.BTreeSet*2";
            scope: "explicit nearest-story-root graph; each glossary-typed part is its own root; shared membership retained; Unowned and Unknown distinct";
            loss: "missing/truncated manifest or relationships; unavailable entry identity; graph traversal/fanout budget";
            fallback: "never substitute main for unreached references; incomplete discovery yields Unknown; only named package-registry consumers may admit unowned declarations";
            package_loss: "independent";
        }

        let owners = owner_scopes(pkg);
        let complete = crate::ownership_discovery_complete(pkg);
        let names: BTreeMap<_, _> = pkg.entries.iter().filter(|e| !e.is_dir).map(|e| (PartId::new(&e.name), e.name.as_str())).collect();
        let relationships: BTreeMap<_, _> = pkg.rels.iter().map(|rp| (PartId::new(&rp.name), rp)).collect();
        let mut memberships: BTreeMap<_, _> =
            names.keys().cloned().map(|id| (id, if complete { Membership::Unowned } else { Membership::Unknown })).collect();
        if owners == [OwnerContext::Fallback] {
            if complete {
                for member in memberships.values_mut() {
                    *member = Membership::Owned(vec![OwnerId(0)]);
                }
            }
            return Self { memberships, budget_exhausted: false, memberships_complete: complete };
        }
        let roots: BTreeSet<_> = owners.iter().filter_map(OwnerContext::root).map(PartId::new).collect();
        // Root ownership itself is established even if descendant traversal later exhausts its allowance.
        for (index, owner) in owners.iter().enumerate() {
            if let Some(root) = owner.root() {
                memberships.insert(PartId::new(root), Membership::Owned(vec![OwnerId(index)]));
            }
        }
        let mut budget_exhausted = false;
        'owners: for (index, owner) in owners.iter().enumerate() {
            let Some(root) = owner.root() else { continue };
            let root_id = PartId::new(root);
            // Borrow canonical entry names: repeated edges must not multiply path-string allocations.
            let mut queue = vec![root];
            let mut visited = BTreeSet::new();
            while let Some(part) = queue.pop() {
                let Some(next) = remaining.checked_sub(1) else {
                    budget_exhausted = true;
                    break 'owners;
                };
                remaining = next;
                let id = PartId::new(part);
                if (roots.contains(&id) && id != root_id) || !visited.insert(id.clone()) {
                    continue;
                }
                match memberships.entry(id).or_insert(Membership::Unknown) {
                    Membership::Owned(contexts) if contexts.last() != Some(&OwnerId(index)) => contexts.push(OwnerId(index)),
                    Membership::Owned(_) => {}
                    state => *state = Membership::Owned(vec![OwnerId(index)]),
                }
                let rel_name = package::rels_part_name(part);
                if let Some(rp) = relationships.get(&PartId::new(&rel_name)) {
                    for rel in &rp.rels {
                        let Some(next) = remaining.checked_sub(1) else {
                            budget_exhausted = true;
                            break 'owners;
                        };
                        remaining = next;
                        if let Some(target) = &rel.target_abs
                            && let Some(&name) = names.get(&PartId::new(target))
                        {
                            queue.push(name);
                        }
                    }
                }
            }
        }
        if budget_exhausted {
            for state in memberships.values_mut() {
                if matches!(state, Membership::Unowned) {
                    *state = Membership::Unknown;
                }
            }
        }
        Self { memberships, budget_exhausted, memberships_complete: complete && !budget_exhausted }
    }
    /// Whether ownership coverage was limited by graph fanout, independent of XML/ZIP loss.
    pub(crate) fn budget_exhausted(&self) -> bool {
        self.budget_exhausted
    }
    /// Whether each retained owner set is exhaustive, rather than only a proven prefix.
    pub(crate) fn memberships_complete(&self) -> bool {
        self.memberships_complete
    }
    /// Proven contexts of this part; negative conclusions also require memberships_complete.
    pub(crate) fn owners(&self, part: &str) -> &[OwnerId] {
        observation_contract! {
            rules: "-";
            collections: "field.memberships*1";
            scope: "explicit nearest-story-root graph; each glossary-typed part is its own root; shared membership retained; Unowned and Unknown distinct";
            loss: "missing/truncated manifest or relationships; unavailable entry identity; graph traversal/fanout budget";
            fallback: "never substitute main for unreached references; incomplete discovery yields Unknown; only named package-registry consumers may admit unowned declarations";
            package_loss: "independent";
        }

        match self.memberships.get(&PartId::new(part)) {
            Some(Membership::Owned(owners)) => owners,
            Some(Membership::Unowned | Membership::Unknown) | None => &[],
        }
    }
    /// Whether discovery leaves membership uncertain, rather than established-unowned.
    pub(crate) fn unknown(&self, part: &str) -> bool {
        observation_contract! {
            rules: "-";
            collections: "field.memberships*1";
            scope: "explicit nearest-story-root graph; each glossary-typed part is its own root; shared membership retained; Unowned and Unknown distinct";
            loss: "missing/truncated manifest or relationships; unavailable entry identity; graph traversal/fanout budget";
            fallback: "never substitute main for unreached references; incomplete discovery yields Unknown; only named package-registry consumers may admit unowned declarations";
            package_loss: "independent";
        }

        matches!(self.memberships.get(&PartId::new(part)), None | Some(Membership::Unknown))
    }
    /// Reached members of one context, including shared parts in each context.
    pub(crate) fn parts(&self, owner: OwnerId) -> impl Iterator<Item = &str> {
        observation_contract! {
            rules: "-";
            collections: "field.memberships*1";
            scope: "explicit nearest-story-root graph; each glossary-typed part is its own root; shared membership retained; Unowned and Unknown distinct";
            loss: "missing/truncated manifest or relationships; unavailable entry identity; graph traversal/fanout budget";
            fallback: "never substitute main for unreached references; incomplete discovery yields Unknown; only named package-registry consumers may admit unowned declarations";
            package_loss: "independent";
        }

        self.memberships.iter().filter_map(move |(part, state)| match state {
            Membership::Owned(owners) if owners.contains(&owner) => Some(part.as_str()),
            _ => None,
        })
    }
}

/// Intentional package-registry scope for declarations in established-unowned parts.
/// Only CUSTOMXML-SCHEMA and informational unreferenced-comment coverage may use this.
/// Owner-local reference joins must use PartGraph directly.
pub(crate) struct PackageRegistryContext {
    contexts: BTreeMap<PartId, Vec<OwnerId>>,
}
impl PackageRegistryContext {
    /// Preserve reached contexts; attach established-unowned declarations to the primary registry.
    /// Unknown membership never establishes a package-registry absence conclusion.
    pub(crate) fn new(graph: &PartGraph) -> Self {
        observation_contract! {
            rules: "-";
            collections: "field.memberships*1";
            scope: "DECLARED EXCEPTION constructor: established-unowned own declarations use the primary package registry, only for CUSTOMXML-SCHEMA and informational unreferenced-comment coverage";
            loss: "missing/truncated manifest or relationships; unavailable entry identity; graph traversal/fanout budget";
            fallback: "never substitute main for unreached references; incomplete discovery yields Unknown; only named package-registry consumers may admit unowned declarations";
            package_loss: "independent";
        }

        let contexts = graph
            .memberships
            .iter()
            .filter_map(|(part, state)| {
                let owners = match state {
                    Membership::Owned(owners) => owners.clone(),
                    Membership::Unowned => vec![OwnerId(0)],
                    Membership::Unknown => return None,
                };
                Some((part.clone(), owners))
            })
            .collect();
        Self { contexts }
    }
    /// Registry contexts only; these are never story ownership memberships.
    pub(crate) fn contexts(&self, part: &str) -> &[OwnerId] {
        observation_contract! {
            rules: "-";
            collections: "field.contexts*1,type.Vec*1";
            scope: "explicit nearest-story-root graph; each glossary-typed part is its own root; shared membership retained; Unowned and Unknown distinct";
            loss: "missing/truncated manifest or relationships; unavailable entry identity; graph traversal/fanout budget";
            fallback: "never substitute main for unreached references; incomplete discovery yields Unknown; only named package-registry consumers may admit unowned declarations";
            package_loss: "independent";
        }

        self.contexts.get(&PartId::new(part)).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolution_tests::common;
    use crate::{Acc, package};

    #[test]
    fn package_registry_exception_never_grants_story_ownership() {
        let mut doc = common::Doc::minimal();
        doc.add_with_override("word/orphan.xml", common::CT_COMMENTS, format!(r#"<w:comments {}/>"#, common::NS));
        let mut pkg = package::load(&doc.build(), &mut Acc::default()).unwrap();
        let graph = PartGraph::new(&pkg);
        assert!(graph.owners("word/orphan.xml").is_empty());
        assert!(!graph.unknown("word/orphan.xml"));
        assert_eq!(PackageRegistryContext::new(&graph).contexts("word/orphan.xml"), &[OwnerId(0)]);
        assert!(graph.owners("word/orphan.xml").is_empty());
        pkg.entries_truncated = true;
        let graph = PartGraph::new(&pkg);
        assert!(graph.unknown("word/orphan.xml"));
        assert!(PackageRegistryContext::new(&graph).contexts("word/orphan.xml").is_empty());
    }
    #[test]
    fn graph_budget_preserves_roots_without_manufacturing_unowned_membership() {
        let mut doc = common::Doc::minimal();
        doc.add_with_override("word/orphan.xml", common::CT_HEADER, format!(r#"<w:hdr {}><w:p/></w:hdr>"#, common::NS));
        let pkg = package::load(&doc.build(), &mut Acc::default()).unwrap();
        let graph = PartGraph::with_budget(&pkg, 0);
        assert!(graph.budget_exhausted());
        assert_eq!(graph.owners("word/document.xml"), &[OwnerId(0)]);
        assert!(graph.unknown("word/orphan.xml"));
        assert!(graph.owners("word/orphan.xml").is_empty());
        assert!(PackageRegistryContext::new(&graph).contexts("word/orphan.xml").is_empty());
    }
}
