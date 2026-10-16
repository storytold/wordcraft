//! A tiny synthetic Spanish (`es-419`) Hunspell dictionary for tests (written for these tests; not
//! taken from any real dictionary). Real dictionaries are installed on the system, never bundled.
//!
//! It exercises what Spanish Hunspell dictionaries use: `FLAG UTF-8` (`F` marks a forbidden
//! spelling), continuation classes (`probable` → `probables`), the suggestion tables (`TRY`, `MAP`
//! for accents and `ñ`, `REP` for common confusions) and one prefix.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY aeosrnidlctumpbgvyqhfzjxñáéíóúükw
MAP 6
MAP aá
MAP eé
MAP ií
MAP oó
MAP uúü
MAP nñ
REP 4
REP haiga haya
REP llendo yendo
REP exepción excepción
REP previlegio privilegio
SFX A Y 2
SFX A ar aba ar
SFX A ar ando ar
SFX B N 1
SFX B   0   able/C   .
SFX C N 1
SFX C   ble   bles   ble
PFX X Y 1
PFX X 0 des .
";

/// Stems (the count line first, as in Hunspell files).
pub const DIC: &str = "76
corazón
canción
acción
accion/F
niño
niña
año
español
mañana
árbol
música
salud
salúd
escribir
computadora
playa
conocer
México
Bogotá
Buenos
Aires
guarda
ropa
correo
marido
ex
vice
presidente
autoestima
frecuencia
frecuensia/F
excepción
privilegio
análisis
haya
yendo
trabajo
cantar/A
prob/B
cubrir/X
avión
pájaros
ustedes
coraje
felicitaciones
está
él
ella
dijo
existen
bien
no
sí
que
con
para
de
del
por
la
el
los
las
un
una
muy
más
como
porque
aquí
también
ya
ser
tener
ir
ver
dar
y
o
es
agua
en
";

/// Make [`AFF`]/[`DIC`] the Spanish dictionary (every test that needs one calls this; they all
/// install the same one, so tests running in parallel agree).
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
