//! A tiny synthetic French Hunspell dictionary for tests (written for these tests; not taken
//! from any real dictionary). Real dictionaries are installed on the system, never bundled.
//!
//! It exercises what the Dicollecte French dictionary uses: two-character flags (`FLAG long`),
//! stems that need an affix (`NEEDAFFIX`, with an empty suffix for the singular), elisions as
//! prefixes licensed by a suffix's continuation (`l'arbre` but not `l'arbres`), irregular verbs
//! through `FULLSTRIP` (`être` → `suis`), `KEEPCASE` (`km`), `NOSUGGEST` (`bah`), `ICONV` (`’`
//! read as `'`) and the
//! suggestion tables (`TRY`, `MAP` for accents, `REP` for common confusions).

pub const AFF: &str = "SET UTF-8
FLAG long
NEEDAFFIX ()
KEEPCASE ||
NOSUGGEST --
FULLSTRIP
ICONV 1
ICONV ’ '
TRY esaitnrulodcpmévqfbghjàxèêyçôîâûùœ
MAP 6
MAP aàâä
MAP eéèêë
MAP iîï
MAP oôö
MAP uùûü
MAP cç
REP 3
REP oe œ
REP ph f
REP f ph
PFX L' Y 1
PFX L' 0 l' [aàâeéèêiîoôuœh]
PFX D' Y 1
PFX D' 0 d' [aàâeéèêiîoôuœh]
PFX Q' Y 1
PFX Q' 0 qu' [aàâeéèêiîoôuœh]
PFX J' Y 1
PFX J' 0 j' [aàâeéèêiîoôuœhy]
SFX S. Y 2
SFX S. 0 0/L'D'Q' [^sxz]
SFX S. 0 s/D'Q' [^sxz]
SFX S= Y 2
SFX S= 0 0 [^sxz]
SFX S= 0 s [^sxz]
SFX F. Y 4
SFX F. 0 0/L'D'Q' .
SFX F. 0 e/L'D'Q' .
SFX F. 0 s/D'Q' .
SFX F. 0 es/D'Q' .
SFX Er Y 6
SFX Er er er/D' er
SFX Er er e/J' er
SFX Er er es er
SFX Er er ons er
SFX Er er ez er
SFX Er er ent er
SFX Et N 7
SFX Et être être être
SFX Et être suis être
SFX Et être es être
SFX Et être est être
SFX Et être sommes être
SFX Et être êtes être
SFX Et être sont être
SFX Av N 7
SFX Av avoir avoir/D' avoir
SFX Av avoir ai/J' avoir
SFX Av avoir as avoir
SFX Av avoir a avoir
SFX Av avoir avons avoir
SFX Av avoir avez avoir
SFX Av avoir ont/Q' avoir
";

/// Stems (the count line first, as in Hunspell files).
pub const DIC: &str = "150
maison/S.()
arbre/S.()
enfant/S.()
ami/S.()
amie/S.()
homme/S.()
école/S.()
café/S.()
chat/S.()
rivière/S.()
ville/S.()
heure/S.()
histoire/S.()
sœur/S.()
œuvre/S.()
photo/S.()
téléphone/S.()
année/S.()
vie/S.()
livre/S.()
table/S.()
porte/S.()
femme/S.()
garçon/S.()
fille/S.()
père/S.()
mère/S.()
langue/S.()
mot/S.()
texte/S.()
phrase/S.()
exemple/S.()
jour/S.()
nuit/S.()
monde/S.()
hasard/S=()
haricot/S=()
grand/F.()
petit/F.()
noir/F.()
aimer/Er()
parler/Er()
chanter/Er()
arriver/Er()
être/Et()
avoir/Av()
il/Q'
ils/Q'
elle/Q'
elles/Q'
on/Q'L'
un/D'Q'L'
une/D'Q'L'
en/Q'J'
y/J'
à/Q'
jusqu'à
aujourd'hui/D'
peut-être
arc-en-ciel
km/||
bah/--
Paris
France
Montréal
le
la
les
de
des
du
et
à
je
tu
nous
vous
que
qui
ne
pas
se
me
te
ce
ça
avec
pour
dans
sur
sous
très
près
beaucoup
si
oui
non
mais
ou
où
dit
demanda
quand
comme
plus
bien
tout
toujours
jamais
ici
là
alors
donc
aussi
encore
déjà
mon
ma
mes
son
sa
ses
notre
votre
leur
cette
ces
au
aux
par
entre
vers
chez
sans
moi
toi
lui
rien
chose
temps
fois
eau
voici
voilà
bonjour
merci
beau
belle
la
etc
M
p
";

/// Make [`AFF`]/[`DIC`] the French dictionary (every test that needs one calls this; they all
/// install the same one, so tests running in parallel agree).
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
