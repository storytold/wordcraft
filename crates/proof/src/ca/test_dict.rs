//! Original miniature fixture. No dictionary entries or rules were copied from installed data.

pub const AFF: &str = "SET UTF-8
FLAG long
FULLSTRIP
TRY acdeghilmnoqrstuxàçéíïóü·-'
ICONV 1
ICONV ’ '
MAP 3
MAP aà
MAP oó
MAP uü
REP 1
REP cataln català
FORBIDDENWORD ZZ
NOSUGGEST NS
SFX AB Y 1
SFX AB à xy .
SFX PL Y 1
SFX PL 0 s .
";
pub const DIC: &str = "14
català
col·legi
il·lusió
qüestió
abraçar
l'home
dóna'm
nord-est
home/PL
home-home/ZZPL
à/AB
zqprohibit/ZZ
zqocult/NS
prova
";

pub fn install() {
    super::install_dictionary(AFF, DIC).unwrap();
}
