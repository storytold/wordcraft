//! A tiny synthetic Turkish Hunspell dictionary for tests (written for these tests; not taken from
//! any real dictionary). Real dictionaries are installed on the system, never bundled.

/// No affix rules: the forms the tests look up are listed whole.
pub const AFF: &str = "SET UTF-8\n";

/// The words the Turkish tests expect to be known.
pub const DIC: &str = "7
kitap
kitaplar
ev
evde
olacaklardan
sonra
geldin
";

pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
