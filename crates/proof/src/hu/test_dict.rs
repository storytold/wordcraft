//! Hand-written synthetic Hungarian test data. No words, rules or suggestion tables were copied
//! from a system dictionary. It uses what Magyar Ispell needs and [`crate::affix`] doesn't read:
//! flag aliases (`AF`) and compounding (`COMPOUNDFLAG`).

pub const AFF: &str = "SET UTF-8
LANG hu_HU
TRY aáeéiíoóöőuúüűtnlkrszmgvbhdpjfcyxwq
KEY öüó|qwertzuiopőú|asdfghjkléáű|íyxcvbnm
MAP 4
MAP aá
MAP eé
MAP ií
MAP oóöő
COMPOUNDFLAG Y
COMPOUNDMIN 2
FORBIDDENWORD W
AF 4
AF AY # 1: back-vowel noun, may compound
AF BY # 2: front-vowel noun, may compound
AF Y # 3: compounds only as written
AF W # 4: forbidden
SFX A Y 2
SFX A 0 ban .
SFX A 0 ak .
SFX B Y 2
SFX B 0 ben .
SFX B 0 ek .
";

pub const DIC: &str = "8
ház/1
kert/2
alma/3
fa/3
helyesírás
ellenőrző
iskola
zqtiltott/4
";

pub fn install() {
    super::install_dictionary(AFF, DIC).unwrap();
}
