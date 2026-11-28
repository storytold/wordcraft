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
        let known = self.model.is_some_and(|m| {
            m.children.iter().any(|c| c.name == name) || transitional_type_model(m.key).is_some_and(|o| o.children.iter().any(|c| c.name == name))
        });
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
    /// A named child below its minimum occurrence count: `(child, min)`.
    Missing(&'static str, u16),
    /// No member of a choice group reached the group's required total: `(group, members, min)`.
    MissingChoice(&'static str, Vec<&'static str>, u16),
}

/// The required-occurrence violations of `model` against the counts a closed
/// element accumulated. Pure: shared by Strict evaluation and Transitional veto.
fn required_violations(model: &TypeModel, frame: &GFrame) -> Vec<Violation> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*5";
        scope: "static ECMA-376 required-child clauses over one closed element's direct children";
        loss: "none within a complete closed element; incomplete/fallback frames are filtered by the caller";
        fallback: "nullable choice branches never fire; returns exact missing clauses for Transitional veto";
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
        if c.name == "<any>" {
            if frame.total < u32::from(c.min) {
                out.push(Violation::Missing(c.name, c.min));
            }
        } else if frame.count(c.name) < u32::from(c.min) {
            out.push(Violation::Missing(c.name, c.min));
        }
    }
    for (group, members, min, has_seq, seq_optional, single_optional) in groups {
        if min < 1 || single_optional || (has_seq && seq_optional) {
            continue;
        }
        let sum: u32 = members.iter().map(|m| if *m == "<any>" { frame.total } else { frame.count(m) }).sum();
        if sum < u32::from(min) {
            out.push(Violation::MissingChoice(group, members, min));
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
            (Violation::Missing(a, x), Violation::Missing(b, y)) => a == b && x == y,
            (Violation::MissingChoice(a, _, x), Violation::MissingChoice(b, _, y)) => a == b && x == y,
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
        violations.retain(|v| over_violations.iter().any(|o| v.same_clause(o)));
    }
    violations
}

/// Judge a closed element's required children (`GRAMMAR-REQUIRED-CHILD`).
/// Runs at element close (prefix-established for the closed element). Fallback
/// frames and frames with skipped `mc:AlternateContent` or unresolved-namespace
/// direct children are skipped so required-child absence is never inferred from
/// an incomplete child set.
pub(crate) fn close(frame: &GFrame, part: &str, path: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "GRAMMAR-REQUIRED-CHILD";
        collections: "-";
        scope: "closed modelled element's direct children in a grammar-scoped XML part";
        loss: "fallback selection, skipped mc:AlternateContent child, or unresolved-namespace direct child";
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
            Violation::Missing(child, min) => {
                let need = if *min == 1 { "at least one".to_string() } else { format!("at least {min}") };
                if *child == "<any>" {
                    format!("{} at {path} in {part} has no child elements; {} requires {need}", frame.name, model.key)
                } else {
                    format!("{} at {path} in {part} has no {child} element; {} requires {need}", frame.name, model.key)
                }
            }
            Violation::MissingChoice(_, members, min) => {
                let need = if *min == 1 { "at least one of them".to_string() } else { format!("at least {min} of them") };
                format!("{} at {path} in {part} has none of {}; {} requires {need}", frame.name, members.join(", "), model.key)
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
/// `GRAMMAR-CHILD-TOO-MANY`, and `GRAMMAR-ORDER` inside `w:rPr`).
pub(crate) fn judge_child(frame: &mut GFrame, child: &str, part: &str, child_path: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "GRAMMAR-CHILD-NOT-ALLOWED,GRAMMAR-CHILD-TOO-MANY";
        collections: "-";
        scope: "direct modelled child of an open modelled element in a grammar-scoped XML part";
        loss: "fallback parents have no single model; Transitional overrides veto Strict-only offences";
        fallback: "fallback parents check union membership only and skip cardinality/order; emits Warning only";
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
                acc.add_grammar(
                    "GRAMMAR-CHILD-TOO-MANY",
                    Some(part.to_string()),
                    format!("{} at {child_path} in {part} has {n} {child} elements; {} allows at most {max}", frame.name, model.key),
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

/// Judge one `w:numFmt` format value (`GRAMMAR-NUMFMT-VALUE`).
pub(crate) fn judge_num_fmt(val: Option<&str>, part: &str, path: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "GRAMMAR-NUMFMT-VALUE";
        collections: "-";
        scope: "w:numFmt/@w:val in Wml-scope parts against the 63-value ST_NumberFormat table";
        loss: "missing w:val attribute carries no value to judge";
        fallback: "exact case-sensitive membership check; missing attribute stays silent; Warning only";
        package_loss: "independent";
    }

    if let Some(v) = val
        && !NUM_FMT_VALUES.contains(&v)
    {
        acc.add_grammar(
            "GRAMMAR-NUMFMT-VALUE",
            Some(part.to_string()),
            format!("numFmt format \"{v}\" at {path} in {part} is not an ST_NumberFormat value"),
        );
    }
}

/// Inputs describing one XML start or empty element for [`GrammarWalk`].
pub(crate) struct ElementEvent<'a> {
    pub ns: Option<&'a [u8]>,
    pub local: &'a str,
    pub is_alt: bool,
    pub unresolved: bool,
    pub suppressed: bool,
    pub numfmt_val: Option<&'a str>,
}

/// One open XML element tracked by [`GrammarWalk`].
struct WalkFrame {
    path: String,
    child_counts: HashMap<String, u32>,
    grammar: Option<GFrame>,
    suppressed: bool,
}

/// Per-part streaming grammar walk, inactive (zero work) when the part has no
/// active [`GrammarScope`].
pub(crate) struct GrammarWalk {
    scope: GrammarScope,
    frames: Vec<WalkFrame>,
    fallback_noted: HashSet<String>,
    cap_noted: bool,
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

        Self { scope, frames: Vec::new(), fallback_noted: HashSet::new(), cap_noted: false }
    }

    /// Compute the 1-based occurrence path for an element about to open under
    /// the current top frame.
    fn next_path(&mut self, ns: Option<&[u8]>, local: &str) -> String {
        observation_contract! {
            rules: "-";
            collections: "field.child_counts*3,field.frames*1";
            scope: "1-based sibling occurrence path from the part root for grammar findings";
            loss: "distinct sibling names capped at 1024 per parent frame";
            fallback: "elements beyond the distinct-name cap keep ordinal 1 without growing memory";
            package_loss: "independent";
        }

        let display = count_key(ns, local);
        match self.frames.last_mut() {
            None => display,
            Some(parent) => {
                let ordinal = if let Some(c) = parent.child_counts.get_mut(&display) {
                    *c = c.saturating_add(1);
                    *c
                } else if parent.child_counts.len() < PATH_ORDINAL_CAP {
                    parent.child_counts.insert(display.clone(), 1);
                    1
                } else {
                    1
                };
                format!("{}/{display}[{ordinal}]", parent.path)
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
            loss: "suppressed mc:AlternateContent subtrees, unresolved namespaces, or >32 unmodelled child names";
            fallback: "records informational coverage notes only, never calls set_incomplete(), emits Warning only";
            package_loss: "independent";
        }

        if ev.suppressed {
            return None;
        }
        if self.scope == GrammarScope::Wml && ev.ns == Some(NS_W.as_bytes()) && ev.local == "numFmt" {
            judge_num_fmt(ev.numfmt_val, part, path, acc);
        }
        let child = count_key(ev.ns, ev.local);
        if let Some(parent) = self.frames.last_mut()
            && let Some(pg) = &mut parent.grammar
        {
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
                judge_child(pg, &display, part, path, acc);
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
            collections: "field.frames*1,type.HashMap*1";
            scope: "push one XML start-element frame onto the per-part grammar stack";
            loss: "inactive or conflicting scope is a no-op";
            fallback: "suppressed subtrees keep stack balance without emitting grammar findings";
            package_loss: "independent";
        }

        if !self.scope.is_active() {
            return;
        }
        let path = self.next_path(ev.ns, ev.local);
        let grammar = self.enter_element(ev, part, &path, acc);
        self.frames.push(WalkFrame { path, child_counts: HashMap::new(), grammar, suppressed: ev.suppressed });
    }

    /// Process an empty tag (`Event::Empty`): opens and closes immediately.
    pub(crate) fn empty(&mut self, ev: &ElementEvent<'_>, part: &str, acc: &mut Acc) {
        if !self.scope.is_active() {
            return;
        }
        let path = self.next_path(ev.ns, ev.local);
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
        assert!(matches!(&v[..], [Violation::Missing("m:den", 1)]));
    }

    fn judged(name: &str, parent_key: &str, child: &str) -> Vec<String> {
        let mut opened = select(name, Some(parent_key), GrammarScope::Wml);
        let frame = opened.frame.as_mut().expect("modelled frame");
        let mut acc = Acc::default();
        frame.observe(child);
        judge_child(frame, child, "word/document.xml", "w:document/w:body[1]/child[1]", &mut acc);
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
            judge_child(frame, "w:tblPr", "word/document.xml", &format!("w:document/w:body[1]/w:tbl[1]/w:tblPr[{i}]"), &mut acc);
        }
        let hits: Vec<_> = acc.findings.iter().filter(|f| f.rule == "GRAMMAR-CHILD-TOO-MANY").collect();
        assert_eq!(hits.len(), 1, "{:?}", acc.findings);
        assert_eq!(hits[0].severity, Severity::Warning);
        assert!(hits[0].message.contains("w:tblPr[2]"), "{:?}", hits[0]);
    }

    #[test]
    fn judge_num_fmt_domain() {
        let mut acc = Acc::default();
        for ok in ["decimal", "custom", "hebrew1", "arabicAlpha", "hindiNumbers", "thaiCounting", "bahtText"] {
            judge_num_fmt(Some(ok), "word/numbering.xml", "w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]", &mut acc);
        }
        judge_num_fmt(None, "word/numbering.xml", "w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]", &mut acc);
        assert!(acc.findings.is_empty(), "{:?}", acc.findings);
        judge_num_fmt(Some("Decimal"), "word/numbering.xml", "w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]", &mut acc);
        judge_num_fmt(Some("banana"), "word/numbering.xml", "w:numbering/w:abstractNum[1]/w:lvl[2]/w:numFmt[1]", &mut acc);
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
        judge_child(frame, "w:sz", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:sz[1]", &mut acc);
        frame.observe("w:rFonts");
        judge_child(frame, "w:rFonts", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:rFonts[1]", &mut acc);
        frame.observe("w:b");
        judge_child(frame, "w:b", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:b[1]", &mut acc);
        let hits: Vec<_> = acc.findings.iter().filter(|f| f.rule == "GRAMMAR-ORDER").collect();
        assert_eq!(hits.len(), 1, "{:?}", acc.findings);
        assert_eq!(hits[0].severity, Severity::Warning);
        assert!(hits[0].message.contains("w:rFonts") && hits[0].message.contains("w:rPr[1]/w:rFonts[1]"), "{:?}", hits[0]);
        let mut opened = select("w:rPr", Some("w:CT_R"), GrammarScope::Wml);
        let frame = opened.frame.as_mut().expect("rPr frame");
        let mut acc = Acc::default();
        frame.observe("w:rFonts");
        judge_child(frame, "w:rFonts", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:rFonts[1]", &mut acc);
        frame.observe("w:sz");
        judge_child(frame, "w:sz", "word/document.xml", "w:document/w:body[1]/w:p[1]/w:r[1]/w:rPr[1]/w:sz[1]", &mut acc);
        assert!(acc.findings.is_empty(), "{:?}", acc.findings);
    }

    #[test]
    fn judge_child_fallback_uses_union_without_cardinality() {
        let opened = child_frame(&Parent::Unmodelled, Some(NS_W.as_bytes()), "sdtContent", GrammarScope::Wml);
        assert!(opened.fallback);
        let mut frame = opened.frame.expect("fallback frame");
        let mut acc = Acc::default();
        judge_child(&mut frame, "w:p", "word/document.xml", "w:document/w:body[1]/w:sdtContent[1]/w:p[1]", &mut acc);
        judge_child(&mut frame, "w:tc", "word/document.xml", "w:document/w:body[1]/w:sdtContent[1]/w:tc[1]", &mut acc);
        for _ in 0..3 {
            judge_child(&mut frame, "w:p", "word/document.xml", "w:document/w:body[1]/w:sdtContent[1]/w:p[2]", &mut acc);
        }
        assert!(acc.findings.is_empty(), "{:?}", acc.findings);
        judge_child(&mut frame, "w:sectPr", "word/document.xml", "w:document/w:body[1]/w:sdtContent[1]/w:sectPr[1]", &mut acc);
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
        assert!(matches!(&v[..], [Violation::Missing("x:b", 1)]));
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
        assert!(matches!(&v[..], [Violation::MissingChoice("choice1", _, 1)]));
    }
}
