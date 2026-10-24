//! The installed Hunspell dictionary of a proofing language, in one shape for every language:
//! found on first use, replaced with [`DictSlot::install`] and forgotten with [`DictSlot::reload`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::affix::AffixDict;

/// A dictionary a language reads: the affix engine's ([`AffixDict`]) or Turkish's own.
pub(crate) trait Dictionary: Default + Send + Sync {
    /// The installed `.aff` and `.dic` files, read.
    fn load(aff: &Path, dic: &Path) -> Result<Self, String>;
    /// Hunspell `.aff` and `.dic` text (decoded to UTF-8), parsed.
    fn parse(aff: &str, dic: &str) -> Result<Self, String>;
    /// No words: the language has no spelling marks.
    fn is_empty(&self) -> bool;
}

impl Dictionary for AffixDict {
    fn load(aff: &Path, dic: &Path) -> Result<Self, String> {
        crate::hunspell::load(aff, dic)
    }

    fn parse(aff: &str, dic: &str) -> Result<Self, String> {
        AffixDict::from_hunspell(aff, dic, &[])
    }

    fn is_empty(&self) -> bool {
        AffixDict::is_empty(self)
    }
}

/// What every language's slot answers, whatever its dictionary type (the language table in
/// lib.rs holds them side by side).
pub(crate) trait Slot: Sync {
    /// See [`DictSlot::name`].
    fn name(&self) -> &'static str;
    /// See [`DictSlot::installed_path`].
    fn installed_path(&self) -> Option<PathBuf>;
    /// See [`DictSlot::has_dictionary`].
    fn has_dictionary(&self) -> bool;
    /// See [`DictSlot::reload`].
    fn reload(&self);
}

impl<D: Dictionary> Slot for DictSlot<D> {
    fn name(&self) -> &'static str {
        DictSlot::name(self)
    }

    fn installed_path(&self) -> Option<PathBuf> {
        DictSlot::installed_path(self)
    }

    fn has_dictionary(&self) -> bool {
        DictSlot::has_dictionary(self)
    }

    fn reload(&self) {
        DictSlot::reload(self);
    }
}

/// The dictionary in use and whether it came from [`DictSlot::install`] rather than a file on
/// disk (it then has no file to report, even if a matching file also happens to exist): one lock,
/// so a reload and an install can't interleave their two fields.
struct State<D> {
    dict: Option<Arc<D>>,
    in_memory: bool,
}

/// One language's dictionary.
pub(crate) struct DictSlot<D> {
    /// File names the dictionary may have, the usual one first (also the key of its load error).
    names: &'static [&'static str],
    /// The language's English name, for the log.
    english: &'static str,
    state: RwLock<State<D>>,
}

impl<D: Dictionary> DictSlot<D> {
    pub(crate) const fn new(names: &'static [&'static str], english: &'static str) -> Self {
        DictSlot { names, english, state: RwLock::new(State { dict: None, in_memory: false }) }
    }

    fn read(&self) -> RwLockReadGuard<'_, State<D>> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, State<D>> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The Hunspell name the dictionary is known by (`pl_PL`): the first of its file names.
    pub(crate) fn name(&self) -> &'static str {
        self.names.first().copied().unwrap_or_default()
    }

    /// The file names the dictionary may have, the usual one first.
    #[cfg(test)]
    pub(crate) fn names(&self) -> &'static [&'static str] {
        self.names
    }

    /// The dictionary: the installed one (read on first use), one given with [`Self::install`], or
    /// an empty one when none is available (the language's spelling is off then).
    pub(crate) fn get(&self) -> Arc<D> {
        if let Some(d) = self.read().dict.as_ref() {
            return d.clone();
        }
        let loaded = Arc::new(self.load_installed());
        // Another thread (or an install) may have filled it meanwhile: that one wins.
        self.write().dict.get_or_insert(loaded).clone()
    }

    fn load_installed(&self) -> D {
        let name = self.name();
        let Some((aff, dic)) = crate::hunspell::find(self.names) else {
            crate::hunspell::clear_load_error(name);
            log::info!("{} spelling: no Hunspell {name} dictionary installed", self.english);
            return D::default();
        };
        crate::hunspell::note_load(name, &dic, D::load(&aff, &dic)).unwrap_or_else(|e| {
            log::warn!("{} spelling: {e}", self.english);
            D::default()
        })
    }

    /// Use this dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8), for
    /// example one the user picked. On error the current dictionary stays.
    pub(crate) fn install(&self, aff: &str, dic: &str) -> Result<(), String> {
        let d = D::parse(aff, dic)?;
        let mut slot = self.write();
        slot.dict = Some(Arc::new(d));
        slot.in_memory = true;
        Ok(())
    }

    /// Forget the dictionary, so the next check looks for an installed one again (after
    /// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
    pub(crate) fn reload(&self) {
        let mut slot = self.write();
        slot.dict = None;
        slot.in_memory = false;
    }

    /// The `.dic` file of the installed dictionary, searching the dictionaries folder and the
    /// system locations now (File › Options › Proofing shows it); `None` when there is none, or
    /// when the dictionary in use was given with [`Self::install`] and so has no file.
    pub(crate) fn installed_path(&self) -> Option<PathBuf> {
        if self.read().in_memory {
            return None;
        }
        crate::hunspell::find(self.names).map(|(_, dic)| dic)
    }

    /// Is a dictionary available (spelling is checked)?
    pub(crate) fn has_dictionary(&self) -> bool {
        !self.get().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SLOT: DictSlot<AffixDict> = DictSlot::new(&["zz_SLOT_TEST"], "Test");

    #[test]
    fn an_installed_dictionary_has_no_file_until_reloaded() {
        assert!(!SLOT.has_dictionary(), "nothing installed under this name");
        assert!(SLOT.install("FLAG long\n", "1\nkot\n").is_err(), "unsupported files are refused");
        assert!(!SLOT.has_dictionary(), "a refused install keeps the current one");
        SLOT.install("", "1\nkot\n").unwrap();
        assert!(SLOT.has_dictionary() && SLOT.get().check("kot"));
        assert_eq!(SLOT.installed_path(), None);
        SLOT.reload();
        assert!(!SLOT.has_dictionary(), "reloaded from disk, where there is none");
        assert_eq!(SLOT.name(), "zz_SLOT_TEST");
    }
}
