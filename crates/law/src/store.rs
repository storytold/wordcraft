//! The profile store: the profiles WordCraft ships, the user's own files, and nothing else.
//!
//! A profile is a file, `<id>.json` in the profiles folder, so a person can hand one around and
//! an agent can write one. The store keeps the bundled profiles (Missouri and federal appellate
//! briefs, Missouri circuit court pleadings) and the user's; a user's own profile wins over a
//! bundled one of the same id. The firm details pleadings use (`firm.json`) live in the same folder.
//!
//! Saving always clears the verified flag: only a person's `law.profiles.approve` sets it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::profile::Profile;

/// Largest court profile file WordCraft reads (JSON text).
pub const MAX_PROFILE_BYTES: u64 = 2 * 1024 * 1024;

/// Errors the store reports.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum LawError {
    /// No profile with that id.
    #[error("no court profile `{0}`")]
    NotFound(String),
    /// The file couldn't be read or written.
    #[error("{0}")]
    Io(String),
    /// The profile isn't valid: every problem, one per line.
    #[error("the profile is not valid:\n  {0}")]
    Invalid(String),
    /// A WordCraft command the profile asked for refused or no longer exists.
    #[error("{0}")]
    Failed(String),
    /// The action is for the person at the keyboard, not automation.
    #[error("{0}")]
    HumanOnly(String),
}

impl LawError {
    pub fn invalid(problems: &[String]) -> Self {
        LawError::Invalid(problems.join("\n  "))
    }
}

/// Where profiles live and which ones there are.
#[derive(Debug)]
pub struct Store {
    /// The profiles this crate ships, verified.
    bundled: Vec<Profile>,
    /// The user's profiles folder; `None` keeps profiles in memory only (the web build, tests).
    dir: Option<PathBuf>,
    /// The user's profiles, by id (a folder file or one saved in this run).
    user: BTreeMap<String, Profile>,
    /// The saved firm details for pleadings (`firm.json` in the same folder), by field.
    firm: BTreeMap<String, String>,
}

impl Default for Store {
    fn default() -> Self {
        Store { bundled: bundled(), dir: None, user: BTreeMap::new(), firm: BTreeMap::new() }
    }
}

impl Store {
    /// A store over `dir` (the user's profiles folder), or one that keeps profiles in memory only.
    pub fn new(dir: Option<PathBuf>) -> Self {
        let mut store = Store { bundled: bundled(), dir, user: BTreeMap::new(), firm: BTreeMap::new() };
        store.load_dir();
        store
    }

    /// Point the store at another folder (the host does this once at start-up).
    pub fn set_dir(&mut self, dir: Option<PathBuf>) {
        if self.dir == dir {
            return;
        }
        self.dir = dir;
        self.user.clear();
        self.firm.clear();
        self.load_dir();
    }

    /// The profiles folder, when there is one.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The file a profile of this id lives in.
    pub fn file(&self, id: &str) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(format!("{id}.json")))
    }

    /// Every profile there is, by id: the bundled ones, with the user's own of the same id
    /// replacing them.
    pub fn list(&self) -> Vec<&Profile> {
        let mut out: BTreeMap<&str, &Profile> = self.bundled.iter().map(|p| (p.id.as_str(), p)).collect();
        for p in self.user.values() {
            out.insert(&p.id, p);
        }
        out.into_values().collect()
    }

    /// A profile by id (the user's own wins over a bundled one).
    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.user.get(id).or_else(|| self.bundled.iter().find(|p| p.id == id))
    }

    /// Save a profile: it is always unverified afterwards, whoever wrote it. Writes the file when
    /// the store has a folder; the profile stays in memory either way.
    pub fn save(&mut self, p: &mut Profile) -> Result<(), LawError> {
        p.verified = false;
        self.insert(p.clone())
    }

    /// Take a profile in from outside (a command, or an imported file), keeping its own verified
    /// flag when a person imports it outside automation. Writes the file when the store has a folder.
    pub fn insert(&mut self, p: Profile) -> Result<(), LawError> {
        let bad = crate::profile::problems(&p);
        if !bad.is_empty() {
            return Err(LawError::invalid(&bad));
        }
        let text = p.to_json().map_err(|e| LawError::Io(e.join("; ")))?;
        if let Some(path) = self.file(&p.id) {
            write_file(&path, &text)?;
        }
        self.user.insert(p.id.clone(), p);
        Ok(())
    }

    /// Mark a profile verified (a person's action outside automation). A bundled profile is copied
    /// to the user's folder first, so the person's approval is theirs and survives an update.
    pub fn approve(&mut self, id: &str) -> Result<Profile, LawError> {
        wordcraft_engine::file_access::require_human("approving a court profile").map_err(LawError::HumanOnly)?;
        let Some(existing) = self.get(id) else { return Err(LawError::NotFound(id.to_string())) };
        let mut approved = existing.clone();
        approved.verified = true;
        self.insert(approved.clone())?;
        Ok(approved)
    }

    /// Read a profile from a file and validate it (no side effects: nothing is saved).
    pub fn read_file(path: &Path) -> Result<Profile, LawError> {
        let text = read_file(path)?;
        Profile::from_json(&text).map_err(|e| LawError::invalid(&e))
    }

    /// Write a profile to a file, making the folder if it needs one.
    pub fn write_file(path: &Path, p: &Profile) -> Result<(), LawError> {
        let problems = crate::profile::problems(p);
        if !problems.is_empty() {
            return Err(LawError::invalid(&problems));
        }
        let text = p.to_json().map_err(|e| LawError::Io(e.join("; ")))?;
        write_file(path, &text)
    }

    /// Read `<dir>/*.json` into the store. A file that isn't a valid profile is skipped, not an
    /// error: one bad file must not stop the rest from loading.
    fn load_dir(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        paths.sort();
        // The firm details live beside the profiles but aren't one.
        if let Some(i) = paths.iter().position(|p| p.file_name().is_some_and(|n| n == FIRM_FILE)) {
            let firm = paths.remove(i);
            self.load_firm(&firm);
        }
        for path in paths {
            if let Ok(p) = Self::read_file(&path) {
                self.user.insert(p.id.clone(), p);
            } else {
                log::warn!("court profile: ignoring {} (not a valid profile)", path.display());
            }
        }
    }
}

/// The firm details file in the profiles folder.
const FIRM_FILE: &str = "firm.json";

impl Store {
    /// The saved firm details (empty when none are saved).
    pub fn firm(&self) -> &BTreeMap<String, String> {
        &self.firm
    }

    /// Replace the saved firm details. Writes `firm.json` when the store has a folder; they stay
    /// in memory either way. The caller has checked the fields ([`crate::pleadings`]).
    pub fn set_firm(&mut self, firm: BTreeMap<String, String>) -> Result<(), LawError> {
        if let Some(dir) = &self.dir {
            let text = serde_json::to_string_pretty(&serde_json::json!({"schema": 1, "firm": firm})).map_err(|e| LawError::Io(e.to_string()))?;
            write_file(&dir.join(FIRM_FILE), &text)?;
        }
        self.firm = firm;
        Ok(())
    }

    fn load_firm(&mut self, path: &Path) {
        let firm = read_file(path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| crate::pleadings::firm_fields(v.get("firm")?).ok());
        match firm {
            Some(f) => self.firm = f,
            None => log::warn!("firm details: ignoring {} (not valid firm details)", path.display()),
        }
    }
}

/// The profiles this crate ships, each one verified: a Missouri appellate brief (Rules 84.03,
/// 84.04 and 84.06), a federal one (FRAP 32) and Missouri circuit court pleadings. A bundled
/// file that doesn't parse is logged and left out, never fatal —
/// `bundled_profiles_are_valid_and_verified` keeps them honest.
pub fn bundled() -> Vec<Profile> {
    [
        include_str!("../profiles/missouri-appellate-brief.json"),
        include_str!("../profiles/federal-appellate-brief.json"),
        include_str!("../profiles/missouri-circuit-court.json"),
    ]
    .iter()
    .filter_map(|text| match Profile::from_json(text) {
        Ok(p) => Some(p),
        Err(problems) => {
            log::error!("bundled court profile is not valid: {}", problems.join("; "));
            None
        }
    })
    .collect()
}

fn path_str(path: &Path) -> Result<String, LawError> {
    path.to_str().map(str::to_string).ok_or_else(|| LawError::Io(format!("path is not UTF-8: {}", path.display())))
}

fn read_file(path: &Path) -> Result<String, LawError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = path_str(path)?;
        let bytes = wordcraft_engine::io::read_file_capped(&path, MAX_PROFILE_BYTES, "court profile file").map_err(LawError::Io)?;
        String::from_utf8(bytes).map_err(|e| LawError::Io(format!("can't read {path}: not UTF-8: {e}")))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err(LawError::Io("reading files isn't available in this build".into()))
    }
}

fn write_file(path: &Path, text: &str) -> Result<(), LawError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = path_str(path)?;
        wordcraft_engine::file_access::check_write(&path).map_err(LawError::Io)?;
        if let Some(dir) = Path::new(&path).parent()
            && !dir.as_os_str().is_empty()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            return Err(LawError::Io(format!("can't create {}: {e}", dir.display())));
        }
        wordcraft_engine::io::write_file(&path, text.as_bytes()).map_err(LawError::Io)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, text);
        Err(LawError::Io("writing files isn't available in this build".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::DocSpec;
    use wordcraft_engine::file_access::AutomationRoots;

    fn profile(id: &str) -> Profile {
        Profile {
            schema: crate::profile::SCHEMA,
            id: id.into(),
            name: format!("court {id}"),
            jurisdiction: "Test".into(),
            verified: true,
            citation_pack: None,
            docs: [("brief".to_string(), DocSpec { source: "Rule 1".into(), ..Default::default() })].into(),
        }
    }

    #[test]
    fn bundled_profiles_are_valid_and_verified() {
        let all = bundled();
        assert_eq!(all.len(), 3, "Missouri appellate, federal and Missouri circuit court");
        for p in &all {
            assert!(p.verified, "{} ships verified", p.id);
            assert!(crate::profile::problems(p).is_empty(), "{}: {:?}", p.id, crate::profile::problems(p));
        }
    }

    #[test]
    fn saving_clears_the_verified_flag() {
        let mut store = Store::new(None);
        let mut p = profile("mine");
        p.verified = true;
        store.save(&mut p).expect("save");
        assert!(!p.verified, "an agent-created profile is unverified");
        assert!(!store.get("mine").map(|p| p.verified).unwrap_or(true));
    }

    #[test]
    fn approving_verifies_a_profile() {
        let mut store = Store::new(None);
        let mut p = profile("mine");
        store.save(&mut p).expect("save");
        assert!(!store.get("mine").map(|p| p.verified).unwrap_or(true));
        let approved = store.approve("mine").expect("approve");
        assert!(approved.verified);
        assert!(store.get("mine").map(|p| p.verified).unwrap_or(false));
    }

    #[test]
    fn approving_is_refused_under_automation() {
        let mut store = Store::new(None);
        let mut p = profile("mine");
        store.save(&mut p).expect("save");
        let err = wordcraft_engine::file_access::invoke_automation(|| store.approve("mine")).expect_err("automation");
        assert!(matches!(err, LawError::HumanOnly(_)), "{err}");
    }

    #[test]
    fn approving_is_refused_when_confined_to_automation_roots() {
        let base = std::env::temp_dir().join(format!("wordcraft-law-approve-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("in")).expect("dir");
        let roots = AutomationRoots::new(Some(base.join("in").as_path()), None).expect("roots").expect("roots");
        let mut store = Store::new(None);
        let mut p = profile("mine");
        store.save(&mut p).expect("save");
        let err = wordcraft_engine::file_access::confine(Some(&roots), || store.approve("mine")).expect_err("confined");
        assert!(matches!(err, LawError::HumanOnly(_)), "{err}");
        assert!(!store.get("mine").map(|p| p.verified).unwrap_or(true), "approval must not stick");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_user_profile_wins_over_a_bundled_one() {
        let mut store = Store::new(None);
        let mut own = bundled().first().expect("a bundled profile").clone();
        own.name = "My version".into();
        store.save(&mut own).expect("save");
        let found = store.get(&own.id).expect("the profile");
        assert_eq!(found.name, "My version");
        assert!(!found.verified, "the copy is unverified again");
        // Both are still listed, once each.
        let ids: Vec<&str> = store.list().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.iter().filter(|i| **i == own.id).count(), 1);
    }

    #[test]
    fn a_folder_is_read_and_written() {
        let dir = std::env::temp_dir().join(format!("wordcraft-law-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::new(Some(dir.clone()));
        assert_eq!(store.list().len(), bundled().len());
        let mut p = profile("mine");
        store.save(&mut p).expect("save");
        assert!(dir.join("mine.json").exists(), "the profile is a file");
        // A second store over the same folder reads it back, still unverified.
        let reopen = Store::new(Some(dir.clone()));
        assert!(!reopen.get("mine").map(|p| p.verified).unwrap_or(true));
        // A file that isn't a profile is skipped, not an error.
        std::fs::write(dir.join("broken.json"), "{not json").expect("write a broken file");
        assert!(Store::new(Some(dir.clone())).get("mine").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reading_a_missing_file_is_an_error_not_a_panic() {
        let err = Store::read_file(Path::new("/nonexistent-wordcraft-law/profile.json")).expect_err("no such file");
        assert!(matches!(err, LawError::Io(_)), "{err}");
        assert!(matches!(store_get_missing(), LawError::NotFound(_)));
    }

    #[test]
    fn oversized_profile_files_are_refused() {
        let dir = std::env::temp_dir().join(format!("wordcraft-law-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("big.json");
        let pad = "x".repeat((MAX_PROFILE_BYTES as usize) + 1);
        std::fs::write(&path, pad).expect("write");
        let err = Store::read_file(&path).expect_err("too large");
        assert!(matches!(err, LawError::Io(_)), "{err}");
        assert!(err.to_string().contains("larger than"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_access_roots_apply_to_profile_io() {
        let base = std::env::temp_dir().join(format!("wordcraft-law-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["in", "out"] {
            std::fs::create_dir_all(base.join(d)).expect("dir");
        }
        let inside = base.join("in/ok.json");
        std::fs::write(&inside, r#"{"schema":1,"id":"x","name":"X","jurisdiction":"Y","docs":{"brief":{"source":"r","styles":[{"name":"Normal"}],"sections":[{"id":"a","name":"A"}]}}}"#).expect("write");
        let outside_read = base.join("outside/secret.json");
        let outside_write = base.join("outside/evil.json");
        let roots = AutomationRoots::new(Some(base.join("in").as_path()), Some(base.join("out").as_path())).expect("roots").expect("roots");
        wordcraft_engine::file_access::confine(Some(&roots), || {
            assert!(Store::read_file(&inside).is_ok());
            let err = Store::read_file(outside_read.as_path()).expect_err("outside read root");
            assert!(err.to_string().contains("outside the read root"), "{err}");
            let p = profile("saved");
            let outside_nested = base.join("outside/nested/evil.json");
            assert!(!base.join("outside/nested").exists(), "parent must not exist yet");
            let err = Store::write_file(outside_nested.as_path(), &p).expect_err("outside write root");
            assert!(err.to_string().contains("outside the write root"), "{err}");
            assert!(!base.join("outside/nested").exists(), "denied export must not create directories");
            let err = Store::write_file(outside_write.as_path(), &p).expect_err("outside write root");
            assert!(err.to_string().contains("outside the write root"), "{err}");
            let ok_path = base.join("out/saved.json");
            Store::write_file(&ok_path, &p).expect("inside write root");
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    fn store_get_missing() -> LawError {
        let mut store = Store::new(None);
        match store.approve("nope") {
            Ok(_) => LawError::NotFound("nope".into()),
            Err(e) => e,
        }
    }
}
