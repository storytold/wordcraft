//! Original tiny German dictionaries for deterministic tests; no upstream dictionary data.

use crate::{ProofLang, affix::AffixDict};

pub const AFF: &str = "SET UTF-8
TRY abcdefghijklmnopqrstuvwxyzäöüß
COMPOUNDMIN 2
COMPOUNDBEGIN x
COMPOUNDMIDDLE y
COMPOUNDEND z
ONLYINCOMPOUND o
KEEPCASE k
NEEDAFFIX h
CHECKSHARPS
MAP 3
MAP aä
MAP oö
MAP uü
REP 1
REP sh sch
";

const COMMON: &str = "Haus/x
tür/oy
schlüssel/oz
Bundes/x
verwaltungs/oy
gericht/oz
ball/oz
Velofahrer
Rüebli
und
ist
Wort
iPhone/k
";

pub fn dictionary(lang: ProofLang) -> AffixDict {
    let local = match lang {
        ProofLang::DeCh => "Strasse\ngross\nFuss/x\n",
        ProofLang::DeAt => "Straße\ngroß\nFuß/x\nJänner\n",
        _ => "Straße\ngroß\nFuß/x\nJanuar\n",
    };
    let count = COMMON.lines().count() + local.lines().count();
    AffixDict::from_hunspell(AFF, &format!("{count}\n{COMMON}{local}"), &[]).unwrap()
}

pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        for lang in [ProofLang::De, ProofLang::DeAt, ProofLang::DeCh] {
            *super::slot(lang).write().unwrap() = Some(std::sync::Arc::new(dictionary(lang)));
        }
    });
}
