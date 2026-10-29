//! A tiny synthetic Polish Hunspell dictionary for tests (written for these tests; not taken from
//! any real dictionary). Real dictionaries are installed on the system, never bundled.

/// Affix rules: a few suffixes and one prefix, plus the suggestion tables the Polish
/// suggestions use (`TRY`, `MAP` for diacritics, `REP` for common confusions).
pub const AFF: &str = "SET UTF-8
TRY aioeznrwcysżtkpdmłjlubąęgchśóźćń
MAP 9
MAP aą
MAP cć
MAP eę
MAP lł
MAP nń
MAP oó
MAP uó
MAP sś
MAP zżź
REP 6
REP ż rz
REP rz ż
REP ch h
REP h ch
REP sz rz
REP ść ć
SFX A Y 1
SFX A a ie a
SFX B Y 3
SFX B y ego y
SFX B i iego i
SFX B i iej i
PFX N Y 1
PFX N 0 nie .
";

/// Stems (the count line first, as in Hunspell files).
pub const DIC: &str = "69
źdźbło
gżegżółka
chrząszcz
chleb
chłep
chlap
chlip
chlup
chlej
pięćdziesięciu
rzeka
którzy
żółw
zażółć
gęślą
jaźń
konstytucja
konstytucyjny/B
niebieski/B
niemiecki/B
ładnie/N
przyszliśmy
zrobilibyśmy
Warszawa/A
kot
biało
czerwony
polsko
mail
w
się
huśtawka
brzoza
przyjaciel
wziąć
dziewczyna
na
pewno
ogóle
nie
wiem
płynie
a
i
o
to
że
tak
jest
brzmi
trzcina/A
napisał
poszedł
spać
bo
wylała
świat
dom
ma
ala
kota
dzień
dobry
jutro
bardzo
dobrze
mamy
czas
ten
";

/// Make [`AFF`]/[`DIC`] the Polish dictionary (every test that needs one calls this; they all
/// install the same one, so tests running in parallel agree).
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
