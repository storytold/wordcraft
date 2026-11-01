//! Hand-written synthetic Serbian test data, one dictionary per script, in `FLAG num` form like
//! the real Serbian dictionaries (and, like them, with a KEY table but no TRY or MAP). No words,
//! rules or tables were copied from a system dictionary; this is a small fixture, not a
//! redistributable Serbian dictionary. Flags are numbers well past one character's range (120,
//! 1500, 65000) so the numeric reader is what reads them.

/// Cyrillic: masculine nouns (120), feminine `-а` nouns (7), a verb in `-ати` (1500) with the
/// prefix `про-` (33), and a forbidden word (65000).
pub const CYRILLIC_AFF: &str = "SET UTF-8
FLAG num
KEY јкл|чћ|шђж
FORBIDDENWORD 65000

SFX 120 Y 5
SFX 120 0 а .
SFX 120 0 у .
SFX 120 0 ом .
SFX 120 0 ови .
SFX 120 0 овима .

SFX 7 Y 4
SFX 7 а е а
SFX 7 а и а
SFX 7 а у а
SFX 7 а ом а

SFX 1500 Y 3
SFX 1500 ти 0 ати
SFX 1500 ти м ати
SFX 1500 ти ју ати

PFX 33 Y 1
PFX 33 0 про .
";

pub const CYRILLIC_DIC: &str = "12
град/120
Београд/120
рачунар/120
универзитет/120
кућа/7
књига/7
читати/1500,33
црно
бели
зкзабрањено/65000
и
у
";

/// Latin: the same rules and words in Serbian Latin script, with the digraph words `ljubav` and
/// `džep`.
pub const LATIN_AFF: &str = "SET UTF-8
FLAG num
KEY jkl|čć|šđž
FORBIDDENWORD 65000

SFX 120 Y 5
SFX 120 0 a .
SFX 120 0 u .
SFX 120 0 om .
SFX 120 0 ovi .
SFX 120 0 ovima .

SFX 7 Y 4
SFX 7 a e a
SFX 7 a i a
SFX 7 a u a
SFX 7 a om a

SFX 9 Y 1
SFX 9 0 lju .

SFX 1500 Y 3
SFX 1500 ti 0 ati
SFX 1500 ti m ati
SFX 1500 ti ju ati

PFX 33 Y 1
PFX 33 0 pro .
";

pub const LATIN_DIC: &str = "14
grad/120
Beograd/120
računar/120
univerzitet/120
džep/120
kuća/7
knjiga/7
ljubav/9
čitati/1500,33
crno
beli
zkzabranjeno/65000
i
u
";

/// Install both fixtures as the Serbian dictionaries.
pub fn install() {
    use super::{Script, install_dictionary};
    install_dictionary(Script::Cyrillic, CYRILLIC_AFF, CYRILLIC_DIC).unwrap();
    install_dictionary(Script::Latin, LATIN_AFF, LATIN_DIC).unwrap();
}
