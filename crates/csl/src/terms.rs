//! Locale terms: the en-US locale of the CSL 1.0.2 spec appendix, written from that public
//! spec in our own arrangement — every term the appendix defines for en-US: months and
//! seasons, ordinals and long ordinals, the locator labels, the name-role verbs, and the
//! misc words (`in`, `circa`, `forthcoming`, `ibid`, quotation marks, …). A style's own
//! `xml:lang="en-US"` (or lang-less, default-locale) `<term>` overrides replace entries.
//!
//! Only en-US is carried: a style's non-en-US `<locale>` sections are refused with a
//! warning by the parser, never silently swapped. Term references the merged set can't
//! resolve are reported as warnings at parse time (they would render empty).

use std::collections::BTreeMap;

use crate::model::Form;

/// A term's two shapes: singular and plural ("page"/"pages", "p."/"pp.").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Term {
    pub single: String,
    pub multiple: String,
}

/// Terms keyed by `(name, form)`. Constructed from the en-US defaults; a style's
/// `xml:lang="en-US"` (or lang-less, default-locale) `<term>` overrides replace entries.
#[derive(Clone, Debug, PartialEq)]
pub struct TermSet(BTreeMap<(String, Form), Term>);

/// A term with one shape for singular and plural.
fn same(s: &str) -> Term {
    Term { single: s.into(), multiple: s.into() }
}

impl Term {
    pub fn new(single: &str, multiple: &str) -> Term {
        Term { single: single.into(), multiple: multiple.into() }
    }
}

impl TermSet {
    /// The spec appendix's en-US terms.
    pub fn en_us() -> TermSet {
        let mut set = TermSet(BTreeMap::new());

        // Months (long and short; the locale keeps May, June and July unabbreviated, and
        // abbreviates September as "Sept.").
        let months_long = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
        let months_short = ["Jan.", "Feb.", "Mar.", "Apr.", "May", "June", "July", "Aug.", "Sept.", "Oct.", "Nov.", "Dec."];
        for (i, m) in months_long.iter().enumerate() {
            set.set(&format!("month-{:02}", i + 1), Form::Long, same(m));
            set.set(&format!("month-{:02}", i + 1), Form::Short, same(months_short[i]));
        }

        // Seasons.
        for (i, s) in ["Spring", "Summer", "Autumn", "Winter"].iter().enumerate() {
            set.set(&format!("season-{:02}", i + 1), Form::Long, same(s));
        }

        // Ordinal suffixes and long ordinals.
        set.set("ordinal", Form::Long, same("th"));
        set.set("ordinal-01", Form::Long, same("st"));
        set.set("ordinal-02", Form::Long, same("nd"));
        set.set("ordinal-03", Form::Long, same("rd"));
        set.set("ordinal-04", Form::Long, same("th"));
        let long_ordinals = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];
        for (i, o) in long_ordinals.iter().enumerate() {
            set.set(&format!("long-ordinal-{:02}", i + 1), Form::Long, same(o));
        }

        // Locator labels (long and short; singular and plural where they vary).
        set.set("book", Form::Long, Term::new("book", "books"));
        set.set("book", Form::Short, Term::new("bk.", "bks."));
        set.set("chapter", Form::Long, Term::new("chapter", "chapters"));
        set.set("chapter", Form::Short, Term::new("chap.", "chaps."));
        set.set("column", Form::Long, Term::new("column", "columns"));
        set.set("column", Form::Short, Term::new("col.", "cols."));
        set.set("figure", Form::Long, Term::new("figure", "figures"));
        set.set("figure", Form::Short, Term::new("fig.", "figs."));
        set.set("folio", Form::Long, Term::new("folio", "folios"));
        set.set("folio", Form::Short, Term::new("fol.", "fols."));
        set.set("issue", Form::Long, Term::new("number", "numbers"));
        set.set("issue", Form::Short, Term::new("no.", "nos."));
        set.set("line", Form::Long, Term::new("line", "lines"));
        set.set("line", Form::Short, Term::new("l.", "ll."));
        set.set("note", Form::Long, Term::new("note", "notes"));
        set.set("note", Form::Short, Term::new("n.", "nn."));
        set.set("opus", Form::Long, Term::new("opus", "opera"));
        set.set("opus", Form::Short, Term::new("op.", "opp."));
        set.set("page", Form::Long, Term::new("page", "pages"));
        set.set("page", Form::Short, Term::new("p.", "pp."));
        set.set("paragraph", Form::Long, Term::new("paragraph", "paragraphs"));
        set.set("paragraph", Form::Short, Term::new("para.", "paras."));
        set.set("part", Form::Long, Term::new("part", "parts"));
        set.set("part", Form::Short, Term::new("pt.", "pts."));
        set.set("scale", Form::Long, same("scale"));
        set.set("section", Form::Long, Term::new("section", "sections"));
        set.set("section", Form::Short, Term::new("sec.", "secs."));
        set.set("sub verbo", Form::Long, Term::new("sub verbo", "sub verbis"));
        set.set("sub verbo", Form::Short, Term::new("s.v.", "s.vv."));
        set.set("verse", Form::Long, Term::new("verse", "verses"));
        set.set("verse", Form::Short, Term::new("v.", "vv."));
        set.set("version", Form::Long, same("version"));
        set.set("volume", Form::Long, Term::new("volume", "volumes"));
        set.set("volume", Form::Short, Term::new("vol.", "vols."));

        // Name roles.
        set.set("editor", Form::Long, Term::new("editor", "editors"));
        set.set("editor", Form::Short, Term::new("ed.", "eds."));
        set.set("editor", Form::Verb, same("edited by"));
        set.set("editor", Form::VerbShort, same("ed. by"));
        set.set("editor-translator", Form::Long, Term::new("editor and translator", "editors and translators"));
        set.set("editor-translator", Form::Verb, same("edited and translated by"));
        set.set("translator", Form::Long, Term::new("translator", "translators"));
        set.set("translator", Form::Short, same("trans."));
        set.set("translator", Form::Verb, same("translated by"));

        // Misc words.
        set.set("accessed", Form::Long, same("accessed"));
        set.set("and", Form::Long, same("and"));
        set.set("and", Form::Symbol, same("&"));
        set.set("anonymous", Form::Long, same("anonymous"));
        set.set("anonymous", Form::Short, same("anon."));
        set.set("at", Form::Long, same("at"));
        set.set("available at", Form::Long, same("available at"));
        set.set("by", Form::Long, same("by"));
        set.set("circa", Form::Long, same("circa"));
        set.set("circa", Form::Short, same("c."));
        set.set("cited", Form::Long, same("cited"));
        set.set("edition", Form::Long, Term::new("edition", "editions"));
        set.set("edition", Form::Short, Term::new("ed.", "eds."));
        set.set("et-al", Form::Long, same("et al."));
        set.set("forthcoming", Form::Long, same("forthcoming"));
        set.set("from", Form::Long, same("from"));
        set.set("ibid", Form::Long, same("ibid."));
        set.set("in", Form::Long, same("in"));
        set.set("in press", Form::Long, same("in press"));
        set.set("internet", Form::Long, same("internet"));
        set.set("no date", Form::Long, same("no date"));
        set.set("no date", Form::Short, same("n.d."));
        set.set("number-of-pages", Form::Long, Term::new("number of pages", "number of pages"));
        set.set("number-of-pages", Form::Short, Term::new("p.", "pp."));
        set.set("number-of-volumes", Form::Long, Term::new("number of volumes", "number of volumes"));
        set.set("number-of-volumes", Form::Short, Term::new("vv.", "vv."));
        set.set("online", Form::Long, same("online"));
        set.set("personal communication", Form::Long, same("personal communication"));
        set.set("presented at", Form::Long, same("presented at"));
        set.set("retrieved", Form::Long, same("retrieved"));
        set.set("review of", Form::Long, same("review of"));

        // Eras and quotation marks.
        set.set("ad", Form::Long, same("AD"));
        set.set("bc", Form::Long, same("BC"));
        set.set("open-quote", Form::Long, same("“"));
        set.set("close-quote", Form::Long, same("”"));
        set.set("open-inner-quote", Form::Long, same("‘"));
        set.set("close-inner-quote", Form::Long, same("’"));

        set
    }

    /// Look a term up, falling back to the long form when the requested form is missing
    /// (the spec's term fallback).
    pub fn get(&self, name: &str, form: Form) -> Option<&Term> {
        if form != Form::Long
            && let Some(t) = self.0.get(&(name.to_string(), form))
        {
            return Some(t);
        }
        self.0.get(&(name.to_string(), Form::Long))
    }

    /// The text of a term: its `multiple` shape when `plural`, else `single`.
    pub fn text<'a>(&'a self, name: &str, form: Form, plural: bool) -> Option<&'a str> {
        let t = self.get(name, form)?;
        Some(if plural { t.multiple.as_str() } else { t.single.as_str() })
    }

    /// Replace or add an entry (a style's en-US override).
    pub(crate) fn set(&mut self, name: &str, form: Form, term: Term) {
        self.0.insert((name.to_string(), form), term);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_months_and_labels() {
        let t = TermSet::en_us();
        assert_eq!(t.text("month-01", Form::Long, false), Some("January"));
        assert_eq!(t.text("month-09", Form::Short, false), Some("Sept."));
        assert_eq!(t.text("month-12", Form::Long, true), Some("December"));
        assert_eq!(t.text("page", Form::Short, false), Some("p."));
        assert_eq!(t.text("page", Form::Short, true), Some("pp."));
        assert_eq!(t.text("page", Form::Long, false), Some("page"));
        assert_eq!(t.text("and", Form::Symbol, false), Some("&"));
        assert_eq!(t.text("et-al", Form::Long, false), Some("et al."));
        assert_eq!(t.text("editor", Form::Verb, true), Some("edited by"));
        assert_eq!(t.text("no date", Form::Short, false), Some("n.d."));
    }

    #[test]
    fn the_appendix_set_answers_common_references() {
        let t = TermSet::en_us();
        // Words real styles lean on.
        assert_eq!(t.text("in", Form::Long, false), Some("in"));
        assert_eq!(t.text("circa", Form::Short, false), Some("c."));
        assert_eq!(t.text("forthcoming", Form::Long, false), Some("forthcoming"));
        assert_eq!(t.text("in press", Form::Long, false), Some("in press"));
        assert_eq!(t.text("by", Form::Long, false), Some("by"));
        assert_eq!(t.text("online", Form::Long, false), Some("online"));
        // Locator labels the renderer's <label> can be pointed at.
        assert_eq!(t.text("verse", Form::Short, true), Some("vv."));
        assert_eq!(t.text("note", Form::Short, false), Some("n."));
        assert_eq!(t.text("line", Form::Short, true), Some("ll."));
        assert_eq!(t.text("opus", Form::Short, true), Some("opp."));
        assert_eq!(t.text("folio", Form::Short, false), Some("fol."));
        assert_eq!(t.text("scale", Form::Long, false), Some("scale"));
        assert_eq!(t.text("version", Form::Long, false), Some("version"));
        assert_eq!(t.text("chapter", Form::Short, false), Some("chap."));
        // Ordinals, long ordinals, seasons, quotes, eras.
        assert_eq!(t.text("ordinal-01", Form::Long, false), Some("st"));
        assert_eq!(t.text("long-ordinal-10", Form::Long, false), Some("tenth"));
        assert_eq!(t.text("season-01", Form::Long, false), Some("Spring"));
        assert_eq!(t.text("open-quote", Form::Long, false), Some("“"));
        assert_eq!(t.text("bc", Form::Long, false), Some("BC"));
        // An absent name still misses (the parse-time warning hinges on this).
        assert_eq!(t.text("not-a-term-at-all", Form::Long, false), None);
    }

    #[test]
    fn missing_forms_fall_back_to_long() {
        let t = TermSet::en_us();
        // et-al has no verb form: the long form answers.
        assert_eq!(t.text("et-al", Form::Verb, false), Some("et al."));
        assert_eq!(t.text("made-up-term", Form::Long, false), None);
    }

    #[test]
    fn overrides_replace_entries() {
        let mut t = TermSet::en_us();
        t.set("page", Form::Short, Term::new("S.", "Ss."));
        assert_eq!(t.text("page", Form::Short, true), Some("Ss."));
        // Other entries are untouched.
        assert_eq!(t.text("page", Form::Long, true), Some("pages"));
    }
}
