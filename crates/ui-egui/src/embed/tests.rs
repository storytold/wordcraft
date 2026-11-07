use super::Rules;

/// Every id the embedded editor refuses, and the prefixes: everything under them.
#[test]
fn denies_commands() {
    let r = Rules;
    for id in [
        "file.new",
        "file.newFromTemplate",
        "file.open",
        "file.saveAs",
        "file.saveTemplate",
        "file.rename",
        "file.encrypt",
        "file.close",
        "file.print",
        "file.printPdf",
        "file.exportPng",
        "file.recover",
        "file.protect",
        "file.setAuthor",
        "review.trackChanges",
        "review.restrict",
        "ui.fileBrowser",
        "ui.saveAsTemplate",
        "ui.startScreen",
        "tools.autocorrect",
        "tools.templates",
        "tools.macros",
        "styles.organizer",
        "ui.changePicture",
        "ui.openFileDialog",
        "ui.openRecipientList",
        "ui.print",
        "ui.printSubmit",
        "ui.printSettings",
        "insert.picture",
        "insert.object",
        "insert.textFromFile",
        "review.compare",
        "review.combine",
        "view.newWindow",
        "file.options",
    ] {
        assert!(r.denies(id), "{id} must be refused");
    }
    for id in ["mailings.start", "mailings.recipients", "ui.zotero.search"] {
        assert!(r.denies(id), "{id} must be refused by its prefix");
    }
    // What an embedded edit needs stays: saving to the host, the PDF export the host
    // stores as a copy, the Review tab's commands, view and zoom — and the ids that take
    // the host's delivered files in (`intake_allows`).
    for id in [
        "file.save",
        "file.exportPdf",
        "file.info",
        "review.newComment",
        "review.comments",
        "review.acceptAll",
        "review.markup",
        "format.bold",
        "view.zoom",
        "document.inspect",
        "edit.paste",
    ] {
        assert!(!r.denies(id), "{id} must be allowed");
    }
    // The intake ids sit in DENIED but stay allowed through `intake_allows`.
    for id in ["file.open", "insert.picture"] {
        assert!(r.denies(id) && r.intake_allows(id), "{id} is a denied intake id");
    }
}

/// The side menu keeps the pages an embedded edit needs and hides the rest.
#[test]
fn pages() {
    let r = Rules;
    for page in ["info", "save", "export"] {
        assert!(r.keeps_page(page), "{page} must stay in the menu");
    }
    for page in ["home", "new", "open", "recover", "saveAs", "print", "close", "options"] {
        assert!(r.hidden_page(page), "{page} must be hidden");
        assert!(!r.keeps_page(page), "{page} must not be in the menu");
    }
    // "save" routes to a command, not to a page, so the hidden-page check must not swallow it;
    // "close" is refused either way (the command under it is denied too).
    assert!(!r.hidden_page("save"));
    assert!(r.hidden_page("close"));
}

/// The embedded editor's kept export: `file.exportPdf` with the document's own name runs and
/// delivers (no host here, so the delivery is a no-op), not a refusal. The PDF lands where
/// the path says: a temp folder, cleaned up after.
#[test]
fn export_pdf_runs() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.embed = Some(Rules);
    let dir = std::env::temp_dir().join(format!("wordcraft-embed-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Contract.pdf");
    let r = app.run("file.exportPdf", serde_json::json!({"path": path.to_string_lossy()}));
    assert!(r.as_ref().err().is_none_or(|e| !e.contains("embedded")), "export must get past the refusal: {:?}", r);
    assert!(r.as_ref().ok().is_some_and(|v| v.get("path").and_then(serde_json::Value::as_str).is_some()), "the export ran: {:?}", r);
    assert!(path.is_file(), "the PDF was written: {:?}", r);
    let _ = std::fs::remove_dir_all(&dir);
}

/// While the host's delivered files drain, the intake ids run though they are denied, and
/// everything else stays refused.
#[test]
fn intake_ids_stay_allowed() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.embed = Some(Rules);
    // The ids that take the host's delivered files in run (their engine error, e.g. no such
    // file, is not the embedded editor's refusal). This is the route a Password or Save
    // Changes dialog retry takes too, so the exemption must not be a window that closes.
    let opened = app.run("file.open", serde_json::json!({"path": "no-such-file.docx"}));
    if let Err(e) = &opened {
        assert!(!e.contains("embedded"), "file.open must get past the refusal: {e}");
    }
    let inserted = app.run("insert.picture", serde_json::json!({"data": "bm90IHBuZw=="}));
    if let Err(e) = &inserted {
        assert!(!e.contains("embedded"), "insert.picture must get past the refusal: {e}");
    }
    // Everything else stays refused, with the refusal as the error.
    for (id, params) in [
        ("file.new", serde_json::json!({})),
        ("file.saveAs", serde_json::json!({})),
        ("review.compare", serde_json::json!({})),
        ("insert.object", serde_json::json!({})),
    ] {
        let r = app.run(id, params);
        assert!(r.as_ref().err().is_some_and(|e| e.contains("embedded")), "{id} must be refused: {:?}", r);
    }
}

/// The UI requests that route files or windows are refused; the rest go on.
#[test]
fn denies_requests() {
    let r = Rules;
    for kind in ["openFile", "saveAs", "print", "options", "newWindow", "insertPicture", "addFont", "loadSystemFonts", "exportPng"] {
        assert!(r.denies_request(kind), "{kind} must be refused");
    }
    for kind in ["exportPdf", "pasteSpecial", "interfaceTheme"] {
        assert!(!r.denies_request(kind), "{kind} must be allowed");
    }
}

/// The Mailings tab is taken out; the tabs an embedded edit needs stay.
#[test]
fn hides_tab() {
    let r = Rules;
    assert!(r.hides_tab("Mailings"));
    for tab in ["Home", "Insert", "Layout", "References", "Review", "View", "Table"] {
        assert!(!r.hides_tab(tab), "{tab} must stay");
    }
}

/// `run` refuses a denied command with a status message, and lets an allowed one reach the
/// engine (here: `format.bold`, which needs a selection and so fails below the refusal layer).
#[test]
fn run_refuses_denied_and_passes_allowed() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.embed = Some(Rules);
    let refusal = app.run("file.new", serde_json::json!({}));
    let message = refusal.as_ref().err().cloned().unwrap_or_default();
    assert!(message.contains("embedded"), "file.new must be refused: {message}");
    // The refusal reported on the status line, checked before any later run can overwrite it.
    assert!(app.status_msg.as_ref().is_some_and(|(m, _)| m.contains("embedded")), "the refusal must show on the status line");
    let allowed = app.run("format.bold", serde_json::json!({}));
    // The command ran and failed in the engine (no selection): its error is not the
    // embedded editor's refusal.
    if let Err(e) = &allowed {
        assert!(!e.contains("embedded"), "format.bold must get past the refusal: {e}");
    }
}

/// Greyed-out buttons: `widgets::enabled` is false for a denied id without an engine lookup.
#[test]
fn denied_commands_are_disabled() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.embed = Some(Rules);
    assert!(!crate::widgets::enabled(&app, "file.open"));
    assert!(!crate::widgets::enabled(&app, "mailings.start"));
    assert!(crate::widgets::enabled(&app, "document.inspect"));
}

/// The dialogs of denied functionality are refused through `ui.dialog`; the editing dialogs
/// (find, font, paragraph, styles, zoom…) stay open.
#[test]
fn denies_dialogs() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.embed = Some(Rules);
    // Every refused dialog name, asserted wholesale so an added name can't ship untested.
    for name in super::DENIED_DIALOGS {
        let r = app.run("ui.dialog", serde_json::json!({"name": name}));
        assert!(r.as_ref().err().is_some_and(|e| e.contains("embedded")), "{name} must be refused: {:?}", r);
    }
    let r = app.run("ui.dialog", serde_json::json!({"name": "wordCount"}));
    assert!(r.is_ok(), "wordCount must open: {:?}", r);
}

/// Track Changes forced on at startup can't be switched off through the refused command.
#[test]
fn track_changes_cannot_be_toggled() {
    let r = Rules;
    assert!(r.denies("review.trackChanges"));
    assert!(super::DENIED.contains(&"review.trackingOptions"), "the author name stays the host's");
}
