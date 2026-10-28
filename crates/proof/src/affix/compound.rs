//! Hunspell compounding — the subset the OpenTaal Dutch dictionary (`nl_NL`) uses.
//!
//! - **`COMPOUNDRULE`**: a pattern of flags, one per part (`(N4)(NH)` → `vijf` + `honderd`), each
//!   optionally repeated (`*`) or optional (`?`). Parts are stems; the final part may carry one suffix.
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

use super::{AffixDict, MAX_WORD_BYTES, Stem};

/// Most parts in one compound.
const MAX_PARTS: usize = 8;
/// Part analyses and rule transitions tried before a word is given up as not a compound.
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
pub(super) fn first_flags(rule: &[(u128, Quant)]) -> u128 {
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
    root: Stem,
}

impl AffixDict {
    /// Does the dictionary compound at all?
    pub(super) fn compounds(&self) -> bool {
        !self.compound_rules.is_empty() || self.cpd_begin | self.cpd_middle | self.cpd_end != 0
    }

    /// Is `w` (already input-converted, in its own case) a compound of dictionary words?
    pub(super) fn compound(&self, w: &str, excluded: u128, budget: u32, capitalized: bool, head_excluded: u128) -> bool {
        if !self.compounds() || w.len() > MAX_WORD_BYTES || w.chars().nth(1).is_none() {
            return false;
        }
        // Part boundaries: every char boundary, the word's end included.
        let bounds: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        let mut budget = budget;
        let end_excluded = excluded | if capitalized { 0 } else { self.force_ucase };
        // Separate head, middle and final restrictions. Capitalization folds only the head's
        // KEEPCASE check; FORCEUCASE still depends on the whole token's original capital.
        let excluded = [excluded | head_excluded, excluded, end_excluded];
        if self.cpd_begin != 0 && self.cpd_end != 0 && self.parts(w, &bounds, 0, 0, None, excluded, &mut budget) {
            return true;
        }
        // A rule's first part is a stem carrying one of the flags it may start with (Dutch rules
        // are numerals, street names…): look the word's beginnings up once before trying them all.
        let first = self.compound_first;
        let starts = bounds
            .iter()
            .skip(1)
            .filter_map(|&to| w.get(..to))
            .any(|p| self.long_enough(p) && self.stems_excluding(p, excluded[0]).any(|s| s.flags & first != 0));
        starts && self.compound_rules.iter().any(|rule| self.rule_matches(w, &bounds, rule, excluded, &mut budget))
    }

    /// Is the stem `part` long enough to be a compound part?
    fn long_enough(&self, part: &str) -> bool {
        part.chars().count() >= self.compound_min
    }

    /// Backtracking over part boundaries and rule elements. Optional transitions use
    /// the same finite budget as part lookups and an explicit work stack, so a long
    /// accepted pattern cannot grow the call stack or allocate a word-by-rule matrix.
    fn rule_matches(&self, w: &str, bounds: &[usize], rule: &[(u128, Quant)], excluded: [u128; 3], budget: &mut u32) -> bool {
        let mut pending = vec![(0usize, 0usize, 0usize)];
        let mut seen = std::collections::HashSet::new();
        while let Some((b, e, parts)) = pending.pop() {
            let Some(&at) = bounds.get(b) else { continue };
            let Some(&(bit, quant)) = rule.get(e) else {
                if at == w.len() && parts >= 2 {
                    return true;
                }
                continue;
            };
            if *budget == 0 {
                return false;
            }
            *budget = budget.saturating_sub(1);
            // The part count belongs in the state: it controls both the minimum of
            // two and the maximum of eight, even at the same boundary and element.
            if !seen.insert((b, e, parts)) {
                continue;
            }
            if parts < MAX_PARTS {
                // Reverse push order keeps the shortest next component first.
                for nb in (b + 1..bounds.len()).rev() {
                    if *budget == 0 {
                        return false;
                    }
                    let Some(part) = bounds.get(nb).and_then(|&to| w.get(at..to)) else { continue };
                    if !self.long_enough(part) {
                        continue;
                    }
                    *budget = budget.saturating_sub(1);
                    let last = nb + 1 == bounds.len();
                    let part_excluded = if parts == 0 {
                        excluded[0]
                    } else if last {
                        excluded[2]
                    } else {
                        excluded[1]
                    };
                    if self.rule_part_matches(part, bit, last, part_excluded) {
                        pending.push((nb, e + 1, parts + 1));
                        if quant == Quant::Star {
                            pending.push((nb, e, parts + 1));
                        }
                    }
                }
            }
            // Skips run first, but never recurse or bypass the budget. At eight
            // consumed parts they can still reach a rule's optional tail.
            if quant != Quant::One {
                pending.push((b, e + 1, parts));
            }
        }
        false
    }

    /// Rule compounds allow a suffix on their final part, independently of positional flags.
    fn rule_part_matches(&self, part: &str, bit: u128, last: bool, excluded: u128) -> bool {
        // Hunspell 1.7.2 permits an independent allowed suffix derivation here even when the
        // surface spelling has a forbidden standalone entry. Root/continuation flags still apply.
        if self.stems_excluding(part, excluded).any(|s| s.flags & bit != 0) {
            return true;
        }
        if !last {
            return false;
        }
        let mut root = String::with_capacity(part.len() + 8);
        let cuts = std::iter::once(part.len()).chain(part.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (part.get(..b), part.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                if self.stems_excluding(&root, excluded).any(|s| {
                    s.flags & bit != 0
                        && g.rules
                            .iter()
                            .filter_map(|&i| self.suffixes.get(i as usize))
                            .any(|r| s.flags & r.bit != 0 && (s.flags | r.cont) & excluded == 0 && r.cond_matches(&root, true))
                }) {
                    return true;
                }
            }
        }
        false
    }

    /// Can `w[bounds[b]..]` be read as the parts after `prev` (the `index`th part onwards)?
    #[allow(clippy::too_many_arguments)]
    fn parts(&self, w: &str, bounds: &[usize], b: usize, index: usize, prev: Option<Prev>, excluded: [u128; 3], budget: &mut u32) -> bool {
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
            let part_excluded = if index == 0 {
                excluded[0]
            } else if last {
                excluded[2]
            } else {
                excluded[1]
            };
            for root in self.part_roots(part, place, part_excluded) {
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
    fn boundary_ok(&self, prev: Prev, part: &str, root: Stem) -> bool {
        if self.check_dup && prev.root.index == root.index {
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
                && (p.end_flag == 0 || prev.root.flags & p.end_flag != 0)
                && part.starts_with(&*p.begin)
                && (p.begin_flag == 0 || root.flags & p.begin_flag != 0)
        })
    }

    /// The flags of every root `part` can be read from at `place`: the part as a stem, or a stem
    /// with one suffix or one prefix, carrying the place's compound flag.
    fn part_roots(&self, part: &str, place: Place, excluded: u128) -> Vec<Stem> {
        let need = match place {
            Place::Begin => self.cpd_begin,
            Place::Middle => self.cpd_middle,
            Place::End => self.cpd_end,
        };
        let mut out: Vec<Stem> = Vec::new();
        if need == 0 || self.excludes_spelling(part, excluded) {
            return out;
        }
        out.extend(self.stems_excluding(part, excluded).filter(|s| s.flags & need != 0));
        let mut root = String::with_capacity(part.len() + 8);
        let cuts = std::iter::once(part.len()).chain(part.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (part.get(..b), part.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                for stem in self.stems_excluding(&root, excluded) {
                    for r in g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                        let permitted = place == Place::End || r.cont & self.cpd_permit != 0;
                        if permitted
                            && stem.flags & r.bit != 0
                            && (stem.flags | r.cont) & need != 0
                            && (stem.flags | r.cont) & excluded == 0
                            && r.cond_matches(&root, true)
                            && !out.contains(&stem)
                        {
                            out.push(stem);
                        }
                    }
                }
            }
        }
        for r in &self.prefixes {
            if place != Place::Begin && r.cont & self.cpd_permit == 0 {
                continue;
            }
            let Some(rest) = part.strip_prefix(&*r.add) else { continue };
            if rest.is_empty() && !self.full_strip {
                continue;
            }
            root.clear();
            root.push_str(&r.strip);
            root.push_str(rest);
            for stem in self.stems_excluding(&root, excluded) {
                if stem.flags & r.bit != 0
                    && (stem.flags | r.cont) & need != 0
                    && (stem.flags | r.cont) & excluded == 0
                    && r.cond_matches(&root, false)
                    && !out.contains(&stem)
                {
                    out.push(stem);
                }
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
    fn positional_parts_preserve_explicit_prohibited_spellings() {
        let base = "SET UTF-8\nFORBIDDENWORD F\nNOSUGGEST N\nCOMPOUNDMIN 2\nCOMPOUNDBEGIN B\nCOMPOUNDMIDDLE M\nCOMPOUNDEND E\nCOMPOUNDPERMITFLAG P\nSFX S N 1\nSFX S 0 s/P .\nPFX R N 1\nPFX R 0 pre/P .\n";
        for flag in ['F', 'N'] {
            for (dic, word, control) in [
                (format!("3\nroot/BS\nroots/B{flag}\nend/E\n"), "rootsend", "rootend"),
                (format!("3\nfirst/B\nroot/ES\nroots/E{flag}\n"), "firstroots", "firstroot"),
                (format!("4\nfirst/B\nroot/MS\nroots/M{flag}\nend/E\n"), "firstrootsend", "firstrootend"),
                (format!("3\nroot/BR\npreroot/B{flag}\nend/E\n"), "prerootend", "rootend"),
            ] {
                let d = AffixDict::from_hunspell(base, &dic, &[]).unwrap();
                let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
                for d in [&d, &back] {
                    assert!(d.check(control) && d.can_suggest(control));
                    assert_eq!(d.check(word), flag != 'F', "{flag}: {word}");
                    assert!(!d.can_suggest(word), "{flag}: {word}");
                }
            }
        }
    }

    #[test]
    fn final_rule_suffix_retains_an_independent_allowed_derivation() {
        let aff = "SET UTF-8\nFORBIDDENWORD F\nNOSUGGEST N\nCOMPOUNDMIN 2\nCOMPOUNDRULE 1\nCOMPOUNDRULE BE\nSFX S N 1\nSFX S 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\nfirst/B\nroot/ES\nroots/EF\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for d in [&d, &back] {
            assert!(!d.check("roots"));
            assert!(d.check("firstroot") && d.can_suggest("firstroot"));
            // Independently observed with Hunspell 1.7.2 and this original dictionary.
            assert!(d.check("firstroots"));
        }
    }

    #[test]
    fn forceucase_restricts_only_final_compound_parts() {
        for (aff, dic) in [
            ("COMPOUNDBEGIN B\nCOMPOUNDEND E\nFORCEUCASE U\n", "3\nalpha/BU\nbeta/ESU\ngamma/E\n"),
            ("COMPOUNDRULE 1\nCOMPOUNDRULE AB\nFORCEUCASE U\n", "3\nalpha/AU\nbeta/BSU\ngamma/B\n"),
            ("FLAG long\nCOMPOUNDBEGIN Bb\nCOMPOUNDEND Ee\nFORCEUCASE Uu\n", "3\nalpha/BbUu\nbeta/EeSsUu\ngamma/Ee\n"),
        ] {
            let suffix = if aff.contains("FLAG long") { "Ss" } else { "S" };
            let aff = format!("SET UTF-8\nCOMPOUNDMIN 2\n{aff}SFX {suffix} N 1\nSFX {suffix} 0 s .\n");
            let d = AffixDict::from_hunspell(&aff, dic, &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                for w in ["alphabeta", "alphabetas"] {
                    assert!(!d.check(w) && !d.can_suggest(w), "{w}");
                }
                for w in ["Alphabeta", "ALPHABETA", "Alphabetas", "ALPHABETAS", "alphagamma", "beta"] {
                    assert!(d.check(w), "{w}");
                }
            }
        }
    }

    #[test]
    fn rule_compounds_allow_only_a_final_suffix() {
        let d = AffixDict::from_hunspell(
            "SET UTF-8\nCOMPOUNDMIN 2\nFORBIDDENWORD F\nNOSUGGEST N\nCOMPOUNDRULE 1\nCOMPOUNDRULE AB\nSFX S N 1\nSFX S 0 s .\n",
            "5\nalpha/AS\nbeta/BS\ngamma/BSN\ndelta/BSF\nzeta/B\n",
            &[],
        )
        .unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for d in [&d, &back] {
            assert!(d.check("alphabetas") && d.can_suggest("alphabetas"));
            assert!(d.check("alphabeta") && d.check("betas"));
            assert!(!d.check("alphasbeta"), "only the final rule component may inflect");
            assert!(d.check("alphagammas") && !d.can_suggest("alphagammas"));
            assert!(!d.check("alphadeltas"), "forbidden roots cannot inflect");
            assert!(!d.check("alphazetas"), "the final stem must allow the suffix");
        }
    }

    #[test]
    fn compound_part_limit_does_not_poison_shorter_paths() {
        let d = AffixDict::from_hunspell("COMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE A*B\n", "3\na/A\naa/A\nb/B\n", &[]).unwrap();
        assert!(d.check("aaaaaaaaab"), "aa+aa+aa+aa+a+b uses six parts");
        assert!(!d.check("aaaaaaaaaaaaaaab"), "at least nine parts would be needed");
    }

    #[test]
    fn fullstrip_applies_to_positional_parts() {
        for enabled in [false, true] {
            let flag = if enabled { "FULLSTRIP\n" } else { "" };
            let aff = format!("{flag}COMPOUNDMIN 1\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nSFX S N 1\nSFX S a xy .\nPFX T N 1\nPFX T c uv .\n");
            let d = AffixDict::from_hunspell(&aff, "4\nleft/B\na/SE\nright/E\nc/TB\n", &[]).unwrap();
            for w in ["xy", "uv", "leftxy", "uvright"] {
                assert_eq!(d.check(w), enabled, "{w}, FULLSTRIP={enabled}");
            }
            assert!(d.check("leftright"));
        }
    }

    #[test]
    fn compound_only_homonym_does_not_hide_an_ordinary_entry() {
        let aff = "ONLYINCOMPOUND X\nCOMPOUNDMIN 2\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nSFX S N 1\nSFX S 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\nexample/XBS\nexample\nend/E\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for d in [&d, &back] {
            assert!(d.check("example"));
            assert!(!d.check("examples"), "the suffix belongs only to the compound-only homonym");
            assert!(d.check("exampleend"));
            assert!(!d.forms("example").contains(&"examples".into()));
        }
    }

    #[test]
    fn forbidden_derivations_do_not_become_compounds() {
        let aff = "FORBIDDENWORD F\nCOMPOUNDMIN 2\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nSFX S N 1\nSFX S 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\nalpha/B\nbeta/ES\nalphabeta/FS\n", &[]).unwrap();
        assert!(!d.check("alphabetas"));
        assert!(!d.check("Alphabetas"));
        assert!(!d.can_suggest("alphabetas"));
        assert!(d.check("betas"));
    }

    #[test]
    fn duplicate_compound_stems_include_inflected_forms() {
        let aff = "COMPOUNDMIN 2\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nCHECKCOMPOUNDDUP\nCOMPOUNDPERMITFLAG P\nSFX S N 1\nSFX S 0 en/P .\n";
        let d = AffixDict::from_hunspell(aff, "2\nbad/BES\nbed/BES\n", &[]).unwrap();
        assert!(!d.check("badbaden"));
        assert!(!d.check("badenbad"));
        assert!(d.check("badbeden"), "different stems may carry identical flags");
    }

    #[test]
    fn compound_pattern_replacements_are_rejected() {
        let aff = "COMPOUNDMIN 2\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nCHECKCOMPOUNDPATTERN 1\n";
        let dic = "2\nalpha/B\nbeta/E\n";
        assert!(AffixDict::from_hunspell(&format!("{aff}CHECKCOMPOUNDPATTERN a b x\n"), dic, &[]).is_err());
        let d = AffixDict::from_hunspell(&format!("{aff}CHECKCOMPOUNDPATTERN a b # a comment\n"), dic, &[]).unwrap();
        assert!(!d.check("alphabeta"));
        let d = AffixDict::from_hunspell("COMPOUNDBEGIN B\nCOMPOUNDEND E\n", dic, &[]).unwrap();
        assert!(d.check("alphabeta"));
    }

    #[test]
    fn unmodified_stem_pattern_mode_is_not_silently_treated_as_empty() {
        let aff = "COMPOUNDMIN 2\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nCHECKCOMPOUNDPATTERN 1\nCHECKCOMPOUNDPATTERN 0/B /E\n";
        assert!(AffixDict::from_hunspell(aff, "2\nalpha/B\nbeta/E\n", &[]).is_err());
        // Ordinary empty sides still survive the existing compact encoding.
        let ordinary = aff.replace("0/B", "/B");
        let d = AffixDict::from_hunspell(&ordinary, "2\nalpha/B\nbeta/E\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(!d.check("alphabeta") && !back.check("alphabeta"));
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

    #[test]
    fn optional_rules_use_bounded_iterative_search() {
        for (n, accepted) in [(32, true), (100_000, false)] {
            let pattern = format!("{}B", "A?".repeat(n));
            let aff = format!("SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE {pattern}\n");
            let d = AffixDict::from_hunspell(&aff, "2\nx/A\ny/B\n", &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                assert_eq!(d.check("xy"), accepted, "{n} optional elements");
                assert_eq!(d.can_suggest("xy"), accepted, "{n} optional suggestion elements");
                assert!(!d.check("xz"));
            }
        }
    }

    #[test]
    fn optional_tail_does_not_consume_a_compound_part() {
        let d = AffixDict::from_hunspell("COMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE AAAAAAAAB?\n", "2\nx/A\ny/B\n", &[]).unwrap();
        assert!(d.check("xxxxxxxx") && d.can_suggest("xxxxxxxx"));
        assert!(!d.check("xxxxxxx") && !d.check("xxxxxxxxx"));
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
