//! Hand-written synthetic Spanish test data. No words, rules or suggestion tables were copied
//! from a system dictionary. This is a small fixture, not a redistributable Spanish dictionary.
//! Spain's word list has `vosotros` forms and Spain's words; the Americas' list (Mexico, Latin
//! America and Uruguay) has theirs instead, so the tests can tell the varieties' dictionaries apart.

use crate::ProofLang;

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY aeosrnidlctumpbgvyqhfzjñáéíóúüxkw
KEY qwertyuiop|asdfghjklñ|zxcvbnm
MAP 6
MAP aá
MAP eé
MAP ií
MAP oó
MAP uúü
MAP nñ
REP 2
REP v b
REP b v
SFX S Y 2 # synthetic plurals
SFX S 0 s [aeiou]
SFX S ón ones ón
SFX E Y 1
SFX E 0 es [^aeiou]
";

/// Spain.
pub const DIC: &str = "32
hablar
tener
venir
decir
andar
mirar
querer
fijar
poner
canción/S
prueba/S
corrector/E
también
México
esto
es
una
del
de
la
el
y
niño/S
pingüino/S
año/S
teórico
práctico
mexicano
franco
habéis
ordenador/E
zqprohibida/F
";

/// Mexico, Latin America and Uruguay.
pub const DIC_AMERICAS: &str = "33
hablar
tener
venir
decir
andar
mirar
querer
fijar
poner
canción/S
prueba/S
corrector/E
también
México
esto
es
una
del
de
la
el
y
niño/S
pingüino/S
año/S
teórico
práctico
mexicano
franco
platicar
computadora/S
han
zqprohibida/F
";

/// Install the fixture for every variety: [`DIC`] for Spain, [`DIC_AMERICAS`] for Mexico, Latin
/// America and Uruguay.
pub fn install() {
    for (lang, dic) in [(ProofLang::Es, DIC), (ProofLang::EsMx, DIC_AMERICAS), (ProofLang::Es419, DIC_AMERICAS), (ProofLang::EsUy, DIC_AMERICAS)] {
        if let Err(e) = super::install_dictionary(lang, AFF, dic) {
            panic!("test dictionary: {e}");
        }
    }
}
