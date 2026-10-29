//! A tiny synthetic Romanian Hunspell dictionary for tests (written for these tests; not taken
//! from any real dictionary). Real dictionaries are installed on the system, never bundled.
//!
//! It exercises what the Romanian dictionary uses: the default single-character `FLAG` type,
//! suffixes (feminine `-ă` → `-e`/`-a`, `câine` → `câini`/`câinii`, verb endings `-a`, `-ez`,
//! `-ează`), a prefix (`re-`), the suggestion tables (`TRY`, `MAP` for diacritics, `REP` for
//! common confusions), the comma-below `ș`/`ț`, and the clipped pieces of hyphenated forms
//! (`într`, `s`, `n`) listed as words.

pub const AFF: &str = "SET UTF-8
TRY iaăâșțîertolncusmpdbgfzvhjxkwyq
MAP 4
MAP aăâî
MAP iîâ
MAP sș
MAP tț
REP 3
REP ce che
REP gi ghi
REP ia ea
SFX N Y 2
SFX N ă e ă
SFX N ă a ă
SFX C Y 2
SFX C e i e
SFX C e ii e
SFX V Y 3
SFX V 0 a .
SFX V 0 ez .
SFX V 0 ează .
PFX R Y 1
PFX R 0 re .
";

/// Stems (the count line first, as in Hunspell files).
pub const DIC: &str = "99
casă/N
fată/N
față/N
carte
cărți
câine/C
mâine
pâine
țară
țara
țări
știință
școală/N
apă
Brașov
București
Cluj
Ion
Maria
lucr/V
face/R
citește
merge
merg
vine
spus
plecat
este
e
sunt
am
ai
are
avem
au
a
o
un
o
și
sau
dar
ci
iar
nici
niciun
nicio
nu
da
de
la
în
pe
cu
din
pentru
mai
foarte
bine
frumos
frumoasă
acasă
azi
ieri
timp
om
oameni
zi
an
noi
voi
el
ea
eu
tu
ce
cine
care
când
unde
cum
acest
această
nord
est
sud
vest
într
dintr
printr
s
n
l
dă
mi
ți
îl
le
ne
vă
vreau
pot
";

/// Make [`AFF`]/[`DIC`] the Romanian dictionary (every test that needs one calls this; they all
/// install the same one, so tests running in parallel agree).
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
