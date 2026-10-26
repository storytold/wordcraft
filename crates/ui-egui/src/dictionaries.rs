//! Spelling dictionaries downloaded from LibreOffice's collection (File › Options › Proofing ›
//! Dictionaries, beside each language without one): `ui.dictionaries` reports each language's
//! dictionary, `ui.dictionaries.download {"lang": "it"}` fetches one into the dictionaries folder
//! ([`wordcraft_engine::proof::libreoffice`]): the one the user chose, else WordCraft's own.
//!
//! The download runs on a background thread with the system's `curl` (shipped with macOS,
//! Windows 10 and later and most Linux distributions), so WordCraft adds no HTTP or TLS
//! dependency. [`poll`] picks up the result each frame, reloads the language's dictionary and
//! reports it in the status bar. The web build has no dictionaries folder: there the command
//! returns an error.

use std::sync::mpsc::Receiver;

use serde_json::{Value, json};
use wordcraft_engine::proof::ProofLang;
use wordcraft_engine::proof::libreoffice::{self, Source};

use crate::WordApp;

/// Fetches a URL's body (the system `curl`; tests use a stand-in).
pub type Fetch = fn(&str) -> Result<Vec<u8>, String>;

/// Dictionary downloads in flight.
pub struct Downloads {
    /// How files are fetched.
    pub fetch: Fetch,
    /// Where downloads go instead of [`wordcraft_engine::proof::hunspell::download_dir`] (tests).
    pub dir: Option<std::path::PathBuf>,
    busy: Option<(ProofLang, Receiver<Result<(), String>>)>,
    /// How the last download ended.
    pub last: Option<Value>,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads { fetch: curl, dir: None, busy: None, last: None }
    }
}

impl Downloads {
    /// The language being downloaded.
    pub fn busy(&self) -> Option<ProofLang> {
        self.busy.as_ref().map(|(l, _)| *l)
    }
}

/// Is `lang`'s Hunspell dictionary installed (found in the dictionaries folder or the system)?
pub fn installed(lang: ProofLang) -> bool {
    wordcraft_engine::proof::installed_dictionaries().into_iter().any(|(l, dic)| l == lang && dic.is_some())
}

fn name(lang: ProofLang) -> &'static str {
    crate::options::language_name(lang)
}

/// `ui.dictionaries` and `ui.dictionaries.download`; `None` for other commands.
pub fn command(app: &mut WordApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    match id {
        "ui.dictionaries" => Some(Ok(status(app))),
        "ui.dictionaries.download" => {
            let lang = p.get("lang").and_then(Value::as_str).map(|t| ProofLang::from_tag(Some(t)));
            let Some(source) = lang.and_then(libreoffice::source) else {
                let codes: Vec<&str> = libreoffice::languages().map(ProofLang::code).collect();
                return Some(Err(format!("`lang` must be one of {}", codes.join(", "))));
            };
            let lang = lang.unwrap_or_default();
            Some(start(app, lang, source))
        }
        _ => None,
    }
}

fn status(app: &WordApp) -> Value {
    let busy = app.dictionaries.busy();
    let languages: Vec<Value> = libreoffice::languages()
        .filter_map(|l| {
            let s = libreoffice::source(l)?;
            Some(json!({
                "lang": l.code(),
                "installed": installed(l),
                "downloading": busy == Some(l),
                "source": s.page_url(),
            }))
        })
        .collect();
    let folder = wordcraft_engine::proof::hunspell::download_dir().map(|d| d.display().to_string());
    json!({"folder": folder, "languages": languages, "last": app.dictionaries.last})
}

#[cfg(target_arch = "wasm32")]
fn start(_app: &mut WordApp, _lang: ProofLang, _source: Source) -> Result<Value, String> {
    Err("dictionaries can be downloaded in the WordCraft desktop app".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn start(app: &mut WordApp, lang: ProofLang, source: Source) -> Result<Value, String> {
    if let Some(l) = app.dictionaries.busy() {
        return Err(format!("the {} dictionary is still downloading", l.code()));
    }
    let Some(dir) = app.dictionaries.dir.clone().or_else(wordcraft_engine::proof::hunspell::download_dir) else {
        return Err("there is no dictionaries folder (settings are off in this session)".into());
    };
    let fetch = app.dictionaries.fetch;
    let wake = app.ctx.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("dictionary-download".into()).spawn(move || {
        let r = fetch(&source.aff_url())
            .and_then(|aff| Ok((aff, fetch(&source.dic_url())?)))
            .and_then(|(aff, dic)| libreoffice::install(&dir, &source, &aff, &dic).map(|_| ()));
        let _ = tx.send(r);
        if let Some(c) = &wake {
            c.request_repaint();
        }
    });
    if let Err(e) = spawned {
        return Err(format!("could not start the download: {e}"));
    }
    app.dictionaries.busy = Some((lang, rx));
    log::info!("dictionary download started: {}", source.folder);
    app.status(crate::i18n::fmt(tl!("Downloading the {language} dictionary…"), &[("language", name(lang))]));
    Ok(json!({"started": lang.code()}))
}

/// Finish a download that ended (each frame).
pub fn poll(app: &mut WordApp) {
    let Some((lang, rx)) = &app.dictionaries.busy else { return };
    let lang = *lang;
    let r = match rx.try_recv() {
        Ok(r) => r,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the download stopped".into()),
    };
    app.dictionaries.busy = None;
    match r {
        Ok(()) => {
            wordcraft_engine::proof::reload_dictionary(lang);
            let installed = installed(lang);
            log::info!("dictionary download finished: {} (loaded: {installed})", lang.code());
            app.dictionaries.last = Some(json!({"lang": lang.code(), "installed": installed}));
            app.status(crate::i18n::fmt(tl!("The {language} dictionary is installed."), &[("language", name(lang))]));
        }
        Err(e) => {
            log::warn!("dictionary download failed: {} ({e})", lang.code());
            app.dictionaries.last = Some(json!({"lang": lang.code(), "error": e}));
            let msg = crate::i18n::fmt(tl!("The {language} dictionary couldn't be downloaded: {error}"), &[("language", name(lang)), ("error", &e)]);
            app.status(msg);
        }
    }
}

/// Fetch `url` with the system's `curl`: HTTPS only, failing on HTTP errors, size- and
/// time-capped. On Windows it starts without a console window (the app uses the GUI subsystem).
#[cfg(not(target_arch = "wasm32"))]
fn curl(url: &str) -> Result<Vec<u8>, String> {
    let max = libreoffice::MAX_FILE.to_string();
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["--fail", "--silent", "--show-error", "--location", "--proto", "=https", "--max-time", "300", "--max-filesize", &max, url]);
    #[cfg(target_os = "windows")]
    {
        /// Win32 `CREATE_NO_WINDOW` process creation flag.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| format!("curl isn't available ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{url}: {}", err.trim()));
    }
    Ok(out.stdout)
}

#[cfg(target_arch = "wasm32")]
fn curl(_url: &str) -> Result<Vec<u8>, String> {
    Err("no downloads on the web".into())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn fake(url: &str) -> Result<Vec<u8>, String> {
        if url.ends_with(".aff") { Ok(b"SET UTF-8\nSFX A Y 1\nSFX A o i o\n".to_vec()) } else { Ok(b"2\ngatto/A\ncane\n".to_vec()) }
    }

    fn broken(_url: &str) -> Result<Vec<u8>, String> {
        Err("offline".into())
    }

    fn wait(app: &mut WordApp) {
        for _ in 0..500 {
            poll(app);
            if app.dictionaries.busy().is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.dictionaries.busy().is_none(), "download never finished");
    }

    #[test]
    fn downloads_into_the_dictionaries_folder_and_reports_errors() {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-dict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        assert!(app.run("ui.dictionaries.download", json!({"lang": "fr"})).is_err(), "no source for French");
        assert!(app.run("ui.dictionaries.download", json!({"lang": "tr"})).is_err(), "Turkish's collection dictionary doesn't load");
        assert!(app.run("ui.dictionaries.download", json!({})).is_err());
        // The dictionaries folder is process-wide and other tests set it, so this one downloads
        // into a folder of its own.
        app.dictionaries.dir = Some(dir.clone());

        app.dictionaries.fetch = broken;
        assert_eq!(app.run("ui.dictionaries.download", json!({"lang": "pt"})).unwrap(), json!({"started": "pt"}));
        wait(&mut app);
        assert_eq!(app.dictionaries.last.as_ref().unwrap()["error"], "offline");
        assert!(!dir.join("pt_BR.dic").exists());

        // A well-formed pair that isn't the pinned LibreOffice file (a stand-in for a changed
        // or tampered download) is refused by its checksum and nothing is saved.
        app.dictionaries.fetch = fake;
        app.run("ui.dictionaries.download", json!({"lang": "pt-BR"})).unwrap();
        assert!(app.run("ui.dictionaries.download", json!({"lang": "pt"})).is_err(), "one at a time");
        wait(&mut app);
        let last = app.dictionaries.last.clone().unwrap();
        assert!(last["lang"] == "pt" && last["error"].as_str().is_some_and(|e| e.contains("checksum")), "{last}");
        assert!(!dir.join("pt_BR.aff").exists() && !dir.join("pt_BR.dic").exists());
        let st = app.run("ui.dictionaries", json!({})).unwrap();
        let pt = st["languages"].as_array().unwrap().iter().find(|l| l["lang"] == "pt").unwrap().clone();
        assert!(pt["installed"].is_boolean() && pt["downloading"] == false);
        let commit = wordcraft_engine::proof::libreoffice::COMMIT;
        assert_eq!(pt["source"], format!("https://github.com/LibreOffice/dictionaries/tree/{commit}/pt_BR"));

        wordcraft_engine::proof::reload_dictionary(ProofLang::Pt);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
