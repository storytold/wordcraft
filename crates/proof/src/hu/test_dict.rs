//! A tiny synthetic Hungarian Hunspell dictionary for tests (written for these tests; not taken
//! from any real dictionary). Real dictionaries are installed on the system, never bundled.

/// Affix rules (a few case endings in both vowel harmonies), compounding in the shape of the
/// Hungarian dictionary, and the suggestion tables the Hungarian suggestions use (`TRY`, `MAP`
/// for accents, `REP` for common confusions).
pub const AFF: &str = "SET UTF-8
TRY aeoiulznrstkmbhgdáéíóöőúüűvjpfcy
MAP 5
MAP aá
MAP eé
MAP ií
MAP oóöő
MAP uúüű
REP 3
REP jl lj
REP j ly
REP ly j
FORBIDDENWORD W
COMPOUNDFLAG Y
COMPOUNDMIN 2
COMPOUNDWORDMAX 2
COMPOUNDSYLLABLE 6 aáeéiíoóöőuúüű
CHECKCOMPOUNDDUP
CHECKCOMPOUNDTRIPLE
CHECKCOMPOUNDREP
BREAK 2
BREAK -
BREAK ^-
SFX A Y 4
SFX A 0 ban [^a]
SFX A a ában a
SFX A 0 ak [^a]
SFX A a ák a
SFX E Y 2
SFX E 0 ben .
SFX E 0 ek .
SFX N Y 1
SFX N 0 en .
SFX M Y 1
SFX M 0 mel m
";

/// Stems (the count line first, as in Hunspell files).
pub const DIC: &str = "52
ház/YA
alma/YA
fa/YA
kulcs/YA
asztal/YA
láb/YA
kert/YE
szék/YE
könyv/YE
ember/YE
kenyér/YE
vers/YE
öröm/YEM
helyesírás/A
haj/Y
lak/Y
haljak
híd/Y
Petőfi/Y
Rákóczi
Rákóczi-híd/W
Budapest/N
SMS
SMS-ben
MÁV
a
az
e
egy
és
is
nem
hogy
mert
de
hanem
ami
amit
aki
amely
szép
nagyon
kérek
kerek
mond
tudom
jön
olvas
igen
október
május
fejezet
";

/// Install the test dictionary as the Hungarian one.
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
