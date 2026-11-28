//! Grammar engine: ECMA-376 content-model checks over the generated tables in
//! [`crate::model`].
//!
//! All five grammar rules ship at [`crate::Severity::Warning`] and never move a
//! document's [`crate::Verdict`] (they never produce an `Error` and never call
//! `set_incomplete()`).
//!
//! Evaluation runs against the **Strict** model first. Where a Transitional
//! override exists for the selected type, it acts as a veto only: a clause
//! that the override does not also violate is suppressed, and a child the
//! override requires is never itself required (Word writes Transitional, so a
//! Strict-only offence may be an artefact of the profile, never the reverse).
//! Transitional-only *element names* are unmodelled; they can appear as
//! children only where a parent's override lists them, and where no override
//! exists the parent model is identical in both profiles, so a membership
//! offence against it is an offence under Transitional too.
//!
//! Model selection is by parent context: an element with several type variants
//! takes the variant whose expanded parent contexts contain the parent frame's
//! selected type. Where no variant can be selected, the element instance runs
//! in **fallback mode**: membership is judged against the union of its
//! variants, and the cardinality clauses — required children and
//! max-occurrences — are not judged at all. A fallback is a scope fact
//! recorded as an informational coverage note, never an incompleteness.
//! Fallback is per element instance: a fallback element's children select
//! independently — a single-variant child is still selected and judged, and a
//! multi-variant child falls back in its turn.
//!
//! Choice groups are evaluated by summing the observed counts of their member
//! names, membership taken from the tables' `choice_group` tags: a choice may
//! offer a whole group as one branch (the chart label types choose between a
//! bare `c:delete` and the `Group_DLbls` sequence), the extractor flattens
//! that branch into the group, and the presence of any of its members
//! satisfies the branch; a branch that can match empty content (an optional
//! single element, or a group branch whose members are all optional) makes the
//! whole choice nullable, so a group containing one never fires.
//!
//! Every grammar finding carries its own 1-based element occurrence path from
//! the part root (for example `w:document/w:body[1]/w:tbl[1]`) directly in its
//! message, leaving the shared [`crate::Finding`] struct and every pre-existing
//! rule's output untouched.

use std::collections::{HashMap, HashSet};

use crate::model::{ChildSpec, Compositor, MaxOccurs, NUM_FMT_VALUES, TypeModel, Variant, element_model, transitional_type_model, type_model};
use crate::{Acc, NS_A, NS_B, NS_C, NS_M, NS_PIC, NS_W, NS_WP};

/// Which modelled vocabularies a part enables. Conflicting declarations from
/// content types and relationships never pick a scope by traversal order.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GrammarScope {
    /// Grammar does not run in this part (package-level parts, custom XML).
    #[default]
    None,
    /// A WordprocessingML part: the `w`, `m`, `b`, `wp` and `pic` models.
    Wml,
    /// A chart part: the `c` models, plus `a` (a chart's `c:spPr` is `a`-typed).
    Chart,
    /// A theme part: the `a` models.
    Theme,
    /// Two or more incompatible scopes were declared for this part; grammar is skipped.
    Conflicting,
}

impl GrammarScope {
    /// Combine two scope declarations without letting order pick a winner.
    pub(crate) fn merge(self, other: Self) -> Self {
        if self == Self::None {
            other
        } else if other == Self::None || self == other {
            self
        } else {
            Self::Conflicting
        }
    }

    /// Whether grammar checks run in a part with this scope.
    pub(crate) fn is_active(self) -> bool {
        !matches!(self, Self::None | Self::Conflicting)
    }

    /// Whether this scope models elements with the given table prefix.
    pub(crate) fn allows(self, prefix: &str) -> bool {
        match self {
            GrammarScope::None | GrammarScope::Conflicting => false,
            GrammarScope::Wml => matches!(prefix, "w" | "m" | "b" | "wp" | "pic"),
            GrammarScope::Chart => matches!(prefix, "c" | "a"),
            GrammarScope::Theme => prefix == "a",
        }
    }
}

/// The table prefix for a namespace URI, covering exactly the modelled
/// vocabularies. An element whose namespace has no prefix here is unmodelled.
pub(crate) fn prefix_for(ns: Option<&[u8]>) -> Option<&'static str> {
    let uri = ns?;
    if uri == NS_W.as_bytes() {
        Some("w")
    } else if uri == NS_M.as_bytes() {
        Some("m")
    } else if uri == NS_B.as_bytes() {
        Some("b")
    } else if uri == NS_WP.as_bytes() {
        Some("wp")
    } else if uri == NS_PIC.as_bytes() {
        Some("pic")
    } else if uri == NS_A.as_bytes() {
        Some("a")
    } else if uri == NS_C.as_bytes() {
        Some("c")
    } else {
        None
    }
}

/// The prefixed name of an element, when its namespace is a modelled one.
pub(crate) fn prefixed_name(ns: Option<&[u8]>, local: &str) -> Option<String> {
    prefix_for(ns).map(|p| format!("{p}:{local}"))
}

/// The key under which a parent frame counts one child observation and displays
/// non-modelled elements in occurrence paths: the prefixed name for a modelled
/// namespace, `{uri}local` for any other namespace, and the bare local name for
/// no namespace.
pub(crate) fn count_key(ns: Option<&[u8]>, local: &str) -> String {
    match prefixed_name(ns, local) {
        Some(p) => p,
        None => match ns {
            Some(uri) => format!("{{{}}}{local}", String::from_utf8_lossy(uri)),
            None => local.to_string(),
        },
    }
}

/// Display/count key for one XML element event: `{?}local` when the element's
/// namespace could not be resolved or its prefix is undeclared (never collapsing
/// an unknown namespace into bare no-namespace `local`), otherwise [`count_key`].
pub(crate) fn display_name(ns: Option<&[u8]>, local: &str, ns_unknown: bool) -> String {
    if ns_unknown { format!("{{?}}{local}") } else { count_key(ns, local) }
}

/// How an element's parent context presents itself to model selection.
pub(crate) enum Parent<'a> {
    /// The element is the part's root: the canonical variant is used.
    Root,
    /// The parent frame selected this type key.
    Modelled(&'a str),
    /// The parent element is unmodelled or in fallback mode: only a
    /// single-variant element can be selected.
    Unmodelled,
}

/// Distinct unmodelled child names retained per element to bound memory.
/// Tripping this bound never sets `Incomplete`; modelled children and total
/// child counts continue to be tracked normally.
const UNKNOWN_CAP: u32 = 32;

/// Distinct sibling display names tracked per parent for occurrence-path
/// ordinals, bounding memory on adversarial inputs with huge distinct tag sets.
const PATH_ORDINAL_CAP: usize = 1024;

/// Grammar state for one open element.
pub(crate) struct GFrame {
    /// The element's prefixed name, as recorded in the tables.
    name: &'static str,
    /// The selected type key (the canonical variant's key in fallback mode;
    /// a fallback frame passes no key to child selection).
    type_key: &'static str,
    /// The selected Strict model; `None` in fallback mode.
    model: Option<&'static TypeModel>,
    /// Fallback mode: cardinality clauses are not judged for this element.
    fallback: bool,
    /// Observed direct-child counts by [`count_key`] (modelled names plus up to
    /// [`UNKNOWN_CAP`] distinct unmodelled ones).
    counts: Vec<(String, u32)>,
    /// Total direct children observed, of any name.
    total: u32,
    /// Distinct unmodelled child names recorded so far.
    unknown: u32,
    /// The unmodelled-name cap tripped: later unknown names are not retained.
    capped: bool,
    /// A direct child was skipped (`mc:AlternateContent`) or had an unresolved
    /// namespace; required-child absence cannot be proven for this element.
    child_set_incomplete: bool,
    /// Order state (`GRAMMAR-ORDER`): the maximum slot position seen so far
    /// among this element's children.
    last_position: Option<u16>,
    /// Order state: an order finding was already emitted for this element.
    order_reported: bool,
}

impl GFrame {
    /// The selected type key, for child model selection.
    pub(crate) fn type_key(&self) -> &str {
        self.type_key
    }

    /// The element's prefixed name.
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    /// Whether this frame is in fallback mode (no cardinality checks).
    pub(crate) fn is_fallback(&self) -> bool {
        self.fallback
    }

    /// Whether the unmodelled-name retention cap has tripped on this frame.
    #[cfg(test)]
    pub(crate) fn capped(&self) -> bool {
        self.capped
    }

    /// Whether `name` is declared as a child of this frame's selected Strict
    /// model or its Transitional override.
    pub(crate) fn is_known_child(&self, name: &str) -> bool {
        self.model.is_some_and(|m| {
            m.children.iter().any(|c| c.name == name) || transitional_type_model(m.key).is_some_and(|o| o.children.iter().any(|c| c.name == name))
        })
    }

    /// Record one direct child, under its [`count_key`]. Returns `true` the
    /// first time this frame trips the unmodelled-name retention cap.
    pub(crate) fn observe(&mut self, name: &str) -> bool {
        observation_contract! {
            rules: "-";
            collections: "field.counts*2";
            scope: "direct-child counts within one open modelled element";
            loss: "distinct unmodelled child names capped at 32 per element to bound memory";
            fallback: "modelled children and total counts stay exact; tripping the cap never sets Incomplete";
            package_loss: "independent";
        }

        self.total = self.total.saturating_add(1);
        if let Some((_, n)) = self.counts.iter_mut().find(|(k, _)| k == name) {
            *n = n.saturating_add(1);
            return false;
        }
        let known = self.is_known_child(name);
        if !known {
            if self.unknown >= UNKNOWN_CAP {
                let was = self.capped;
                self.capped = true;
                return !was;
            }
            self.unknown += 1;
        }
        self.counts.push((name.to_string(), 1));
        false
    }

    /// Observed count of one child name.
    pub(crate) fn count(&self, name: &str) -> u32 {
        observation_contract! {
            rules: "-";
            collections: "field.counts*1";
            scope: "direct-child counts within one open modelled element";
            loss: "distinct unmodelled child names capped at 32 per element";
            fallback: "returns exact count for all modelled children and 0 for unretained unmodelled names";
            package_loss: "independent";
        }

        self.counts.iter().find(|(k, _)| k == name).map_or(0, |(_, n)| *n)
    }
}

/// The outcome of opening an element: its grammar frame, if any, and whether
/// the frame is a fallback the caller should note once per part and name.
pub(crate) struct Opened {
    /// The element's grammar frame (`None`: unmodelled in this scope).
    pub frame: Option<GFrame>,
    /// The element fell back to union membership / no cardinality checks.
    pub fallback: bool,
}

/// Whether a static [`Variant`] from [`crate::model`] lists `key` among its
/// expanded parent type contexts. Isolated so the observation gate's check on
/// `PackageRegistryContext::contexts` stays fail-closed everywhere else.
fn variant_has_parent(v: &Variant, key: &str) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.contexts*1";
        scope: "static ECMA-376 Variant parent-context slice in model.rs (not PackageRegistryContext)";
        loss: "none";
        fallback: "returns whether parent type key matches this static schema variant";
        package_loss: "independent";
    }

    v.contexts.contains(&key)
}

/// Select the model for an element opening under `parent` in a part with the
/// given scope, and build its grammar frame.
pub(crate) fn child_frame(parent: &Parent<'_>, ns: Option<&[u8]>, local: &str, scope: GrammarScope) -> Opened {
    observation_contract! {
        rules: "-";
        collections: "field.first*3,type.Vec*4";
        scope: "static ECMA-376 content-model variant selection for one XML element";
        loss: "ambiguous or unmodelled parent context prevents single-variant selection";
        fallback: "enter per-element fallback mode (union membership only, no cardinality checks)";
        package_loss: "independent";
    }

    let unmodelled = Opened { frame: None, fallback: false };
    let Some(prefix) = prefix_for(ns) else { return unmodelled };
    if !scope.allows(prefix) {
        return unmodelled;
    }
    let name = format!("{prefix}:{local}");
    let Some(elem) = element_model(&name) else { return unmodelled };
    let candidates: Vec<(&'static str, &'static TypeModel)> =
        elem.variants.iter().filter_map(|v| type_model(v.type_key).map(|m| (v.type_key, m))).collect();
    if candidates.is_empty() {
        return unmodelled;
    }
    let selected = if candidates.len() == 1 {
        candidates.first().copied()
    } else {
        match parent {
            Parent::Root => candidates.iter().find(|(k, _)| *k == elem.canonical).copied().or_else(|| candidates.first().copied()),
            Parent::Modelled(key) => {
                let hits: Vec<(&'static str, &'static TypeModel)> = candidates
                    .iter()
                    .filter(|(k, _)| elem.variants.iter().any(|v| v.type_key == *k && variant_has_parent(v, key)))
                    .copied()
                    .collect();
                if hits.len() == 1 { hits.first().copied() } else { None }
            }
            Parent::Unmodelled => None,
        }
    };
    match selected {
        Some((key, model)) => Opened {
            frame: Some(GFrame {
                name: elem.name,
                type_key: key,
                model: Some(model),
                fallback: false,
                counts: Vec::new(),
                total: 0,
                unknown: 0,
                capped: false,
                child_set_incomplete: false,
                last_position: None,
                order_reported: false,
            }),
            fallback: false,
        },
        None => Opened {
            frame: Some(GFrame {
                name: elem.name,
                type_key: elem.canonical,
                model: None,
                fallback: true,
                counts: Vec::new(),
                total: 0,
                unknown: 0,
                capped: false,
                child_set_incomplete: false,
                last_position: None,
                order_reported: false,
            }),
            fallback: true,
        },
    }
}

/// One required-occurrence clause a closed element violates. Tuple variants
/// keep pattern matching inside the closed observation syntax grammar.
#[derive(PartialEq, Eq)]
enum Violation {
    /// A named child below its minimum occurrence count: `(child, observed, min)`.
    Missing(&'static str, u32, u16),
    /// A choice group whose member sum is below its required minimum: `(group, members, observed, min)`.
    MissingChoice(&'static str, Vec<&'static str>, u32, u16),
}

/// The required-occurrence violations of `model` against the counts a closed
/// element accumulated. Pure: shared by Strict evaluation and Transitional veto.
fn required_violations(model: &TypeModel, frame: &GFrame) -> Vec<Violation> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*5";
        scope: "static ECMA-376 required-child clauses over one closed element's direct children";
        loss: "none within a complete closed element; incomplete/fallback frames are filtered by the caller";
        fallback: "nullable choice branches never fire; returns exact missing clauses with observed counts for Transitional veto";
        package_loss: "independent";
    }

    let mut out = Vec::new();
    // (group, members, requirement, has-sequence-branch, sequence-branch-is-all-optional, has-optional-single-branch)
    let mut groups: Vec<(&'static str, Vec<&'static str>, u16, bool, bool, bool)> = Vec::new();
    for c in model.children {
        if let Some(g) = c.choice_group {
            let is_seq = c.compositor == Compositor::Sequence;
            match groups.iter_mut().find(|(id, _, _, _, _, _)| *id == g) {
                Some((_, members, min, has_seq, seq_optional, single_optional)) => {
                    members.push(c.name);
                    *min = (*min).max(c.min);
                    *has_seq = *has_seq || is_seq;
                    *seq_optional = *seq_optional && !(is_seq && c.min >= 1);
                    *single_optional = *single_optional || (!is_seq && c.min == 0);
                }
                None => groups.push((g, vec![c.name], c.min, is_seq, !(is_seq && c.min >= 1), !is_seq && c.min == 0)),
            }
            continue;
        }
        if c.min < 1 {
            continue;
        }
        let observed = if c.name == "<any>" { frame.total } else { frame.count(c.name) };
        if observed < u32::from(c.min) {
            out.push(Violation::Missing(c.name, observed, c.min));
        }
    }
    for (group, members, min, has_seq, seq_optional, single_optional) in groups {
        if min < 1 || single_optional || (has_seq && seq_optional) {
            continue;
        }
        let sum: u32 = members.iter().map(|m| if *m == "<any>" { frame.total } else { frame.count(m) }).sum();
        if sum < u32::from(min) {
            out.push(Violation::MissingChoice(group, members, sum, min));
        }
    }
    out
}

impl Violation {
    /// Whether two violations represent the same clause across Strict and
    /// Transitional profiles. Choice violations are keyed by `(group, min)`,
    /// not member list, so a Transitional widening (e.g. `w:CT_NumPicBullet`)
    /// still matches when both profiles fail on an empty element.
    fn same_clause(&self, other: &Violation) -> bool {
        match (self, other) {
            (Violation::Missing(a, _, x), Violation::Missing(b, _, y)) => a == b && x == y,
            (Violation::MissingChoice(a, _, _, x), Violation::MissingChoice(b, _, _, y)) => a == b && x == y,
            _ => false,
        }
    }
}

/// The required-child violations that survive the Transitional veto.
fn judge(model: &TypeModel, over: Option<&TypeModel>, frame: &GFrame) -> Vec<Violation> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "Strict required-child clauses vetoed by any Transitional override";
        loss: "none";
        fallback: "override only vetoes Strict clauses and never introduces override-only requirements";
        package_loss: "independent";
    }

    let mut violations = required_violations(model, frame);
    if violations.is_empty() {
        return violations;
    }
    if let Some(over) = over {
        let over_violations = required_violations(over, frame);
        violations.retain_mut(|v| {
            let Some(o) = over_violations.iter().find(|o| v.same_clause(o)) else {
                return false;
            };
            if let (Violation::MissingChoice(_, members, observed, _), Violation::MissingChoice(_, om, oo, _)) = (v, o)
                && *oo > *observed
            {
                *members = om.clone();
                *observed = *oo;
            }
            true
        });
    }
    violations
}

/// Judge a closed element's required children (`GRAMMAR-REQUIRED-CHILD`).
/// Runs at element close (prefix-established for the closed element). Fallback
/// frames and frames whose direct-child set is incomplete (`mc:AlternateContent`,
/// unresolved-namespace direct child, or unexpanded entity reference in element
/// content) are skipped so required-child absence is never inferred from
/// partial child evidence.
pub(crate) fn close(frame: &GFrame, part: &str, path: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "GRAMMAR-REQUIRED-CHILD";
        collections: "-";
        scope: "closed modelled element's direct children in a grammar-scoped XML part";
        loss: "fallback selection, skipped mc:AlternateContent child, unresolved-namespace direct child, or unexpanded entity reference in element content";
        fallback: "skip required-child check when the element's child set is incomplete; emit Warning only and never set Incomplete";
        package_loss: "independent";
    }

    if frame.fallback || frame.child_set_incomplete {
        return;
    }
    let Some(model) = frame.model else { return };
    let violations = judge(model, transitional_type_model(model.key), frame);
    if violations.is_empty() {
        return;
    }
    for v in violations {
        let message = match &v {
            Violation::Missing(child, observed, min) => {
                let need = if *min == 1 { "at least one".to_string() } else { format!("at least {min}") };
                let short = u32::from(*min).saturating_sub(*observed);
                if *child == "<any>" {
                    if *observed == 0 {
                        format!("{} at {path} in {part} has no child elements; {} requires {need}", frame.name, model.key)
                    } else {
                        let noun = if *observed == 1 { "child element" } else { "child elements" };
                        format!("{} at {path} in {part} has {observed} {noun} (short by {short}); {} requires {need}", frame.name, model.key)
                    }
                } else if *observed == 0 {
                    format!("{} at {path} in {part} has no {child} element; {} requires {need}", frame.name, model.key)
                } else {
                    let noun = if *observed == 1 { "element" } else { "elements" };
                    format!("{} at {path} in {part} has {observed} {child} {noun} (short by {short}); {} requires {need}", frame.name, model.key)
                }
            }
            Violation::MissingChoice(_, members, observed, min) => {
                let need = if *min == 1 { "at least one of them".to_string() } else { format!("at least {min} of them") };
                let short = u32::from(*min).saturating_sub(*observed);
                if *observed == 0 {
                    format!("{} at {path} in {part} has none of {}; {} requires {need}", frame.name, members.join(", "), model.key)
                } else {
                    format!(
                        "{} at {path} in {part} has {observed} of {} (short by {short}); {} requires {need}",
                        frame.name,
                        members.join(", "),
                        model.key
                    )
                }
            }
        };
        acc.add_grammar("GRAMMAR-REQUIRED-CHILD", Some(part.to_string()), message);
    }
}

/// Whether `child` is a permitted child under `model`: it names a slot, or the
/// model carries an `<any>` wildcard slot.
fn member_of(model: &TypeModel, child: &str) -> bool {
    model.children.iter().any(|c| c.name == child || c.name == "<any>")
}

/// The first slot named `child` in `model`, if any.
fn slot_of<'m>(model: &'m TypeModel, child: &str) -> Option<&'m ChildSpec> {
    model.children.iter().find(|c| c.name == child)
}

/// The combined maximum occurrences allowed for `child` across all slots named
/// `child` in `model` (`None` if `child` has no explicit slot).
fn max_occurs_of(model: &TypeModel, child: &str) -> Option<MaxOccurs> {
    let mut found = false;
    let mut total: u16 = 0;
    for slot in model.children {
        if slot.name != child {
            continue;
        }
        found = true;
        match slot.max {
            MaxOccurs::Unbounded => return Some(MaxOccurs::Unbounded),
            MaxOccurs::Limited(m) => total = total.saturating_add(m),
        }
    }
    if found { Some(MaxOccurs::Limited(total)) } else { None }
}

/// Whether any candidate model for a fallback element permits `child`: the
/// union of every variant's slots, in both Strict and Transitional profiles.
fn union_member(frame: &GFrame, child: &str) -> bool {
    let Some(elem) = element_model(frame.name) else { return false };
    elem.variants.iter().any(|v| {
        type_model(v.type_key).is_some_and(|m| member_of(m, child)) || transitional_type_model(v.type_key).is_some_and(|m| member_of(m, child))
    })
}

/// Whether the tables hold a model for the named element at all. A child the
/// tables do not model (such as MS-OI29500 extensions like `w:uiCompat97to2003`)
/// is unmodelled content, not a membership finding.
pub(crate) fn is_modelled(name: &str) -> bool {
    element_model(name).is_some()
}

/// Judge one observed child of an open element (`GRAMMAR-CHILD-NOT-ALLOWED`,
/// `GRAMMAR-CHILD-TOO-MANY`, and `GRAMMAR-ORDER` inside `w:rPr`). When
/// `count_lower_bound` is `true` (because an earlier unexpanded entity, skipped
/// `mc:AlternateContent`, or unresolved-namespace sibling with matching local
/// name may also contribute instances of `child`), `GRAMMAR-CHILD-TOO-MANY`
/// reports a lower bound (`at least {n}`) rather than an exact count.
pub(crate) fn judge_child(frame: &mut GFrame, child: &str, part: &str, child_path: &str, count_lower_bound: bool, acc: &mut Acc) {
    observation_contract! {
        rules: "GRAMMAR-CHILD-NOT-ALLOWED,GRAMMAR-CHILD-TOO-MANY";
        collections: "-";
        scope: "direct modelled child of an open modelled element in a grammar-scoped XML part";
        loss: "fallback parents have no single model; Transitional overrides veto Strict-only offences; prior unexpanded entity, skipped mc:AlternateContent, or matching-local unresolved-namespace sibling in parent makes child counts a lower bound";
        fallback: "fallback parents check union membership only and skip cardinality/order; GRAMMAR-CHILD-TOO-MANY reports at least N when child counts are a lower bound; emits Warning only";
        package_loss: "independent";
    }

    if frame.fallback {
        if !union_member(frame, child) {
            acc.add_grammar(
                "GRAMMAR-CHILD-NOT-ALLOWED",
                Some(part.to_string()),
                format!(
                    "{child} at {child_path} in {part} is not a permitted child of {}; no candidate model for {} allows it",
                    frame.name, frame.name
                ),
            );
        }
        return;
    }
    let Some(model) = frame.model else { return };
    let over = transitional_type_model(model.key);
    if !member_of(model, child) {
        if over.is_some_and(|o| member_of(o, child)) {
            return;
        }
        acc.add_grammar(
            "GRAMMAR-CHILD-NOT-ALLOWED",
            Some(part.to_string()),
            format!("{child} at {child_path} in {part} is not a permitted child of {}; {} does not allow it", frame.name, model.key),
        );
        return;
    }
    if frame.name == "w:rPr" {
        judge_order(frame, model, child, part, child_path, acc);
    }
    if let Some(MaxOccurs::Limited(max)) = max_occurs_of(model, child) {
        let n = frame.count(child);
        if n == u32::from(max) + 1 {
            let vetoed = over.is_some_and(|o| match max_occurs_of(o, child) {
                Some(MaxOccurs::Unbounded) => true,
                Some(MaxOccurs::Limited(tm)) => n <= u32::from(tm),
                // No slot of this name in the override: permitted via `<any>`, which carries no per-name cap.
                None => member_of(o, child),
            });
            if !vetoed {
                let observed = if count_lower_bound { format!("at least {n}") } else { n.to_string() };
                acc.add_grammar(
                    "GRAMMAR-CHILD-TOO-MANY",
                    Some(part.to_string()),
                    format!("{} at {child_path} in {part} has {observed} {child} elements; {} allows at most {max}", frame.name, model.key),
                );
            }
        }
    }
}

/// Judge one child's position inside a `w:rPr` (`GRAMMAR-ORDER`).
fn judge_order(frame: &mut GFrame, model: &TypeModel, child: &str, part: &str, child_path: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "GRAMMAR-ORDER";
        collections: "-";
        scope: "direct child declaration order inside w:rPr (EG_RPrBase schema order)";
        loss: "none";
        fallback: "fires at most once per w:rPr element at the first inversion; Warning only";
        package_loss: "independent";
    }

    let Some(slot) = slot_of(model, child) else { return };
    let pos = slot.position;
    if let Some(last) = frame.last_position
        && pos < last
        && !frame.order_reported
    {
        frame.order_reported = true;
        let earlier = model.children.iter().find(|c| c.position == last).map_or("?", |c| c.name);
        acc.add_grammar(
            "GRAMMAR-ORDER",
            Some(part.to_string()),
            format!(
                "{child} at {child_path} in {part} is out of order in {}: it appears after {earlier}, but Word reads run properties in schema order and may drop {child}",
                frame.name
            ),
        );
    }
    if frame.last_position.is_none_or(|l| pos > l) {
        frame.last_position = Some(pos);
    }
}

/// Outcome of reading one XML attribute for grammar evaluation, keeping
/// uncertain attribute identity separate from uncertain value delivery.
#[derive(PartialEq, Eq)]
pub(crate) enum GrammarAttrValue {
    /// No target attribute (and no unresolved-prefix attribute with the same
    /// local name) is present on this element.
    Missing,
    /// The target namespaced attribute is present and its XML-delivered value
    /// was resolved.
    Present(String),
    /// The target namespaced attribute is present, but its XML-delivered value
    /// could not be resolved (nested/markup entity, unresolvable DOCTYPE subset,
    /// or grammar output bound).
    ValueUnreadable,
    /// No bound target attribute is present, but an attribute with the same
    /// local name has an unresolved namespace prefix, so whether it identifies
    /// the target attribute cannot be established.
    IdentityUnresolved,
}

/// Why `GRAMMAR-NUMFMT-VALUE` could not evaluate a `w:numFmt` element.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumFmtSkip {
    /// `w:numFmt/@w:val` is present, but its delivered value could not be read.
    ValueUnreadable,
    /// `w:numFmt` carries a `val` attribute whose namespace prefix is unresolved,
    /// so whether it is `w:val` could not be established.
    IdentityUnresolved,
}

/// Judge one `w:numFmt` format value (`GRAMMAR-NUMFMT-VALUE`). Returns
/// `Some(NumFmtSkip)` when checking was skipped due to an undeliverable value
/// or an unresolved attribute namespace identity.
pub(crate) fn judge_num_fmt(val: &GrammarAttrValue, part: &str, path: &str, acc: &mut Acc) -> Option<NumFmtSkip> {
    observation_contract! {
        rules: "GRAMMAR-NUMFMT-VALUE";
        collections: "-";
        scope: "w:numFmt/@w:val in Wml-scope parts against the 63-value ST_NumberFormat table";
        loss: "missing w:val attribute carries no value; bound w:val with undeliverable value (ValueUnreadable) cannot be read; unresolved-prefix val attribute (IdentityUnresolved) has uncertain namespace identity";
        fallback: "exact case-sensitive membership check on delivered values; missing attribute stays silent; ValueUnreadable and IdentityUnresolved skip NUMFMT and return distinct skip reasons for truthful grammar coverage disclosure without Incomplete";
        package_loss: "independent";
    }

    match val {
        GrammarAttrValue::Missing => None,
        GrammarAttrValue::Present(v) => {
            if !NUM_FMT_VALUES.contains(&v.as_str()) {
                acc.add_grammar(
                    "GRAMMAR-NUMFMT-VALUE",
                    Some(part.to_string()),
                    format!("numFmt format \"{v}\" at {path} in {part} is not an ST_NumberFormat value"),
                );
            }
            None
        }
        GrammarAttrValue::ValueUnreadable => Some(NumFmtSkip::ValueUnreadable),
        GrammarAttrValue::IdentityUnresolved => Some(NumFmtSkip::IdentityUnresolved),
    }
}

/// Inputs describing one XML start or empty element for [`GrammarWalk`].
pub(crate) struct ElementEvent<'a> {
    pub ns: Option<&'a [u8]>,
    pub local: &'a str,
    pub is_alt: bool,
    pub unresolved: bool,
    pub ns_unknown: bool,
    pub suppressed: bool,
    pub numfmt_val: &'a GrammarAttrValue,
}

/// One open XML element tracked by [`GrammarWalk`].
struct WalkFrame {
    path: String,
    child_counts: HashMap<String, Option<u32>>,
    unmodelled_path_names: usize,
    all_ordinals_lower_bound: bool,
    uncertain_locals: Vec<String>,
    grammar: Option<GFrame>,
    suppressed: bool,
}

impl WalkFrame {
    /// Whether sibling ordinals and direct-child counts for elements with local
    /// name `local` under this parent are only lower bounds (due to a prior
    /// unexpanded entity, skipped `mc:AlternateContent`, or unresolved-namespace
    /// direct child with the same local name).
    fn is_child_count_lower_bound(&self, local: &str) -> bool {
        observation_contract! {
            rules: "-";
            collections: "field.uncertain_locals*1";
            scope: "direct-child lower-bound uncertainty state on one open WalkFrame";
            loss: "distinct unresolved-namespace local names capped at 1024 per parent frame, promoting to all_ordinals_lower_bound";
            fallback: "conservatively returns true for all local names once promoted; never claims exactness after loss";
            package_loss: "independent";
        }

        self.all_ordinals_lower_bound || self.uncertain_locals.iter().any(|l| l == local)
    }

    /// Record that a direct child with an unresolved namespace had local name
    /// `local`, making subsequent counts and ordinals for `local` lower bounds
    /// (promoting to `all_ordinals_lower_bound` past [`PATH_ORDINAL_CAP`]).
    fn mark_local_uncertain(&mut self, local: &str) {
        observation_contract! {
            rules: "-";
            collections: "field.uncertain_locals*4";
            scope: "direct-child unresolved-namespace local-name set on one open WalkFrame";
            loss: "distinct unresolved-namespace local names capped at 1024 per parent frame";
            fallback: "past 1024 distinct unresolved local names, clears the set and promotes to all_ordinals_lower_bound so no uncertainty is dropped";
            package_loss: "independent";
        }

        if self.all_ordinals_lower_bound || self.uncertain_locals.iter().any(|l| l == local) {
            return;
        }
        if self.uncertain_locals.len() < PATH_ORDINAL_CAP {
            self.uncertain_locals.push(local.to_string());
        } else {
            self.all_ordinals_lower_bound = true;
            self.uncertain_locals.clear();
        }
    }
}

/// Per-part streaming grammar walk, inactive (zero work) when the part has no
/// active [`GrammarScope`].
pub(crate) struct GrammarWalk {
    scope: GrammarScope,
    frames: Vec<WalkFrame>,
    fallback_noted: HashSet<String>,
    cap_noted: bool,
    unreadable_attr_noted: bool,
    unresolved_attr_noted: bool,
}

impl GrammarWalk {
    /// Start a grammar walk for one part under `scope`.
    pub(crate) fn new(scope: GrammarScope) -> Self {
        observation_contract! {
            rules: "-";
            collections: "type.HashSet*1,type.Vec*1";
            scope: "per-part streaming grammar element stack and deduplication sets";
            loss: "inactive or conflicting part scope disables grammar checks for this part";
            fallback: "zero-cost no-op when scope is inactive; never sets Incomplete";
            package_loss: "independent";
        }

        Self {
            scope,
            frames: Vec::new(),
            fallback_noted: HashSet::new(),
            cap_noted: false,
            unreadable_attr_noted: false,
            unresolved_attr_noted: false,
        }
    }

    /// Mark the currently open element's direct-child stream as incomplete (for
    /// example, after an unexpanded entity reference in element content):
    /// suppresses required-child absence on a modelled parent and marks all
    /// subsequent sibling ordinals and child counts under this element as lower
    /// bounds.
    pub(crate) fn mark_child_set_incomplete(&mut self) {
        observation_contract! {
            rules: "-";
            collections: "field.frames*1";
            scope: "currently open element in a grammar-scoped XML part (modelled or unmodelled)";
            loss: "unexpanded entity reference in element content";
            fallback: "suppresses GRAMMAR-REQUIRED-CHILD on the affected open modelled element and marks subsequent sibling ordinals and child counts under this element as lower bounds; unaffected elements stay exact; never sets Incomplete";
            package_loss: "independent";
        }

        if !self.scope.is_active() {
            return;
        }
        if let Some(frame) = self.frames.last_mut()
            && !frame.suppressed
        {
            frame.all_ordinals_lower_bound = true;
            if let Some(pg) = &mut frame.grammar {
                pg.child_set_incomplete = true;
            }
        }
    }

    /// Compute the occurrence path for an element about to open under the
    /// current top frame. Elements whose namespace is unknown/unresolved render
    /// `{?}local[?]` (or `{?}local` at root). Known-namespace elements retain
    /// exact 1-based `[n]` ordinals while no child-stream loss occurred in the
    /// parent; after parent loss they render lower-bound `[>=n]`; unmodelled
    /// names past [`PATH_ORDINAL_CAP`] or overflowing `u32` render `[?]`.
    fn next_path(&mut self, ev: &ElementEvent<'_>) -> String {
        observation_contract! {
            rules: "-";
            collections: "field.child_counts*3,field.frames*1";
            scope: "1-based sibling occurrence path from the part root for grammar findings";
            loss: "distinct unmodelled sibling names capped at 1024 per parent frame; u32 ordinal overflow; unknown/unresolved element namespace; prior unexpanded entity, skipped mc:AlternateContent, or matching-local unresolved-namespace sibling in parent frame";
            fallback: "modelled sibling names are never capped and retain exact [n] ordinals when no parent child-stream loss occurred; after parent loss, subsequent known-namespace siblings render lower-bound [>=n]; elements with unknown/unresolved namespaces render {?}local[?] (or {?}local at root) instead of bare no-namespace names; unmodelled names past the cap or overflowing u32 render [?]";
            package_loss: "independent";
        }

        let display = display_name(ev.ns, ev.local, ev.ns_unknown);
        match self.frames.last_mut() {
            None => display,
            Some(parent) => {
                if ev.ns_unknown {
                    return format!("{}/{display}[?]", parent.path);
                }
                let ordinal = if let Some(slot) = parent.child_counts.get_mut(&display) {
                    *slot = slot.and_then(|c| c.checked_add(1));
                    *slot
                } else {
                    let modelled = prefixed_name(ev.ns, ev.local).is_some_and(|p| is_modelled(&p))
                        || parent.grammar.as_ref().is_some_and(|g| g.is_known_child(&display));
                    if modelled {
                        parent.child_counts.insert(display.clone(), Some(1));
                        Some(1)
                    } else if parent.unmodelled_path_names < PATH_ORDINAL_CAP {
                        parent.unmodelled_path_names += 1;
                        parent.child_counts.insert(display.clone(), Some(1));
                        Some(1)
                    } else {
                        None
                    }
                };
                let lower_bound = parent.is_child_count_lower_bound(ev.local);
                match (ordinal, lower_bound) {
                    (Some(n), false) => format!("{}/{display}[{n}]", parent.path),
                    (Some(n), true) => format!("{}/{display}[>={n}]", parent.path),
                    (None, _) => format!("{}/{display}[?]", parent.path),
                }
            }
        }
    }

    /// Observe an opening or empty element and return its selected grammar
    /// frame (if not suppressed).
    fn enter_element(&mut self, ev: &ElementEvent<'_>, part: &str, path: &str, acc: &mut Acc) -> Option<GFrame> {
        observation_contract! {
            rules: "-";
            collections: "field.fallback_noted*1,field.frames*2";
            scope: "streaming grammar evaluation for one XML start/empty event";
            loss: "suppressed mc:AlternateContent subtrees, unresolved namespaces, undeliverable w:val values, unresolved-prefix val attributes, or >32 unmodelled child names";
            fallback: "records distinct isolated informational grammar notes for undeliverable values vs unresolved attribute identity, propagates parent lower-bound uncertainty for mc:AlternateContent and unresolved-namespace children, never calls set_incomplete(), emits Warning only";
            package_loss: "independent";
        }

        if ev.suppressed {
            return None;
        }
        if self.scope == GrammarScope::Wml && ev.ns == Some(NS_W.as_bytes()) && ev.local == "numFmt" {
            match judge_num_fmt(ev.numfmt_val, part, path, acc) {
                Some(NumFmtSkip::ValueUnreadable) if !self.unreadable_attr_noted => {
                    self.unreadable_attr_noted = true;
                    acc.note_grammar(format!(
                        "{part} has a w:numFmt/@w:val attribute at {path} whose value could not be delivered; format value was not checked"
                    ));
                }
                Some(NumFmtSkip::IdentityUnresolved) if !self.unresolved_attr_noted => {
                    self.unresolved_attr_noted = true;
                    acc.note_grammar(format!(
                        "{part} has a val attribute with an unresolved namespace prefix on w:numFmt at {path}; whether it is w:val could not be established, so format value was not checked"
                    ));
                }
                _ => {}
            }
        }
        let child = display_name(ev.ns, ev.local, ev.ns_unknown);
        if let Some(parent) = self.frames.last_mut() {
            let count_lower_bound = parent.is_child_count_lower_bound(ev.local);
            if ev.is_alt {
                parent.all_ordinals_lower_bound = true;
            }
            if ev.unresolved {
                parent.mark_local_uncertain(ev.local);
            }
            if let Some(pg) = &mut parent.grammar {
                if ev.is_alt || ev.unresolved {
                    pg.child_set_incomplete = true;
                }
                if pg.observe(&child) && !self.cap_noted {
                    self.cap_noted = true;
                    acc.note_grammar(format!(
                        "{part} has an element with more than {UNKNOWN_CAP} distinct unmodelled child names; further unmodelled names were not tracked for grammar"
                    ));
                }
                if let Some(prefix) = prefix_for(ev.ns)
                    && self.scope.allows(prefix)
                    && let Some(display) = prefixed_name(ev.ns, ev.local)
                    && is_modelled(&display)
                {
                    judge_child(pg, &display, part, path, count_lower_bound, acc);
                }
            }
        }
        let parent = match self.frames.last() {
            None => Parent::Root,
            Some(f) => match &f.grammar {
                Some(g) if !g.is_fallback() => Parent::Modelled(g.type_key()),
                _ => Parent::Unmodelled,
            },
        };
        let opened = child_frame(&parent, ev.ns, ev.local, self.scope);
        if opened.fallback
            && let Some(f) = &opened.frame
            && self.fallback_noted.insert(f.name().to_string())
        {
            acc.note_grammar(format!("grammar model for {} in {part} could not be selected by parent context; cardinality not checked", f.name()));
        }
        opened.frame
    }

    /// Process a start tag (`Event::Start`).
    pub(crate) fn open(&mut self, ev: &ElementEvent<'_>, part: &str, acc: &mut Acc) {
        observation_contract! {
            rules: "-";
            collections: "field.frames*1,type.HashMap*1,type.Vec*1";
            scope: "push one XML start-element frame onto the per-part grammar stack";
            loss: "inactive or conflicting scope is a no-op";
            fallback: "suppressed subtrees keep stack balance without emitting grammar findings";
            package_loss: "independent";
        }

        if !self.scope.is_active() {
            return;
        }
        let path = if ev.suppressed { String::new() } else { self.next_path(ev) };
        let grammar = self.enter_element(ev, part, &path, acc);
        self.frames.push(WalkFrame {
            path,
            child_counts: HashMap::new(),
            unmodelled_path_names: 0,
            all_ordinals_lower_bound: false,
            uncertain_locals: Vec::new(),
            grammar,
            suppressed: ev.suppressed,
        });
    }

    /// Process an empty tag (`Event::Empty`): opens and closes immediately.
    pub(crate) fn empty(&mut self, ev: &ElementEvent<'_>, part: &str, acc: &mut Acc) {
        if !self.scope.is_active() {
            return;
        }
        let path = if ev.suppressed { String::new() } else { self.next_path(ev) };
        if let Some(g) = self.enter_element(ev, part, &path, acc) {
            close(&g, part, &path, acc);
        }
    }

    /// Process an end tag (`Event::End`).
    pub(crate) fn close(&mut self, part: &str, acc: &mut Acc) {
        observation_contract! {
            rules: "-";
            collections: "field.frames*1";
            scope: "pop one XML end-element frame and judge its closed required children";
            loss: "inactive or conflicting scope is a no-op";
            fallback: "prefix-established on closed elements; emits Warning only";
            package_loss: "independent";
        }

        if !self.scope.is_active() {
            return;
        }
        if let Some(f) = self.frames.pop()
            && !f.suppressed
            && let Some(g) = &f.grammar
        {
            close(g, part, &f.path, acc);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::Severity;
    use crate::model::{ChildSpec, MaxOccurs, TRANSITIONAL_TYPES, TYPES};

    /// Build the frame `child_frame` selects for `name` under a modelled
    /// parent of type `parent_key` (or at the root when `None`).
    fn select(name: &str, parent_key: Option<&str>, scope: GrammarScope) -> Opened {
        let (prefix, local) = name.split_once(':').unwrap();
        let ns = match prefix {
            "w" => Some(NS_W.as_bytes()),
            "m" => Some(NS_M.as_bytes()),
            "a" => Some(NS_A.as_bytes()),
            "c" => Some(NS_C.as_bytes()),
            "pic" => Some(NS_PIC.as_bytes()),
            _ => None,
        }
        .unwrap();
        let parent = match parent_key {
            None => Parent::Root,
            Some(k) => Parent::Modelled(k),
        };
        child_frame(&parent, Some(ns), local, scope)
    }

    #[test]
    fn flagship_sectpr_selection() {
        let in_body = select("w:sectPr", Some("w:CT_Body"), GrammarScope::Wml);
        assert_eq!(in_body.frame.as_ref().map(|f| f.type_key), Some("w:CT_SectPr"));
        let in_change = select("w:sectPr", Some("w:CT_SectPrChange"), GrammarScope::Wml);
        assert_eq!(in_change.frame.as_ref().map(|f| f.type_key), Some("w:CT_SectPrBase"));
    }

    #[test]
    fn flagship_run_and_style_selection() {
        let wr = select("w:r", Some("w:CT_P"), GrammarScope::Wml);
        assert_eq!(wr.frame.as_ref().map(|f| f.type_key), Some("w:CT_R"));
        let mr = select("m:r", Some("m:CT_OMath"), GrammarScope::Wml);
        assert_eq!(mr.frame.as_ref().map(|f| f.type_key), Some("m:CT_R"));
        let style = select("w:style", Some("w:CT_Styles"), GrammarScope::Wml);
        let f = style.frame.expect("w:style is modelled");
        assert_eq!(f.type_key, "w:CT_Style");
        assert_eq!(f.model.map(|m| m.children.len()), Some(22));
    }

    #[test]
    fn flagship_ins_selection_differs_by_parent() {
        let in_p = select("w:ins", Some("w:CT_P"), GrammarScope::Wml);
        let in_numpr = select("w:ins", Some("w:CT_NumPr"), GrammarScope::Wml);
        let a = in_p.frame.as_ref().map(|f| f.type_key);
        let b = in_numpr.frame.as_ref().map(|f| f.type_key);
        assert!(a.is_some() && b.is_some() && a != b, "w:ins variants must differ by parent: {a:?} vs {b:?}");
    }

    #[test]
    fn scope_merge_conflicts_instead_of_choosing_order() {
        assert!(GrammarScope::None.merge(GrammarScope::Wml) == GrammarScope::Wml);
        assert!(GrammarScope::Wml.merge(GrammarScope::Wml) == GrammarScope::Wml);
        assert!(GrammarScope::Wml.merge(GrammarScope::Chart) == GrammarScope::Conflicting);
        assert!(GrammarScope::Chart.merge(GrammarScope::Wml) == GrammarScope::Conflicting);
        assert!(!GrammarScope::Conflicting.is_active());
        assert!(!GrammarScope::Conflicting.allows("w"));
    }

    #[test]
    fn dual_group_names_are_never_in_two_required_groups() {
        for t in TYPES.iter().chain(TRANSITIONAL_TYPES.iter()) {
            let mut groups: Vec<(&str, u16, Vec<&str>)> = Vec::new();
            for c in t.children {
                if let Some(g) = c.choice_group {
                    match groups.iter_mut().find(|(id, _, _)| *id == g) {
                        Some((_, req, members)) => {
                            *req = (*req).max(c.min);
                            members.push(c.name);
                        }
                        None => groups.push((g, c.min, vec![c.name])),
                    }
                }
            }
            for (name, _, _) in groups.iter().flat_map(|(g, req, members)| members.iter().map(move |m| (*m, g, req))) {
                let required: Vec<&str> =
                    groups.iter().filter(|(_, req, members)| *req >= 1 && members.contains(&name)).map(|(g, _, _)| *g).collect();
                assert!(required.len() <= 1, "{} places {name} in two required choice groups: {required:?}", t.key);
            }
        }
    }

    #[test]
    fn plotarea_choice_group_clause() {
        let opened = select("c:plotArea", Some("c:CT_ChartSpace"), GrammarScope::Chart);
        let mut f = opened.frame.expect("plotArea is modelled");
        f.observe("c:layout");
        let v = required_violations(f.model.unwrap(), &f);
        assert_eq!(v.len(), 1, "layout alone satisfies no chart-type member");
        f.observe("c:barChart");
        assert!(required_violations(f.model.unwrap(), &f).is_empty());
    }

    #[test]
    fn fraction_requires_num_and_den() {
        let opened = select("m:f", Some("m:CT_OMath"), GrammarScope::Wml);
        let mut f = opened.frame.expect("m:f is modelled");
        f.observe("m:num");
        let v = required_violations(f.model.unwrap(), &f);
        assert!(matches!(&v[..], [Violation::Missing("m:den", 0, 1)]));
    }

    fn judged(name: &str, parent_key: &str, child: &str) -> Vec<String> {
        let mut opened = select(name, Some(parent_key), GrammarScope::Wml);
        let frame = opened.frame.as_mut().expect("modelled frame");
        let mut acc = Acc::default();
        frame.observe(child);
        judge_child(frame, child, "word/document.xml", "w:document/w:body[1]/child[1]", false, &mut acc);
        acc.findings.iter().map(|f| f.rule.to_string()).collect()
    }

    #[test]
    fn judge_child_vetoes_transitional_only_children() {
        assert!(judged("w:tcBorders", "w:CT_TcPr", "w:left").is_empty());
        assert!(judged("w:tcBorders", "w:CT_TcPr", "w:start").is_empty());
    }

    #[test]
    fn judge_child_flags_a_child_no_profile_allows() {
        let rules = judged("w:pPr", "w:CT_P", "w:r");
        assert_eq!(rules, ["GRAMMAR-CHILD-NOT-ALLOWED"]);
    }

    #[test]
    fn judge_child_too_many_fires_once_at_the_crossing() {
        let mut opened = select("w:tbl", Some("w:CT_Body"), GrammarScope::Wml);
        let frame = opened.frame.as_mut().expect("tbl frame");
        let mut acc = Acc::default();
        for i in 1..=3 {
            frame.observe("w:tblPr");
            judge_child(frame, "w:tblPr", "word/document.xml", &format!("w:document/w:body[1]/w:tbl[1]/w:tblPr[{i}]"), false, &mut acc);
        }
        let hits: Vec<_> = acc.findings.iter().filter(|f| f.rule == "GRAMMAR-CHILD-TOO-MANY").collect();
        assert_eq!(hits.len(), 1, "{:?}", acc.findings);
        assert_eq!(hits[0].severity, Severity::Warning);
        assert!(hits[0].message.contains("w:tblPr[2]") && hits[0].message.contains("has 2 w:tblPr elements"), "{:?}", hits[0]);

        // Lower-bound count reports "has at least 2" instead of exact "has 2".
        let mut opened_lb = select("w:tbl", Some("w:CT_Body"), GrammarScope::Wml);
        let frame_lb = opened_lb.frame.as_mut().expect("tbl frame");
        let mut acc_lb = Acc::default();
        for i in 1..=2 {
            frame_lb.observe("w:tblPr");
            judge_child(frame_lb, "w:tblPr", "word/document.xml", &format!("w:document/w:body[1]/w:tbl[1]/w:tblPr[>={i}]"), true, &mut acc_lb);
        }
        let hits_lb: Vec<_> = acc_lb.findings.iter().filter(|f| f.rule == "GRAMMAR-CHILD-TOO-MANY").collect();
        assert_eq!(hits_lb.len(), 1, "{:?}", acc_lb.findings);
        assert!(hits_lb[0].message.contains("w:tblPr[>=2]") && hits_lb[0].message.contains("has at least 2 w:tblPr elements"), "{:?}", hits_lb[0]);
    }

    #[test]
    fn judge_num_fmt_domain() {
        let mut acc = Acc::default();
        for ok in ["decimal", "custom", "hebrew1", "arabicAlpha", "hindiNumbers", "thaiCounting", "bahtText"] {
            assert!(
                judge_num_fmt(
                    &GrammarAttrValue::Present(ok.to_string()),
                    "word/numbering.xml",
                    "w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]",
                    &mut acc,
                )
                .is_none()
            );
        }
        assert!(
            judge_num_fmt(&GrammarAttrValue::Missing, "word/numbering.xml", "w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]", &mut acc).is_none()
        );
        assert!(matches!(
            judge_num_fmt(&GrammarAttrValue::ValueUnreadable, "word/numbering.xml", "w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]", &mut acc),
            Some(NumFmtSkip::ValueUnreadable)
        ));
        assert!(matches!(
            judge_num_fmt(&GrammarAttrValue::IdentityUnresolved, "word/numbering.xml", "w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]", &mut acc,),
            Some(NumFmtSkip::IdentityUnresolved)
        ));
        assert!(acc.findings.is_empty(), "{:?}", acc.findings);
        assert!(
            judge_num_fmt(
                &GrammarAttrValue::Present("Decimal".to_string()),
                "word/numbering.xml",
                "w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]",
                &mut acc,
            )
            .is_none()
        );
        assert!(
            judge_num_fmt(
                &GrammarAttrValue::Present("banana".to_string()),
                "word/numbering.xml",
                "w:numbering/w:abstractNum[1]/w:lvl[2]/w:numFmt[1]",
                &mut acc,
            )
            .is_none()
        );
        let hits: Vec<_> = acc.findings.iter().filter(|f| f.rule == "GRAMMAR-NUMFMT-VALUE").collect();
        assert_eq!(hits.len(), 2, "{:?}", acc.findings);
        assert!(hits.iter().all(|f| f.severity == Severity::Warning));
    }

    #[test]
    fn judge_child_order_in_rpr_fires_once() {
        let mut opened = select("w:rPr", Some("w:CT_R"), GrammarScope::Wml);
        let frame = opened.frame.as_mut().expect("rPr frame");
        let mut acc = Acc::default();
        frame.observe("w:sz");
        judge_child(frame, "w:sz", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:sz[1]", false, &mut acc);
        frame.observe("w:rFonts");
        judge_child(frame, "w:rFonts", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:rFonts[1]", false, &mut acc);
        frame.observe("w:b");
        judge_child(frame, "w:b", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:b[1]", false, &mut acc);
        let hits: Vec<_> = acc.findings.iter().filter(|f| f.rule == "GRAMMAR-ORDER").collect();
        assert_eq!(hits.len(), 1, "{:?}", acc.findings);
        assert_eq!(hits[0].severity, Severity::Warning);
        assert!(hits[0].message.contains("w:rFonts") && hits[0].message.contains("w:rPr[1]/w:rFonts[1]"), "{:?}", hits[0]);
        let mut opened = select("w:rPr", Some("w:CT_R"), GrammarScope::Wml);
        let frame = opened.frame.as_mut().expect("rPr frame");
        let mut acc = Acc::default();
        frame.observe("w:rFonts");
        judge_child(frame, "w:rFonts", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:rFonts[1]", false, &mut acc);
        frame.observe("w:sz");
        judge_child(frame, "w:sz", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:sz[1]", false, &mut acc);
        assert!(acc.findings.is_empty(), "{:?}", acc.findings);
    }

    #[test]
    fn judge_child_fallback_uses_union_without_cardinality() {
        let opened = child_frame(&Parent::Unmodelled, Some(NS_W.as_bytes()), "sdtContent", GrammarScope::Wml);
        assert!(opened.fallback);
        let mut frame = opened.frame.expect("fallback frame");
        let mut acc = Acc::default();
        judge_child(&mut frame, "w:p", "word/document.xml", "w:document/w:body[1]/w:sdtContent[1]/w:p[1]", false, &mut acc);
        judge_child(&mut frame, "w:tc", "word/document.xml", "w:document/w:body[1]/w:sdtContent[1]/w:tc[1]", false, &mut acc);
        for _ in 0..3 {
            judge_child(&mut frame, "w:p", "word/document.xml", "w:document/w:body[1]/w:sdtContent[1]/w:p[2]", false, &mut acc);
        }
        assert!(acc.findings.is_empty(), "{:?}", acc.findings);
        judge_child(&mut frame, "w:sectPr", "word/document.xml", "w:document/w:body[1]/w:sdtContent[1]/w:sectPr[1]", false, &mut acc);
        assert_eq!(acc.findings.len(), 1, "{:?}", acc.findings);
        assert_eq!(acc.findings[0].rule, "GRAMMAR-CHILD-NOT-ALLOWED");
        assert_eq!(acc.findings[0].severity, Severity::Warning);
    }

    #[test]
    fn fallback_is_per_element_not_subtree() {
        let ins = child_frame(&Parent::Unmodelled, Some(NS_W.as_bytes()), "ins", GrammarScope::Wml);
        assert!(ins.fallback, "multi-variant w:ins under an unmodelled parent falls back");
        let tbl = child_frame(&Parent::Unmodelled, Some(NS_W.as_bytes()), "tbl", GrammarScope::Wml);
        assert!(!tbl.fallback, "single-variant w:tbl is selected regardless of parent");
        let f = tbl.frame.expect("tbl is modelled");
        assert_eq!(f.type_key(), "w:CT_Tbl");
        let v = required_violations(f.model.as_ref().expect("modelled"), &f);
        assert_eq!(v.len(), 2, "an unjudged parent does not shield the child");
    }

    #[test]
    fn cap_counts_extension_namespace_children_without_losing_modelled_children() {
        let opened = select("w:tbl", Some("w:CT_Body"), GrammarScope::Wml);
        let mut f = opened.frame.expect("w:tbl is modelled");
        for i in 0..32 {
            assert!(!f.observe(&format!("{{urn:example:x}}e{i}")), "no trip at {i}");
        }
        assert!(f.observe("{urn:example:x}e32"), "the 33rd distinct name trips");
        assert!(!f.observe("{urn:example:x}e33"), "the trip is reported once");
        assert!(f.capped());
        // Modelled children arriving after the unmodelled cap are still tracked accurately.
        assert!(!f.observe("w:tblPr"));
        assert!(!f.observe("w:tblGrid"));
        assert_eq!(f.count("w:tblPr"), 1);
        assert_eq!(f.count("w:tblGrid"), 1);
        assert!(required_violations(f.model.unwrap(), &f).is_empty());
    }

    static VETO_STRICT_CHILDREN: &[ChildSpec] = &[
        ChildSpec { name: "x:a", min: 1, max: MaxOccurs::Limited(1), compositor: Compositor::Sequence, choice_group: None, position: 0 },
        ChildSpec { name: "x:b", min: 1, max: MaxOccurs::Limited(1), compositor: Compositor::Sequence, choice_group: None, position: 1 },
    ];

    static VETO_OVER_CHILDREN: &[ChildSpec] = &[
        ChildSpec { name: "x:b", min: 1, max: MaxOccurs::Limited(1), compositor: Compositor::Sequence, choice_group: None, position: 0 },
        ChildSpec { name: "x:c", min: 1, max: MaxOccurs::Limited(1), compositor: Compositor::Sequence, choice_group: None, position: 1 },
    ];

    #[test]
    fn veto_drops_strict_only_keeps_shared_never_adds() {
        let strict = TypeModel { key: "x:CT_Veto", children: VETO_STRICT_CHILDREN };
        let over = TypeModel { key: "x:CT_Veto", children: VETO_OVER_CHILDREN };
        let opened = select("w:p", Some("w:CT_Body"), GrammarScope::Wml);
        let mut f = opened.frame.expect("w:p is modelled");
        f.observe("x:b");
        assert!(judge(&strict, Some(&over), &f).is_empty(), "strict-only violation vetoed; override-only violation not introduced");
        let opened = select("w:p", Some("w:CT_Body"), GrammarScope::Wml);
        let f = opened.frame.expect("w:p is modelled");
        let v = judge(&strict, Some(&over), &f);
        assert!(matches!(&v[..], [Violation::Missing("x:b", 0, 1)]));
        let v = judge(&strict, None, &f);
        assert_eq!(v.len(), 2);
    }

    static VETO_STRICT_CHOICE: &[ChildSpec] =
        &[ChildSpec { name: "x:a", min: 1, max: MaxOccurs::Limited(1), compositor: Compositor::Choice, choice_group: Some("choice1"), position: 0 }];

    static VETO_OVER_CHOICE: &[ChildSpec] = &[
        ChildSpec { name: "x:a", min: 1, max: MaxOccurs::Limited(1), compositor: Compositor::Choice, choice_group: Some("choice1"), position: 0 },
        ChildSpec { name: "x:b", min: 1, max: MaxOccurs::Limited(1), compositor: Compositor::Choice, choice_group: Some("choice1"), position: 1 },
    ];

    #[test]
    fn shipped_overrides_never_change_a_choice_groups_minimum() {
        fn requirements(t: &TypeModel) -> Vec<(&'static str, u16)> {
            let mut out: Vec<(&'static str, u16)> = Vec::new();
            for c in t.children {
                if let Some(g) = c.choice_group {
                    match out.iter_mut().find(|(id, _)| *id == g) {
                        Some((_, req)) => *req = (*req).max(c.min),
                        None => out.push((g, c.min)),
                    }
                }
            }
            out
        }
        for over in TRANSITIONAL_TYPES {
            let Some(strict) = type_model(over.key) else { continue };
            let strict_reqs = requirements(strict);
            for (group, req) in requirements(over) {
                if let Some((_, strict_req)) = strict_reqs.iter().find(|(g, _)| *g == group) {
                    assert_eq!(*strict_req, req, "{} choice group {group} changes minimum across profiles", over.key);
                }
            }
        }
    }

    #[test]
    fn dlbls_choice_is_satisfied_by_a_group_branch_member() {
        let opened = select("c:dLbls", Some("c:CT_Ser"), GrammarScope::Chart);
        let mut f = opened.frame.expect("c:dLbls is modelled");
        let model = f.model.expect("modelled");
        assert!(required_violations(model, &f).is_empty(), "the all-optional group branch makes the choice nullable: empty is valid");
        f.observe("c:showVal");
        assert!(required_violations(model, &f).is_empty(), "a group member satisfies the choice");
        let opened = select("c:dLbls", Some("c:CT_Ser"), GrammarScope::Chart);
        let mut f = opened.frame.expect("c:dLbls is modelled");
        f.observe("c:delete");
        assert!(required_violations(f.model.expect("modelled"), &f).is_empty(), "the bare branch satisfies it too");
    }

    #[test]
    fn veto_choice_widening_keeps_the_shared_clause() {
        let strict = TypeModel { key: "x:CT_VetoChoice", children: VETO_STRICT_CHOICE };
        let over = TypeModel { key: "x:CT_VetoChoice", children: VETO_OVER_CHOICE };
        let opened = select("w:p", Some("w:CT_Body"), GrammarScope::Wml);
        let mut f = opened.frame.expect("w:p is modelled");
        f.observe("x:b");
        assert!(judge(&strict, Some(&over), &f).is_empty());
        let opened = select("w:p", Some("w:CT_Body"), GrammarScope::Wml);
        let f = opened.frame.expect("w:p is modelled");
        let v = judge(&strict, Some(&over), &f);
        assert!(matches!(&v[..], [Violation::MissingChoice("choice1", _, 0, 1)]));
    }

    static SHORTFALL_ANY_CHILDREN: &[ChildSpec] =
        &[ChildSpec { name: "<any>", min: 2, max: MaxOccurs::Unbounded, compositor: Compositor::Sequence, choice_group: None, position: 0 }];

    static SHORTFALL_NAMED_CHILDREN: &[ChildSpec] =
        &[ChildSpec { name: "x:item", min: 3, max: MaxOccurs::Unbounded, compositor: Compositor::Sequence, choice_group: None, position: 0 }];

    static SHORTFALL_STRICT_CHOICE: &[ChildSpec] =
        &[ChildSpec { name: "x:a", min: 3, max: MaxOccurs::Unbounded, compositor: Compositor::Choice, choice_group: Some("choice1"), position: 0 }];

    static SHORTFALL_OVER_CHOICE: &[ChildSpec] = &[
        ChildSpec { name: "x:a", min: 3, max: MaxOccurs::Unbounded, compositor: Compositor::Choice, choice_group: Some("choice1"), position: 0 },
        ChildSpec { name: "x:b", min: 3, max: MaxOccurs::Unbounded, compositor: Compositor::Choice, choice_group: Some("choice1"), position: 1 },
    ];

    #[test]
    fn shortfall_messages_report_observed_count_for_named_choice_and_any() {
        let make_frame = |name: &'static str, model: &'static TypeModel, children: &[&str]| {
            let mut f = GFrame {
                name,
                type_key: model.key,
                model: Some(model),
                fallback: false,
                counts: Vec::new(),
                total: 0,
                unknown: 0,
                capped: false,
                child_set_incomplete: false,
                last_position: None,
                order_reported: false,
            };
            for c in children {
                f.observe(c);
            }
            f
        };

        // <any>: 0 vs 1 of 2
        static ANY_MODEL: TypeModel = TypeModel { key: "x:CT_Any", children: SHORTFALL_ANY_CHILDREN };
        let mut acc0 = Acc::default();
        close(&make_frame("x:any", &ANY_MODEL, &[]), "p.xml", "x:any", &mut acc0);
        assert!(acc0.findings[0].message.contains("has no child elements; x:CT_Any requires at least 2"), "{:?}", acc0.findings);
        let mut acc1 = Acc::default();
        close(&make_frame("x:any", &ANY_MODEL, &["x:one"]), "p.xml", "x:any", &mut acc1);
        assert!(acc1.findings[0].message.contains("has 1 child element (short by 1); x:CT_Any requires at least 2"), "{:?}", acc1.findings);

        // Named child: 0, 1, 2 of 3
        static NAMED_MODEL: TypeModel = TypeModel { key: "x:CT_Named", children: SHORTFALL_NAMED_CHILDREN };
        let mut acc_n1 = Acc::default();
        close(&make_frame("x:box", &NAMED_MODEL, &["x:item"]), "p.xml", "x:box", &mut acc_n1);
        assert!(acc_n1.findings[0].message.contains("has 1 x:item element (short by 2); x:CT_Named requires at least 3"), "{:?}", acc_n1.findings);
        let mut acc_n2 = Acc::default();
        close(&make_frame("x:box", &NAMED_MODEL, &["x:item", "x:item"]), "p.xml", "x:box", &mut acc_n2);
        assert!(acc_n2.findings[0].message.contains("has 2 x:item elements (short by 1); x:CT_Named requires at least 3"), "{:?}", acc_n2.findings);

        // Choice group with Transitional widening: observed count reflects the Transitional union.
        let strict = TypeModel { key: "x:CT_ChoiceShort", children: SHORTFALL_STRICT_CHOICE };
        let over = TypeModel { key: "x:CT_ChoiceShort", children: SHORTFALL_OVER_CHOICE };
        let mut f = select("w:p", Some("w:CT_Body"), GrammarScope::Wml).frame.unwrap();
        f.observe("x:a");
        f.observe("x:b");
        let v = judge(&strict, Some(&over), &f);
        assert!(matches!(&v[..], [Violation::MissingChoice("choice1", members, 2, 3)] if members.as_slice() == ["x:a", "x:b"]));
    }
}
