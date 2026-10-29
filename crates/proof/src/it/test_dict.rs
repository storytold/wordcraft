//! A tiny synthetic Italian Hunspell dictionary for tests (written for these tests; not taken
//! from any real dictionary). Real dictionaries are installed on the system, never bundled.
//!
//! It exercises what the Italian dictionaries use: the default single-character `FLAG` type,
//! suffixes (verb endings `-are`, noun plurals `-o` → `-i`), a prefix (`ri-`), the suggestion
//! tables (`TRY`, `MAP` for accents, `REP` for common confusions) and elided forms with an
//! apostrophe (`l'amico`).

pub const AFF: &str = "SET UTF-8
TRY aeiortnslcdupmghfbqvàèéìòóùz
MAP 5
MAP aà
MAP eéè
MAP iì
MAP oòó
MAP uù
REP 3
REP caffé caffè
REP perche perché
REP cioe cioè
SFX A Y 3
SFX A 0 are .
SFX A 0 a .
SFX A 0 ano .
SFX N Y 2
SFX N o i o
SFX N a e a
PFX R Y 1
PFX R 0 ri .
";

/// Stems (the count line first, as in Hunspell files).
pub const DIC: &str = "97
città
caffè
perché
più
può
è
già
così
virtù
tribù
menù
tè
Mosca
Roma
Milano
Torino
casa/N
libro/N
cane
gatto
sole
luna
mare
amico
amica
idea
parl/A
mangiare
dormire
fare/R
scuola
bambino/N
albero
fiore
pane
acqua
terra
cielo
notte
giorno
vecchia
vado
con
ha
ho
bel
cioè
la
un
in
piazza
lavoro
tempo
uomo
donna
bello/N
buono
grande
piccolo
nuovo/N
vecchio
bianco
nero
rosso
verde
giallo
uno
due
tre
tutto
come
quando
dove
senza
sopra
sotto
tra
molto
sempre
mai
qui
lì
sì
no
non
che
chi
cui
mio
tuo
suo
nostro
a
per
me
quello
io
";

/// Make [`AFF`]/[`DIC`] the Italian dictionary (every test that needs one calls this; they all
/// install the same one, so tests running in parallel agree).
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
