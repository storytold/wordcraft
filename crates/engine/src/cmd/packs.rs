//! Language packs as commands (CP4.3): list, install and remove, for the CLI, MCP and the control
//! channel. They open nothing and never turn downloading on: an install needs the font store's
//! consent setting, which only a person in the app turns on (File › Options › Language).
//! See [`crate::packs`].

use serde_json::{Value, json};

use crate::packs::{self, Cancel};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("proof.packs", "Language Packs", "File › Options › Language", list)
            .params(r#"{"refresh"?: bool (fetch the pinned index again; only while downloading is on)} — opens nothing. Returns {"packs": [{"locale", "name", "size" (bytes), "licence" (SPDX), "class", "source" (where the data comes from), "archive" (the download), "spelling", "hyphenation", "installed"}], "installed": [{"locale", "name", "licence"}], "network": bool (downloading on), "available": bool (a font store to install into)}"#)
            .pure(),
        CommandSpec::new("proof.installPack", "Install Language Pack", "File › Options › Language", install)
            .params(r#"{"locale": string (a pack's locale from proof.packs, like nl or pt-pt)} — downloads, verifies (sha256) and installs it; spelling and hyphenation in that language use it at once. Needs downloading on (only the person in the app can turn it on). Opens nothing."#)
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
    let index = if p::bool(v, "refresh") == Some(true) { packs::refresh(&Cancel::new()).map_err(CmdError::Failed)? } else { packs::available() };
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
        "network": backend.as_ref().is_some_and(|b| b.network_on()),
        "available": backend.is_some(),
    }))
}

fn install(s: &mut Session, v: &Value) -> CmdResult {
    let locale = locale_param(v)?;
    if packs::is_installed(&locale) {
        return Ok(json!({"locale": locale, "installed": true, "already": true}));
    }
    let done = packs::install(&locale, &Cancel::new(), &mut |_, _| {}).map_err(CmdError::Failed)?;
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

    fn with_backend<T>(b: FixedBackend, f: impl FnOnce(&mut Session, &FixedBackend) -> T) -> T {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let b = Arc::new(b);
        packs::set_backend(Some(b.clone() as Arc<dyn Backend>));
        let mut s = Session::new(crate::sample::sample_document());
        let r = f(&mut s, &b);
        packs::set_backend(None);
        r
    }

    #[test]
    fn lists_installs_and_removes_without_opening_anything() {
        with_backend(FixedBackend::sample(), |s, b| {
            let r = s.run("proof.packs", &json!({})).unwrap();
            assert_eq!(r["packs"].as_array().unwrap().len(), 6);
            assert_eq!(r["network"], false);
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

    use crate::packs::Pack;
}
