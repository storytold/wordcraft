//! Hand-written synthetic Latvian test data. No words, rules or suggestion tables were copied
//! from a system dictionary. This is a small fixture, not a redistributable Latvian dictionary.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY aiestnrlkumopdvzgjbcšēāīūčžģķļņfh
KEY qwertyuiop|asdfghjkl|zxcvbnm
MAP 11
MAP aā
MAP eē
MAP iī
MAP uū
MAP cč
MAP gģ
MAP kķ
MAP lļ
MAP nņ
MAP sš
MAP zž
REP 1
REP uu u
SFX A Y 3
SFX A a as a
SFX A a ai a
SFX A a u a
";

pub const DIC: &str = "15
suns
kaķis
māja/A
skola/A
cilvēks
ūdens
Rīga
Latvija
es
tu
un
ir
liels
mazs
zqterlarangs/F
";

pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
