//! Small synthetic UTF-8 Croatian Hunspell dictionary for deterministic tests.
pub const AFF: &str = "SET UTF-8
TRY abcčćdđefghijklmnoprsštuvzž
MAP 5
MAP cčć
MAP dđ
MAP sš
MAP zž
MAP iije
REP 3
REP c č
REP dj đ
REP je ije
SFX A Y 2
SFX A a e a
SFX A a i a
";
pub const DIC: &str = "18
kuća/A
računalo
čovjek
riječ
dijete
Hrvatska
čokolada
škola/A
more
sunce
dobar
hrvatski
jezik
tekst
pogreška
pravopis
provjera
računala
";
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
