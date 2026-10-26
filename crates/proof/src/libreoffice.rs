//! Spelling dictionaries from LibreOffice's dictionary collection
//! (<https://github.com/LibreOffice/dictionaries>), downloaded on request into WordCraft's
//! dictionaries folder ([`crate::hunspell::download_dir`]): the one the user chose in File ›
//! Options › Proofing, else WordCraft's own.
//!
//! WordCraft doesn't bundle these files: most are GPL, LGPL or MPL, and WordCraft is MIT OR
//! Apache-2.0. Downloaded at the user's request, a dictionary stays a separate data file under
//! its own licence that WordCraft reads at runtime, like one installed by the system or by
//! LibreOffice. This module only knows where each language's pair lives and checks and saves
//! what was downloaded; the download itself belongs to the front end (the web has no folder).
//!
//! The collection has a folder for some 70 languages (`it_IT`, `pl_PL`, `pt_BR`…), each with a
//! `<name>.aff`/`<name>.dic` pair and a README giving its licence. [`source`] maps the languages
//! WordCraft proofs today; a language that gets a [`ProofLang`] only needs a row here.
//!
//! Downloads are pinned: every URL names one commit of the collection ([`COMMIT`]), and every
//! file must match the SHA-256 recorded for it here before it is parsed or saved. A change
//! upstream, a different file served at that path, or a truncated or tampered download is refused.
//! Moving to a newer dictionary release means updating [`COMMIT`] and the hashes together.

use std::path::{Path, PathBuf};

use crate::ProofLang;

/// The collection's raw-file root.
pub const BASE_URL: &str = "https://raw.githubusercontent.com/LibreOffice/dictionaries";

/// The commit of the collection every download comes from (2026-10-12).
pub const COMMIT: &str = "67032ee957631908098a72edf48dc81cac27e369";

/// Largest `.aff` or `.dic` file accepted: what [`crate::hunspell::load`] reads (Turkish's
/// `.dic`, the largest, is ~36 MB).
pub const MAX_FILE: usize = crate::hunspell::MAX_FILE as usize;

/// Where a language's dictionary lives in the collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    /// The folder in the collection (`it_IT`).
    pub folder: &'static str,
    /// The file name without extension (`it_IT`): the pair is `<name>.aff` and `<name>.dic`,
    /// saved under the same names so [`crate::hunspell::find`] picks them up.
    pub name: &'static str,
    /// The SHA-256 of the `.aff` file at [`COMMIT`], lower-case hex.
    pub aff_sha256: &'static str,
    /// The SHA-256 of the `.dic` file at [`COMMIT`], lower-case hex.
    pub dic_sha256: &'static str,
}

impl Source {
    /// The `.aff` URL.
    pub fn aff_url(&self) -> String {
        format!("{BASE_URL}/{COMMIT}/{}/{}.aff", self.folder, self.name)
    }

    /// The `.dic` URL.
    pub fn dic_url(&self) -> String {
        format!("{BASE_URL}/{COMMIT}/{}/{}.dic", self.folder, self.name)
    }

    /// The folder's page, with the README that gives the dictionary's authors and licence.
    pub fn page_url(&self) -> String {
        format!("https://github.com/LibreOffice/dictionaries/tree/{COMMIT}/{}", self.folder)
    }
}

/// Where `lang`'s dictionary is in the collection, with its files' hashes at [`COMMIT`]; `None`
/// for English (bundled), for languages WordCraft can't proof yet, and for Croatian, Turkish and
/// Indonesian, whose collection dictionaries use two-character and numeric affix flags (`FLAG
/// long`, `FLAG num`) that [`crate::affix`] doesn't read yet: [`install`] would refuse them. The
/// Spanish varieties aren't offered yet either: each looks for its own file names (`es_419`,
/// `es_UY`…), which the collection's one `es` folder doesn't map onto one to one.
pub fn source(lang: ProofLang) -> Option<Source> {
    let (folder, name, aff_sha256, dic_sha256) = match lang {
        ProofLang::Pl => (
            "pl_PL",
            "pl_PL",
            "82973651651aa930335c865b339b98db376ca3dbf3a661b70b9eeb71fdf41dca",
            "c0848440599eb88e5aca500418d5f389e562ec2c157b63dbe39d354658ffba49",
        ),
        ProofLang::Pt => (
            "pt_BR",
            "pt_BR",
            "21d8ad2a769a60e17e2b5ea4ef11d4d593a58b9e2a82d642ef82d6a4c5523865",
            "a38bfb26b68ece2834e79fe83e48d5792652970ace12db89d1b9674bf9933183",
        ),
        ProofLang::It => (
            "it_IT",
            "it_IT",
            "951afaa19272f13555b8823e8bcf9ccf78f8fe1a07835bdfb912ab3e4d537c2b",
            "bae1e3501dcd2a923669592493b3fde6c02aae7c7aab83bf5e5b49077e73dd64",
        ),
        ProofLang::Ru => (
            "ru_RU",
            "ru_RU",
            "38ce7d4af78e211e9bafe4bf7e3d6a2c420591136cb738ec6648f8fdf6524cd7",
            "f6047416a0204adbecf3a451b874ec8a97ee37e2cbc714466ef04d8dbcc0d6fc",
        ),
        ProofLang::Cs => (
            "cs_CZ",
            "cs_CZ",
            "7ecb20620ecd46ebd9c36f3f33e69dd4eda385cba5b2bb4e6bc396d910e297f7",
            "d8e8c88c006fdae72dac8c85df11b0c99a773e05a4ab0fcbe92244876668ca74",
        ),
        ProofLang::En
        | ProofLang::EnGb
        | ProofLang::EnCa
        | ProofLang::Hr
        | ProofLang::Tr
        | ProofLang::Id
        | ProofLang::Bg
        | ProofLang::Es
        | ProofLang::EsMx
        | ProofLang::Es419
        | ProofLang::EsUy
        | ProofLang::De
        | ProofLang::Other => {
            return None;
        }
    };
    Some(Source { folder, name, aff_sha256, dic_sha256 })
}

/// The lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The languages a dictionary can be downloaded for, in display order.
pub fn languages() -> impl Iterator<Item = ProofLang> {
    ProofLang::PROOFED.into_iter().filter(|l| source(*l).is_some())
}

/// Check a downloaded pair and save it in `dir` as `<name>.aff`/`<name>.dic`, replacing an
/// earlier download. Each file must match its SHA-256 in `source`, and the pair must decode and
/// parse as a non-empty Hunspell dictionary; each
/// file is written next to its final name and renamed into place, so a failed or partial
/// download never leaves a broken pair behind. Returns the `.dic` path.
pub fn install(dir: &Path, source: &Source, aff: &[u8], dic: &[u8]) -> Result<PathBuf, String> {
    for (bytes, what) in [(aff, ".aff"), (dic, ".dic")] {
        if bytes.is_empty() {
            return Err(format!("the downloaded {what} file is empty"));
        }
        if bytes.len() > MAX_FILE {
            return Err(format!("the downloaded {what} file is too large ({} bytes)", bytes.len()));
        }
    }
    for (bytes, expected, what) in [(aff, source.aff_sha256, ".aff"), (dic, source.dic_sha256, ".dic")] {
        if !sha256_hex(bytes).eq_ignore_ascii_case(expected) {
            return Err(format!("the downloaded {what} file doesn't match the expected checksum; nothing was saved"));
        }
    }
    let (aff_text, dic_text) = crate::hunspell::decode(aff, dic)?;
    let parsed = crate::affix::AffixDict::from_hunspell(&aff_text, &dic_text, &[])?;
    if parsed.is_empty() {
        return Err("the downloaded dictionary has no words".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let aff_path = dir.join(format!("{}.aff", source.name));
    let dic_path = dir.join(format!("{}.dic", source.name));
    let part = |path: &Path| {
        let mut p = path.as_os_str().to_owned();
        p.push(".part");
        PathBuf::from(p)
    };
    let (aff_part, dic_part) = (part(&aff_path), part(&dic_path));
    let clean = || {
        let _ = std::fs::remove_file(&aff_part);
        let _ = std::fs::remove_file(&dic_part);
    };
    // Both files are written before either is renamed, so a failed write (a full disk) leaves
    // an earlier pair untouched.
    for (bytes, p) in [(aff, &aff_part), (dic, &dic_part)] {
        if let Err(e) = std::fs::write(p, bytes) {
            clean();
            return Err(format!("{}: {e}", p.display()));
        }
    }
    for (p, path) in [(&aff_part, &aff_path), (&dic_part, &dic_path)] {
        if let Err(e) = std::fs::rename(p, path) {
            clean();
            return Err(format!("{}: {e}", path.display()));
        }
    }
    Ok(dic_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_languages_whose_collection_dictionary_loads_have_a_source() {
        for lang in ProofLang::PROOFED {
            let expected =
                !matches!(lang, ProofLang::En | ProofLang::EnGb | ProofLang::EnCa | ProofLang::Hr | ProofLang::Tr | ProofLang::Id | ProofLang::De)
                    && !lang.is_spanish();
            assert_eq!(source(lang).is_some(), expected, "{lang:?}");
        }
        assert_eq!(source(ProofLang::Other), None);
        let it = source(ProofLang::It).unwrap();
        assert_eq!(it.aff_url(), format!("https://raw.githubusercontent.com/LibreOffice/dictionaries/{COMMIT}/it_IT/it_IT.aff"));
        assert_eq!(it.dic_url(), format!("https://raw.githubusercontent.com/LibreOffice/dictionaries/{COMMIT}/it_IT/it_IT.dic"));
        assert_eq!(COMMIT.len(), 40, "a full commit id, not a branch");
        for lang in languages() {
            let s = source(lang).unwrap();
            for h in [s.aff_sha256, s.dic_sha256] {
                assert!(h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "{lang:?}: {h}");
            }
        }
        let listed: Vec<ProofLang> = crate::installed_dictionaries().into_iter().map(|(l, _)| l).collect();
        assert!(languages().all(|l| listed.contains(&l)), "every downloadable language is in File › Options › Proofing");
        assert_eq!(languages().collect::<Vec<_>>(), [ProofLang::Pl, ProofLang::Pt, ProofLang::It, ProofLang::Ru, ProofLang::Cs]);
    }

    #[test]
    fn install_saves_a_valid_pair_and_refuses_a_broken_one() {
        let dir = std::env::temp_dir().join(format!("wordcraft-lo-dict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let aff: &[u8] = b"SET UTF-8\nSFX A Y 1\nSFX A o i o\n";
        let good: &[u8] = b"2\ngatto/A\ncane\n";
        // A source whose hashes are those of the test files (the real ones are LibreOffice's).
        let pinned = |aff: &[u8], dic: &[u8]| Source {
            aff_sha256: Box::leak(sha256_hex(aff).into_boxed_str()),
            dic_sha256: Box::leak(sha256_hex(dic).into_boxed_str()),
            ..source(ProofLang::It).unwrap()
        };
        assert!(install(&dir, &pinned(aff, b""), aff, b"").is_err(), "empty .dic");
        assert!(install(&dir, &pinned(aff, b"0\n"), aff, b"0\n").is_err(), "no words");
        assert!(install(&dir, &pinned(b"SET EBCDIC\n", good), b"SET EBCDIC\n", good).is_err(), "unknown encoding");
        // A valid pair that isn't the pinned file: the real Italian hashes don't match it.
        let e = install(&dir, &source(ProofLang::It).unwrap(), aff, good).unwrap_err();
        assert!(e.contains("checksum"), "{e}");
        let mut tampered = good.to_vec();
        tampered.push(b'x');
        assert!(install(&dir, &pinned(aff, good), aff, &tampered).unwrap_err().contains("checksum"), "one byte more");
        assert!(!dir.join("it_IT.dic").exists(), "nothing saved after a refused download");
        let dic = install(&dir, &pinned(aff, good), aff, good).unwrap();
        assert_eq!(dic, dir.join("it_IT.dic"));
        assert_eq!(crate::hunspell::find_in(&dir, &["it_IT"]), Some((dir.join("it_IT.aff"), dic.clone())));
        let d = crate::hunspell::load(&dir.join("it_IT.aff"), &dic).unwrap();
        assert!(d.check("gatti") && d.check("cane") && !d.check("gatte"));
        assert!(!dir.join("it_IT.aff.part").exists() && !dir.join("it_IT.dic.part").exists(), "no leftovers");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
