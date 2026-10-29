//! A tiny synthetic Russian Hunspell dictionary for tests (written for these tests; not taken from
//! any real dictionary). Real dictionaries are installed on the system, never bundled.

/// Affix rules: the endings of a hard masculine noun, a feminine noun in `-а`, hard and soft
/// adjectives and a verb in `-ать`, plus the letters suggestions try.
pub const AFF: &str = "SET UTF-8
TRY оеаинтсрвлкмдпуяыьгзбчйхжшюцщэфъё
SFX A Y 9
SFX A 0 а .
SFX A 0 у .
SFX A 0 ом .
SFX A 0 е .
SFX A 0 ы .
SFX A 0 ов .
SFX A 0 ам .
SFX A 0 ами .
SFX A 0 ах .
SFX D Y 5
SFX D а ы а
SFX D а е а
SFX D а у а
SFX D а ой а
SFX D а ою а
SFX B Y 11
SFX B ый ого ый
SFX B ый ому ый
SFX B ый ым ый
SFX B ый ом ый
SFX B ый ая ый
SFX B ый ой ый
SFX B ый ую ый
SFX B ый ое ый
SFX B ый ые ый
SFX B ый ых ый
SFX B ый ыми ый
SFX E Y 8
SFX E ий его ий
SFX E ий ему ий
SFX E ий им ий
SFX E ий ей ий
SFX E ий яя ий
SFX E ий ее ий
SFX E ий ие ий
SFX E ий их ий
SFX C Y 11
SFX C ть ю ть
SFX C ть ешь ть
SFX C ть ет ть
SFX C ть ем ть
SFX C ть ете ть
SFX C ть ют ть
SFX C ть ются ть
SFX C ть ется ть
SFX C ть л ть
SFX C ть ла ть
SFX C ть ться ть
";

/// Stems (the count line first, as in Hunspell files).
pub const DIC: &str = "62
привет
здесь
здравствуй
жить
часто
знаю
знать/C
не
в
общем
общий/E
дом/A
стол/A
город/A
Москва/D
книга/D
красный/B
новый/B
белый/B
западный/B
синий/E
зелёный/B
русский
английский
читать/C
работать/C
кто
где
что
давай
всё
все
ещё
ёлка/D
север/A
из
за
на
и
а
но
или
он
она
я
ты
мы
вы
это
так
как
когда
который/B
чтобы
потому
если
пришёл
сказал
вскрытый/B
высоко
эластичный/B
консультант/A
";

/// Install the test dictionary as the Russian one.
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
