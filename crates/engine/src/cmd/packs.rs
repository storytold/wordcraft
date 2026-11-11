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
            .params(r#"{"refresh"?: bool (fetch the pinned index again; only while downloading is on; in the app it runs in the background: see "refreshing")} — opens nothing. Returns {"packs": [{"locale", "name", "size" (bytes), "licence" (SPDX), "class", "source" (where the data comes from), "archive" (the download), "spelling", "hyphenation", "installed"}], "languages": [{"tags", "pack", "installed", "size", "proofing" (built_in, installed, own_dictionary, system_dictionary, install, grammar_built_in, not_available), "default"}] (Options › Language › Your languages), "installed": [{"locale", "name", "licence"}], "installing": [{"locale", "done", "total"} (bytes)], "refreshing": bool, "errors": {locale or "" (the list): message} (the last failure of each), "index_override": string|null (a test index in use instead of the pinned one), "network": bool (downloading on), "available": bool (a font store to install into)}"#)
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

fn list(s: &mut Session, v: &Value) -> CmdResult {
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
        "languages": packs::your_languages(s),
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

    /// The bar's missing-pack cache keys on `index_generation`, so a refresh that returns a list
    /// of the same length must still change it (CodeRabbit on #5208).
    #[test]
    fn a_refresh_of_the_same_length_still_changes_the_index_generation() {
        with_backend(FixedBackend::sample(), |s, b| {
            b.network.store(true, Ordering::Relaxed);
            let n = packs::available_len();
            let before = packs::index_generation();
            s.run("proof.packs", &json!({"refresh": true})).unwrap();
            assert_eq!(packs::available_len(), n, "same length");
            assert_ne!(packs::index_generation(), before);
        });
    }

    /// Options › Language › Your languages: fresh, with Dutch installed, a document with Thai,
    /// Turkish without a pack, and en-GB as the default.
    mod your_languages {
        use super::*;
        use crate::packs::Proofing;

        fn none(_: &str) -> Option<std::path::PathBuf> {
            None
        }

        fn row<'a>(rows: &'a [packs::LanguageRow], tag: &str) -> &'a packs::LanguageRow {
            rows.iter().find(|r| r.is_tag(tag) || r.pack.as_deref() == Some(tag)).unwrap_or_else(|| panic!("no row for {tag}: {rows:?}"))
        }

        fn mark(s: &mut Session, para: usize, lang: &str) {
            s.run("caret.set", &json!({"pos": {"block": para, "off": 0}})).unwrap();
            s.run("select.paragraph", &json!({})).unwrap();
            s.run("review.language", &json!({"lang": lang})).unwrap();
        }

        #[test]
        fn a_fresh_install_lists_only_english() {
            with_backend(FixedBackend::sample(), |s, _| {
                let rows = packs::your_languages_with(s, None, &none);
                assert_eq!(rows.len(), 1, "{rows:?}");
                assert_eq!(rows[0].proofing, Proofing::BuiltIn);
                assert!(rows[0].default);
                assert_eq!(rows[0].pack, None);
            });
        }

        #[test]
        fn an_installed_pack_is_a_row_with_remove() {
            with_backend(FixedBackend::sample(), |s, b| {
                b.network.store(true, Ordering::Relaxed);
                s.run("proof.installPack", &json!({"locale": "nl"})).unwrap();
                let rows = packs::your_languages_with(s, None, &none);
                assert_eq!(rows.len(), 2);
                let nl = row(&rows, "nl");
                assert!(nl.installed && nl.tags.is_empty() && !nl.default);
                assert_eq!(nl.proofing, Proofing::Installed);
                let r = s.run("proof.packs", &json!({})).unwrap();
                assert_eq!(r["languages"].as_array().unwrap().len(), 2, "MCP sees the same rows");
            });
        }

        #[test]
        fn document_languages_show_install_grammar_and_not_available() {
            with_backend(FixedBackend::sample(), |s, b| {
                s.run("document.setText", &json!({"text": "Hallo\nMerhaba\nสวัสดี\nCześć\nBonjour"})).unwrap();
                mark(s, 0, "nl-NL");
                mark(s, 1, "tr-TR");
                mark(s, 2, "th-TH");
                mark(s, 3, "pl-PL");
                mark(s, 4, "fr-FR");
                let rows = packs::your_languages_with(s, None, &none);
                let nl = row(&rows, "nl-NL");
                assert_eq!((nl.proofing, nl.size, nl.installed), (Proofing::Install, 1_950_000, false));
                // Turkish has grammar rules built in and no pack in the sample index.
                assert_eq!(row(&rows, "tr-TR").proofing, Proofing::NotAvailable);
                assert_eq!(row(&rows, "th-TH").proofing, Proofing::NotAvailable);
                let pl = row(&rows, "pl-PL");
                assert_eq!((pl.proofing, pl.size), (Proofing::GrammarBuiltIn, 4_620_000));
                // Installing turns Install into Installed; the document keeps the row.
                b.network.store(true, Ordering::Relaxed);
                s.run("proof.installPack", &json!({"locale": "nl"})).unwrap();
                let rows = packs::your_languages_with(s, None, &none);
                assert_eq!(row(&rows, "nl-NL").proofing, Proofing::Installed);
                s.run("proof.removePack", &json!({"locale": "nl"})).unwrap();
                let rows = packs::your_languages_with(s, None, &none);
                assert_eq!(row(&rows, "nl-NL").proofing, Proofing::Install, "still used by the document");
            });
        }

        #[test]
        fn valencian_text_is_on_the_valencian_packs_row() {
            let mut b = FixedBackend::sample();
            let mut v = b.packs.first().cloned().unwrap();
            v.locale = "ca-valencia".into();
            v.size = 1_500_000;
            b.packs.push(v);
            with_backend(b, |s, b| {
                s.run("document.setText", &json!({"text": "Bon dia"})).unwrap();
                mark(s, 0, "ca-ES-valencia");
                let rows = packs::your_languages_with(s, None, &none);
                assert_eq!(row(&rows, "ca-ES-valencia").pack.as_deref(), Some("ca-valencia"), "as the bar offers it");
                // Installing the Catalan pack doesn't make Valencian installed.
                b.network.store(true, Ordering::Relaxed);
                s.run("proof.installPack", &json!({"locale": "ca"})).unwrap();
                let rows = packs::your_languages_with(s, None, &none);
                let v = row(&rows, "ca-ES-valencia");
                assert_eq!((v.pack.as_deref(), v.installed, v.proofing), (Some("ca-valencia"), false, Proofing::Install));
            });
        }

        #[test]
        fn the_own_folder_and_the_default_language() {
            with_backend(FixedBackend::sample(), |s, _| {
                s.run("document.setText", &json!({"text": "Cześć\nMerhaba"})).unwrap();
                mark(s, 0, "pl-PL");
                mark(s, 1, "tr-TR");
                let dic = |tag: &str| {
                    if tag.starts_with("pl") {
                        Some(std::path::PathBuf::from("/mine/dicts/pl_PL.dic"))
                    } else if tag.starts_with("tr") {
                        Some(std::path::PathBuf::from("/usr/share/hunspell/tr_TR.dic"))
                    } else {
                        None
                    }
                };
                let rows = packs::your_languages_with(s, Some("/mine/dicts"), &dic);
                assert_eq!(row(&rows, "pl-PL").proofing, Proofing::OwnDictionary);
                assert_eq!(row(&rows, "tr-TR").proofing, Proofing::SystemDictionary);
                // en-GB as the default is its own row, the default one; en-US stays if the document uses it.
                s.run("review.language", &json!({"lang": "en-GB", "default": true})).unwrap();
                let rows = packs::your_languages_with(s, None, &none);
                let gb = row(&rows, "en-GB");
                assert!(gb.default && gb.proofing == Proofing::BuiltIn);
                assert_eq!(rows.iter().filter(|r| r.default).count(), 1);
            });
        }

        #[test]
        fn set_as_default_sets_the_pref_and_nothing_else() {
            with_backend(FixedBackend::sample(), |s, _| {
                s.run("document.setText", &json!({"text": "Hello"})).unwrap();
                s.run("select.all", &json!({})).unwrap();
                let (doc, sel) = (s.doc.styles.default_chr.lang.clone(), s.sel.clone());
                let normal = s.doc.styles.styles.iter().find(|x| x.id == "Normal").map(|x| x.chr.lang.clone());
                // A redo step exists: the setting is no edit, so it neither adds an undo step nor clears it.
                s.run("text.insert", &json!({"text": "x"})).unwrap();
                s.run("edit.undo", &json!({})).unwrap();
                let rev = s.rev();
                let (depth, redo, dirty) = (s.undo_depth(), s.can_redo(), s.dirty);
                assert!(redo);
                s.run("review.language.preferred", &json!({"lang": "nl-NL"})).unwrap();
                assert_eq!((s.undo_depth(), s.can_redo(), s.dirty), (depth, redo, dirty), "history, redo and modified flag are kept");
                assert_eq!(s.prefs.default_language.as_deref(), Some("nl-NL"));
                assert_eq!(s.doc.styles.default_chr.lang, doc);
                assert_eq!(s.doc.styles.styles.iter().find(|x| x.id == "Normal").map(|x| x.chr.lang.clone()), normal);
                assert_eq!(s.sel, sel);
                assert_eq!(packs::document_languages(s), ["en-US"], "the text keeps its language");
                assert_eq!(s.rev(), rev);
                assert!(s.run("review.language.preferred", &json!({})).is_err());
                assert!(s.run("review.language.preferred", &json!({"lang": "not a tag!!"})).is_err());
            });
        }

        #[test]
        fn hostile_tags_never_panic() {
            with_backend(FixedBackend::sample(), |s, _| {
                s.run("document.setText", &json!({"text": "a b"})).unwrap();
                s.prefs.default_language = Some("x".repeat(10_000));
                let _ = packs::your_languages_with(s, Some(""), &none);
                s.prefs.default_language = Some("\u{0}-\u{202e}".into());
                let _ = packs::your_languages_with(s, None, &none);
            });
        }
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
        // A script-qualified tag finds its script's pack before the language's (CP4.3: Serbian
        // (Latin) chosen in the Language dialog is proofed by `sr-latn`, not the Cyrillic `sr`).
        packs.push(Pack { locale: "sr".into(), ..packs[0].clone() });
        packs.push(Pack { locale: "sr-latn".into(), ..packs[0].clone() });
        assert_eq!(packs::pack_for_tag("sr-Latn-RS", &packs).map(|p| p.locale.as_str()), Some("sr-latn"));
        assert_eq!(packs::pack_for_tag("sr-Cyrl-RS", &packs).map(|p| p.locale.as_str()), Some("sr"));
        assert_eq!(packs::pack_for_tag("sr-latn", &packs).map(|p| p.locale.as_str()), Some("sr-latn"));
        assert_eq!(packs::pack_for_tag("-", &packs), None);
        assert_eq!(packs::pack_for_tag("nl--", &packs).map(|p| p.locale.as_str()), Some("nl"));
        // A variant after the region is its language-and-variant pack's; the region alone isn't.
        packs.push(Pack { locale: "ca-valencia".into(), ..packs[0].clone() });
        assert_eq!(packs::pack_for_tag("ca-ES-valencia", &packs).map(|p| p.locale.as_str()), Some("ca-valencia"));
        assert_eq!(packs::pack_for_tag("ca_es_VALENCIA", &packs).map(|p| p.locale.as_str()), Some("ca-valencia"));
        assert_eq!(packs::pack_for_tag("ca-ES", &packs).map(|p| p.locale.as_str()), Some("ca"));
        assert_eq!(packs::pack_for_tag("ca", &packs).map(|p| p.locale.as_str()), Some("ca"));
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

    /// Sets up [`Slow`] with downloading on and installs on a thread; returns its go-ahead.
    fn slow_in_background() -> std::sync::mpsc::Sender<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        let slow = Arc::new(Slow { inner: FixedBackend::sample(), go: std::sync::Mutex::new(rx) });
        slow.inner.network.store(true, Ordering::Relaxed);
        packs::set_backend(Some(slow as Arc<dyn Backend>));
        packs::set_background(Some(Arc::new(|| {})));
        tx
    }

    fn wait_for_finish(before: u64) {
        for _ in 0..500 {
            if packs::finished() != before {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_ne!(packs::finished(), before, "the install thread ended");
    }

    /// FU.1: the person's own Cancel isn't a failure. The install stops being listed at once and
    /// ends without an error ("Couldn't install: cancelled" in red was wrong); removing the pack
    /// while the cancelled download is still stopping says so.
    #[test]
    fn a_cancelled_install_ends_without_an_error() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = Reset;
        let go = slow_in_background();
        let before = packs::finished();
        assert!(packs::start_install("pl").unwrap());
        assert_eq!(packs::installing().len(), 1);
        packs::cancel("pl");
        assert!(packs::installing().is_empty(), "Install shows again at once");
        let e = packs::remove("pl").unwrap_err();
        assert!(e.contains("still stopping"), "{e}");
        go.send(()).unwrap();
        wait_for_finish(before);
        assert!(packs::installing().is_empty());
        assert_eq!(packs::error("pl"), None, "a cancel is no error");
        assert!(packs::errors().is_empty(), "{:?}", packs::errors());
        assert!(!packs::is_installed("pl"));
    }

    /// FU.1: Install right after Cancel, before the cancelled download has stopped, isn't
    /// refused as still running: it shows as installing and the pack installs once the cancelled
    /// run ends (never two downloads of a pack at once).
    #[test]
    fn install_right_after_cancel_starts_again() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = Reset;
        let go = slow_in_background();
        let before = packs::finished();
        assert!(packs::start_install("pl").unwrap());
        packs::cancel("pl");
        assert!(packs::start_install("pl").unwrap(), "started again");
        let running = packs::installing();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].locale, "pl");
        // The cancelled run ends, then the new one.
        go.send(()).unwrap();
        go.send(()).unwrap();
        wait_for_finish(before);
        assert!(packs::is_installed("pl"), "the second Install installed it");
        assert!(packs::installing().is_empty());
        assert!(packs::errors().is_empty(), "{:?}", packs::errors());
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

    /// Japanese, Chinese and Thai have no Hunspell dictionary anywhere: without a pack for them
    /// in the list the status bar doesn't say "(no dictionary)" and the bar offers nothing; once
    /// the list has one, both do.
    #[test]
    fn a_language_with_no_pack_and_no_dictionary_asks_for_nothing() {
        with_backend(FixedBackend::sample(), |s, _| {
            for tag in ["ja-JP", "zh-CN", "th-TH"] {
                assert_eq!(packs::missing_dictionary(Some(tag)), None, "{tag}");
            }
            s.run("document.setText", &json!({"text": "日本語のテキスト"})).unwrap();
            s.run("select.all", &json!({})).unwrap();
            s.run("review.language", &json!({"lang": "ja-JP"})).unwrap();
            assert!(packs::missing(s).is_empty(), "no Japanese pack: nothing offered");
        });
        let mut b = FixedBackend::sample();
        let ja = Pack { locale: "ja".into(), name: "日本語".into(), ..b.packs[0].clone() };
        b.packs.push(ja);
        with_backend(b, |s, _| {
            assert!(packs::missing_dictionary(Some("ja-JP")).is_some(), "a Japanese pack is available");
            s.run("document.setText", &json!({"text": "日本語のテキスト"})).unwrap();
            s.run("select.all", &json!({})).unwrap();
            s.run("review.language", &json!({"lang": "ja-JP"})).unwrap();
            assert_eq!(packs::missing(s).into_iter().map(|p| p.locale).collect::<Vec<_>>(), ["ja"]);
        });
    }

    /// Swiss German and European Portuguese are pack languages in craft-proof (`de-ch`, `pt-pt`:
    /// their spelling is their own) but hyphenate and check grammar as German and Portuguese.
    #[test]
    fn swiss_german_and_portugal_portuguese_keep_their_languages_rules() {
        use wordcraft_proof::ProofLang;
        let de_ch = ProofLang::from_tag(Some("de-CH"));
        let pt_pt = ProofLang::from_tag(Some("pt-PT"));
        assert_eq!(de_ch.tag(), "de-ch");
        assert_eq!(pt_pt.tag(), "pt-pt");
        assert!(packs::proofable("de-CH") && packs::proofable("pt-PT"));
        for word in ["Zusammenarbeit", "Rechtschreibung"] {
            let german = wordcraft_proof::hyphen::word_breaks_in(word, ProofLang::De).0;
            assert!(!german.is_empty(), "German hyphenates {word}");
            assert_eq!(wordcraft_proof::hyphen::word_breaks_in(word, de_ch).0, german, "Swiss German hyphenates {word} as German");
        }
        for word in ["computador", "informação"] {
            assert_eq!(
                wordcraft_proof::hyphen::word_breaks_in(word, pt_pt).0,
                wordcraft_proof::hyphen::word_breaks_in(word, ProofLang::Pt).0,
                "{word}"
            );
        }
        let messages = |lang| wordcraft_proof::check_grammar_in("Ele disse que que viria .", lang).into_iter().map(|i| i.message).collect::<Vec<_>>();
        assert!(!messages(ProofLang::Pt).is_empty());
        assert_eq!(messages(pt_pt), messages(ProofLang::Pt), "Portugal text gets the Portuguese grammar checks");
    }

    /// The one region test and region-dropping step the engine and the UI share: two letters or
    /// three digits after the language; hostile text never panics.
    #[test]
    fn a_region_is_two_letters_or_three_digits() {
        for r in ["NL", "nl", "419", "ZZ"] {
            assert!(packs::is_region(r), "{r}");
        }
        for r in ["", "n", "nld", "4190", "41", "Latn", "valencia", "é1", "\u{202e}\u{202e}"] {
            assert!(!packs::is_region(r), "{r:?}");
        }
        for (tag, bare) in [
            ("nl-NL", "nl"),
            ("sr-Latn-RS", "sr-Latn"),
            ("ca-ES-valencia", "ca-valencia"),
            ("es_419", "es"),
            ("pt", "pt"),
            ("en-US-x-test", "en-x-test"),
            ("", ""),
            ("-", "-"),
        ] {
            assert_eq!(packs::without_region(tag), bare, "{tag}");
        }
        // The language itself is never a region, even when it looks like one.
        assert_eq!(packs::without_region("de"), "de");
        let _ = packs::without_region(&"-".repeat(10_000));
        let _ = packs::without_region("\u{0}-\u{202e}");
    }

    /// A font store that has never fetched the list (a fresh install), or whose cached list is
    /// for an older pin: `index` has nothing until a fetch.
    struct Uncached(FixedBackend);

    impl Backend for Uncached {
        fn index(&self) -> Result<Vec<Pack>, String> {
            Ok(Vec::new())
        }
        fn fetch_index(&self, c: &packs::Cancel) -> Result<Vec<Pack>, String> {
            self.0.fetch_index(c)
        }
        fn network_on(&self) -> bool {
            self.0.network_on()
        }
        fn enable_network(&self) -> Result<(), String> {
            self.0.enable_network()
        }
        fn install(&self, pack: &Pack, cancel: &packs::Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
            self.0.install(pack, cancel, progress)
        }
        fn remove(&self, locale: &str) -> Result<(), String> {
            self.0.remove(locale)
        }
        fn packs_dir(&self) -> Option<std::path::PathBuf> {
            None
        }
        fn installed(&self) -> Vec<packs::Installed> {
            self.0.installed()
        }
    }

    /// The sample packs and Greenlandic (`kl`), a language no system has a dictionary for.
    fn with_greenlandic() -> FixedBackend {
        let mut b = FixedBackend::sample();
        let kl = Pack { locale: "kl".into(), name: "Kalaallisut".into(), ..b.packs[0].clone() };
        b.packs.push(kl);
        b
    }

    fn with_uncached<T>(b: FixedBackend, f: impl FnOnce(&mut Session, &Uncached) -> T) -> T {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = Reset;
        let b = Arc::new(Uncached(b));
        packs::set_backend(Some(b.clone() as Arc<dyn Backend>));
        let mut s = Session::new(wordcraft_doc::Document::new());
        f(&mut s, &b)
    }

    /// Martin's test of a fresh install: Greenlandic text, no list fetched yet. The bar still
    /// offers the language (never English, nor one with a dictionary); getting it fetches the
    /// list and installs its pack. A language the list then has no pack for (Klingon) stops
    /// being offered.
    #[test]
    fn with_no_list_yet_the_bar_still_offers_the_language() {
        with_uncached(with_greenlandic(), |s, b| {
            s.run("document.setText", &json!({"text": "Hello there\nAluu, qanoq ippit?\ntlhIngan Hol"})).unwrap();
            for (para, lang) in [(1, "kl-GL"), (2, "tlh")] {
                s.run("caret.set", &json!({"pos": {"block": para, "off": 0}})).unwrap();
                s.run("select.paragraph", &json!({})).unwrap();
                s.run("review.language", &json!({"lang": lang})).unwrap();
            }
            assert!(packs::available().is_empty() && packs::missing(s).is_empty(), "no list: no pack to offer");
            for tag in ["kl-GL", "tlh"] {
                assert!(wordcraft_proof::missing_dictionary(Some(tag)).is_some(), "{tag} has no dictionary here");
            }
            assert_eq!(packs::unlisted(s), ["kl-GL", "tlh"], "English is never offered");
            // Downloading off: getting it fails up front, and asks for nothing.
            assert!(packs::start_get("kl-GL").is_err());
            assert!(!packs::fetched());
            b.0.network.store(true, Ordering::Relaxed);
            assert_eq!(packs::start_get("kl-GL"), Ok(false), "fetched and installed here (no background thread)");
            assert!(packs::is_installed("kl"));
            assert!(packs::fetched());
            assert!(packs::unlisted(s).is_empty(), "the list is known now: Klingon, which it lacks, isn't offered");
            assert!(packs::missing(s).is_empty());
        });
    }

    /// Review: a get refused because downloading is off isn't remembered, so a later fetch (the
    /// person's Refresh in Options) doesn't install it unasked.
    #[test]
    fn a_refused_get_is_not_installed_by_a_later_fetch() {
        with_uncached(with_greenlandic(), |_, b| {
            assert!(packs::start_get("kl-GL").is_err(), "downloading off");
            b.0.network.store(true, Ordering::Relaxed);
            assert_eq!(packs::start_refresh(), Ok(false));
            assert!(packs::find("kl").is_some(), "the list has it");
            assert!(!packs::is_installed("kl"), "but nobody asked for it after the refusal");
        });
    }

    /// The list was fetched, but lists nothing for the language: the bar stops offering it.
    #[test]
    fn a_language_the_fetched_list_lacks_is_no_longer_offered() {
        with_uncached(FixedBackend::sample(), |s, b| {
            s.run("document.setText", &json!({"text": "tlhIngan Hol"})).unwrap();
            s.run("select.all", &json!({})).unwrap();
            s.run("review.language", &json!({"lang": "tlh"})).unwrap();
            assert_eq!(packs::unlisted(s), ["tlh"]);
            b.0.network.store(true, Ordering::Relaxed);
            assert_eq!(packs::start_get("tlh"), Ok(false));
            assert!(packs::installed().is_empty(), "nothing to install");
            assert!(packs::unlisted(s).is_empty() && packs::missing(s).is_empty());
        });
    }

    /// With downloading already on and no list (or one for an older pin), the window fetches it
    /// once in the background, asking nothing; never with downloading off, never twice.
    #[test]
    fn a_missing_list_is_fetched_once_when_downloading_is_on() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = Reset;
        let b = Arc::new(Uncached(with_greenlandic()));
        packs::set_backend(Some(b.clone() as Arc<dyn Backend>));
        assert!(!packs::refresh_if_stale(), "no background thread (the CLI, MCP): never");
        packs::set_background(Some(Arc::new(|| {})));
        assert!(!packs::refresh_if_stale(), "downloading off: no request");
        b.0.network.store(true, Ordering::Relaxed);
        assert!(!packs::refresh_if_stale(), "asked once a session: turned on later, the person's own click fetches");
        // A new session (the backend set again) with downloading on.
        packs::set_backend(Some(b.clone() as Arc<dyn Backend>));
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("document.setText", &json!({"text": "Aluu"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        s.run("review.language", &json!({"lang": "kl"})).unwrap();
        let before = packs::finished();
        assert!(packs::refresh_if_stale());
        wait_for_finish(before);
        assert!(packs::fetched());
        assert_eq!(packs::missing(&s).into_iter().map(|p| p.locale).collect::<Vec<_>>(), ["kl"], "the bar offers the pack");
        assert!(!packs::refresh_if_stale(), "once a session");
        assert!(packs::installed().is_empty(), "nothing installed without a click");
    }

    /// Martin's test: choosing a language with nothing selected (the caret only) sets the typing
    /// language; the bar offers its pack before anything is typed.
    #[test]
    fn the_caret_typing_language_asks_for_its_pack() {
        with_backend(FixedBackend::sample(), |s, _| {
            s.run("document.setText", &json!({"text": "Hello there"})).unwrap();
            s.run("caret.set", &json!({"pos": {"block": 0, "off": 11}})).unwrap();
            assert!(packs::missing(s).is_empty());
            s.run("review.language", &json!({"lang": "nl-NL"})).unwrap();
            assert!(s.sel.is_collapsed());
            assert!(!packs::document_languages(s).iter().any(|l| l == "nl-NL"), "no Dutch text yet");
            assert_eq!(packs::caret_language(s).as_deref(), Some("nl-NL"));
            assert_eq!(packs::missing(s).into_iter().map(|p| p.locale).collect::<Vec<_>>(), ["nl"]);
            // Do not check spelling or grammar at the caret: nothing asked.
            s.run("review.language", &json!({"lang": "nl-NL", "noProof": true})).unwrap();
            assert_eq!(packs::caret_language(s), None);
            assert!(packs::missing(s).is_empty());
        });
    }

    use crate::packs::Pack;
}
