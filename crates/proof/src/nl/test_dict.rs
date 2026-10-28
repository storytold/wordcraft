//! Hand-written synthetic Dutch test data in the shape of a compounding dictionary (two-character
//! flags, `ij` stored as `ĳ`, compound parts, a vowel-clash pattern). No words, rules or
//! suggestion tables were copied from a system dictionary. This is a small fixture, not a
//! redistributable Dutch dictionary.

pub const AFF: &str = "SET UTF-8
FLAG long
TRY eantirodslgvhkmuwbpjzcfĳ
KEY qwertyuiop|asdfghjkl|zxcvbnm
ICONV 3
ICONV ’ '
ICONV ij ĳ
ICONV IJ Ĳ
OCONV 2
OCONV ĳ ij
OCONV Ĳ IJ
FORBIDDENWORD Fw
COMPOUNDMIN 1
COMPOUNDRULE 1
COMPOUNDRULE (N1)(NH)
CHECKCOMPOUNDCASE
COMPOUNDBEGIN Ca
COMPOUNDMIDDLE Cb
COMPOUNDEND Cc
COMPOUNDPERMITFLAG Cp
CHECKCOMPOUNDDUP
CHECKCOMPOUNDPATTERN 1
CHECKCOMPOUNDPATTERN e /V2
REP 2
REP ei ĳ
REP ^nml$ namelĳk
SFX CA Y 2
SFX CA 0 /CaCp .
SFX CA 0 -/CaCp .
SFX En Y 1
SFX En 0 en .
SFX As Y 1
SFX As 0 's .
";

pub const DIC: &str = "22
zwembad/CACbCcEn
bouwer/CcEn
tuin/CACb
huis/CACbCc
zĳn
Ĳsland
auto/As
Noord-Holland
's-Hertogenbosch
zee/CA
egel/CcV2
fiets/CA
hok/Cc
vĳf/N1
honderd/NH
het
de
een
fietsenhok/Fw
renovatie/Cc
in
namelĳk
";

pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
