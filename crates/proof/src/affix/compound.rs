//! Hunspell compounding — the subset the OpenTaal Dutch dictionary (`nl_NL`) uses.
//!
//! - **`COMPOUNDRULE`**: a pattern of flags, one per part (`(N4)(NH)` → `vijf` + `honderd`), each
//!   optionally repeated (`*`) or optional (`?`). Parts are stems, without affixes.
//! - **`COMPOUNDBEGIN`/`COMPOUNDMIDDLE`/`COMPOUNDEND`**: a word made of two or more parts, the
//!   first carrying the begin flag, the last the end flag and any between the middle flag
//!   (`zwembad` + `bouwer`). A part may be a stem or a stem with one affix; the flag may come from
//!   the stem or from the affix's continuation classes (`SFX Ch 0 s/CaCbCxCp`: `trainings-`). An
//!   affix inside the compound (a suffix before the last part, a prefix after the first) needs
//!   `COMPOUNDPERMITFLAG` among its continuation classes. `ONLYINCOMPOUND` stems and forms are
//!   valid here only.
//! - **Checks on each boundary**: `CHECKCOMPOUNDDUP` (no part twice in a row),
//!   `CHECKCOMPOUNDCASE` (no capital beside a boundary, except at a hyphen) and
//!   `CHECKCOMPOUNDPATTERN` (an ending, optionally with its root's flag, followed by a beginning,
//!   optionally with its root's flag: `a /C1` keeps `zee` + `egel` apart, which Dutch hyphenates).
//!
//! Every part needs at least `COMPOUNDMIN` characters. A word is split into at most [`MAX_PARTS`]
//! parts and analysed within a budget of part lookups ([`CHECK_BUDGET`] when checking a word,
//! the much smaller [`SUGGEST_BUDGET`] for each of a suggestion's many candidates), so hostile
//! input stays cheap.

use super::{AffixDict, MAX_WORD_BYTES};

/// Most parts in one compound.
const MAX_PARTS: usize = 8;
/// Part analyses tried for one word before it is given up as not a compound.
pub(super) const CHECK_BUDGET: u32 = 4_000;
/// The same for a suggestion candidate: enough for the two or three parts of an ordinary
/// compound, small enough that the thousands of candidates a suggestion tries stay fast.
pub(super) const SUGGEST_BUDGET: u32 = 200;

/// How often a `COMPOUNDRULE` flag may occur in a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Quant {
    /// Exactly once.
    One,
    /// Zero times or once (`?`).
    Opt,
    /// Zero or more times (`*`).
    Star,
}

/// A `CHECKCOMPOUNDPATTERN`: a part ending in `end` (whose root carries `end_flag`, when set)
/// may not be followed by one starting with `begin` (whose root carries `begin_flag`).
#[derive(Clone, Debug)]
pub(super) struct Pattern {
    pub end: Box<str>,
    pub end_flag: u128,
    pub begin: Box<str>,
    pub begin_flag: u128,
}

/// Where a part sits in a compound.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Begin,
    Middle,
    End,
}

/// The flags a word matching `rule` may start with: the first element's, and the next ones' as
/// long as the elements before them are optional.
fn first_flags(rule: &[(u128, Quant)]) -> u128 {
    let mut m = 0;
    for &(bit, q) in rule {
        m |= bit;
        if q == Quant::One {
            break;
        }
    }
    m
}

/// The part before a boundary: its text and its root's flags.
#[derive(Clone, Copy)]
struct Prev<'a> {
    text: &'a str,
    root: u128,
}

impl AffixDict {
    /// Does the dictionary compound at all?
    pub(super) fn compounds(&self) -> bool {
        !self.compound_rules.is_empty() || self.cpd_begin | self.cpd_middle | self.cpd_end != 0
    }

    /// Is `w` (already input-converted, in its own case) a compound of dictionary words?
    pub(super) fn compound(&self, w: &str, excluded: u128, budget: u32) -> bool {
        if !self.compounds() || w.len() > MAX_WORD_BYTES || w.chars().nth(1).is_none() {
            return false;
        }
        // Part boundaries: every char boundary, the word's end included.
        let bounds: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        let mut budget = budget;
        if self.cpd_begin != 0 && self.cpd_end != 0 && self.parts(w, &bounds, 0, 0, None, excluded, &mut budget) {
            return true;
        }
        // A rule's first part is a stem carrying one of the flags it may start with (Dutch rules
        // are numerals, street names…): look the word's beginnings up once before trying them all.
        let first = self.compound_rules.iter().fold(0u128, |m, rule| m | first_flags(rule));
        let starts = bounds
            .iter()
            .skip(1)
            .filter_map(|&to| w.get(..to))
            .any(|p| self.long_enough(p) && self.stem_flags_excluding(p, excluded).is_some_and(|f| f & first != 0));
        starts && self.compound_rules.iter().any(|rule| self.rule_matches(w, &bounds, rule, excluded, &mut budget))
    }

    /// Is the stem `part` long enough to be a compound part?
    fn long_enough(&self, part: &str) -> bool {
        part.chars().count() >= self.compound_min
    }

    /// Does `w` split into stems carrying the rule's flags, in order? A backtracking match over
    /// the char boundaries, with failed `(boundary, element)` states remembered.
    fn rule_matches(&self, w: &str, bounds: &[usize], rule: &[(u128, Quant)], excluded: u128, budget: &mut u32) -> bool {
        let width = rule.len() + 1;
        let mut failed = vec![false; bounds.len().saturating_mul(width)];
        self.rule_from(w, bounds, rule, 0, 0, 0, excluded, &mut failed, budget)
    }

    #[allow(clippy::too_many_arguments)]
    fn rule_from(
        &self,
        w: &str,
        bounds: &[usize],
        rule: &[(u128, Quant)],
        b: usize,
        e: usize,
        parts: usize,
        excluded: u128,
        failed: &mut [bool],
        budget: &mut u32,
    ) -> bool {
        let Some(&at) = bounds.get(b) else { return false };
        let Some(&(bit, quant)) = rule.get(e) else {
            return at == w.len() && parts >= 2;
        };
        let state = b * (rule.len() + 1) + e;
        if failed.get(state).copied().unwrap_or(true) || *budget == 0 || parts >= MAX_PARTS {
            return false;
        }
        if quant != Quant::One && self.rule_from(w, bounds, rule, b, e + 1, parts, excluded, failed, budget) {
            return true;
        }
        for nb in b + 1..bounds.len() {
            let Some(part) = bounds.get(nb).and_then(|&to| w.get(at..to)) else { continue };
            if !self.long_enough(part) {
                continue;
            }
            *budget = budget.saturating_sub(1);
            if !self.stem_flags_excluding(part, excluded).is_some_and(|f| f & bit != 0) {
                continue;
            }
            // `*` may take this flag again; every quantifier may move on to the next element.
            if quant == Quant::Star && self.rule_from(w, bounds, rule, nb, e, parts + 1, excluded, failed, budget) {
                return true;
            }
            if self.rule_from(w, bounds, rule, nb, e + 1, parts + 1, excluded, failed, budget) {
                return true;
            }
        }
        if let Some(f) = failed.get_mut(state) {
            *f = true;
        }
        false
    }

    /// Can `w[bounds[b]..]` be read as the parts after `prev` (the `index`th part onwards)?
    #[allow(clippy::too_many_arguments)]
    fn parts(&self, w: &str, bounds: &[usize], b: usize, index: usize, prev: Option<Prev>, excluded: u128, budget: &mut u32) -> bool {
        let Some(&at) = bounds.get(b) else { return false };
        if index >= MAX_PARTS {
            return false;
        }
        for nb in b + 1..bounds.len() {
            if *budget == 0 {
                return false;
            }
            let Some(&to) = bounds.get(nb) else { continue };
            let Some(part) = w.get(at..to) else { continue };
            let last = to == w.len();
            if (index == 0 && last) || !self.long_enough(part) {
                continue;
            }
            let place = if index == 0 {
                Place::Begin
            } else if last {
                Place::End
            } else {
                Place::Middle
            };
            *budget = budget.saturating_sub(1);
            for root in self.part_roots(part, place, excluded) {
                if let Some(p) = prev
                    && !self.boundary_ok(p, part, root)
                {
                    continue;
                }
                if last || self.parts(w, bounds, nb, index + 1, Some(Prev { text: part, root }), excluded, budget) {
                    return true;
                }
            }
        }
        false
    }

    /// May `part` (whose root has flags `root`) follow `prev`?
    fn boundary_ok(&self, prev: Prev, part: &str, root: u128) -> bool {
        if self.check_dup && prev.text == part {
            return false;
        }
        if self.check_case
            && let (Some(a), Some(b)) = (prev.text.chars().next_back(), part.chars().next())
            && (a.is_uppercase() || b.is_uppercase())
            && a != '-'
            && b != '-'
        {
            return false;
        }
        !self.patterns.iter().any(|p| {
            prev.text.ends_with(&*p.end)
                && (p.end_flag == 0 || prev.root & p.end_flag != 0)
                && part.starts_with(&*p.begin)
                && (p.begin_flag == 0 || root & p.begin_flag != 0)
        })
    }

    /// The flags of every root `part` can be read from at `place`: the part as a stem, or a stem
    /// with one suffix or one prefix, carrying the place's compound flag.
    fn part_roots(&self, part: &str, place: Place, excluded: u128) -> Vec<u128> {
        let need = match place {
            Place::Begin => self.cpd_begin,
            Place::Middle => self.cpd_middle,
            Place::End => self.cpd_end,
        };
        let mut out: Vec<u128> = Vec::new();
        if need == 0 {
            return out;
        }
        if let Some(f) = self.stem_flags_excluding(part, excluded)
            && f & need != 0
        {
            out.push(f);
        }
        // Suffixes: inside the compound (before the last part) only with COMPOUNDPERMITFLAG.
        let mut root = String::with_capacity(part.len() + 8);
        let cuts = std::iter::once(part.len()).chain(part.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (part.get(..b), part.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                let Some(f) = self.stem_flags_excluding(&root, excluded) else { continue };
                for r in g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                    let permitted = place == Place::End || r.cont & self.cpd_permit != 0;
                    if permitted && f & r.bit != 0 && (f | r.cont) & need != 0 && r.cond_matches(&root, true) && !out.contains(&f) {
                        out.push(f);
                    }
                }
            }
        }
        // Prefixes: after the first part only with COMPOUNDPERMITFLAG.
        for r in &self.prefixes {
            if place != Place::Begin && r.cont & self.cpd_permit == 0 {
                continue;
            }
            let Some(rest) = part.strip_prefix(&*r.add) else { continue };
            if rest.is_empty() {
                continue;
            }
            root.clear();
            root.push_str(&r.strip);
            root.push_str(rest);
            if let Some(f) = self.stem_flags_excluding(&root, excluded)
                && f & r.bit != 0
                && (f | r.cont) & need != 0
                && r.cond_matches(&root, false)
                && !out.contains(&f)
            {
                out.push(f);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::AffixDict;

    /// Hand-written synthetic fixture in the shape of a compounding dictionary (two-character
    /// flags, a linking `s`, a number rule); no words or rules come from a real dictionary.
    const AFF: &str = "SET UTF-8
FLAG long
FORBIDDENWORD Fw
ONLYINCOMPOUND Cx
COMPOUNDMIN 1
COMPOUNDRULE 2
COMPOUNDRULE (N1)(NH)
COMPOUNDRULE (N1)*(Nd)
CHECKCOMPOUNDCASE
COMPOUNDBEGIN Ca
COMPOUNDMIDDLE Cb
COMPOUNDEND Cc
COMPOUNDPERMITFLAG Cp
CHECKCOMPOUNDDUP
CHECKCOMPOUNDPATTERN 2
CHECKCOMPOUNDPATTERN e /V1
CHECKCOMPOUNDPATTERN /Ls /Xs	# a comment
SFX CA Y 2
SFX CA 0 /CaCp .
SFX CA 0 -/CaCp .
SFX Ls Y 1
SFX Ls 0 s/CaCbCxCp .
SFX En Y 1
SFX En 0 en .
";

    const DIC: &str = "15
bad/CACbCcEn
zwem/CACb
bouwer/CcEn
huis/CcCbCA
water/CACbCcEn
oefen/Ls
plaats/CcXs
zaal/Cc
zee/CA
egel/CcV1
Delft/CACc
vijf/N1
honderd/NH
duizend/Nd
waterbad/Fw
";

    fn dict() -> AffixDict {
        AffixDict::from_hunspell(AFF, DIC, &[]).unwrap()
    }

    #[test]
    fn begin_middle_end_parts_compound() {
        let d = dict();
        for w in ["zwembad", "zwembadbouwer", "Zwembadbouwer", "ZWEMBADBOUWER", "waterhuis", "huiswater", "zwem-bad", "Delftwater"] {
            assert!(d.check(w), "{w}");
        }
        assert!(d.check("zwembadbouweren"), "the last part may carry a suffix");
        for w in ["bouwerzwem", "badbad", "zwemzwem", "bouwer-bad", "zwemBad", "zwembadbouwers"] {
            assert!(!d.check(w), "{w}");
        }
    }

    #[test]
    fn suffixes_inside_need_the_permit_flag() {
        // `bad` + `en` + `huis`: En has no COMPOUNDPERMITFLAG, so a suffixed first part can't begin.
        let d = dict();
        assert!(d.check("baden") && d.check("badhuis"));
        assert!(!d.check("badenhuis"));
    }

    #[test]
    fn only_in_compound_forms_need_a_compound() {
        let d = dict();
        assert!(!d.check("oefens"), "the linking s exists only inside compounds");
        assert!(d.check("oefenszaal"));
        assert!(!d.contains("oefens") && !d.forms("oefen").iter().any(|f| f == "oefens"));
    }

    #[test]
    fn case_dup_and_patterns_guard_boundaries() {
        let d = dict();
        // CHECKCOMPOUNDCASE: no capital beside a boundary, unless a hyphen separates them.
        assert!(!d.check("waterDelft"));
        assert!(d.check("water-Delft"));
        // `e /V1`: `zee` + `egel` collides two vowels; Dutch writes `zee-egel`.
        assert!(!d.check("zeeegel"));
        assert!(d.check("zee-egel"));
        // `/Ls /Xs`: a root with Ls before a root with Xs (`oefens` + `plaats`).
        assert!(!d.check("oefensplaats"));
        let without = AffixDict::from_hunspell(&AFF.replace("CHECKCOMPOUNDPATTERN /Ls /Xs\t# a comment\n", ""), DIC, &[]).unwrap();
        assert!(without.check("oefensplaats"));
    }

    #[test]
    fn forbidden_words_stay_wrong_as_compounds() {
        let d = dict();
        assert!(!d.check("waterbad") && !d.check("Waterbad"));
        assert!(d.check("waterhuis"));
    }

    #[test]
    fn compound_rules_and_their_quantifiers() {
        let d = dict();
        for w in ["vijfhonderd", "vijfduizend", "vijfvijfduizend", "duizend", "Vijfhonderd"] {
            assert!(d.check(w), "{w}");
        }
        for w in ["honderdvijf", "vijfhonderdduizend", "vijf-honderd"] {
            assert!(!d.check(w), "{w}");
        }
        assert!(d.contains_suggestible("vijfhonderd") && d.can_suggest("zwembadbouwer"), "short compounds may be suggested");
    }

    #[test]
    fn compound_data_survives_the_compact_format() {
        let d = dict();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for w in ["zwembadbouwer", "vijfhonderd", "oefenszaal", "zee-egel", "water-Delft"] {
            assert!(back.check(w), "{w}");
        }
        for w in ["zeeegel", "oefens", "waterbad", "badbad", "waterDelft", "oefensplaats", "badenhuis"] {
            assert!(!back.check(w), "{w}");
        }
    }

    #[test]
    fn compounding_limits_this_engine_lacks_are_an_error() {
        assert!(AffixDict::from_hunspell(&format!("{AFF}COMPOUNDWORDMAX 2\n"), DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("COMPOUNDWORDMAX 2\n", "1\nwoord\n", &[]).is_ok(), "harmless without compounding");
    }

    #[test]
    fn long_inputs_stay_within_the_budget() {
        let d = dict();
        let long = "zwem".repeat(30);
        assert!(!d.check(&long));
        let many = "bad".repeat(40);
        assert!(!d.check(&many));
    }

    proptest::proptest! {
        #[test]
        fn compound_lookups_never_panic(s in "[a-zA-Z -]{0,60}") {
            let d = dict();
            let _ = d.check(&s);
            let _ = d.contains(&s);
        }
    }
}
