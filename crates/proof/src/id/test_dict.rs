//! Hand-written synthetic Indonesian test data. No words, rules or suggestion tables were copied
//! from a system dictionary. This is a small fixture, not a redistributable Indonesian dictionary.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY aniekrumgtslhbdopy
KEY qw|as|zx
MAP 1
MAP eé
REP 2
REP oe u
REP dj j
SFX A Y 1 # synthetic -an
SFX A 0 an .
PFX D Y 1
PFX D 0 di .
PFX M Y 1
PFX M 0 mem b
";

pub const DIC: &str = "20
rumah
makan/AD
baca/ADM
sekolah
anak
buku
kupu-kupu
Jakarta
Indonesia
saya
kamu
dan
yang
ini
itu
pergi
jalan/A
besar
kecil
zqterlarang/F
";

pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
