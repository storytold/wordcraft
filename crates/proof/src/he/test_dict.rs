//! Hand-written synthetic Hebrew test fixture for unit tests.

/// Synthetic Hunspell `.aff` rules for Hebrew tests.
pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
NEEDAFFIX X
TRY שלוםביתספרעבודהאניאתהךםןףץ
REP 2
REP מ ם
REP נ ן
PFX H Y 3
PFX H 0 ה .
PFX H 0 ב .
PFX H 0 וב .
SFX S Y 1
SFX S 0 ים .
";

/// Synthetic Hunspell `.dic` word list for Hebrew tests.
pub const DIC: &str = "12
שלום/H
בית/H
ספר/HS
עולם/H
עבודה/H
ילד/HS
ילדה/H
ירושלים
ישראל
צה\"ל
וגו'
מילהאסורה/F
";

/// Installs the synthetic Hebrew dictionary for unit tests.
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
