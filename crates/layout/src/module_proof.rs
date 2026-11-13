//! Per-session English supplements from an optional module (LawCraft's legal word list and
//! confusion checks), applied around `wordcraft_proof` (craft-proof) without changing it.

use std::ops::Range;

use wordcraft_proof::{Issue, IssueKind, Options, ProofLang};

/// Extra English grammar or style checks from an installed module.
pub type EnParagraphGrammarCheck = fn(&str) -> Vec<Issue>;

/// A module's English supplements for one session. The host sets this on the session when a
/// module is installed and clears it when the module is turned off, so sessions stay independent.
#[derive(Clone, Copy, Default)]
pub struct ModuleEn {
    spelling: Option<&'static [&'static str]>,
    grammar: Option<EnParagraphGrammarCheck>,
}

impl std::fmt::Debug for ModuleEn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleEn").field("spelling_words", &self.spelling.map(<[_]>::len)).field("grammar", &self.grammar.is_some()).finish()
    }
}

impl PartialEq for ModuleEn {
    fn eq(&self, other: &Self) -> bool {
        self.spelling == other.spelling
            && match (self.grammar, other.grammar) {
                (None, None) => true,
                (Some(a), Some(b)) => std::ptr::fn_addr_eq(a, b),
                _ => false,
            }
    }
}

impl Eq for ModuleEn {}

impl std::hash::Hash for ModuleEn {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash the spelling contents, matching the content comparison in `PartialEq`.
        self.spelling.hash(state);
        self.grammar.map(|g| g as usize).hash(state);
    }
}

impl ModuleEn {
    /// Module base forms (sorted, lower case), or none when the module is off.
    pub fn with_spelling(spelling: Option<&'static [&'static str]>) -> Self {
        ModuleEn { spelling, grammar: None }
    }

    /// English grammar or style checks from the module.
    pub fn with_grammar(self, grammar: Option<EnParagraphGrammarCheck>) -> Self {
        ModuleEn { spelling: self.spelling, grammar }
    }

    /// Is this module off (no words, no checks)?
    pub fn is_empty(&self) -> bool {
        self.spelling.is_none() && self.grammar.is_none()
    }

    fn has(&self, lower: &str) -> bool {
        self.spelling.is_some_and(|words| words.binary_search(&lower).is_ok())
    }

    /// Does the module's word list cover `word`: the word itself, a plural or possessive of a
    /// listed word, or a hyphenated compound whose parts are each known?
    pub fn knows(&self, word: &str) -> bool {
        if self.spelling.is_none() {
            return false;
        }
        let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').trim_matches('\'');
        let lower = w.to_lowercase().replace('\u{2019}', "'");
        if lower.is_empty() {
            return false;
        }
        let known = |p: &str| {
            self.has(p)
                || p.strip_suffix("'s").is_some_and(|b| self.has(b))
                || p.strip_suffix("es").is_some_and(|b| self.has(b))
                || p.strip_suffix('s').is_some_and(|b| self.has(b))
        };
        known(&lower) || (lower.contains('-') && lower.split('-').all(|p| p.is_empty() || known(p) || wordcraft_proof::is_correct(p)))
    }
}

/// [`wordcraft_proof::is_correct_with`] plus the module's word list.
pub fn is_correct_with_module(word: &str, opts: &Options, module: ModuleEn) -> bool {
    wordcraft_proof::is_correct_with(word, opts) || module.knows(word)
}

/// [`wordcraft_proof::is_correct_in_with`] plus the module's word list (English only).
pub fn is_correct_in_with_module(word: &str, lang: ProofLang, opts: &Options, module: ModuleEn) -> bool {
    wordcraft_proof::is_correct_in_with(word, lang, opts) || (lang.is_english() && module.knows(word))
}

fn apply(text: &str, mut issues: Vec<Issue>, lang_at: impl Fn(usize) -> ProofLang, grammar: bool, module: ModuleEn) -> Vec<Issue> {
    if module.is_empty() {
        return issues;
    }
    issues.retain(|i| !(i.kind == IssueKind::Spelling && i.lang.is_english() && text.get(i.start..i.end).is_some_and(|w| module.knows(w))));
    if grammar && let Some(f) = module.grammar {
        issues.extend(f(text).into_iter().filter(|i| lang_at(i.start).is_english()));
        issues.sort_by_key(|i| (i.start, i.end));
    }
    issues
}

/// [`wordcraft_proof::check_spelling_with`] plus the module's word list.
pub fn check_spelling_with_module(text: &str, opts: &Options, module: ModuleEn) -> Vec<Issue> {
    apply(text, wordcraft_proof::check_spelling_with(text, opts), |_| ProofLang::En, false, module)
}

/// [`wordcraft_proof::check_text_with`] plus the module's supplements: its words are not
/// misspelled, and its grammar checks run on English text when `grammar` is on.
pub fn check_paragraph(
    text: &str,
    spans: &[(Range<usize>, ProofLang)],
    opts: &Options,
    grammar: bool,
    list_item: bool,
    module: ModuleEn,
) -> Vec<Issue> {
    let issues = wordcraft_proof::check_text_with(text, spans, opts, grammar, list_item);
    let lang_at = |at: usize| spans.iter().find(|(r, _)| r.contains(&at)).map_or(ProofLang::En, |(_, l)| *l);
    apply(text, issues, lang_at, grammar, module)
}

/// [`check_paragraph`] for text that is not a list item.
pub fn check_text_with_module(text: &str, spans: &[(Range<usize>, ProofLang)], opts: &Options, grammar: bool, module: ModuleEn) -> Vec<Issue> {
    check_paragraph(text, spans, opts, grammar, false, module)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_en_spelling_supplement() {
        assert!(!wordcraft_proof::is_correct("zxqmoduleword"));
        let on = ModuleEn::with_spelling(Some(&["zxqmoduleword"]));
        assert!(is_correct_with_module("zxqmoduleword", &Options::default(), on));
        assert!(is_correct_with_module("zxqmodulewords", &Options::default(), on));
        assert!(!is_correct_with_module("zxqmoduleword", &Options::default(), ModuleEn::default()));
        let text = "A zxqmoduleword here";
        assert!(check_spelling_with_module(text, &Options::default(), on).is_empty());
        assert!(!check_spelling_with_module(text, &Options::default(), ModuleEn::default()).is_empty());
    }

    #[test]
    fn module_en_grammar_hook() {
        fn always_hello(text: &str) -> Vec<Issue> {
            text.find("hello")
                .map(|start| Issue {
                    start,
                    end: start + 5,
                    kind: IssueKind::Grammar,
                    message: "test".into(),
                    suggestions: vec![],
                    lang: ProofLang::En,
                })
                .into_iter()
                .collect()
        }
        let module = ModuleEn::default().with_grammar(Some(always_hello));
        let spans = [(0..8, ProofLang::En)];
        let issues = check_text_with_module("Ok hello", &spans, &Options::default(), true, module);
        assert_eq!(issues.iter().filter(|i| i.message == "test").count(), 1);
        assert!(check_text_with_module("Ok hello", &spans, &Options::default(), true, ModuleEn::default()).iter().all(|i| i.message != "test"));
        assert!(check_text_with_module("Ok hello", &spans, &Options::default(), false, module).iter().all(|i| i.message != "test"));
    }
}
