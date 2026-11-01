//! Language packs as commands (CP4.3): list, install and remove, for the CLI, MCP and the control
//! channel. They open nothing and never turn downloading on: an install needs the font store's
//! consent setting, which only a person in the app turns on (File › Options › Language).
//! See [`crate::packs`].

use serde_json::{Value, json};

use crate::packs;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("proof.packs", "Language Packs", "File › Options › Language", list)
            .params(r#"{"refresh"?: bool (fetch the pinned index again; only while downloading is on; in the app it runs in the background: see "refreshing")} — opens nothing. Returns {"packs": [{"locale", "name", "size" (bytes), "licence" (SPDX), "class", "source" (where the data comes from), "archive" (the download), "spelling", "hyphenation", "installed"}], "installed": [{"locale", "name", "licence"}], "installing": [{"locale", "done", "total"} (bytes)], "refreshing": bool, "errors": {locale or "" (the list): message} (the last failure of each), "index_override": string|null (a test index in use instead of the pinned one), "network": bool (downloading on), "available": bool (a font store to install into)}"#)
            .pure(),
        CommandSpec::new("proof.installPack", "Install Language Pack", "File › Options › Language", install)
            .params(r#"{"locale": string (a pack's locale from proof.packs, like nl or pt-pt)} — downloads, verifies (sha256) and installs it; spelling and hyphenation in that language use it at once. Needs downloading on (only the person in the app can turn it on). Opens nothing. In the app it runs in the background and returns {"started": true}: follow it with proof.packs ("installing", then "installed" or "errors"); elsewhere it returns when installed."#)
            .pure()
            .when(available),
        CommandSpec::new("proof.removePack", "Remove Language Pack", "File › Options › Language", remove)
            .params(r#"{"locale": string} — removes the installed pack. Opens nothing."#)
            .pure()
            .when(available),
    ]
}

fn available(_: &Session) -> Option<&'static str> {
    if cfg!(target_arch = "wasm32") { Some("Language packs aren't available in the browser") } else { None }
}

fn locale_param(v: &Value) -> Result<String, CmdError> {
    let raw = p::str(v, "locale").ok_or_else(|| CmdError::Params("`locale` (a language tag like nl) is required".into()))?;
    packs::parse_locale(raw)
}

fn list(_: &mut Session, v: &Value) -> CmdResult {
    if p::bool(v, "refresh") == Some(true) {
        packs::start_refresh().map_err(CmdError::Failed)?;
        if let Some(e) = packs::error("") {
            return Err(CmdError::Failed(e));
        }
    }
    let index = packs::available();
    let installed = packs::installed();
    let backend = packs::backend();
    let rows: Vec<Value> = index
        .iter()
        .map(|p| {
            json!({
                "locale": p.locale, "name": p.name, "size": p.size, "licence": p.licence, "class": p.class,
                "source": p.source_link(), "archive": p.url, "spelling": p.spelling, "hyphenation": p.hyphenation,
                "installed": installed.iter().any(|i| i.locale == p.locale),
            })
        })
        .collect();
    Ok(json!({
        "packs": rows,
        "installed": installed.iter().map(|i| json!({"locale": i.locale, "name": i.name, "licence": i.licence})).collect::<Vec<_>>(),
        "installing": packs::installing(),
        "refreshing": packs::fetching(),
        "errors": packs::errors().into_iter().map(|(l, e)| (l, Value::String(e))).collect::<serde_json::Map<String, Value>>(),
        "network": backend.as_ref().is_some_and(|b| b.network_on()),
        // The packs come from a test index (`CRAFT_LANGUAGE_PACK_INDEX`), not the pinned one.
        "index_override": packs::index_override(),
        "available": backend.is_some(),
    }))
}

fn install(s: &mut Session, v: &Value) -> CmdResult {
    let locale = locale_param(v)?;
    if packs::is_installed(&locale) {
        return Ok(json!({"locale": locale, "installed": true, "already": true}));
    }
    if packs::start_install(&locale).map_err(CmdError::Failed)? {
        return Ok(json!({"locale": locale, "started": true}));
    }
    let done =
        packs::installed().into_iter().find(|p| p.locale == locale).ok_or_else(|| CmdError::Failed(format!("the {locale} pack didn't install")))?;
    // Spelling and hyphenation in that language change.
    s.relayout();
    Ok(json!({"locale": done.locale, "name": done.name, "licence": done.licence, "installed": true}))
}

fn remove(s: &mut Session, v: &Value) -> CmdResult {
    let locale = locale_param(v)?;
    packs::remove(&locale).map_err(CmdError::Failed)?;
    s.relayout();
    Ok(json!({"locale": locale, "installed": false}))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::packs::{Backend, FixedBackend};

    /// Tests that set the process-wide backend take turns.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Puts the process back to no backend and no background thread when dropped, even when
    /// the test panicked.
    pub(crate) struct Reset;

    impl Drop for Reset {
        fn drop(&mut self) {
            packs::set_background(None);
            packs::set_backend(None);
        }
    }

    fn with_backend<T>(b: FixedBackend, f: impl FnOnce(&mut Session, &FixedBackend) -> T) -> T {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = Reset;
        let b = Arc::new(b);
        packs::set_backend(Some(b.clone() as Arc<dyn Backend>));
        let mut s = Session::new(crate::sample::sample_document());
        f(&mut s, &b)
    }

    #[test]
    fn lists_installs_and_removes_without_opening_anything() {
        with_backend(FixedBackend::sample(), |s, b| {
            let r = s.run("proof.packs", &json!({})).unwrap();
            assert_eq!(r["packs"].as_array().unwrap().len(), 6);
            assert_eq!(r["network"], false);
            assert_eq!(r["index_override"], Value::Null, "the pinned index");
            let nl = r["packs"].as_array().unwrap().iter().find(|p| p["locale"] == "nl").unwrap();
            assert_eq!(nl["licence"], "BSD-3-Clause");
            assert_eq!(nl["installed"], false);
            // Downloading is off: an agent can't install, and can't turn it on.
            let e = s.run("proof.installPack", &json!({"locale": "nl"})).unwrap_err();
            assert!(e.to_string().contains("downloading is turned off"), "{e}");
            assert!(packs::installed().is_empty());
            b.network.store(true, Ordering::Relaxed);
            let r = s.run("proof.installPack", &json!({"locale": "NL"})).unwrap();
            assert_eq!(r["locale"], "nl");
            assert!(packs::is_installed("nl"));
            let again = s.run("proof.installPack", &json!({"locale": "nl"})).unwrap();
            assert_eq!(again["already"], true);
            let r = s.run("proof.packs", &json!({})).unwrap();
            assert_eq!(r["installed"][0]["locale"], "nl");
            s.run("proof.removePack", &json!({"locale": "nl"})).unwrap();
            assert!(!packs::is_installed("nl"));
            assert!(s.run("proof.removePack", &json!({"locale": "nl"})).is_err(), "not installed");
            assert!(s.ui_requests.is_empty(), "no dialog opens");
        });
    }

    #[test]
    fn unknown_and_hostile_locales_are_errors() {
        with_backend(FixedBackend::sample(), |s, b| {
            b.network.store(true, Ordering::Relaxed);
            for bad in [json!({}), json!({"locale": 5}), json!({"locale": "../../etc"}), json!({"locale": "x".repeat(10_000)}), json!({"locale": ""})]
            {
                assert!(s.run("proof.installPack", &bad).is_err(), "{bad}");
                assert!(s.run("proof.removePack", &bad).is_err(), "{bad}");
            }
            let e = s.run("proof.installPack", &json!({"locale": "tlh"})).unwrap_err();
            assert!(e.to_string().contains("no language pack for tlh"), "{e}");
            assert!(packs::installed().is_empty());
        });
    }

    #[test]
    fn an_agent_backend_cannot_turn_downloading_on() {
        let b = FixedBackend { agent: true, ..FixedBackend::sample() };
        assert!(b.enable_network().is_err());
        assert!(!b.network_on());
    }

    #[test]
    fn without_a_store_nothing_is_available() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = Reset;
        packs::set_backend(None);
        let mut s = Session::new(crate::sample::sample_document());
        let r = s.run("proof.packs", &json!({})).unwrap();
        assert_eq!(r["packs"], json!([]));
        assert_eq!(r["available"], false);
        assert!(s.run("proof.installPack", &json!({"locale": "nl"})).is_err());
        assert!(s.run("proof.packs", &json!({"refresh": true})).is_err());
    }

    #[test]
    fn the_bar_offers_only_languages_with_a_pack() {
        with_backend(FixedBackend::sample(), |s, b| {
            s.run("document.setText", &json!({"text": "Hallo wereld\nDzień dobry\nGuten Tag\nHello\nWitam"})).unwrap();
            assert!(packs::missing(s).is_empty(), "all English");
            for (para, lang) in [(0, "nl-NL"), (1, "pl-PL"), (2, "de-DE"), (4, "pl")] {
                s.run("caret.set", &json!({"pos": {"block": para, "off": 0}})).unwrap();
                s.run("select.paragraph", &json!({})).unwrap();
                s.run("review.language", &json!({"lang": lang})).unwrap();
            }
            let langs = packs::document_languages(s);
            assert!(langs.iter().any(|l| l == "nl-NL"), "{langs:?}");
            let missing: Vec<String> = packs::missing(s).into_iter().map(|p| p.locale).collect();
            // German has no pack: never offered.
            assert_eq!(missing, ["nl", "pl"]);
            b.network.store(true, Ordering::Relaxed);
            s.run("proof.installPack", &json!({"locale": "pl"})).unwrap();
            let missing: Vec<String> = packs::missing(s).into_iter().map(|p| p.locale).collect();
            assert_eq!(missing, ["nl"]);
            // Text marked "no language" (`zxx`) never asks for a pack.
            assert!(!packs::proofable("zxx"));
        });
    }

    #[test]
    fn tags_map_to_the_most_specific_pack() {
        let mut packs = FixedBackend::sample().packs;
        packs.push(Pack { locale: "pt-pt".into(), ..packs[0].clone() });
        packs.push(Pack { locale: "pt".into(), ..packs[0].clone() });
        assert_eq!(packs::pack_for_tag("pt-PT", &packs).map(|p| p.locale.as_str()), Some("pt-pt"));
        assert_eq!(packs::pack_for_tag("pt_BR", &packs).map(|p| p.locale.as_str()), Some("pt"));
        assert_eq!(packs::pack_for_tag("nl-BE", &packs).map(|p| p.locale.as_str()), Some("nl"));
        assert_eq!(packs::pack_for_tag("en-GB", &packs), None);
        assert_eq!(packs::pack_for_tag("", &packs), None);
        assert!(packs::valid_locale("sr-latn"));
        assert!(!packs::valid_locale("sr--latn") && !packs::valid_locale("-x") && !packs::valid_locale("x/y"));
    }

    /// An install or a removal in one window proofs every open document again, parked ones too:
    /// their layouts are keyed on the dictionaries' generation, not only their own revision.
    #[test]
    fn installing_proofs_parked_documents_again() {
        with_backend(FixedBackend::sample(), |s, b| {
            let mut parked = Session::new(crate::sample::sample_document());
            let before = parked.layout();
            assert!(std::sync::Arc::ptr_eq(&before, &parked.layout()), "unchanged: the same layout");
            b.network.store(true, Ordering::Relaxed);
            s.run("proof.installPack", &json!({"locale": "pl"})).unwrap();
            assert!(!std::sync::Arc::ptr_eq(&before, &parked.layout()), "the parked document is laid out (and proofed) again");
            let after = parked.layout();
            s.run("proof.removePack", &json!({"locale": "pl"})).unwrap();
            assert!(!std::sync::Arc::ptr_eq(&after, &parked.layout()));
        });
    }

    /// A backend whose installs wait until the test lets them finish.
    struct Slow {
        inner: FixedBackend,
        go: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl Backend for Slow {
        fn index(&self) -> Result<Vec<Pack>, String> {
            self.inner.index()
        }
        fn fetch_index(&self, c: &packs::Cancel) -> Result<Vec<Pack>, String> {
            self.inner.fetch_index(c)
        }
        fn network_on(&self) -> bool {
            self.inner.network_on()
        }
        fn enable_network(&self) -> Result<(), String> {
            self.inner.enable_network()
        }
        fn install(&self, pack: &Pack, cancel: &packs::Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
            progress(1, pack.size);
            let _ = self.go.lock().unwrap().recv();
            self.inner.install(pack, cancel, progress)
        }
        fn remove(&self, locale: &str) -> Result<(), String> {
            self.inner.remove(locale)
        }
        fn packs_dir(&self) -> Option<std::path::PathBuf> {
            None
        }
        fn installed(&self) -> Vec<packs::Installed> {
            self.inner.installed()
        }
    }

    /// In the app an install runs on a thread, whoever asks: the command returns at once, and
    /// `proof.packs` shows it running, then installed. A removal of the same pack meanwhile is
    /// refused.
    #[test]
    fn in_the_app_installs_run_in_the_background() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = Reset;
        let (tx, rx) = std::sync::mpsc::channel();
        let slow = Arc::new(Slow { inner: FixedBackend::sample(), go: std::sync::Mutex::new(rx) });
        slow.inner.network.store(true, Ordering::Relaxed);
        packs::set_backend(Some(slow.clone() as Arc<dyn Backend>));
        let notified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let n = notified.clone();
        packs::set_background(Some(Arc::new(move || {
            n.fetch_add(1, Ordering::Relaxed);
        })));
        let mut s = Session::new(crate::sample::sample_document());
        let before = packs::finished();
        let r = s.run("proof.installPack", &json!({"locale": "pl"})).unwrap();
        assert_eq!(r["started"], true);
        let list = s.run("proof.packs", &json!({})).unwrap();
        assert_eq!(list["installing"][0]["locale"], "pl");
        let e = s.run("proof.removePack", &json!({"locale": "pl"})).unwrap_err();
        assert!(e.to_string().contains("being installed"), "{e}");
        tx.send(()).unwrap();
        for _ in 0..500 {
            if packs::finished() != before {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_ne!(packs::finished(), before);
        assert!(notified.load(Ordering::Relaxed) > 0, "the window is told");
        let list = s.run("proof.packs", &json!({})).unwrap();
        assert_eq!(list["installing"], json!([]));
        assert_eq!(list["installed"][0]["locale"], "pl");
        assert!(s.ui_requests.is_empty(), "no dialog opens");
    }

    /// A pack read from its folder proofs and hyphenates its language's runs, and leaves the
    /// English runs to English: a tiny Lithuanian pack (`ProofLang::Pack`, no rules of its own).
    #[test]
    fn an_installed_pack_proofs_and_hyphenates_its_runs() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = Reset;
        let dir = std::env::temp_dir().join(format!("wordcraft-pack-fixture-{}", std::process::id()));
        let lt = dir.join("lt");
        std::fs::create_dir_all(&lt).unwrap();
        std::fs::write(lt.join("pack.toml"), "locale = \"lt\"\nname = \"Lietuvių\"\nlicence = \"MIT\"\nclass = \"Embeddable\"\n").unwrap();
        std::fs::write(lt.join("spelling.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(lt.join("spelling.dic"), "2\nlabas\nrytas\n").unwrap();
        // One break, laba-rytas; the other letters only make up the patterns' alphabet.
        std::fs::write(lt.join("hyphenation.pat"), "a1r b l s t y z\n").unwrap();
        wordcraft_proof::packs::set_packs_dir(Some(dir.clone()));
        let text = "labas rytaz labarytas typography beautifull";
        let mut s = Session::new(wordcraft_doc::Document::from_text(text));
        let eng = text.find("typography").unwrap();
        s.doc.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, eng), &|c| c.lang = Some("lt-LT".into())).unwrap();
        s.doc.format_range(&wordcraft_doc::Pos::body(0, eng), &wordcraft_doc::Pos::body(0, text.len()), &|c| c.lang = Some("en-US".into())).unwrap();
        s.doc.settings.auto_hyphenation = true;
        s.relayout();
        let issues = s.run("review.issues", &json!({})).map(|v| v.to_string()).unwrap_or_default();
        let layout = s.layout();
        let ends: Vec<usize> = layout
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|i| if let wordcraft_layout::Placed::Lines { para, .. } = i { Some(para) } else { None })
            .flat_map(|para| para.hyph_after.iter().filter_map(|&k| para.clusters.get(k as usize).map(|c| c.end)).collect::<Vec<_>>())
            .collect();
        wordcraft_proof::packs::set_packs_dir(None);
        let _ = std::fs::remove_dir_all(&dir);
        let laba = text.find("labarytas").unwrap() + "laba".len();
        assert!(ends.contains(&laba), "the pack's pattern breaks laba-rytas: {ends:?}");
        assert!(issues.contains("rytaz"), "the Lithuanian misspelling, from the pack: {issues}");
        assert!(issues.contains("beautifull"), "the English one: {issues}");
        assert!(!issues.contains("\"labas\"") && !issues.contains("typography"), "{issues}");
    }

    use crate::packs::Pack;
}
