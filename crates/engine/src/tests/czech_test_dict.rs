//! Hand-written synthetic Czech test data. No words, rules or suggestion tables were copied
//! from a system dictionary. This is a small fixture, not a redistributable Czech dictionary.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY aeiouěščřžýáíéůúntklmp
KEY xn|qř
MAP 5
MAP eěé
MAP cč
MAP rř
MAP uůú
MAP ií
REP 1
REP mňe mě
SFX A Y 2 # synthetic declension
SFX A o a o
SFX A o ě o
SFX B N 1
SFX B 0 e/C .
SFX C N 1
SFX C e ejší e
SFX D N 1
SFX D 0 itel/E .
SFX E N 1
SFX E 0 é .
SFX P Y 1
SFX P ha ze ha
PFX X Y 1
PFX X 0 ne .
";

pub const DIC: &str = "20
město/AX
rychl/B
uč/D
Praha/P
škola
počítač
příliš
žluťoučký
kůň
úpěl
ďábelské
ódy
zpráva
zprava
děkujeme
mě
nůž
si
to
zqzakázané/F
";

pub fn install() {
    super::install_dictionary(AFF, DIC).unwrap();
}
