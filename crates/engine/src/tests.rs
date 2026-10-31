// Reuse our hand-written Czech proof fixture; its install helper resolves this parent import.
use wordcraft_proof::cs::install_dictionary;
#[path = "../../proof/src/cs/test_dict.rs"]
mod czech_test_dict;

use serde_json::json;
use wordcraft_doc::props::BorderStyle;
use wordcraft_doc::{Pos, StoryRef};

use crate::{Session, cmd};

mod bidi;
mod documents;
mod font_sizes;
mod fonts;
mod ime_noop;
mod revision_checks;
mod typing;
mod wordart;

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

#[test]
fn doc_extension_dispatches_to_docbin() {
    // Garbage bytes with a .doc name must produce an error through the Word 97-2003
    // reader — never a panic and never a silently empty document.
    for name in ["x.doc", "x.dot"] {
        assert!(crate::io::open_bytes(name, &[0u8; 64]).is_err(), "{name} should fail");
    }
}

#[test]
fn encrypted_doc_asks_for_a_password_and_rejects_a_wrong_one() {
    // #1079: a Word 97-2003 file with the encryption flag and an RC4 header whose verifier does
    // not match any password asks for one, and a given password is reported as wrong.
    use std::io::Write;
    let mut fib = vec![0u8; 0x100];
    fib[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
    fib[0x0A..0x0C].copy_from_slice(&0x0100u16.to_le_bytes());
    let mut header = vec![1u8, 0, 1, 0];
    header.extend_from_slice(&[0x5A; 48]);
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
    for (name, data) in [("WordDocument", &fib), ("0Table", &header)] {
        comp.create_stream(format!("/{name}")).unwrap().write_all(data).unwrap();
    }
    let bytes = comp.into_inner().into_inner();
    for name in ["x.doc", "x.dot"] {
        let e = crate::io::open_bytes_with(name, &bytes, None).unwrap_err();
        assert!(crate::io::needs_password(&e), "{e}");
        let e = crate::io::open_bytes_with(name, &bytes, Some("nope")).unwrap_err();
        assert!(crate::io::wrong_password(&e), "{e}");
    }
    // Opened by content (no extension) it asks the same way.
    let e = crate::io::open_bytes_with("noext", &bytes, None).unwrap_err();
    assert!(crate::io::needs_password(&e), "{e}");
}

/// Gap 9: AES-encrypted Word files — Standard encryption (Word 2007) and agile packages — open
/// with the right password whatever Word extension they carry, and ask for one or refuse a wrong
/// one the same way.
#[test]
fn aes_encrypted_packages_open_under_any_word_extension() {
    let mut doc = wordcraft_doc::Document::from_text("Old secret minutes");
    doc.core.title = "Minutes".into();
    let pkg = wordcraft_docx::write(&doc).unwrap();
    let standard = wordcraft_docx::encrypt_standard_for_tests(&pkg, "letmein", 128).unwrap();
    let agile = wordcraft_docx::encrypt_with_spin_count(&pkg, "letmein", 8).unwrap();
    for (kind, bytes) in [("standard", &standard), ("agile", &agile)] {
        for name in ["old.doc", "old.docx", "old.dot", "noext"] {
            let (d, encrypted) = crate::io::open_bytes_with(name, bytes, Some("letmein")).unwrap_or_else(|e| panic!("{kind} {name}: {e}"));
            assert!(encrypted, "{kind} {name}");
            assert!(d.plain_text(StoryRef::Body).contains("Old secret minutes"), "{kind} {name}");
            assert_eq!(d.core.title, "Minutes");
            let e = crate::io::open_bytes_with(name, bytes, None).unwrap_err();
            assert!(crate::io::needs_password(&e), "{kind} {name}: {e}");
            let e = crate::io::open_bytes_with(name, bytes, Some("letmeout")).unwrap_err();
            assert!(crate::io::wrong_password(&e), "{kind} {name}: {e}");
        }
    }
}

#[test]
fn txt_open_decodes_windows_1252_and_unmarked_utf16() {
    // #433: the TXT open path sniffs encodings like the formats importer.
    let open = |b: &[u8]| crate::io::open_bytes("x.txt", b).unwrap().plain_text(StoryRef::Body);
    assert_eq!(open(b"\x80"), "€");
    assert_eq!(open(b"A\0B\0"), "AB");
    assert_eq!(open(b"\0A\0B"), "AB");
    assert_eq!(open("€".as_bytes()), "€");
    assert_eq!(open(b"\xFF\xFEA\0B\0"), "AB");
    assert_eq!(open(b"A\0B"), "AB", "odd byte count stays safe");
    assert_eq!(crate::io::decode_text(b"caf\xE9 \x80"), "café €");
}

#[test]
fn txt_save_writes_list_labels() {
    // #434: list paragraphs keep their bullet or number and a tab in plain text.
    let doc = crate::io::open_bytes("l.md", b"- alpha\n- beta\n\n1. first\n2. second\n").unwrap();
    assert_eq!(crate::io::save_bytes("l.txt", &doc).unwrap(), "•\talpha\r\n•\tbeta\r\n1.\tfirst\r\n2.\tsecond".as_bytes());
    let plain = crate::io::open_bytes("p.md", b"alpha\n\nbeta\n").unwrap();
    assert_eq!(crate::io::save_bytes("p.txt", &plain).unwrap(), b"alpha\r\nbeta");
}

#[test]
fn every_command_has_unique_id() {
    let reg = cmd::registry();
    let mut ids: Vec<&str> = reg.all().iter().map(|c| c.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate command ids");
    assert!(n > 150, "{n} commands");
}

#[test]
fn every_shortcut_runs_one_command() {
    // `by_shortcut` takes the first match, so a shared key silently shadows the later command (#186).
    let reg = cmd::registry();
    let mut seen = std::collections::HashMap::new();
    for c in reg.all() {
        for k in c.shortcut.split(" / ").filter(|k| !k.is_empty()) {
            if let Some(other) = seen.insert(k.to_ascii_lowercase(), c.id) {
                panic!("{k} is bound to both {other} and {}", c.id);
            }
        }
    }
    assert_eq!(reg.by_shortcut("Shift+F3").map(|c| c.id), Some("format.changeCase"));
}

/// The command a shortcut runs.
fn id(k: &str) -> Option<&'static str> {
    cmd::registry().by_shortcut(k).map(|c| c.id)
}

#[test]
fn word_default_shortcuts_resolve() {
    // #330: Word's documented default shortcuts reach the matching commands, with Word for
    // Mac's keys where macOS takes Word's (⌘Q quits, ⌘Space is Spotlight, ⌘⇧G is Find Previous).
    assert_eq!(id("Mod+Alt+1"), Some("para.heading1"));
    assert_eq!(id("Mod+Alt+P"), Some("view.printLayout"));
    assert_eq!(id("Mod+Alt+O"), Some("view.outline"));
    assert_eq!(id("Mod+Alt+N"), Some("view.draft"));
    assert_eq!(id("Mod+Alt+S"), Some("view.split"));
    assert_eq!(id("Mod+Alt+-"), Some("text.emDash"));
    assert_eq!(id("Mod+Shift+F5"), Some("insert.bookmark"));
    assert_eq!(id("Alt+F7"), Some("review.spelling"));
    assert_eq!(id("Shift+F12"), Some("file.save"));
    assert_eq!(id(cmd::mac_or("Ctrl+Q", "Mod+Q")), Some("para.reset"));
    assert_eq!(id(cmd::mac_or("Ctrl+Space", "Mod+Space")), Some("format.resetChar"));
    assert_eq!(id(cmd::mac_or("Ctrl+Shift+D", "Alt+Shift+D")), Some("insert.dateField"));
    // Option+arrows move by word everywhere; ⌘←/⌘→ go to the line's ends on macOS (#954).
    assert_eq!(id("Alt+Left"), Some("caret.wordLeft"));
    assert_eq!(id("Alt+Right"), Some("caret.wordRight"));
    assert_eq!(id("Mod+Left"), Some(cmd::mac_or("caret.home", "caret.wordLeft")));
    assert_eq!(id("Mod+Right"), Some(cmd::mac_or("caret.end", "caret.wordRight")));
    assert_eq!(id("Shift+F1"), Some("styles.inspector"));
    assert_eq!(id("Alt+F8"), Some("tools.macros"));
    assert_eq!(id("Mod+Shift+X"), Some("format.strikethrough"));
    assert_eq!(id("Shift+F4"), Some("edit.findNext"));
    assert_eq!(id("Mod+Insert"), Some("edit.copy"));
    assert_eq!(id("Shift+Insert"), Some("edit.paste"));
    assert_eq!(id("Mod+Alt+Z"), Some("edit.goBack"));
    assert_eq!(id("Mod+Alt+F2"), Some("file.open"));
    assert_eq!(id("Alt+Shift+F2"), Some("file.save"));
    assert_eq!(id("Mod+Alt+I"), Some("file.print"));
    assert_eq!(id("Alt+Shift+F1"), Some("fields.previous"));
}

#[test]
fn word_default_shortcuts_resolve_per_platform() {
    if cfg!(target_os = "macos") {
        assert_eq!(id("Mod+Shift+G"), Some("edit.findPrevious"));
        // Word for Mac's own keys: ⌘⇧S Save As, ⌘⌥E endnote (⌘⌥D hides the Dock), ⌘⌥A comment, ⌃H Replace.
        assert_eq!(id("Mod+Shift+S"), Some("file.saveAs"));
        assert_eq!(id("Mod+Alt+E"), Some("references.endnote"));
        assert_eq!(id("Mod+Alt+D"), None);
        assert_eq!(id("Mod+Alt+A"), Some("review.newComment"));
        assert_eq!(id("Ctrl+H"), Some("edit.findReplace"));
        assert_eq!(id("Mod+Alt+L"), Some("review.spelling"));
    } else {
        assert_eq!(id("Mod+Shift+S"), Some("styles.apply"));
        assert_eq!(id("Mod+Alt+D"), Some("references.endnote"));
        assert_eq!(id("Mod+Alt+A"), None);
        assert_eq!(id("Mod+Shift+G"), Some("review.wordCount"));
        assert_eq!(id("Mod+Alt+C"), Some("text.copyright"));
        assert_eq!(id("Mod+Alt+H"), Some("format.highlight"));
        assert_eq!(id("Alt+Shift+L"), Some("outline.firstLineOnly"));
        assert_eq!(id("Alt+Shift+>"), Some("references.nextFootnote"));
        assert_eq!(id("Alt+Shift+."), Some("references.nextFootnote"));
        assert_eq!(id("Alt+Shift+R"), Some("layout.linkToPrevious"));
        assert_eq!(id("Alt+F3"), Some("insert.autoText"));
        assert_eq!(id("Alt+F2"), Some("file.saveAs"));
        assert_eq!(id("Alt+F1"), Some("fields.next"));
    }
}

#[test]
fn word_default_shortcuts_resolve_function_keys() {
    assert_eq!(id("Mod+PageUp"), Some("edit.findPrevious"));
    // The Spike: Word's Ctrl+F3 and Ctrl+Shift+F3 (⌘F3 and ⌘⇧F3 in Word for Mac).
    assert_eq!(id("Mod+F3"), Some("edit.spike"));
    assert_eq!(id("Mod+Shift+F3"), Some("edit.insertSpike"));
    assert_eq!(id("F3"), Some("edit.findNext"));
    assert_eq!(id("Shift+F3"), Some("format.changeCase"));
    assert_eq!(id("F12"), Some("file.saveAs"));
    assert_eq!(id("Mod+Alt+M"), Some("review.newComment"));
    assert_eq!(id("Mod+H"), Some("edit.findReplace"));
}

#[test]
fn reset_shortcuts_remove_direct_formatting_but_keep_styles() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello world"}));
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "para.indent", json!({}));
    run(&mut s, "para.alignCenter", json!({}));
    run(&mut s, "para.reset", json!({}));
    let props = s.doc.para_at(&Pos::body(0, 0)).unwrap().props.clone();
    assert_eq!(props.style.as_deref(), Some("Heading1"), "Ctrl+Q keeps the paragraph style");
    assert_eq!((props.indent_left, props.align), (None, None), "Ctrl+Q drops direct paragraph formatting");

    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "format.resetChar", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(1).bold, None, "Ctrl+Space drops direct character formatting");
    assert_eq!(p.props.style.as_deref(), Some("Heading1"), "Ctrl+Space leaves the paragraph alone");

    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.emDash", json!({}));
    assert_eq!(text(&s), "Hello world—");
}

#[test]
fn custom_dictionary_words_persist_through_their_file() {
    // #40: adding a word through a command writes it to the session's file, and loading
    // that file makes the word known (only the desktop app wires this up).
    let dir = std::env::temp_dir().join(format!("wc-engdict-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("custom-dictionary.txt");
    let _ = std::fs::remove_file(&path);
    let mut s = s();
    s.user_dict_path = Some(path.clone());
    run(&mut s, "review.addToDictionary", json!({"word": "zxqengpersist"}));
    let saved = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(saved.lines().any(|l| l == "zxqengpersist"), "the word reaches the file: {saved:?}");
    run(&mut s, "file.options", json!({"removeWords": ["zxqengpersist"]}));
    let saved = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(!saved.lines().any(|l| l == "zxqengpersist"), "removal reaches the file too: {saved:?}");

    // Ignore All is for this session only, as in Word: the word is accepted, never saved.
    assert!(!wordcraft_proof::is_correct("zxqengignored"));
    run(&mut s, "review.ignoreAll", json!({"word": "zxqengignored"}));
    assert!(wordcraft_proof::is_correct("zxqengignored"));
    assert!(!wordcraft_proof::user_dictionary().contains(&"zxqengignored".to_string()));
    let saved = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(!saved.contains("zxqengignored"), "Ignore All doesn't reach the file: {saved:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn confined_sessions_neither_save_the_dictionary_nor_choose_folders_outside_their_roots() {
    // #3737 (findings 34 and WC-35): the custom dictionary file and the folder settings are
    // held to the automation roots like any other path.
    let base = scratch_dir("dict-roots");
    for d in ["in", "out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let roots = crate::file_access::AutomationRoots::new(Some(&base.join("in")), Some(&base.join("out"))).unwrap();
    let mut s = s();
    let outside = base.join("outside/custom-dictionary.txt");
    s.user_dict_path = Some(outside.clone());
    let confined = |s: &mut Session, cmd: &str, v: serde_json::Value| crate::file_access::confine(roots.as_ref(), || s.run(cmd, &v));
    assert!(confined(&mut s, "review.addToDictionary", json!({"word": "zxqengconfined"})).is_ok());
    assert!(wordcraft_proof::user_dictionary().contains(&"zxqengconfined".to_string()), "kept in memory");
    assert!(!outside.exists(), "nothing written outside the write root");
    for key in ["dictionariesFolder", "userTemplatesFolder"] {
        assert!(confined(&mut s, "file.options", json!({key: base.join("outside").to_string_lossy()})).is_err(), "{key}");
    }
    assert_eq!((s.prefs.dictionaries_folder.as_deref(), s.prefs.user_templates_folder.as_deref()), (None, None));
    // A folder inside the read root is still taken.
    let inside = base.join("in");
    assert!(confined(&mut s, "file.options", json!({"userTemplatesFolder": inside.to_string_lossy()})).is_ok());
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(s.prefs.user_templates_folder.as_deref(), Some(inside.to_string_lossy().as_ref()));
}

#[test]
fn add_to_dictionary_takes_only_words_the_file_keeps() {
    // #40: the same rule as loading the dictionary file, so nothing added is dropped on the next load.
    let mut s = s();
    for bad in ["two words", "tab\tword", "bell\u{7}", &"x".repeat(65)] {
        assert!(s.run("review.addToDictionary", &json!({"word": bad})).is_err(), "{bad:?}");
        assert!(!wordcraft_proof::user_dictionary().iter().any(|w| w == &bad.to_lowercase()));
    }
    run(&mut s, "review.addToDictionary", json!({"word": " zxqengtrimmed "}));
    assert!(wordcraft_proof::user_dictionary().contains(&"zxqengtrimmed".to_string()));
}

#[test]
fn non_english_runs_are_not_proofed() {
    // #100: runs tagged with a proofing language that has no dictionary are skipped
    // by both proofing paths; English still is proofed.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "ąžuolas labas"}));
    let before = run(&mut s, "review.issues", json!({}));
    assert!(!before.as_array().map(|a| a.is_empty()).unwrap_or(true), "default language flags the words: {before}");

    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "lt-LT"}));
    let after = run(&mut s, "review.issues", json!({}));
    assert_eq!(after, json!([]), "Lithuanian runs are skipped: {after}");

    // English typos are still caught.
    let mut e = Session::new(wordcraft_doc::Document::new());
    run(&mut e, "text.insert", json!({"text": "Hello helo world"}));
    let en = run(&mut e, "review.issues", json!({}));
    assert_eq!(en.as_array().map(|a| a.len()).unwrap_or(0), 1, "English typo still flagged: {en}");
}

#[test]
fn typing_enter_undo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "text.insert", json!({"text": " world"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Second"}));
    assert_eq!(text(&s), "Hello world\nSecond");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "Hello world\n");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "Hello world");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "Hello world");
}

/// A command that changes nothing is not an edit (#23): no undo step, no unsaved changes
/// and no relayout.
#[test]
fn a_command_that_changes_nothing_is_not_an_edit() {
    let mut s = s();
    let rev = s.rev();
    // Backspace at the start of the document: there is nothing to delete.
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(s.undo_depth(), 0, "no undo step");
    assert!(!s.dirty, "no unsaved changes");
    assert_eq!(s.rev(), rev, "no relayout");
    // Typing nothing is the same.
    run(&mut s, "text.insert", json!({"text": ""}));
    assert_eq!(s.undo_depth(), 0, "no undo step");
    assert!(!s.dirty, "no unsaved changes");
    // A command that changes something is an edit.
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    assert_eq!(s.undo_depth(), 1, "one undo step");
    assert!(s.dirty, "the document is edited");
    // Setting the alignment a paragraph already has changes nothing.
    let depth = s.undo_depth();
    run(&mut s, "para.align", json!({"value": "center"}));
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    let depth = s.undo_depth();
    run(&mut s, "para.align", json!({"value": "center"}));
    assert_eq!(s.undo_depth(), depth, "no undo step");
    // A command that changes nothing leaves the unsaved-changes flag and the layout revision as
    // they were, even when it calls `touch` itself: a saved document is not marked edited for
    // nothing, and its layout is not laid out again.
    let mut clean = Session::new(wordcraft_doc::Document::new());
    run(&mut clean, "para.align", json!({"value": "left"}));
    clean.dirty = false;
    let rev = clean.rev();
    run(&mut clean, "para.align", json!({"value": "left"}));
    assert!(!clean.dirty, "no unsaved changes");
    assert_eq!(clean.rev(), rev, "no relayout");
    assert_eq!(clean.undo_depth(), 1, "only the first align is a step");
}

/// Five hundred commands that change nothing used to fill the whole undo stack and push the
/// user's own edits out of it, so they could never be undone again (#23).
#[test]
fn no_op_commands_do_not_evict_the_undo_history() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "caret.docStart", json!({}));
    for _ in 0..600 {
        run(&mut s, "text.backspace", json!({}));
    }
    assert_eq!(s.undo_depth(), 1, "the typing step is still there");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "", "the typed text is still undoable");
}

/// A command that changes nothing does not throw the redo stack away (#23).
#[test]
fn a_no_op_command_keeps_the_redo_stack() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.can_redo());
    run(&mut s, "text.backspace", json!({}));
    assert!(s.can_redo(), "the redo stack survives a command that changed nothing");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "Hello");
}

/// Undo and Redo move between steps even when a step's document is the one showing: they are
/// never taken back as commands that changed nothing, so Undo can't get stuck on such a step.
#[test]
fn undo_moves_past_a_step_that_changed_nothing() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    s.checkpoint("Nothing");
    let depth = s.undo_depth();
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.undo_depth(), depth - 1);
    assert!(s.can_redo());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.undo_depth(), depth);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "");
}

/// Commands that run as one undo step (`join_next_undo`: a dialog's OK, the frames of a drag):
/// a group whose first command changes nothing has no step to merge into, so the next command
/// in it starts the group's step instead of merging into an older edit (#23 follow-up).
#[test]
fn a_joined_command_starts_its_group_when_the_first_changed_nothing() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let (depth, margins) = (s.undo_depth(), (s.doc.last_section.margin_left, s.doc.last_section.margin_top));
    // Portrait on a portrait page changes nothing.
    run(&mut s, "layout.orientation", json!({"value": "portrait"}));
    assert_eq!(s.undo_depth(), depth, "the first command of the group is no edit");
    // The next command of the group starts the step the group needs.
    s.join_next_undo();
    run(&mut s, "layout.margins", json!({"left": 100.0}));
    assert_eq!(s.doc.last_section.margin_left, 100.0);
    assert_eq!(s.undo_depth(), depth + 1, "the group's one step");
    // A command that changes nothing in the middle of the group leaves that step alone, and
    // the next one merges into it.
    s.join_next_undo();
    run(&mut s, "layout.orientation", json!({"value": "portrait"}));
    s.join_next_undo();
    run(&mut s, "layout.margins", json!({"top": 60.0}));
    assert_eq!(s.undo_depth(), depth + 1, "still the group's one step");
    // Undo takes the whole group back and no more: the typing is still there.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((s.doc.last_section.margin_left, s.doc.last_section.margin_top), margins, "the margins the group started from");
    assert_eq!(text(&s), "Hello");
}

/// Bold with nothing selected arms the formatting for the next word: the document is the
/// same, so only the caret's formatting changed — and that is an edit (#23).
#[test]
fn a_command_that_only_arms_the_caret_formatting_is_an_edit() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "caret.docStart", json!({}));
    let depth = s.undo_depth();
    run(&mut s, "format.bold", json!({}));
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    // Undo takes the armed formatting back, as Word does (docs/typing-parity.md).
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "text.insert", json!({"text": "x"}));
    assert_eq!(s.doc.para_at(&s.sel.focus).map(|p| p.props_of_char(0).bold), Some(None));
}

/// The Undo button's list (#701): several steps undone at once, newest first.
#[test]
fn undo_to_takes_several_steps_back_at_once() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "World"}));
    let h = run(&mut s, "edit.undoHistory", json!({}));
    assert_eq!(h["undo"], json!(["Typing", "New Paragraph", "Bold", "Typing"]));
    assert_eq!(h["depth"], 4);
    assert_eq!(s.undo_history(), ["Typing", "New Paragraph", "Bold", "Typing"]);
    // Three steps in one command: World, the paragraph and the bold go; Hello stays.
    let r = run(&mut s, "edit.undoTo", json!({"count": 3}));
    assert_eq!(r["undone"], 3);
    assert_eq!(r["depth"], 1);
    assert_eq!(text(&s), "Hello");
    assert_eq!(s.doc.para_at(&s.sel.focus).map(|p| p.props_of_char(1).bold), Some(None));
    assert_eq!(s.undo_history(), ["Typing"]);
    // Each undone step is one redo step.
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "Hello\nWorld");
    // More than the depth undoes everything.
    let r = run(&mut s, "edit.undoTo", json!({"count": 1e300}));
    assert_eq!(r["undone"], 4);
    assert_eq!(text(&s), "");
    assert!(!s.can_undo());
    assert!(s.run("edit.undoTo", &json!({"count": 1})).is_err(), "nothing to undo");
}

#[test]
fn undo_to_rejects_bad_counts_and_leaves_the_document_alone() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "text.newParagraph", json!({}));
    let before = (s.doc.clone(), s.undo_depth());
    for junk in
        [json!({}), json!(null), json!({"count": 0}), json!({"count": -3}), json!({"count": "2"}), json!({"count": [2]}), json!({"count": -1e308})]
    {
        assert!(s.run("edit.undoTo", &junk).is_err(), "{junk}");
        assert_eq!((s.doc.clone(), s.undo_depth()), before, "{junk}");
    }
    // A fraction counts whole steps.
    let r = run(&mut s, "edit.undoTo", json!({"count": 1.9}));
    assert_eq!(r["undone"], 1);
    assert_eq!(text(&s), "Hello");
}

#[test]
fn undo_history_is_capped() {
    let mut s = s();
    for i in 0..(crate::UNDO_LIST_MAX + 20) {
        run(&mut s, "para.indents", json!({"left": i as f64}));
    }
    assert_eq!(s.undo_history().len(), crate::UNDO_LIST_MAX);
    assert_eq!(run(&mut s, "edit.undoHistory", json!({}))["undo"].as_array().map(Vec::len), Some(crate::UNDO_LIST_MAX));
}

#[test]
fn joined_commands_are_one_undo_step() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "para.indents", json!({"left": 9.0}));
    run(&mut s, "para.indents", json!({"left": 18.0}));
    for x in [27.0, 36.0] {
        s.join_next_undo();
        run(&mut s, "para.indents", json!({"left": x}));
    }
    let indent = |s: &Session| s.doc.para_at(&s.sel.focus).and_then(|p| p.props.indent_left);
    assert_eq!(indent(&s), Some(36.0));
    // One undo reverts the whole second drag, not just its last frame.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(indent(&s), Some(9.0));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(indent(&s), None);
    assert_eq!(text(&s), "Hello");
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(indent(&s), Some(36.0));
}

#[test]
fn restore_takes_a_command_back_exactly() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "alpha beta"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "gamma"}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.can_redo());
    let (doc, sel, labels, depth, rev) = (s.doc.clone(), s.sel.clone(), s.undo_labels(), s.undo_depth(), s.rev());
    s.dirty = false;
    let snap = s.edit_snapshot();
    assert_eq!(snap.doc(), &doc);
    assert_eq!(snap.undo_depth(), depth);
    assert!(!snap.dirty());
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    assert_ne!(s.doc, doc);
    assert_eq!(s.undo_depth(), depth + 1);
    assert!(!s.can_redo(), "a new command clears redo");
    s.restore(snap);
    assert_eq!(s.doc, doc);
    assert_eq!(s.sel, sel);
    assert_eq!(s.undo_labels(), labels, "the command's undo step is gone");
    assert!(s.can_redo(), "redo is back");
    assert!(!s.dirty);
    assert!(s.rev() > rev, "the layout is recomputed");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "alpha beta\ngamma");
}

#[test]
fn restore_is_exact_at_the_undo_limit() {
    // 510 steps, each with its own indent: the stack holds the newest 500.
    let build = || {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "alpha"}));
        for i in 0..510 {
            run(&mut s, "para.indents", json!({"left": f64::from(i)}));
        }
        s
    };
    // What undo walks through: (label, indent) for every step, newest first.
    let walk = |s: &mut Session| {
        let mut steps = Vec::new();
        while let Some(label) = s.undo_label().map(str::to_string) {
            s.undo();
            steps.push((label, s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props.indent_left)));
        }
        steps
    };
    let expected = walk(&mut build());
    assert_eq!(expected.len(), 500);
    // One command, and several (each pushes the oldest step out of the full stack).
    for n in [1, 2, 4] {
        let mut s = build();
        let snap = s.edit_snapshot();
        for _ in 0..n {
            run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
        }
        assert_eq!(s.undo_label(), Some("Table"));
        assert_eq!(s.undo_depth(), 500);
        s.restore(snap);
        assert_eq!(walk(&mut s), expected, "{n} command(s): the oldest steps are back, the commands' steps are gone");
    }
    // A command that fails at the limit puts its eviction back with the stacks, so a later
    // restore doesn't bring back a step that never left.
    let mut s = build();
    let snap = s.edit_snapshot();
    assert!(s.run("para.align", &json!({"value": "bogus"})).is_err());
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    s.restore(snap);
    assert_eq!(walk(&mut s), expected, "a failed command between snapshot and restore");
    // An automatic change recorded after the fact (`push_undo`) at the limit counts its eviction.
    let mut s = build();
    let snap = s.edit_snapshot();
    let (doc, sel) = (s.doc.clone(), s.sel.clone());
    s.push_undo("AutoCorrect", doc, sel);
    s.restore(snap);
    assert_eq!(walk(&mut s), expected, "push_undo between snapshot and restore");
}

#[test]
fn restore_keeps_the_open_undo_step() {
    // Typing in progress: text typed after the restore joins the same undo step.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "alpha"}));
    let depth = s.undo_depth();
    let snap = s.edit_snapshot();
    run(&mut s, "para.alignCenter", json!({}));
    s.restore(snap);
    run(&mut s, "text.insert", json!({"text": " beta"}));
    assert_eq!(s.undo_depth(), depth, "still one typing step");
    // A drag in progress (`join_next_undo`): its next frame joins the same undo step.
    run(&mut s, "para.indents", json!({"left": 9.0}));
    s.join_next_undo();
    let depth = s.undo_depth();
    let snap = s.edit_snapshot();
    run(&mut s, "para.indents", json!({"left": 18.0}));
    s.restore(snap);
    run(&mut s, "para.indents", json!({"left": 27.0}));
    assert_eq!(s.undo_depth(), depth, "still one drag step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props.indent_left), None);
}

#[test]
fn failed_command_keeps_undo_and_redo() {
    // A command that fails after its undo checkpoint leaves the history exactly as it was:
    // the redo stack survives, and no empty undo step is added.
    for (id, params) in [("text.insert", json!({})), ("para.align", json!({"value": "bogus"}))] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "Hello"}));
        run(&mut s, "edit.undo", json!({}));
        assert!(s.can_redo());
        assert!(s.run(id, &params).is_err(), "{id} should fail");
        assert!(s.can_redo(), "{id}: redo lost");
        assert!(!s.can_undo(), "{id}: failed command left an undo step");
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(text(&s), "Hello", "{id}");
    }

    // At the undo limit, the oldest step is not evicted by a command that fails.
    let mut s = s();
    let steps = 600;
    for i in 1..=steps {
        run(&mut s, "para.indents", json!({"left": i as f32}));
    }
    let kept = s.undo_labels().len();
    assert!(kept < steps, "the history should be at its limit");
    assert!(s.run("para.align", &json!({"value": "bogus"})).is_err());
    assert_eq!(s.undo_labels().len(), kept);
    while s.can_undo() {
        run(&mut s, "edit.undo", json!({}));
    }
    let indent = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.indent_left);
    assert_eq!(indent, Some((steps - kept) as f32));
}

#[test]
fn failed_command_restores_history_changed_by_nested_commands() {
    // `file.inspect` runs `review.deleteComment` (allowed in a comments-only document, and it
    // checkpoints), then `review.acceptAll` (refused). The refusal must leave the undo and
    // redo stacks exactly as they were, including at and next to the history limit, where the
    // outer and nested checkpoints each evict the oldest step.
    fn timeline(s: &mut Session) -> Vec<(Vec<String>, wordcraft_doc::Document)> {
        let mut out = Vec::new();
        // Straight through the history (the commands won't undo past Restrict Editing).
        while s.can_redo() {
            s.redo();
        }
        loop {
            // Comments are stamped with the time to the second, and the two sessions compared
            // are built a moment apart, so leave the stamp out of the comparison.
            let mut doc = s.doc.clone();
            doc.comments.values_mut().for_each(|c| {
                c.date.clear();
                c.date_utc = None;
            });
            out.push((s.undo_labels(), doc));
            if !s.can_undo() {
                return out;
            }
            s.undo();
        }
    }
    let kept = {
        let mut s = s();
        for i in 1..=600 {
            run(&mut s, "para.indents", json!({"left": i as f32}));
        }
        s.undo_labels().len()
    };
    // Below the limit there is room for a redo step too; at the limit an Undo would leave one free.
    for (len, redo) in [(kept - 1, Some("New Comment")), (kept, None)] {
        let session = || {
            let mut s = s();
            run(&mut s, "text.insert", json!({"text": "note"}));
            for i in 1..=len + usize::from(redo.is_some()) - 4 {
                run(&mut s, "para.indents", json!({"left": i as f32}));
            }
            run(&mut s, "review.restrict", json!({"mode": "comments"}));
            run(&mut s, "review.newComment", json!({"text": "one"}));
            run(&mut s, "review.newComment", json!({"text": "two"}));
            if redo.is_some() {
                run(&mut s, "edit.undo", json!({}));
            }
            assert_eq!(s.undo_labels().len(), len);
            assert_eq!(s.redo_label(), redo);
            s
        };
        let mut failed = session();
        let err = failed.run("file.inspect", &json!({"remove": ["comments", "revisions"]}));
        assert!(matches!(err, Err(crate::CmdError::Disabled(_))), "{len}: {err:?}");
        assert_eq!(failed.undo_labels().len(), len, "{len}: undo steps");
        assert_eq!(failed.redo_label(), redo, "{len}: redo step");
        assert!(timeline(&mut failed) == timeline(&mut session()), "{len}: undo history changed");
    }
}

#[test]
fn backspace_and_delete() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "abc"}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(text(&s), "ab");
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "cd"}));
    run(&mut s, "caret.home", json!({}));
    run(&mut s, "text.backspace", json!({}));
    // Enter capitalised the first word, as Word's AutoCorrect does.
    assert_eq!(text(&s), "Abcd");
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(text(&s), "bcd");
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.deleteWordBack", json!({}));
    assert_eq!(text(&s), "");
}

/// Backspace at the start of the paragraph after a table nested in a cell goes into the nested
/// table's last cell, not into the last cell of the table around it.
#[test]
fn backspace_after_a_nested_table_stays_in_its_cell() {
    let mut s = s();
    run(&mut s, "table.quick", json!({"kind": "matrix"}));
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, 1, 1, 0], "off": 0}}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let outer = s.doc.body.get(1).and_then(|b| b.as_table()).expect("the outer table");
    let blocks = &outer.rows[1].cells[1].blocks;
    let nested = blocks.iter().position(|b| b.as_table().is_some()).expect("a nested table");
    assert!(nested + 1 < blocks.len(), "a paragraph follows the nested table");
    let after = nested as u32 + 1;
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, 1, 1, after], "off": 0}}));
    run(&mut s, "text.backspace", json!({}));
    // The nested table's last cell: row 1, cell 1, its first paragraph.
    assert_eq!(s.sel.focus.path.0, vec![1, 1, 1, after - 1, 1, 1, 0]);
}

#[test]
fn bold_toggles_selection_and_caret_word() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "make this bold"}));
    run(&mut s, "select.text", json!({"text": "this"}));
    run(&mut s, "format.bold", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(5).bold, Some(true));
    assert_eq!(p.props_of_char(0).bold, None);
    run(&mut s, "format.bold", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(5).bold, Some(false));
    // Caret inside a word formats the word.
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [0], "off": 11}}));
    run(&mut s, "format.italic", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(10).italic, Some(true));
    assert_eq!(p.props_of_char(13).italic, Some(true));
    assert_eq!(p.props_of_char(9).italic, None);
}

#[test]
fn border_toggles_and_saves() {
    use wordcraft_doc::props::Border;
    let border = |s: &Session| s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(5).border;
    // Saved .docx carries the border iff it is on (read back: the reader maps only `w:bdr` to `border`).
    let docx_has_bdr = |s: &Session| {
        let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        back.para_at(&Pos::body(0, 0)).unwrap().props_of_char(5).border.is_some()
    };
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "make this boxed"}));
    run(&mut s, "select.text", json!({"text": "this"}));
    run(&mut s, "format.border", json!({}));
    assert_eq!(border(&s), Some(Border::single(0.5)));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(0).border, None);
    assert!(docx_has_bdr(&s));
    run(&mut s, "format.border", json!({}));
    assert_eq!(border(&s), None);
    run(&mut s, "format.border", json!({"value": true}));
    run(&mut s, "format.border", json!({"value": true}));
    assert_eq!(border(&s), Some(Border::single(0.5)));
    run(&mut s, "format.border", json!({"value": false}));
    assert_eq!(border(&s), None);
    assert!(!docx_has_bdr(&s));
}

#[test]
fn pending_format_applies_to_typing() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "a "}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "text.insert", json!({"text": "b"}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(2).bold, Some(true));
    assert_eq!(p.props_of_char(0).bold, None);
}

#[test]
fn styles_and_lists() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Title"}));
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    // Heading's next style is Normal.
    assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().props.style.as_deref(), Some("Normal"));
    run(&mut s, "para.bullets", json!({}));
    run(&mut s, "text.insert", json!({"text": "one"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "two"}));
    let n1 = s.doc.para_at(&Pos::body(1, 0)).unwrap().props.numbering.unwrap();
    let n2 = s.doc.para_at(&Pos::body(2, 0)).unwrap().props.numbering.unwrap();
    assert_eq!(n1.num, n2.num);
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.newParagraph", json!({})); // empty item ends the list
    assert!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.is_none_or(|n| n.num == 0));
    // "1. " autoformat.
    run(&mut s, "text.insert", json!({"text": "1."}));
    run(&mut s, "text.insert", json!({"text": " "}));
    assert!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.is_some_and(|n| n.num != 0));
}

#[test]
fn applying_a_heading_clears_list_numbering() {
    // Applying a heading (or Title/Normal) to a numbered paragraph removes its direct list
    // numbering. A dead `&& id == "Normal"` term used to let only Normal clear it, so headings
    // kept the numbering.
    for style_cmd in ["para.heading1", "para.heading2", "para.normal"] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "item"}));
        run(&mut s, "para.bullets", json!({}));
        assert!(
            s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.is_some_and(|n| n.num != 0),
            "{style_cmd}: expected list numbering before applying the style"
        );
        run(&mut s, style_cmd, json!({}));
        assert!(
            s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.is_none_or(|n| n.num == 0),
            "{style_cmd}: should clear direct list numbering"
        );
    }
}

#[test]
fn find_replace() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "cat dog cat\nCat bird"}));
    let r = run(&mut s, "edit.find", json!({"text": "cat"}));
    assert_eq!(r["count"], 3);
    let r = run(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox", "matchCase": true}));
    assert_eq!(r["replaced"], 2);
    assert_eq!(text(&s), "fox dog fox\nCat bird");
    let r = run(&mut s, "edit.find", json!({"text": "\\b\\w{3}\\b", "regex": true, "matchCase": false}));
    assert_eq!(r["count"], 4);
}

#[test]
fn find_replace_special_codes() {
    // ^p finds paragraph marks across paragraphs; Replace All with ^p splits paragraphs and is one undo step.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "one\ntwo\nthree"}));
    let paras = |s: &Session| s.doc.para_paths(StoryRef::Body).len();
    assert_eq!(run(&mut s, "edit.find", json!({"text": "^p"}))["count"], 2);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "o^pt", "with": "O+T"}))["replaced"], 1);
    assert_eq!((text(&s).as_str(), paras(&s)), ("one\ntwO+Three", 2));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((text(&s).as_str(), paras(&s)), ("one\ntwo\nthree", 3));
    run(&mut s, "document.setText", json!({"text": "a, b, c"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": ", ", "with": "^p"}))["replaced"], 2);
    assert_eq!((text(&s).as_str(), paras(&s)), ("a\nb\nc", 3));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((text(&s).as_str(), paras(&s)), ("a, b, c", 1));
    // ^t, ^# and ^& (the found text), ^^ is a caret.
    run(&mut s, "document.setText", json!({"text": "x\ty 12 a^b"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "^t", "with": " "}))["replaced"], 1);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "^#", "with": "<^&>"}))["replaced"], 2);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "^^", "with": "-"}))["replaced"], 1);
    assert_eq!(text(&s), "x y <1><2> a-b");
    // `special: false` reads the text literally.
    assert_eq!(run(&mut s, "edit.find", json!({"text": "^#", "special": false}))["count"], 0);
}

#[test]
fn find_replace_wildcards() {
    let mut s = s();
    // Groups and back-references swap words; Use wildcards matches case.
    run(&mut s, "document.setText", json!({"text": "Smith, John and Doe, Jane; smith, x"}));
    let r = run(&mut s, "edit.replaceAll", json!({"text": "([A-Z][a-z]@), ([A-Z][a-z]@)", "with": "\\2 \\1", "wildcards": true}));
    assert_eq!(r["replaced"], 2);
    assert_eq!(text(&s), "John Smith and Jane Doe; smith, x");
    // < and > anchor to word starts and ends; * is the shortest run; {n} counts.
    run(&mut s, "document.setText", json!({"text": "cat concat cats scatter"}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "<cat", "wildcards": true}))["count"], 2);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat>", "wildcards": true}))["count"], 2);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "<c*t>", "wildcards": true}))["count"], 2);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "t{2}", "wildcards": true}))["count"], 1);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[!c]at", "wildcards": true}))["count"], 0);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[!s ]cat", "wildcards": true}))["count"], 1);
    // Options of the normal search: prefix, suffix, ignore punctuation.
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat", "wildcards": false, "matchPrefix": true}))["count"], 2);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat", "matchPrefix": false, "matchSuffix": true}))["count"], 2);
    run(&mut s, "document.setText", json!({"text": "e.g. eg"}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "eg", "matchSuffix": false, "ignorePunct": true}))["count"], 2);
}

#[test]
fn regex_replace_expands_groups() {
    // #4177: with Regular expressions on, Replace with expands capture groups.
    let rep = |text: &str, q: &str, with: &str| rep_mode(text, q, with, json!({"regex": true}));
    let names = "Smith, John and Doe, Jane";
    // The issue's cases: `\n` and `$n` swap words; `${name}` names a group.
    assert_eq!(rep(names, r"(\w+), (\w+)", r"\2 \1"), "John Smith and Jane Doe");
    assert_eq!(rep(names, r"(\w+), (\w+)", "$2 $1"), "John Smith and Jane Doe");
    assert_eq!(rep("2024-05", r"(?<y>\d{4})-", "${y}/"), "2024/05");
    // Braced numbers, `$0` / `${0}` (whole match), `$$` (a dollar sign).
    assert_eq!(rep("ab", "(a)(b)", "${2}${1}"), "ba");
    assert_eq!(rep("ab", "a", "[$0|${0}]"), "[a|a]b");
    assert_eq!(rep("5", r"(\d)", "$$$1"), "$5");
    // `$name` takes the longest run of letters, digits and `_` (the regex crate's rule).
    assert_eq!(rep("ab", "(?<x>a)", "$x_"), "b");
    assert_eq!(rep("ab", "(?<x>a)", "${x}_"), "a_b");
    assert_eq!(rep("ab", "(a)", "$1a"), "b");
    // Missing, unknown and out-of-range groups expand to nothing.
    assert_eq!(rep("ab", "(a)", "<$2>"), "<>b");
    assert_eq!(rep("ab", "(a)", r"<\9>"), "<>b");
    assert_eq!(rep("ab", "(a)", "<${nope}>"), "<>b");
    assert_eq!(rep("ab", "(a)", "<$99999999999999999999999>"), "<>b");
    assert_eq!(rep("ab", "(a)|(z)", "<$2>"), "<>b");
    // A `$` that starts no group, and an unclosed brace, stay as written.
    assert_eq!(rep("ab", "a", "$ ${x"), "$ ${xb");
    // `\t` a tab, `\\` a backslash; other escapes stay as written.
    assert_eq!(rep("ab", "a", r"\t\\\q"), "\t\\\\qb");
    // Word's `^` codes still work.
    assert_eq!(rep("ab", "a", "^&^t^&"), "a\tab");
    // `\$` and `\^` keep the character as typed (CodeRabbit on #4177).
    assert_eq!(rep("ab", "(a)", r"\$1\^p"), "$1^pb");
    // `special: false` turns off the `^` codes only; groups still expand (CodeRabbit on #4177).
    assert_eq!(rep_mode("ab", "(a)", "$1^t", json!({"regex": true, "special": false})), "a^tb");
    // `\n` is a paragraph mark: it splits the paragraph, as `^p` does, in one undo step.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "one, two"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": r"(\w+), (\w+)", "with": r"$1\n$2", "regex": true}))["replaced"], 1);
    assert_eq!(text(&s), "one\ntwo");
    assert_eq!(s.doc.para_paths(StoryRef::Body).len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "one, two");
    run(&mut s, "document.setText", json!({"text": "a-b"}));
    run(&mut s, "edit.replaceAll", json!({"text": "-", "with": "^p", "regex": true}));
    assert_eq!(s.doc.para_paths(StoryRef::Body).len(), 2);
    // Plain and wildcard Replace are unchanged: `$1` is literal there.
    assert_eq!(rep_mode("ab", "a", "$1", json!({})), "$1b");
    assert_eq!(rep_mode("ab", "(a)", "$1\\1", json!({"wildcards": true})), "$1ab");
    // Hostile Replace text never panics.
    run(&mut s, "document.setText", json!({"text": "ab ab"}));
    for with in ["$", "${", "${}", "${0", "$\u{e9}", "\\", "${\u{1F600}}", "$18446744073709551616", "${-1}", "$$$", "\\0\\n\\", "^"] {
        let _ = s.run("edit.replaceAll", &json!({"text": "(a)(?<n>b)?", "with": with, "regex": true}));
    }
}

/// The body text after Replace All of `q` with `with` in `text`, matching case, with `extra` options.
fn rep_mode(text: &str, q: &str, with: &str, extra: serde_json::Value) -> String {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": text}));
    let mut v = json!({"text": q, "with": with, "matchCase": true});
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    run(&mut s, "edit.replaceAll", v);
    self::text(&s)
}

#[test]
fn hostile_find_patterns_are_bounded() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": &"a".repeat(5000)}));
    // Patterns that backtrack catastrophically elsewhere finish quickly (linear-time engine).
    let t = std::time::Instant::now();
    for q in ["(a@)@b", "(*)@b", "(a{1,1000}){1,1000}", "(((((a*)*)*)*)*)b"] {
        let _ = s.run("edit.find", &json!({"text": q, "wildcards": true}));
    }
    assert!(t.elapsed().as_secs() < 20);
    // Bad and oversized patterns are errors, not crashes.
    for q in ["[z-a]", "a{5", "(ab", "\\1", "x{2000}", "[", "@"] {
        assert!(s.run("edit.find", &json!({"text": q, "wildcards": true})).is_err(), "{q}");
    }
    assert!(s.run("edit.find", &json!({"text": "x".repeat(2000), "wildcards": false})).is_err());
    for q in ["^", "^p^", "^u", "^99999999", "^0", "a^"] {
        let _ = s.run("edit.replaceAll", &json!({"text": q, "with": "^&^c\\9^p^", "wildcards": false}));
    }
}

#[test]
fn replace_under_track_changes_skips_deleted_text() {
    // Replace marks the match deleted and leaves it in the text; the next
    // step must not find it again, or Replace never advances.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "one cat two cat"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat"}))["count"], 2);
    let mut remaining = Vec::new();
    for _ in 0..4 {
        remaining.push(run(&mut s, "edit.replace", json!({"text": "cat", "with": "dog"}))["remaining"].clone());
    }
    assert_eq!(remaining, [json!(1), json!(0), json!(0), json!(0)]);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat"}))["count"], 0);
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "one dog two dog");

    // A match that runs across tracked-deleted text is not in the document.
    let mut s = self::s();
    run(&mut s, "document.setText", json!({"text": "cat"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 1}, "focus": {"block": 0, "off": 2}}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(text(&s), "cat", "the deletion is tracked, not applied");
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat"}))["count"], 0);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "t"}))["count"], 1);

    // Without Track Changes, Replace steps through every match as before.
    let mut s = self::s();
    run(&mut s, "document.setText", json!({"text": "one cat two cat"}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "edit.find", json!({"text": "cat"}));
    run(&mut s, "edit.replace", json!({"text": "cat", "with": "dog"}));
    let r = run(&mut s, "edit.replace", json!({"text": "cat", "with": "dog"}));
    assert_eq!(r["remaining"], 0);
    assert_eq!(text(&s), "one dog two dog");
}

#[test]
fn find_reads_the_text_around_tracked_deletions() {
    // `edits` are (start, end) byte ranges deleted with Track Changes on, last first.
    fn tracked(text: &str, edits: &[(usize, usize)]) -> Session {
        let mut s = self::s();
        run(&mut s, "document.setText", json!({"text": text}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        for (a, b) in edits {
            run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": a}, "focus": {"block": 0, "off": b}}));
            run(&mut s, "text.delete", json!({}));
        }
        assert_eq!(text, self::text(&s), "the deletions are tracked, not applied");
        s
    }
    let offs = |r: &serde_json::Value| -> Vec<(u64, u64)> {
        r["matches"].as_array().unwrap().iter().map(|m| (m["start"]["off"].as_u64().unwrap(), m["end"]["off"].as_u64().unwrap())).collect()
    };

    // "aaa" with the first "a" deleted reads "aa": the rejected raw match 0..2 must not hide
    // the live one at 1..3.
    let mut s = tracked("aaa", &[(0, 1)]);
    assert_eq!(offs(&run(&mut s, "edit.find", json!({"text": "aa"}))), [(1, 3)]);
    assert_eq!(offs(&run(&mut s, "edit.find", json!({"text": "^a", "regex": true}))), [(1, 2)]);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "aa", "with": "b"}))["replaced"], 1);

    // Whole words follow the live text: deleting the space joins "cat" and "fish"...
    let mut s = tracked("cat fish", &[(3, 4)]);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "fish", "wholeWord": true}))["count"], 0);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "catfish", "wholeWord": true}))["count"], 0, "a match may not span a deletion");
    assert_eq!(run(&mut s, "edit.find", json!({"text": "fish", "wholeWord": false}))["count"], 1);
    // ...and deleting the "X" leaves "fish" a word of its own.
    let mut s = tracked("cat Xfish", &[(4, 5)]);
    assert_eq!(offs(&run(&mut s, "edit.find", json!({"text": "FISH", "wholeWord": true}))), [(5, 9)]);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "FISH", "wholeWord": true, "matchCase": true}))["count"], 0);

    // Replace and the Find Next/Previous steps agree with Find.
    let mut s = tracked("aaa aaa", &[(4, 5), (0, 1)]);
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "aa"}))["count"], 2);
    assert_eq!(run(&mut s, "edit.findNext", json!({}))["index"], 1);
    assert_eq!(run(&mut s, "edit.findPrevious", json!({}))["index"], 0);
    let r = run(&mut s, "edit.replace", json!({"text": "aa", "with": "b"}));
    assert_eq!(r["remaining"], 1);
}

#[test]
fn find_next_with_options_visits_every_match() {
    // The Find dialog sends its options with each Find Next (#468): every click is one step.
    let opts = json!({"text": "cat", "with": "", "matchCase": false, "wholeWord": false, "regex": false});
    let starts = |text: &str, clicks: usize, cmd: &str| {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": text}));
        run(&mut s, "caret.docStart", json!({}));
        (0..clicks).map(|_| run(&mut s, cmd, opts.clone())["index"].as_u64().unwrap()).collect::<Vec<_>>()
    };
    assert_eq!(starts("cat cat cat cat", 5, "edit.findNext"), [0, 1, 2, 3, 0]);
    assert_eq!(starts("cat cat", 3, "edit.findNext"), [0, 1, 0]);
    assert_eq!(starts("cat cat cat", 4, "edit.findPrevious"), [2, 1, 0, 2]);

    // A new query takes effect at once, without a separate `edit.find`.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "cat dog cat dog"}));
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "edit.findNext", opts.clone())["index"], 0);
    let r = run(&mut s, "edit.findNext", json!({"text": "dog"}));
    assert_eq!((r["index"].clone(), r["count"].clone()), (json!(0), json!(2)));
    assert_eq!(s.sel.ordered().0.off, 4);
}

#[test]
fn clipboard_round_trip() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "alpha beta\ngamma"}));
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 1, "off": 2}}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard_text, "beta\nga");
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(text(&s), "alpha beta\ngammabeta\nga");
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(text(&s), "");
    run(&mut s, "edit.paste", json!({"text": "plain\ntext"}));
    assert_eq!(text(&s), "plain\ntext");
}

/// Equations in the paragraphs of a session's body, as linear text.
fn equations(s: &Session) -> Vec<String> {
    let mut out = Vec::new();
    for b in &s.doc.body {
        if let wordcraft_doc::Block::Para(p) = &**b {
            for o in &p.objects {
                if let wordcraft_doc::para::InlineObject::Equation { linear, .. } = o {
                    out.push(linear.clone());
                }
            }
        }
    }
    out
}

#[test]
fn copying_an_equation_puts_mathml_on_the_clipboard() {
    // #816: other programs get the equation as MathML in HTML, beside the linear text.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Speed "}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.equation", json!({"linear": "y=2x_s", "edit": false}));
    run(&mut s, "select.all", json!({}));
    let r = run(&mut s, "edit.copy", json!({}));
    let html = r["html"].as_str().unwrap_or_default().to_string();
    assert!(html.contains("<math xmlns=\"http://www.w3.org/1998/Math/MathML\">"), "{html}");
    assert!(html.contains("<msub><mrow><mi>x</mi></mrow><mrow><mi>s</mi></mrow></msub>"), "{html}");
    assert!(html.contains("Speed"), "{html}");
    assert_eq!(s.clipboard_html, html);
    assert!(r["text"].as_str().unwrap_or_default().contains("y=2x_s"));
    let clip = r["text"].as_str().unwrap_or_default().to_string();

    // Pasting it into another document reads the equation back from the HTML.
    let mut t = Session::new(wordcraft_doc::Document::new());
    run(&mut t, "edit.paste", json!({"text": clip, "html": html}));
    assert_eq!(equations(&t), vec!["y=2x_s".to_string()]);
    assert!(text(&t).starts_with("Speed"), "{}", text(&t));

    // And back into this one, from our own copy.
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({"text": clip, "html": html}));
    assert_eq!(equations(&s), vec!["y=2x_s".to_string(), "y=2x_s".to_string()]);

    // A copy without an equation carries HTML too (#1523).
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}}));
    let r = run(&mut s, "edit.copy", json!({}));
    assert!(r["html"].as_str().is_some_and(|h| h.contains("Speed") && !h.contains("<math")), "{r}");
}

/// Whether `xml` is one element whose tags all nest and close (enough to check a clipboard
/// payload without an XML parser in the engine's tests).
fn one_balanced_element(xml: &str) -> bool {
    let mut stack: Vec<String> = Vec::new();
    let mut closed_root = false;
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        if closed_root || (stack.is_empty() && !rest[..lt].trim().is_empty()) {
            return false;
        }
        let Some(gt) = rest[lt..].find('>') else { return false };
        let tag = &rest[lt + 1..lt + gt];
        rest = &rest[lt + gt + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            if stack.pop().as_deref() != Some(name.trim()) {
                return false;
            }
            closed_root = stack.is_empty();
        } else if !tag.ends_with('/') {
            stack.push(tag.split_whitespace().next().unwrap_or_default().to_string());
        } else if stack.is_empty() {
            closed_root = true;
        }
    }
    closed_root && stack.is_empty() && rest.trim().is_empty()
}

#[test]
fn copying_one_equation_gives_its_mathml_for_the_math_clipboard_formats() {
    // #1297: Word reads a pasted equation first from the clipboard's "MathML" formats; a copy of
    // one equation alone carries its MathML for the host to put there.
    let cases = [
        ("y=(a+b)/(c-d)", "<mfrac>"),
        ("x_i^2+e^(-t)", "<msubsup>"),
        ("√(x^2+1)+∛8", "<msqrt>"),
        ("∑_(k=1)^n k^2", "<munderover>"),
        ("∫_0^1 f(x)dx", "∫"),
        ("■(1&0@0&1)", "<mtable>"),
    ];
    for (linear, element) in cases {
        let mut s = s();
        run(&mut s, "insert.equation", json!({"linear": linear, "edit": false}));
        run(&mut s, "select.all", json!({}));
        let r = run(&mut s, "edit.copy", json!({}));
        let mathml = r["mathml"].as_str().unwrap_or_default().to_string();
        assert!(mathml.starts_with("<math xmlns=\"http://www.w3.org/1998/Math/MathML\""), "{linear}: {mathml}");
        assert!(mathml.contains(element), "{linear}: {mathml}");
        // Alone on its line, it is a display equation.
        assert!(mathml.contains("display=\"block\""), "{linear}: {mathml}");
        assert!(one_balanced_element(&mathml), "{linear}: not one well-formed element: {mathml}");
        // No XML prolog: the W3C transfer rules want `<math` first, so readers that sniff the
        // start of the text recognise it.
        assert!(!mathml.contains("<?xml"), "{mathml}");
        // Pasting it back (as a MathML reader would get it) gives the same equation.
        let mut t = s_with_paste(&mathml);
        assert_eq!(equations(&t), equations(&s), "{mathml}");
        // Its presentation elements alone, without our linear-format annotation (which Word
        // ignores), still read as an equation of the same shape.
        let bare = match (mathml.find("<annotation"), mathml.find("</annotation>")) {
            (Some(a), Some(b)) => format!("{}{}", &mathml[..a], &mathml[b + "</annotation>".len()..]),
            _ => mathml.clone(),
        };
        t = s_with_paste(&bare);
        assert_eq!(equations(&t).len(), 1, "{linear}: {bare}");
    }
}

fn s_with_paste(html: &str) -> Session {
    let mut t = s();
    run(&mut t, "edit.paste", json!({"text": "", "html": html}));
    t
}

#[test]
fn copying_an_equation_with_text_offers_no_math_clipboard_formats() {
    // #1297: Word takes the "MathML" format before the HTML, so offering it for a copy with text
    // around the equation would drop the text. Text alone has none either.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Speed "}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.equation", json!({"linear": "y=2x_s", "edit": false}));
    run(&mut s, "select.all", json!({}));
    let r = run(&mut s, "edit.copy", json!({}));
    assert!(r.get("mathml").is_none() && r["html"].as_str().is_some_and(|h| h.contains("<math")), "{r}");
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}}));
    let r = run(&mut s, "edit.copy", json!({}));
    assert!(r.get("mathml").is_none(), "{r}");
    // Cut of the equation alone offers it.
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 9}}));
    let r = run(&mut s, "edit.cut", json!({}));
    assert!(r["mathml"].as_str().is_some_and(|m| m.contains("<msub>") && !m.contains("display=")), "an inline equation: {r}");
}

#[test]
fn copying_bold_text_puts_html_on_the_clipboard() {
    // #1523: formatted text reaches other programs as HTML, not only as plain text.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "plain bold"}));
    s.doc.format_range(&Pos::body(0, 6), &Pos::body(0, 10), &|c| c.bold = Some(true)).unwrap();
    run(&mut s, "select.all", json!({}));
    let r = run(&mut s, "edit.copy", json!({}));
    let html = r["html"].as_str().unwrap_or_default().to_string();
    assert!(html.contains("<strong>bold</strong>"), "{html}");
    assert_eq!(s.clipboard_html, html);
}

#[test]
fn copying_symbol_font_text_puts_the_unicode_symbols_on_the_clipboard() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "ok J\u{F0FC}"}));
    s.doc.format_range(&Pos::body(0, 3), &Pos::body(0, 7), &|c| c.font = Some("Wingdings".into())).unwrap();
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard_text, "ok \u{263A}\u{2713}");
    // Pasting our own copy back (the system clipboard holds the mapped text) keeps the stored
    // Wingdings characters, not the Unicode ones.
    run(&mut s, "caret.docEnd", json!({}));
    let clip = s.clipboard_text.clone();
    run(&mut s, "edit.paste", json!({"text": clip}));
    assert_eq!(text(&s), "ok J\u{F0FC}ok J\u{F0FC}");
}

/// The text drawn on the first page.
fn drawn_text(s: &mut Session) -> String {
    let layout = s.layout();
    let draws = wordcraft_layout::display::page_display(&s.doc, &layout.pages[0], &Default::default());
    draws.iter().filter_map(|d| if let wordcraft_layout::display::Draw::Glyphs { text, .. } = d { Some(text.as_str()) } else { None }).collect()
}

#[test]
fn an_inserted_symbol_keeps_its_font_when_the_text_font_changes() {
    // #2036: Insert › Symbol in Wingdings, then the whole document set in Times New Roman: the
    // symbol stays a check mark (as Word keeps a `w:sym` character's font).
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "ab"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.symbol", json!({"char": "\u{F0FC}", "font": "Wingdings"}));
    run(&mut s, "text.insert", json!({"text": "c"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.font", json!({"name": "Times New Roman"}));
    assert_eq!(text(&s), "ab\u{F0FC}c", "the stored character is unchanged");
    assert_eq!(drawn_text(&mut s), "ab\u{2713}c");
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard_text, "ab\u{2713}c");
    // Text typed after the symbol is in the text's font, not the symbol's.
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": "J"}));
    assert_eq!(drawn_text(&mut s), "ab\u{2713}cJ");
    // Clear Formatting keeps the symbol too.
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.clear", json!({}));
    assert_eq!(drawn_text(&mut s), "ab\u{2713}cJ");
}

#[test]
fn format_painter_does_not_carry_a_symbol_font_to_text() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "ok "}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.symbol", json!({"char": "\u{F0FC}", "font": "Wingdings"}));
    run(&mut s, "text.insert", json!({"text": " J"}));
    // Pick up the symbol's formatting and paint it on "J".
    run(&mut s, "select.text", json!({"text": "\u{F0FC}"}));
    run(&mut s, "edit.formatPainter", json!({}));
    run(&mut s, "select.text", json!({"text": "J"}));
    run(&mut s, "edit.pasteFormat", json!({}));
    let shown = drawn_text(&mut s);
    assert!(shown.ends_with('J'), "{shown}");
}

/// "ab" then a Wingdings check mark, with the caret after it.
fn text_then_symbol() -> Session {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "ab"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.symbol", json!({"char": "\u{F0FC}", "font": "Wingdings"}));
    s
}

#[test]
fn a_style_created_next_to_a_symbol_does_not_take_its_font() {
    let mut s = text_then_symbol();
    let r = run(&mut s, "styles.create", json!({"name": "Mine"}));
    let id = r["id"].as_str().unwrap().to_string();
    assert_eq!(s.doc.styles.get(&id).unwrap().chr.sym_font, None);
    assert_eq!(drawn_text(&mut s), "ab\u{2713}");
}

#[test]
fn updating_a_style_to_match_a_symbol_does_not_take_its_font() {
    let mut s = text_then_symbol();
    run(&mut s, "select.text", json!({"text": "\u{F0FC}"}));
    run(&mut s, "styles.updateToMatch", json!({}));
    assert_eq!(s.doc.styles.get("Normal").unwrap().chr.sym_font, None);
    assert_eq!(drawn_text(&mut s), "ab\u{2713}");
}

#[test]
fn replacing_a_symbol_with_text_gives_the_text_no_symbol_font() {
    let mut s = text_then_symbol();
    run(&mut s, "edit.replaceAll", json!({"text": "\u{F0FC}", "with": "OK"}));
    assert_eq!(text(&s), "abOK");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(2).sym_font, None);
    assert_eq!(drawn_text(&mut s), "abOK");
}

fn bold_at(s: &Session, block: usize, off: usize) -> bool {
    let pos = Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(block), off };
    s.doc.para_at(&pos).and_then(|p| p.props_of_char(off).bold).unwrap_or(false)
}

#[test]
fn paste_special_lists_formats_and_pastes_text_without_formatting() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Bold words"}));
    run(&mut s, "select.text", json!({"text": "Bold"}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "edit.copy", json!({}));
    // No format: the formats are listed (and a front end is asked to show the dialog).
    let r = run(&mut s, "edit.pasteSpecial", json!({}));
    let ids: Vec<&str> = r["formats"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["formatted", "text"]);
    assert_eq!(s.ui_requests.last().unwrap()["open"], "pasteSpecial");
    run(&mut s, "caret.docEnd", json!({}));
    let r = run(&mut s, "edit.pasteSpecial", json!({"as": "text"}));
    assert_eq!(r["pastedAs"], "text");
    assert_eq!(text(&s), "Bold wordsBold");
    assert!(bold_at(&s, 0, 0));
    assert!(!bold_at(&s, 0, 11), "unformatted text drops the bold");
    // Formatted keeps it.
    run(&mut s, "edit.pasteSpecial", json!({"as": "formatted"}));
    assert_eq!(text(&s), "Bold wordsBoldBold");
    assert!(bold_at(&s, 0, 15));
    // Text that someone else copied since: our rich copy is no longer offered.
    let r = run(&mut s, "edit.pasteSpecial", json!({"text": "elsewhere"}));
    assert_eq!(r["formats"].as_array().unwrap().len(), 1);
    assert!(s.run("edit.pasteSpecial", &json!({"as": "formatted", "text": "elsewhere"})).is_err());
}

#[test]
fn pasted_rtf_tracked_changes_point_at_their_own_revisions() {
    // Gap 19: RTF tracked changes come with revisions of their own; pasted into a document that
    // already has revisions, they must not point at the document's (index 0 here is "Zed").
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "x"}));
    s.doc.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "Zed", ""));
    run(&mut s, "caret.docEnd", json!({}));
    let rtf = r"{\rtf1{\*\revtbl {Unknown;}{Alice;}}{\revised\revauth1 added}\par}";
    run(&mut s, "edit.pasteSpecial", json!({"as": "rtf", "rtf": rtf}));
    let pos = Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 2 };
    let ins = s.doc.para_at(&pos).and_then(|p| p.props_of_char(2).ins);
    let rev = ins.and_then(|i| s.doc.revisions.get(i as usize));
    assert_eq!(rev.map(|r| (r.kind, r.author.as_str())), Some((wordcraft_doc::RevisionKind::Insert, "Alice")), "{:?}", s.doc.revisions);
}

/// Insertion and deletion revision kinds at body paragraph 0, offset `off`.
fn rev_kinds_at(s: &Session, off: usize) -> (Option<wordcraft_doc::RevisionKind>, Option<wordcraft_doc::RevisionKind>) {
    let pos = Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off };
    let cp = s.doc.para_at(&pos).map(|p| p.props_of_char(off).clone()).unwrap_or_default();
    let kind = |i: Option<u32>| i.and_then(|i| s.doc.revisions.get(i as usize)).map(|r| r.kind);
    (kind(cp.ins), kind(cp.del))
}

const RTF_WITH_DELETION: &str = r"{\rtf1{\*\revtbl {Unknown;}{Bob;}}kept{\deleted\revauthdel1 gone}\par}";

#[test]
fn pasted_rtf_deletions_stay_deleted_under_track_changes() {
    // Gap 19: with Track Changes on the paste is an insertion, and text deleted in the source is
    // inserted-then-deleted (struck), never live text.
    use wordcraft_doc::RevisionKind::{Delete, Insert};
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "x"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "edit.pasteSpecial", json!({"as": "rtf", "rtf": RTF_WITH_DELETION}));
    assert_eq!(rev_kinds_at(&s, 1), (Some(Insert), None), "kept");
    assert_eq!(rev_kinds_at(&s, 5), (Some(Insert), Some(Delete)), "gone");
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "xkept");
}

#[test]
fn paste_merge_formatting_keeps_rtf_deletions() {
    use wordcraft_doc::RevisionKind::Delete;
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "x"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.pasteMerge", json!({"rtf": RTF_WITH_DELETION}));
    assert_eq!(rev_kinds_at(&s, 1), (None, None), "kept");
    assert_eq!(rev_kinds_at(&s, 5), (None, Some(Delete)), "gone");
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "xkept");
}

#[test]
fn pasted_rtf_sections_link_their_headers_to_the_previous() {
    // #1378: the RTF's own header part ids mean nothing in this document; a pasted section
    // break keeps its page but links its headers and footers to the previous section.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "x"}));
    let note = s.doc.add_part(wordcraft_doc::PartKind::Footnote, Vec::new());
    let _comment = s.doc.add_part(wordcraft_doc::PartKind::Comment, Vec::new());
    run(&mut s, "caret.docEnd", json!({}));
    let rtf = r"{\rtf1\ansi\sectd\lndscpsxn\titlepg{\header Head\par}{\footer Foot\par}{\headerf First\par} a\par\sect\sectd{\header Two\par} b\par}";
    run(&mut s, "edit.pasteSpecial", json!({"as": "rtf", "rtf": rtf}));
    let secs = s.doc.sections();
    assert_eq!(secs.len(), 2, "the pasted section break is kept");
    assert!(secs[0].1.landscape, "with its page settings");
    for (_, sp) in &secs {
        for (set, kind) in [(sp.headers, wordcraft_doc::PartKind::Header), (sp.footers, wordcraft_doc::PartKind::Footer)] {
            for id in [set.default, set.first, set.even].into_iter().flatten() {
                assert_eq!(s.doc.parts.get(&id).map(|p| p.kind), Some(kind), "section refers to part {id} (footnote is {note})");
            }
        }
    }
    assert_unlinked(&s);
}

/// Every section break in the body has no headers, footers or first-page setting of its own.
fn assert_unlinked(s: &Session) {
    for (i, sp) in s.doc.sections() {
        assert_eq!((sp.headers, sp.footers, sp.title_page), Default::default(), "section ending at block {i}");
    }
}

#[test]
fn text_from_file_unlinks_inserted_sections_headers() {
    // #1378: Insert › Text from File brings the file's section breaks with their pages, but
    // the file's header and footer ids are its own: they link to the previous section's.
    let dir = std::env::temp_dir().join(format!("wordcraft-text-from-file-sections-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let rtf = r"{\rtf1\ansi\sectd\lndscpsxn\titlepg{\header Head\par}{\footer Foot\par}{\headerf First\par} a\par\sect\sectd{\header Two\par} b\par}";
    let rtf_path = dir.join("sections.rtf");
    std::fs::write(&rtf_path, rtf).unwrap();
    // The same document as DOCX.
    let docx_path = dir.join("sections.docx");
    let src = crate::io::open_bytes("sections.rtf", rtf.as_bytes()).unwrap();
    std::fs::write(&docx_path, crate::io::save_bytes("sections.docx", &src).unwrap()).unwrap();
    for path in [&rtf_path, &docx_path] {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": "x"}));
        s.doc.add_part(wordcraft_doc::PartKind::Footnote, Vec::new());
        run(&mut s, "caret.docEnd", json!({}));
        run(&mut s, "insert.textFromFile", json!({"path": path.to_str().unwrap()}));
        let secs = s.doc.sections();
        assert!(secs.len() >= 2, "{path:?}: the inserted section break is kept");
        assert!(secs[0].1.landscape, "{path:?}: with its page settings");
        assert_unlinked(&s);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn paste_special_html_and_rtf_keep_bold() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "x"}));
    run(&mut s, "caret.docEnd", json!({}));
    let html = "<p>plain <b>strong</b></p>";
    let r = run(&mut s, "edit.pasteSpecial", json!({"html": html, "text": "plain strong"}));
    let ids: Vec<&str> = r["formats"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["html", "text"]);
    run(&mut s, "edit.pasteSpecial", json!({"as": "html", "html": html}));
    assert_eq!(text(&s), "xplain strong");
    assert!(!bold_at(&s, 0, 2));
    assert!(bold_at(&s, 0, 8), "the <b> run stays bold");
    // HTML source copied as text can be pasted as HTML too.
    run(&mut s, "document.setText", json!({"text": ""}));
    run(&mut s, "edit.pasteSpecial", json!({"as": "html", "text": "<b>hi</b> there</p>"}));
    assert_eq!(text(&s), "hi there");
    assert!(bold_at(&s, 0, 0) && !bold_at(&s, 0, 4));
    // RTF.
    run(&mut s, "document.setText", json!({"text": ""}));
    run(&mut s, "edit.pasteSpecial", json!({"as": "rtf", "rtf": r"{\rtf1\ansi {\b bold}\b0  then}"}));
    assert!(text(&s).starts_with("bold"), "{}", text(&s));
    assert!(bold_at(&s, 0, 0));
    // Asking for a format the clipboard lacks is an error, not a panic.
    assert!(s.run("edit.pasteSpecial", &json!({"as": "rtf", "text": "plain"})).is_err());
    assert!(s.run("edit.pasteSpecial", &json!({"as": "bogus", "text": "plain"})).is_err());
    let huge = "a".repeat(crate::cmd::paste::MAX_PASTE_BYTES + 1);
    assert!(s.run("edit.pasteSpecial", &json!({"as": "text", "text": huge})).is_err());
}

#[test]
fn paste_special_respects_track_changes_and_brings_lists() {
    let mut s = s();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "edit.pasteSpecial", json!({"as": "html", "html": "<ul><li>one</li><li>two</li></ul>"}));
    assert!(text(&s).starts_with("one\ntwo"), "{}", text(&s));
    let ch = run(&mut s, "review.changes", json!({}));
    assert!(!ch.as_array().unwrap().is_empty(), "the paste is a tracked insertion");
    let first = s.doc.body.first().and_then(|b| b.as_para()).unwrap();
    let n = first.props.numbering.expect("list kept");
    assert!(s.doc.numbering.nums.iter().any(|x| x.id == n.num), "the list definition came along");
    run(&mut s, "review.acceptAll", json!({}));
    assert!(text(&s).starts_with("one\ntwo"));
}

/// Character formatting at a body offset of the first paragraph.
fn props_at(s: &Session, off: usize) -> wordcraft_doc::CharProps {
    let pos = Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off };
    s.doc.para_at(&pos).map(|p| p.props_of_char(off).clone()).unwrap_or_default()
}

/// Paste of another program's formatted copy keeps its formatting (#687); Merge Formatting keeps
/// only its emphasis, in the destination's font; Keep Text Only drops it all.
#[test]
fn paste_keeps_source_formatting_from_other_programs() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "x"}));
    run(&mut s, "caret.docEnd", json!({}));
    let html = "<p><span style=\"font-family:'Courier New';color:#ff0000\">red <b>bold</b></span></p>";
    let clip = json!({"text": "red bold", "html": html});
    run(&mut s, "edit.paste", clip.clone());
    assert_eq!(text(&s), "xred bold");
    assert_eq!(props_at(&s, 1).font.as_deref(), Some("Courier New"));
    assert!(props_at(&s, 1).color.is_some());
    assert!(!bold_at(&s, 0, 1) && bold_at(&s, 0, 5));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "x");
    run(&mut s, "edit.pasteMerge", clip.clone());
    assert_eq!(text(&s), "xred bold");
    assert_eq!(props_at(&s, 1).font, None, "Merge Formatting takes the destination's font");
    assert_eq!(props_at(&s, 1).color, None);
    assert!(bold_at(&s, 0, 5), "and keeps the emphasis");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.pasteText", clip);
    assert_eq!(text(&s), "xred bold");
    assert!(!bold_at(&s, 0, 5));

    // Windows' "HTML Format" carries a header with the fragment's byte offsets.
    run(&mut s, "document.setText", json!({"text": ""}));
    let body = "<html><body><!--StartFragment--><b>win</b><!--EndFragment--></body></html>";
    let head = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:SSSSSSSSSS\r\nEndFragment:EEEEEEEEEE\r\n";
    let start = head.len() + body.find("<b>").unwrap();
    let end = head.len() + body.find("<!--EndFragment").unwrap();
    let cf = format!("{head}{body}").replace("SSSSSSSSSS", &format!("{start:010}")).replace("EEEEEEEEEE", &format!("{end:010}"));
    run(&mut s, "edit.paste", json!({"text": "win", "html": cf}));
    assert_eq!(text(&s), "win");
    assert!(bold_at(&s, 0, 0));
    assert_eq!(crate::cmd::paste::html_fragment("Version:1\r\nStartFragment:99999\r\nEndFragment:5\r\n<i>x</i>"), "<i>x</i>");

    // RTF when there is no HTML.
    run(&mut s, "document.setText", json!({"text": ""}));
    run(&mut s, "edit.paste", json!({"text": "bold then", "rtf": r"{\rtf1\ansi {\b bold}\b0  then}"}));
    assert!(text(&s).starts_with("bold"), "{}", text(&s));
    assert!(bold_at(&s, 0, 0));

    // HTML with nothing to paste (a web picture by address) or too large: the text is pasted.
    run(&mut s, "document.setText", json!({"text": ""}));
    run(&mut s, "edit.paste", json!({"text": "alt", "html": "<img src='https://example.com/a.png'>"}));
    assert_eq!(text(&s), "alt");
    let huge = format!("<b>{}</b>", "a".repeat(crate::cmd::paste::MAX_PASTE_BYTES));
    run(&mut s, "edit.paste", json!({"text": "!", "html": huge}));
    assert_eq!(text(&s), "alt!");
    assert!(s.run("edit.paste", &json!({"html": "<p> </p>"})).is_err(), "nothing to paste is an error");
    assert!(s.run("edit.pasteMerge", &json!({"rtf": "not rtf"})).is_err());
    assert!(s.run("edit.pasteText", &json!({})).is_err());

    // Our own copy still wins while the system clipboard holds its text.
    run(&mut s, "document.setText", json!({"text": "Own"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.italic", json!({}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({"text": "Own"}));
    assert_eq!(text(&s), "OwnOwn");
    assert_eq!(props_at(&s, 4).italic, Some(true));
}

/// A CF_HTML ("HTML Format") clipboard payload around `html`, with the byte offsets Windows writes.
fn cf_html(html: &str) -> String {
    let n = "Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n".len();
    let a = html.find("<body>").map_or(0, |i| i + 6);
    let b = html.find("</body>").unwrap_or(html.len());
    format!(
        "Version:1.0\r\nStartHTML:{n:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n{html}",
        n + html.len(),
        n + a,
        n + b
    )
}

/// LibreOffice copies a paragraph indented on both sides as `<blockquote>`, its margins in the
/// page's stylesheet (#1270). Pasting it gives a Normal paragraph with those indents — not the
/// Quote style (centred, italic) — by Paste and by Paste Special > HTML alike.
#[test]
fn pasted_blockquote_is_an_indented_paragraph_not_a_quote() {
    let page = "<!DOCTYPE html><html><head><meta name=\"generator\" content=\"LibreOffice\"/><style type=\"text/css\">\
        blockquote { margin-left: 1cm; margin-right: 2cm; background: transparent }\
        p { line-height: 115%; margin-bottom: 0.25cm }</style></head>\
        <body><p>First plain.</p><blockquote lang=\"en-US\"><b>Bold indented.</b></blockquote><p>Last plain.</p></body></html>";
    let check = |s: &Session, left: f32, right: f32| {
        assert!(text(s).starts_with("First plain.\nBold indented.\nLast plain."), "{}", text(s));
        let q = s.doc.body.get(1).and_then(|b| b.as_para()).unwrap();
        assert_ne!(q.props.style.as_deref(), Some("Quote"), "not the Quote style");
        let rp = s.doc.styles.resolve_para(&q.props);
        assert_ne!(rp.align, wordcraft_doc::props::Align::Center, "not centred");
        assert!((rp.indent_left - left).abs() < 0.1, "left {}", rp.indent_left);
        assert!((rp.indent_right - right).abs() < 0.1, "right {}", rp.indent_right);
        let rc = s.doc.styles.resolve_char(q.props.style.as_deref(), q.props_of_char(0));
        assert!(!rc.italic, "not italic");
        assert!(rc.bold, "keeps its bold");
    };
    let mut s = s();
    run(&mut s, "edit.paste", json!({"text": "First plain.\nBold indented.\nLast plain.", "html": cf_html(page)}));
    check(&s, 72.0 / 2.54, 144.0 / 2.54);
    let mut s2 = self::s();
    run(&mut s2, "edit.pasteSpecial", json!({"as": "html", "html": cf_html(page)}));
    check(&s2, 72.0 / 2.54, 144.0 / 2.54);
    // No margins given: half an inch each side.
    let mut s3 = self::s();
    run(&mut s3, "edit.paste", json!({"text": "a\nq", "html": "<p>a</p><blockquote>q</blockquote>"}));
    let q = s3.doc.body.get(1).and_then(|b| b.as_para()).unwrap();
    assert_ne!(q.props.style.as_deref(), Some("Quote"));
    assert_eq!((q.props.indent_left, q.props.indent_right), (Some(36.0), Some(36.0)));
}

/// Opening an .html file keeps `<blockquote>` as the Quote style, so HTML round-trips.
#[test]
fn opened_html_blockquote_stays_a_quote() {
    let d = crate::io::open_bytes("q.html", b"<p>a</p><blockquote>q</blockquote>").unwrap();
    let q = d.body.get(1).and_then(|b| b.as_para()).unwrap();
    assert_eq!(q.props.style.as_deref(), Some("Quote"));
}

/// Paste Special lists a picture on the system clipboard and pastes it (#687).
#[test]
fn paste_special_lists_and_pastes_a_picture() {
    let mut s = s();
    let png = crate::cmd::insert::base64_encode(&tiny_png());
    let r = run(&mut s, "edit.pasteSpecial", json!({"picture": png, "html": "<img src='x.png'>"}));
    let ids: Vec<&str> = r["formats"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["html", "picture"]);
    run(&mut s, "edit.pasteSpecial", json!({"as": "picture", "picture": png}));
    assert_eq!(s.doc.media.len(), 1);
    assert!(s.run("edit.pasteSpecial", &json!({"as": "picture", "picture": "%%%"})).is_err());
    assert!(s.run("edit.pasteSpecial", &json!({"as": "picture"})).is_err());
}

#[test]
fn clipboard_pane_collects_copies_and_pastes_one_undoably() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "one two three"}));
    for (a, b) in [(0, 3), (4, 7), (8, 13)] {
        run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": a}, "focus": {"block": 0, "off": b}}));
        run(&mut s, "edit.copy", json!({}));
    }
    // Copying the same text again adds nothing.
    run(&mut s, "edit.copy", json!({}));
    let items = run(&mut s, "edit.clipboardItems", json!({}));
    let previews: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["preview"].as_str().unwrap()).collect();
    assert_eq!(previews, ["three", "two", "one"], "newest first");
    assert_eq!(run(&mut s, "edit.clipboardPane", json!({}))["value"], true);
    assert!(s.view.clipboard_pane);

    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.pasteClipboardItem", json!({"index": 1}));
    assert_eq!(text(&s), "one two threetwo");
    assert_eq!(s.clipboard_text, "three", "the clipboard itself is unchanged");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "one two three");

    run(&mut s, "edit.pasteAllClipboard", json!({}));
    assert_eq!(text(&s), "one two threeonetwothree", "Paste All goes in copy order");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "one two three", "Paste All is one undo step");

    let left = run(&mut s, "edit.deleteClipboardItem", json!({"index": 0}));
    assert_eq!(left[0]["preview"], "two");
    run(&mut s, "edit.clearClipboard", json!({}));
    assert!(s.clip_history.is_empty());
    assert!(s.run("edit.pasteClipboardItem", &json!({"index": 0})).is_err());
}

fn spiked(s: &Session) -> Option<String> {
    s.building_blocks.get("Spike").map(|f| f.plain_text())
}

#[test]
fn spike_collects_cuts_and_insert_spike_pastes_them_and_empties_it() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Alpha beta gamma.\n"));
    run(&mut s, "select.text", json!({"text": "beta"}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "select.text", json!({"text": "Alpha"}));
    run(&mut s, "edit.spike", json!({}));
    run(&mut s, "select.text", json!({"text": "gamma"}));
    run(&mut s, "edit.spike", json!({}));
    assert_eq!(text(&s), " beta .\n", "Ctrl+F3 cuts");
    assert_eq!(spiked(&s).as_deref(), Some("Alpha\ngamma\n"), "each piece is followed by a paragraph mark");
    assert_eq!((s.clipboard_text.as_str(), s.clip_history.len()), ("beta", 1), "the clipboard is left alone");
    // Ctrl+Shift+F3 on the empty last paragraph: everything, in the order it was collected.
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.insertSpike", json!({}));
    assert_eq!(text(&s), " beta .\nAlpha\ngamma\n");
    assert_eq!(spiked(&s), None, "Ctrl+Shift+F3 empties the Spike");
    assert!(matches!(s.run("edit.insertSpike", &json!({})), Err(crate::CmdError::Disabled(_))));
}

#[test]
fn spike_keeps_character_formatting() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Bold plain\n"));
    run(&mut s, "select.text", json!({"text": "Bold"}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "edit.spike", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.insertSpike", json!({}));
    assert_eq!(text(&s), " plain\nBold\n");
    let p = s.doc.para_at(&Pos::body(1, 0)).unwrap();
    assert_eq!(p.props_of_char(0).bold, Some(true));
}

#[test]
fn spike_and_insert_spike_are_one_undo_step_each() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("one two\n"));
    run(&mut s, "select.text", json!({"text": "one"}));
    run(&mut s, "edit.spike", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "one two\n");
    assert_eq!(spiked(&s).as_deref(), Some("one\n"), "like Cut and the clipboard, undo leaves the Spike as it is");
    run(&mut s, "caret.docEnd", json!({}));
    let before = text(&s);
    run(&mut s, "edit.insertSpike", json!({}));
    assert_eq!(text(&s), "one two\none\n");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), before);
}

#[test]
fn spike_needs_a_selection() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("one two"));
    let r = s.run("edit.spike", &json!({}));
    assert!(matches!(r, Err(crate::CmdError::Disabled(_))), "{r:?}");
    assert_eq!(spiked(&s), None);
    assert_eq!(text(&s), "one two");
}

#[test]
fn autotext_inserts_the_spike_without_emptying_it() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Alpha beta\n"));
    run(&mut s, "select.text", json!({"text": "Alpha"}));
    run(&mut s, "edit.spike", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.autoText", json!({"insert": "Spike"}));
    run(&mut s, "insert.autoText", json!({"insert": "Spike"}));
    assert_eq!(text(&s), " beta\nAlpha\nAlpha\n");
    assert_eq!(spiked(&s).as_deref(), Some("Alpha\n"));
}

#[test]
fn spike_outside_the_editable_regions_changes_nothing() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Locked intro.\nFill in here.\n"));
    run(&mut s, "select.text", json!({"text": "Fill in here."}));
    run(&mut s, "review.restrict", json!({"exception": "add"}));
    run(&mut s, "review.restrict", json!({"mode": "readOnly"}));
    // Collected inside the exception: allowed.
    run(&mut s, "select.text", json!({"text": "here"}));
    run(&mut s, "edit.spike", json!({}));
    assert_eq!(spiked(&s).as_deref(), Some("here\n"));
    // Outside it: refused, and neither the document nor the Spike changes.
    let before = text(&s);
    run(&mut s, "select.text", json!({"text": "intro"}));
    assert!(s.run("edit.spike", &json!({})).is_err());
    assert_eq!(text(&s), before);
    assert_eq!(spiked(&s).as_deref(), Some("here\n"));
    // Inserting outside it: refused, and the Spike keeps what it holds.
    assert!(s.run("edit.insertSpike", &json!({})).is_err());
    assert_eq!(text(&s), before);
    assert_eq!(spiked(&s).as_deref(), Some("here\n"));
}

#[test]
fn clipboard_pane_is_capped_and_rejects_bad_indexes() {
    let mut s = s();
    let words: Vec<String> = (0..30).map(|i| format!("w{i}")).collect();
    run(&mut s, "document.setText", json!({"text": words.join(" ")}));
    // Cutting each word in turn leaves the spaces: word `off` starts at `off`.
    for (off, w) in words.iter().enumerate() {
        run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": off}, "focus": {"block": 0, "off": off + w.len()}}));
        run(&mut s, "edit.cut", json!({}));
    }
    let items = run(&mut s, "edit.clipboardItems", json!({}));
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), crate::cmd::edit::CLIP_MAX_ITEMS);
    assert_eq!(items[0]["preview"], "w29");
    assert_eq!(items[23]["preview"], "w6", "the oldest drop out");

    let before = text(&s);
    for bad in [json!({"index": 24}), json!({"index": -1}), json!({"index": 1e300}), json!({"index": "x"}), json!({})] {
        assert!(s.run("edit.pasteClipboardItem", &bad).is_err(), "{bad}");
        assert!(s.run("edit.deleteClipboardItem", &bad).is_err(), "{bad}");
    }
    assert_eq!(text(&s), before);
    assert_eq!(s.clip_history.len(), 24);

    // A huge copy still reaches the clipboard but is not collected.
    run(&mut s, "document.setText", json!({"text": "x".repeat(3 << 20)}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard_text.len(), 3 << 20);
    assert_eq!(s.clip_history.items()[0].text, "w29");
}

#[test]
fn tables_commands() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 3}));
    assert!(s.sel.focus.path.cell().is_some());
    run(&mut s, "text.insert", json!({"text": "A1"}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.insert", json!({"text": "B1"}));
    run(&mut s, "table.insertRowBelow", json!({}));
    run(&mut s, "table.insertColumnRight", json!({}));
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 3);
    assert_eq!(t.cols(), 4);
    run(&mut s, "table.deleteTable", json!({}));
    assert!(s.doc.body.iter().all(|b| b.as_table().is_none()));
    assert!(s.run("table.merge", &json!({})).is_err());
}

/// A 2×2 table "A1 B1 / A2 B2" after an "Intro" paragraph, the caret in its first cell.
fn table_doc() -> Session {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Intro"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    for (i, t) in ["A1", "B1", "A2", "B2"].iter().enumerate() {
        if i > 0 {
            run(&mut s, "text.tab", json!({}));
        }
        run(&mut s, "text.insert", json!({"text": t}));
    }
    s
}

fn body_table_cells(s: &Session) -> Vec<Vec<String>> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_table())
        .map(|t| {
            t.rows
                .iter()
                .flat_map(|r| r.cells.iter())
                .map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<String>())
                .collect()
        })
        .collect()
}

/// #1744: Select Table → Copy → Paste outside the table pastes a copy of the whole table.
#[test]
fn copy_whole_table_pastes_a_table() {
    let mut s = table_doc();
    run(&mut s, "table.selectTable", json!({}));
    let r = run(&mut s, "edit.copy", json!({}));
    assert!(r["text"].as_str().unwrap().contains("A1\tB1"), "clipboard text is tab separated: {r}");
    assert!(matches!(s.clipboard.as_ref().unwrap().blocks.as_slice(), [wordcraft_doc::Block::Table(_)]), "the copy is the table");
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "edit.paste", json!({}));
    let want = vec!["A1".to_string(), "B1".into(), "A2".into(), "B2".into()];
    assert_eq!(body_table_cells(&s), vec![want.clone(), want]);
}

/// #1744: Select Table → Cut removes the whole table, and one Undo brings it back.
#[test]
fn cut_whole_table_removes_it_and_undoes() {
    let mut s = table_doc();
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "edit.cut", json!({}));
    assert!(body_table_cells(&s).is_empty(), "the table is gone");
    assert!(s.sel.focus.path.cell().is_none(), "the caret left the table");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_table_cells(&s), vec![vec!["A1".to_string(), "B1".into(), "A2".into(), "B2".into()]]);
    // Paste puts it back as a table.
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "edit.cut", json!({}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(body_table_cells(&s).len(), 1);
}

/// A 3×3 table "A1 B1 C1 / A2 B2 C2 / A3 B3 C3" after an "Intro" paragraph (#1744).
fn table3_doc() -> Session {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Intro"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 3}));
    for (i, t) in ["A1", "B1", "C1", "A2", "B2", "C2", "A3", "B3", "C3"].iter().enumerate() {
        if i > 0 {
            run(&mut s, "text.tab", json!({}));
        }
        run(&mut s, "text.insert", json!({"text": t}));
    }
    s
}

/// Select whole cells of the body's table: rows and grid columns, both inclusive.
fn select_table_cells(s: &mut Session, rows: (usize, usize), cols: (usize, usize)) {
    let tp = wordcraft_doc::Path(vec![1]);
    crate::cmd::table::select_range(s, StoryRef::Body, &tp, rows, cols).unwrap();
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some((rows, cols)));
}

fn row_cells(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|t| t.to_string()).collect()
}

/// #1744 (Word check A): copying whole rows, two cells of a row, or a block of cells over
/// several rows, then pasting outside the table, pastes a table of just those cells.
#[test]
fn copy_rows_and_cells_pastes_a_table_of_them() {
    let full = row_cells(&["A1", "B1", "C1", "A2", "B2", "C2", "A3", "B3", "C3"]);
    for (rows, cols, want) in [
        ((1, 1), (0, 2), row_cells(&["A2", "B2", "C2"])),
        ((0, 0), (0, 1), row_cells(&["A1", "B1"])),
        ((1, 2), (1, 2), row_cells(&["B2", "C2", "B3", "C3"])),
    ] {
        let mut s = table3_doc();
        select_table_cells(&mut s, rows, cols);
        run(&mut s, "edit.copy", json!({}));
        let Some([wordcraft_doc::Block::Table(t)]) = s.clipboard.as_ref().map(|f| f.blocks.as_slice()) else {
            panic!("{rows:?} {cols:?}: the copy is a table, not {:?}", s.clipboard_text);
        };
        assert_eq!(t.rows.len(), rows.1 - rows.0 + 1, "{rows:?} {cols:?}");
        assert_eq!(t.grid.len(), cols.1 - cols.0 + 1, "{rows:?} {cols:?}: the grid has the copied columns");
        run(&mut s, "caret.docStart", json!({}));
        run(&mut s, "edit.paste", json!({}));
        assert_eq!(body_table_cells(&s), vec![want, full.clone()], "{rows:?} {cols:?}");
    }
}

/// #1744 (Word check A): Cut of whole rows, or of some cells, clears their text and leaves
/// the rows and cells; the clipboard still gets a table of them. One Undo brings the text back.
#[test]
fn cut_rows_and_cells_clears_their_text() {
    let full = row_cells(&["A1", "B1", "C1", "A2", "B2", "C2", "A3", "B3", "C3"]);
    for (rows, cols, left, copied) in [
        ((1, 1), (0, 2), row_cells(&["A1", "B1", "C1", "", "", "", "A3", "B3", "C3"]), row_cells(&["A2", "B2", "C2"])),
        ((0, 0), (0, 1), row_cells(&["", "", "C1", "A2", "B2", "C2", "A3", "B3", "C3"]), row_cells(&["A1", "B1"])),
        ((1, 2), (1, 2), row_cells(&["A1", "B1", "C1", "A2", "", "", "A3", "", ""]), row_cells(&["B2", "C2", "B3", "C3"])),
    ] {
        let mut s = table3_doc();
        select_table_cells(&mut s, rows, cols);
        run(&mut s, "edit.cut", json!({}));
        assert_eq!(body_table_cells(&s), vec![left.clone()], "{rows:?} {cols:?}: rows and cells stay");
        assert!(s.sel.is_collapsed(), "{rows:?} {cols:?}");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(body_table_cells(&s), vec![full.clone()], "{rows:?} {cols:?}: one undo step");
        select_table_cells(&mut s, rows, cols);
        run(&mut s, "edit.cut", json!({}));
        run(&mut s, "caret.docStart", json!({}));
        run(&mut s, "edit.paste", json!({}));
        assert_eq!(body_table_cells(&s), vec![copied, left], "{rows:?} {cols:?}: the cut pastes as a table");
    }
}

/// #1744 review: Cut clears every cell Copy takes: a cell merged across the selected columns
/// widens them, so the cell below the merge's second column is copied and cleared too.
#[test]
fn cut_clears_the_cells_a_merged_cell_widens_the_copy_to() {
    let mut s = table3_doc();
    select_table_cells(&mut s, (0, 0), (0, 1));
    run(&mut s, "table.merge", json!({}));
    select_table_cells(&mut s, (0, 1), (0, 0));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "edit.cut", json!({}));
    let cells = body_table_cells(&s).remove(0);
    assert_eq!(cells.len(), 8, "one merged cell and seven others: {cells:?}");
    assert_eq!(cells[1], "C1");
    assert!(cells[2..4].iter().all(String::is_empty), "A2 and B2 are cleared: {cells:?}");
    assert_eq!(cells[4..], ["C2", "A3", "B3", "C3"], "the rest stays");
}

/// #1744 review: an empty cell's selection enables Cut but not the Spike, which takes text only.
#[test]
fn spike_stays_disabled_on_an_empty_cell_selection() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Intro"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    select_table_cells(&mut s, (0, 0), (0, 0));
    let r = s.run("edit.spike", &json!({}));
    assert!(matches!(r, Err(crate::CmdError::Disabled(_))), "{r:?}");
}

fn clip_is_table(s: &Session) -> bool {
    matches!(s.clipboard.as_ref().map(|f| f.blocks.as_slice()), Some([wordcraft_doc::Block::Table(_)]))
}

/// The body table's cells as they read with every change accepted.
fn body_table_final(s: &Session) -> Vec<String> {
    let t = s.doc.body.iter().find_map(|b| b.as_table()).expect("a table");
    t.rows.iter().flat_map(|r| r.cells.iter()).map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.final_text()).collect()).collect()
}

/// #1744 review: an empty table or cell selected whole still copies, and Cut of the empty
/// table removes it, as in Word.
#[test]
fn copy_and_cut_of_an_empty_table_or_cell() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Intro"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "edit.copy", json!({}));
    assert!(clip_is_table(&s), "an empty 1×1 table copies: {:?}", s.clipboard);
    run(&mut s, "edit.cut", json!({}));
    assert!(body_table_cells(&s).is_empty(), "Cut removes the empty table");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_table_cells(&s), vec![vec![String::new()]]);

    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": "Intro"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    select_table_cells(&mut s, (0, 0), (0, 0));
    run(&mut s, "edit.copy", json!({}));
    assert!(clip_is_table(&s), "an empty cell copies as a one-cell table");
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(body_table_cells(&s).len(), 1, "Cut of an empty cell leaves the table");
}

/// #1744 review: a text selection over exactly a whole table (no whole-cell record) copies the
/// table; so does one over exactly a table nested in a cell. Other users of the copied range
/// read it as a table too: the selected text is tab separated, a Quick Part saved from it
/// inserts a table.
#[test]
fn text_selection_of_a_whole_table_copies_the_table() {
    let mut s = table_doc();
    let at = |path: Vec<u32>, off: usize| json!({"story": "body", "path": path, "off": off});
    run(&mut s, "select.range", json!({"anchor": at(vec![1, 0, 0, 0], 0), "focus": at(vec![1, 1, 1, 0], 2)}));
    assert!(s.cell_range().is_none(), "a text selection");
    assert_eq!(s.selected_text(), "A1\tB1\nA2\tB2");
    run(&mut s, "edit.copy", json!({}));
    assert!(clip_is_table(&s));
    run(&mut s, "insert.quickParts", json!({"save": "grid"}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "insert.quickParts", json!({"insert": "grid"}));
    assert_eq!(body_table_cells(&s).len(), 2, "the Quick Part is a table");

    // A table nested in cell A1: selecting its text whole copies just it.
    let mut s = table_doc();
    run(&mut s, "select.range", json!({"anchor": at(vec![1, 0, 0, 0], 2), "focus": at(vec![1, 0, 0, 0], 2)}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 1}));
    run(&mut s, "text.insert", json!({"text": "N1"}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.insert", json!({"text": "N2"}));
    let (inner, _, _) = s.sel.focus.path.cell().unwrap();
    assert_eq!(inner.0.len(), 4, "nested in a cell");
    let first = [inner.0.clone(), vec![0, 0, 0]].concat();
    let last = [inner.0.clone(), vec![1, 0, 0]].concat();
    run(&mut s, "select.range", json!({"anchor": at(first, 0), "focus": at(last, 2)}));
    assert!(s.cell_range().is_none(), "a text selection");
    run(&mut s, "edit.copy", json!({}));
    let Some([wordcraft_doc::Block::Table(t)]) = s.clipboard.as_ref().map(|f| f.blocks.as_slice()) else { panic!("{:?}", s.clipboard_text) };
    assert_eq!(t.rows.len(), 2, "the nested table, not the outer one");
}

/// #1744 review: with Track Changes on, Cut of the whole table or of a row marks the text
/// deleted and keeps the table; one Undo takes the marks back.
#[test]
fn tracked_cut_of_a_table_or_row_marks_text_deleted() {
    let full = row_cells(&["A1", "B1", "C1", "A2", "B2", "C2", "A3", "B3", "C3"]);
    let mut s = table3_doc();
    s.doc.settings.track_changes = true;
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "edit.cut", json!({}));
    assert!(clip_is_table(&s));
    assert_eq!(body_table_cells(&s), vec![full.clone()], "the table and its text stay, marked");
    assert!(body_table_final(&s).iter().all(String::is_empty), "{:?}", body_table_final(&s));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_table_final(&s), full);

    select_table_cells(&mut s, (1, 1), (0, 2));
    run(&mut s, "edit.cut", json!({}));
    assert!(clip_is_table(&s));
    assert_eq!(body_table_cells(&s), vec![full.clone()]);
    assert_eq!(body_table_final(&s), row_cells(&["A1", "B1", "C1", "", "", "", "A3", "B3", "C3"]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_table_final(&s), full);
}

/// #1744 review: Cut of a table, or of cells, holding a content control locked against deletion
/// is refused and changes nothing in the document; the copy is made first, so the clipboard
/// holds the table, as for any refused Cut.
#[test]
fn cut_of_cells_with_a_locked_control_is_refused() {
    for whole in [true, false] {
        let mut s = table3_doc();
        run(&mut s, "edit.find", json!({"text": "B2"}));
        run(&mut s, "developer.richText", json!({"tag": "keep"}));
        run(&mut s, "developer.properties", json!({"control": "keep", "lockDelete": true}));
        if whole {
            run(&mut s, "table.selectTable", json!({}));
        } else {
            select_table_cells(&mut s, (1, 1), (0, 2));
        }
        let before = s.doc.clone();
        assert!(s.run("edit.cut", &json!({})).is_err(), "whole table: {whole}");
        assert_eq!(s.doc, before, "whole table: {whole}");
        assert!(s.status.contains("can't be deleted"), "{}", s.status);
        assert!(clip_is_table(&s), "whole table: {whole}");
    }
}

#[test]
fn inserting_a_column_keeps_cell_widths_in_step_with_the_grid() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let widths = |s: &Session| {
        let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        (t.grid.clone(), t.rows[0].cells.iter().map(|c| c.props.width.unwrap_or(0.0)).collect::<Vec<_>>())
    };
    let (grid_before, _) = widths(&s);
    assert!(grid_before.iter().all(|w| *w > 0.0));
    run(&mut s, "table.insertColumnRight", json!({}));
    let (grid_after, cells) = widths(&s);
    // The table keeps its total width, and each cell carries its grid columns' width: the
    // layout and the saved `w:tcW` see the same shrink the grid got.
    let sum = |v: &[f32]| v.iter().sum::<f32>();
    assert!((sum(&grid_after) - sum(&grid_before)).abs() < 0.5, "{grid_after:?} vs {grid_before:?}");
    assert_eq!(cells.len(), grid_after.len());
    for (g, w) in grid_after.iter().zip(cells.iter()) {
        assert!((w - g).abs() < 0.01, "cell width {w} vs grid {g}");
    }
}

/// A row that skips grid columns (`w:gridBefore`) gives its cells the widths of the columns
/// they sit in, not of the first ones.
#[test]
fn cell_widths_follow_the_grid_after_skipped_columns() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 3}));
    let i = s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap();
    let wordcraft_doc::Block::Table(t) = std::sync::Arc::make_mut(&mut s.doc.body[i]) else { panic!("no table") };
    t.grid = vec![100.0, 200.0, 300.0];
    t.rows[1].props.grid_before = 1;
    t.rows[1].cells.pop();
    crate::cmd::table::sync_cell_widths(t);
    let widths: Vec<f32> = t.rows[1].cells.iter().map(|c| c.props.width.unwrap_or(0.0)).collect();
    assert_eq!(widths, [200.0, 300.0]);
}

#[test]
fn a_rejected_version_restore_saves_nothing() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Original text."));
    s.run("review.restrict", &json!({"mode": "readOnly", "password": "pw"})).unwrap();
    s.run("file.versions", &json!({"save": "before"})).unwrap();
    // One command asking for both a save and a restore the protection refuses: the save must
    // not happen either.
    let e = s.run("file.versions", &json!({"save": "mid", "restore": 0})).unwrap_err().to_string();
    assert!(e.contains("stop protection"), "{e}");
    assert_eq!(s.versions.len(), 1, "the refused command saved a version anyway");
    // A bad index is refused before anything is saved, too.
    let e = s.run("file.versions", &json!({"save": "mid", "restore": 99})).unwrap_err().to_string();
    assert!(e.contains("no such version"), "{e}");
    assert_eq!(s.versions.len(), 1, "the refused command saved a version anyway");
    // A restore that is there but not a non-negative integer is refused, not read as absent:
    // {"save": "mid", "restore": -1} must not save quietly without restoring.
    let e = s.run("file.versions", &json!({"save": "mid", "restore": -1})).unwrap_err().to_string();
    assert!(e.contains("restore (non-negative integer)"), "{e}");
    assert_eq!(s.versions.len(), 1, "the refused command saved a version anyway");
    // An index too large for `usize` names no version: on a 32-bit target `as usize` would wrap
    // it to 0 and restore the first version instead.
    let e = s.run("file.versions", &json!({"save": "mid", "restore": 4294967296u64})).unwrap_err().to_string();
    assert!(e.contains("no such version"), "{e}");
    assert_eq!(s.versions.len(), 1, "the refused command saved a version anyway");
}

/// #3931: the page as a command would leave it, laid out without running it on the document:
/// Insert › Table's grid previews the table it would insert. The document, the caret, the undo
/// history and Repeat stay as they were.
#[test]
fn preview_layout_shows_a_command_without_running_it() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Lanterns"));
    run(&mut s, "caret.docEnd", json!({}));
    let (doc, sel, rev) = (s.doc.clone(), s.sel.clone(), s.rev());
    let cells = |l: &wordcraft_layout::DocLayout| l.index.keys().filter(|(st, p)| *st == StoryRef::Body && p.0.len() == 4).count();
    let (_, _, preview) = s.preview_layout("insert.table", &json!({"rows": 4, "cols": 3})).expect("a table can be inserted here");
    assert_eq!(cells(&preview), 12, "four rows of three cells");
    // At the caret: after "Lanterns", the second block.
    let tables: std::collections::BTreeSet<u32> =
        preview.index.keys().filter(|(st, p)| *st == StoryRef::Body && p.0.len() == 4).map(|(_, p)| p.0[0]).collect();
    assert_eq!(tables, [1].into(), "the table goes where the caret is");
    assert_eq!(s.doc, doc);
    assert_eq!((&s.sel, s.rev(), s.dirty), (&sel, rev, false));
    assert!(!s.can_undo() && s.last_command.is_none());
    assert_eq!(cells(&s.layout()), 0, "the page itself has no table");
    assert!(s.preview_layout("no.such", &json!({})).is_none());
}

/// #3931 review: the preview comes back with the document it was laid out from. The table moves
/// the paragraphs after the caret down a block, so this document would draw the layout's
/// paragraphs from the wrong blocks, or from none.
#[test]
fn preview_layout_comes_with_the_document_it_lays_out() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites"));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.end", json!({}));
    let (doc, _, preview) = s.preview_layout("insert.table", &json!({"rows": 2, "cols": 2})).expect("a table can be inserted here");
    for (story, path) in preview.index.keys().filter(|(st, _)| *st == StoryRef::Body) {
        assert!(doc.para(*story, path).is_some(), "{path:?} names a paragraph of the previewed document");
    }
    let kites = preview.index.keys().find(|(st, p)| *st == StoryRef::Body && doc.para(*st, p).is_some_and(|p| p.text == "Kites"));
    assert!(kites.is_some_and(|(_, p)| p.0.len() == 1 && p.0[0] > 1), "Kites is laid out after the table: {kites:?}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Lanterns\nKites", "the document itself has no table");
}

/// #3931 review: the word being typed has no spelling squiggle on the preview either, though
/// the table moves the copy's caret off it and splits its paragraph.
#[test]
fn preview_layout_keeps_the_word_being_typed_unmarked() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("helo"));
    s.view.proofing = true;
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": " wrld"}));
    let marks = |l: &wordcraft_layout::DocLayout, doc: &wordcraft_doc::Document| -> Vec<String> {
        let text = &doc.para(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap().text;
        l.pages[0]
            .items
            .iter()
            .filter_map(|i| match i {
                wordcraft_layout::Placed::Lines { para, path, .. } if path.0 == [0] => {
                    Some(para.issues.iter().filter(|i| !i.2).map(|i| text[i.0..i.1].to_string()).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect()
    };
    let doc = s.doc.clone();
    assert_eq!(marks(&s.layout(), &doc), ["helo"], "the page itself: only the finished word is marked");
    let (copy, _, preview) = s.preview_layout("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    assert_eq!(marks(&preview, &copy), ["helo"]);
}

#[test]
fn delete_row_on_a_merge_start_promotes_the_cell_below() {
    use wordcraft_doc::props::VMerge;
    let vm = |s: &Session, r: usize| {
        let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        t.rows[r].cells[0].props.vmerge
    };
    // Merge of two rows: the cell below becomes an ordinary cell.
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 2}));
    run(&mut s, "caret.down", json!({}));
    run(&mut s, "caret.up", json!({"extend": true}));
    run(&mut s, "table.merge", json!({}));
    run(&mut s, "caret.up", json!({}));
    run(&mut s, "table.deleteRow", json!({}));
    assert_eq!(vm(&s, 0), VMerge::None, "first row's cell continues a merge that no longer exists");
    // Merge of three rows: the cell below restarts the merge.
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "insert.table", json!({"rows": 4, "cols": 2}));
    run(&mut s, "caret.down", json!({}));
    run(&mut s, "caret.down", json!({}));
    run(&mut s, "caret.up", json!({"extend": true}));
    run(&mut s, "caret.up", json!({"extend": true}));
    run(&mut s, "table.merge", json!({}));
    run(&mut s, "caret.up", json!({}));
    run(&mut s, "caret.up", json!({}));
    run(&mut s, "table.deleteRow", json!({}));
    assert_eq!(vm(&s, 0), VMerge::Restart);
    assert_eq!(vm(&s, 1), VMerge::Continue);
}

/// Split Table under a vertically merged cell: the merge ends in the first table and starts
/// again in the second.
#[test]
fn split_table_ends_and_restarts_a_merge_it_cuts() {
    use wordcraft_doc::props::VMerge::{self, Continue, None as No, Restart};
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let tables = |s: &Session| -> Vec<Vec<Vec<VMerge>>> {
        let rows = |t: &wordcraft_doc::Table| t.rows.iter().map(|r| r.cells.iter().map(|c| c.props.vmerge).collect()).collect();
        s.doc.body.iter().filter_map(|b| b.as_table()).map(rows).collect()
    };
    let merged = |last: u32| {
        let mut s = table_from("A\tB\nC\tD\nE\tF\nG\tH");
        run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(last, 0)}));
        run(&mut s, "table.merge", json!({}));
        s
    };
    // Rows 0 to 2 merged, split at row 1: the second table's merge covers rows 1 and 2.
    let mut s = merged(2);
    let before = tables(&s);
    run(&mut s, "caret.set", json!({"pos": cell(1, 1)}));
    run(&mut s, "table.splitTable", json!({}));
    assert_eq!(tables(&s), [vec![vec![No, No]], vec![vec![Restart, No], vec![Continue, No], vec![No, No]]]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(tables(&s), before);
    // Rows 0 and 1 merged, split at row 1: neither part keeps a merge.
    let mut s = merged(1);
    run(&mut s, "caret.set", json!({"pos": cell(1, 1)}));
    run(&mut s, "table.splitTable", json!({}));
    assert_eq!(tables(&s), [vec![vec![No, No]], vec![vec![No, No], vec![No, No], vec![No, No]]]);
}

/// Split Table inside a row content control (a repeating section) around several rows: the
/// control closes after the first table and opens again before the second.
#[test]
fn split_table_keeps_a_row_control_on_both_parts() {
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let control = |tag: &str| wordcraft_doc::ContentControl { id: Some(7), tag: tag.into(), ..Default::default() };
    let mut s = table_from("A\tB\nC\tD\nE\tF\nG\tH");
    if let Some(b) = s.doc.body.iter_mut().find(|b| b.as_table().is_some())
        && let wordcraft_doc::Block::Table(t) = std::sync::Arc::make_mut(b)
    {
        // "outer" goes around every row, and "inner" goes around row 1 alone.
        t.rows[0].controls.open = vec![control("outer")];
        t.rows[1].controls.open = vec![control("inner")];
        t.rows[1].controls.close = 1;
        t.rows[3].controls.close = 1;
    }
    run(&mut s, "caret.set", json!({"pos": cell(2, 0)}));
    run(&mut s, "table.splitTable", json!({}));
    let wraps = |t: &wordcraft_doc::Table| -> Vec<(Vec<String>, u32)> {
        t.rows.iter().map(|r| (r.controls.open.iter().map(|c| c.tag.clone()).collect(), r.controls.close)).collect()
    };
    let parts: Vec<&wordcraft_doc::Table> = s.doc.body.iter().filter_map(|b| b.as_table()).collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(wraps(parts[0]), [(vec!["outer".to_string()], 0), (vec!["inner".to_string()], 2)], "the first table closes the control");
    assert_eq!(wraps(parts[1]), [(vec!["outer".to_string()], 0), (vec![], 1)], "the second table opens it again");
    assert_eq!(parts[1].rows[0].controls.open[0].id, None, "the copy doesn't repeat the control's id");
}

fn cells(s: &Session) -> Option<((usize, usize), (usize, usize))> {
    s.cell_range().map(|x| (x.rows, x.cols))
}

#[test]
fn tracked_table_changes_are_kept_as_formatting_changes() {
    // Word records a table property change made while Track Changes and Track Formatting are
    // on (`w:tblPrChange`): Reject All puts the properties back, Accept All keeps the change.
    // The model, OOXML I/O and both answers already existed; only the recording was missing.
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "review.trackingOptions", json!({"trackFormatting": true}));
    run(&mut s, "table.borders", json!({"kind": "all"}));
    let at = |s: &Session| -> (bool, bool) {
        let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        (t.props.borders.unwrap_or_default().top.is_some(), t.props.fmt_change.is_some())
    };
    let (b, ch) = at(&s);
    assert!(b, "the borders were set");
    assert!(ch, "the border change was recorded as a formatting change");
    run(&mut s, "review.rejectAll", json!({}));
    let (b, ch) = at(&s);
    assert!(!b, "Reject All restores the table's formatting before the change");
    assert!(!ch, "the change is answered");
    run(&mut s, "table.borders", json!({"kind": "all"}));
    assert!(at(&s).1, "the second change records again");
    run(&mut s, "review.acceptAll", json!({}));
    let (b, ch) = at(&s);
    assert!(b && !ch, "Accept All keeps it");
}

#[test]
fn select_all_grows_cell_then_row_then_table_then_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Before"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    run(&mut s, "text.insert", json!({"text": "A1"}));
    assert_eq!(s.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((0, 0)));

    run(&mut s, "select.all", json!({}));
    assert_eq!(cells(&s), Some(((0, 0), (0, 0))), "first Ctrl+A selects the cell");
    assert_eq!(s.selected_text(), "A1");

    run(&mut s, "select.all", json!({}));
    assert_eq!(cells(&s), Some(((0, 0), (0, 1))), "second Ctrl+A selects the row");

    run(&mut s, "select.all", json!({}));
    assert_eq!(cells(&s), Some(((0, 1), (0, 1))), "third Ctrl+A selects the table");

    run(&mut s, "select.all", json!({}));
    assert!(s.column.is_none());
    let (a, b) = s.sel.ordered();
    assert_eq!(a, s.doc.start_of(StoryRef::Body));
    assert_eq!(b, s.doc.end_of(StoryRef::Body));
    assert!(s.selected_text().starts_with("Before"));

    run(&mut s, "select.all", json!({}));
    let (a, b) = s.sel.ordered();
    assert_eq!((a, b), (s.doc.start_of(StoryRef::Body), s.doc.end_of(StoryRef::Body)));
}

#[test]
fn select_all_from_a_column_grows_to_the_table() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    run(&mut s, "table.selectColumn", json!({}));
    assert_eq!(cells(&s), Some(((0, 1), (0, 0))));
    run(&mut s, "select.all", json!({}));
    assert_eq!(cells(&s), Some(((0, 1), (0, 1))), "Ctrl+A on a column selects the table");
}

#[test]
fn select_all_keeps_the_whole_story_when_it_ends_in_a_table() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    run(&mut s, "text.insert", json!({"text": "A1"}));
    s.doc.body.pop();
    run(&mut s, "select.all", json!({}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "select.all", json!({}));
    let whole = (s.doc.start_of(StoryRef::Body), s.doc.end_of(StoryRef::Body));
    assert!(s.doc.end_of(StoryRef::Body).path.cell().is_some());
    assert_eq!(s.sel.ordered(), whole);
    run(&mut s, "select.all", json!({}));
    assert!(s.column.is_none());
    assert_eq!(s.sel.ordered(), whole);
}

#[test]
fn select_extend_from_a_table_paragraph_selects_the_document() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    run(&mut s, "text.insert", json!({"text": "Hello world"}));
    run(&mut s, "select.paragraph", json!({}));
    run(&mut s, "select.extend", json!({}));
    assert!(s.column.is_none());
    let (a, b) = s.sel.ordered();
    assert_eq!(a, s.doc.start_of(StoryRef::Body));
    assert_eq!(b, s.doc.end_of(StoryRef::Body));
}

/// #1596: tables nest 24 deep at most, the depth the DOCX reader and writer keep. Insert › Table
/// in the deepest cell says so; a table pasted there brings its text as paragraphs.
#[test]
fn tables_nest_at_most_24_deep() {
    use wordcraft_doc::table::MAX_TABLE_DEPTH;
    let mut s = s();
    for _ in 0..MAX_TABLE_DEPTH {
        run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    }
    assert_eq!(s.sel.focus.path.depth(), MAX_TABLE_DEPTH);
    let before = s.doc.clone();
    let e = s.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap_err();
    assert!(e.to_string().contains("24 deep"), "{e}");
    assert_eq!(s.doc, before);
    run(&mut s, "edit.paste", json!({"text": "b", "html": "<table><tr><td>b</td></tr></table>"}));
    let cell = s.sel.focus.path.clone();
    let blocks: Vec<_> = (0..).map_while(|i| s.doc.block(StoryRef::Body, &cell.with_last(i))).collect();
    assert!(blocks.iter().all(|b| b.as_table().is_none()), "no table below the limit");
    assert!(blocks.iter().any(|b| b.as_para().is_some_and(|p| p.text.contains('b'))), "the pasted text is there");
}

/// AutoFit Contents (#44) measures the text: a column of short words narrows, a column with a
/// long sentence widens (wrapping within the page), and the widths are written to the grid.
#[test]
fn autofit_contents_sizes_columns_to_their_text() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 3}));
    let grid = |s: &Session| s.doc.body.iter().find_map(|b| b.as_table()).unwrap().grid.clone();
    let before = grid(&s);
    run(&mut s, "text.insert", json!({"text": "A"}));
    run(&mut s, "text.tab", json!({}));
    run(
        &mut s,
        "text.insert",
        json!({"text": "This cell holds a long sentence that would wrap many times in a third of the page width, so it should get most of the room."}),
    );
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.insert", json!({"text": "B"}));
    run(&mut s, "table.autofit", json!({"mode": "contents"}));
    let after = grid(&s);
    let tw = cmd::page::sect(&s).text_width();
    assert!(after[0] < before[0] / 2.0, "short column narrows: {before:?} → {after:?}");
    assert!(after[2] < before[2] / 2.0, "short column narrows: {before:?} → {after:?}");
    assert!(after[1] > before[1] * 1.5, "long column widens: {before:?} → {after:?}");
    let total: f32 = after.iter().sum();
    assert!(total <= tw + 0.5 && total > tw - 1.0, "the long text fills the page width: {total} vs {tw}");
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert!(t.props.width_pct.is_none() && !t.props.fixed);
    assert_eq!(t.rows[0].cells[1].props.width, Some(after[1]), "cell widths follow the grid");
    // Short text only: every column gets just what its text needs.
    s.doc = wordcraft_doc::Document::new();
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 2}));
    run(&mut s, "text.insert", json!({"text": "Name"}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.insert", json!({"text": "Quantity"}));
    run(&mut s, "table.autofit", json!({"mode": "contents"}));
    let g = grid(&s);
    assert!(g[0] < g[1] && g[1] < 100.0, "{g:?}");
    // The layout uses the new widths.
    let w = s.layout().pages[0].items.iter().find_map(|it| match it {
        wordcraft_layout::Placed::Cell { rect, row: 0, cell: 0, .. } => Some(rect.w),
        _ => None,
    });
    assert!(w.is_some_and(|w| (w - g[0]).abs() < 0.5), "{w:?} vs {g:?}");
}

/// The Height and Width boxes (Table Layout › Cell Size) run `table.rowHeight` and
/// `table.columnWidth`; each edit is one Undo.
#[test]
fn cell_size_boxes_set_row_height_and_column_width() {
    use wordcraft_doc::props::HeightRule;
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    run(&mut s, "table.autofit", json!({"mode": "window"}));
    let t0 = s.doc.body.iter().find_map(|b| b.as_table()).unwrap().clone();
    run(&mut s, "table.columnWidth", json!({"width": 100.0}));
    run(&mut s, "table.rowHeight", json!({"height": 30.0, "rule": "exact"}));
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.grid[0], 100.0);
    assert_eq!(t.rows[0].cells[0].props.width, Some(100.0));
    assert!(t.props.width_pct.is_none(), "the column isn't scaled back to the window");
    assert_eq!((t.rows[0].props.height, t.rows[0].props.height_rule), (Some(30.0), HeightRule::Exact));
    let rect = s.layout().pages[0].items.iter().find_map(|it| match it {
        wordcraft_layout::Placed::Cell { rect, row: 0, cell: 0, .. } => Some(*rect),
        _ => None,
    });
    assert!(rect.is_some_and(|r| (r.w - 100.0).abs() < 0.5 && (r.h - 30.0).abs() < 0.5), "{rect:?}");
    assert!(s.run("table.rowHeight", &json!({"height": 30.0, "rule": "sideways"})).is_err());
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.body.iter().find_map(|b| b.as_table()).unwrap(), &t0);
}

/// The Height and Width boxes act on every row and column the selection covers (#1242), not only
/// on the caret's: the whole table, a row, a column, and the caret alone.
#[test]
fn cell_size_boxes_apply_to_the_whole_selection() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 3}));
    run(&mut s, "table.autofit", json!({"mode": "window"}));
    let table = |s: &Session| s.doc.body.iter().find_map(|b| b.as_table()).unwrap().clone();
    let heights = |s: &Session| table(s).rows.iter().map(|r| r.props.height).collect::<Vec<_>>();
    let widths = |s: &Session| table(s).grid;
    let before = widths(&s);
    // The caret alone changes its own row and column.
    run(&mut s, "table.rowHeight", json!({"height": 20.0}));
    run(&mut s, "table.columnWidth", json!({"width": 80.0}));
    assert_eq!(heights(&s), vec![Some(20.0), None, None]);
    assert_eq!(widths(&s)[0], 80.0);
    assert_eq!(&widths(&s)[1..], &before[1..]);
    // A selected column: just that column.
    run(&mut s, "table.selectColumn", json!({}));
    run(&mut s, "table.columnWidth", json!({"width": 70.0}));
    assert_eq!(widths(&s)[0], 70.0);
    assert_eq!(&widths(&s)[1..], &before[1..]);
    // The whole table: every row and every column, one Undo per edit.
    let mid = (heights(&s), widths(&s));
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "table.rowHeight", json!({"height": 40.0}));
    assert_eq!(heights(&s), vec![Some(40.0); 3]);
    run(&mut s, "table.columnWidth", json!({"width": 90.0}));
    assert_eq!(widths(&s), vec![90.0; 3]);
    assert!(table(&s).rows.iter().all(|r| r.cells.iter().all(|c| c.props.width == Some(90.0))), "cell widths follow the grid");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((heights(&s), widths(&s)), mid);
}
/// Table Properties (#44): table, row, column and cell settings land as one undo step, and
/// without settings the command reports the current ones (the dialog's source).
#[test]
fn table_properties_apply_in_one_step() {
    use wordcraft_doc::props::{Align, HeightRule, VAlign};
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let t0 = s.doc.body.iter().find_map(|b| b.as_table()).unwrap().clone();
    run(
        &mut s,
        "table.properties",
        json!({"align": "center", "indent": 18.0, "width": 300.0, "rowHeight": 40.0, "rowHeightRule": "exact", "allowBreak": false,
               "headerRow": true, "columnWidth": 120.0, "cellWidth": 110.0, "valign": "bottom"}),
    );
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!((t.props.align, t.props.indent, t.props.width), (Some(Align::Center), Some(18.0), Some(300.0)));
    let row = &t.rows[0];
    assert_eq!((row.props.height, row.props.height_rule, row.props.cant_split, row.props.header), (Some(40.0), HeightRule::Exact, true, true));
    assert_eq!(t.grid[0], 120.0);
    assert_eq!((row.cells[0].props.width, row.cells[0].props.valign), (Some(110.0), Some(VAlign::Bottom)));
    let applied = t.clone();
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.body.iter().find_map(|b| b.as_table()).unwrap(), &t0, "one undo step per apply");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc.body.iter().find_map(|b| b.as_table()).unwrap(), &applied);
    let state = run(&mut s, "table.properties", json!({}));
    assert_eq!(state["align"], "center");
    assert_eq!(state["row"]["heightRule"], "exact");
    assert_eq!(state["columnWidth"], 120.0);
    assert_eq!(state["cell"]["valign"], "bottom");
    // Clearing the row height makes it automatic again; bad values are refused.
    run(&mut s, "table.properties", json!({"rowHeight": null}));
    assert_eq!(s.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows[0].props.height_rule, HeightRule::Auto);
    assert!(s.run("table.properties", &json!({"valign": "middle"})).is_err());
}

/// #4857: Table Properties' row settings apply to every selected row (the whole table, say), the
/// column width to every selected column and the cell settings to every selected cell, as in
/// Word, in one undo step; with nothing selected, to the caret's alone.
#[test]
fn table_properties_apply_to_the_selected_rows_columns_and_cells() {
    use wordcraft_doc::props::VAlign;
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 2}));
    let table = |s: &Session| s.doc.body.iter().find_map(|b| b.as_table()).unwrap().clone();
    let cant_split = |s: &Session| table(s).rows.iter().map(|r| r.props.cant_split).collect::<Vec<_>>();
    run(&mut s, "table.properties", json!({"allowBreak": false}));
    assert_eq!(cant_split(&s), [true, false, false], "the caret's row alone");
    run(&mut s, "edit.undo", json!({}));
    let before = table(&s);
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "table.properties", json!({"allowBreak": false, "rowHeight": 30.0, "columnWidth": 100.0, "cellWidth": 90.0, "valign": "center"}));
    let t = table(&s);
    assert_eq!(cant_split(&s), [true, true, true], "every selected row");
    assert!(t.rows.iter().all(|r| r.props.height == Some(30.0)));
    assert_eq!(t.grid, [100.0, 100.0], "every selected column");
    assert!(t.rows.iter().flat_map(|r| &r.cells).all(|c| c.props.width == Some(90.0) && c.props.valign == Some(VAlign::Center)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(table(&s), before, "one undo step");
    // A selected row: that row alone; a selected column: that column alone.
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "table.selectRow", json!({}));
    run(&mut s, "table.properties", json!({"allowBreak": false}));
    assert_eq!(cant_split(&s), [false, true, false]);
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "table.selectColumn", json!({}));
    run(&mut s, "table.properties", json!({"columnWidth": 150.0}));
    assert_eq!(table(&s).grid.get(1), Some(&150.0));
    assert_ne!(table(&s).grid.first(), Some(&150.0));
}

/// #4857: a tall row across a page break moves to the next page whole once the whole table is
/// selected and "Allow row to break across pages" is turned off.
#[test]
fn a_selected_table_kept_from_breaking_moves_its_row_whole() {
    // Lines above the table until its 12-line second row starts across the page break.
    let found = (0..80u32).find_map(|above| {
        let mut s = s();
        for k in 0..above {
            run(&mut s, "text.insert", json!({"text": format!("Line {k}")}));
            run(&mut s, "text.newParagraph", json!({}));
        }
        run(&mut s, "insert.table", json!({"rows": 3, "cols": 1}));
        run(&mut s, "text.insert", json!({"text": "Kiln"}));
        run(&mut s, "text.tab", json!({}));
        for i in 0..12 {
            run(&mut s, "text.insert", json!({"text": format!("Press note {i}")}));
            run(&mut s, "text.newParagraph", json!({}));
        }
        (row_pages(&mut s, above).len() == 2).then_some((s, above))
    });
    let (mut s, table) = found.expect("the row starts across the page break");
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "table.properties", json!({"allowBreak": false}));
    assert_eq!(row_pages(&mut s, table), [1], "the row moves to the next page whole");
}

/// The pages the second row of the table at body block `table` has lines on.
fn row_pages(s: &mut Session, table: u32) -> Vec<usize> {
    let row = [table, 1];
    let on_page = |p: &wordcraft_layout::Page| {
        p.items.iter().any(|it| matches!(it, wordcraft_layout::Placed::Lines { path, .. } if path.0.get(..2) == Some(&row[..])))
    };
    s.layout().pages.iter().enumerate().filter(|(_, p)| on_page(p)).map(|(i, _)| i).collect()
}

/// Table Layout › Text Direction (#226) cycles the selected cells' text through horizontal,
/// top-to-bottom and bottom-to-top, and the layout turns the text.
#[test]
fn table_text_direction_cycles_and_turns_the_text() {
    use wordcraft_doc::props::TextDirection;
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 3}));
    run(&mut s, "text.insert", json!({"text": "Turned"}));
    let dirs = |s: &Session| -> Vec<TextDirection> {
        s.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows[0].cells.iter().map(|c| c.props.text_direction).collect()
    };
    let turned = |s: &mut Session| {
        s.layout().pages[0].items.iter().find_map(|it| match it {
            wordcraft_layout::Placed::Lines { para, turn, .. } if para.lines.first().is_some_and(|l| l.stop > 0) => Some(*turn),
            _ => None,
        })
    };
    assert_eq!(turned(&mut s), Some(TextDirection::Horizontal));
    run(&mut s, "table.textDirection", json!({}));
    assert_eq!(dirs(&s), [TextDirection::Down, TextDirection::Horizontal, TextDirection::Horizontal]);
    assert_eq!(turned(&mut s), Some(TextDirection::Down), "the text is drawn turned");
    run(&mut s, "table.textDirection", json!({}));
    assert_eq!(dirs(&s)[0], TextDirection::Up);
    assert_eq!(turned(&mut s), Some(TextDirection::Up));
    run(&mut s, "table.textDirection", json!({}));
    assert_eq!(dirs(&s)[0], TextDirection::Horizontal);
    // Every selected cell gets the caret cell's next direction (the issue's selection spans cells).
    let a = s.sel.focus.clone();
    let mut b = a.clone();
    if let Some(last) = b.path.0.get_mut(2) {
        *last = 1;
    }
    b.off = 0;
    s.sel = crate::Selection { anchor: a, focus: b };
    run(&mut s, "table.textDirection", json!({}));
    assert_eq!(dirs(&s), [TextDirection::Down, TextDirection::Down, TextDirection::Horizontal]);
    run(&mut s, "table.textDirection", json!({"value": "up"}));
    assert_eq!(dirs(&s), [TextDirection::Up, TextDirection::Up, TextDirection::Horizontal]);
    assert!(s.run("table.textDirection", &json!({"value": "sideways"})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(dirs(&s), [TextDirection::Down, TextDirection::Down, TextDirection::Horizontal]);
    // Outside a table it is disabled.
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    if s.sel.focus.path.cell().is_none() {
        assert!(s.run("table.textDirection", &json!({})).is_err());
    }
}

/// While the caret is in a table, Word shows its move handle past the top-left corner and its
/// resize handle past the bottom-right one (#2428); the app paints the ones this gives.
#[test]
fn the_caret_table_has_its_handles() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let (tp, _, _) = s.sel.focus.path.cell().unwrap();
    let l = s.layout();
    let (m, z) = crate::cmd::table_mouse::caret_handles(&l, &s.doc, s.sel.focus.story, &tp, 0);
    let (m, z) = (m.expect("the move handle"), z.expect("the resize handle"));
    let (_, x, y) = crate::cmd::table_mouse::corner(&l, s.sel.focus.story, &tp).unwrap();
    assert!(m.right() <= x && m.bottom() <= y, "past the top-left corner");
    assert!(z.x > m.right() && z.y > m.bottom(), "past the bottom-right one");
    // Exactly where the pointer handles are, so a click on them grabs them.
    let mid = |r: wordcraft_geom::Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (hx, hy) = mid(m);
    let g = crate::cmd::table_mouse::handle_table(&l, &s.doc, s.sel.focus.story, 0, hx, hy, 4.0).expect("the move handle grabs");
    assert_eq!(crate::cmd::table_mouse::handle(&g), m);
    let (hx, hy) = mid(z);
    let g = crate::cmd::table_resize::table_under(&l, &s.doc, 0, hx, hy, 4.0).expect("the resize handle grabs");
    assert_eq!(g.handle(), z);
    let (m, z) = crate::cmd::table_mouse::caret_handles(&l, &s.doc, s.sel.focus.story, &tp, 1);
    assert!(m.is_none() && z.is_none(), "not on a page without the table");
}

/// A table over more than one page shows its move handle only where it starts and its resize
/// handle only where it ends, also when its header row repeats or its one row runs on (#2428).
#[test]
fn the_caret_table_shows_its_handles_where_it_starts_and_ends() {
    for header in [true, false] {
        let mut s = s();
        if header {
            // 60 rows under a header row that repeats on the next page.
            run(&mut s, "insert.table", json!({"rows": 60, "cols": 2}));
            run(&mut s, "table.properties", json!({"headerRow": true}));
        } else {
            // One row whose cell runs on to the next page.
            run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
            for i in 0..80 {
                run(&mut s, "text.insert", json!({"text": format!("Line {i}")}));
                run(&mut s, "text.newParagraph", json!({}));
            }
        }
        let (tp, _, _) = s.sel.focus.path.cell().unwrap();
        let story = s.sel.focus.story;
        let l = s.layout();
        let holds = |p: usize, r: Option<usize>| {
            l.pages[p]
                .items
                .iter()
                .any(|it| matches!(it, wordcraft_layout::Placed::Cell { table, row, .. } if *table == tp && r.is_none_or(|r| *row == r)))
        };
        assert!(l.pages.len() >= 2 && holds(1, Some(0)), "row 0 is drawn on page 2 too (header {header})");
        let end = (0..l.pages.len()).rev().find(|p| holds(*p, None)).unwrap();
        for p in 0..l.pages.len() {
            let (m, z) = crate::cmd::table_mouse::caret_handles(&l, &s.doc, story, &tp, p);
            assert_eq!((m.is_some(), z.is_some()), (p == 0, p == end), "page {p} (header {header})");
        }
    }
}

/// Right-to-left tables and sections (#362): `table.rtl` and `layout.sectionRtl` set or toggle
/// the direction, one Undo step each, and the layout mirrors the table.
#[test]
fn table_and_section_direction_commands() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 3}));
    let rtl = |s: &Session| s.doc.body.iter().find_map(|b| b.as_table()).unwrap().props.bidi_visual;
    let first_cell_x = |s: &mut Session| {
        s.layout().pages[0].items.iter().find_map(|it| match it {
            wordcraft_layout::Placed::Cell { rect, cell: 0, .. } => Some(rect.x),
            _ => None,
        })
    };
    let ltr_x = first_cell_x(&mut s).unwrap();
    run(&mut s, "table.rtl", json!({}));
    assert!(rtl(&s));
    assert!(first_cell_x(&mut s).unwrap() > ltr_x + 100.0, "the first column moves to the right");
    run(&mut s, "table.rtl", json!({"value": true}));
    assert!(rtl(&s), "an explicit value sets rather than toggles");
    run(&mut s, "table.properties", json!({"rtl": false}));
    assert!(!rtl(&s));
    run(&mut s, "edit.undo", json!({}));
    assert!(rtl(&s));
    assert_eq!(s.run("table.properties", &json!({})).unwrap()["bidiVisual"], json!(true));
    run(&mut s, "layout.sectionRtl", json!({}));
    assert!(crate::cmd::page::sect(&s).rtl);
    run(&mut s, "layout.sectionRtl", json!({"value": false}));
    assert!(!crate::cmd::page::sect(&s).rtl);
    run(&mut s, "edit.undo", json!({}));
    assert!(crate::cmd::page::sect(&s).rtl);
    // Outside a table `table.rtl` is disabled.
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    if s.sel.focus.path.cell().is_none() {
        assert!(s.run("table.rtl", &json!({})).is_err());
    }
}

#[test]
fn table_border_presets_mask_the_sides_they_clear() {
    // On a default (TableGrid) table the "outside"/"inside" presets must write nil over the
    // sides they clear; without it those sides inherit the style's grid (#143) and every
    // preset renders like "all".
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    // Distinct vertical/horizontal rule positions on page 0.
    let rule_pos = |s: &mut Session| {
        let (mut vx, mut hy) = (Vec::new(), Vec::new());
        for it in &s.layout().pages[0].items {
            if let crate::layout::Placed::Rule { x0, y0, x1, y1, .. } = it {
                if (x0 - x1).abs() < 1e-3 && !vx.contains(x0) {
                    vx.push(*x0);
                }
                if (y0 - y1).abs() < 1e-3 && !hy.contains(y0) {
                    hy.push(*y0);
                }
            }
        }
        (vx, hy)
    };
    // Style default: a full 2x2 grid — 3 vertical + 3 horizontal lines.
    let (vx, hy) = rule_pos(&mut s);
    assert_eq!((vx.len(), hy.len()), (3, 3), "baseline grid: {vx:?} {hy:?}");
    run(&mut s, "table.borders", json!({"kind": "outside"}));
    let (vx, hy) = rule_pos(&mut s);
    assert_eq!((vx.len(), hy.len()), (2, 2), "outside keeps only the frame: {vx:?} {hy:?}");
    let b = s.doc.body.iter().find_map(|x| x.as_table()).unwrap().props.borders.unwrap();
    assert!(b.between.is_some_and(|x| x.style == BorderStyle::None));
    assert!(b.inside_v.is_some_and(|x| x.style == BorderStyle::None));
    run(&mut s, "table.borders", json!({"kind": "inside"}));
    let (vx, hy) = rule_pos(&mut s);
    assert_eq!((vx.len(), hy.len()), (1, 1), "inside keeps only the inner rules: {vx:?} {hy:?}");
    let b = s.doc.body.iter().find_map(|x| x.as_table()).unwrap().props.borders.unwrap();
    for side in [b.top, b.bottom, b.left, b.right] {
        assert!(side.is_some_and(|x| x.style == BorderStyle::None));
    }
}

#[test]
fn page_setup_and_breaks() {
    let mut s = s();
    run(&mut s, "layout.orientation", json!({"value": "landscape"}));
    assert!(s.doc.last_section.landscape);
    run(&mut s, "layout.size", json!({"name": "A4"}));
    assert!((s.doc.last_section.page_w - 841.89).abs() < 0.1);
    run(&mut s, "layout.margins", json!({"preset": "narrow"}));
    assert_eq!(s.doc.last_section.margin_left, 36.0);
    run(&mut s, "text.insert", json!({"text": "one"}));
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "text.insert", json!({"text": "two"}));
    assert_eq!(s.doc.sections().len(), 2);
    assert_eq!(s.layout().pages.len(), 2);
    assert!(s.run("layout.margins", &json!({"left": 1000.0})).is_err());
    // Mirrored margins: from the preset or `mirror` (#1181); plain numbers keep the setting.
    run(&mut s, "layout.margins", json!({"preset": "mirrored"}));
    assert!(s.doc.settings.mirror_margins);
    run(&mut s, "layout.margins", json!({"left": 80.0}));
    assert!(s.doc.settings.mirror_margins);
    run(&mut s, "layout.margins", json!({"left": 80.0, "mirror": false}));
    assert!(!s.doc.settings.mirror_margins && s.doc.last_section.margin_left == 80.0);
    run(&mut s, "layout.margins", json!({"mirror": true}));
    assert!(s.doc.settings.mirror_margins);
}

/// #1433: a custom page size (More Paper Sizes…) sets the section in its orientation, and
/// out-of-range sizes clamp to Word's 0.1"–22" instead of failing or reaching the layout.
#[test]
fn layout_size_takes_a_custom_size_and_clamps_it() {
    let mut s = s();
    let size = |s: &Session| (s.doc.last_section.page_w, s.doc.last_section.page_h);
    run(&mut s, "layout.size", json!({"width": 300.0, "height": 500.0}));
    assert_eq!(size(&s), (300.0, 500.0));
    run(&mut s, "layout.margins", json!({"top": 0.0, "bottom": 0.0, "left": 0.0, "right": 0.0}));
    run(&mut s, "layout.size", json!({"width": 40.0, "height": 5000.0}));
    assert_eq!(size(&s), (40.0, 1584.0), "clamped to 22\"");
    for junk in [
        json!({"width": 1e300, "height": 400.0}),
        json!({"width": "NaN", "height": 400.0}),
        json!({"width": null}),
        json!({}),
        // Clamped to 0.1", still less than the 36 pt of text room the margins need.
        json!({"width": -50.0, "height": 400.0}),
        json!({"width": 1.0, "height": 400.0}),
    ] {
        assert!(s.run("layout.size", &junk).is_err(), "{junk}");
        assert_eq!(size(&s), (40.0, 1584.0), "{junk} leaves the size alone");
    }
    // A landscape section keeps its orientation: the long side goes across.
    run(&mut s, "layout.orientation", json!({"value": "landscape"}));
    run(&mut s, "layout.size", json!({"width": 300.0, "height": 500.0}));
    assert_eq!(size(&s), (500.0, 300.0));
    assert!(s.doc.last_section.landscape);
}

/// #1433: `layout.size` refuses a page its margins don't fit, by the rule `layout.margins` uses
/// (36 pt of text room each way), and changes nothing.
#[test]
fn layout_size_refuses_a_page_its_margins_dont_fit() {
    let mut s = s();
    run(&mut s, "layout.margins", json!({"preset": "normal"}));
    let before = s.doc.last_section.clone();
    for (w, h) in [(100.0, 500.0), (500.0, 100.0), (179.0, 500.0)] {
        let e = s.run("layout.size", &json!({"width": w, "height": h})).unwrap_err().to_string();
        assert!(e.contains("margins don't fit"), "{w}×{h}: {e}");
        assert_eq!(s.doc.last_section, before, "{w}×{h} changes nothing");
    }
    run(&mut s, "layout.size", json!({"width": 180.0, "height": 180.0}));
    assert_eq!((s.doc.last_section.page_w, s.doc.last_section.page_h), (180.0, 180.0), "exactly 36 pt of room is enough");
}

/// A negative margin (or gutter) counts as zero, so it can't offset an oversized one past the
/// text-room check: `layout.margins` refuses and changes nothing.
#[test]
fn layout_margins_negative_cannot_hide_an_oversized_margin() {
    let mut s = s();
    run(&mut s, "layout.margins", json!({"preset": "normal"}));
    let before = s.doc.last_section.clone();
    for v in [json!({"top": 2000, "bottom": -2000}), json!({"left": 2000, "right": -2000}), json!({"left": 600, "right": 0, "gutter": -600})] {
        let e = s.run("layout.margins", &v).unwrap_err().to_string();
        assert!(e.contains("margins leave no room for text"), "{v}: {e}");
        assert_eq!(s.doc.last_section, before, "{v} changes nothing");
    }
}

/// #2072: `layout.size` when the focus is in a header or footer modifies that section, not
/// always section 0, and `sect` reports the focused section's properties.
#[test]
fn layout_size_when_focus_is_in_header_updates_correct_section() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "section 1"}));
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "text.insert", json!({"text": "section 2"}));
    assert_eq!(s.doc.sections().len(), 2);

    let hid = s.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![std::sync::Arc::new(wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::with_text("header text", Default::default())))],
    );
    s.doc.last_section.headers.default = Some(hid);

    // Focus inside section 2's header
    s.sel = crate::Selection::caret(s.doc.start_of(wordcraft_doc::StoryRef::Part(hid)));
    assert_eq!(crate::cmd::page::sect(&s).page_w, s.doc.last_section.page_w);

    // Change size to A4
    run(&mut s, "layout.size", json!({"name": "A4"}));
    assert!((s.doc.last_section.page_w - 595.28).abs() < 0.1);
    assert!((s.doc.last_section.page_h - 841.89).abs() < 0.1);

    // Section 1 remains Letter (612.0 x 792.0)
    let s0 = s.doc.sections()[0].1;
    assert_eq!((s0.page_w, s0.page_h), (612.0, 792.0));
}

/// #1433: the smallest page there can be (0.1" wide, no margins, set straight through Page
/// Setup's section) still lays out and exports text, a table and two columns.
#[test]
fn a_tiny_page_lays_out_and_exports() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "A paragraph of text that cannot fit on so small a page."}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 3}));
    run(&mut s, "text.insert", json!({"text": "cell"}));
    run(&mut s, "layout.columns", json!({"count": 2}));
    let mut sect = s.doc.last_section.clone();
    (sect.page_w, sect.page_h) = (7.2, 40.0);
    (sect.margin_top, sect.margin_bottom, sect.margin_left, sect.margin_right, sect.gutter) = (0.0, 0.0, 0.0, 0.0, 0.0);
    run(&mut s, "layout.pageSetup", json!({"section": serde_json::to_value(&sect).unwrap()}));
    assert_eq!((s.doc.last_section.page_w, s.doc.last_section.page_h), (7.2, 40.0));
    assert!(!s.layout().pages.is_empty());
    let pdf = crate::io::save_bytes("tiny.pdf", &s.doc).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn track_changes_and_accept() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "original"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": " added"}));
    run(&mut s, "select.text", json!({"text": "orig"}));
    s.author = "Someone Else".into();
    run(&mut s, "text.delete", json!({}));
    // Deleted text stays (marked) until accepted.
    assert_eq!(text(&s), "original added");
    let ch = run(&mut s, "review.changes", json!({}));
    assert_eq!(ch.as_array().unwrap().len(), 2);
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "inal added");
}

#[test]
fn document_text_drops_tracked_deletions() {
    // #866: document.text (MCP get_text) omits tracked deletions; plain_text still includes them.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Signed by Alice Smith, Partner."}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.text", json!({"text": "Alice Smith, Partner"}));
    run(&mut s, "text.insert", json!({"text": "Bob Jones, Director"}));
    assert_eq!(text(&s), "Signed by Bob Jones, DirectorAlice Smith, Partner.");
    assert_eq!(run(&mut s, "document.text", json!({}))["text"], "Signed by Bob Jones, Director.");
}

/// The author of the revision `rid` points to.
fn author_of(s: &Session, rid: Option<u32>) -> Option<String> {
    rid.and_then(|r| s.doc.revisions.get(r as usize)).map(|r| r.author.clone())
}

#[test]
fn tracked_delete_keeps_another_authors_deletion() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "alpha beta gamma"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Ana".into();
    run(&mut s, "select.text", json!({"text": "beta"}));
    run(&mut s, "text.delete", json!({}));
    // A second author deletes the whole line, including the text Ana already deleted.
    s.author = "Ben".into();
    run(&mut s, "select.text", json!({"text": "alpha beta gamma"}));
    run(&mut s, "text.delete", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let by: Vec<(String, Option<String>)> = p.run_ranges().map(|(r, c)| (p.text[r].to_string(), author_of(&s, c.del))).collect();
    assert!(by.iter().any(|(t, a)| t == "beta" && a.as_deref() == Some("Ana")), "{by:?}");
    assert!(by.iter().any(|(t, a)| t.contains("alpha") && a.as_deref() == Some("Ben")), "{by:?}");
    assert!(by.iter().any(|(t, a)| t.contains("gamma") && a.as_deref() == Some("Ben")), "{by:?}");
}

#[test]
fn tracked_split_gives_the_new_paragraph_mark_to_its_author() {
    // Ben presses Enter at the end of Ana's insertion, and in the middle of it.
    for off in [None, Some(8)] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "alpha"}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        s.author = "Ana".into();
        run(&mut s, "text.insert", json!({"text": " beta"}));
        s.author = "Ben".into();
        if let Some(off) = off {
            run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [0], "off": off}}));
        }
        run(&mut s, "text.newParagraph", json!({}));
        let head = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let tail = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap();
        assert_eq!(author_of(&s, head.mark.ins).as_deref(), Some("Ben"), "split at {off:?}: the new paragraph mark is Ben's");
        // The second paragraph ends with the original mark, which nobody inserted (not Ana).
        assert_eq!(tail.mark.ins, None, "split at {off:?}");
        assert_eq!(tail.mark.del, None, "split at {off:?}");
    }
}

fn body_texts(s: &Session) -> Vec<String> {
    s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect()
}

/// Issue #229's steps: a paragraph split while tracking changes.
fn tracked_split(text: &str) -> Session {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": text}));
    run(&mut s, "file.setAuthor", json!({"name": "Owned Bob"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 12}}));
    run(&mut s, "text.newParagraph", json!({}));
    s
}

#[test]
fn reject_all_removes_a_tracked_paragraph_break() {
    // Issue #229: Reject All left the paragraph break typed with Track Changes on.
    for (text, head) in [("Owned ALPHA Owned BETA", "Owned ALPHA "), ("Owned GAMMA Owned DELTA", "Owned GAMMA ")] {
        let mut s = tracked_split(text);
        assert_eq!(body_texts(&s), [head, &text[12..]]);
        // The break is listed as a change, by its author.
        let ch = run(&mut s, "review.changes", json!({}));
        assert_eq!(ch.as_array().map(Vec::len), Some(1), "{ch}");
        assert_eq!((ch[0]["kind"].as_str(), ch[0]["author"].as_str()), (Some("insert"), Some("Owned Bob")));
        run(&mut s, "review.rejectAll", json!({}));
        assert_eq!(body_texts(&s), [text]);
        assert!(s.doc.revisions.is_empty());
        let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
        assert_eq!((p.mark.ins, p.mark.del), (None, None));
        // Undo brings the tracked split back; redo rejects it again.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(body_texts(&s), [head, &text[12..]]);
        assert!(s.doc.para_at(&Pos::body(0, 0)).unwrap().mark.ins.is_some());
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(body_texts(&s), [text]);
    }
    // Split → Undo → Redo → Reject All (the issue's other variant).
    let mut s = tracked_split("Owned ALPHA Owned BETA");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_texts(&s), ["Owned ALPHA Owned BETA"]);
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(body_texts(&s), ["Owned ALPHA Owned BETA"]);
}

#[test]
fn accept_all_keeps_a_tracked_paragraph_break() {
    let mut s = tracked_split("Owned ALPHA Owned BETA");
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(body_texts(&s), ["Owned ALPHA ", "Owned BETA"]);
    assert!(s.doc.revisions.is_empty());
    assert!(s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.mark.ins.is_none() && p.mark.del.is_none()));
    assert_eq!(run(&mut s, "review.changes", json!({})), json!([]));
}

/// Issue #864: five paragraphs, a tracked deletion from the end of the first to the end of the
/// third. The two breaks inside the range and the text of the second and third paragraphs are
/// deleted; the third paragraph's own break is not.
fn tracked_delete_from_paragraph_end() -> Session {
    let mut s = s();
    for (i, word) in ["Alpha", "Bravo", "Charlie", "Delta", "Echo"].iter().enumerate() {
        if i > 0 {
            run(&mut s, "text.newParagraph", json!({}));
        }
        run(&mut s, "text.insert", json!({"text": word}));
    }
    run(&mut s, "review.trackChanges", json!({"value": true}));
    let range = json!({"anchor": {"path": [0], "off": 5, "story": "body"}, "focus": {"path": [2], "off": 7, "story": "body"}});
    run(&mut s, "select.range", range);
    run(&mut s, "text.delete", json!({}));
    s
}

#[test]
fn accept_all_after_a_tracked_delete_from_a_paragraph_end_keeps_the_next_paragraph() {
    let mut s = tracked_delete_from_paragraph_end();
    assert_eq!(body_texts(&s), ["Alpha", "Bravo", "Charlie", "Delta", "Echo"], "tracked deletions stay in the text");
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(body_texts(&s), ["Alpha", "Delta", "Echo"]);
    assert!(s.doc.revisions.is_empty());
    assert!(s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.mark.ins.is_none() && p.mark.del.is_none()));
    // Rejecting instead restores everything, as it already did.
    let mut s = tracked_delete_from_paragraph_end();
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(body_texts(&s), ["Alpha", "Bravo", "Charlie", "Delta", "Echo"]);
}

#[test]
fn reject_all_after_typing_into_an_empty_paragraph_keeps_the_paragraph() {
    // Same cause as #864 seen from the other side: rejecting the typed text emptied the
    // paragraph, and its mark picked up the text's insertion, so it was rejected as well.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "One"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Three"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    run(&mut s, "text.insert", json!({"text": "Two"}));
    assert_eq!(body_texts(&s), ["One", "Two", "Three"]);
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(body_texts(&s), ["One", "", "Three"]);
}

#[test]
fn reject_one_paragraph_break_and_rejecting_a_typed_paragraph() {
    // Reject with the caret on the break (Next Change selects it), or at the end of its paragraph.
    let mut s = tracked_split("Owned ALPHA Owned BETA");
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 0}}));
    run(&mut s, "review.nextChange", json!({}));
    run(&mut s, "review.reject", json!({}));
    assert_eq!(body_texts(&s), ["Owned ALPHA Owned BETA"]);
    let mut s = tracked_split("Owned ALPHA Owned BETA");
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 12}}));
    run(&mut s, "review.accept", json!({}));
    assert_eq!(body_texts(&s), ["Owned ALPHA ", "Owned BETA"]);
    assert_eq!(run(&mut s, "review.changes", json!({})), json!([]));

    // Enter at the end of a heading, then a typed paragraph: rejecting everything restores
    // the heading alone, with its style.
    let mut s = self::s();
    run(&mut s, "text.insert", json!({"text": "Title"}));
    run(&mut s, "para.style", json!({"style": "Heading1"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Body"}));
    run(&mut s, "text.newParagraph", json!({}));
    assert_eq!(body_texts(&s), ["Title", "Body", ""]);
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(body_texts(&s), ["Title"]);
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.style.as_deref(), Some("Heading1"));
}

#[test]
fn reject_all_of_an_inserted_paragraph_keeps_the_next_paragraphs_style() {
    // A whole paragraph inserted before a heading, in another style: rejecting it removes the
    // paragraph, and the heading keeps its own style (it used to take the rejected one's).
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Intro"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Heading"}));
    run(&mut s, "para.style", json!({"style": "Heading1"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    run(&mut s, "text.insert", json!({"text": "Inserted"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "review.trackChanges", json!({"value": false}));
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    run(&mut s, "para.style", json!({"style": "Normal"}));
    assert_eq!(body_texts(&s), ["Intro", "Inserted", "Heading"]);
    let style = |s: &Session, i: usize| s.doc.para_at(&Pos::body(i, 0)).unwrap().props.style.clone();
    assert_eq!(style(&s, 2).as_deref(), Some("Heading1"));
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(body_texts(&s), ["Intro", "Heading"]);
    assert_eq!(style(&s, 1).as_deref(), Some("Heading1"));
    // Accepting instead keeps all three, each with its style.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(body_texts(&s), ["Intro", "Inserted", "Heading"]);
    assert_eq!(style(&s, 2).as_deref(), Some("Heading1"));
}

#[test]
fn tracked_backspace_and_delete_mark_a_paragraph_break_deleted() {
    let two = || {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "One"}));
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": "Two"}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        s.author = "Ana".into();
        s
    };
    // Backspace at the start of "Two": the mark above is deleted, the caret moves before it.
    for accept in [false, true] {
        let mut s = two();
        run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
        run(&mut s, "text.backspace", json!({}));
        assert_eq!(body_texts(&s), ["One", "Two"], "a tracked deletion stays in the text");
        assert_eq!(s.sel.focus, Pos::body(0, 3));
        assert_eq!(author_of(&s, s.doc.para_at(&Pos::body(0, 0)).unwrap().mark.del).as_deref(), Some("Ana"));
        let ch = run(&mut s, "review.changes", json!({}));
        assert_eq!(ch[0]["kind"], "delete", "{ch}");
        if accept {
            run(&mut s, "review.acceptAll", json!({}));
            assert_eq!(body_texts(&s), ["OneTwo"]);
        } else {
            run(&mut s, "review.rejectAll", json!({}));
            assert_eq!(body_texts(&s), ["One", "Two"]);
            assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().mark.del, None);
        }
    }

    // Delete at the end of "One": the same, and the caret steps over the deleted mark.
    let mut s = two();
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 3}}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(body_texts(&s), ["One", "Two"]);
    assert_eq!(s.sel.focus, Pos::body(1, 0));
    assert!(s.doc.para_at(&Pos::body(0, 0)).unwrap().mark.del.is_some());
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(body_texts(&s), ["OneTwo"]);

    // A break the same author inserted is simply removed again.
    let mut s = two();
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 3}}));
    run(&mut s, "text.newParagraph", json!({}));
    assert_eq!(body_texts(&s), ["One", "Two", ""]);
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(body_texts(&s), ["One", "Two"]);
    assert!(s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.mark.ins.is_none() && p.mark.del.is_none()));
}

#[test]
fn set_author_names_tracked_changes_headlessly() {
    // Issue #90: a script or agent labels its own edits without any dialog.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "original"}));
    let r = run(&mut s, "file.setAuthor", json!({"name": "  Claude (copyedit)\n"}));
    assert_eq!(r["name"], "Claude (copyedit)");
    assert!(s.ui_requests.is_empty(), "programmatic calls never open dialogs");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": " added"}));
    run(&mut s, "select.text", json!({"text": "orig"}));
    run(&mut s, "text.delete", json!({}));
    let ch = run(&mut s, "review.changes", json!({}));
    let ch = ch.as_array().unwrap();
    assert_eq!(ch.len(), 2);
    assert!(ch.iter().all(|c| c["author"] == "Claude (copyedit)"), "{ch:?}");
    // Comments use the same name.
    run(&mut s, "select.text", json!({"text": "added"}));
    run(&mut s, "review.newComment", json!({"text": "Tightened."}));
    assert!(s.doc.comments.values().all(|c| c.author == "Claude (copyedit)" && c.initials == "C("));
}

#[test]
fn privacy_options_strip_names_and_dates_from_the_saved_docx_only() {
    // Issue #1062: Remove personal information / date and time apply to the saved file.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "original"}));
    run(&mut s, "file.setAuthor", json!({"name": "Mira Calloway"}));
    run(&mut s, "file.properties", json!({"author": "Mira Calloway"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": " added"}));
    run(&mut s, "select.text", json!({"text": "added"}));
    run(&mut s, "review.newComment", json!({"text": "Tightened."}));
    let off = run(&mut s, "file.privacy", json!({}));
    assert_eq!(off, json!({"removePersonalInfo": false, "removeDateTime": false}));
    let plain = wordcraft_docx::write(&s.doc).unwrap();
    let back = wordcraft_docx::read(&plain).unwrap();
    assert!(back.comments.values().all(|c| c.author == "Mira Calloway" && !c.date.is_empty()));
    let on = run(&mut s, "file.privacyOptions", json!({"removePersonalInfo": true, "removeDateTime": true}));
    assert_eq!(on, json!({"removePersonalInfo": true, "removeDateTime": true}));
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert!(back.settings.remove_personal_info && back.settings.remove_date_time);
    assert!(back.comments.values().all(|c| c.author == "Author" && c.date.is_empty()), "{:?}", back.comments);
    assert!(!back.revisions.is_empty());
    assert!(back.revisions.iter().all(|r| r.author == "Author" && r.date.is_empty()), "{:?}", back.revisions);
    assert!(back.core.creator.is_empty() && back.core.last_modified_by.is_empty());
    // The open document keeps the real names and dates.
    assert!(s.doc.comments.values().all(|c| c.author == "Mira Calloway" && !c.date.is_empty()));
    assert!(s.doc.revisions.iter().any(|r| r.author == "Mira Calloway"));
    assert_eq!(s.doc.core.creator, "Mira Calloway");
    // Undoable like any document change.
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc.settings.remove_personal_info);
}

/// Reading the privacy settings changes nothing: no undo step, no unsaved changes, and it works
/// on a protected document. `file.privacyOptions` without a setting is refused, not a silent edit.
#[test]
fn reading_privacy_options_is_side_effect_free() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Draft"}));
    s.dirty = false;
    let undo = s.undo_depth();
    assert_eq!(run(&mut s, "file.privacy", json!({})), json!({"removePersonalInfo": false, "removeDateTime": false}));
    assert!(s.run("file.privacyOptions", &json!({})).is_err());
    assert!(s.run("file.privacyOptions", &json!({"removePersonalInfo": "yes"})).is_err());
    assert_eq!(s.undo_depth(), undo, "no undo step");
    assert!(!s.dirty, "no unsaved changes");
    s.doc.settings.protection = Some("readOnly".into());
    assert_eq!(run(&mut s, "file.privacy", json!({}))["removeDateTime"], false);
}

#[test]
fn set_author_refuses_blank_names_and_clamps_long_ones() {
    let mut s = s();
    for bad in [json!(null), json!({}), json!({"name": 5}), json!({"name": ""}), json!({"name": " \t\n "}), json!({"name": "\u{0}\u{7}"})] {
        assert!(s.run("file.setAuthor", &bad).is_err(), "{bad} should be refused");
        assert_eq!(s.author, "WordCraft User", "a refused name leaves the author alone");
    }
    run(&mut s, "file.setAuthor", json!({"name": "Ada\u{0}\u{1b} Lovelace"}));
    assert_eq!(s.author, "Ada Lovelace", "control characters can't reach the saved XML");
    run(&mut s, "file.setAuthor", json!({"name": "é".repeat(100_000)}));
    assert_eq!(s.author.chars().count(), cmd::file::MAX_AUTHOR_CHARS);
}

#[test]
fn track_changes_and_set_author_list_their_params() {
    // Scripts that only read the command list must find the explicit, idempotent form.
    let reg = cmd::registry();
    assert!(reg.get("review.trackChanges").unwrap().params.contains(r#""value"?: bool"#));
    assert!(reg.get("file.setAuthor").unwrap().params.contains(r#""name": string"#));
    let mut s = s();
    for _ in 0..2 {
        assert_eq!(run(&mut s, "review.trackChanges", json!({"value": true}))["value"], true);
    }
}

#[test]
fn line_and_page_break_toggles_take_an_explicit_value() {
    // #967: a script sets keep with next on a heading and clears it on the paragraph after,
    // which inherits it through Enter, without tracking the toggle state.
    let reg = cmd::registry();
    type Field = fn(&wordcraft_doc::ParaProps) -> Option<bool>;
    let cases: [(&str, Field); 4] = [
        ("para.keepNext", |p| p.keep_next),
        ("para.keepLines", |p| p.keep_lines),
        ("para.pageBreakBefore", |p| p.page_break_before),
        ("para.widowControl", |p| p.widow_control),
    ];
    for (id, field) in cases {
        assert!(reg.get(id).unwrap().params.contains(r#""value"?: bool"#), "{id} lists its param");
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "Heading"}));
        let at = |s: &Session, block| s.doc.para_at(&Pos::body(block, 0)).and_then(|p| field(&p.props));
        for value in [true, true, false, false, true] {
            run(&mut s, id, json!({"value": value}));
            assert_eq!(at(&s, 0), Some(value), "{id} value {value} sets, never toggles");
        }
        run(&mut s, "text.newParagraph", json!({}));
        assert_eq!(at(&s, 1), Some(true), "{id} carries over to the new paragraph");
        run(&mut s, id, json!({"value": false}));
        assert_eq!((at(&s, 0), at(&s, 1)), (Some(true), Some(false)), "{id} clears only the new paragraph");
        run(&mut s, id, json!({}));
        assert_eq!(at(&s, 1), Some(true), "{id} without value toggles");
        run(&mut s, id, json!({}));
        assert_eq!(at(&s, 1), Some(false), "{id} without value toggles back");
        run(&mut s, id, json!({"value": "yes"}));
        assert_eq!(at(&s, 1), Some(true), "{id} ignores a value that isn't a bool and toggles, as format.bold does");
    }
}

#[test]
fn no_markup_view_lays_out_the_final_text() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "original"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.text", json!({"text": "orig"}));
    run(&mut s, "text.delete", json!({}));
    let gap = |s: &mut Session| {
        let l = s.layout();
        let x = |off| l.caret(&Pos::body(0, off)).map(|c| c.x).unwrap_or(f32::NAN);
        x(4) - x(0)
    };
    assert!(gap(&mut s) > 5.0, "markup shows the deletion");
    run(&mut s, "review.markup", json!({"value": "noMarkup"}));
    assert!(gap(&mut s).abs() < 0.01, "No Markup leaves it out");
    run(&mut s, "review.showMarkup", json!({"value": true}));
    assert!(gap(&mut s) > 5.0);
    run(&mut s, "review.showMarkup", json!({"value": false}));
    assert!(gap(&mut s).abs() < 0.01);
}

#[test]
fn replace_all_is_tracked() {
    let mut s = s();
    let original = "We walked towards the light, then towards home.";
    run(&mut s, "document.setText", json!({"text": original}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    let r = run(&mut s, "edit.replaceAll", json!({"text": "towards", "with": "toward", "matchCase": true}));
    assert_eq!(r["replaced"], 2);
    let ch = run(&mut s, "review.changes", json!({}));
    assert!(!ch.as_array().unwrap().is_empty(), "replace all must leave tracked revisions");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(text(&s), original);
}

#[test]
fn comments() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Some text here"}));
    run(&mut s, "select.text", json!({"text": "text"}));
    let r = run(&mut s, "review.newComment", json!({"text": "Nice"}));
    let id = r["id"].as_u64().unwrap();
    let l = run(&mut s, "review.comments", json!({}));
    assert_eq!(l[0]["text"], "Nice");
    assert_eq!(text(&s), "Some text here");
    // Editing the text is one undo step; replies hang off their comment.
    run(&mut s, "review.editComment", json!({"id": id, "text": "Nicer\nTwo lines"}));
    assert_eq!(run(&mut s, "review.comments", json!({}))[0]["text"], "Nicer\nTwo lines");
    assert!(s.undo());
    assert_eq!(run(&mut s, "review.comments", json!({}))[0]["text"], "Nice");
    assert!(s.run("review.editComment", &json!({"id": 999, "text": "x"})).is_err());
    let r = run(&mut s, "review.reply", json!({"id": id, "text": "Agreed"}));
    assert_eq!(s.doc.comments.get(&(r["id"].as_u64().unwrap() as u32)).unwrap().parent, Some(id as u32));
    run(&mut s, "review.deleteComment", json!({"id": id}));
    assert!(s.doc.comments.is_empty());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().objects.len(), 0);
}

/// #1085: a new comment with no text, in an empty document or on a word, goes with Review ›
/// Delete at the caret (no id) and by id.
#[test]
fn empty_comments_delete() {
    for body in ["", "Some text here"] {
        for by_id in [false, true] {
            let mut s = Session::new(wordcraft_doc::Document::new());
            if !body.is_empty() {
                run(&mut s, "text.insert", json!({"text": body}));
                run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [0], "off": 6}}));
            }
            let id = run(&mut s, "review.newComment", json!({}))["id"].clone();
            assert_eq!(run(&mut s, "review.comments", json!({}))[0]["text"], "");
            run(&mut s, "review.deleteComment", if by_id { json!({"id": id}) } else { json!({}) });
            assert!(s.doc.comments.is_empty(), "{body:?} by id {by_id}");
            assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().objects.len(), 0, "{body:?}: anchors gone");
            assert_eq!(text(&s), body);
        }
    }
}

#[test]
fn toc_and_fields() {
    let mut s = Session::new(crate::sample::report());
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({}));
    let t = text(&s);
    assert!(t.contains("Contents"), "{t}");
    assert!(t.contains("Summary\t1"), "{t}");
    run(&mut s, "references.updateToc", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body).matches("Summary\t").count(), 1, "one TOC entry after update");
}

/// A comment or reply made here carries its UTC time as `date_utc` too, so it shows in local time
/// and is saved as `dateUtc`.
#[test]
fn new_comments_and_replies_carry_their_utc_time() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Some text"}));
    run(&mut s, "select.all", json!({}));
    let id = run(&mut s, "review.newComment", json!({"text": "one"}))["id"].as_u64().unwrap();
    run(&mut s, "review.reply", json!({"id": id, "text": "two"}));
    assert_eq!(s.doc.comments.len(), 2);
    for c in s.doc.comments.values() {
        assert!(c.date.ends_with('Z'), "{}", c.date);
        assert_eq!(c.date_utc.as_deref(), Some(c.date.as_str()));
    }
}

/// Update Table with nothing to change lays the document out at the revisions it touches, then
/// puts the revision back. The next edits reach those numbers again with a different document, so
/// the layout must be the new one.
#[test]
fn a_command_that_changes_nothing_leaves_no_stale_layout() {
    let mut s = Session::new(crate::sample::report());
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({}));
    let (rev, steps) = (s.rev(), s.undo_depth());
    let pages = s.layout().pages.len();
    run(&mut s, "references.updateToc", json!({}));
    assert_eq!((s.rev(), s.undo_depth()), (rev, steps), "Update Table changed nothing");
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": "A"}));
    run(&mut s, "text.insert", json!({"text": "word ".repeat(4000)}));
    assert!(s.layout().pages.len() > pages, "the layout has the inserted text");
}

#[test]
fn toc_refresh_after_save_and_reopen() {
    for extension in ["docx", "json"] {
        for (headings, levels) in [(&["Top"][..], 1), (&["Top", "Next"][..], 1), (&["Top"][..], 4)] {
            let mut s = s();
            run(&mut s, "document.setText", json!({"text": headings.join("\n")}));
            for i in 0..headings.len() {
                run(&mut s, "caret.set", json!({"pos": Pos::body(i, 0)}));
                run(&mut s, "styles.apply", json!({"style": "Heading 1"}));
            }
            // A level-two heading must stay out of a depth-one TOC after reopening.
            if levels == 1 {
                run(&mut s, "caret.docEnd", json!({}));
                run(&mut s, "text.newParagraph", json!({}));
                run(&mut s, "text.insert", json!({"text": "Detail"}));
                run(&mut s, "styles.apply", json!({"style": "Heading 2"}));
            }
            run(&mut s, "caret.docStart", json!({}));
            run(&mut s, "references.toc", json!({"levels": levels}));
            run(&mut s, "references.updateToc", json!({}));
            let expected = text(&s);
            let name = format!("toc.{extension}");
            let bytes = crate::io::save_bytes(&name, &s.doc).unwrap();
            s = Session::new(crate::io::open_bytes(&name, &bytes).unwrap());
            for command in ["references.updateToc", "references.updateToc", "references.updateFields"] {
                run(&mut s, command, json!({}));
                assert_eq!(text(&s), expected, "{extension}, levels={levels}, {command}");
            }
            let heading = s
                .doc
                .body
                .iter()
                .position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("Heading1") && p.plain_text() == "Top"))
                .unwrap();
            // Hyperlinked TOCs bookmark their headings; select the text after those markers.
            let off = s.doc.body[heading].as_para().unwrap().text.find("Top").unwrap();
            run(&mut s, "select.range", json!({"anchor": Pos::body(heading, off), "focus": Pos::body(heading, off + 3)}));
            run(&mut s, "text.insert", json!({"text": "Revised"}));
            let before_refresh = text(&s);
            run(&mut s, "references.updateToc", json!({}));
            let revised = text(&s);
            assert_eq!(revised.matches("Revised\t").count(), 1, "{revised}");
            assert!(!revised.contains("Top"), "{revised}");
            assert_eq!(revised.matches("Next\t").count(), headings.len() - 1, "{revised}");
            assert_eq!(revised.matches("Detail\t").count(), 0, "{revised}");
            assert_eq!(revised.lines().filter(|line| *line == "Detail").count(), usize::from(levels == 1), "{revised}");
            assert!(s.undo());
            assert_eq!(text(&s), before_refresh);
            run(&mut s, "references.updateToc", json!({}));
            assert_eq!(text(&s), revised);
            let bytes = crate::io::save_bytes(&name, &s.doc).unwrap();
            s = Session::new(crate::io::open_bytes(&name, &bytes).unwrap());
            run(&mut s, "references.updateFields", json!({}));
            assert_eq!(text(&s), revised, "saved edited TOC, {extension}");
        }
    }
}

/// A TOC imported as a range field (no TOC heading, so it isn't turned back into the engine's
/// TOC on open) is still found by Update Table, Update Field and Table of Contents: it is
/// refreshed in place, not duplicated, and the refresh is one undo step.
#[test]
fn imported_range_toc_is_refreshed_in_place() {
    use wordcraft_doc::para::InlineObject;
    use wordcraft_doc::props::CharProps;
    for locked in [false, true] {
        let mut d = wordcraft_doc::Document::from_text("Before.\nOld entry\t9\nAlpha\nBeta");
        {
            let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap();
            let n = p.len();
            p.insert_object(n, InlineObject::FieldEnd, &CharProps::default()).unwrap();
            p.insert_object(0, InlineObject::FieldStart { instr: r#"TOC \o "1-2" \h"#.into(), locked, dirty: false }, &CharProps::default()).unwrap();
        }
        let mut s = Session::new(d);
        for (i, style) in [(2, "Heading 1"), (3, "Heading 2")] {
            run(&mut s, "caret.set", json!({"pos": Pos::body(i, 0)}));
            run(&mut s, "styles.apply", json!({"style": style}));
        }
        // Through DOCX, as an imported file is.
        let bytes = wordcraft_docx::write(&s.doc).unwrap();
        s = Session::new(wordcraft_docx::read(&bytes).unwrap());
        assert_eq!(s.doc.field_ranges(StoryRef::Body).len(), 1, "read back as a range field");
        let before = text(&s);
        run(&mut s, "references.updateToc", json!({}));
        let refreshed = text(&s);
        if locked {
            // Like Word, a locked field keeps its result.
            assert_eq!(refreshed, before);
            continue;
        }
        assert!(!refreshed.contains("Old entry"), "{refreshed}");
        assert!(refreshed.starts_with("Before.\n"), "{refreshed}");
        assert_eq!(refreshed.matches("Alpha\t").count(), 1, "{refreshed}");
        assert_eq!(refreshed.matches("Beta\t").count(), 1, "{refreshed}");
        assert!(s.doc.field_ranges(StoryRef::Body).is_empty(), "now the engine's TOC");
        for command in ["references.updateToc", "references.updateFields", "references.toc"] {
            run(&mut s, command, json!({}));
            assert_eq!(text(&s), refreshed, "{command} refreshes the same TOC");
        }
        while s.undo() {}
        assert_eq!(text(&s), before);
        assert_eq!(s.doc.field_ranges(StoryRef::Body).len(), 1, "undo brings the imported range back");
    }
}

/// #2182 (the save and reopen gap, not the flow in the report, which #1197 fixed): after a save
/// and reopen a TOC is a range field opening in its first entry. The context menu offers Update
/// Field wherever the caret is in it, including the field's own marker (offset 0 of the first
/// entry) and the end of the last entry; and repeated update, save and reopen adds no blank line.
#[test]
fn toc_menu_offers_update_field_after_save_and_reopen() {
    use wordcraft_doc::para::InlineObject;
    use wordcraft_doc::props::CharProps;
    // What the canvas context menu checks before offering Update Field.
    let offered = |s: &Session| !crate::cmd::fields::selected(s).is_empty() || crate::cmd::references::caret_in_toc(s);
    let mut d = wordcraft_doc::Document::from_text("Before.\nOld entry\t9\nAlpha\nBeta");
    {
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap();
        let n = p.len();
        p.insert_object(n, InlineObject::FieldEnd, &CharProps::default()).unwrap();
        p.insert_object(0, InlineObject::FieldStart { instr: r#"TOC \o "1-2" \h"#.into(), locked: false, dirty: false }, &CharProps::default())
            .unwrap();
    }
    let mut s = Session::new(d);
    for (i, style) in [(2, "Heading 1"), (3, "Heading 2")] {
        run(&mut s, "caret.set", json!({"pos": Pos::body(i, 0)}));
        run(&mut s, "styles.apply", json!({"style": style}));
    }
    let check = |s: &mut Session, what: &str| {
        let entries: Vec<usize> = (0..s.doc.body.len())
            .filter(|i| {
                s.doc.body.get(*i).and_then(|b| b.as_para()).is_some_and(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC")))
            })
            .collect();
        assert!(!entries.is_empty(), "{what}: no TOC paragraphs: {}", text(s));
        for (k, i) in entries.iter().enumerate() {
            let len = s.doc.body.get(*i).and_then(|b| b.as_para()).map_or(0, |p| p.len());
            // The leftover empty field paragraph before the range (first block) has no menu (minor).
            if k == 0 && len == 0 {
                continue;
            }
            for off in 0..=len {
                s.sel = crate::Selection::caret(Pos::body(*i, off));
                assert!(offered(s), "{what}: no Update Field at block {i} offset {off}: {}", text(s));
            }
        }
    };
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    s = Session::new(wordcraft_docx::read(&bytes).unwrap());
    let mut blocks = None;
    for round in 0..3 {
        run(&mut s, "references.updateToc", json!({}));
        check(&mut s, &format!("after update {round}"));
        // No extra blank line per cycle.
        assert!(blocks.is_none_or(|n| n == s.doc.body.len()), "after update {round}: the document grew: {}", text(&s));
        blocks = Some(s.doc.body.len());
        let bytes = wordcraft_docx::write(&s.doc).unwrap();
        s = Session::new(wordcraft_docx::read(&bytes).unwrap());
        check(&mut s, &format!("after reopen {round}"));
    }
}

/// Word's own TOC layout (#2427): its Table of Contents control around a `TOCHeading`, the entries
/// with the field opening in the first, and the field's end alone in a paragraph after them. Update
/// Table and a save keep one control around the heading and the new entries, and repeating them
/// doesn't grow the document.
#[test]
fn word_toc_control_with_the_field_end_after_the_entries_survives_update_and_save() {
    use std::io::{Read, Write};
    let style = |st: &str, inner: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{st}"/></w:pPr>{inner}</w:p>"#);
    let t = |s: &str| format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#);
    let begin = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#;
    let toc = format!(
        "{}{}{}<w:p><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>",
        style("TOCHeading", &t("Contents")),
        style("TOC1", &format!("{begin}{}", t("Chapter One\t1"))),
        style("TOC2", &t("Section\t1")),
    );
    let gallery = format!(
        r#"<w:sdt><w:sdtPr><w:id w:val="-1234567"/><w:docPartObj><w:docPartGallery w:val="Table of Contents"/><w:docPartUnique/></w:docPartObj></w:sdtPr><w:sdtContent>{toc}</w:sdtContent></w:sdt>"#
    );
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{gallery}{}{}</w:body></w:document>"#,
        style("Heading1", &t("Chapter One")),
        style("Heading2", &t("Section"))
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, value) in [
        ("word/document.xml", xml.as_str()),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
    let mut s = Session::new(wordcraft_docx::read(&zip.finish().unwrap().into_inner()).unwrap());
    let mut blocks = None;
    for round in 0..3 {
        run(&mut s, "references.updateToc", json!({}));
        let bytes = wordcraft_docx::write(&s.doc).unwrap();
        let mut xml = String::new();
        zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap().by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert_eq!(xml.matches("<w:sdt>").count(), 1, "round {round}: {xml}");
        let (open, close) = (xml.find("<w:sdt>").unwrap(), xml.find("</w:sdt>").unwrap());
        assert!(open < xml.find(">Contents<").unwrap(), "round {round}: the heading is in the control: {xml}");
        let entries = xml.find(r#"<w:pStyle w:val="TOC1"/>"#).unwrap();
        assert!(open < entries && xml.rfind(r#"<w:pStyle w:val="TOC2"/>"#).unwrap() < close, "round {round}: {xml}");
        assert!(close < xml.rfind(">Chapter One<").unwrap(), "round {round}: {xml}");
        s = Session::new(wordcraft_docx::read(&bytes).unwrap());
        assert!(blocks.is_none_or(|n| n == s.doc.body.len()), "round {round}: the document grew: {}", text(&s));
        blocks = Some(s.doc.body.len());
    }
}

/// A paragraph style `id` named `name`, based on Normal.
fn style_xml(id: &str, name: &str, ppr: &str, rpr: &str) -> String {
    format!(
        r#"<w:style w:type="paragraph" w:styleId="{id}"><w:name w:val="{name}"/><w:basedOn w:val="Normal"/><w:pPr>{ppr}</w:pPr><w:rPr>{rpr}</w:rPr></w:style>"#
    )
}

/// A DOCX package with `body` and the paragraph `styles` after a default Normal style.
fn docx_with_styles(body: &str, styles: &str) -> Vec<u8> {
    use std::io::Write;
    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
    const RELS: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;
    const TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, value) in [
        ("word/document.xml", format!(r#"<w:document {W}><w:body>{body}</w:body></w:document>"#)),
        (
            "word/styles.xml",
            format!(
                r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>{styles}</w:styles>"#
            ),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(r#"<Relationships {RELS}><Relationship Id="rId1" Type="{TYPE}/styles" Target="styles.xml"/></Relationships>"#),
        ),
        (
            "_rels/.rels",
            format!(r#"<Relationships {RELS}><Relationship Id="main" Type="{TYPE}/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// #3891: Update Table on a TOC Word wrote in Portuguese keeps the formatting of its TOC styles,
/// stored under the ids `Sumrio1`/`Sumrio3` (`toc 1`/`toc 3`): bold Arial at level 1, italic and
/// indented at level 3.
#[test]
fn update_table_keeps_the_formatting_of_localized_toc_styles() {
    let t = |s: &str| format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#);
    let p = |st: &str, inner: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{st}"/></w:pPr>{inner}</w:p>"#);
    let begin = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#;
    let body = [
        p("CabealhodoSumrio", &t("Contents")),
        p("Sumrio1", &format!("{begin}{}", t("Chapter\t1"))),
        p("Sumrio3", &t("Detail\t1")),
        r#"<w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#.to_string(),
        p("Ttulo1", &t("Chapter")),
        p("Ttulo3", &t("Detail")),
    ]
    .concat();
    let styles = [
        style_xml("Ttulo1", "heading 1", r#"<w:outlineLvl w:val="0"/>"#, ""),
        style_xml("Ttulo3", "heading 3", r#"<w:outlineLvl w:val="2"/>"#, ""),
        style_xml("CabealhodoSumrio", "TOC Heading", "", ""),
        style_xml("Sumrio1", "toc 1", "", r#"<w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:b/>"#),
        style_xml("Sumrio3", "toc 3", r#"<w:ind w:left="220"/>"#, "<w:i/>"),
    ]
    .concat();
    let mut s = Session::new(wordcraft_docx::read(&docx_with_styles(&body, &styles)).unwrap());
    run(&mut s, "references.updateToc", json!({}));
    let entries: Vec<(String, bool, bool, f32)> = s
        .doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
        .map(|p| {
            let c = s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(0));
            (c.font, c.bold, c.italic, s.doc.styles.resolve_para(&p.props).indent_left)
        })
        .collect();
    let level3 = entries.get(1).map(|e| e.0.clone()).unwrap_or_default();
    assert_eq!(entries, [("Arial".to_string(), true, false, 0.0), (level3, false, true, 11.0)], "{}", text(&s));
}

/// #3889: Word's TOC in two columns, written by Word in Portuguese. The TOC heading ends a
/// one-column section; the entries, styled `Sumrio1`/`Sumrio2` (`toc 1`/`toc 2`), fill a continuous
/// two-column one, and the field's end and the control's end share a paragraph after them. Update
/// Table keeps the entries' styles, puts the page numbers at the first column's right edge (3888
/// twips, as Word's own entries have them) and adds no blank line above the first column; update,
/// save and reopen keeps it so.
#[test]
fn toc_in_columns_with_localized_styles_keeps_its_layout_through_update_table() {
    let t = |s: &str| format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#);
    let p = |st: &str, inner: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{st}"/></w:pPr>{inner}</w:p>"#);
    let tab = r#"<w:tabs><w:tab w:val="right" w:leader="dot" w:pos="3888"/></w:tabs>"#;
    let entry = |st: &str, inner: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{st}"/>{tab}</w:pPr>{inner}</w:p>"#);
    let sect = |cols: &str, kind: &str| {
        format!(
            r#"<w:sectPr>{kind}<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1701" w:bottom="1417" w:left="1701" w:header="708" w:footer="708" w:gutter="0"/><w:cols {cols} w:space="708"/></w:sectPr>"#
        )
    };
    let continuous = r#"<w:type w:val="continuous"/>"#;
    let begin = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-2" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#;
    let toc = [
        p("CabealhodoSumrio", &t("Contents")),
        format!("<w:p><w:pPr>{}</w:pPr></w:p>", sect("", "")),
        entry("Sumrio1", &format!("{begin}{}", t("Chapter One\t1"))),
        entry("Sumrio2", &t("Section\t1")),
        r#"<w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#.to_string(),
    ]
    .concat();
    let body = [
        format!(
            r#"<w:sdt><w:sdtPr><w:id w:val="-1360965913"/><w:docPartObj><w:docPartGallery w:val="Table of Contents"/><w:docPartUnique/></w:docPartObj></w:sdtPr><w:sdtContent>{toc}</w:sdtContent></w:sdt>"#
        ),
        format!(r#"<w:p><w:pPr>{}</w:pPr></w:p>"#, sect(r#"w:num="2""#, continuous)),
        p("Ttulo1", &t("Chapter One")),
        p("Ttulo2", &t("Section")),
        sect("", ""),
    ]
    .concat();
    let styles = [
        style_xml("Ttulo1", "heading 1", r#"<w:outlineLvl w:val="0"/>"#, ""),
        style_xml("Ttulo2", "heading 2", r#"<w:outlineLvl w:val="1"/>"#, ""),
        style_xml("CabealhodoSumrio", "TOC Heading", "", ""),
        style_xml("Sumrio1", "toc 1", r#"<w:spacing w:after="100"/>"#, ""),
        style_xml("Sumrio2", "toc 2", r#"<w:spacing w:after="100"/><w:ind w:left="220"/>"#, ""),
    ]
    .concat();
    let mut s = Session::new(wordcraft_docx::read(&docx_with_styles(&body, &styles)).unwrap());
    let mut blocks = None;
    for round in 0..3 {
        run(&mut s, "references.updateToc", json!({}));
        let entries: Vec<(usize, &wordcraft_doc::Paragraph)> = s
            .doc
            .body
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.as_para().map(|p| (i, p)))
            .filter(|(_, p)| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
            .collect();
        // The chapters' section starts a new page.
        let found: Vec<(Option<&str>, String)> = entries.iter().map(|(_, p)| (p.props.style.as_deref(), p.plain_text())).collect();
        assert_eq!(found, [(Some("TOC1"), "Chapter One\t2".to_string()), (Some("TOC2"), "Section\t2".to_string())], "round {round}: {}", text(&s));
        assert_eq!(s.doc.styles.get("TOC2").and_then(|st| st.para.indent_left), Some(11.0), "round {round}: the document's own TOC 2");
        for (_, e) in &entries {
            let right = e.props.tabs.as_ref().and_then(|t| t.iter().find(|t| t.align == wordcraft_doc::props::TabAlign::Right)).unwrap();
            assert!((right.pos - 194.4).abs() < 0.1, "round {round}: page numbers at the first column's edge, not the page's: {}", right.pos);
        }
        // The paragraph above the first entry ends the one-column section: no blank line tops the
        // first column.
        let above = entries[0].0 - 1;
        assert!(s.doc.body[above].as_para().unwrap().section.is_some(), "round {round}: a blank line above the entries: {}", text(&s));
        let bytes = wordcraft_docx::write(&s.doc).unwrap();
        s = Session::new(wordcraft_docx::read(&bytes).unwrap());
        assert!(blocks.is_none_or(|n| n == s.doc.body.len()), "round {round}: the document grew: {}", text(&s));
        blocks = Some(s.doc.body.len());
    }
    // The field's paragraph ends the one-column section: replacing or removing the TOC keeps it.
    let columns = |s: &Session| s.doc.sections().iter().map(|(_, x)| x.columns.count).collect::<Vec<_>>();
    assert_eq!(columns(&s), [1, 2, 1]);
    // The heading goes with the table: one "Contents" after Replace, none after Remove.
    let titles = |s: &Session| s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.plain_text() == "Contents").count();
    run(&mut s, "references.toc", json!({"levels": 2}));
    assert_eq!(columns(&s), [1, 2, 1], "{}", text(&s));
    assert_eq!(titles(&s), 1, "{}", text(&s));
    run(&mut s, "references.removeToc", json!({}));
    assert_eq!(columns(&s), [1, 2, 1], "{}", text(&s));
    assert_eq!(titles(&s), 0, "{}", text(&s));
}

/// #2182: with two TOCs only the one Update Table would refresh is offered Update Field; the other
/// range TOC doesn't claim the menu, so it never refreshes a different table.
#[test]
fn toc_menu_targets_the_toc_update_table_refreshes() {
    use wordcraft_doc::para::InlineObject;
    use wordcraft_doc::props::CharProps;
    let mut d = wordcraft_doc::Document::from_text("One\nTwo\nHead\nFirst\nSecond\nTail");
    for (para, instr) in [(0, r#"TOC \o "1-2""#), (3, r#"TOC \o "1-3""#)] {
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(para)).unwrap();
        let n = p.len();
        p.insert_object(n, InlineObject::FieldEnd, &CharProps::default()).unwrap();
        p.insert_object(0, InlineObject::FieldStart { instr: instr.into(), locked: false, dirty: false }, &CharProps::default()).unwrap();
    }
    let mut s = Session::new(d);
    let at = |s: &mut Session, block: usize| {
        s.sel = crate::Selection::caret(Pos::body(block, 1));
        crate::cmd::references::caret_in_toc(s)
    };
    assert!(at(&mut s, 0), "the first TOC is the one Update Table converts");
    assert!(!at(&mut s, 3), "the second range TOC is not offered the menu item");
}

/// Updating a TOC field that had a non-empty result (placeholder from an external generator, #878)
/// clears the old result so the exported DOCX nests entries inside the field rather than leaving
/// them after the field's end marker.
#[test]
fn update_toc_clears_stale_field_result_for_docx_span() {
    use wordcraft_doc::para::InlineObject;
    use wordcraft_doc::props::CharProps;
    let mut d = wordcraft_doc::Document::from_text("\nAlpha\nBeta");
    {
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        p.insert_object(
            0,
            InlineObject::Field {
                instr: r#"TOC \o "1-3" \h \z \u"#.into(),
                result: "Placeholder: update fields (F9).".into(),
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            },
            &CharProps::default(),
        )
        .unwrap();
    }
    let mut s = Session::new(d);
    for (i, style) in [(1, "Heading 1"), (2, "Heading 2")] {
        run(&mut s, "caret.set", json!({"pos": Pos::body(i, 0)}));
        run(&mut s, "styles.apply", json!({"style": style}));
    }
    // Before update: the field result is present in plain text.
    assert!(text(&s).contains("Placeholder: update fields (F9)."));

    run(&mut s, "references.updateToc", json!({}));

    // After update: the placeholder is gone, replaced by regenerated entries.
    let refreshed = text(&s);
    assert!(!refreshed.contains("Placeholder"), "{refreshed}");
    assert!(refreshed.contains("Alpha\t"), "{refreshed}");
    assert!(refreshed.contains("Beta\t"), "{refreshed}");

    // DOCX round-trip: entries must be enclosed within the TOC field rather than outside it.
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let xml = {
        let cursor = std::io::Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        let mut file = archive.by_name("word/document.xml").unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut file, &mut s).unwrap();
        s
    };
    let b = xml.find(r#"w:fldCharType="begin""#).expect("TOC begin marker present");
    // The TOC's end is the last one: the entries' page numbers are `PAGEREF` fields inside it.
    let e = xml.rfind(r#"w:fldCharType="end""#).expect("TOC end marker present");
    assert!(b < e, "begin marker precedes end marker");
    let inside_field = &xml[b..e];
    let after_field = &xml[e..];
    assert!(inside_field.contains("Alpha"), "Alpha entry is inside TOC field");
    assert!(inside_field.contains("Beta"), "Beta entry is inside TOC field");
    assert!(!inside_field.contains("Placeholder"), "Placeholder is not inside TOC field");
    assert!(!after_field.contains(r#"w:val="TOC1""#), "no TOC1 styles after the field end marker");
}

/// #877: computing a field's result clears `w:dirty` — the result is no longer stale, like
/// Word's F9. A field WordCraft can't compute keeps the flag so Word refreshes it on open.
#[test]
fn update_fields_clears_dirty() {
    let mut s = s();
    s.doc.core.title = "Report".into();
    let mut p = wordcraft_doc::Paragraph::new();
    for instr in ["TITLE", "NUMWORDS", "NOTAFIELD \\x"] {
        let end = p.len();
        p.insert_object(
            end,
            wordcraft_doc::InlineObject::Field {
                instr: instr.into(),
                result: String::new(),
                locked: false,
                dirty: true,
                code: false,
                form: String::new(),
            },
            &wordcraft_doc::props::CharProps::default(),
        )
        .unwrap();
    }
    s.doc.body = vec![wordcraft_doc::para_block(p)];
    run(&mut s, "references.updateFields", json!({}));
    let objs = &s.doc.body[0].as_para().unwrap().objects;
    let dirty: Vec<bool> = objs.iter().map(|o| matches!(o, wordcraft_doc::InlineObject::Field { dirty: true, .. })).collect();
    assert_eq!(dirty, vec![false, false, true], "computed fields clear the flag, unknown ones keep it: {:?}", objs);
    assert!(matches!(objs.first(), Some(wordcraft_doc::InlineObject::Field { result, .. }) if result == "Report"), "{objs:?}");
}

/// #877/#878: an imported TOC field whose begin marker carries `w:dirty` (a generator wrote a
/// placeholder result and asked Word to refresh it on open) keeps the flag through saves until
/// WordCraft computes the entries: Update Table replaces the placeholder in place (#878) and
/// clears the flag, like Word's F9. Undo brings the dirty imported field back.
#[test]
fn imported_dirty_toc_updates_in_place_and_clears() {
    let mut d = wordcraft_doc::Document::from_text("Placeholder: update fields (F9).\nAlpha\nBody text.\nBeta\nGamma");
    {
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let n = p.len();
        p.insert_object(n, wordcraft_doc::InlineObject::FieldEnd, &wordcraft_doc::props::CharProps::default()).unwrap();
        p.insert_object(
            0,
            wordcraft_doc::InlineObject::FieldStart { instr: r#"TOC \o "1-3" \h \z \u"#.into(), locked: false, dirty: true },
            &wordcraft_doc::props::CharProps::default(),
        )
        .unwrap();
    }
    let mut s = Session::new(d);
    for (i, style) in [(1, "Heading 1"), (3, "Heading 2"), (4, "Heading 1")] {
        run(&mut s, "caret.set", json!({"pos": Pos::body(i, 0)}));
        run(&mut s, "styles.apply", json!({"style": style}));
    }
    // As an imported file: the flag survives the round trip, and a plain save keeps it.
    s = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap());
    let toc_dirty = |d: &wordcraft_doc::Document| {
        d.body.iter().find_map(|b| {
            b.as_para()?.objects.iter().find_map(|o| match o {
                wordcraft_doc::InlineObject::FieldStart { dirty, .. } => Some(*dirty),
                _ => None,
            })
        })
    };
    assert_eq!(toc_dirty(&s.doc), Some(true), "the imported TOC reads back dirty");
    let saved = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(toc_dirty(&saved), Some(true), "a plain save keeps the flag for Word to refresh");
    // Update Table replaces the placeholder in place and clears the flag.
    run(&mut s, "references.updateToc", json!({}));
    let t = text(&s);
    assert!(!t.contains("Placeholder"), "{t}");
    for entry in ["Alpha\t", "Beta\t", "Gamma\t"] {
        assert_eq!(t.matches(entry).count(), 1, "{t}");
    }
    assert!(s.doc.field_ranges(StoryRef::Body).is_empty(), "now the engine's TOC");
    let updated = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(toc_dirty(&updated), Some(false), "the computed TOC no longer asks Word to refresh it");
    // Undo brings the dirty imported field back.
    while s.undo() {}
    assert_eq!(toc_dirty(&s.doc), Some(true));
    assert!(text(&s).contains("Placeholder"));
}

/// A TOC as Word and other writers lay it out (a TOC heading, then the field in paragraphs of its
/// own), styles with a number tab stop and a hanging indent, and a wider last section with the
/// caret in it (Select All, Update Field): no blank line between the heading and the entries,
/// and every page number at the TOC section's right margin, after its leader: short entries,
/// entries that wrap, and level-one entries whose style has no stops of its own.
#[test]
fn toc_page_numbers_reach_the_right_margin() {
    use wordcraft_doc::para::InlineObject;
    use wordcraft_doc::props::{CharProps, TabAlign, TabLeader, TabStop};
    let long = "A deliberately long section title that does not fit on one line of the table of contents and wraps onto a second line";
    let mut d = wordcraft_doc::Document::from_text(&format!("Contents\n\nNo entries.\n\nShort\n{long}\nTop\nAppendix"));
    {
        let toc2 = d.styles.get_mut("TOC2").unwrap();
        toc2.para.tabs = Some(vec![
            TabStop { pos: 77.0, align: TabAlign::Left, leader: TabLeader::None },
            TabStop { pos: 467.2, align: TabAlign::Right, leader: TabLeader::Dot },
        ]);
        toc2.para.indent_left = Some(36.9);
        toc2.para.indent_first = Some(-22.7);
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap();
        p.insert_object(0, InlineObject::FieldStart { instr: r#"TOC \o "1-3" \h \z \u"#.into(), locked: false, dirty: false }, &CharProps::default())
            .unwrap();
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(3)).unwrap();
        p.insert_object(0, InlineObject::FieldEnd, &CharProps::default()).unwrap();
    }
    let mut s = Session::new(d);
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
    run(&mut s, "styles.apply", json!({"style": "TOC Heading"}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(1, 0)}));
    run(&mut s, "styles.apply", json!({"style": "TOC 1"}));
    for (i, style) in [(4, "Heading 2"), (5, "Heading 2"), (6, "Heading 1"), (7, "Heading 1")] {
        run(&mut s, "caret.set", json!({"pos": Pos::body(i, 0)}));
        run(&mut s, "styles.apply", json!({"style": style}));
    }
    run(&mut s, "caret.set", json!({"pos": Pos::body(7, 0)}));
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "layout.orientation", json!({"value": "landscape"}));
    // Through DOCX, as an imported file is.
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    s = Session::new(wordcraft_docx::read(&bytes).unwrap());
    run(&mut s, "select.all", json!({}));
    run(&mut s, "references.updateFields", json!({}));

    let body: Vec<(String, String)> =
        s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| (p.props.style.clone().unwrap_or_default(), p.plain_text())).collect();
    assert_eq!(body[0], ("TOCHeading".into(), "Contents".into()), "{body:?}");
    assert!(body[1].0.starts_with("TOC") && !body[1].1.is_empty(), "the first entry right under the heading: {body:?}");
    let toc_width = s.doc.sections()[0].1.text_width();
    let margin = s.doc.sections()[0].1.margin_left;
    let l = s.layout();
    for (i, b) in s.doc.body.iter().enumerate() {
        let Some(p) = b.as_para() else { continue };
        if !p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading") {
            continue;
        }
        let page = p.text.rfind('\t').unwrap() + 1;
        let end = l.caret(&Pos::body(i, p.len())).unwrap();
        let number = l.caret(&Pos::body(i, page)).unwrap();
        assert!(
            (end.x - (margin + toc_width)).abs() < 2.0,
            "{:?}: page number ends at {} (right margin {})",
            p.plain_text(),
            end.x,
            margin + toc_width
        );
        assert!(end.page == number.page && number.x > margin + toc_width - 30.0, "{:?}: page number at {}", p.plain_text(), number.x);
    }
    // Updating again keeps the same table.
    let before = text(&s);
    run(&mut s, "references.updateToc", json!({}));
    assert_eq!(text(&s), before);
}

#[test]
fn toc_page_numbers_follow_headings() {
    let mut s = s();
    run(&mut s, "references.toc", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    for name in ["Alpha", "Bravo", "Charlie"] {
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.insert", json!({"text": name}));
        run(&mut s, "para.heading1", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "references.updateToc", json!({}));
    let t = text(&s);
    for (name, page) in [("Alpha", 2), ("Bravo", 3), ("Charlie", 4)] {
        assert!(t.contains(&format!("{name}\t{page}")), "{name} should be on page {page}: {t}");
    }
}

/// A hyperlinked TOC's page numbers are `PAGEREF` fields to the headings' bookmarks, as Word
/// writes them, so updating them in Word follows the headings; they survive a save and reopen.
#[test]
fn toc_page_numbers_are_pageref_fields() {
    let mut s = s();
    run(&mut s, "references.toc", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    for name in ["Alpha", "Bravo"] {
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.insert", json!({"text": name}));
        run(&mut s, "para.heading1", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "references.updateToc", json!({}));
    let fields: Vec<(String, String)> = s
        .doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref() == Some("TOC1"))
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            wordcraft_doc::para::InlineObject::Field { instr, result, .. } => Some((instr.clone(), result.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(fields.len(), 2, "{fields:?}");
    for ((instr, result), page) in fields.iter().zip(["2", "3"]) {
        let mark = instr.strip_prefix("PAGEREF ").and_then(|r| r.strip_suffix(" \\h")).unwrap_or_else(|| panic!("{instr}"));
        assert!(mark.starts_with("_Toc") && s.doc.bookmarks().iter().any(|(n, _)| n == mark), "{instr}");
        assert_eq!(result, page);
    }
    let expected = text(&s);
    assert!(expected.contains("Alpha\t2") && expected.contains("Bravo\t3"), "{expected}");
    let bytes = crate::io::save_bytes("toc.docx", &s.doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert_eq!(xml.matches("PAGEREF _Toc").count(), 2, "{xml}");
    let mut s = Session::new(crate::io::open_bytes("toc.docx", &bytes).unwrap());
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(text(&s), expected);
}

/// Headings numbered through a list carry their numbers, and the level's separator, into the
/// TOC entries.
#[test]
fn toc_entries_carry_heading_numbers() {
    let mut s = s();
    run(&mut s, "references.toc", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    for name in ["Alpha", "Bravo"] {
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.insert", json!({"text": name}));
        run(&mut s, "para.heading1", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    let num = s.doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    for b in s.doc.body.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b)
            && p.props.style.as_deref() == Some("Heading1")
        {
            p.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
        }
    }
    run(&mut s, "references.updateToc", json!({}));
    let t = text(&s);
    assert!(t.contains("1.\tAlpha\t2") && t.contains("2.\tBravo\t3"), "{t}");
}

/// A heading above the TOC keeps its index when the entries go in below it, so its page is read
/// where it is, not `entries` blocks further down.
#[test]
fn toc_page_number_for_heading_above_toc() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Preface"}));
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.pageBreak", json!({}));
    run(&mut s, "references.toc", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    for name in ["Alpha", "Bravo"] {
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.insert", json!({"text": name}));
        run(&mut s, "para.heading1", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "references.updateToc", json!({}));
    let t = text(&s);
    for (name, page) in [("Preface", 1), ("Alpha", 3), ("Bravo", 4)] {
        assert!(t.contains(&format!("{name}\t{page}")), "{name} should be on page {page}: {t:?}");
    }
}

#[test]
fn failed_command_leaves_document_unchanged() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "keep"}));
    let before = text(&s);
    assert!(s.run("format.size", &json!({"size": -3})).is_err());
    assert!(s.run("no.such", &json!({})).is_err());
    assert!(s.run("text.insert", &json!({})).is_err());
    assert_eq!(text(&s), before);
}

/// Review of #3692: the line cap on typed paragraphs is the engine's, so a control-channel or
/// CLI `text.insert` is held to it as the MCP tool is. At the cap every line goes in as one undo
/// step; one line more is refused before anything is typed.
#[test]
fn typed_paragraphs_are_capped_for_every_caller() {
    use crate::CmdError;
    use crate::cmd::text::MAX_TYPED_PARAGRAPHS;
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "keep"}));
    let (before, depth) = (body_texts(&s), s.undo_depth());
    // "x\n" repeated ends in an empty line: one over the cap. A lone "\r" counts as a break too.
    for over in ["x\n".repeat(MAX_TYPED_PARAGRAPHS), format!("{}x", "x\r\n".repeat(MAX_TYPED_PARAGRAPHS - 1)) + "\rx"] {
        match s.run("text.insert", &json!({"text": over, "paragraphs": true})) {
            Err(CmdError::Params(m)) => assert!(m.contains("10001 lines") && m.contains("at most 10000"), "{m}"),
            r => panic!("not refused: {r:?}"),
        }
        assert_eq!(body_texts(&s), before, "nothing typed");
        assert_eq!(s.undo_depth(), depth, "no undo step");
    }
    let at_cap = vec!["y"; MAX_TYPED_PARAGRAPHS].join("\r\n");
    run(&mut s, "text.insert", json!({"text": at_cap, "paragraphs": true}));
    let paras = body_texts(&s);
    assert_eq!(paras.len(), MAX_TYPED_PARAGRAPHS);
    // Each break is Enter's own, AutoCorrect included: the first sentence gets its capital.
    assert_eq!((paras.first().map(String::as_str), paras.last().map(String::as_str)), (Some("Keepy"), Some("y")));
    assert!(s.undo(), "undo");
    assert_eq!(body_texts(&s).len(), 1, "one undo step takes every line back out");
    assert!(!body_texts(&s).iter().any(|p| p.contains('y')));
}

#[test]
fn hostile_params_never_panic() {
    let reg = cmd::registry();
    let junk = [
        json!(null),
        json!({}),
        json!({"text": 5, "size": "x", "path": [], "pos": {"block": 9999, "off": 99999}}),
        json!({"value": -1e308, "rows": 1e9}),
        json!({"page": 0, "x": 10, "y": 100, "extend": "x", "toX": null, "toY": 1e30, "reach": "r"}),
        json!({"page": 0, "x": 1e30, "y": -1e30, "extend": 1}),
        json!({"width": 1e-9, "height": 1e30, "top": -1e9, "left": 1e9}),
        json!({"section": {"pageW": -1e308, "pageH": 0, "marginTop": 1e30, "header": -5, "columns": {"count": 1000000, "space": -1e300, "widths": [[0, 0]]}}}),
    ];
    for spec in reg.all() {
        // These reach outside the session (files). Read Aloud (`review.readAloud`, `readAloud.*`)
        // is fuzzed too: under `cfg(test)` its backend is the silent `Hold`, asserted below.
        if spec.id.starts_with("file.") || spec.id == "insert.picture" || spec.id == "insert.textFromFile" {
            continue;
        }
        for j in &junk {
            let mut s = Session::new(crate::sample::sample_document());
            assert_ne!(s.read_aloud.backend, crate::speech::Backend::System, "tests must never start real speech");
            let _ = s.run(spec.id, j);
            s.clamp_selection();
            let _ = s.layout();
        }
    }
}

#[test]
fn inspect_reports_structure() {
    let mut s = Session::new(crate::sample::sample_document());
    let r = run(&mut s, "document.inspect", json!({}));
    assert!(r["pages"].as_u64().unwrap() >= 1);
    assert!(r["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "table"));
    let f = run(&mut s, "format.state", json!({}));
    assert_eq!(f["styleName"], "Title");
}

#[test]
fn caret_navigation() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "first line\nsecond line"}));
    run(&mut s, "caret.down", json!({}));
    assert_eq!(s.sel.focus.path.last(), 1);
    run(&mut s, "caret.end", json!({}));
    assert_eq!(s.sel.focus.off, 11);
    run(&mut s, "caret.up", json!({"extend": true}));
    assert_eq!(s.sel.focus.path.last(), 0);
    assert!(!s.sel.is_collapsed());
    run(&mut s, "caret.wordLeft", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    assert_eq!(s.sel.focus, Pos::body(1, 11));
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, 10);
}

#[test]
fn text_box_insert_puts_the_caret_inside() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Before "}));
    let id = run(&mut s, "insert.textBox", json!({}))["story"].as_u64().unwrap() as u32;
    let boxed = StoryRef::Part(id);
    assert_eq!(s.sel.focus.story, boxed);
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    assert_eq!(s.doc.plain_text(boxed), "Hello");
    assert!(!text(&s).contains("Hello"), "{}", text(&s));
    // Leaving the box lands just after it in the body.
    let anchor = s.doc.text_box_anchor(id).unwrap();
    assert_eq!(anchor, Pos::body(0, "Before \u{FFFC}".len()));
    run(&mut s, "caret.set", json!({"pos": anchor}));
    run(&mut s, "text.insert", json!({"text": " after"}));
    assert_eq!(s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).map(|p| p.text.as_str()), Some("Before \u{FFFC} after"));
    // Undo goes back through the typing to before the box, in the body.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.sel.focus.story, StoryRef::Body);
    assert_eq!(text(&s), "Before ");
    // Given text: the caret ends after it.
    let id = run(&mut s, "insert.textBox", json!({"text": "Note"}))["story"].as_u64().unwrap() as u32;
    assert_eq!(s.sel.focus, Pos { story: StoryRef::Part(id), path: wordcraft_doc::Path::top(0), off: 4 });
}

#[test]
fn font_commands_on_a_selected_text_box_format_all_its_text() {
    // Word: with a text box selected as an object (its frame, not text in it), the Font group
    // formats all the box's text. Select All doesn't reach a floating box's text, so this is how
    // its font is changed (and the only way when the box sits behind text, where a click goes to
    // the text over it).
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Before "}));
    let id = run(&mut s, "insert.textBox", json!({"text": "Box words"}))["story"].as_u64().unwrap() as u32;
    let boxed = StoryRef::Part(id);
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "second"}));
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, "Before ".len()), "focus": Pos::body(0, "Before \u{FFFC}".len())}));
    run(&mut s, "format.font", json!({"name": "TH Sarabun New"}));
    run(&mut s, "format.size", json!({"size": 18}));
    run(&mut s, "format.bold", json!({}));
    let paths = s.doc.para_paths(boxed);
    assert_eq!(paths.len(), 2);
    for path in &paths {
        for (_, c) in s.doc.para(boxed, path).unwrap().run_ranges() {
            assert_eq!(c.font.as_deref(), Some("TH Sarabun New"));
            assert_eq!(c.font_cs.as_deref(), Some("TH Sarabun New"));
            assert_eq!(c.size, Some(18.0));
            assert_eq!(c.bold, Some(true));
        }
    }
    // The body text around the box is left as it was, and the box stays selected.
    let body = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(body.run_ranges().all(|(_, c)| c.font.is_none() && c.bold.is_none()));
    assert!(crate::cmd::objects::object_selection(&s).is_some());
    // Bold reads the box's text, so pressing it again takes it off.
    run(&mut s, "format.bold", json!({}));
    assert!(s.doc.para(boxed, &paths[0]).unwrap().run_ranges().all(|(_, c)| c.bold == Some(false)));
    // One undo step per command.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.para(boxed, &paths[0]).unwrap().run_ranges().all(|(_, c)| c.bold == Some(true)));
}

#[test]
fn case_and_clearing_commands_on_a_selected_text_box_work_on_its_text() {
    // Word, with a text box selected as an object: Change Case, Clear All Formatting and Reset
    // Character Formatting work on the box's text and leave the body text around it alone.
    let boxed = |s: &mut Session| {
        run(s, "text.insert", json!({"text": "Before "}));
        run(s, "format.bold", json!({"value": true}));
        run(s, "text.insert", json!({"text": "body"}));
        run(s, "caret.set", json!({"pos": Pos::body(0, "Before ".len())}));
        let id = run(s, "insert.textBox", json!({"text": "box text"}))["story"].as_u64().unwrap() as u32;
        run(s, "select.all", json!({}));
        run(s, "format.italic", json!({"value": true}));
        run(s, "select.range", json!({"anchor": Pos::body(0, "Before ".len()), "focus": Pos::body(0, "Before \u{FFFC}".len())}));
        StoryRef::Part(id)
    };
    let body_bold = |s: &Session| s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().run_ranges().any(|(_, c)| c.bold == Some(true));

    let mut cased = s();
    let id = boxed(&mut cased);
    run(&mut cased, "format.changeCase", json!({"mode": "upper"}));
    assert_eq!(cased.doc.plain_text(id), "BOX TEXT");
    assert_eq!(cased.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).map(|p| p.text.as_str()), Some("Before \u{FFFC}body"));
    assert!(crate::cmd::objects::object_selection(&cased).is_some(), "the box stays selected");

    for cmd in ["format.clear", "format.resetChar"] {
        let mut cleared = s();
        let id = boxed(&mut cleared);
        run(&mut cleared, cmd, json!({}));
        let p = cleared.doc.para(id, &wordcraft_doc::Path::top(0)).unwrap();
        assert!(p.run_ranges().all(|(_, c)| c.italic.is_none()), "{cmd} clears the box's text");
        assert!(body_bold(&cleared), "{cmd} leaves the body text's formatting");
        assert!(crate::cmd::objects::object_selection(&cleared).is_some(), "the box stays selected");
    }

    // An empty box: Bold reads (and sets) its paragraph mark, not the anchor's run around it.
    let mut empty = s();
    run(&mut empty, "format.bold", json!({"value": true}));
    run(&mut empty, "text.insert", json!({"text": "Bold body "}));
    let id = run(&mut empty, "insert.textBox", json!({}))["story"].as_u64().unwrap() as u32;
    let anchor = empty.doc.text_box_anchor(id).unwrap();
    run(&mut empty, "select.range", json!({"anchor": Pos { off: anchor.off - "\u{FFFC}".len(), ..anchor.clone() }, "focus": anchor}));
    run(&mut empty, "format.bold", json!({}));
    let mark = &empty.doc.para(StoryRef::Part(id), &wordcraft_doc::Path::top(0)).unwrap().mark;
    assert_eq!(mark.bold, Some(true), "the empty box wasn't bold, so Bold turns it on");
}

#[test]
fn a_selected_text_box_shows_its_own_paragraph_style() {
    // Word, with a text box selected as an object, shows the style of the box's text, not of the
    // body paragraph the box is anchored in, as it does for its font.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Body "}));
    let id = run(&mut s, "insert.textBox", json!({"text": "Title"}))["story"].as_u64().unwrap() as u32;
    run(&mut s, "para.style", json!({"style": "Heading 1"}));
    let inside = run(&mut s, "format.state", json!({}));
    let anchor = s.doc.text_box_anchor(id).unwrap();
    run(&mut s, "select.range", json!({"anchor": Pos { off: anchor.off - "\u{FFFC}".len(), ..anchor.clone() }, "focus": anchor}));
    let selected = run(&mut s, "format.state", json!({}));
    assert_ne!(inside["style"], json!("Normal"), "the box's paragraph has a style of its own: {inside}");
    assert_eq!((&selected["style"], &selected["styleName"]), (&inside["style"], &inside["styleName"]));
}

#[test]
fn commands_after_the_largest_ids_take_free_ones() {
    // A file can carry any ids, 0 and u32::MAX included: comments, notes, boxes and lists the
    // commands add take free ids, without overflowing or replacing what has the id they wrap to.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Some words here"}));
    for id in [0, u32::MAX] {
        let blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::new())];
        s.doc.parts.insert(id, wordcraft_doc::Part { kind: wordcraft_doc::PartKind::Comment, blocks, ..Default::default() });
        s.doc.comments.insert(id, wordcraft_doc::Comment { part: id, ..Default::default() });
    }
    s.doc.numbering.add_list(wordcraft_doc::ListKind::Bullet);
    s.doc.numbering.abstracts[0].id = u32::MAX;
    s.doc.numbering.nums[0].id = u32::MAX;
    s.doc.numbering.nums[0].abstract_id = u32::MAX;
    let before = s.doc.clone();
    assert_eq!(run(&mut s, "review.newComment", json!({"text": "new"})), json!({"id": 1, "story": 1}));
    assert_eq!(run(&mut s, "review.reply", json!({"id": 0, "text": "re"})), json!({"id": 2}));
    run(&mut s, "para.numbering", json!({}));
    assert_eq!(run(&mut s, "list.define", json!({"levels": []}))["num"], 2);
    assert_eq!(run(&mut s, "references.footnote", json!({"text": "note"}))["id"], 3);
    for id in ["insert.wordArt", "insert.textBox"] {
        run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
        run(&mut s, id, json!({"text": "Box"}));
    }
    assert_eq!((s.doc.parts.len(), s.doc.comments.len()), (before.parts.len() + 5, 4));
    assert!(before.parts.iter().all(|(k, p)| s.doc.parts.get(k) == Some(p)));
    assert!(before.comments.iter().all(|(k, c)| s.doc.comments.get(k) == Some(c)));
    let n = &s.doc.numbering;
    assert!(before.numbering.nums.iter().all(|x| n.nums.contains(x)) && before.numbering.abstracts.iter().all(|a| n.abstracts.contains(a)));
    let unique = |ids: Vec<u32>| ids.iter().collect::<std::collections::HashSet<_>>().len() == ids.len();
    assert!(unique(n.nums.iter().map(|x| x.id).collect()) && unique(n.abstracts.iter().map(|a| a.id).collect()), "{n:?}");
    assert_eq!(n.nums.len(), 3);
}

#[test]
fn text_box_in_a_table_cell_shows_its_text() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    assert!(s.sel.focus.path.0.len() > 1);
    let id = run(&mut s, "insert.textBox", json!({}))["story"].as_u64().unwrap() as u32;
    // The caret goes into the box, and typing shows there (it has a caret position).
    assert_eq!(s.sel.focus.story, StoryRef::Part(id));
    run(&mut s, "text.insert", json!({"text": "In a cell"}));
    let caret = s.layout().caret(&s.sel.focus).expect("laid out");
    let cell = s.layout().find_object(0, |o| o.text_box == Some(id)).unwrap();
    assert!(cell.rect.contains(wordcraft_geom::Point::new(caret.x, caret.top)), "{caret:?} in {:?}", cell.rect);
    // No text box inside a text box.
    assert!(s.run("insert.textBox", &json!({})).is_err());
}

#[test]
fn copy_paste_text_box_makes_an_independent_copy() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "A"}));
    let id = run(&mut s, "insert.textBox", json!({"text": "Original"}))["story"].as_u64().unwrap() as u32;
    // Select "A" and the box in the body, copy, paste at the end.
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 1 + 3)}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 4)}));
    let parts = s.doc.parts.len();
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.doc.parts.len(), parts + 1);
    let stories: Vec<u32> = s
        .doc
        .para(StoryRef::Body, &wordcraft_doc::Path::top(0))
        .unwrap()
        .objects
        .iter()
        .filter_map(|o| if let wordcraft_doc::InlineObject::Shape { story, .. } = o { *story } else { None })
        .collect();
    assert_eq!(stories.len(), 2);
    assert_eq!(stories[0], id);
    let copy = stories[1];
    assert_ne!(copy, id);
    // Typing in the copy leaves the original alone.
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Part(copy), path: wordcraft_doc::Path::top(0), off: 0 }}));
    run(&mut s, "text.insert", json!({"text": "Copy of "}));
    assert_eq!(s.doc.plain_text(StoryRef::Part(copy)), "Copy of Original");
    assert_eq!(s.doc.plain_text(StoryRef::Part(id)), "Original");
    // Undo the typing and the paste: the copy's story goes too.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.parts.len(), parts);
}

#[test]
fn word_count_includes_text_boxes_by_default() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "one two "}));
    run(&mut s, "insert.textBox", json!({"text": "three four five"}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
    assert_eq!(s.word_count(), 5);
    let v = run(&mut s, "review.wordCount", json!({}));
    assert_eq!((v["words"].as_u64(), v["includeTextBoxes"].as_bool()), (Some(5), Some(true)));
    assert_eq!(v["paragraphs"].as_u64(), Some(2));
    // Turned off: body only, and it sticks.
    let v = run(&mut s, "review.wordCount", json!({"includeTextBoxes": false}));
    assert_eq!(v["words"].as_u64(), Some(2));
    assert_eq!(s.word_count(), 2);
    assert_eq!(run(&mut s, "review.wordCount", json!({}))["words"].as_u64(), Some(2));
    // Deleting the box drops its words (its story stays behind, unused).
    run(&mut s, "review.wordCount", json!({"includeTextBoxes": true}));
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 8), "focus": Pos::body(0, 8 + 3)}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(s.word_count(), 2);
}

#[test]
fn word_count_includes_numbered_list_labels_in_words_but_not_characters() {
    let mut d = wordcraft_doc::Document::new();
    let num_id = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    let mut p = wordcraft_doc::Paragraph::with_text("Apple", wordcraft_doc::CharProps::default());
    p.props.numbering = Some(wordcraft_doc::props::NumRef { num: num_id, level: 0 });
    d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Para(p))];
    let mut s = Session::new(d);

    let v = run(&mut s, "review.wordCount", json!({}));
    // Generated label "1." + "Apple" = 2 words.
    assert_eq!(v["words"].as_u64(), Some(2));
    assert_eq!(s.word_count(), 2);
    // Generated list label "1." is formatting metadata and not run characters,
    // so Word excludes it from character counts: "Apple" has 5 characters.
    assert_eq!(v["characters"].as_u64(), Some(5));
    assert_eq!(v["charactersWithSpaces"].as_u64(), Some(5));
}

#[test]
fn word_count_excludes_carriage_returns_from_character_counts() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello\r\nWorld"}));
    run(&mut s, "select.all", json!({}));
    let v = run(&mut s, "review.wordCount", json!({}));
    assert_eq!(v["words"].as_u64(), Some(2));
    assert_eq!(v["characters"].as_u64(), Some(10));
    assert_eq!(v["charactersWithSpaces"].as_u64(), Some(10));
}

/// The laid-out area of the only text box.
fn box_area(s: &mut Session) -> wordcraft_layout::hit::ObjectHit {
    let l = s.layout();
    l.find_object(0, |o| o.text_box.is_some()).unwrap()
}

fn select_box(s: &mut Session) {
    let o = box_area(s);
    let end = Pos { off: o.off + 3, ..o.pos() };
    run(s, "select.range", json!({"anchor": o.pos(), "focus": end}));
}

#[test]
fn arrange_bounds_resizes_and_moves_objects() {
    use wordcraft_doc::para::{Anchor, Wrap};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Some text "}));
    run(&mut s, "insert.textBox", json!({"text": "Box"}));
    select_box(&mut s);
    // Resize: stays inline, keeps a text box's minimum.
    run(&mut s, "arrange.bounds", json!({"width": 200, "height": 5}));
    let o = box_area(&mut s);
    assert_eq!((o.rect.w, o.rect.h, o.wrap), (200.0, 18.0, Wrap::Inline));
    assert!(s.run("arrange.nudge", &json!({"dx": 5})).is_err(), "inline objects don't nudge");
    // Move: floats exactly where it was dropped, still selected.
    let r = run(&mut s, "arrange.bounds", json!({"page": 0, "x": 300, "y": 400}));
    assert_eq!(r["object"]["float"]["wrap"], "square");
    let o = box_area(&mut s);
    assert_eq!((o.page, o.rect.x, o.rect.y), (0, 300.0, 400.0));
    assert_eq!(s.doc.plain_text(StoryRef::Part(o.text_box.unwrap())), "Box");
    // Moving it again anchors it to the paragraph under its top edge, relative to that
    // paragraph (it moves with the text), landing exactly where dropped.
    let before = box_area(&mut s).rect;
    select_box(&mut s);
    run(&mut s, "arrange.bounds", json!({"x": before.x + 30.0, "y": before.y - 5.0}));
    let o = box_area(&mut s);
    assert!((o.rect.x - before.x - 30.0).abs() < 0.01 && (o.rect.y - before.y + 5.0).abs() < 0.01, "{:?} → {:?}", before, o.rect);
    let p = s.doc.para(StoryRef::Body, &o.path).unwrap();
    let Some(wordcraft_doc::InlineObject::Shape { float, .. }) = p.object_at(o.off) else { panic!() };
    assert_eq!((float.h_rel, float.v_rel), (Anchor::Column, Anchor::Paragraph));
    assert!((float.x - (o.rect.x - o.origin.x)).abs() < 0.01 && (float.y - (o.rect.y - o.origin.y)).abs() < 0.01);
    // Nudge.
    run(&mut s, "arrange.nudge", json!({"dx": 6, "dy": -1}));
    let n = box_area(&mut s).rect;
    assert!((n.x - o.rect.x - 6.0).abs() < 0.01 && (n.y - o.rect.y + 1.0).abs() < 0.01);
    // Hostile values stay on the page.
    run(&mut s, "arrange.bounds", json!({"x": -1e9, "y": 1e9, "width": 1e9}));
    let o = box_area(&mut s);
    assert!(o.rect.right() >= 12.0 && o.rect.y <= 792.0 && o.rect.w <= 4000.0, "{:?}", o.rect);
    // One undo per change.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(box_area(&mut s).rect, n);
}

/// An object dropped near the bottom of a page lands there, though at the top of the paragraph
/// it anchors to it would push that paragraph onto the next page (#3169).
#[test]
fn an_object_dropped_near_the_bottom_of_a_page_lands_there() {
    let para = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
    let mut s = Session::new(wordcraft_doc::Document::from_text(&vec![para; 20].join("\n")));
    run(&mut s, "insert.textBox", json!({"text": "Box"}));
    select_box(&mut s);
    run(&mut s, "arrange.bounds", json!({"width": 450, "height": 300}));
    for y in [400.0, 600.0, 760.0] {
        select_box(&mut s);
        run(&mut s, "arrange.bounds", json!({"page": 0, "x": 80, "y": y}));
        let o = box_area(&mut s);
        assert_eq!(o.page, 0);
        assert!((o.rect.x - 80.0).abs() < 0.01 && (o.rect.y - y).abs() < 0.01, "dropped at {y}: {:?}", o.rect);
    }
}

#[test]
fn nudging_an_aligned_object_moves_it_from_where_it_is() {
    use wordcraft_doc::para::{Anchor, FloatAlign};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Some text "}));
    run(&mut s, "insert.textBox", json!({"text": "Box"}));
    select_box(&mut s);
    run(&mut s, "arrange.position", json!({"preset": "topLeft"}));
    // As a .docx can place it: centred on the page, at the bottom margin.
    let o = box_area(&mut s);
    let p = s.doc.para_mut(StoryRef::Body, &o.path).unwrap();
    if let Some(wordcraft_doc::InlineObject::Shape { float, .. }) = p.object_at_mut(o.off) {
        (float.h_rel, float.h_align, float.v_rel, float.v_align) =
            (Anchor::Page, Some(FloatAlign::Center), Anchor::BottomMargin, Some(FloatAlign::Start));
    }
    p.touch();
    s.touch();
    let before = box_area(&mut s).rect;
    select_box(&mut s);
    run(&mut s, "arrange.nudge", json!({"dx": 6, "dy": -2}));
    let o = box_area(&mut s);
    assert!((o.rect.x - before.x - 6.0).abs() < 0.01 && (o.rect.y - before.y + 2.0).abs() < 0.01, "{before:?} → {:?}", o.rect);
    let p = s.doc.para(StoryRef::Body, &o.path).unwrap();
    let Some(wordcraft_doc::InlineObject::Shape { float, .. }) = p.object_at(o.off) else { panic!() };
    assert_eq!((float.h_align, float.v_align), (None, None));
    // Dragging it clears alignment too, and it lands where dropped.
    select_box(&mut s);
    run(&mut s, "arrange.bounds", json!({"x": 100, "y": 150}));
    let o = box_area(&mut s);
    assert_eq!((o.rect.x, o.rect.y), (100.0, 150.0));
}

#[test]
fn arrange_bounds_moves_an_object_to_another_page() {
    let mut s = s();
    for i in 0..60 {
        run(&mut s, "text.insert", json!({"text": format!("Paragraph {i} of filler text.")}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
    run(&mut s, "insert.textBox", json!({"text": "Traveller"}));
    select_box(&mut s);
    assert!(s.layout().pages.len() >= 2);
    run(&mut s, "arrange.bounds", json!({"page": 1, "x": 100, "y": 200}));
    let o = box_area(&mut s);
    assert_eq!((o.page, o.rect.x, o.rect.y), (1, 100.0, 200.0));
    assert!(o.path.0[0] > 0, "anchored to text on page 2: {:?}", o.path);
    assert_eq!(s.doc.plain_text(StoryRef::Part(o.text_box.unwrap())), "Traveller");
    // The selection follows the object.
    assert_eq!(s.sel.anchor, o.pos());
    assert!(s.run("arrange.bounds", &json!({"page": 99, "x": 1, "y": 1})).is_err());
}

#[test]
fn moved_text_box_makes_the_text_it_lands_on_wrap() {
    let mut s = s();
    for i in 0..6 {
        run(
            &mut s,
            "text.insert",
            json!({"text": format!("Paragraph {i}: the quick brown fox jumps over the lazy dog, again and again, across the page.")}),
        );
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "caret.set", json!({"pos": Pos::body(4, 0)}));
    run(&mut s, "insert.textBox", json!({"text": "Box"}));
    select_box(&mut s);
    // Drop it over paragraph 1, right half of the page: well above its own paragraph (4).
    let l = s.layout();
    let p1_top = wordcraft_layout::hit::page_lines(&l.pages[0], StoryRef::Body).iter().find(|ln| ln.path.0 == [1] && ln.li == 0).unwrap().top;
    run(&mut s, "arrange.bounds", json!({"x": 300, "y": p1_top + 2.0}));
    let o = box_area(&mut s);
    assert_eq!(o.path.0, vec![1], "anchored to the paragraph it was dropped on");
    assert_eq!((o.rect.x, o.rect.y), (300.0, p1_top + 2.0));
    // Lines beside the box stop short of it; lines below it use the full width again.
    let l = s.layout();
    let lines = wordcraft_layout::hit::page_lines(&l.pages[0], StoryRef::Body);
    let beside: Vec<_> = lines.iter().filter(|ln| ln.bottom > o.rect.y && ln.top < o.rect.bottom()).collect();
    assert!(!beside.is_empty());
    assert!(beside.iter().all(|ln| ln.right <= o.rect.x + 0.5 || ln.left >= o.rect.right() - 0.5), "text overlaps the box");
    assert!(lines.iter().any(|ln| ln.top > o.rect.bottom() + 10.0 && ln.right > o.rect.right()));
    // Its old paragraph no longer holds it.
    assert!(s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(4)).unwrap().objects.is_empty());
}

#[test]
fn deleted_text_box_takes_its_text_with_it_until_undo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "A"}));
    let id = run(&mut s, "insert.textBox", json!({"text": "Inside"}))["story"].as_u64().unwrap() as u32;
    let box_sel = json!({"anchor": Pos::body(0, 1), "focus": Pos::body(0, 1 + 3)});
    run(&mut s, "select.range", box_sel.clone());
    run(&mut s, "text.delete", json!({}));
    assert!(!s.doc.parts.contains_key(&id), "the deleted box's text is gone");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Part(id)), "Inside", "undo brings it back");
    // Cut and paste: one box, one story, same text.
    run(&mut s, "select.range", box_sel);
    run(&mut s, "edit.cut", json!({}));
    assert!(s.doc.parts.is_empty());
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.doc.parts.len(), 1);
    let (_, part) = s.doc.parts.iter().next().unwrap();
    assert_eq!(part.kind, wordcraft_doc::PartKind::TextBox);
    assert_eq!(s.doc.word_count_including_notes(), 2);
}

#[test]
fn text_box_in_a_table_nested_seventeen_deep_survives_an_edit_and_a_save() {
    // #1339: opened from a .docx, a box shown only from a table nested deeper than 16 was taken
    // for unshown, so the next unrelated edit pruned its story and saving dropped its text.
    use wordcraft_doc::{Block, Document, InlineObject, Paragraph, PartKind, Path, Table, para_block};
    let mut d = Document::from_text("Intro");
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Deep box", Default::default()))]);
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 72.0,
        h: 36.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: Some(id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let mut p = Paragraph::with_text("Cell", Default::default());
    p.insert_object(0, shape, &Default::default()).unwrap();
    d.body.extend((0..17).fold(vec![para_block(p)], |inner, _| {
        let mut t = Table::new(1, 1, 300.0);
        t.rows[0].cells[0].blocks = inner;
        vec![std::sync::Arc::new(Block::Table(t))]
    }));
    // The innermost cell's paragraph: the table after "Intro", then row, cell and block 0 down.
    let path = Path([vec![1], vec![0; 17 * 3]].concat());
    let deep_box = |doc: &Document| {
        let id = doc.para(StoryRef::Body, &path)?.objects.iter().find_map(InlineObject::text_box)?;
        doc.parts.get(&id).filter(|p| p.kind == PartKind::TextBox).map(|_| doc.plain_text(StoryRef::Part(id)))
    };
    let mut s = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap());
    assert_eq!(deep_box(&s.doc).as_deref(), Some("Deep box"), "opened");
    let unshown = s.doc.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Shown nowhere", Default::default()))]);
    run(&mut s, "text.insert", json!({"text": "Edited. "}));
    assert_eq!(deep_box(&s.doc).as_deref(), Some("Deep box"), "kept through the edit");
    assert!(!s.doc.parts.contains_key(&unshown), "a box shown nowhere still goes");
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(deep_box(&back).as_deref(), Some("Deep box"), "saved");
    assert!(back.plain_text(StoryRef::Body).starts_with("Edited. Intro"));
}

#[test]
fn word_count_with_an_object_selected_counts_the_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "one two "}));
    run(&mut s, "insert.textBox", json!({"text": "three"}));
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 8), "focus": Pos::body(0, 8 + 3)}));
    assert_eq!(run(&mut s, "review.wordCount", json!({}))["words"].as_u64(), Some(3));
    // A real text selection still counts just itself.
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 3)}));
    assert_eq!(run(&mut s, "review.wordCount", json!({}))["words"].as_u64(), Some(1));
}

/// #2982: a property imported from a file made by an older Word may have a name longer than
/// today's limit. Such a name is still deletable (the dialog sends `null`), while setting or
/// renaming a property to it stays refused.
#[test]
fn custom_properties_delete_legacy_names_but_new_names_keep_the_limit() {
    let mut s = s();
    let long = "x".repeat(crate::cmd::file::MAX_CUSTOM_NAME + 1);
    s.doc.custom_props.push(wordcraft_doc::CustomProp { name: long.clone(), kind: "raw".into(), value: "<data/>".into(), ..Default::default() });
    let mut custom = serde_json::Map::new();
    custom.insert(long.clone(), serde_json::Value::Null);
    run(&mut s, "file.properties", json!({"custom": custom}));
    assert!(s.doc.custom_props.is_empty(), "the legacy name could be deleted");
    assert!(s.run("file.properties", &json!({"custom": { (long): "x" }})).is_err(), "a new or updated name keeps the limit");
}

#[test]
fn custom_properties_set_read_remove_and_undo() {
    let mut s = s();
    let r = run(&mut s, "file.properties", json!({"custom": {"ZOTERO_PREF_1": "<data/>", "Status": "draft"}}));
    let custom = r["custom"].as_array().cloned().unwrap_or_default();
    assert!(custom.iter().any(|p| p["name"] == "Status" && p["value"] == "draft" && p["kind"] == "lpwstr"));
    assert_eq!(custom.len(), 2);
    assert_eq!(s.doc.custom_prop("zotero_pref_1"), Some("<data/>"));
    let r = run(&mut s, "file.properties", json!({"custom": {"status": null}}));
    assert_eq!(r["custom"].as_array().map(|a| a.len()), Some(1));
    assert!(s.run("file.properties", &json!({"custom": "x"})).is_err());
    assert!(s.run("file.properties", &json!({"custom": {"": "x"}})).is_err());
    assert!(s.run("file.properties", &json!({"custom": {"n": 3}})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.custom_prop("Status"), Some("draft"));
}

/// #2982: a case-only rename arrives as the old spelling with null and the new one with its
/// value. The removal must run first: object keys are sorted, so `Status` sorts before
/// `status`, and setting before removing would let the removal take the new spelling too.
#[test]
fn custom_properties_case_only_rename_keeps_the_new_spelling() {
    let mut s = s();
    run(&mut s, "file.properties", json!({"custom": {"status": "draft"}}));
    run(&mut s, "file.properties", json!({"custom": {"Status": "draft", "status": null}}));
    let props: Vec<(&str, &str)> = s.doc.custom_props.iter().map(|p| (p.name.as_str(), p.value.as_str())).collect();
    assert_eq!(props, [("Status", "draft")], "the property keeps the new spelling and its value");
}

/// Linked text objects carry rename metadata explicitly and retain the existing input bounds.
#[test]
fn custom_properties_linked_values_are_typed_bounded_and_atomic() {
    let mut s = s();
    s.doc.set_custom_prop("Status", "draft");
    s.doc.custom_props[0].link_target = Some("OriginalBookmark".into());
    let before = s.doc.clone();
    let depth = s.undo_depth();
    let max = crate::cmd::file::MAX_CUSTOM_VALUE;
    for value in [
        json!({"value": "changed", "linkTarget": 7}),
        json!({"value": 7, "linkTarget": "OriginalBookmark"}),
        json!({"value": "changed", "linkTarget": "OriginalBookmark", "kind": "raw"}),
        json!({"value": "x".repeat(max + 1), "linkTarget": "OriginalBookmark"}),
        json!({"value": "changed", "linkTarget": "x".repeat(max + 1)}),
    ] {
        assert!(s.run("file.properties", &json!({"title": "must roll back", "custom": {"Status": null, "Renamed": value}})).is_err());
        assert_eq!(s.doc, before);
        assert_eq!(s.undo_depth(), depth);
    }
    run(&mut s, "file.properties", json!({"custom": {"Status": {"value": "final", "linkTarget": "OriginalBookmark"}}}));
    run(&mut s, "file.properties", json!({"custom": {"Status": "string edit"}}));
    assert_eq!(s.doc.custom_props[0].link_target.as_deref(), Some("OriginalBookmark"));
    for value in [json!({"value": "unlinked", "linkTarget": null}), json!({"value": "unlinked"})] {
        s.doc.custom_props[0].link_target = Some("OriginalBookmark".into());
        run(&mut s, "file.properties", json!({"custom": {"Status": value}}));
        assert_eq!(s.doc.custom_props[0].link_target, None);
    }
}

/// #2982: what the Custom Properties dialog saves (through `file.properties`) survives a .docx
/// save and reopen.
#[test]
fn custom_properties_round_trip_through_docx() {
    let mut s = s();
    run(&mut s, "file.properties", json!({"custom": {"Status": "draft", "Reviewer": "Ada L."}}));
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(back.custom_prop("Status"), Some("draft"));
    assert_eq!(back.custom_prop("Reviewer"), Some("Ada L."));
    assert_eq!(back.custom_props.len(), 2);
    // One call removes one and adds another; the saved file follows the document.
    run(&mut s, "file.properties", json!({"custom": {"Status": null, "Owner": "Ada L."}}));
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(back.custom_prop("Status"), None);
    assert_eq!(back.custom_prop("Reviewer"), Some("Ada L."));
    assert_eq!(back.custom_prop("Owner"), Some("Ada L."));
    assert_eq!(back.custom_props.len(), 2);
}

/// `document_id` tells an edit from a replacement (the UI drops a "Save changes?" prompt
/// about a document that has been replaced).
#[test]
fn document_id_changes_only_when_the_document_is_replaced() {
    let mut s = s();
    let first = s.document_id();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "format.bold", json!({}));
    assert_eq!(s.document_id(), first);
    run(&mut s, "file.new", json!({"template": "letter"}));
    let second = s.document_id();
    assert_ne!(second, first);
    run(&mut s, "file.new", json!({}));
    assert_ne!(s.document_id(), second);
}

/// Envelopes, Labels and Finish & Merge open a new, untitled document beside the main one, as Word
/// does, and like New its undo history starts afresh. Undo/Redo across documents would put the
/// wrong document under a file (Undo, Save As, Redo, Save wrote the envelope over the saved file),
/// so it isn't offered. The main document stays open as it was, with no undo step for the command.
#[test]
fn mailings_results_are_new_untitled_documents() {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-mailings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("letter.docx");
    let saved_as = dir.join("saved-as.docx");
    let on_file = |p: &std::path::Path| crate::io::open_path(p).unwrap().plain_text(StoryRef::Body);
    for id in ["mailings.envelopes", "mailings.labels", "mailings.finish"] {
        let mut s = s();
        run(&mut s, "mailings.recipients", json!({"csv": "First Name\nAda\nAlan"}));
        run(&mut s, "text.insert", json!({"text": "Dear "}));
        run(&mut s, "mailings.insertField", json!({"field": "First Name"}));
        run(&mut s, "file.save", json!({"path": original.to_string_lossy()}));
        let on_disk = std::fs::read(&original).unwrap();
        run(&mut s, "text.insert", json!({"text": ", unsaved"}));
        let before = text(&s);
        let document = s.document_id();

        let undo_steps = s.undo_labels().len();
        run(&mut s, id, json!({}));
        let result = text(&s);
        assert_ne!(result, before, "{id}: the result is the active document");
        assert_ne!(s.document_id(), document, "{id}: a different document");
        let main = s.with_document(document, |s| (text(s), s.undo_labels().len())).unwrap();
        assert_eq!(main, (before.clone(), undo_steps), "{id}: the main document stays open as it was");
        assert_eq!(s.path, None, "{id}: the result is untitled");
        assert_eq!(run(&mut s, "file.save", json!({}))["saved"], false, "{id}: Save asks where (Save As)");
        assert_eq!(std::fs::read(&original).unwrap(), on_disk, "{id}: the original file is untouched");

        // Undo doesn't bring the old document back under the new one's identity; Redo can't
        // put the result back under a file saved in between.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(text(&s), result, "{id}: history starts afresh, like New");
        run(&mut s, "file.saveAs", json!({"path": saved_as.to_string_lossy()}));
        let written = on_file(&saved_as);
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(text(&s), result, "{id}: nothing to redo");
        run(&mut s, "file.save", json!({}));
        assert_eq!(on_file(&saved_as), written, "{id}: the saved file still holds what was saved");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// #2376: a note.txt opened and saved stayed unsaved, so closing asked to save it forever. Saving
/// the document's own file is a save in any format; Save As to plain text elsewhere is a copy.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn saving_a_text_file_over_itself_saves_the_document() {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-own-txt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["note.txt", "note.md"] {
        let path = dir.join(name);
        std::fs::write(&path, b"").unwrap();
        let mut s = s();
        run(&mut s, "file.open", json!({"path": path.to_string_lossy()}));
        run(&mut s, "text.insert", json!({"text": "Hello"}));
        assert!(s.dirty);
        assert_eq!(run(&mut s, "file.save", json!({}))["saved"], true);
        assert!(!s.dirty, "{name}: saved over itself");
        assert!(std::fs::read_to_string(&path).unwrap().contains("Hello"), "{name}: written");

        // Saving the same file by name is the same save.
        run(&mut s, "text.insert", json!({"text": " again"}));
        run(&mut s, "file.save", json!({"path": path.to_string_lossy()}));
        assert!(!s.dirty, "{name}: saved by its own path");

        // Plain text somewhere else is still a copy: the document keeps its file and its changes.
        run(&mut s, "text.insert", json!({"text": "!"}));
        let copy = dir.join(format!("copy-{name}"));
        run(&mut s, "file.saveAs", json!({"path": copy.to_string_lossy()}));
        assert!(copy.exists());
        assert!(s.dirty, "{name}: a copy elsewhere leaves it unsaved");
        assert_eq!(s.path.as_deref(), Some(path.as_path()));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A save that fails to write (no permission, disk full, a bad path) leaves the document's own
/// save stamps untouched: `stamp_save` runs before the fallible write, so a prior version of this
/// left `modified`/`created`/`last_modified_by`/`revision`/`zoom` changed for a save that never
/// actually happened, which a later successful save would then persist.
#[test]
fn a_failed_save_leaves_the_save_stamps_untouched() {
    // pid + nanos (not pid alone): a name no other test or run could already own, so there's
    // nothing to pre-clean and nothing of anyone else's this test could delete by mistake. `dir`
    // itself is deliberately never created: that's what makes the write below fail.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-save-stamps-{}-{nanos}", std::process::id()));
    assert!(!dir.exists(), "a fresh name");
    let bogus = dir.join("does-not-exist").join("out.docx");
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "draft"}));
    let core_before = s.doc.core.clone();
    let zoom_before = s.doc.settings.zoom;

    let error = s.run("file.save", &json!({"path": bogus.to_string_lossy()}));
    assert!(error.is_err(), "the write fails: its parent directory doesn't exist");
    assert!(!bogus.exists());
    assert_eq!(s.doc.core, core_before, "modified/created/author/revision are unchanged");
    assert_eq!(s.doc.settings.zoom, zoom_before);
    assert!(s.path.is_none(), "the failed write never takes the document's path");
    assert!(s.dirty, "still unsaved");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_load_relative_to_the_file() {
    // Issue #98: `<img src="logo.png">` beside an HTML file is embedded when it is opened.
    let dir = std::env::temp_dir().join(format!("wordcraft-html-img-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("img")).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(dir.join("img/my logo.png"), &png).unwrap();
    std::fs::write(dir.join("secret.txt"), b"not a picture").unwrap();
    let html = r#"<p><img src="img/my%20logo.png" alt="Logo"></p><p><img src="secret.txt" alt="T"></p><p><img src="http://example.com/x.png" alt="Web"></p>"#;
    std::fs::write(dir.join("page.html"), html).unwrap();
    let doc = crate::io::open_path(&dir.join("page.html"));
    let _ = std::fs::remove_dir_all(&dir);
    let doc = doc.unwrap();
    assert_eq!(doc.media.len(), 1);
    let text = doc.plain_text(StoryRef::Body);
    assert!(!text.contains("Logo") && text.contains('T') && text.contains("Web"), "{text:?}");
}

/// A fresh, empty scratch folder for one test.
#[cfg(not(target_arch = "wasm32"))]
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(not(target_arch = "wasm32"))]
fn tiny_png() -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_outside_the_folder_are_not_loaded() {
    // #98 review: `..` must not reach files outside the HTML file's folder.
    let root = scratch_dir("html-img-escape");
    std::fs::create_dir_all(root.join("site/img")).unwrap();
    std::fs::write(root.join("outside.png"), tiny_png()).unwrap();
    std::fs::write(root.join("site/inside.png"), tiny_png()).unwrap();
    let html = r#"<p><img src="../outside.png" alt="Up"></p><p><img src="img/../../outside.png" alt="Sneak"></p><p><img src="img/../inside.png" alt="In"></p>"#;
    std::fs::write(root.join("site/page.html"), html).unwrap();
    let doc = crate::io::open_path(&root.join("site/page.html"));
    let _ = std::fs::remove_dir_all(&root);
    let doc = doc.unwrap();
    assert_eq!(doc.media.len(), 1, "only the picture inside the folder loads");
    let text = doc.plain_text(StoryRef::Body);
    assert!(text.contains("Up") && text.contains("Sneak") && !text.contains("In"), "{text:?}");
}

#[cfg(unix)]
#[test]
fn html_pictures_through_a_symlink_out_of_the_folder_are_not_loaded() {
    // #98 review: a symlink inside the folder must not lead outside it.
    let root = scratch_dir("html-img-symlink");
    std::fs::create_dir_all(root.join("site")).unwrap();
    std::fs::write(root.join("outside.png"), tiny_png()).unwrap();
    std::os::unix::fs::symlink(root.join("outside.png"), root.join("site/link.png")).unwrap();
    std::fs::write(root.join("site/page.html"), r#"<p><img src="link.png" alt="Link"></p>"#).unwrap();
    let doc = crate::io::open_path(&root.join("site/page.html"));
    let _ = std::fs::remove_dir_all(&root);
    let doc = doc.unwrap();
    assert!(doc.media.is_empty());
    assert!(doc.plain_text(StoryRef::Body).contains("Link"));
}

#[cfg(unix)]
#[test]
fn html_pictures_load_only_from_inside_the_read_root() {
    // #3736: the HTML file is checked through its link, but its pictures load from beside the
    // link. Opened through a link outside the read root, they must not come from there.
    let root = scratch_dir("html-img-root");
    for d in ["in", "outside"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("in/page.html"), r#"<p><img src="pic.png" alt="Pic"></p>"#).unwrap();
    std::fs::write(root.join("in/pic.png"), tiny_png()).unwrap();
    std::fs::write(root.join("outside/pic.png"), tiny_png()).unwrap();
    std::os::unix::fs::symlink(root.join("in/page.html"), root.join("outside/page.html")).unwrap();
    let roots = crate::file_access::AutomationRoots::new(Some(&root.join("in")), None).unwrap();
    let through_link = crate::file_access::confine(roots.as_ref(), || crate::io::open_path(&root.join("outside/page.html")));
    let direct = crate::file_access::confine(roots.as_ref(), || crate::io::open_path(&root.join("in/page.html")));
    let _ = std::fs::remove_dir_all(&root);
    let through_link = through_link.unwrap();
    assert!(through_link.media.is_empty(), "no picture from outside the read root");
    assert!(through_link.plain_text(StoryRef::Body).contains("Pic"));
    assert_eq!(direct.unwrap().media.len(), 1, "pictures inside the root still load");
}

/// `file.open` on a path reads only a regular file: a named pipe blocked until a writer came, and
/// `/dev/zero` read until memory ran out.
#[cfg(unix)]
#[test]
fn opening_a_named_pipe_or_device_is_refused_without_blocking() {
    let dir = scratch_dir("open-fifo");
    let fifo = dir.join("pipe.docx");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("mkfifo unavailable; skipping");
        return;
    }
    let zero = dir.join("zero.txt");
    std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let (pipe, device) = (fifo.clone(), zero.clone());
    std::thread::spawn(move || {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let by_command = s.run("file.open", &json!({"path": pipe.to_string_lossy()})).map(|_| ());
        let _ = tx.send([by_command.map_err(|e| e.to_string()), crate::io::read_document(&device).map(|_| ())]);
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    if got.is_err() {
        // Unblock the stuck reader so the thread ends, then fail.
        let _ = std::os::unix::fs::OpenOptionsExt::custom_flags(std::fs::OpenOptions::new().write(true), libc::O_NONBLOCK).open(&fifo);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let [pipe, device] = got.expect("opening a named pipe blocked");
    assert!(pipe.unwrap_err().contains("not a file"));
    assert!(device.unwrap_err().contains("not a file"));
}

#[cfg(unix)]
#[test]
fn html_picture_that_is_a_named_pipe_is_skipped_without_blocking() {
    // #98 review: opening a FIFO blocks until a writer appears; it must never be opened.
    let dir = scratch_dir("html-img-fifo");
    let fifo = dir.join("pipe.png");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("mkfifo unavailable; skipping");
        return;
    }
    std::fs::write(dir.join("page.html"), r#"<p><img src="pipe.png" alt="Pipe"></p>"#).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let page = dir.join("page.html");
    std::thread::spawn(move || {
        let _ = tx.send(crate::io::open_path(&page));
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    if got.is_err() {
        // Unblock the stuck reader so the thread ends, then fail.
        let _ = std::os::unix::fs::OpenOptionsExt::custom_flags(std::fs::OpenOptions::new().write(true), libc::O_NONBLOCK).open(&fifo);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let doc = got.expect("opening the HTML file blocked on a named pipe").unwrap();
    assert!(doc.media.is_empty());
    assert!(doc.plain_text(StoryRef::Body).contains("Pipe"));
}

/// #98's rule for HTML pictures, for `insert.picture`'s own paths: opening a named pipe blocks
/// until a writer appears, and a device like `/dev/zero` never reaches end of file, so a picture
/// path that is not a regular file must be refused, not opened.
#[cfg(unix)]
#[test]
fn insert_picture_from_a_named_pipe_is_refused_without_blocking() {
    let dir = scratch_dir("insert-img-fifo");
    let fifo = dir.join("pipe.png");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("mkfifo unavailable; skipping");
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let path = fifo.to_string_lossy().into_owned();
    std::thread::spawn(move || {
        let _ = tx.send(crate::io::read_file(&path));
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    if got.is_err() {
        // Unblock the stuck reader so the thread ends, then fail.
        let _ = std::os::unix::fs::OpenOptionsExt::custom_flags(std::fs::OpenOptions::new().write(true), libc::O_NONBLOCK).open(&fifo);
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(got.expect("reading a picture path blocked on a named pipe").is_err(), "a pipe is not a file to read");
    if std::path::Path::new("/dev/zero").exists() {
        assert!(crate::io::read_file("/dev/zero").is_err(), "a device is not a file to read");
    }
}

/// #3666: `read_file_capped` (fonts, citation sources, `read_file`) and `read_file_head` (the
/// Word 2003 check on saving to an `.xml` path) read only a regular file. A named pipe must be
/// refused, not opened: opening one blocks until a writer appears.
#[cfg(unix)]
#[test]
fn file_head_and_capped_reads_refuse_a_named_pipe_without_blocking() {
    let dir = scratch_dir("read-helpers-fifo");
    let fifo = dir.join("pipe.xml");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("mkfifo unavailable; skipping");
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let path = fifo.to_string_lossy().into_owned();
    std::thread::spawn(move || {
        let head = crate::io::read_file_head(&path, 64 * 1024).map(|_| ());
        let capped = crate::io::read_file_capped(&path, 1 << 20, "font file").map(|_| ());
        let _ = tx.send([head, capped]);
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    if got.is_err() {
        // Unblock the stuck reader (twice: each helper may be stuck in turn) so the thread ends, then
        // fail. Non-blocking: with no reader left the open fails at once instead of hanging the test.
        let _ = std::os::unix::fs::OpenOptionsExt::custom_flags(std::fs::OpenOptions::new().write(true), libc::O_NONBLOCK).open(&fifo);
        let _ = std::os::unix::fs::OpenOptionsExt::custom_flags(std::fs::OpenOptions::new().write(true), libc::O_NONBLOCK).open(&fifo);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let [head, capped] = got.expect("reading a named pipe blocked");
    assert!(head.unwrap_err().contains("not a file"));
    assert!(capped.unwrap_err().contains("not a file"));
}

/// #3666: a folder is not a file to read, whichever helper reads it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn file_head_and_capped_reads_refuse_a_folder() {
    let dir = scratch_dir("read-helpers-dir");
    let path = dir.to_string_lossy().into_owned();
    let head = crate::io::read_file_head(&path, 64);
    let capped = crate::io::read_file_capped(&path, 1 << 20, "font file");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(head.unwrap_err().contains("not a file"));
    assert!(capped.unwrap_err().contains("not a file"));
}

/// #3666: `read_file_capped` reads a file of up to `max` bytes whole and refuses a larger one;
/// `read_file_head` returns at most `max` bytes.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn file_capped_read_refuses_content_over_the_cap() {
    let dir = scratch_dir("read-helpers-cap");
    let file = dir.join("data.bin");
    std::fs::write(&file, vec![7u8; 100]).unwrap();
    let path = file.to_string_lossy().into_owned();
    let exact = crate::io::read_file_capped(&path, 100, "file");
    let over = crate::io::read_file_capped(&path, 99, "file");
    let head = crate::io::read_file_head(&path, 10);
    let whole = crate::io::read_file_head(&path, 1000);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(exact.unwrap().len(), 100);
    assert!(over.unwrap_err().contains("larger than"));
    assert_eq!(head.unwrap(), vec![7u8; 10]);
    assert_eq!(whole.unwrap().len(), 100);
}

/// #3666: `read_regular_file_within` (Paste Special's copied picture file) reads a file of up to
/// `limit` bytes whole and refuses one whose handle reports more, before reading any of it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn regular_file_read_within_a_limit_refuses_a_larger_file() {
    let dir = scratch_dir("read-within");
    let file = dir.join("data.bin");
    std::fs::write(&file, vec![7u8; 100]).unwrap();
    let big = dir.join("big.bin");
    std::fs::File::create(&big).unwrap().set_len(64 << 20).unwrap();
    let exact = crate::io::read_regular_file_within(&file, 100);
    let over = crate::io::read_regular_file_within(&file, 99);
    let large = crate::io::read_regular_file_within(&big, 1 << 20);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(exact.map(|b| b.len()), Some(100));
    assert_eq!(over, None);
    assert_eq!(large, None);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_picture_size_limit_holds_whatever_length_the_file_reports() {
    use crate::io::LocalImages;
    let dir = scratch_dir("html-img-limit");
    std::fs::write(dir.join("ten.bin"), [7u8; 10]).unwrap();
    std::fs::write(dir.join("eleven.bin"), [7u8; 11]).unwrap();
    let images = LocalImages::with_limits(&dir, 10, 1000);
    let ten = images.load("ten.bin").map(|d| d.len());
    let eleven = images.load("eleven.bin");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(ten, Some(10));
    assert!(eleven.is_none());
    #[cfg(unix)]
    {
        // A device reports a length of 0 and never ends: it is not a regular file, and the read is
        // bounded anyway.
        let dev = LocalImages::with_limits(std::path::Path::new("/dev"), 1 << 20, 1 << 20);
        assert!(dev.load("zero").is_none());
        assert!(dev.load("null").is_none());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_share_one_read_and_a_per_document_budget() {
    // #98 review: a picture referenced many times is read once, and every reference counts
    // against the document's budget so a large picture can't be multiplied without bound.
    use crate::io::LocalImages;
    let dir = scratch_dir("html-img-budget");
    let png = tiny_png();
    let n = png.len() as u64;
    std::fs::write(dir.join("a.png"), &png).unwrap();
    let images = LocalImages::with_limits(&dir, 1 << 20, 3 * n);
    let first = images.load("a.png").unwrap();
    std::fs::write(dir.join("a.png"), b"changed on disk").unwrap();
    let second = images.load("./a.png").unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second), "the second reference reuses the first read");
    assert_eq!(*second, png);
    assert!(images.load("a.png").is_some());
    assert!(images.load("a.png").is_none(), "budget spent");
    // Files over the per-picture limit still cost what was read.
    std::fs::write(dir.join("big.bin"), vec![1u8; 100]).unwrap();
    std::fs::write(dir.join("small.bin"), [1u8; 5]).unwrap();
    let images = LocalImages::with_limits(&dir, 10, 25);
    assert!(images.load("big.bin").is_none());
    assert!(images.load("big.bin").is_none());
    let small = images.load("small.bin");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(small.is_none(), "two oversized reads used up the budget");
}

fn para_style(s: &Session, i: usize) -> Option<String> {
    s.doc.para_at(&Pos::body(i, 0)).and_then(|p| p.props.style.clone())
}

#[test]
fn styles_apply_by_name() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Title text"}));
    run(&mut s, "styles.apply", json!({"style": "Heading 1"}));
    assert_eq!(para_style(&s, 0).as_deref(), Some("Heading1"));
    assert_eq!(s.undo_label(), Some("Apply Style"));
    assert!(s.dirty);
}

#[test]
fn styles_apply_pane_is_not_an_edit() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    s.dirty = false;
    let undo = s.undo_labels();
    s.view.styles_pane = false;
    assert_eq!(run(&mut s, "styles.apply", json!({})), json!({"pane": true}));
    assert!(s.view.styles_pane);
    // Shows, never toggles.
    run(&mut s, "styles.apply", json!({}));
    assert!(s.view.styles_pane);
    assert_eq!(s.undo_labels(), undo);
    assert!(!s.dirty);
    // Works on a read-only document.
    s.doc.settings.protection = Some("readOnly".into());
    s.view.styles_pane = false;
    run(&mut s, "styles.apply", json!({}));
    assert!(s.view.styles_pane);
}

/// #1756: the Styles pane docks on the right (the default), on the left, or floats.
#[test]
fn styles_pane_docks_left_right_or_floats() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    s.dirty = false;
    let undo = s.undo_labels();
    assert_eq!(run(&mut s, "view.state", json!({}))["stylesPaneDock"], "right");
    assert_eq!(run(&mut s, "view.stylesPaneDock", json!({})), json!({"dock": "right"}), "no dock reads it");
    for dock in ["left", "float", "right"] {
        assert_eq!(run(&mut s, "view.stylesPaneDock", json!({"dock": dock})), json!({"dock": dock}));
        assert_eq!(run(&mut s, "view.stylesPaneDock", json!({})), json!({"dock": dock}));
        assert_eq!(run(&mut s, "view.state", json!({}))["stylesPaneDock"], dock);
    }
    run(&mut s, "view.stylesPaneDock", json!({"dock": "left"}));
    for junk in [json!({"dock": "top"}), json!({"dock": 1}), json!({"dock": null}), json!({"dock": ["left"]}), json!({"dock": "LEFT "})] {
        assert!(s.run("view.stylesPaneDock", &junk).is_err(), "{junk} is an error");
        assert_eq!(run(&mut s, "view.stylesPaneDock", json!({})), json!({"dock": "left"}), "{junk} changes nothing");
    }
    // A view setting: no undo step, the document isn't changed, and the pane's visibility stays.
    assert_eq!(s.undo_labels(), undo);
    assert!(!s.dirty);
    assert!(!s.view.styles_pane);
}

#[test]
fn styles_apply_rejects_bad_style_params() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let undo = s.undo_labels();
    let before = text(&s);
    for bad in [json!(5), json!(null), json!([]), json!({}), json!(""), json!("   "), json!(true)] {
        let r = s.run("styles.apply", &json!({"style": bad}));
        assert!(matches!(r, Err(crate::CmdError::Params(_))), "{bad}: {r:?}");
    }
    assert_eq!(s.undo_labels(), undo);
    assert_eq!(text(&s), before);
    assert_eq!(para_style(&s, 0), None);
}

#[test]
fn styles_apply_unknown_style_changes_nothing() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let undo = s.undo_labels();
    let doc = s.doc.clone();
    assert!(s.run("styles.apply", &json!({"style": "No Such Style"})).is_err());
    assert_eq!(s.undo_labels(), undo);
    assert_eq!(s.doc, doc);
    assert!(!s.can_redo());
}

#[test]
fn styles_apply_undo_redo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "styles.apply", json!({"style": "Heading 2"}));
    assert_eq!(para_style(&s, 0).as_deref(), Some("Heading2"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(para_style(&s, 0), None);
    assert_eq!(text(&s), "Hello");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(para_style(&s, 0).as_deref(), Some("Heading2"));
}

#[test]
fn styles_apply_character_style() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "make this strong"}));
    run(&mut s, "select.text", json!({"text": "this"}));
    run(&mut s, "styles.apply", json!({"style": "Strong"}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(6).style.as_deref(), Some("Strong"));
    assert_eq!(p.props_of_char(0).style, None);
    // A character style leaves the paragraph style alone.
    assert_eq!(p.props.style, None);
}

#[test]
fn styles_apply_multi_paragraph_selection() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "One"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Two"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Three"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "styles.apply", json!({"style": "Quote"}));
    for i in 0..3 {
        assert_eq!(para_style(&s, i).as_deref(), Some("Quote"), "paragraph {i}");
    }
    // One undo step for the whole selection.
    run(&mut s, "edit.undo", json!({}));
    for i in 0..3 {
        assert_eq!(para_style(&s, i), None, "paragraph {i}");
    }
}

/// #865: `styles.modify` takes a colour the way `format.color` does (`RRGGBB`, `#RRGGBB`, `auto`),
/// still reads the serialized forms, and rejects a bad colour without touching the style.
#[test]
fn modify_style_takes_hex_colours_like_format_color() {
    use wordcraft_doc::{CharProps, Rgb, TextColor};
    let mut s = s();
    let color = |s: &Session| s.doc.styles.get("Heading1").and_then(|st| st.chr.color);
    let modify = |s: &mut Session, c: serde_json::Value| s.run("styles.modify", &json!({"style": "Heading1", "chr": {"color": c}}));

    modify(&mut s, json!("1B1611")).unwrap();
    assert_eq!(color(&s), Some(TextColor::Rgb(Rgb(0x1B, 0x16, 0x11))));
    modify(&mut s, json!("auto")).unwrap();
    assert_eq!(color(&s), Some(TextColor::Auto));
    modify(&mut s, json!("#1B1611")).unwrap();
    assert_eq!(color(&s), Some(TextColor::Rgb(Rgb(0x1B, 0x16, 0x11))));
    modify(&mut s, json!("Auto")).unwrap();
    assert_eq!(color(&s), Some(TextColor::Auto));
    modify(&mut s, json!({"Rgb": [192, 0, 0]})).unwrap();
    let red = TextColor::Rgb(Rgb(0xC0, 0, 0));
    assert_eq!(color(&s), Some(red));
    assert_eq!(s.doc.styles.resolve_char(Some("Heading1"), &CharProps::default()).color, red);

    for bad in [json!("1B16"), json!("not a colour"), json!({"Rgb": "1B16"}), json!(7)] {
        let r = modify(&mut s, bad.clone());
        assert!(matches!(r, Err(crate::CmdError::Params(_))), "{bad}: {r:?}");
        assert_eq!(color(&s), Some(red), "{bad} leaves the style alone");
    }
    modify(&mut s, json!({"Rgb": "1B1611"})).unwrap();
    assert_eq!(color(&s), Some(TextColor::Rgb(Rgb(0x1B, 0x16, 0x11))), "the hex string also reads inside {{\"Rgb\": …}}");
}

/// #865: the other colours in CharProps and ParaProps (`underlineColor`, `shading`, a border's
/// `color`) take `RRGGBB` / `#RRGGBB` like `para.borders` and `table.shading`, and still `[r, g, b]`.
#[test]
fn props_colours_take_hex_strings() {
    use wordcraft_doc::{Pos, Rgb};
    let ink = Rgb(0x1B, 0x16, 0x11);
    let mut s = s();
    run(
        &mut s,
        "styles.modify",
        json!({"style": "Heading1",
            "chr": {"underlineColor": "1B1611", "shading": "#1b1611", "border": {"style": "single", "width": 1, "color": "1B1611"}},
            "para": {"shading": "1B1611", "borders": {"bottom": {"style": "single", "width": 1, "color": "#1B1611"}}}}),
    );
    let st = s.doc.styles.get("Heading1").unwrap();
    assert_eq!((st.chr.underline_color, st.chr.shading, st.chr.border.and_then(|b| b.color)), (Some(ink), Some(ink), Some(ink)));
    assert_eq!((st.para.shading, st.para.borders.and_then(|b| b.bottom).and_then(|b| b.color)), (Some(ink), Some(ink)));

    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "para.set", json!({"props": {"shading": "C00000"}}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.shading, Some(Rgb(0xC0, 0, 0)));
    run(&mut s, "para.set", json!({"props": {"shading": [1, 2, 3]}}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.shading, Some(Rgb(1, 2, 3)), "[r, g, b] still reads");

    for bad in [json!("C0000"), json!("auto"), json!([1, 2]), json!([1, 2, 300]), json!(7)] {
        let r = s.run("para.set", &json!({"props": {"shading": bad}}));
        assert!(matches!(r, Err(crate::CmdError::Params(_))), "{bad}: {r:?}");
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.shading, Some(Rgb(1, 2, 3)), "{bad} changes nothing");
    }
}

/// #555: Update Heading 2 to Match Selection (the styles pane's context menu) makes Heading 2
/// look like the selected text — its paragraph style and direct formatting — so every Heading 2
/// paragraph changes; the selected paragraph takes Heading 2, and it is one undo step.
#[test]
fn update_heading_to_match_selection_takes_the_selections_formatting() {
    use wordcraft_doc::{Align, CharProps, ParaProps, Rgb, TextColor};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Heading text"}));
    run(&mut s, "styles.apply", json!({"style": "Heading 2"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Body text"}));
    run(&mut s, "styles.apply", json!({"style": "Normal"}));
    run(&mut s, "select.text", json!({"text": "Body text"}));
    run(&mut s, "format.size", json!({"size": 20}));
    run(&mut s, "format.italic", json!({}));
    run(&mut s, "format.color", json!({"color": "C00000"}));
    run(&mut s, "para.alignCenter", json!({}));
    let normal = s.doc.styles.resolve_char(Some("Normal"), &CharProps::default());
    let before = s.undo_depth();

    run(&mut s, "styles.updateToMatch", json!({"style": "Heading2"}));

    let h2 = ParaProps { style: Some("Heading2".into()), ..Default::default() };
    let c = s.doc.styles.resolve_char(Some("Heading2"), &CharProps::default());
    assert_eq!(c.size, 20.0);
    assert!(c.italic);
    assert_eq!(c.color, TextColor::Rgb(Rgb(0xC0, 0, 0)));
    assert_eq!(c.font, normal.font, "the heading font gives way to the selection's");
    let p = s.doc.styles.resolve_para(&h2);
    assert_eq!(p.align, Align::Center);
    assert_eq!(p.outline_level, Some(1), "Heading 2 stays a level-2 heading");
    // The selected paragraph now carries Heading 2, its direct formatting absorbed by the style.
    assert_eq!(para_style(&s, 1).as_deref(), Some("Heading2"));
    let body = s.doc.para_at(&Pos::body(1, 0)).unwrap();
    assert!(body.props_of_char(0).formatting().is_empty(), "{:?}", body.props_of_char(0));
    assert_eq!(body.props.align, None);
    assert_eq!(s.undo_depth(), before + 1, "one undo step");

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.styles.resolve_char(Some("Heading2"), &CharProps::default()).size, 16.0);
    assert_eq!(para_style(&s, 1).as_deref(), Some("Normal"));
}

#[test]
fn styles_apply_rejected_on_protected_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    s.doc.settings.protection = Some("readOnly".into());
    let undo = s.undo_labels();
    let r = s.run("styles.apply", &json!({"style": "Heading 1"}));
    assert!(matches!(r, Err(crate::CmdError::Disabled(_))), "{r:?}");
    assert_eq!(para_style(&s, 0), None);
    assert_eq!(s.undo_labels(), undo);
}

fn column_doc() -> Session {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "abcdef\nabcdef\nabcdef"}));
    s
}

fn block(s: &mut Session) {
    let a = serde_json::to_value(Pos::body(0, 2)).unwrap();
    let f = serde_json::to_value(Pos::body(2, 4)).unwrap();
    let r = run(s, "select.column", json!({"anchor": a, "focus": f}));
    assert_eq!(r["rows"], json!(["cd", "cd", "cd"]));
}

#[test]
fn column_selection_copies_the_block() {
    let mut s = column_doc();
    block(&mut s);
    let r = run(&mut s, "edit.copy", json!({}));
    assert_eq!(r["text"], "cd\ncd\ncd");
    assert_eq!(s.clipboard.as_ref().map(|f| f.blocks.len()), Some(3));
    // A caret move drops the block; pasting the copy gives three paragraphs.
    run(&mut s, "caret.docEnd", json!({}));
    assert!(s.column_segments().is_none());
    s.prefs.autocorrect.enabled = false;
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(text(&s), "abcdef\nabcdef\nabcdef\ncd\ncd\ncd");
}

#[test]
fn column_selection_deletes_only_the_block() {
    let mut s = column_doc();
    block(&mut s);
    run(&mut s, "text.delete", json!({}));
    assert_eq!(text(&s), "abef\nabef\nabef");
    assert_eq!(s.sel, crate::Selection::caret(Pos::body(0, 2)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "abcdef\nabcdef\nabcdef");
    // Cut copies and deletes; typing replaces the block.
    block(&mut s);
    let r = run(&mut s, "edit.cut", json!({}));
    assert_eq!(r["text"], "cd\ncd\ncd");
    assert_eq!(text(&s), "abef\nabef\nabef");
    run(&mut s, "edit.undo", json!({}));
    block(&mut s);
    run(&mut s, "text.insert", json!({"text": "X"}));
    assert_eq!(text(&s), "abXef\nabef\nabef");
}

#[test]
fn column_selection_formats_each_row() {
    let mut s = column_doc();
    block(&mut s);
    run(&mut s, "format.bold", json!({}));
    for i in 0..3 {
        let p = s.doc.para_at(&Pos::body(i, 0)).unwrap();
        assert_ne!(p.props_of_char(1).bold, Some(true));
        assert_eq!(p.props_of_char(2).bold, Some(true));
        assert_eq!(p.props_of_char(3).bold, Some(true));
        assert_ne!(p.props_of_char(4).bold, Some(true));
    }
    // The block survives formatting, and a second Bold turns it off again.
    assert!(s.column_segments().is_some());
    run(&mut s, "format.bold", json!({}));
    assert_ne!(s.doc.para_at(&Pos::body(1, 0)).unwrap().props_of_char(2).bold, Some(true));
    run(&mut s, "format.changeCase", json!({"mode": "upper"}));
    assert_eq!(text(&s), "abCDef\nabCDef\nabCDef");
}

#[test]
fn column_mode_extends_with_the_arrow_keys() {
    let mut s = column_doc();
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 1}}));
    let r = run(&mut s, "select.column", json!({}));
    assert_eq!(r["columnMode"], true);
    run(&mut s, "caret.right", json!({}));
    run(&mut s, "caret.right", json!({}));
    run(&mut s, "caret.down", json!({}));
    let r = run(&mut s, "caret.down", json!({}));
    assert_eq!(r["rows"], json!(["bc", "bc", "bc"]));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(text(&s), "adef\nadef\nadef");
    assert!(!s.column_mode);
    // Escape leaves the mode.
    run(&mut s, "select.column", json!({}));
    run(&mut s, "select.collapse", json!({}));
    assert!(!s.column_mode);
}

#[test]
fn column_selection_from_points() {
    let mut s = column_doc();
    let l = s.layout();
    let a = l.caret(&Pos::body(0, 1)).unwrap();
    let b = l.caret(&Pos::body(1, 5)).unwrap();
    let r = run(
        &mut s,
        "select.column",
        json!({"from": {"page": a.page, "x": a.x, "y": a.top + 2.0}, "to": {"page": b.page, "x": b.x, "y": b.top + 2.0}}),
    );
    assert_eq!(r["rows"], json!(["bcde", "bcde"]));
    for bad in [
        json!({"from": {"page": 99, "x": 1, "y": 1}, "to": {"page": 0, "x": 1, "y": 1}}),
        json!({"from": 1, "to": "x"}),
        json!({"anchor": 1, "focus": {}}),
    ] {
        assert!(s.run("select.column", &bad).is_err());
    }
}

#[test]
fn style_inspector_reports_and_clears_levels() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Chapter one"}));
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({"value": true}));
    run(&mut s, "para.alignCenter", json!({}));

    let r = run(&mut s, "styles.inspect", json!({}));
    assert_eq!(r["paragraph"]["style"], "Heading1");
    assert_eq!(r["paragraph"]["styleName"], "Heading 1");
    let pdirect = r["paragraph"]["direct"].as_array().unwrap();
    assert!(pdirect.iter().any(|d| d["prop"] == "align" && d["value"] == "center"), "{pdirect:?}");
    assert!(r["character"]["style"].is_null());
    let cdirect = r["character"]["direct"].as_array().unwrap();
    assert_eq!(cdirect, &vec![json!({"prop": "bold", "value": true}), json!({"prop": "boldCs", "value": true})]);

    // Formatting that matches the style isn't a difference (Heading 1 is 20 pt).
    run(&mut s, "format.size", json!({"size": 20}));
    let r = run(&mut s, "styles.inspect", json!({}));
    let cdirect = r["character"]["direct"].as_array().unwrap();
    assert_eq!(cdirect, &vec![json!({"prop": "bold", "value": true}), json!({"prop": "boldCs", "value": true})]);

    // Character style level.
    run(&mut s, "format.charStyle", json!({"style": "Emphasis"}));
    let r = run(&mut s, "styles.inspect", json!({}));
    assert_eq!(r["character"]["style"], "Emphasis");
    assert_eq!(r["character"]["styleName"], "Emphasis");
    run(&mut s, "styles.inspectorClear", json!({"level": "characterStyle"}));
    assert!(run(&mut s, "styles.inspect", json!({}))["character"]["style"].is_null());

    // Clearing character formatting removes the bold and keeps the paragraph levels.
    run(&mut s, "styles.inspectorClear", json!({"level": "characterFormatting"}));
    let r = run(&mut s, "styles.inspect", json!({}));
    assert!(r["character"]["direct"].as_array().unwrap().is_empty(), "{r}");
    assert!(!run(&mut s, "format.state", json!({}))["bold"].as_bool().unwrap());
    assert_eq!(r["paragraph"]["style"], "Heading1");
    assert!(!r["paragraph"]["direct"].as_array().unwrap().is_empty());

    run(&mut s, "styles.inspectorClear", json!({"level": "paragraphFormatting"}));
    let r = run(&mut s, "styles.inspect", json!({}));
    assert!(r["paragraph"]["direct"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["paragraph"]["style"], "Heading1");

    run(&mut s, "styles.inspectorClear", json!({"level": "paragraphStyle"}));
    assert_eq!(run(&mut s, "styles.inspect", json!({}))["paragraph"]["style"], "Normal");

    // Each clear is one undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "styles.inspect", json!({}))["paragraph"]["style"], "Heading1");
    assert_eq!(text(&s), "Chapter one");
}

#[test]
fn style_inspector_pane_and_hostile_params() {
    let mut s = Session::new(crate::sample::sample_document());
    let r = run(&mut s, "styles.inspector", json!({}));
    assert_eq!(r["pane"], true);
    assert!(s.view.style_inspector);
    assert_eq!(r["paragraph"]["styleName"], "Title");
    let r = run(&mut s, "styles.inspector", json!({"value": false}));
    assert_eq!(r["pane"], false);
    let before = text(&s);
    for junk in [json!(null), json!({}), json!({"level": 5}), json!({"level": "everything"}), json!({"level": ""})] {
        assert!(s.run("styles.inspectorClear", &junk).is_err(), "{junk}");
    }
    assert_eq!(text(&s), before);
    assert!(s.run("styles.inspector", &json!({"value": "yes"})).is_ok());
    // A selection reaching far outside the document.
    s.sel.focus = Pos::body(9999, 99999);
    s.sel.anchor = Pos::body(0, 0);
    assert!(s.run("styles.inspect", &json!({})).is_ok());
    assert!(s.run("styles.inspectorClear", &json!({"level": "characterFormatting"})).is_ok());
}

/// Home › Editing › Find › Advanced Find: every match, in the body or a selection, with Reading Highlight.
#[test]
fn advanced_find_lists_and_highlights_matches() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "cat dog cat"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Cat"}));
    // From the UI it opens the dialog.
    run(&mut s, "edit.advancedFind", json!({}));
    assert_eq!(s.ui_requests.last(), Some(&json!({"open": "find"})));
    let r = run(&mut s, "edit.advancedFind", json!({"text": "cat", "highlight": true}));
    assert_eq!(r["count"], 3);
    assert_eq!(s.find_highlights().len(), 3);
    assert_eq!(run(&mut s, "edit.advancedFind", json!({"text": "cat", "matchCase": true}))["count"], 2);
    // Highlights follow edits and the latest search.
    assert_eq!(s.find_highlights().len(), 2);
    run(&mut s, "text.insert", json!({"text": " cat"}));
    assert_eq!(s.find_highlights().len(), 3);
    // Only inside the selection.
    run(&mut s, "select.text", json!({"text": "cat dog"}));
    assert_eq!(run(&mut s, "edit.advancedFind", json!({"text": "cat", "in": "selection"}))["count"], 1);
    run(&mut s, "edit.advancedFind", json!({"highlight": false}));
    assert!(s.find_highlights().is_empty());
    // Hostile params.
    assert!(s.run("edit.advancedFind", &json!({"text": "x", "in": "elsewhere"})).is_err());
    assert!(s.run("edit.advancedFind", &json!({"text": "(", "regex": true})).is_err());
    run(&mut s, "select.all", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert!(s.run("edit.advancedFind", &json!({"text": "x", "in": "selection"})).is_err());
}

/// #146: a table style made by command styles the table at the caret (header row fill and bold
/// text, banded rows), can be modified, and saves as a table style the table refers to.
#[test]
fn custom_table_style_applies_modifies_and_saves() {
    use wordcraft_doc::{Rgb, StyleKind, TextColor};
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 2}));
    run(&mut s, "text.insert", json!({"text": "Head"}));
    run(&mut s, "table.look", json!({"headerRow": true, "bandedRows": true}));
    let (tp, _, _) = s.sel.focus.path.cell().unwrap();
    let r = run(
        &mut s,
        "table.newStyle",
        json!({"name": "Thesis Table", "basedOn": "Table Grid",
            "wholeTable": {"size": 10},
            "headerRow": {"fill": "1F3864", "bold": true, "color": "FFFFFF", "borders": true},
            "bandedRows": {"fill": "EEEEEE", "italic": true}}),
    );
    let id = r["id"].as_str().unwrap().to_string();
    let table_style = |s: &Session| s.doc.table(StoryRef::Body, &tp).unwrap().props.style.clone();
    assert_eq!(table_style(&s).as_deref(), Some(id.as_str()), "applied to the current table");
    assert!(s.run("table.newStyle", &json!({"name": "thesis table"})).is_err(), "names are unique");
    assert!(s.run("table.newStyle", &json!({"name": "X", "basedOn": "Heading 1"})).is_err(), "bases are table styles");
    assert!(s.run("table.style", &json!({"style": "Normal"})).is_err(), "only table styles apply to tables");

    let fills = |s: &mut Session, c: Rgb| {
        s.layout().pages[0].items.iter().filter(|i| matches!(i, crate::layout::Placed::Fill { color, .. } if *color == c)).count()
    };
    let head = |s: &mut Session| {
        let mut p = tp.0.clone();
        p.extend([0, 0, 0]);
        s.layout().pages[0]
            .items
            .iter()
            .find_map(
                |i| if let crate::layout::Placed::Lines { path, para, .. } = i { (path.0 == p).then(|| para.styles[0].rc.clone()) } else { None },
            )
            .unwrap()
    };
    assert_eq!(fills(&mut s, Rgb(0x1F, 0x38, 0x64)), 2, "header cells");
    assert_eq!(fills(&mut s, Rgb(0xEE, 0xEE, 0xEE)), 2, "first band");
    let rc = head(&mut s);
    assert!(rc.bold && rc.size == 10.0, "{rc:?}");
    assert_eq!(rc.color, TextColor::Rgb(Rgb::WHITE));

    // Modify the current table's style: the table follows.
    run(&mut s, "table.modifyStyle", json!({"headerRow": {"fill": "C00000", "bold": false}, "bandSize": 2}));
    assert_eq!(fills(&mut s, Rgb(0x1F, 0x38, 0x64)), 0);
    assert_eq!(fills(&mut s, Rgb(0xC0, 0, 0)), 2);
    assert_eq!(fills(&mut s, Rgb(0xEE, 0xEE, 0xEE)), 4, "two rows per band");
    assert!(!head(&mut s).bold);

    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let st = back.styles.get(&id).unwrap();
    assert_eq!((st.kind, st.name.as_str(), st.based_on.as_deref()), (StyleKind::Table, "Thesis Table", Some("TableGrid")));
    let parts = st.table.as_ref().unwrap();
    assert_eq!((parts.header_fill, parts.band_fill, parts.band_size), (Some(Rgb(0xC0, 0, 0)), Some(Rgb(0xEE, 0xEE, 0xEE)), Some(2)));
    assert_eq!(parts.col_band_size, Some(1), "a new style bands one column at a time and writes it, so Word bands it too (#3415)");
    assert_eq!((parts.header_chr.bold, parts.band_chr.italic), (Some(false), Some(true)));
    assert!(parts.header_borders.is_some_and(|b| b.any_visible()));
    let t = back.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.props.style.as_deref(), Some(id.as_str()), "w:tblStyle");
}

/// #714: Modify Table Style edits one part at a time (header row, even bands, a corner cell…)
/// with font, fill, borders and cell alignment, and every table using the style follows.
/// #1288: a cell's own Top alignment beats a table style that centres vertically, and it
/// survives a .docx round trip (`w:vAlign w:val="top"`).
#[test]
fn explicit_cell_top_beats_a_centred_table_style() {
    use wordcraft_doc::props::VAlign;
    fn text_y(s: &mut Session) -> f32 {
        s.layout().pages[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::layout::Placed::Lines { para, y, .. } if para.text_len == 4 => Some(*y),
                _ => None,
            })
            .unwrap()
    }
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    run(&mut s, "text.insert", json!({"text": "Cell"}));
    run(&mut s, "table.properties", json!({"rowHeight": 120.0, "rowHeightRule": "exact"}));
    let top = text_y(&mut s);
    run(&mut s, "table.newStyle", json!({"name": "Mid", "wholeTable": {"vAlign": "center"}}));
    let centred = text_y(&mut s);
    assert!(centred > top + 20.0, "the style centres the text: {top} {centred}");
    run(&mut s, "table.cellAlign", json!({"value": "topLeft"}));
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows[0].cells[0].props.valign, Some(VAlign::Top));
    assert!((text_y(&mut s) - top).abs() < 0.5, "the cell's Top wins over the style");
    // It saves and comes back as an explicit Top.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let t = back.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!((t.rows[0].cells[0].props.valign, t.rows[0].cells[1].props.valign), (Some(VAlign::Top), None));
    let mut back = Session::new(back);
    assert!((text_y(&mut back) - top).abs() < 0.5);
}

#[test]
fn table_style_parts_format_every_table_using_the_style() {
    use wordcraft_doc::props::{Align, BorderStyle, VAlign};
    use wordcraft_doc::styles::TablePart;
    use wordcraft_doc::{Rgb, TextColor};
    let mut s = s();
    let look = json!({"headerRow": true, "bandedRows": true, "firstColumn": true, "totalRow": false});
    run(&mut s, "insert.table", json!({"rows": 4, "cols": 3}));
    run(&mut s, "text.insert", json!({"text": "Head"}));
    run(&mut s, "table.look", look.clone());
    run(&mut s, "table.newStyle", json!({"name": "Parts", "basedOn": "Table Grid"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.table", json!({"rows": 4, "cols": 3}));
    run(&mut s, "text.insert", json!({"text": "Head"}));
    run(&mut s, "table.look", look);
    run(&mut s, "table.style", json!({"style": "Parts"}));
    let tables: Vec<_> = s.doc.body.iter().filter_map(|b| b.as_table()).collect();
    assert_eq!(tables.len(), 2);
    assert!(tables.iter().all(|t| t.props.style.as_deref() == Some("Parts")));

    run(
        &mut s,
        "table.modifyStyle",
        json!({"style": "Parts",
            "firstRow": {"bold": true, "underline": true, "color": "FFFFFF", "font": "Liberation Serif", "cellAlign": "bottomCenter",
                "borders": [{"edges": "none"}, {"edges": "bottom", "style": "double", "width": 1.5, "color": "FF0000"}]},
            "band2Horz": {"fill": "DDEEFF"},
            "nwCell": {"fill": "112233"},
            "parts": [{"part": "lastCol", "italic": true}]}),
    );
    run(&mut s, "table.modifyStyle", json!({"style": "Parts", "part": "band1Horz", "fill": "AABBCC", "vAlign": "center"}));
    let st = s.doc.styles.get("Parts").unwrap().clone();
    let head = st.table_part(TablePart::FirstRow);
    assert_eq!((head.chr.bold, head.align, head.v_align), (Some(true), Some(Align::Center), Some(VAlign::Bottom)));
    let b = head.borders.unwrap();
    assert_eq!(b.bottom.map(|x| (x.style, x.width, x.color)), Some((BorderStyle::Double, 1.5, Some(Rgb(0xFF, 0, 0)))));
    assert_eq!(b.top.map(|x| x.style), Some(BorderStyle::None), "`none` clears the other edges");
    assert_eq!(st.table_part(TablePart::LastCol).chr.italic, Some(true));
    assert_eq!(st.table_part(TablePart::Band1Horz).v_align, Some(VAlign::Center));

    let fills = |s: &mut Session, c: Rgb| {
        s.layout().pages[0].items.iter().filter(|i| matches!(i, crate::layout::Placed::Fill { color, .. } if *color == c)).count()
    };
    // Per table: the top-left corner cell; odd bands are rows 1 and 3, the even band row 2.
    assert_eq!(fills(&mut s, Rgb(0x11, 0x22, 0x33)), 2, "top left cells");
    assert_eq!(fills(&mut s, Rgb(0xAA, 0xBB, 0xCC)), 12, "odd banded rows");
    assert_eq!(fills(&mut s, Rgb(0xDD, 0xEE, 0xFF)), 6, "even banded rows");
    let rules = s
        .layout()
        .pages[0]
        .items
        .iter()
        .filter(|i| matches!(i, crate::layout::Placed::Rule { border, .. } if border.style == BorderStyle::Double && border.color == Some(Rgb(0xFF, 0, 0))))
        .count();
    assert!(rules >= 6, "the header rows' bottom edges in both tables: {rules}");
    // The header text: bold, white, underlined and centred, in both tables.
    let heads: Vec<_> = s.layout().pages[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::layout::Placed::Lines { para, .. } if para.text_len == 4 => Some((para.rp.align, para.styles[0].rc.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(heads.len(), 2, "{heads:?}");
    for (align, rc) in heads {
        assert_eq!(align, Align::Center);
        assert!(rc.bold && rc.underline != wordcraft_doc::props::Underline::None, "{rc:?}");
        assert_eq!(rc.color, TextColor::Rgb(Rgb::WHITE));
    }
    // It saves: the parts come back.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let got = back.styles.get("Parts").unwrap();
    for part in TablePart::ALL {
        assert_eq!(got.table_part(part), st.table_part(part), "{}", part.ooxml());
    }

    // Hostile params fail cleanly and change nothing.
    for bad in [
        json!({"style": "Parts", "part": 5}),
        json!({"style": "Parts", "part": "diagonal"}),
        json!({"style": "Parts", "parts": "x"}),
        json!({"style": "Parts", "parts": [{"fill": "FF0000"}]}),
        json!({"style": "Parts", "firstRow": {"borders": {"edges": "diagonal"}}}),
        json!({"style": "Parts", "firstRow": {"borders": {"style": "zigzag"}}}),
        json!({"style": "Parts", "firstRow": {"borders": [1, 2]}}),
        json!({"style": "Parts", "firstRow": {"cellAlign": 3}}),
        json!({"style": "Parts", "firstRow": {"cellAlign": "middleish"}}),
        json!({"style": "Parts", "firstRow": {"align": "up"}}),
        json!({"style": "Parts", "firstRow": {"underline": "zigzag"}}),
        json!({"style": "Parts", "firstRow": "bold"}),
        // Part fields without `part` (#1288).
        json!({"style": "Parts", "fill": "FF0000"}),
        json!({"style": "Parts", "bold": true}),
        json!({"style": "Parts", "borders": false}),
        json!({"style": "Parts", "firstRow": {"font": 5}}),
    ] {
        assert!(s.run("table.modifyStyle", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.doc.styles.get("Parts"), Some(&st));
    // Huge and odd numbers are clamped.
    run(&mut s, "table.modifyStyle", json!({"style": "Parts", "firstRow": {"size": 1e30, "borders": {"edges": "top", "width": -5}}}));
    let head = s.doc.styles.get("Parts").unwrap().table_part(TablePart::FirstRow);
    assert_eq!((head.chr.size, head.borders.and_then(|b| b.top).map(|b| b.width)), (Some(1638.0), Some(0.25)));
    // A null or empty font makes the part's font inherited again (#1288).
    assert_eq!(head.chr.font.as_deref(), Some("Liberation Serif"));
    run(&mut s, "table.modifyStyle", json!({"style": "Parts", "firstRow": {"font": null}}));
    assert_eq!(s.doc.styles.get("Parts").unwrap().table_part(TablePart::FirstRow).chr.font, None);
    run(&mut s, "table.modifyStyle", json!({"style": "Parts", "part": "lastCol", "font": "Liberation Mono"}));
    run(&mut s, "table.modifyStyle", json!({"style": "Parts", "part": "lastCol", "font": " "}));
    assert_eq!(s.doc.styles.get("Parts").unwrap().table_part(TablePart::LastCol).chr.font, None);
}

#[test]
fn page_and_table_gridlines_toggle_independently() {
    // #69: View › Gridlines (the page drawing grid) and Table Layout › View Gridlines (table cell
    // outlines) are separate view switches; neither touches the document or the undo stack.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let undo = s.undo_labels();
    // Table cell outlines are on by default, as in Word; the page grid is off.
    assert!(!s.view.gridlines && s.view.table_gridlines);
    assert_eq!(run(&mut s, "table.viewGridlines", json!({"value": false}))["value"], false);

    assert_eq!(run(&mut s, "view.gridlines", json!({}))["value"], true);
    assert!(s.view.gridlines && !s.view.table_gridlines);

    // Works with the caret outside a table.
    assert_eq!(run(&mut s, "table.viewGridlines", json!({}))["value"], true);
    assert!(s.view.gridlines && s.view.table_gridlines);

    assert_eq!(run(&mut s, "view.gridlines", json!({"value": false}))["value"], false);
    assert!(!s.view.gridlines && s.view.table_gridlines);
    assert_eq!(run(&mut s, "view.gridlines", json!({"value": false}))["value"], false);
    assert!(!s.view.gridlines);

    assert_eq!(run(&mut s, "table.viewGridlines", json!({}))["value"], false);
    assert!(!s.view.gridlines && !s.view.table_gridlines);

    assert_eq!(text(&s), "Hello");
    assert_eq!(s.undo_labels(), undo);
    let state = run(&mut s, "view.state", json!({}));
    assert_eq!(state["gridlines"], false);
    assert_eq!(state["tableGridlines"], false);
}

#[test]
fn focus_and_full_screen_replace_each_other() {
    // #1678: Focus and Full Screen View are separate screen states; one on turns the other off.
    // Both are registry commands, so Customize Keyboard can bind them.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let undo = s.undo_labels();
    assert!(s.registry.get("view.fullScreenView").is_some() && s.registry.get("view.focus").is_some());

    assert_eq!(run(&mut s, "view.focus", json!({}))["value"], true);
    assert!(s.view.focus_mode && !s.view.full_screen_view);
    assert_eq!(run(&mut s, "view.fullScreenView", json!({}))["value"], true);
    assert!(s.view.full_screen_view && !s.view.focus_mode);
    assert_eq!(run(&mut s, "view.focus", json!({"value": true}))["value"], true);
    assert!(s.view.focus_mode && !s.view.full_screen_view);
    assert_eq!(run(&mut s, "view.focus", json!({}))["value"], false);
    assert!(!s.view.focus_mode && !s.view.full_screen_view);
    assert_eq!(run(&mut s, "view.fullScreenView", json!({"value": false}))["value"], false);
    assert!(!s.view.full_screen_view);
    // Full Screen (#980) is the window alone: it leaves Focus be.
    run(&mut s, "view.focus", json!({"value": true}));
    run(&mut s, "view.fullScreen", json!({"value": true}));
    assert!(s.view.focus_mode && s.view.full_screen);
    run(&mut s, "view.focus", json!({"value": false}));
    run(&mut s, "view.fullScreen", json!({"value": false}));

    // Immersive Reader is a reading view, not Focus.
    run(&mut s, "view.immersive", json!({}));
    assert!(s.view.read_mode && !s.view.focus_mode);

    assert_eq!(text(&s), "Hello");
    assert_eq!(s.undo_labels(), undo);
}

/// Issue #768: zooming by percent after One Page or Page Width keeps one page per row.
#[test]
fn zoom_after_one_page_keeps_single_column() {
    let mut s = s();
    run(&mut s, "view.onePage", json!({}));
    assert!(s.view.single_column);
    run(&mut s, "view.zoomOut", json!({}));
    assert!(s.view.fit.is_empty() && s.view.single_column);
    run(&mut s, "view.zoom", json!({"value": 50}));
    assert!(s.view.single_column);
    run(&mut s, "view.multiplePages", json!({}));
    assert!(!s.view.single_column);
    run(&mut s, "view.zoomIn", json!({}));
    assert!(!s.view.single_column);
    run(&mut s, "view.pageWidth", json!({}));
    run(&mut s, "view.sideToSide", json!({}));
    assert!(!s.view.single_column);
}

/// Issue #995: One Page is saved with the document (`w:zoom`, like Word) and restored when it is
/// reopened; changing the zoom alone doesn't make the document unsaved.
#[test]
fn one_page_is_restored_when_the_document_is_reopened() {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-zoom-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("one-page.docx");
    let path = path.to_string_lossy();
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "file.save", json!({"path": path}));
    run(&mut s, "view.onePage", json!({}));
    assert!(!s.dirty, "a zoom isn't an edit");
    run(&mut s, "file.save", json!({}));

    let mut other = Session::new(wordcraft_doc::Document::new());
    run(&mut other, "file.open", json!({"path": path}));
    assert_eq!(other.view.fit, "onePage");
    assert!(other.view.single_column && !other.view.multi_page);
    assert!(!other.dirty);

    // A percentage is restored as a percentage, the fit mode dropped.
    run(&mut other, "view.zoom", json!({"value": 80}));
    run(&mut other, "file.save", json!({}));
    // Opening an open file only shows it: close it first to read it again.
    run(&mut s, "file.closeDocument", json!({}));
    run(&mut s, "file.open", json!({"path": path}));
    assert!(s.view.fit.is_empty() && !s.view.single_column);
    assert!((s.view.zoom - 0.8).abs() < 1e-6);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn navigation_pane_opens_without_changing_view_mode() {
    // Reading Mode's heading navigation is the Navigation pane; Outline is an editing view (#493).
    let mut s = s();
    for command in ["view.printLayout", "view.webLayout", "view.readMode"] {
        run(&mut s, command, json!({}));
        let before = (s.view.mode, s.view.read_mode);
        s.view.nav_pane = false;
        run(&mut s, "view.navigationPane", json!({"value": true}));
        assert!(s.view.nav_pane);
        assert_eq!((s.view.mode, s.view.read_mode), before, "{command}");
    }
}

#[test]
fn interface_theme_queues_a_front_end_request_and_rejects_junk() {
    // #1861: the session can't paint, so the command asks the front end; the document,
    // view state and undo stack are untouched.
    let mut s = s();
    let dirty = s.dirty;
    for value in ["light", "dark", "system"] {
        let r = run(&mut s, "view.interfaceTheme", json!({"value": value}));
        assert_eq!(r["value"], value);
        assert_eq!(s.ui_requests.last().unwrap()["interfaceTheme"], value);
    }
    let r = run(&mut s, "view.interfaceTheme", json!({}));
    assert_eq!(r["value"], "toggle");
    assert_eq!(s.ui_requests.last().unwrap()["toggle"], "interfaceTheme");
    assert_eq!(s.dirty, dirty);
    for bad in [json!({"value": "midnight"}), json!({"value": 3})] {
        assert!(s.run("view.interfaceTheme", &bad).is_err(), "{bad}");
    }
}

#[test]
fn reading_mode_reflows_per_viewport_without_changing_document_or_export() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "A long paragraph for reading. ".repeat(200)}));
    let original = s.doc.clone();
    let print = s.export_layout();
    let dirty = s.dirty;
    run(&mut s, "view.readMode", json!({}));
    assert!(s.view.read_mode);
    s.view.read_width = 240.0;
    s.view.read_height = 420.0;
    let columns = s.layout();
    assert!(columns.pages.len() > print.pages.len());
    assert!(columns.pages.iter().all(|p| p.w == 240.0 && p.h == 420.0));
    s.view.read_height = 260.0;
    assert!(s.layout().pages.len() > columns.pages.len(), "height is part of the layout cache key");
    s.view.read_width = 300.0;
    assert_eq!(s.layout().pages[0].w, 300.0, "width is part of the layout cache key");
    let serialized = serde_json::to_value(&s.view).unwrap();
    assert!(serialized.get("readWidth").is_none() && serialized.get("readHeight").is_none());
    let restored: crate::ViewState = serde_json::from_value(serialized).unwrap();
    assert_eq!((restored.read_width, restored.read_height), (300.0, 500.0));
    run(&mut s, "view.readColumnWidth", json!({"value": "page"}));
    run(&mut s, "view.readTextSize", json!({"value": 125}));
    let default_spacing_pages = s.layout().pages.len();
    run(&mut s, "view.readLineSpacing", json!({"value": 2.0}));
    assert_eq!(s.view.read_column_width, Some(612.0));
    assert_eq!(s.view.read_zoom, 1.25);
    assert_eq!(s.view.read_line_spacing, Some(2.0));
    let spaced = s.layout();
    assert!(spaced.pages.len() > default_spacing_pages, "line spacing must invalidate cached pagination");
    assert!(s.run("view.readTextSize", &json!({"value": -10})).is_err());
    assert!(s.run("view.readLineSpacing", &json!({"value": 0.01})).is_err());
    assert!(s.run("view.readColumnWidth", &json!({"value": "giant"})).is_err());
    let serialized = serde_json::to_value(&s.view).unwrap();
    for key in ["readColumnWidth", "readZoom", "readLineSpacing"] {
        assert!(serialized.get(key).is_none());
    }
    run(&mut s, "view.printLayout", json!({}));
    assert!(!s.view.read_mode);
    assert_eq!(s.layout().pages[0].w, print.pages[0].w);
    assert_eq!(s.export_layout().pages.len(), print.pages.len());
    assert_eq!(s.doc, original);
    assert_eq!(s.dirty, dirty);
    run(&mut s, "view.readMode", json!({}));
    assert_eq!(s.view.read_column_width, None);
    assert_eq!(s.view.read_zoom, 1.0);
    assert_eq!(s.view.read_line_spacing, None);
}

/// Read Mode › View › Page Color: None, Sepia or Inverse, a view setting that leaves the
/// document alone, is not saved, and starts at None on each entry to Read Mode.
#[test]
fn reading_page_color_is_a_transient_view_setting() {
    use crate::cmd::view::ReadPageColor;
    let mut s = s();
    let original = s.doc.clone();
    let dirty = s.dirty;
    run(&mut s, "view.readMode", json!({}));
    assert_eq!(s.view.read_page_color, ReadPageColor::None);
    assert_eq!(s.run("view.readPageColor", &json!({"value": "sepia"})).unwrap(), json!({"pageColor": "sepia"}));
    assert_eq!(s.view.read_page_color, ReadPageColor::Sepia);
    run(&mut s, "view.readPageColor", json!({"value": "inverse"}));
    assert_eq!(s.view.read_page_color, ReadPageColor::Inverse);
    for bad in [json!({"value": "black"}), json!({"value": 1}), json!({})] {
        assert!(s.run("view.readPageColor", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.view.read_page_color, ReadPageColor::Inverse, "rejected values keep the color");
    assert!(serde_json::to_value(&s.view).unwrap().get("readPageColor").is_none());
    assert_eq!(s.doc, original);
    assert_eq!(s.dirty, dirty);
    run(&mut s, "view.printLayout", json!({}));
    run(&mut s, "view.readMode", json!({}));
    assert_eq!(s.view.read_page_color, ReadPageColor::None);
}

/// File › Options › Advanced › Numeral: the display mode for digits (display only).
#[test]
fn numeral_mode_sets_gets_and_rejects_junk() {
    use wordcraft_layout::NumeralMode;
    let mut s = s();
    assert_eq!(s.view.numeral, NumeralMode::Arabic, "Western digits by default");
    let r = run(&mut s, "view.numeral", json!({"value": "hindi"}));
    assert_eq!(r["value"], "hindi");
    assert_eq!(s.view.numeral, NumeralMode::Hindi);
    let r = run(&mut s, "view.numeral", json!({}));
    assert_eq!(r["value"], "hindi", "no params reads the current mode");
    run(&mut s, "view.numeral", json!({"value": "Context"}));
    assert_eq!(s.view.numeral, NumeralMode::Context, "mode names ignore case");
    assert!(s.run("view.numeral", &json!({"value": "western"})).is_err());
    assert!(s.run("view.numeral", &json!({"value": 3})).is_err());
    assert!(s.run("view.numeral", &json!({"value": ["hindi"]})).is_err());
    assert_eq!(s.view.numeral, NumeralMode::Context, "rejected values keep the mode");
    // Switching modes relayouts without disturbing the text.
    s.doc = wordcraft_doc::Document::from_text("Year 2024");
    run(&mut s, "view.numeral", json!({"value": "hindi"}));
    let _l = s.layout();
    assert_eq!(text(&s), "Year 2024");
}

/// The Numeral setting also applies to print and PDF output, not just the screen layout.
#[test]
fn export_layout_follows_the_numeral_setting() {
    use wordcraft_layout::Placed;
    fn glyph_ids(s: &Session) -> Vec<u32> {
        let mut out = Vec::new();
        for page in &s.export_layout().pages {
            for it in &page.items {
                if let Placed::Lines { para, .. } = it {
                    out.extend(para.glyphs.iter().map(|g| g.gid));
                }
            }
        }
        out
    }
    let mut s = s();
    s.doc = wordcraft_doc::Document::from_text("Year 2024");
    let western = glyph_ids(&s);
    assert!(!western.is_empty());
    run(&mut s, "view.numeral", json!({"value": "hindi"}));
    assert_ne!(glyph_ids(&s), western, "Hindi digits reach the printed layout");
}

/// File › Export PDF, Save As PDF and page images take the Numeral setting too.
#[test]
fn pdf_and_png_copies_follow_the_numeral_setting() {
    let mut s = s();
    s.doc = wordcraft_doc::Document::from_text("Year 2024");
    // A fixed creation date, so the PDF bytes depend only on the layout.
    s.doc.core.created = "2024-01-01T00:00:00Z".into();
    let copy = |s: &Session, name: &str| crate::io::save_bytes_opts(name, &s.doc, &cmd::file::export_options(s, &json!({}))).unwrap();
    let (pdf, png) = (copy(&s, "x.pdf"), copy(&s, "x.png"));
    assert_eq!(copy(&s, "x.pdf"), pdf);
    run(&mut s, "view.numeral", json!({"value": "hindi"}));
    assert_eq!(cmd::file::export_options(&s, &json!({})).numeral, wordcraft_layout::NumeralMode::Hindi);
    assert_ne!(copy(&s, "x.pdf"), pdf, "Hindi digits reach the PDF");
    assert_ne!(copy(&s, "x.png"), png, "Hindi digits reach the page image");
}

/// Issue #67: View › Zoom steps. Zoom In/Out leave a fit mode and step 10% from the current zoom
/// (the UI keeps `view.zoom` equal to the shown zoom while a fit mode is on), and never get stuck
/// short of the 10%–500% limits.
#[test]
fn zoom_in_and_out_step_from_current_zoom_and_leave_fit_modes() {
    let mut s = s();
    let pct = |s: &Session| (s.view.zoom * 100.0).round() as i32;
    run(&mut s, "view.zoomIn", json!({}));
    assert_eq!(pct(&s), 110);
    run(&mut s, "view.zoomOut", json!({}));
    run(&mut s, "view.zoomOut", json!({}));
    assert_eq!(pct(&s), 90);
    for fit in ["view.pageWidth", "view.onePage", "view.multiplePages"] {
        run(&mut s, fit, json!({}));
        assert!(!s.view.fit.is_empty());
        // What the canvas reports while a fit mode shows the page at 163%.
        s.view.zoom = 1.63;
        run(&mut s, "view.zoomIn", json!({}));
        assert_eq!((pct(&s), s.view.fit.as_str(), s.view.multi_page), (170, "", false), "{fit}");
        run(&mut s, fit, json!({}));
        s.view.zoom = 1.63;
        run(&mut s, "view.zoomOut", json!({}));
        assert_eq!((pct(&s), s.view.fit.as_str()), (150, ""), "{fit}");
    }
    let mut last = pct(&s);
    for _ in 0..60 {
        run(&mut s, "view.zoomIn", json!({}));
        assert!(pct(&s) > last || pct(&s) == 500);
        last = pct(&s);
    }
    assert_eq!(last, 500);
    for _ in 0..60 {
        run(&mut s, "view.zoomOut", json!({}));
        assert!(pct(&s) < last || pct(&s) == 10);
        last = pct(&s);
    }
    assert_eq!(last, 10);
    run(&mut s, "view.zoom100", json!({}));
    assert_eq!(pct(&s), 100);
}

/// Comments, revisions and dates read the real clock (#188: the web stamped 2026-01-01).
#[test]
fn timestamps_come_from_the_clock() {
    let before = cmd::now_unix();
    assert!(before > 1_760_000_000, "{before}");
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Dated"}));
    run(&mut s, "select.text", json!({"text": "Dated"}));
    run(&mut s, "review.newComment", json!({"text": "When?"}));
    let date = run(&mut s, "review.comments", json!({}))[0]["date"].as_str().unwrap_or_default().to_string();
    // Fixed-width ISO strings sort by time.
    assert!(date >= cmd::iso_from_unix_secs(before) && date <= cmd::iso_from_unix_secs(cmd::now_unix()), "{date}");
}

#[test]
fn charts_and_diagrams_can_be_selected_and_deleted_but_not_moved() {
    use std::sync::Arc;
    use wordcraft_doc::para::InlineObject;
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Before "}));
    let chart = InlineObject::Graphic { w: 200.0, h: 100.0, alt: String::new(), float: Default::default(), graphic: Arc::new(Default::default()) };
    s.doc.insert_object(&Pos::body(0, 7), chart, &Default::default()).unwrap();
    let end = 7 + wordcraft_doc::para::OBJ.len_utf8();
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 7), "focus": Pos::body(0, end)}));
    assert!(crate::cmd::objects::object_selection(&s).is_some(), "the chart is selected");
    for (id, v) in [
        ("arrange.bounds", json!({"width": 50})),
        ("picture.size", json!({"width": 50})),
        ("arrange.wrap", json!({"wrap": "square"})),
        ("arrange.wrapText", json!({"side": "left"})),
        ("arrange.position", json!({"preset": "topLeft"})),
        ("arrange.align", json!({"value": "left"})),
        ("arrange.bringForward", json!({})),
    ] {
        assert!(s.run(id, &v).is_err(), "{id} must not change a chart");
    }
    run(&mut s, "text.delete", json!({}));
    assert!(s.doc.para_at(&Pos::body(0, 0)).is_some_and(|p| p.objects.is_empty()), "the chart is deleted");
}

/// The chart model of the first object in the body's first paragraph.
fn first_chart(doc: &wordcraft_doc::Document) -> Option<wordcraft_doc::chart::ChartSpec> {
    use wordcraft_doc::para::InlineObject;
    let p = doc.para_at(&Pos::body(0, 0))?;
    p.object_offsets().into_iter().find_map(|off| match p.object_at(off) {
        Some(InlineObject::Graphic { graphic, .. }) => graphic.chart.as_deref().cloned(),
        _ => None,
    })
}

#[test]
fn inserted_chart_is_saved_and_reopened_with_its_data() {
    let mut s = s();
    let data =
        json!({"categories": ["Jan", "Feb", "Mar"], "series": [{"name": "Rain", "values": [12.5, null, 3]}, {"name": "Sun", "values": [4, 5, 6]}]});
    let r = run(&mut s, "insert.chart", json!({"type": "line", "data": data, "title": "Weather", "legend": "right"}));
    assert_eq!(r["chart"]["type"], "line");
    let spec = first_chart(&s.doc).expect("a chart with a model");
    assert_eq!(spec.series[0].values, vec![Some(12.5), None, Some(3.0)]);
    assert!(crate::cmd::chart::selected_chart(&s).is_some(), "the new chart is selected");
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    assert!(bytes.windows(22).any(|w| w == b"word/charts/chart1.xml"), "a chart part is written");
    let back = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(first_chart(&back), Some(spec), "the chart reads back as the same editable chart");
    // Sample data when none is given.
    let mut s2 = self::s();
    run(&mut s2, "insert.chart", json!({"type": "pie"}));
    assert!(first_chart(&s2.doc).is_some_and(|c| c.kind == wordcraft_doc::chart::ChartType::Pie && !c.series.is_empty()));
    // No params: the picker, nothing inserted.
    let mut s3 = self::s();
    run(&mut s3, "insert.chart", json!({}));
    assert!(s3.ui_requests.iter().any(|r| r["open"] == "insertChart"));
    assert!(first_chart(&s3.doc).is_none());
}

#[test]
fn chart_edits_are_one_undo_step_each() {
    use wordcraft_doc::chart::{ChartType, LegendPos};
    let mut s = s();
    run(&mut s, "insert.chart", json!({"type": "column"}));
    let before = first_chart(&s.doc).unwrap();
    run(&mut s, "chart.editData", json!({"data": {"categories": ["A", "B"], "series": [{"name": "X", "values": [1, 2]}]}}));
    run(&mut s, "chart.type", json!({"type": "barStacked"}));
    run(&mut s, "chart.title", json!({"text": "Totals"}));
    run(&mut s, "chart.legend", json!({"pos": "none"}));
    run(&mut s, "chart.dataLabels", json!({"on": true}));
    let c = first_chart(&s.doc).unwrap();
    assert_eq!((c.kind, c.title.as_deref(), c.legend, c.data_labels), (ChartType::BarStacked, Some("Totals"), None, true));
    assert_eq!((c.categories.len(), c.series.len()), (2, 1));
    let steps: [&dyn Fn(&wordcraft_doc::chart::ChartSpec) -> bool; 5] = [
        &|c| !c.data_labels,
        &|c| c.legend == Some(LegendPos::Bottom),
        &|c| c.title.is_none(),
        &|c| c.kind == ChartType::Column && c.series.len() == 1,
        &|c| *c == before,
    ];
    for (k, check) in steps.iter().enumerate() {
        run(&mut s, "edit.undo", json!({}));
        assert!(check(&first_chart(&s.doc).unwrap()), "undo step {k}");
    }
    // Resizing draws the chart again at its new size.
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "picture.size", json!({"width": 300, "height": 150, "lockAspect": false}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let Some(wordcraft_doc::para::InlineObject::Graphic { graphic, w, .. }) = p.object_offsets().into_iter().find_map(|o| p.object_at(o)) else {
        panic!()
    };
    assert_eq!((*w, graphic.w, graphic.h), (300.0, 300.0, 150.0));
}

#[test]
fn hostile_chart_data_is_capped() {
    use wordcraft_doc::chart::{MAX_CELLS, MAX_POINTS, MAX_SERIES, MAX_TEXT};
    let mut s = s();
    let values: Vec<serde_json::Value> = (0..5000).map(|i| if i % 2 == 0 { json!("NaN") } else { json!(1e308) }).collect();
    let series: Vec<serde_json::Value> = (0..300).map(|i| json!({"name": "n".repeat(10_000), "values": values, "x": i})).collect();
    let cats: Vec<serde_json::Value> = (0..6000).map(|i| json!(i)).collect();
    run(
        &mut s,
        "insert.chart",
        json!({"type": "scatter", "data": {"categories": cats, "series": series}, "title": "\u{0}".repeat(9999), "width": 1e30}),
    );
    let c = first_chart(&s.doc).unwrap();
    assert!(c.series.len() <= MAX_SERIES && c.points() <= MAX_POINTS && c.series.len() * c.points() <= MAX_CELLS);
    assert!(c.series.iter().all(|x| x.name.chars().count() <= MAX_TEXT && x.values.len() == c.points()));
    assert!(c.series[0].values.iter().all(|v| v.is_none_or(f64::is_finite)));
    let _ = s.layout();
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(first_chart(&back), Some(c));
    for bad in [json!({"type": "radar"}), json!({"data": {"series": 5}}), json!({"data": {"series": []}})] {
        assert!(s.run("insert.chart", &bad).is_err(), "{bad}");
    }
    // A chart without a model (from another program) is shown, not edited.
    let mut s = self::s();
    let foreign =
        wordcraft_doc::para::InlineObject::Graphic { w: 200.0, h: 100.0, alt: String::new(), float: Default::default(), graphic: Default::default() };
    s.doc.insert_object(&Pos::body(0, 0), foreign, &Default::default()).unwrap();
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, wordcraft_doc::para::OBJ.len_utf8())}));
    assert!(s.run("chart.type", &json!({"type": "pie"})).is_err());
}

#[test]
fn accessibility_reports_a_chart_without_alt_text() {
    use std::sync::Arc;
    use wordcraft_doc::para::InlineObject;
    let mut s = s();
    let chart = InlineObject::Graphic { w: 200.0, h: 100.0, alt: " ".into(), float: Default::default(), graphic: Arc::new(Default::default()) };
    s.doc.insert_object(&Pos::body(0, 0), chart, &Default::default()).unwrap();
    let v = run(&mut s, "file.accessibility", json!({}));
    let issues = v["issues"].as_array().expect("issues");
    assert!(issues.iter().any(|i| i["issue"] == "Chart or diagram has no alternative text"), "{issues:?}");
}

/// #320: the Tabs dialog's command sets and clears stops (inherited ones too), changes the
/// default interval, and is one undo step.
#[test]
fn para_tabs_set_clear_and_undo() {
    use wordcraft_doc::props::{TabAlign, TabLeader};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Name\tPrice"}));
    run(&mut s, "para.style", json!({"style": "Header"}));
    let stops = |s: &Session| {
        let p = s.doc.para_at(&s.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
        s.doc.styles.resolve_para(&p).tabs.iter().map(|t| (t.pos, t.align, t.leader)).collect::<Vec<_>>()
    };
    assert_eq!(stops(&s).len(), 2, "Header's own stops");
    run(
        &mut s,
        "para.tabs",
        json!({"set": [{"pos": 100.0, "align": "decimal", "leader": "dot"}, {"pos": 300.0}], "clear": [234.0], "default": 18.0}),
    );
    assert_eq!(
        stops(&s),
        vec![(100.0, TabAlign::Decimal, TabLeader::Dot), (300.0, TabAlign::Left, TabLeader::None), (468.0, TabAlign::Right, TabLeader::None)]
    );
    assert_eq!(s.doc.settings.default_tab, 18.0);
    // Setting a stop at an existing position replaces it.
    run(&mut s, "para.tabs", json!({"set": [{"pos": 300.0, "align": "bar"}]}));
    assert!(stops(&s).contains(&(300.0, TabAlign::Bar, TabLeader::None)));
    run(&mut s, "para.tabs", json!({"clearAll": true}));
    assert!(stops(&s).is_empty(), "Clear All removes inherited stops too");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(stops(&s).len(), 3);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(stops(&s).len(), 2);
    assert_eq!(s.doc.settings.default_tab, 36.0, "one undo step restores stops and default together");
    assert!(s.run("para.tabs", &json!({"set": [{"pos": f64::MAX}]})).is_err());
    assert!(s.run("para.tabs", &json!({"clear": "x"})).is_err());
}

/// #320: changing only the default tab interval (a document setting) is its own undo step,
/// and redo puts it back.
#[test]
fn para_tabs_default_only_undoes() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "a\tb"}));
    let before = s.doc.settings.default_tab;
    run(&mut s, "para.tabs", json!({"default": 72.0}));
    assert_eq!(s.doc.settings.default_tab, 72.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.settings.default_tab, before, "undo restores the default interval");
    assert!(text(&s).contains("a\tb"), "only the setting was undone");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc.settings.default_tab, 72.0, "redo sets it again");
}

/// #320: exact border sides with a style, for paragraphs and page borders.
#[test]
fn border_sides_apply_exactly() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Boxed"}));
    let side = json!({"style": "double", "width": 1.5, "color": "C00000"});
    run(&mut s, "para.borders", json!({"sides": {"top": side, "bottom": side, "left": side, "right": side}}));
    let b = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.borders).unwrap();
    for x in [b.top, b.bottom, b.left, b.right] {
        let x = x.unwrap();
        assert_eq!((x.style, x.width, x.color), (BorderStyle::Double, 1.5, Some(wordcraft_doc::props::Rgb(0xC0, 0, 0))));
    }
    assert!(b.between.is_none());
    run(&mut s, "para.borders", json!({"sides": {}}));
    assert!(!s.doc.para_at(&s.sel.focus).and_then(|p| p.props.borders).unwrap().any_visible());
    run(&mut s, "design.pageBorders", json!({"sides": {"top": {"style": "dashed", "width": 3.0}}, "applyTo": "document"}));
    let pb = s.doc.last_section.page_borders.unwrap();
    assert_eq!(pb.top.map(|t| (t.style, t.width)), Some((BorderStyle::Dashed, 3.0)));
    assert!(pb.left.is_none());
    run(&mut s, "design.pageBorders", json!({"kind": "box", "style": "thick"}));
    assert_eq!(s.doc.last_section.page_borders.unwrap().left.map(|t| t.style), Some(BorderStyle::Thick));
    run(&mut s, "design.pageBorders", json!({"sides": {}}));
    assert!(s.doc.last_section.page_borders.is_none());
}

/// Encrypt with Password (#55): a document saved with a password opens only with it, keeps it
/// for the next save, and saves unencrypted once the password is removed.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn password_protected_documents_round_trip() {
    let dir = scratch_dir("password");
    let path = dir.join("secret.docx");
    let path_str = path.to_string_lossy().to_string();
    let mut a = s();
    run(&mut a, "document.setText", json!({"text": "Quarterly numbers"}));
    let r = run(&mut a, "file.save", json!({"path": path_str, "password": "hunter2"}));
    assert_eq!(r["encrypted"], true);
    assert!(wordcraft_docx::is_encrypted(&std::fs::read(&path).unwrap()));
    assert!(a.run("file.save", &json!({"path": dir.join("x.txt").to_string_lossy(), "password": "pw"})).is_err(), "only Word packages encrypt");

    // No password, then a wrong one: clear errors, and the open document stays.
    let mut t = s();
    let e = t.run("file.open", &json!({"path": path_str})).unwrap_err().to_string();
    assert!(crate::io::needs_password(&e), "{e}");
    let e = t.run("file.open", &json!({"path": path_str, "password": "hunter3"})).unwrap_err().to_string();
    assert!(crate::io::wrong_password(&e), "{e}");
    assert!(t.path.is_none());

    let r = run(&mut t, "file.open", json!({"path": path_str, "password": "hunter2"}));
    assert_eq!(r["encrypted"], true);
    assert_eq!(text(&t), "Quarterly numbers");
    assert_eq!(run(&mut t, "file.info", json!({}))["encrypted"], true);
    // Saving again keeps the password, as Word does.
    run(&mut t, "text.insert", json!({"text": "Q3 "}));
    run(&mut t, "file.save", json!({}));
    let (doc, encrypted) = crate::io::open_path_with(&path, Some("hunter2")).unwrap();
    assert!(encrypted && doc.plain_text(StoryRef::Body).contains("Q3"));

    // Removing the password saves the document unencrypted.
    assert!(t.run("file.encrypt", &json!({})).is_err());
    assert!(t.run("file.encrypt", &json!({"password": "x".repeat(300)})).is_err());
    t.dirty = false;
    assert_eq!(run(&mut t, "file.encrypt", json!({"password": null}))["encrypted"], false);
    assert!(t.dirty, "removing the password is a change to save");
    run(&mut t, "file.save", json!({}));
    assert!(crate::io::open_path(&path).unwrap().plain_text(StoryRef::Body).contains("Quarterly"));
    // A password never shows in debug output.
    assert_eq!(format!("{:?}", crate::io::Password::new("hunter2")), "Password(***)");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Shape Format › Shape Effects (#275): presets and custom values apply to the selected shape,
/// an omitted key is kept, hostile numbers are clamped and each change is one undo step.
#[test]
fn shape_effects_apply_and_undo() {
    use wordcraft_doc::effects::{MAX_BLUR, MAX_DISTANCE, Shadow};
    let mut s = s();
    run(&mut s, "insert.shape", json!({"kind": "ellipse"}));
    let effects = |s: &Session| match cmd::objects::selected(s) {
        Some((_, wordcraft_doc::InlineObject::Shape { effects, float, .. })) => (effects, float.effect),
        o => panic!("{o:?}"),
    };
    assert!(effects(&s).0.is_empty());
    run(&mut s, "shape.effects", json!({"shadow": "offsetBottomRight", "glow": {"color": "FF0000", "size": 8}, "softEdge": 5}));
    let (e, extent) = effects(&s);
    assert_eq!(e.shadow, Shadow::preset("offsetBottomRight"));
    assert_eq!(e.glow.map(|g| (g.color, g.size)), Some((wordcraft_doc::props::Rgb(255, 0, 0), 8.0)));
    assert_eq!(e.soft_edge, Some(5.0));
    assert!(extent.iter().all(|v| *v >= 8.0), "room for the glow: {extent:?}");
    // Clearing the glow leaves the shadow and soft edges.
    run(&mut s, "shape.effects", json!({"glow": null}));
    let (e, _) = effects(&s);
    assert!(e.glow.is_none() && e.shadow.is_some() && e.soft_edge == Some(5.0));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(effects(&s).0.glow.map(|g| g.size), Some(8.0));
    // Hostile numbers.
    run(&mut s, "shape.effects", json!({"shadow": {"blur": 1e30, "distance": -5, "angle": 1e20, "transparency": 400}, "softEdge": -3}));
    let (e, _) = effects(&s);
    let sh = e.shadow.unwrap();
    assert!(sh.blur == MAX_BLUR && sh.distance == 0.0 && (0.0..360.0).contains(&sh.angle) && sh.transparency == 100.0, "{sh:?}");
    assert!(e.soft_edge.is_none());
    run(&mut s, "shape.effects", json!({"shadow": {"distance": 1e9}}));
    assert_eq!(effects(&s).0.shadow.map(|s| s.distance), Some(MAX_DISTANCE));
    assert!(s.run("shape.effects", &json!({"shadow": "perspective"})).is_err());
    // Off a shape it's disabled.
    run(&mut s, "select.collapse", json!({"end": true}));
    run(&mut s, "text.insert", json!({"text": "x"}));
    assert!(s.run("shape.effects", &json!({"shadow": null})).is_err());
}

#[test]
fn asian_typography_sets_flags_on_selected_paragraphs_and_undoes() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "一\n二\n三"}));
    run(&mut s, "select.all", json!({}));
    let r = run(&mut s, "para.asianTypography", json!({"kinsoku": false, "wordWrap": false, "topLinePunct": true}));
    assert_eq!(r["asianTypography"]["kinsoku"], false);
    assert_eq!(r["asianTypography"]["autoSpaceDE"], true, "a flag left out keeps its default");
    for p in s.doc.body.iter().filter_map(|b| b.as_para()) {
        assert_eq!(
            (p.props.kinsoku, p.props.word_wrap, p.props.top_line_punct, p.props.overflow_punct),
            (Some(false), Some(false), Some(true), None)
        );
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.props.kinsoku.is_none() && p.props.word_wrap.is_none()));
    // Without flags it only reports them.
    let r = run(&mut s, "para.asianTypography", json!({}));
    assert_eq!(r["asianTypography"]["kinsoku"], true);
}

#[test]
fn manage_styles_lists_paragraph_character_linked_and_table_styles() {
    let mut s = s();
    let list = run(&mut s, "styles.manage", json!({"sort": "alphabetical"}));
    let list = list.as_array().unwrap();
    let find = |name: &str| list.iter().find(|x| x["name"] == name).unwrap_or_else(|| panic!("{name} listed"));
    assert_eq!(find("Normal")["type"], "paragraph");
    assert_eq!(find("Normal")["builtIn"], true);
    assert_eq!(find("Normal")["inGallery"], true);
    assert_eq!(find("Heading 1")["type"], "linked");
    assert_eq!(find("Heading 1")["basedOn"], "Normal");
    assert!(find("Heading 1")["format"]["size"].is_number(), "description formatting");
    assert_eq!(find("Table Grid")["type"], "table");
    assert!(list.iter().any(|x| x["type"] == "character"));
    // The character half of a linked pair is listed once, as the linked style.
    assert!(!list.iter().any(|x| x["id"] == "Heading1Char"));
    let names: Vec<String> = list.iter().map(|x| x["name"].as_str().unwrap().to_lowercase()).collect();
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "alphabetical");
    // Recommended order starts with Normal (priority 0); listing is not an edit.
    let rec = run(&mut s, "styles.manage", json!({}));
    assert_eq!(rec[0]["name"], "Normal");
    assert!(s.run("styles.manage", &json!({"sort": "sideways"})).is_err());
    assert!(!s.dirty);
}

#[test]
fn deleting_a_style_falls_back_to_its_base_and_undoes() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Styled"}));
    run(&mut s, "styles.create", json!({"name": "Mine", "basedOn": "Heading 1"}));
    run(&mut s, "styles.create", json!({"name": "Child", "basedOn": "Mine", "fromSelection": false}));
    run(&mut s, "styles.apply", json!({"style": "Mine"}));
    assert_eq!(para_style(&s, 0).as_deref(), Some("Mine"));
    let r = run(&mut s, "styles.delete", json!({"style": "Mine"}));
    assert_eq!(r["restyled"], 1);
    assert!(s.doc.styles.get("Mine").is_none());
    assert_eq!(para_style(&s, 0).as_deref(), Some("Heading1"), "falls back to its base");
    assert_eq!(s.doc.styles.get("Child").and_then(|c| c.based_on.clone()).as_deref(), Some("Heading1"), "rebased");
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.styles.get("Mine").is_some());
    assert_eq!(para_style(&s, 0).as_deref(), Some("Mine"));
    // Built-in styles are refused, unknown ones are errors.
    assert!(s.run("styles.delete", &json!({"style": "Heading 1"})).is_err());
    assert!(s.run("styles.delete", &json!({"style": "No Such Style"})).is_err());
    assert!(s.doc.styles.get("Heading1").is_some());
}

/// Manage Styles' Delete on a table style is Table Design's Delete Table Style: its tables take
/// Table Grid, styles based on it keep their look, and Word's built-in table styles are refused.
#[test]
fn deleting_a_table_style_from_manage_styles_uses_table_style_deletion() {
    use wordcraft_doc::Rgb;
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let (tp, _, _) = s.sel.focus.path.cell().unwrap();
    run(&mut s, "table.newStyle", json!({"name": "Base", "headerRow": {"fill": "C00000"}}));
    run(&mut s, "table.newStyle", json!({"name": "Child", "basedOn": "Base", "apply": false}));
    let r = run(&mut s, "styles.delete", json!({"style": "Base"}));
    assert_eq!((r["deleted"][0].as_str(), r["restyled"].as_u64()), (Some("Base"), Some(1)));
    assert_eq!(s.doc.table(StoryRef::Body, &tp).unwrap().props.style.as_deref(), Some("TableGrid"));
    assert_eq!(s.doc.styles.table_style("Child").unwrap().parts.header_fill, Some(Rgb(0xC0, 0, 0)), "the child keeps its look");
    for st in s.doc.styles.styles.iter_mut() {
        st.builtin = false;
    }
    assert!(s.run("styles.delete", &json!({"style": "Table Grid"})).is_err());
    let list = run(&mut s, "styles.manage", json!({}));
    let grid = list.as_array().unwrap().iter().find(|x| x["name"] == "Table Grid").unwrap();
    assert_eq!(grid["builtIn"], true);
}

#[test]
fn style_visibility_round_trips_through_docx() {
    let mut s = s();
    run(&mut s, "styles.create", json!({"name": "Mine", "fromSelection": false}));
    run(&mut s, "styles.setVisibility", json!({"style": "Heading 1", "gallery": false}));
    run(&mut s, "styles.setVisibility", json!({"style": "Mine", "hidden": true}));
    run(&mut s, "styles.setVisibility", json!({"style": "Heading 4", "gallery": true}));
    assert!(s.run("styles.setVisibility", &json!({"style": "Mine"})).is_err());
    let gallery: Vec<String> = s.doc.styles.gallery().iter().map(|x| x.id.clone()).collect();
    assert!(!gallery.contains(&"Heading1".to_string()) && !gallery.contains(&"Mine".to_string()) && gallery.contains(&"Heading4".to_string()));
    let bytes = crate::io::save_bytes("v.docx", &s.doc).unwrap();
    let doc = crate::io::open_bytes("v.docx", &bytes).unwrap();
    let st = |id: &str| doc.styles.get(id).cloned().unwrap_or_else(|| panic!("{id} kept"));
    assert!(!st("Heading1").quick && !st("Heading1").hidden);
    assert!(st("Mine").hidden && !st("Mine").builtin);
    assert!(st("Heading4").quick);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.styles.get("Heading4").is_some_and(|h| !h.quick), "undoable");
}

/// #548: `.xml` saves a Word XML Document (Flat OPC) and opens it again; other XML is refused.
#[test]
fn word_xml_saves_and_opens_through_io() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Kept in one XML file"}));
    let bytes = crate::io::save_bytes("notes.xml", &s.doc).unwrap();
    assert_eq!(wordcraft_docx::sniff_xml(&bytes), wordcraft_docx::XmlKind::FlatOpc);
    let doc = crate::io::open_bytes("notes.XML", &bytes).unwrap();
    assert_eq!(doc.body, crate::io::open_bytes("n.docx", &crate::io::save_bytes("n.docx", &s.doc).unwrap()).unwrap().body);
    let err = crate::io::open_bytes("feed.xml", b"<rss><channel/></rss>").unwrap_err();
    assert!(err.contains("not a Word XML document"), "{err}");
}

/// #1081: `format: "wordml2003"` saves an `.xml` file as a Word 2003 XML Document, a plain save
/// over such a file keeps that format, and `format: "flatopc"` converts it back.
#[test]
fn word_2003_xml_saves_through_the_save_commands() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Kept for old readers"}));
    let dir = std::env::temp_dir().join(format!("wordcraft-2003-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.xml").to_string_lossy().to_string();
    let kind = |p: &str| wordcraft_docx::sniff_xml(&std::fs::read(p).unwrap());
    run(&mut s, "file.saveAs", json!({"path": path, "format": "wordml2003"}));
    assert_eq!(kind(&path), wordcraft_docx::XmlKind::WordMl2003);
    assert!(crate::io::open_path(std::path::Path::new(&path)).unwrap().plain_text(wordcraft_doc::StoryRef::Body).contains("Kept for old readers"));
    run(&mut s, "file.save", json!({}));
    assert_eq!(kind(&path), wordcraft_docx::XmlKind::WordMl2003);
    run(&mut s, "file.save", json!({"path": path, "format": "flatopc"}));
    assert_eq!(kind(&path), wordcraft_docx::XmlKind::FlatOpc);
    for bad in [
        json!({"path": dir.join("a.docx").to_string_lossy(), "format": "wordml2003"}),
        json!({"path": path, "format": "bogus"}),
        json!({"path": path, "format": 7}),
    ] {
        assert!(s.run("file.save", &bad).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// #146: the first/last column, total row and banded column regions apply in layout (later
/// regions win: the total row's fill beats the first column's) and round-trip through .docx.
#[test]
fn custom_table_style_column_and_total_regions() {
    use wordcraft_doc::Rgb;
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 4}));
    run(
        &mut s,
        "table.look",
        json!({"headerRow": false, "bandedRows": false, "firstColumn": true, "lastColumn": true, "totalRow": true, "bandedColumns": true}),
    );
    let r = run(
        &mut s,
        "table.newStyle",
        json!({"name": "Ledger",
            "firstColumn": {"fill": "112233", "bold": true},
            "lastColumn": {"fill": "445566"},
            "lastRow": {"fill": "778899", "borders": true},
            "bandedColumns": {"fill": "ABCDEF"}}),
    );
    let id = r["id"].as_str().unwrap().to_string();
    let fills = |s: &mut Session, c: Rgb| {
        s.layout().pages[0].items.iter().filter(|i| matches!(i, crate::layout::Placed::Fill { color, .. } if *color == c)).count()
    };
    // 3 rows x 4 columns: the total row takes all 4 of its cells, the first and last columns the
    // two cells above it, and the column band (column 2 of the inner columns 2 and 3) one each.
    assert_eq!(fills(&mut s, Rgb(0x77, 0x88, 0x99)), 4, "total row");
    assert_eq!(fills(&mut s, Rgb(0x11, 0x22, 0x33)), 2, "first column");
    assert_eq!(fills(&mut s, Rgb(0x44, 0x55, 0x66)), 2, "last column");
    assert_eq!(fills(&mut s, Rgb(0xAB, 0xCD, 0xEF)), 2, "first column band");

    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let p = back.styles.get(&id).unwrap().table.clone().unwrap();
    assert_eq!(
        (p.first_col_fill, p.first_col_chr.bold, p.last_col_fill, p.total_fill, p.col_band_fill),
        (Some(Rgb(0x11, 0x22, 0x33)), Some(true), Some(Rgb(0x44, 0x55, 0x66)), Some(Rgb(0x77, 0x88, 0x99)), Some(Rgb(0xAB, 0xCD, 0xEF)))
    );
    assert!(p.total_borders.is_some_and(|b| b.any_visible()));
}

/// #146: deleting a custom table style re-bases the styles based on it (keeping their look),
/// puts its tables on Table Grid, refuses built-in styles, and undoes.
#[test]
fn delete_table_style_falls_back_and_undoes() {
    use wordcraft_doc::Rgb;
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let (tp, _, _) = s.sel.focus.path.cell().unwrap();
    run(&mut s, "table.newStyle", json!({"name": "Base", "headerRow": {"fill": "C00000"}}));
    run(&mut s, "table.newStyle", json!({"name": "Child", "basedOn": "Base", "apply": false, "wholeTable": {"bold": true}}));
    let style = |s: &Session| s.doc.table(StoryRef::Body, &tp).unwrap().props.style.clone();
    assert_eq!(style(&s).as_deref(), Some("Base"));
    assert!(s.run("table.deleteStyle", &json!({"style": "Table Grid"})).is_err(), "built-in");
    assert!(s.run("table.deleteStyle", &json!({"style": "Normal Table"})).is_err(), "built-in");

    let r = run(&mut s, "table.deleteStyle", json!({}));
    assert_eq!((r["deleted"].as_str(), r["tables"].as_u64()), (Some("Base"), Some(1)));
    assert!(s.doc.styles.get("Base").is_none());
    assert_eq!(style(&s).as_deref(), Some("TableGrid"), "tables fall back to Table Grid");
    let child = s.doc.styles.table_style("Child").unwrap();
    assert_eq!((child.parts.header_fill, child.chr.bold), (Some(Rgb(0xC0, 0, 0)), Some(true)), "the child keeps its look");
    assert_eq!(s.doc.styles.get("Child").unwrap().based_on.as_deref(), Some("TableGrid"));

    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.styles.get("Base").is_some());
    assert_eq!(style(&s).as_deref(), Some("Base"));
    assert_eq!(s.doc.styles.get("Child").unwrap().based_on.as_deref(), Some("Base"));
}

/// #146: a .docx may mark Word's built-in table styles as custom; they still can't be deleted, so
/// no table is left pointing at a style that's gone.
#[test]
fn builtin_table_styles_marked_custom_in_a_docx_cant_be_deleted() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    for st in s.doc.styles.styles.iter_mut() {
        st.builtin = false;
    }
    for name in ["Table Grid", "Normal Table", "TableGrid"] {
        assert!(s.run("table.deleteStyle", &json!({"style": name})).is_err(), "{name}");
    }
    let (tp, _, _) = s.sel.focus.path.cell().unwrap();
    let id = s.doc.table(StoryRef::Body, &tp).unwrap().props.style.clone().unwrap();
    assert!(s.doc.styles.get(&id).is_some(), "the table's style still exists");
}

/// Delete Cells can leave a row shorter than the table grid; Delete Columns then removed that row's
/// only cell and every row with it, leaving a table block with no rows.
#[test]
fn deleting_the_last_cell_column_never_leaves_a_table_without_rows() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 2}));
    run(&mut s, "table.deleteCells", json!({}));
    run(&mut s, "table.deleteColumn", json!({}));
    for b in s.doc.body.iter() {
        if let Some(t) = b.as_table() {
            assert!(!t.rows.is_empty(), "table without rows");
        }
    }
    assert!(s.doc.body.iter().all(|b| b.as_table().is_none()), "an emptied table is removed");
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.body.iter().any(|b| b.as_table().is_some_and(|t| !t.rows.is_empty())));
}

#[test]
fn center_after_select_paragraph_leaves_the_next_paragraph_alone() {
    // #418: Select Paragraph ends at the next paragraph's start; that paragraph isn't selected.
    use wordcraft_doc::Align;
    let align = |s: &Session, i: usize| s.doc.para_at(&Pos::body(i, 0)).and_then(|p| p.props.align);
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Alpha\rBeta", "raw": true}));
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 2}}));
    run(&mut s, "select.paragraph", json!({}));
    assert_eq!(s.selected_text(), "Alpha\n");
    run(&mut s, "para.alignCenter", json!({}));
    assert_eq!((align(&s, 0), align(&s, 1)), (Some(Align::Center), None));
    // Reversed, too; a range into Beta centres both; a caret at Beta's start centres Beta.
    for (a, b, want) in [((1, 0), (0, 0), (Some(Align::Center), None)), ((0, 0), (1, 1), (Some(Align::Center), Some(Align::Center)))] {
        let mut s = crate::Session::new(wordcraft_doc::Document::new());
        run(&mut s, "text.insert", json!({"text": "Alpha\rBeta", "raw": true}));
        run(&mut s, "caret.set", json!({"pos": {"block": a.0, "off": a.1}}));
        run(&mut s, "caret.set", json!({"pos": {"block": b.0, "off": b.1}, "extend": true}));
        run(&mut s, "para.align", json!({"value": "center"}));
        assert_eq!((align(&s, 0), align(&s, 1)), want, "{a:?}..{b:?}");
    }
    let mut s = crate::Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Alpha\rBeta", "raw": true}));
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    run(&mut s, "para.alignCenter", json!({}));
    assert_eq!((align(&s, 0), align(&s, 1)), (None, Some(Align::Center)));
}

#[test]
fn change_case_keeps_the_selection_when_lengths_change() {
    // #420: İ lowers to i + U+0307 (one byte more); the selection still covers the result.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "aİxZ", "raw": true}));
    run(&mut s, "select.text", json!({"text": "İx"}));
    run(&mut s, "format.changeCase", json!({"mode": "lower"}));
    assert_eq!(text(&s), "ai\u{307}xZ");
    assert_eq!(s.selected_text(), "i\u{307}x");
    // ı uppers to I (one byte less); the trailing Z stays out of the selection.
    let mut s = crate::Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "ıxZ", "raw": true}));
    run(&mut s, "select.text", json!({"text": "ıx"}));
    run(&mut s, "format.changeCase", json!({"mode": "upper"}));
    assert_eq!(text(&s), "IXZ");
    assert_eq!(s.selected_text(), "IX");
}

#[test]
fn change_case_keeps_each_runs_formatting_when_lengths_change() {
    // #421: lowercasing a plain İ and a bold x keeps x bold (and the i̇ plain).
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "İx", "raw": true}));
    run(&mut s, "select.text", json!({"text": "x"}));
    run(&mut s, "format.bold", json!({"value": true}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.changeCase", json!({"mode": "lower"}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.text, "i\u{307}x");
    assert_ne!(p.props_of_char(0).bold, Some(true));
    assert_ne!(p.props_of_char(1).bold, Some(true));
    assert_eq!(p.props_of_char(3).bold, Some(true));
    // Typing over the converted selection replaces all of it.
    run(&mut s, "text.insert", json!({"text": "Q", "raw": true}));
    assert_eq!(text(&s), "Q");
}

#[test]
fn change_case_sentence_ignores_decimal_points() {
    // #3485: Sentence case should not capitalize after decimal points.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "one 3.14 apples and two 2.5 pears."}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.changeCase", json!({"mode": "sentence"}));
    assert_eq!(text(&s), "One 3.14 apples and two 2.5 pears.");
}

#[test]
fn document_grid_sets_lines_and_characters_in_one_undo_step() {
    // #391: Layout › Page Setup › Document Grid.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "日本語"}));
    let undo = s.undo_labels().len();
    let r = run(&mut s, "layout.documentGrid", json!({"type": "linesAndChars", "linesPerPage": 40, "charsPerLine": 30}));
    assert_eq!(r["type"], "linesAndChars");
    assert_eq!(r["linesPerPage"], 40);
    assert_eq!(r["charsPerLine"], 30);
    let g = crate::cmd::page::sect(&s).doc_grid.unwrap();
    assert!((g.line_pitch - 648.0 / 40.0).abs() < 1e-3, "{g:?}");
    assert_eq!(s.undo_labels().len(), undo + 1);
    // Hostile values are clamped, not trusted.
    let r = run(&mut s, "layout.documentGrid", json!({"linePitch": -5, "charSpace": 1e15}));
    assert_eq!(r["linePitch"], 1.0);
    assert!(r["charPitch"].as_f64().unwrap() <= 1584.0);
    assert!(s.run("layout.documentGrid", &json!({"type": "diagonal"})).is_err());
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(crate::cmd::page::sect(&s).doc_grid, None);
}

#[test]
fn insert_icon_places_a_picture_of_the_requested_size_that_saves_and_recolours() {
    use wordcraft_doc::para::InlineObject;
    let mut s = s();
    // Without an id: the library is listed and a front end is asked for its picker.
    let r = run(&mut s, "insert.icon", json!({"query": "envelope"}));
    assert_eq!(r["icons"][0]["id"], "mail");
    assert_eq!(s.ui_requests.last(), Some(&json!({"open": "icons"})));
    let all = run(&mut s, "insert.icon", json!({}));
    assert!(all["icons"].as_array().unwrap().len() >= 40);
    assert!(s.run("insert.icon", &json!({"id": "no-such-icon"})).is_err());
    assert!(s.run("insert.icon", &json!({"id": "heart", "color": "red"})).is_err());

    let r = run(&mut s, "insert.icon", json!({"id": "heart", "color": "C00000", "size": 48}));
    assert_eq!((r["width"].as_f64(), r["height"].as_f64(), r["icon"].as_str()), (Some(48.0), Some(48.0), Some("heart")));
    let Some((_, InlineObject::Image { media, w, h, alt, .. })) = crate::cmd::objects::selected(&s) else { panic!("no picture") };
    assert_eq!((w, h, alt.as_str()), (48.0, 48.0, "Heart"));
    let png = image::load_from_memory(&s.doc.media[&media]).unwrap().to_rgba8();
    assert_eq!(png.dimensions(), (200, 200), "about 300 pixels per inch");
    let inked: Vec<_> = png.pixels().filter(|p| p.0[3] == 255).collect();
    assert!(!inked.is_empty() && inked.iter().all(|p| p.0[..3] == [0xC0, 0, 0]));
    assert!(png.get_pixel(0, 0).0[3] == 0, "transparent background");
    // It is an ordinary picture: it saves to .docx.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(back.media.len(), 1);

    // Icon Color repaints it, keeping the transparency.
    run(&mut s, "picture.iconColor", json!({"color": "1F4E79"}));
    let Some((_, InlineObject::Image { media: m2, .. })) = crate::cmd::objects::selected(&s) else { panic!("no picture") };
    let png2 = image::load_from_memory(&s.doc.media[&m2]).unwrap().to_rgba8();
    assert!(png2.pixels().zip(png.pixels()).all(|(a, b)| a.0[3] == b.0[3] && (a.0[3] == 0 || a.0[..3] == [0x1F, 0x4E, 0x79])));
    assert!(s.run("picture.iconColor", &json!({})).is_err());
}

/// Print Markup (#589): `file.printMarkup` or a command's `markup` puts the comments into PDFs
/// (balloons beside the scaled page) and at the end of HTML, Markdown and text copies.
#[test]
fn print_markup_exports_comments() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("The quarterly figures look strong."));
    s.author = "Ada Reviewer".into();
    s.run("select.text", &json!({"text": "figures"})).unwrap();
    let id = s.run("review.newComment", &json!({"text": "Check the totals"})).unwrap()["id"].as_u64().unwrap();
    s.run("review.reply", &json!({"id": id, "text": "Totals checked"})).unwrap();
    let opts = |s: &Session, v| cmd::file::export_options(s, &v);
    assert!(!opts(&s, json!({})).markup, "off by default");
    let pdf = |s: &Session, v| crate::io::save_bytes_opts("x.pdf", &s.doc, &opts(s, v)).unwrap();
    let html = |s: &Session, v| String::from_utf8(crate::io::save_bytes_opts("x.html", &s.doc, &opts(s, v)).unwrap()).unwrap();
    let txt = |s: &Session, v| String::from_utf8(crate::io::save_bytes_opts("x.txt", &s.doc, &opts(s, v)).unwrap()).unwrap();
    assert!(!html(&s, json!({})).contains("Check the totals"));
    assert!(!txt(&s, json!({})).contains("Check the totals"));
    let plain = pdf(&s, json!({}));
    // The setting turns it on for every copy; a command's `markup` overrides it.
    assert_eq!(s.run("file.printMarkup", &json!({})).unwrap()["value"], true);
    let h = html(&s, json!({}));
    assert!(
        h.contains("<section class=\"comments\">") && h.contains("Ada Reviewer") && h.contains("<q>figures</q>") && h.contains("Totals checked"),
        "{h}"
    );
    assert!(h.find("Check the totals") < h.find("</body>"));
    let t = txt(&s, json!({}));
    assert!(t.contains("[1] Ada Reviewer on “figures”: Check the totals") && t.contains("Totals checked"), "{t}");
    let md = String::from_utf8(crate::io::save_bytes_opts("x.md", &s.doc, &opts(&s, json!({}))).unwrap()).unwrap();
    assert!(md.contains("## Comments") && md.contains("1. **Ada Reviewer**"), "{md}");
    assert_ne!(pdf(&s, json!({})), plain);
    assert!(!html(&s, json!({"markup": false})).contains("Check the totals"));
    // Comments hidden in Track Changes Options stay out.
    s.prefs.markup.comments = false;
    assert!(!html(&s, json!({})).contains("Check the totals"));
    s.prefs.markup.comments = true;
    assert_eq!(s.run("file.printMarkup", &json!({"value": false})).unwrap()["value"], false);
    assert!(html(&s, json!({"markup": true})).contains("Check the totals"));
    // file.exportPdf and file.exportPng take it too.
    let dir = std::env::temp_dir().join(format!("wordcraft-print-markup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.pdf");
    let r = s.run("file.exportPdf", &json!({"path": path.to_string_lossy(), "markup": true})).unwrap();
    assert_eq!(r["markup"], true);
    assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF-"));
    let png = dir.join("m.png");
    let wide = s.run("file.exportPng", &json!({"path": png.to_string_lossy(), "markup": true, "scale": 1.0})).unwrap()["width"].as_u64().unwrap();
    let narrow = s.run("file.exportPng", &json!({"path": png.to_string_lossy(), "scale": 1.0})).unwrap()["width"].as_u64().unwrap();
    assert!(wide > narrow + 200, "{wide} vs {narrow}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Kerning for fonts (`w:kern`) at the selection, for the Font dialog: the size it starts at, 0
/// when off, null when the selected text differs.
#[test]
fn format_state_reports_kerning_for_fonts() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Kerned plain"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.set", json!({"props": {"kern": 14}}));
    assert_eq!(run(&mut s, "format.state", json!({}))["kern"], json!(14.0));
    run(&mut s, "format.set", json!({"props": {"kern": 0}}));
    assert_eq!(run(&mut s, "format.state", json!({}))["kern"], json!(0.0));
    run(&mut s, "select.text", json!({"text": "Kerned"}));
    run(&mut s, "format.set", json!({"props": {"kern": 12}}));
    run(&mut s, "select.all", json!({}));
    assert_eq!(run(&mut s, "format.state", json!({}))["kern"], serde_json::Value::Null);
}

/// #606: the colour commands behind More Colors… take any `RRGGBB` (with or without `#`, any
/// case), not only the palette's colours; junk is a params error and changes nothing.
#[test]
fn colour_commands_take_any_custom_hex() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Custom"}));
    run(&mut s, "select.all", json!({}));
    for (given, want) in [("7A3EC8", "7A3EC8"), ("#0b5d1e", "0B5D1E"), ("  ff8800 ", "FF8800")] {
        run(&mut s, "format.color", json!({"color": given}));
        assert_eq!(run(&mut s, "format.state", json!({})).get("color"), Some(&json!(want)));
    }
    let doc = s.doc.clone();
    for junk in [json!("#12"), json!("purple"), json!("GG0000"), json!(12), json!(null), json!("ééé")] {
        assert!(s.run("format.color", &json!({"color": junk})).is_err(), "{junk}");
    }
    assert_eq!(s.doc, doc);
    run(&mut s, "format.color", json!({"color": "auto"}));
    assert_eq!(run(&mut s, "format.state", json!({})).get("color"), Some(&json!("auto")));
    // Shading and page colour take custom colours the same way.
    run(&mut s, "para.shading", json!({"color": "#C0FFEE"}));
    let shading = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.shading);
    assert_eq!(shading, wordcraft_doc::Rgb::parse("C0FFEE"));
    run(&mut s, "design.pageColor", json!({"color": "123ABC"}));
    assert_eq!(s.doc.settings.page_color, wordcraft_doc::Rgb::parse("123ABC"));
}

/// #2738: Home › Underline › Underline Color sets the underline's own colour and leaves its style
/// alone; `auto` drops it again. One undo step each, kept through a DOCX round trip, and junk is
/// a params error that changes nothing.
#[test]
fn underline_color_sets_and_clears_the_underlines_own_colour() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Lanterns"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.underline", json!({"style": "wave"}));
    assert_eq!(run(&mut s, "format.state", json!({})).get("underlineColor"), Some(&json!("auto")));
    let style = |s: &Session| s.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap().runs[0].props.underline;
    let wave = style(&s);
    for (given, want) in [("C00000", "C00000"), ("#1b5e20", "1B5E20")] {
        run(&mut s, "format.underlineColor", json!({"color": given}));
        assert_eq!(run(&mut s, "format.state", json!({})).get("underlineColor"), Some(&json!(want)));
        assert_eq!(style(&s), wave, "the style is kept");
    }
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let para = back.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap();
    assert_eq!(para.runs[0].props.underline_color, wordcraft_doc::Rgb::parse("1B5E20"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "format.state", json!({})).get("underlineColor"), Some(&json!("C00000")));
    run(&mut s, "format.underlineColor", json!({"color": "auto"}));
    assert_eq!(run(&mut s, "format.state", json!({})).get("underlineColor"), Some(&json!("auto")));
    assert_eq!(style(&s), wave);
    let doc = s.doc.clone();
    for junk in [json!({}), json!({"color": "purple"}), json!({"color": 12}), json!({"color": null}), json!({"color": "#12"})] {
        assert!(s.run("format.underlineColor", &junk).is_err(), "{junk}");
    }
    assert_eq!(s.doc, doc);
}

/// A document whose content is another format than its extension says opens by what it holds,
/// as in Word: RTF saved as `.doc` (accounting systems and report generators save their documents
/// that way), a `.docx` package renamed `.doc`, and RTF renamed `.docx`.
#[test]
fn documents_open_by_content_when_the_extension_is_wrong() {
    let text = |doc: &wordcraft_doc::Document| doc.plain_text(StoryRef::Body);
    let rtf = b"{\\rtf1\\ansi\\ansicpg1251\\deff0{\\fonttbl{\\f0 Times New Roman;}}\\f0 \\'d1\\'f7\\'e5\\'f2 \\'b9 12\\par Invoice line\\par}";
    for name in ["Счет 12.doc", "report.DOC", "letter.dot", "renamed.docx"] {
        let doc = crate::io::open_bytes(name, rtf).unwrap_or_else(|e| panic!("{name}: {e}"));
        let t = text(&doc);
        assert!(t.contains("Счет № 12") && t.contains("Invoice line"), "{name}: {t:?}");
    }
    // A Word package renamed `.doc` or `.rtf`.
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Packaged text"}));
    let docx = crate::io::save_bytes("a.docx", &s.doc).unwrap();
    for name in ["package.doc", "package.rtf"] {
        let doc = crate::io::open_bytes(name, &docx).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(text(&doc).contains("Packaged text"), "{name}");
    }
    // Matching content still goes the usual way, and garbage stays an error.
    assert!(text(&crate::io::open_bytes("plain.rtf", rtf).unwrap()).contains("Invoice line"));
    assert!(crate::io::open_bytes("broken.doc", b"\x00\x01not a document").is_err());
}

/// #2853: a Word XML Document (Word 2003 XML or Flat OPC) or a web page saved under a `.doc`
/// name opens by what it holds, as in Word; it went to the Word 97-2003 reader, which refused it,
/// and the window showed a blank page.
#[test]
fn word_xml_and_web_pages_saved_as_doc_open_by_content() {
    let text = |doc: &wordcraft_doc::Document| doc.plain_text(StoryRef::Body);
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Truyện ngắn, chương một"}));
    run(&mut s, "format.bold", json!({}));
    let word2003 = crate::io::ExportOptions { word2003: true, ..Default::default() };
    let xml2003 = crate::io::save_bytes_opts("a.xml", &s.doc, &word2003).unwrap();
    let flat = crate::io::save_bytes("a.xml", &s.doc).unwrap();
    assert!(xml2003.windows(14).any(|w| w == b"w:wordDocument") && flat.windows(11).any(|w| w == b"pkg:package"));
    for (what, bytes) in [("Word 2003 XML", &xml2003), ("Flat OPC", &flat)] {
        for name in ["soulsoul.txt.doc", "Story.DOC", "story.dot", "story.docx", "story.rtf"] {
            let doc = crate::io::open_bytes(name, bytes).unwrap_or_else(|e| panic!("{what} as {name}: {e}"));
            assert!(text(&doc).contains("Truyện ngắn, chương một"), "{what} as {name}: {:?}", text(&doc));
        }
    }
    // With a byte-order mark and blank lines before the declaration.
    let bom = [&[0xEF, 0xBB, 0xBF][..], b"\r\n  ", &xml2003].concat();
    assert!(text(&crate::io::open_bytes("bom.doc", &bom).unwrap()).contains("chương một"));

    // Web pages: Word's Save as Web Page, and a report generator's HTML under a `.doc` name.
    let pages: [&str; 3] = [
        "<html xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:w=\"urn:schemas-microsoft-com:office:word\">\r\n<head><meta charset=utf-8></head>\r\n<body lang=VI><p class=MsoNormal><b>Báo cáo</b> tháng 9</p></body></html>",
        "\u{FEFF}<!DOCTYPE html>\n<!-- generated -->\n<html><body><h1>Báo cáo</h1><p>tháng 9</p></body></html>",
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE HTML PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"x\"><HTML><BODY><P>Báo cáo tháng 9</P></BODY></HTML>",
    ];
    for (i, page) in pages.iter().enumerate() {
        for name in ["report.doc", "report.docx", "report.rtf"] {
            let doc = crate::io::open_bytes(name, page.as_bytes()).unwrap_or_else(|e| panic!("page {i} as {name}: {e}"));
            let t = text(&doc);
            assert!(t.contains("Báo cáo") && t.contains("tháng 9") && !t.contains('<'), "page {i} as {name}: {t:?}");
        }
    }
    // A web page in an unknown extension is still plain text, other XML still no Word document.
    assert!(text(&crate::io::open_bytes("page.log", pages[1].as_bytes()).unwrap()).contains("<html>"));
    assert!(crate::io::open_bytes("note.doc", b"<?xml version=\"1.0\"?><note>hi</note>").is_err());
    // Nor is XML whose root only starts like `html`.
    for xml in [&b"<htmlish>text</htmlish>"[..], b"<!DOCTYPE htmlish><htmlish>text</htmlish>"] {
        assert!(crate::io::open_bytes("other.doc", xml).is_err(), "{:?}", String::from_utf8_lossy(xml));
    }
}

/// #569: a playlist (.m3u / .m3u8) — or any file with an extension WordCraft doesn't know —
/// opens as plain text with its encoding detected, like Word's text import; a binary file is a
/// clean error, never garbage or a panic.
#[test]
fn unknown_text_extensions_open_as_text() {
    let lines = |doc: &wordcraft_doc::Document| doc.plain_text(StoryRef::Body);
    let m3u8 = "#EXTM3U\r\n#EXTINF:215,Björk — Jóga\r\nmusic/Jóga.mp3\r\n#EXTINF:-1,Радио\r\nhttp://example.org/live\r\n";
    let doc = crate::io::open_bytes("Mix.M3U8", m3u8.as_bytes()).unwrap();
    assert!(
        lines(&doc).starts_with("#EXTM3U\n#EXTINF:215,Björk — Jóga\nmusic/Jóga.mp3\n#EXTINF:-1,Радио\nhttp://example.org/live"),
        "{:?}",
        lines(&doc)
    );
    assert!(!lines(&doc).contains('\r'));
    // With a BOM, which must not show up as a character.
    let bom = [&[0xEF, 0xBB, 0xBF][..], m3u8.as_bytes()].concat();
    assert!(lines(&crate::io::open_bytes("a.m3u8", &bom).unwrap()).starts_with("#EXTM3U\n"));
    // A plain .m3u in Windows-1252 (é = E9, — = 97).
    let cp1252 = b"#EXTM3U\r\n#EXTINF:100,Caf\xE9 \x97 Live\r\nC:\\Music\\caf\xE9.mp3\r\n";
    assert_eq!(lines(&crate::io::open_bytes("old.m3u", cp1252).unwrap()), "#EXTM3U\n#EXTINF:100,Café — Live\nC:\\Music\\café.mp3\n");
    // Other text files, a file without an extension, a dot in a folder name only.
    for name in ["server.log", "settings.ini", "subs.srt", "README", "/home/a.b/README", "C:\\v1.2\\notes"] {
        assert_eq!(lines(&crate::io::open_bytes(name, b"one\ntwo").unwrap()), "one\ntwo", "{name}");
    }
    // Binary files: an error naming the file, no panic.
    for (name, bytes) in
        [("song.mp3", &b"ID3\x04\0\0\0\0\x01\x02\x03"[..]), ("pic.dat", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), ("blob", &[0u8, 1, 2, 3, 0, 255])]
    {
        let err = crate::io::open_bytes(name, bytes).unwrap_err();
        assert!(err.contains(name) && err.contains("not a document"), "{name}: {err}");
    }
    assert!(crate::io::open_bytes("x.pdf", b"%PDF-1.7\n1 0 obj\n").is_err());
    // A Word document or RTF file with the wrong extension opens by what it holds.
    let docx = crate::io::save_bytes("x.docx", &crate::sample::letter()).unwrap();
    assert_eq!(lines(&crate::io::open_bytes("letter.bak", &docx).unwrap()), lines(&crate::io::open_bytes("x.docx", &docx).unwrap()));
    let rtf = crate::io::open_bytes("note.tmp", br"{\rtf1\ansi Hello {\b bold}\par}").unwrap();
    assert!(lines(&rtf).starts_with("Hello bold"), "{:?}", lines(&rtf));
    // Through file.open, both from disk and with the bytes (the web's picker and drops).
    let dir = std::env::temp_dir().join(format!("wordcraft-569-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("road trip.m3u");
    std::fs::write(&path, cp1252).unwrap();
    let mut s = s();
    let r = run(&mut s, "file.open", json!({"path": path.to_string_lossy()}));
    assert_eq!(r["paragraphs"], 4);
    assert!(text(&s).contains("Café — Live"));
    let data = crate::cmd::insert::base64_encode(m3u8.as_bytes());
    run(&mut s, "file.open", json!({"path": "Mix.m3u8", "data": data}));
    assert!(text(&s).contains("Радио"));
    let bad = crate::cmd::insert::base64_encode(b"\0\x01\x02binary\0");
    assert!(s.run("file.open", &json!({"path": "x.bin", "data": bad})).is_err());
    assert!(text(&s).contains("Радио"), "a failed open leaves the document alone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1384: File Explorer's New › Microsoft Word Document makes an empty, zero-byte .docx. It opens
/// as a new blank document on the Normal template, and Save writes it back to that file instead
/// of asking for a name.
#[test]
fn an_empty_file_opens_as_a_blank_document_saved_back_to_it() {
    let dir = std::env::temp_dir().join(format!("wordcraft-1384-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("New Microsoft Word Document.docx");
    std::fs::write(&path, b"").unwrap();
    let mut s = s();
    run(&mut s, "file.options", json!({"region": "it-IT"}));
    s.set_editing_language(Some("it-IT"));
    run(&mut s, "text.insert", json!({"text": "An untitled draft"}));
    let r = run(&mut s, "file.open", json!({"path": path.to_string_lossy()}));
    assert_eq!((r["paragraphs"].as_u64(), r["words"].as_u64()), (Some(1), Some(0)), "{r}");
    assert_eq!(text(&s), "");
    assert_eq!(s.path.as_deref(), Some(path.as_path()));
    assert!(!s.dirty);
    let sp = &s.doc.last_section;
    assert_eq!((sp.page_w, sp.page_h), (595.28, 841.89), "the region's paper, as File › New gives");
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("it-IT"), "in the editing language");

    run(&mut s, "text.insert", json!({"text": "Ciao"}));
    let r = run(&mut s, "file.save", json!({}));
    assert_eq!(r["saved"], true, "{r}");
    assert!(!s.ui_requests.iter().any(|q| q["open"] == "saveAs"), "no Save As");
    let saved = crate::io::open_path(&path).unwrap();
    assert_eq!(saved.plain_text(StoryRef::Body), "Ciao");

    // The same from the browser's picker (the web), which hands over the bytes.
    run(&mut s, "file.open", json!({"path": "Vuoto.docx", "data": ""}));
    assert_eq!((text(&s), s.doc.last_section.page_w), (String::new(), 595.28));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #549: after a double-click, dragging extends the selection by whole words; after a
/// triple-click by whole paragraphs; after Ctrl/⌘+click by whole sentences. Forward drags keep
/// the first unit's start as the anchor, backward drags its end.
#[test]
fn drag_after_multi_click_extends_by_units() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Alpha beta gamma. Delta epsilon.\rSecond para here.\rThird one."}));
    let drag = |s: &mut Session, unit: &str, start: Pos, end: Pos, pos: Pos| {
        run(s, "select.dragUnit", json!({"unit": unit, "start": start, "end": end, "pos": pos}));
        (s.sel.anchor.clone(), s.sel.focus.clone())
    };
    // Double-click "beta" (6..11 with its trailing space).
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 7)}));
    run(&mut s, "select.word", json!({}));
    let (a, b) = s.sel.ordered();
    assert_eq!((a.off, b.off), (6, 11));
    // Forward into the middle of "gamma": the whole word joins.
    assert_eq!(drag(&mut s, "word", a.clone(), b.clone(), Pos::body(0, 13)), (Pos::body(0, 6), Pos::body(0, 16)));
    // Back into "Alpha": anchored at the end of "beta ", focus at the start of "Alpha".
    assert_eq!(drag(&mut s, "word", a.clone(), b.clone(), Pos::body(0, 2)), (Pos::body(0, 11), Pos::body(0, 0)));
    // Back inside the first word: just that word.
    assert_eq!(drag(&mut s, "word", a.clone(), b.clone(), Pos::body(0, 8)), (Pos::body(0, 6), Pos::body(0, 11)));
    // Into the next paragraph: whole word there.
    assert_eq!(drag(&mut s, "word", a.clone(), b.clone(), Pos::body(1, 8)), (Pos::body(0, 6), Pos::body(1, 12)));

    // Ctrl+click a sentence, drag into the next sentence.
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 3)}));
    run(&mut s, "select.sentence", json!({}));
    let (a, b) = s.sel.ordered();
    let (_, f) = drag(&mut s, "sentence", a, b, Pos::body(0, 20));
    assert_eq!(f.off, 32, "the second sentence is selected to its end");

    // Triple-click the second paragraph, drag up into the first and down into the third.
    run(&mut s, "caret.set", json!({"pos": Pos::body(1, 3)}));
    run(&mut s, "select.paragraph", json!({}));
    let (a, b) = s.sel.ordered();
    assert_eq!((a.clone(), b.clone()), (Pos::body(1, 0), Pos::body(2, 0)));
    assert_eq!(drag(&mut s, "paragraph", a.clone(), b.clone(), Pos::body(0, 5)), (Pos::body(2, 0), Pos::body(0, 0)));
    assert_eq!(drag(&mut s, "paragraph", a.clone(), b.clone(), Pos::body(2, 2)), (Pos::body(1, 0), Pos::body(2, 10)));

    // Junk never panics and leaves the document alone.
    let before = text(&s);
    for v in [
        json!({}),
        json!({"unit": "page", "start": Pos::body(0, 0), "end": Pos::body(0, 1), "pos": Pos::body(0, 1)}),
        json!({"unit": "word", "start": Pos::body(99, 999), "end": Pos::body(0, 0), "pos": Pos::body(77, usize::MAX)}),
        json!({"unit": "word", "start": {"block": 0, "off": 9_999}, "end": {"block": 0, "off": 1}, "pos": {"block": 5}}),
    ] {
        let _ = s.run("select.dragUnit", &v);
    }
    assert_eq!(text(&s), before);
}

/// #856: a click in the selection bar selects the whole line (its trailing space; the
/// paragraph mark on the last line); dragging extends by lines; triple-click takes the document.
#[test]
fn selection_bar_selects_and_drags_by_lines() {
    let mut s = s();
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(8);
    run(&mut s, "text.insert", json!({"text": format!("{}\rShort one.\rLast.", long.trim_end())}));
    let l = s.layout();
    let c = l.caret(&Pos::body(0, 0)).unwrap();
    let line2 = l.selection_bar_at(c.page, 20.0, c.top + c.height * 1.5).unwrap();
    assert!(line2.off > 0);
    run(&mut s, "select.line", json!({"pos": line2.clone()}));
    let (a, b) = s.sel.ordered();
    assert_eq!(a, line2);
    assert_eq!(b.path, wordcraft_doc::Path::top(0));
    assert!(b.off > a.off && b.off < long.trim_end().len());
    assert_eq!(s.doc.para_at(&b).unwrap().text.as_bytes()[b.off - 1], b' ', "the trailing space is in");
    // The last line of a paragraph takes its mark.
    run(&mut s, "select.line", json!({"pos": Pos::body(1, 3)}));
    assert_eq!(s.sel.ordered(), (Pos::body(1, 0), Pos::body(2, 0)));
    // Drag from the "Short one." line down to "Last." and back up into the first paragraph.
    let (a, b) = s.sel.ordered();
    run(&mut s, "select.dragUnit", json!({"unit": "line", "start": a.clone(), "end": b.clone(), "pos": Pos::body(2, 0)}));
    assert_eq!((s.sel.anchor.clone(), s.sel.focus.clone()), (Pos::body(1, 0), Pos::body(2, 5)));
    run(&mut s, "select.dragUnit", json!({"unit": "line", "start": a.clone(), "end": b.clone(), "pos": line2.clone()}));
    assert_eq!((s.sel.anchor.clone(), s.sel.focus.clone()), (Pos::body(2, 0), line2));
    run(&mut s, "select.dragUnit", json!({"unit": "document", "start": a.clone(), "end": b.clone(), "pos": Pos::body(0, 0)}));
    assert_eq!(s.sel.ordered(), (Pos::body(0, 0), Pos::body(2, 5)));
    // Junk positions never panic.
    let _ = s.run("select.line", &json!({"pos": {"block": 99, "off": usize::MAX}}));
    let _ = s.run("select.line", &json!({"pos": 5}));
}

/// Home › Font › Enclose Characters (#297): the selection becomes an `EQ \o\ac` field, saved the
/// way Word stores it (the style in the code's run sizes) and read back; null takes the text out
/// again, and each step undoes.
#[test]
fn enclose_characters_make_an_eq_field_and_come_back_out() {
    use wordcraft_doc::InlineObject;
    let field = |s: &Session| {
        let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
        p.objects.iter().find_map(|o| match o {
            InlineObject::Field { instr, result, .. } => Some((instr.clone(), result.clone())),
            _ => None,
        })
    };
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "No 12 and 字 here"}));
    run(&mut s, "select.text", json!({"text": "No 12"}));
    assert!(s.run("format.enclose", &json!({"shape": "circle"})).is_err(), "five characters don't fit");
    run(&mut s, "select.text", json!({"text": "字"}));
    assert!(s.run("format.enclose", &json!({})).is_err(), "a script must say which shape");
    run(&mut s, "format.enclose", json!({"shape": "circle"}));
    assert_eq!(field(&s), Some(("eq \\o\\ac(○,字)".to_string(), "字".to_string())));
    assert_eq!(text(&s), "No 12 and 字 here", "the field's text reads as the character");
    run(&mut s, "select.text", json!({"text": "12"}));
    run(&mut s, "format.enclose", json!({"shape": "diamond", "style": "enlarge"}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let codes: Vec<String> =
        p.objects.iter().filter_map(|o| if let InlineObject::Field { instr, .. } = o { Some(instr.clone()) } else { None }).collect();
    assert_eq!(codes, ["eq \\o\\ac(◇,12) \\* enlarge", "eq \\o\\ac(○,字)"]);

    // Word's structure in the .docx: no result, the symbol and the text in runs of their own
    // sizes; reading it back gives the same fields, styles included.
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let back = wordcraft_docx::read(&bytes).unwrap();
    let bp = back.para_at(&Pos::body(0, 0)).unwrap();
    let back_codes: Vec<(String, String)> = bp
        .objects
        .iter()
        .filter_map(|o| if let InlineObject::Field { instr, result, .. } = o { Some((instr.clone(), result.clone())) } else { None })
        .collect();
    assert_eq!(back_codes, [("eq \\o\\ac(◇,12) \\* enlarge".to_string(), "12".to_string()), ("eq \\o\\ac(○,字)".to_string(), "字".to_string())]);

    // Change the shape of the enclosed characters at the caret, then remove it.
    let e = s.doc.para_at(&Pos::body(0, 0)).unwrap().text.find('\u{FFFC}').unwrap();
    s.sel = crate::Selection::caret(Pos::body(0, e + 3));
    run(&mut s, "format.enclose", json!({"shape": "square"}));
    assert!(field(&s).unwrap().0.starts_with("eq \\o\\ac(□,12) \\* enlarge"), "the style stays");
    run(&mut s, "format.enclose", json!({"shape": null}));
    assert_eq!(text(&s), "No 12 and 字 here");
    assert_eq!(s.selected_text(), "12");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(field(&s).unwrap().0.starts_with("eq \\o\\ac(◇,12)"));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(field(&s), None, "back to plain text");
    assert!(s.run("format.enclose", &json!({"shape": null})).is_err(), "nothing to remove");
    // Text given (the dialog's Text box) goes in at a caret.
    run(&mut s, "caret.docEnd", json!({}));
    assert!(s.run("format.enclose", &json!({"shape": "triangle", "text": "ABC"})).is_err());
    run(&mut s, "format.enclose", json!({"shape": "triangle", "text": "A"}));
    assert_eq!(text(&s), "No 12 and 字 hereA");
    assert_eq!(field(&s), Some(("eq \\o\\ac(△,A)".to_string(), "A".to_string())));
}

#[test]
fn list_buttons_follow_the_paragraphs_own_level() {
    use crate::cmd::para::{ListButtons, list_buttons};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "item"}));
    let state = |s: &Session| {
        let rp = s.doc.para_at(&s.sel.focus).map(|p| s.doc.styles.resolve_para(&p.props));
        list_buttons(&s.doc, rp.and_then(|r| r.numbering))
    };
    assert_eq!(state(&s), ListButtons::default(), "plain text: no list button on");
    run(&mut s, "para.bullets", json!({}));
    assert_eq!(state(&s), ListButtons { bullets: true, numbering: false, multilevel: false });
    // A bulleted list whose second level is numbered: at level 2 the paragraph is numbered.
    let num = s.doc.para_at(&s.sel.focus).unwrap().props.numbering.unwrap().num;
    let numbered = wordcraft_doc::numbering::Level { text: "%2.".into(), ..Default::default() };
    s.doc.numbering.nums.iter_mut().find(|n| n.id == num).unwrap().level_overrides.push((1, numbered));
    run(&mut s, "para.listLevel", json!({"level": 1}));
    assert_eq!(state(&s), ListButtons { bullets: false, numbering: true, multilevel: false });
    run(&mut s, "para.numbering", json!({}));
    assert_eq!(state(&s), ListButtons::default(), "Numbering toggles off the paragraph's effective numbered level");
    run(&mut s, "para.numbering", json!({}));
    run(&mut s, "para.listLevel", json!({"level": 1}));
    assert_eq!(state(&s), ListButtons { bullets: false, numbering: true, multilevel: false }, "1. a. i. is not multilevel");
    // Legal numbering (1. 1.1. 1.1.1.) numbers levels together, even on its first level. (Its
    // first level reads like "1." numbering, which the button would toggle off, so start plain.)
    run(&mut s, "para.numbering", json!({"off": true}));
    run(&mut s, "para.multilevel", json!({"kind": "legal"}));
    run(&mut s, "para.listLevel", json!({"level": 0}));
    assert_eq!(state(&s), ListButtons { bullets: false, numbering: true, multilevel: true });
    run(&mut s, "para.listLevel", json!({"level": 2}));
    assert!(state(&s).multilevel);
    run(&mut s, "para.numbering", json!({"off": true}));
    assert_eq!(state(&s), ListButtons::default());
}

#[test]
fn replace_all_splits_paragraphs_at_a_paragraph_mark_in_the_replacement() {
    // Issue #423: untracked Replace All left `\r` in the paragraph instead of splitting it.
    for with in ["A\rB", "A\u{2029}B"] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "cat cat"}));
        run(&mut s, "edit.replaceAll", json!({"text": "cat", "with": with}));
        assert_eq!(body_texts(&s), ["A", "B A", "B"], "{with:?}");
    }
    // A line feed stays a line break.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "cat"}));
    run(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "A\nB"}));
    assert_eq!(body_texts(&s), ["A\nB"]);
}

/// A one-row table converted from tab-separated `text`.
fn table_from(text: &str) -> Session {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": text}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "table.fromText", json!({}));
    s
}

fn table_texts(s: &Session) -> Vec<Vec<String>> {
    let t = s.doc.body.iter().find_map(|b| b.as_table()).expect("a table");
    t.rows.iter().map(|r| r.cells.iter().map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect()).collect()).collect()
}

#[test]
fn convert_text_to_table_refuses_to_drop_content() {
    // Issue #1318: a 64th field and a table between the selected paragraphs were silently lost.
    {
        let wide = (0..64).map(|i| i.to_string()).collect::<Vec<_>>().join("\t");
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": wide}));
        run(&mut s, "select.all", json!({}));
        let depth = s.undo_depth();
        assert!(s.run("table.fromText", &json!({})).is_err());
        assert_eq!(s.doc.body.len(), 1);
        assert!(s.doc.body.iter().all(|b| b.as_table().is_none()));
        assert_eq!(s.undo_depth(), depth);
    }
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "x\ny\nz"}));
    let at = |i: u32| json!({"story": "body", "path": [i], "off": 0});
    run(&mut s, "select.range", json!({"anchor": at(1), "focus": at(1)}));
    run(&mut s, "table.fromText", json!({}));
    assert!(s.doc.body.get(1).and_then(|b| b.as_table()).is_some());
    run(&mut s, "select.range", json!({"anchor": at(0), "focus": at(2)}));
    let depth = s.undo_depth();
    assert!(s.run("table.fromText", &json!({})).is_err());
    assert_eq!(s.doc.body.len(), 3);
    assert!(s.doc.body.get(1).and_then(|b| b.as_table()).is_some());
    assert_eq!(s.undo_depth(), depth);
}

#[test]
fn convert_text_to_table_keeps_inline_content() {
    // Issue #3751: the cells were rebuilt from plain text, losing formatting, pictures, links
    // and tracked deletions.
    use wordcraft_doc::para::InlineObject;
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Name\t\tLink\tOld"}));
    {
        let p = s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        p.format(0, 4, &|c| c.bold = Some(true)).unwrap();
        p.format(6, 10, &|c| c.link = Some("https://example.com".into())).unwrap();
        p.format(11, 14, &|c| c.del = Some(1)).unwrap();
        let img = InlineObject::Image {
            media: "pic".into(),
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        };
        p.insert_object(5, img, &Default::default()).unwrap();
    }
    run(&mut s, "select.all", json!({}));
    run(&mut s, "table.fromText", json!({}));
    let t = s.doc.body.iter().find_map(|b| b.as_table()).expect("a table");
    let cell = |c: usize| t.rows[0].cells[c].blocks.iter().find_map(|b| b.as_para()).expect("a paragraph");
    assert_eq!(cell(0).text, "Name");
    assert_eq!(cell(0).props_of_char(0).bold, Some(true), "cell 1 lost bold");
    assert!(matches!(cell(1).objects.as_slice(), [InlineObject::Image { media, .. }] if media == "pic"), "cell 2 lost its picture");
    assert_eq!(cell(2).text, "Link");
    assert_eq!(cell(2).props_of_char(0).link.as_deref(), Some("https://example.com"), "cell 3 lost its link");
    assert_eq!(cell(3).text, "Old");
    assert_eq!(cell(3).props_of_char(0).del, Some(1), "cell 4 lost its tracked deletion");
}

/// A table `cols` wide holding `cells` row by row, typed in with Tab; the caret ends in the
/// last cell.
fn typed_table(cols: usize, cells: &[&str]) -> Session {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": cells.len() / cols, "cols": cols}));
    for (k, c) in cells.iter().enumerate() {
        if k > 0 {
            run(&mut s, "text.tab", json!({}));
        }
        if !c.is_empty() {
            run(&mut s, "text.insert", json!({"text": c}));
        }
    }
    s
}

/// The body as `T` for a table and the text of each paragraph.
fn body_outline(s: &Session) -> Vec<String> {
    s.doc.body.iter().map(|b| b.as_para().map_or_else(|| "T".to_string(), |p| p.text.clone())).collect()
}

fn cell_at(table: u32, r: u32, c: u32) -> serde_json::Value {
    json!({"story": "body", "path": [table, r, c, 0], "off": 0})
}

/// Convert to Text sets a row's cells apart with a tab, a comma, another character or
/// paragraph marks; rows are paragraphs.
#[test]
fn convert_to_text_takes_each_separator() {
    let cases = [
        (json!({}), vec!["a\tb", "c\td"]),
        (json!({"separator": "tab"}), vec!["a\tb", "c\td"]),
        (json!({"separator": "comma"}), vec!["a,b", "c,d"]),
        (json!({"separator": "other", "char": ";"}), vec!["a;b", "c;d"]),
        (json!({"separator": "paragraph"}), vec!["a", "b", "c", "d"]),
    ];
    for (params, want) in cases {
        let mut s = typed_table(2, &["a", "b", "c", "d"]);
        run(&mut s, "table.toText", params.clone());
        let got = body_outline(&s);
        assert_eq!(got.get(..want.len()), Some(&want.iter().map(|x| x.to_string()).collect::<Vec<_>>()[..]), "{params}: {got:?}");
        assert!(s.doc.body.iter().all(|b| b.as_table().is_none()), "{params}");
    }
    // U+FFFC marks an object in a paragraph's text, so it cannot be the separator.
    let obj = wordcraft_doc::para::OBJ.to_string();
    for bad in [
        json!({"separator": "other"}),
        json!({"separator": "other", "char": "\n"}),
        json!({"separator": "other", "char": obj}),
        json!({"separator": "pipe"}),
    ] {
        let mut s = typed_table(2, &["a", "b", "c", "d"]);
        let depth = s.undo_depth();
        assert!(s.run("table.toText", &bad).is_err(), "{bad}");
        assert!(s.doc.body.iter().any(|b| b.as_table().is_some()), "{bad}: the table stays");
        assert_eq!(s.undo_depth(), depth);
    }
}

/// With rows selected, only those become text; the rows above and below stay tables, and a
/// vertical merge cut by the selection starts again below it.
#[test]
fn convert_to_text_converts_the_selected_rows() {
    let mut s = typed_table(2, &["a", "b", "c", "d", "e", "f", "g", "h"]);
    let t = s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32;
    // Column 1 merged down rows 1 to 3.
    run(&mut s, "select.range", json!({"anchor": cell_at(t, 1, 1), "focus": cell_at(t, 3, 1)}));
    run(&mut s, "table.merge", json!({}));
    run(&mut s, "select.range", json!({"anchor": cell_at(t, 1, 0), "focus": cell_at(t, 1, 1)}));
    run(&mut s, "table.toText", json!({}));
    let at = t as usize;
    // The merged cell holds d, f and h, one paragraph each.
    assert_eq!(body_outline(&s).get(at..at + 5), Some(&["T", "c\td", "f", "h", "T"].map(String::from)[..]), "{:?}", body_outline(&s));
    let head = s.doc.body[at].as_table().unwrap();
    let tail = s.doc.body[at + 4].as_table().unwrap();
    assert_eq!((head.rows.len(), tail.rows.len()), (1, 2));
    use wordcraft_doc::props::VMerge;
    let merges: Vec<VMerge> = tail.rows.iter().map(|r| r.cells.last().unwrap().props.vmerge).collect();
    assert_eq!(merges, [VMerge::Restart, VMerge::Continue], "the merge starts again in the first row left");
    assert_eq!(s.sel.focus.path.0, [t + 1], "the caret goes to the converted text");

    // A merge that went on into the converted rows ends in the last row above them.
    let mut s = typed_table(2, &["a", "b", "c", "d", "e", "f"]);
    let t = s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32;
    run(&mut s, "select.range", json!({"anchor": cell_at(t, 0, 0), "focus": cell_at(t, 1, 0)}));
    run(&mut s, "table.merge", json!({}));
    run(&mut s, "select.range", json!({"anchor": cell_at(t, 1, 1), "focus": cell_at(t, 2, 1)}));
    run(&mut s, "table.toText", json!({}));
    let head = s.doc.body[t as usize].as_table().unwrap();
    assert_eq!(head.rows.len(), 1);
    assert_eq!(head.rows[0].cells[0].props.vmerge, VMerge::None, "no merge goes on below the table");

    // A floating table keeps its place in the part above; the part below goes in the text flow.
    let mut s = typed_table(2, &["a", "b", "c", "d", "e", "f"]);
    let t = s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32;
    let float = wordcraft_doc::props::TableFloat { x: 100.0, y: 200.0, overlap: true, ..Default::default() };
    s.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![t])).unwrap().props.float = Some(float);
    run(&mut s, "select.range", json!({"anchor": cell_at(t, 1, 0), "focus": cell_at(t, 1, 1)}));
    run(&mut s, "table.toText", json!({}));
    let at = t as usize;
    assert_eq!(s.doc.body[at].as_table().unwrap().props.float, Some(float));
    assert_eq!(s.doc.body[at + 2].as_table().unwrap().props.float, None, "the part below doesn't sit on the part above");

    // The caret alone in a cell converts every row, as before.
    let mut s = typed_table(2, &["a", "b", "c", "d", "e", "f"]);
    run(&mut s, "table.toText", json!({}));
    assert!(s.doc.body.iter().all(|b| b.as_table().is_none()));
}

/// A content control around the table keeps both its ends when the first rows, the last rows
/// or every row become text.
#[test]
fn convert_to_text_keeps_a_content_control_around_the_table() {
    use wordcraft_doc::{ContentControl, ControlWrap, InlineObject};
    let has = |p: &wordcraft_doc::Paragraph, start: bool| {
        p.objects.iter().any(|o| if start { matches!(o, InlineObject::ControlStart { .. }) } else { matches!(o, InlineObject::ControlEnd) })
    };
    for rows in [Some((0, 0)), Some((2, 2)), None] {
        let mut s = typed_table(2, &["a", "b", "c", "d", "e", "f"]);
        let t = s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32;
        s.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![t])).unwrap().controls =
            ControlWrap { open: vec![ContentControl { tag: "wrap".into(), ..Default::default() }], close: 1 };
        assert!(!s.doc.has_unbalanced_controls());
        match rows {
            Some((r0, r1)) => run(&mut s, "select.range", json!({"anchor": cell_at(t, r0, 0), "focus": cell_at(t, r1, 1)})),
            None => run(&mut s, "select.range", json!({"anchor": cell_at(t, 1, 0), "focus": cell_at(t, 1, 0)})),
        };
        run(&mut s, "table.toText", json!({}));
        assert!(!s.doc.has_unbalanced_controls(), "{rows:?}: {:?}", body_outline(&s));
        let at = t as usize;
        if rows != Some((2, 2)) {
            assert!(s.doc.body[at].as_para().is_some_and(|p| has(p, true)), "{rows:?}: the control starts on the text");
        }
        if rows != Some((0, 0)) {
            let last = s.doc.body.iter().skip(at).rfind(|b| b.as_para().is_some_and(|p| !p.text.is_empty()) || b.as_table().is_some()).unwrap();
            assert!(last.as_para().is_some_and(|p| has(p, false)), "{rows:?}: the control ends on the text");
        }
    }

    // A control around rows 0 to 3, row 1 converted: the table above closes it, the table below
    // opens it again.
    let mut s = typed_table(2, &["a", "b", "c", "d", "e", "f", "g", "h"]);
    let t = s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32;
    {
        let table = s.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![t])).unwrap();
        table.rows[0].controls.open = vec![ContentControl { id: Some(7), tag: "rows".into(), ..Default::default() }];
        table.rows[3].controls.close = 1;
    }
    run(&mut s, "select.range", json!({"anchor": cell_at(t, 1, 0), "focus": cell_at(t, 1, 1)}));
    run(&mut s, "table.toText", json!({}));
    let at = t as usize;
    let head = s.doc.body[at].as_table().unwrap();
    let tail = s.doc.body[at + 2].as_table().unwrap();
    assert_eq!((head.rows.len(), head.rows[0].controls.close), (1, 1), "the table above closes the control");
    assert_eq!(tail.rows[0].controls.open.iter().map(|c| c.tag.as_str()).collect::<Vec<_>>(), ["rows"], "the table below opens it");
    assert_eq!((head.rows[0].controls.open[0].id, tail.rows[0].controls.open[0].id), (Some(7), None), "one w:id in the file");
    assert_eq!(tail.rows[1].controls.close, 1);
    assert!(!s.doc.has_unbalanced_controls());
}

/// The text keeps its formatting, and a table nested in a cell stays a table.
#[test]
fn convert_to_text_keeps_formatting_and_nested_tables() {
    let mut s = typed_table(2, &["Name", "X"]);
    let t = s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32;
    let first = json!({"story": "body", "path": [t, 0, 0, 0], "off": 0});
    let end = json!({"story": "body", "path": [t, 0, 0, 0], "off": 4});
    run(&mut s, "select.range", json!({"anchor": first, "focus": end}));
    run(&mut s, "format.bold", json!({}));
    // A table nested in the second cell, after its text.
    s.sel = crate::Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![t, 0, 1, 0]), off: 1 });
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    run(&mut s, "text.insert", json!({"text": "Inner"}));
    s.sel = crate::Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![t, 0, 0, 0]), off: 0 });
    run(&mut s, "table.toText", json!({}));
    let at = t as usize;
    let line = s.doc.body[at].as_para().unwrap();
    assert_eq!(line.text, "Name\tX");
    assert_eq!(line.props_of_char(0).bold, Some(true), "the text lost its bold");
    assert_ne!(line.props_of_char(4).bold, Some(true), "the tab takes no formatting");
    let inner = s.doc.body.iter().skip(at + 1).find_map(|b| b.as_table()).expect("the nested table stays a table");
    assert_eq!(table_cell_text(inner), "Inner");

    // A selected row whose first cell starts with a nested table: the caret goes to the row's
    // text, not back into the table above.
    let mut s = typed_table(2, &["A", "B", "", "D"]);
    let t = s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32;
    s.sel = crate::Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![t, 1, 0, 0]), off: 0 });
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    let row = &s.doc.body[t as usize].as_table().unwrap().rows[1];
    assert!(row.cells[0].blocks.first().is_some_and(|b| b.as_table().is_some()), "the cell starts with the nested table");
    run(&mut s, "select.range", json!({"anchor": cell_at(t, 1, 1), "focus": cell_at(t, 1, 1)}));
    s.sel.anchor = wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![t, 1, 0, 1]), off: 0 };
    run(&mut s, "table.toText", json!({}));
    let at = t as usize;
    assert_eq!(body_outline(&s).get(at..at + 3), Some(&["T", "T", "\tD"].map(String::from)[..]), "{:?}", body_outline(&s));
    assert_eq!(s.sel.focus.path.0, [t + 2], "the caret is on the row's text");
}

fn table_cell_text(t: &wordcraft_doc::Table) -> String {
    t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks).filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect()
}

#[test]
fn delete_columns_deletes_every_selected_column() {
    // Issue #431: only the selection's focus column went.
    let cell = |r: u32, c: u32, off: usize| json!({"story": "body", "path": [0, r, c, 0], "off": off});
    for (anchor, focus) in [(cell(0, 0, 0), cell(1, 1, 1)), (cell(1, 1, 1), cell(0, 0, 0))] {
        let mut s = table_from("Left\tMiddle\tRight\nA\tB\tC");
        run(&mut s, "select.range", json!({"anchor": anchor, "focus": focus}));
        run(&mut s, "table.deleteColumn", json!({}));
        assert_eq!(table_texts(&s), [["Right"], ["C"]]);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(table_texts(&s), [["Left", "Middle", "Right"], ["A", "B", "C"]]);
    }
    // A caret deletes its own column only.
    let mut s = table_from("Left\tMiddle\tRight");
    run(&mut s, "caret.set", json!({"pos": cell(0, 1, 0)}));
    run(&mut s, "table.deleteColumn", json!({}));
    assert_eq!(table_texts(&s), [["Left", "Right"]]);
}

#[test]
fn split_cells_leaves_the_other_rows_alone() {
    // Issue #432: splitting a cell inserted a grid column with a new cell in every row.
    let spans = |s: &Session| -> Vec<Vec<usize>> {
        let t = s.doc.body.iter().find_map(|b| b.as_table()).expect("a table");
        t.rows.iter().map(|r| r.cells.iter().map(wordcraft_doc::Cell::span).collect()).collect()
    };
    let mut s = table_from("Left\nA");
    let width: f32 = s.doc.body.iter().find_map(|b| b.as_table()).unwrap().total_width();
    run(&mut s, "table.split", json!({"columns": 2}));
    assert_eq!(table_texts(&s), [vec!["Left", ""], vec!["A"]]);
    assert_eq!(spans(&s), [vec![1, 1], vec![2]]);
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert!((t.total_width() - width).abs() < 0.1, "{:?}", t.grid);
    assert!((t.grid[0] - width / 2.0).abs() < 0.1, "{:?}", t.grid);
    let mut s = table_from("Left\tMiddle\tRight\nA\tB\tC");
    run(&mut s, "table.split", json!({"columns": 2}));
    assert_eq!(table_texts(&s), [vec!["Left", "", "Middle", "Right"], vec!["A", "B", "C"]]);
    assert_eq!(spans(&s), [vec![1, 1, 1, 1], vec![2, 1, 1]]);
}

fn table_spans(s: &Session) -> Vec<Vec<usize>> {
    let t = s.doc.body.iter().find_map(|b| b.as_table()).expect("a table");
    t.rows.iter().map(|r| r.cells.iter().map(wordcraft_doc::Cell::span).collect()).collect()
}

fn table_vmerge(s: &Session) -> Vec<Vec<wordcraft_doc::props::VMerge>> {
    let t = s.doc.body.iter().find_map(|b| b.as_table()).expect("a table");
    t.rows.iter().map(|r| r.cells.iter().map(|c| c.props.vmerge).collect()).collect()
}

/// #2592: Split Cells into columns and rows. A one-row cell gets new rows under it, the row's
/// other cells merging down over them; one undo puts the table back.
#[test]
fn split_cells_into_columns_and_rows() {
    use wordcraft_doc::props::VMerge::{Continue, None as No, Restart};
    let mut s = table_from("A\tB\nC\tD");
    let depth = s.undo_depth();
    run(&mut s, "table.split", json!({"columns": 2, "rows": 2}));
    assert_eq!(s.undo_depth(), depth + 1);
    assert_eq!(table_texts(&s), [vec!["A", "", "B"], vec!["", "", ""], vec!["C", "D"]]);
    assert_eq!(table_spans(&s), [vec![1, 1, 1], vec![1, 1, 1], vec![2, 1]]);
    assert_eq!(table_vmerge(&s), [vec![No, No, Restart], vec![No, No, Continue], vec![No, No]]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(table_texts(&s), [["A", "B"], ["C", "D"]]);
    assert_eq!(table_spans(&s), [[1, 1], [1, 1]]);
    // Rows only.
    let mut s = table_from("A\tB");
    run(&mut s, "table.split", json!({"columns": 1, "rows": 3}));
    assert_eq!(table_texts(&s), [vec!["A", "B"], vec!["", ""], vec!["", ""]]);
    assert_eq!(table_vmerge(&s), [[No, Restart], [No, Continue], [No, Continue]]);
}

/// #2592: Merge cells before split merges the selection, then splits the merged cell; without
/// it every selected cell is split on its own.
#[test]
fn split_cells_merge_before_split() {
    use wordcraft_doc::props::VMerge::None as No;
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let mut s = table_from("A\tB\nC\tD");
    run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 1)}));
    let depth = s.undo_depth();
    run(&mut s, "table.split", json!({"columns": 2, "rows": 2}));
    assert_eq!(s.undo_depth(), depth + 1, "merge and split are one step");
    assert_eq!(table_spans(&s), [[1, 1], [1, 1]]);
    assert_eq!(table_vmerge(&s), [[No, No], [No, No]]);
    assert_eq!(table_texts(&s), [["ABCD", ""], ["", ""]]);

    let mut s = table_from("A\tB\nC\tD");
    run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 1)}));
    run(&mut s, "table.split", json!({"columns": 2, "mergeFirst": false}));
    assert_eq!(table_texts(&s), [["A", "", "B", ""], ["C", "", "D", ""]]);
    assert_eq!(table_spans(&s), [[1, 1, 1, 1], [1, 1, 1, 1]]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(table_texts(&s), [["A", "B"], ["C", "D"]]);

    // As in Word: merged cells split into at most as many rows as were selected, and cells split
    // one by one keep their rows.
    let mut s = table_from("A\tB\nC\tD");
    run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 0)}));
    let depth = s.undo_depth();
    assert!(s.run("table.split", &json!({"columns": 1, "rows": 3})).is_err(), "rows 1 to 2 only");
    assert!(s.run("table.split", &json!({"columns": 2, "rows": 2, "mergeFirst": false})).is_err());
    assert_eq!(s.undo_depth(), depth);
    assert_eq!(table_texts(&s), [["A", "B"], ["C", "D"]]);
    run(&mut s, "table.split", json!({"columns": 1, "rows": 2}));
    assert_eq!(table_texts(&s), [["AC", "B"], ["", "D"]]);
}

/// #2592: a vertically merged cell splits into rows that divide its height; other counts are
/// refused without changing anything.
#[test]
fn split_cells_of_a_vertically_merged_cell() {
    use wordcraft_doc::props::VMerge::{Continue, None as No, Restart};
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let merged = || {
        let mut s = table_from("A\tB\nC\tD\nE\tF\nG\tH");
        run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(3, 0)}));
        run(&mut s, "table.merge", json!({}));
        s
    };
    let mut s = merged();
    assert_eq!(table_vmerge(&s), [[Restart, No], [Continue, No], [Continue, No], [Continue, No]]);
    run(&mut s, "table.split", json!({"columns": 2, "rows": 2}));
    assert_eq!(s.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows.len(), 4, "no new rows");
    assert_eq!(table_vmerge(&s), [[Restart, Restart, No], [Continue, Continue, No], [Restart, Restart, No], [Continue, Continue, No]]);
    assert_eq!(table_spans(&s), [[1, 1, 1], [1, 1, 1], [1, 1, 1], [1, 1, 1]]);
    // The caret in a continued cell splits the whole merged cell.
    let mut s = merged();
    run(&mut s, "caret.set", json!({"pos": cell(2, 0)}));
    run(&mut s, "table.split", json!({"columns": 1, "rows": 4}));
    assert_eq!(table_vmerge(&s), [[No, No], [No, No], [No, No], [No, No]]);
    let mut s = merged();
    let (depth, before) = (s.undo_depth(), table_vmerge(&s));
    let e = s.run("table.split", &json!({"columns": 1, "rows": 3})).unwrap_err();
    assert!(matches!(e, crate::CmdError::Params(_)), "{e:?}");
    assert!(e.to_string().contains("The number must be a divisor of 4"), "3 doesn't divide 4: {e}");
    assert_eq!((s.undo_depth(), table_vmerge(&s)), (depth, before));
    // 1 × 1 on the 4-row cell changes nothing either.
    let mut s = merged();
    run(&mut s, "table.split", json!({"columns": 1, "rows": 1}));
    assert_eq!(table_vmerge(&s), [[Restart, No], [Continue, No], [Continue, No], [Continue, No]]);
}

/// #2592 review: cells split one by one split a vertically merged cell once, not once per row it
/// covers, and every other selected cell in two.
#[test]
fn split_cells_one_by_one_with_a_merged_cell() {
    use wordcraft_doc::props::VMerge::{Continue, None as No, Restart};
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let merged = || {
        let mut s = table_from("A\tB\nC\tD");
        run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 0)}));
        run(&mut s, "table.merge", json!({}));
        s
    };
    let mut s = merged();
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "table.split", json!({"columns": 2, "mergeFirst": false}));
    assert_eq!(table_spans(&s), [[1, 1, 1, 1], [1, 1, 1, 1]]);
    assert_eq!(table_texts(&s), [["AC", "", "B", ""], ["", "", "D", ""]]);
    assert_eq!(table_vmerge(&s), [[Restart, Restart, No, No], [Continue, Continue, No, No]]);
    // The merged cell alone is one cell.
    let mut s = merged();
    run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 0)}));
    run(&mut s, "table.split", json!({"columns": 2, "mergeFirst": false}));
    assert_eq!(table_spans(&s), [[1, 1, 1], [1, 1, 1]]);
    assert_eq!(table_vmerge(&s), [[Restart, Restart, No], [Continue, Continue, No]]);
}

/// #2592: Convert Text to Table separates at paragraphs, commas, tabs or another character,
/// takes a column count and an AutoFit behaviour, and undoes in one step.
#[test]
fn convert_text_to_table_dialog_options() {
    let convert = |text: &str, params: serde_json::Value| {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": text}));
        run(&mut s, "select.all", json!({}));
        let depth = s.undo_depth();
        run(&mut s, "table.fromText", params);
        assert_eq!(s.undo_depth(), depth + 1);
        s
    };
    let s1 = convert("a;b;c\nd;e", json!({"separator": "other", "char": ";"}));
    assert_eq!(table_texts(&s1), [["a", "b", "c"], ["d", "e", ""]]);
    let s1 = convert("a\nb\nc\nd", json!({"separator": "paragraph"}));
    assert_eq!(table_texts(&s1), [["a"], ["b"], ["c"], ["d"]]);
    let s1 = convert("a\nb\nc", json!({"separator": "paragraph", "columns": 2}));
    assert_eq!(table_texts(&s1), [["a", "b"], ["c", ""]]);
    // More columns than fields pads; fewer wraps a paragraph's fields onto more rows.
    let s1 = convert("a\tb\tc\nd", json!({"columns": 4}));
    assert_eq!(table_texts(&s1), [["a", "b", "c", ""], ["d", "", "", ""]]);
    let s1 = convert("a\tb\tc\nd", json!({"columns": 2}));
    assert_eq!(table_texts(&s1), [["a", "b"], ["c", ""], ["d", ""]]);
    let mut s1 = convert("a,b\nc,d", json!({"separator": "comma", "autofit": "fixed"}));
    let t = |s: &Session| s.doc.body.iter().find_map(|b| b.as_table()).unwrap().clone();
    assert!(t(&s1).props.fixed);
    run(&mut s1, "edit.undo", json!({}));
    assert_eq!(body_texts(&s1), ["a,b", "c,d"]);
    let s1 = convert("a,b\nc,d", json!({"separator": "comma", "autofit": "window"}));
    assert_eq!((t(&s1).props.fixed, t(&s1).props.width_pct), (false, Some(100.0)));
    let s1 = convert("a,b\nc,d", json!({"separator": "comma", "autofit": "contents"}));
    let width = cmd::page::sect(&s1).text_width();
    assert!(!t(&s1).props.fixed && t(&s1).total_width() < width / 2.0, "{:?}", t(&s1).grid);
    // Junk is refused or clamped, never a panic.
    let mut s1 = s();
    run(&mut s1, "document.setText", json!({"text": "a-b"}));
    run(&mut s1, "select.all", json!({}));
    assert!(s1.run("table.fromText", &json!({"separator": "other"})).is_err(), "no character");
    assert!(s1.run("table.fromText", &json!({"separator": "other", "char": ""})).is_err());
    assert!(s1.run("table.fromText", &json!({"separator": "nonsense"})).is_err());
    assert!(s1.run("table.fromText", &json!({"autofit": "nonsense"})).is_err());
    assert_eq!(body_texts(&s1), ["a-b"]);
    run(&mut s1, "table.fromText", json!({"separator": "other", "char": "-", "columns": 1_000_000_000_u64}));
    assert_eq!(table_texts(&s1).first().map(Vec::len), Some(63));
}

/// #2592, as Word does: Initial column width Auto gives equal columns across the text width;
/// a width gives every column that preferred width (kept even when the table is drawn narrower
/// to fit the text width), with no table width; both leave the table resizing to its contents.
#[test]
fn convert_text_to_table_initial_column_width() {
    let convert = |params: serde_json::Value| {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": "a\tb\tc\nd\te\tf"}));
        run(&mut s, "select.all", json!({}));
        let r = s.run("table.fromText", &params);
        (s, r)
    };
    let t = |s: &Session| s.doc.body.iter().find_map(|b| b.as_table()).unwrap().clone();
    let (s1, _) = convert(json!({"autofit": "initial"}));
    let width = cmd::page::sect(&s1).text_width();
    let tb = t(&s1);
    assert!(!tb.props.fixed && tb.props.width.is_none());
    assert!(tb.grid.iter().all(|&w| (w - width / 3.0).abs() < 0.1), "{:?}", tb.grid);
    let (s0, _) = convert(json!({"autofit": "initial", "width": 0}));
    assert_eq!(t(&s0).grid, tb.grid, "0 is Auto");

    let (s1, _) = convert(json!({"autofit": "initial", "width": 72}));
    let tb = t(&s1);
    assert_eq!(tb.grid, [72.0, 72.0, 72.0]);
    assert!(tb.rows.iter().flat_map(|r| &r.cells).all(|c| c.props.width == Some(72.0)));
    assert!(!tb.props.fixed && tb.props.width.is_none());

    // 5 in columns are stored as such but drawn within the text width.
    let (mut s1, _) = convert(json!({"width": 360}));
    assert_eq!(t(&s1).grid, [360.0, 360.0, 360.0]);
    let drawn: f32 = s1.layout().pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Cell { rect, row: 0, .. } => Some(rect.w),
            _ => None,
        })
        .sum();
    assert!(drawn > 0.0 && drawn <= width + 0.5, "{drawn} vs {width}");

    // A width is the Initial column width only: AutoFit to contents or the window ignores it.
    for mode in ["contents", "window"] {
        let (with, _) = convert(json!({"autofit": mode, "width": 72}));
        let (without, _) = convert(json!({ "autofit": mode }));
        let cell_widths = |s: &Session| t(s).rows.iter().flat_map(|r| r.cells.iter().map(|c| c.props.width)).collect::<Vec<_>>();
        assert_eq!((t(&with).grid, cell_widths(&with)), (t(&without).grid, cell_widths(&without)), "{mode}");
    }

    // More than 22 in, or less than nothing, is refused without converting.
    for bad in [json!(1585), json!(-1), json!("wide")] {
        let (s1, r) = convert(json!({ "width": bad }));
        assert!(r.is_err(), "{bad}");
        assert!(s1.doc.body.iter().all(|b| b.as_table().is_none()));
    }
}

/// #2592, as Word does: a vertically merged cell splits into 1 to its height rows; its height
/// cuts it into one cell per row, 1 × 1 changes nothing, a number that doesn't divide the height or more than it is refused with Word's messages and
/// nothing changes, also when neighbours are merged with it first.
#[test]
fn split_cells_of_a_merged_cell_follow_word() {
    use wordcraft_doc::props::VMerge::{Continue, None as No, Restart};
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let merged = || {
        let mut s = table_from("A\tB\nC\tD\nE\tF\nG\tH");
        run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 0)}));
        run(&mut s, "table.merge", json!({}));
        s
    };
    for params in [json!({"columns": 1, "rows": 3}), json!({"columns": 1, "rows": 4}), json!({"columns": 2, "rows": 3})] {
        let mut s = merged();
        let (depth, before) = (s.undo_depth(), table_vmerge(&s));
        let e = s.run("table.split", &params).unwrap_err().to_string();
        assert!(e.contains("The number must be between 1 and 2"), "{e}");
        assert_eq!((s.undo_depth(), table_vmerge(&s)), (depth, before));
    }
    // 1 × 1 on a merged cell is a no-op (Word, 2 to 5 rows).
    let mut s = merged();
    let before = table_vmerge(&s);
    run(&mut s, "table.split", json!({"columns": 1, "rows": 1}));
    assert_eq!(table_vmerge(&s), before);
    {
        let params = json!({"columns": 1, "rows": 2});
        let mut s = merged();
        run(&mut s, "table.split", params.clone());
        assert_eq!(table_vmerge(&s), [[No, No], [No, No], [No, No], [No, No]], "{params}");
        assert_eq!(table_texts(&s)[0][0], "AC");
        assert_eq!(table_texts(&s)[1][0], "");
    }
    // A 3-row cell: 2 rows is not a divisor (Word's alert, nothing changes); 1 × 1 is a no-op; 3 rows gives one cell per row.
    let tall = || {
        let mut s = table_from("A\tB\nC\tD\nE\tF\nG\tH");
        run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(2, 0)}));
        run(&mut s, "table.merge", json!({}));
        s
    };
    for params in [json!({"columns": 1, "rows": 2}), json!({"columns": 2, "rows": 2})] {
        let mut s = tall();
        let (depth, before) = (s.undo_depth(), table_vmerge(&s));
        let e = s.run("table.split", &params).unwrap_err();
        assert!(matches!(e, crate::CmdError::Params(_)), "{e:?}");
        assert!(e.to_string().contains("The number must be a divisor of 3"), "{e}");
        assert_eq!((s.undo_depth(), table_vmerge(&s)), (depth, before));
    }
    let mut s = tall();
    run(&mut s, "table.split", json!({"columns": 1, "rows": 1}));
    assert_eq!(table_vmerge(&s), [[Restart, No], [Continue, No], [Continue, No], [No, No]]);
    assert_eq!(table_texts(&s)[0][0], "ACE");
    let mut s = tall();
    run(&mut s, "table.split", json!({"columns": 1, "rows": 3}));
    assert_eq!(table_vmerge(&s), [[No, No], [No, No], [No, No], [No, No]]);
    // Merge cells before split, then 1 × 1: one cell, as before (merging through Split Cells).
    let mut s = table_from("A\tB\nC\tD");
    run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 1)}));
    run(&mut s, "table.split", json!({"columns": 1, "rows": 1}));
    assert_eq!(table_spans(&s), [[2], [2]]);
    assert_eq!(table_vmerge(&s), [[wordcraft_doc::props::VMerge::Restart], [wordcraft_doc::props::VMerge::Continue]]);
    // The merged cell and its neighbours, merged first: the limit is the selection's height.
    let mut s = merged();
    run(&mut s, "select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 1)}));
    let e = s.run("table.split", &json!({"columns": 1, "rows": 3})).unwrap_err().to_string();
    assert!(e.contains("The number must be between 1 and 2"), "{e}");
}

/// #2592, as Word does: a cell merged across columns splits into equal parts inside its old
/// width; the other rows' cells keep their widths.
#[test]
fn split_cells_across_columns_into_equal_parts() {
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let widths = |s: &Session| -> Vec<Vec<f32>> {
        let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        (0..t.rows.len())
            .map(|r| {
                let row = &t.rows[r];
                (0..row.cells.len()).map(|c| (0..row.cells[c].span()).map(|k| t.grid[t.grid_col(r, c) + k]).sum()).collect()
            })
            .collect()
    };
    for (across, parts) in [(2, 3), (3, 2), (2, 2)] {
        let mut s = table_from("A\tB\tC\nD\tE\tF");
        // Unequal columns, as in a real document.
        s.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap().grid = vec![100.0, 50.0, 150.0];
        run(&mut s, "select.range", json!({"anchor": cell(1, 0), "focus": cell(1, across - 1)}));
        run(&mut s, "table.merge", json!({}));
        let (top, old) = (widths(&s)[0].clone(), widths(&s)[1][0]);
        run(&mut s, "table.split", json!({"columns": parts, "rows": 1}));
        let w = widths(&s);
        let new: Vec<f32> = w[1].iter().take(parts as usize).copied().collect();
        assert!(new.iter().all(|x| (x - old / parts as f32).abs() < 0.1), "{across}->{parts}: {w:?}");
        let mut top_lines: Vec<f32> = Vec::new();
        let mut x = 0.0;
        for v in &w[0] {
            x += v;
            top_lines.push(x);
        }
        let mut expect = Vec::new();
        let mut x = 0.0;
        for v in &top {
            x += v;
            expect.push(x);
        }
        assert_eq!(top_lines.len(), expect.len(), "{across}->{parts}: {w:?}");
        assert!(top_lines.iter().zip(&expect).all(|(a, b)| (a - b).abs() < 0.1), "{across}->{parts}: {w:?}");
    }
}

/// #385: note options for the whole document or one section, and converting notes, which undoes
/// in one step.
#[test]
fn note_options_per_section_and_convert_notes() {
    use wordcraft_doc::PartKind;
    use wordcraft_doc::para::{InlineObject, NoteKind};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "One"}));
    let a = run(&mut s, "references.footnote", json!({"text": "First", "mark": "*"}))["id"].as_u64().unwrap() as u32;
    s.sel = crate::Selection::caret(s.doc.end_of(StoryRef::Body));
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "text.insert", json!({"text": "Two"}));
    let b = run(&mut s, "references.footnote", json!({"text": "Second"}))["id"].as_u64().unwrap() as u32;
    s.sel = crate::Selection::caret(s.doc.end_of(StoryRef::Body));
    // The second section restarts at 3 in letters; the first keeps the document's options.
    let got = run(&mut s, "references.noteOptions", json!({"numFmt": "lowerLetter", "numStart": 3, "numRestart": "eachSect", "scope": "section"}));
    assert_eq!((got["footnote"]["numFmt"].as_str(), got["footnote"]["numStart"].as_u64()), (Some("lowerLetter"), Some(3)));
    let sections = s.doc.sections();
    assert!(sections[0].1.footnote_pr.is_empty());
    assert_eq!(sections[1].1.footnote_pr.num_start, Some(3));
    // Hostile or wrong values are refused or clamped.
    assert!(s.run("references.noteOptions", &json!({"kind": "endnote", "pos": "pageBottom"})).is_err());
    assert!(s.run("references.noteOptions", &json!({"kind": "endnote", "numRestart": "eachPage"})).is_err());
    let got = run(&mut s, "references.noteOptions", json!({"kind": "endnote", "pos": "sectEnd", "numStart": -5}));
    assert_eq!((got["endnote"]["pos"].as_str(), got["endnote"]["numStart"].as_u64()), (Some("sectEnd"), Some(1)));
    // The whole document: the section's own start gives way.
    run(&mut s, "references.noteOptions", json!({"numStart": 2}));
    assert_eq!(s.doc.sections()[1].1.footnote_pr.num_start, None);
    assert_eq!(s.doc.settings.footnote_pr.num_start, Some(2));
    // Convert footnotes to endnotes: kinds, marks and styles; the custom mark stays. One undo.
    let kinds = |s: &Session| -> Vec<NoteKind> {
        let mut v = Vec::new();
        s.doc.objects_in_reading_order(&mut |o| {
            if let InlineObject::NoteRef { kind, .. } = o {
                v.push(*kind);
            }
        });
        v
    };
    assert_eq!(run(&mut s, "references.convertNotes", json!({"to": "endnote"}))["converted"], 2);
    assert_eq!(kinds(&s), vec![NoteKind::Endnote, NoteKind::Endnote]);
    assert!([a, b].iter().all(|id| s.doc.parts[id].kind == PartKind::Endnote));
    let note = s.doc.para(StoryRef::Part(a), &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(note.props.style.as_deref(), Some("EndnoteText"));
    assert!(matches!(&note.objects[0], InlineObject::NoteRef { kind: NoteKind::Endnote, custom, .. } if custom == "*"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(kinds(&s), vec![NoteKind::Footnote, NoteKind::Footnote]);
    assert_eq!(s.doc.parts[&b].kind, PartKind::Footnote);
    assert!(s.run("references.convertNotes", &json!({"to": "sideways"})).is_err());
}

#[test]
fn next_footnote_skips_endnotes() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "A"}));
    run(&mut s, "references.footnote", json!({"text": "Fn1"}));
    s.sel = crate::Selection::caret(s.doc.end_of(StoryRef::Body));
    run(&mut s, "text.insert", json!({"text": "B"}));
    run(&mut s, "references.endnote", json!({"text": "En1"}));
    s.sel = crate::Selection::caret(s.doc.end_of(StoryRef::Body));
    run(&mut s, "text.insert", json!({"text": "C"}));
    run(&mut s, "references.footnote", json!({"text": "Fn2"}));

    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "references.nextFootnote", json!({}));
    assert_eq!(s.sel.focus.off, 1);

    run(&mut s, "references.nextFootnote", json!({}));
    assert_eq!(s.sel.focus.off, 9);
}

/// A tiny synthetic Polish Hunspell dictionary (written for these tests). Real dictionaries are
/// installed on the system, never bundled; every Polish test installs this same one.
fn polish_test_dictionary() {
    wordcraft_proof::pl::install_dictionary(POLISH_TEST_AFF, POLISH_TEST_DIC).unwrap();
}

const POLISH_TEST_AFF: &str = "SET UTF-8\nTRY aioeznrwcyszżtkpdmłjlubąęgh\nMAP 2\nMAP oó\nMAP zż\nREP 2\nREP ż rz\nREP rz ż\n";
const POLISH_TEST_DIC: &str = "12\npłynie\nrzeka\na\nto\nwiem\nże\ntak\nw\nogóle\nzażółć\ngęślą\njaźń\n";

/// A tiny synthetic Portuguese Hunspell dictionary (written for these tests). Real dictionaries
/// are installed on the system, never bundled; every Portuguese test installs this same one.
fn portuguese_test_dictionary() {
    wordcraft_proof::pt::install_dictionary(PORTUGUESE_TEST_AFF, PORTUGUESE_TEST_DIC).unwrap();
}

const PORTUGUESE_TEST_AFF: &str =
    "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD F\nTRY aeiousrntmdlcpvgqbzfhjxçãõáéíóúâêô\nMAP 2\nMAP aáã\nMAP eé\nREP 1\nREP seje seja\n";
const PORTUGUESE_TEST_DIC: &str = "11\nsaudade\ncoração\ne\nele\ndisse\nnão\né\nque\nseja\nfrequencia/F\nmuito\n";

/// #1063: File › Options › Proofing › Dictionaries folder. A folder of Hunspell files is saved,
/// searched first and reported per language; a missing folder or a relative path is refused and
/// changes nothing. The folder holds the same test dictionaries the other tests install, so tests
/// running meanwhile see the same words; it stays set for the rest of the run for that reason.
#[test]
fn dictionaries_folder_option_is_saved_searched_and_reported() {
    let dir = std::env::temp_dir().join(format!("wc-engine-dictionaries-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, aff, dic) in [
        ("pl_PL", POLISH_TEST_AFF, POLISH_TEST_DIC),
        ("pt_BR", PORTUGUESE_TEST_AFF, PORTUGUESE_TEST_DIC),
        ("id_ID", INDONESIAN_TEST_AFF, INDONESIAN_TEST_DIC),
        // The others the tests install, so reloading from the folder keeps their words.
        ("it_IT", ITALIAN_TEST_AFF, ITALIAN_TEST_DIC),
        ("es_ES", SPANISH_TEST_AFF, SPANISH_TEST_DIC),
        ("es_UY", SPANISH_TEST_AFF, URUGUAYAN_TEST_DIC),
        // #3252: set_dictionaries_folder() now reloads Czech too (it used to leave its cache
        // alone), so a run of this test overlapping czech_proofing_through_the_review_commands
        // would otherwise force a reload that finds no cs_CZ here and empties it mid-test. Same
        // fixture content either way keeps such a race harmless, as it already does for Polish.
        ("cs_CZ", czech_test_dict::AFF, czech_test_dict::DIC),
    ] {
        std::fs::write(dir.join(format!("{name}.aff")), aff).unwrap();
        std::fs::write(dir.join(format!("{name}.dic")), dic).unwrap();
    }
    let mut s = s();
    let missing = dir.join("no-such-folder");
    assert!(s.run("file.options", &json!({"dictionariesFolder": missing.to_string_lossy()})).is_err());
    assert!(s.run("file.options", &json!({"dictionariesFolder": "relative/dictionaries"})).is_err());
    assert!(s.run("file.options", &json!({"dictionariesFolder": 7})).is_err());
    assert_eq!(s.prefs.dictionaries_folder, None, "a refused folder changes nothing");
    let r = run(&mut s, "file.options", json!({"dictionariesFolder": dir.to_string_lossy()}));
    assert_eq!(s.prefs.dictionaries_folder.as_deref(), Some(dir.to_string_lossy().as_ref()));
    assert_eq!(r["dictionariesFolder"], json!(dir.to_string_lossy()));
    let dicts = r["dictionaries"].as_array().unwrap();
    let file = |lang: &str| dicts.iter().find(|d| d["language"] == lang).map(|d| d["file"].clone());
    assert_eq!(file("pl"), Some(json!(dir.join("pl_PL.dic").to_string_lossy())));
    assert_eq!(file("pt"), Some(json!(dir.join("pt_BR.dic").to_string_lossy())));
    assert_eq!(file("id"), Some(json!(dir.join("id_ID.dic").to_string_lossy())));
    // Each Spanish variety reports its own file (Spain, Mexico, Latin America, Uruguay, in order).
    let spanish: Vec<serde_json::Value> = dicts.iter().filter(|d| d["language"] == "es").map(|d| d["file"].clone()).collect();
    assert_eq!(spanish.len(), 4);
    assert_eq!(spanish.first(), Some(&json!(dir.join("es_ES.dic").to_string_lossy())));
    assert_eq!(spanish.last(), Some(&json!(dir.join("es_UY.dic").to_string_lossy())));
    // Polish text is checked against the folder's dictionary.
    run(&mut s, "document.setText", json!({"text": "rzeka płynie żeka"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "pl-PL"}));
    let issues = run(&mut s, "review.issues", json!({}));
    let words: Vec<&str> = issues.as_array().unwrap().iter().filter(|i| i["kind"] == "spelling").filter_map(|i| i["text"].as_str()).collect();
    assert_eq!(words, ["żeka"], "{issues}");
    // Saved with the settings.
    let prefs = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(prefs["dictionariesFolder"], json!(dir.to_string_lossy()));
}

/// Review › Language and the proofing commands work in Portuguese end to end: the language makes
/// the document's words Portuguese, F7 and the issue list use the Portuguese dictionary and
/// grammar, suggestions come from it, and a suggestion fixes the text.
#[test]
fn portuguese_proofing_through_the_review_commands() {
    portuguese_test_dictionary();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Saudade coraçao, e ele ele disse."}));
    // English by default: the Portuguese words are English misspellings.
    assert!(run(&mut s, "review.issues", json!({})).as_array().unwrap().len() >= 5);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "pt_br", "default": true}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("pt-BR"), "the tag is normalized");
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "pt-BR");
    let issues = run(&mut s, "review.issues", json!({}));
    let arr = issues.as_array().unwrap();
    let has = |text: &str, kind: &str, sugg: &str| {
        arr.iter()
            .any(|i| i["text"] == text && i["kind"] == kind && i["lang"] == "pt" && i["suggestions"].as_array().unwrap().iter().any(|x| x == sugg))
    };
    assert!(has("coraçao", "spelling", "coração"), "{issues}");
    assert!(has("ele ele", "grammar", "ele"), "{issues}");
    assert_eq!(arr.len(), 2, "{issues}");
    assert!(arr.iter().any(|i| i["message"] == "Palavra repetida: \"ele\""), "{issues}");
    // F7 from the start selects the first issue; Change applies a suggestion.
    run(&mut s, "caret.docStart", json!({}));
    let first = run(&mut s, "review.spelling", json!({}));
    assert_eq!(first["text"], "coraçao");
    run(&mut s, "review.applySuggestion", json!({"text": "coração"}));
    assert!(text(&s).starts_with("Saudade coração,"), "{}", text(&s));
    // Add to Dictionary works for Portuguese words too.
    run(&mut s, "review.addToDictionary", json!({"word": "coraçao"}));
    let left = run(&mut s, "review.issues", json!({}));
    assert_eq!(left.as_array().unwrap().len(), 1, "{left}");
}

fn indonesian_test_dictionary() {
    wordcraft_proof::id::install_dictionary(INDONESIAN_TEST_AFF, INDONESIAN_TEST_DIC).unwrap();
}

// Hand-written synthetic fixture (no system dictionary data).
const INDONESIAN_TEST_AFF: &str = "SET UTF-8\nFLAG UTF-8\nTRY aniekrumgtslhbdopy\nSFX A Y 1\nSFX A 0 an .\n";
const INDONESIAN_TEST_DIC: &str = "8\nrumah\nsekolah\nanak\nbermain\ndi\nmakan/A\nsaya\nkami\n";

/// Indonesian proofing (#498) through the same commands as Review, F7, Change and Add to
/// Dictionary.
#[test]
fn indonesian_proofing_through_the_review_commands() {
    indonesian_test_dictionary();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Saya di sekolha, anak anak bermain."}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "id_id", "default": true}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("id-ID"));
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "id-ID");
    let issues = run(&mut s, "review.issues", json!({}));
    let arr = issues.as_array().unwrap();
    assert_eq!(arr.len(), 2, "{issues}");
    assert!(arr.iter().all(|i| i["lang"] == "id"), "{issues}");
    assert!(
        arr.iter().any(|i| i["text"] == "sekolha" && i["kind"] == "spelling" && i["suggestions"].as_array().unwrap().iter().any(|s| s == "sekolah")),
        "{issues}"
    );
    assert!(arr.iter().any(|i| i["text"] == "anak anak" && i["kind"] == "grammar" && i["suggestions"][0] == "anak-anak"), "{issues}");
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "review.spelling", json!({}))["text"], "sekolha");
    run(&mut s, "review.applySuggestion", json!({"text": "sekolah"}));
    assert!(text(&s).starts_with("Saya di sekolah,"), "{}", text(&s));
    run(&mut s, "select.text", json!({"text": "sekolah"}));
    run(&mut s, "text.insert", json!({"text": "zqkataidengine"}));
    assert_eq!(run(&mut s, "review.issues", json!({})).as_array().unwrap().len(), 2);
    run(&mut s, "review.addToDictionary", json!({"word": "zqkataidengine"}));
    let left = run(&mut s, "review.issues", json!({}));
    let left = left.as_array().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["kind"], "grammar");
    assert_eq!(left[0]["lang"], "id");
}

/// The spelling issues of the document: each word with its first suggestion.
fn spelling_with_first_suggestion(s: &mut Session) -> Vec<(String, String)> {
    let issues = run(s, "review.issues", json!({}));
    issues
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "spelling")
        .map(|i| (i["text"].as_str().unwrap().to_string(), i["suggestions"][0].as_str().unwrap_or("").to_string()))
        .collect()
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// English (UK), (Canada) and (Australia) through Review › Language (#1281, #1045): each accepts
/// its own spellings and flags the other dialect's with its own form suggested first, F7 finds
/// it and Change fixes it.
#[test]
fn english_dialects_through_the_review_commands() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "The colour of the center."}));
    // US English by default: the British spelling is flagged, the American one isn't.
    assert_eq!(spelling_with_first_suggestion(&mut s), pairs(&[("colour", "color")]));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "en_gb", "default": true}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("en-GB"));
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "en-GB");
    assert_eq!(spelling_with_first_suggestion(&mut s), pairs(&[("center", "centre")]));
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "review.spelling", json!({}))["text"], "center");
    run(&mut s, "review.applySuggestion", json!({"text": "centre"}));
    assert_eq!(text(&s), "The colour of the centre.");
    assert!(spelling_with_first_suggestion(&mut s).is_empty());
    // Canada: British -our and -re, American -ize.
    run(&mut s, "document.setText", json!({"text": "We organise the colour and analyze the centre."}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "en-CA", "default": true}));
    assert_eq!(spelling_with_first_suggestion(&mut s), pairs(&[("organise", "organize")]));
    // Australia spells the British way: -ise is right, -yze isn't.
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "en-AU", "default": true}));
    assert_eq!(spelling_with_first_suggestion(&mut s), pairs(&[("analyze", "analyse")]));
    // Add to Dictionary overrules the dialect.
    run(&mut s, "review.addToDictionary", json!({"word": "analyze"}));
    assert!(spelling_with_first_suggestion(&mut s).is_empty());
    wordcraft_proof::remove_word("analyze");
}

/// A British document keeps its languages through .docx (`w:lang` in the defaults and on a run)
/// and is proofed the same way when it comes back.
#[test]
fn english_dialects_survive_a_docx_round_trip() {
    use std::io::Read;
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Colour and color. Centre and center."}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "en-GB", "default": true}));
    run(&mut s, "select.text", json!({"text": "Centre and center."}));
    run(&mut s, "review.language", json!({"lang": "en-US"}));
    let before = spelling_with_first_suggestion(&mut s);
    assert_eq!(before, pairs(&[("color", "colour"), ("Centre", "Center")]));
    let bytes = crate::io::save_bytes("british.docx", &s.doc).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut part = |name: &str| {
        let mut xml = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
        xml
    };
    assert!(part("word/styles.xml").contains(r#"w:val="en-GB""#), "the default language is written");
    assert!(part("word/document.xml").contains(r#"w:val="en-US""#), "the run's language is written");
    let doc = crate::io::open_bytes("british.docx", &bytes).unwrap();
    assert_eq!(doc.styles.default_chr.lang.as_deref(), Some("en-GB"));
    let mut back = Session::new(doc);
    assert_eq!(spelling_with_first_suggestion(&mut back), before);
}

/// An English interface writes new documents in the region's English (English (UK) on a British
/// system, #1045); another region's language leaves US English.
#[test]
fn new_documents_follow_the_regions_english() {
    let mut s = s();
    s.system_region = wordcraft_geom::Region::from_tag("en_GB.UTF-8");
    assert_eq!(s.region_english().as_deref(), Some("en-GB"));
    let tag = s.region_english();
    s.set_editing_language(tag.as_deref());
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("en-GB"), "the untouched document follows");
    run(&mut s, "document.setText", json!({"text": "Colour, color."}));
    assert_eq!(spelling_with_first_suggestion(&mut s), pairs(&[("color", "colour")]));
    // The region chosen in Options wins over the system's.
    run(&mut s, "file.options", json!({"region": "en-CA"}));
    assert_eq!(s.region_english().as_deref(), Some("en-CA"));
    for (tag, english) in [("pl-PL", None), ("en-US", None), ("en-AU", Some("en-AU")), ("auto", Some("en-GB"))] {
        run(&mut s, "file.options", json!({"region": tag}));
        assert_eq!(s.region_english().as_deref(), english, "{tag}");
    }
    s.system_region = wordcraft_geom::Region::from_tag("fr-FR");
    assert_eq!(s.region_english(), None);
}

/// A Spanish interface writes new documents in the region's Spanish (#3360): Uruguay's on a
/// Uruguayan system, Mexico's on a Mexican one, Spain's elsewhere.
#[test]
fn new_documents_follow_the_regions_spanish() {
    let mut s = s();
    s.system_region = wordcraft_geom::Region::from_tag("es_UY.UTF-8");
    assert_eq!(s.region_spanish(), "es-UY");
    // The ISO 639-2 code is Spanish too (CodeRabbit review on #3562).
    s.system_region = wordcraft_geom::Region::from_tag("spa_MX.UTF-8");
    assert_eq!(s.region_spanish(), "es-MX");
    s.system_region = wordcraft_geom::Region::from_tag("es_UY.UTF-8");
    s.set_editing_language(Some(&s.region_spanish()));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("es-UY"), "the untouched document follows");
    for (tag, spanish) in [("es-MX", "es-MX"), ("es-419", "es-419"), ("es-ES", "es-ES"), ("en-US", "es-ES"), ("fr-FR", "es-ES"), ("auto", "es-UY")] {
        run(&mut s, "file.options", json!({"region": tag}));
        assert_eq!(s.region_spanish(), spanish, "{tag}");
    }
}

// Hand-written synthetic fixtures (no system dictionary data): Spain's has `vosotros` forms,
// Uruguay's has `vos` ones' verbs only (the forms come from the voseo rules).
const SPANISH_TEST_AFF: &str =
    "SET UTF-8\nFLAG UTF-8\nTRY aeosrnidlctumpbgvyqhfzjñáéíóú\nMAP 2\nMAP eé\nMAP ií\nREP 2\nREP v b\nREP b v\nSFX S Y 1\nSFX S 0 s [aeiou]\n";
const SPANISH_TEST_DIC: &str = "10\nesto\nes\nuna\nprueba/S\ndel\ncorrector\ntambién\nhabéis\nvenido\ndecir\n";
const URUGUAYAN_TEST_DIC: &str = "10\nesto\nes\nuna\nprueba/S\ndel\ncorrector\ntambién\ntener\nvenir\ndecir\n";

fn spanish_test_dictionaries() {
    use wordcraft_proof::ProofLang;
    for (lang, dic) in [(ProofLang::Es, SPANISH_TEST_DIC), (ProofLang::EsUy, URUGUAYAN_TEST_DIC)] {
        wordcraft_proof::es::install_dictionary(lang, SPANISH_TEST_AFF, dic).unwrap();
    }
}

/// #3553: Spanish proofing through the same commands as Review, F7, Change and Add to
/// Dictionary: `prueva` and `corector` are flagged with `prueba` and `corrector` offered.
#[test]
fn spanish_proofing_through_the_review_commands() {
    spanish_test_dictionaries();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "esto es una prueva del corector. Habéis venido?"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "es_es", "default": true}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("es-ES"));
    let issues = run(&mut s, "review.issues", json!({}));
    let arr = issues.as_array().unwrap();
    assert!(arr.iter().all(|i| i["lang"] == "es"), "{issues}");
    let has = |text: &str, kind: &str, sugg: &str| {
        arr.iter().any(|i| i["text"] == text && i["kind"] == kind && i["suggestions"].as_array().unwrap().iter().any(|x| x == sugg))
    };
    assert!(has("prueva", "spelling", "prueba"), "{issues}");
    assert!(has("corector", "spelling", "corrector"), "{issues}");
    assert!(has("Habéis venido?", "grammar", "¿Habéis venido?"), "{issues}");
    assert_eq!(arr.len(), 3, "{issues}");
    assert!(arr.iter().any(|i| i["message"] == "Posible error ortográfico"), "{issues}");
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "review.spelling", json!({}))["text"], "prueva");
    run(&mut s, "review.applySuggestion", json!({"text": "prueba"}));
    assert!(text(&s).starts_with("esto es una prueba del"), "{}", text(&s));
    run(&mut s, "review.addToDictionary", json!({"word": "corector"}));
    let left = run(&mut s, "review.issues", json!({}));
    assert_eq!(left.as_array().unwrap().len(), 1, "{left}");
}

/// #3562 review: Add to Dictionary and Ignore All get the issue's own text, decomposed (NFD) as
/// typed; the word is then accepted decomposed and composed, and its issue range was the typed
/// bytes.
#[test]
fn add_and_ignore_all_accept_both_normalization_forms() {
    spanish_test_dictionaries();
    let mut s = s();
    let text = "esto zqanade\u{301}do y zqignora\u{301}do";
    run(&mut s, "document.setText", json!({"text": text}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "es-ES"}));
    let issues = run(&mut s, "review.issues", json!({}));
    let words: Vec<(String, u64)> = issues
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "spelling")
        .map(|i| (i["text"].as_str().unwrap().to_string(), i["start"]["off"].as_u64().unwrap()))
        .collect();
    assert_eq!(words, [("zqanade\u{301}do".to_string(), 5), ("zqignora\u{301}do".to_string(), text.find("zqign").unwrap() as u64)], "{issues}");
    run(&mut s, "review.addToDictionary", json!({"word": words[0].0}));
    run(&mut s, "review.ignoreAll", json!({"word": words[1].0}));
    assert!(run(&mut s, "review.issues", json!({})).as_array().unwrap().is_empty());
    // The composed spellings are the same words.
    run(&mut s, "document.setText", json!({"text": "esto zqanadédo y zqignorádo"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "es-ES"}));
    assert!(run(&mut s, "review.issues", json!({})).as_array().unwrap().is_empty());
    wordcraft_proof::remove_word("zqanadédo");
}

/// Uruguayan Spanish accepts the `vos` forms, imperatives included, that Spain's flags.
#[test]
fn uruguayan_spanish_accepts_vos_forms() {
    spanish_test_dictionaries();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Vení, decime qué tenés."}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "es-UY"}));
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "es-UY");
    let spelling = |s: &mut Session| -> Vec<String> {
        let issues = run(s, "review.issues", json!({}));
        issues.as_array().unwrap().iter().filter(|i| i["kind"] == "spelling").filter_map(|i| i["text"].as_str().map(String::from)).collect()
    };
    assert_eq!(spelling(&mut s), ["qué"], "only the word the fixture lacks");
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "es-ES"}));
    assert_eq!(spelling(&mut s), ["Vení", "decime", "qué", "tenés"]);
}

/// Czech proofing through the same commands as Review, F7, Change and Add to Dictionary.
#[test]
fn czech_proofing_through_the_review_commands() {
    czech_test_dict::install();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Město pocitac, škola škola."}));
    assert!(run(&mut s, "review.issues", json!({})).as_array().unwrap().len() >= 3);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "cs_cz", "default": true}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("cs-CZ"));
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "cs-CZ");
    let issues = run(&mut s, "review.issues", json!({}));
    let arr = issues.as_array().unwrap();
    assert_eq!(arr.len(), 2, "{issues}");
    assert!(arr.iter().all(|i| i["lang"] == "cs"), "{issues}");
    assert!(
        arr.iter().any(|i| i["text"] == "pocitac" && i["kind"] == "spelling" && i["suggestions"].as_array().unwrap().iter().any(|s| s == "počítač")),
        "{issues}"
    );
    assert!(arr.iter().any(|i| i["text"] == "škola škola" && i["kind"] == "grammar" && i["message"] == "Opakované slovo: „škola“"), "{issues}");
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "review.spelling", json!({}))["text"], "pocitac");
    run(&mut s, "review.applySuggestion", json!({"text": "počítač"}));
    assert!(text(&s).starts_with("Město počítač,"));
    // Exercise dictionary addition on a misspelling still present in the document.
    run(&mut s, "select.text", json!({"text": "počítač"}));
    run(&mut s, "text.insert", json!({"text": "zqcsenginevlastni"}));
    assert_eq!(run(&mut s, "review.issues", json!({})).as_array().unwrap().len(), 2);
    run(&mut s, "review.addToDictionary", json!({"word": "zqcsenginevlastni"}));
    let left = run(&mut s, "review.issues", json!({}));
    let left = left.as_array().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["kind"], "grammar");
    assert_eq!(left[0]["lang"], "cs");
}

/// Review › Language and the proofing commands work in Polish end to end: the language makes
/// the document's words Polish, F7 and the issue list use the Polish dictionary and grammar,
/// suggestions come from it, and a suggestion fixes the text.
const ITALIAN_TEST_AFF: &str = "SET UTF-8\nTRY aeiortnsclcaffèéè\nMAP 2\nMAP aà\nMAP eéè\nREP 1\nREP caffe caffè\n";
const ITALIAN_TEST_DIC: &str = "6\ncasa\ncaffè\nroma\ndisse\ne\nper\n";

fn italian_test_dictionary() {
    wordcraft_proof::it::install_dictionary(ITALIAN_TEST_AFF, ITALIAN_TEST_DIC).unwrap();
}

/// Review › Language and the proofing commands work in Italian end to end: the language makes
/// the document's words Italian, F7 and the issue list use the Italian dictionary and grammar,
/// suggestions come from it, and a suggestion fixes the text.
#[test]
fn italian_proofing_through_the_review_commands() {
    italian_test_dictionary();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Casa caffe, e roma roma disse."}));
    // English by default: the Italian words are English misspellings.
    assert!(run(&mut s, "review.issues", json!({})).as_array().unwrap().len() >= 3);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "it_IT", "default": true}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("it-IT"), "the tag is normalized");
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "it-IT");
    let issues = run(&mut s, "review.issues", json!({}));
    let arr = issues.as_array().unwrap();
    let has = |text: &str, kind: &str, sugg: &str| {
        arr.iter()
            .any(|i| i["text"] == text && i["kind"] == kind && i["lang"] == "it" && i["suggestions"].as_array().unwrap().iter().any(|x| x == sugg))
    };
    assert!(has("caffe", "spelling", "caffè"), "{issues}");
    assert!(has("roma roma", "grammar", "roma"), "{issues}");
    assert_eq!(arr.len(), 2, "{issues}");
    assert!(arr.iter().any(|i| i["message"] == "Parola ripetuta: \"roma\""), "{issues}");
    // F7 from the start selects the first issue; Change applies a suggestion.
    run(&mut s, "caret.docStart", json!({}));
    let first = run(&mut s, "review.spelling", json!({}));
    assert_eq!(first["text"], "caffe");
    run(&mut s, "review.applySuggestion", json!({"text": "caffè"}));
    assert!(text(&s).starts_with("Casa caffè,"), "{}", text(&s));
    // Add to Dictionary works for Italian words too.
    run(&mut s, "review.addToDictionary", json!({"word": "caffe"}));
    let left = run(&mut s, "review.issues", json!({}));
    assert_eq!(left.as_array().unwrap().len(), 1, "{left}");
}

#[test]
fn polish_proofing_through_the_review_commands() {
    polish_test_dictionary();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Płynie żeka, a wogóle to wiem że tak."}));
    // English by default: the Polish words are English misspellings.
    assert!(run(&mut s, "review.issues", json!({})).as_array().unwrap().len() >= 5);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "pl_PL", "default": true}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("pl-PL"), "the tag is normalized");
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "pl-PL");
    let issues = run(&mut s, "review.issues", json!({}));
    let arr = issues.as_array().unwrap();
    let has = |text: &str, kind: &str, sugg: &str| {
        arr.iter()
            .any(|i| i["text"] == text && i["kind"] == kind && i["lang"] == "pl" && i["suggestions"].as_array().unwrap().iter().any(|x| x == sugg))
    };
    assert!(has("żeka", "spelling", "rzeka"), "{issues}");
    assert!(has("wogóle", "spelling", "w ogóle"), "{issues}");
    assert!(has(" że", "grammar", ", że"), "{issues}");
    assert_eq!(arr.len(), 3, "{issues}");
    assert!(arr.iter().any(|i| i["message"] == "Brak przecinka przed „że”"), "{issues}");
    // F7 from the start selects the first issue; Change applies a suggestion.
    run(&mut s, "caret.docStart", json!({}));
    let first = run(&mut s, "review.spelling", json!({}));
    assert_eq!(first["text"], "żeka");
    run(&mut s, "review.applySuggestion", json!({"text": "rzeka"}));
    assert!(text(&s).starts_with("Płynie rzeka,"), "{}", text(&s));
    // Add to Dictionary works for Polish words too.
    run(&mut s, "review.addToDictionary", json!({"word": "wogóle"}));
    let left = run(&mut s, "review.issues", json!({}));
    assert_eq!(left.as_array().unwrap().len(), 1, "{left}");
}

/// Review › Language and proofing commands work in Turkish end to end.
#[test]
fn turkish_proofing_through_the_review_commands() {
    // Spelling and suggestions come from the installed Turkish Hunspell dictionary; machines
    // without one (CI, most desktops) have nothing to check here.
    if !crate::proof::tr::has_dictionary() {
        return;
    }
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Bana bazen gelenler oldu ise de. Sana olacaklardan sonra sorna"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "tr-TR", "default": true}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("tr-TR"));

    let issues = run(&mut s, "review.issues", json!({}));
    let arr = issues.as_array().unwrap();
    // "sorna" must be flagged as spelling issue
    let sorna_issue = arr.iter().find(|i| i["text"] == "sorna" && i["kind"] == "spelling");
    assert!(sorna_issue.is_some(), "sorna must be flagged: {issues}");

    // Check suggestions for sorna
    let sugs: Vec<String> = sorna_issue.unwrap()["suggestions"].as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
    assert!(sugs.contains(&"sonra".to_string()), "sorna must suggest sonra: {sugs:?}");

    // Add to Dictionary: adding "sorna" should remove its issue
    run(&mut s, "review.addToDictionary", json!({"word": "sorna"}));
    let issues_after = run(&mut s, "review.issues", json!({}));
    let arr_after = issues_after.as_array().unwrap();
    assert!(!arr_after.iter().any(|i| i["text"] == "sorna"), "sorna should no longer be flagged after adding to dictionary: {issues_after}");
}

/// `review.language`: tags are validated and normalized, `noProof` alone keeps the language,
/// no parameters keep the old meaning (US English), and `default` needs a language.
#[test]
fn review_language_validates_tags_and_marks_text_unchecked() {
    polish_test_dictionary();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Hello wrold"}));
    run(&mut s, "select.all", json!({}));
    assert!(s.run("review.language", &json!({"lang": "not a tag!"})).is_err());
    assert!(s.run("review.language", &json!({"lang": ""})).is_err());
    assert!(s.run("review.language", &json!({"default": true, "noProof": true})).is_err(), "default needs a language");
    assert_eq!(text(&s), "Hello wrold", "a refused command changes nothing");
    run(&mut s, "review.language", json!({"noProof": true}));
    let st = run(&mut s, "format.state", json!({}));
    assert_eq!((st["noProof"].clone(), st["lang"].clone()), (json!(true), json!("en-US")));
    assert_eq!(run(&mut s, "review.issues", json!({})), json!([]));
    run(&mut s, "review.language", json!({"noProof": false}));
    assert_eq!(run(&mut s, "review.issues", json!({})).as_array().unwrap().len(), 1);
    run(&mut s, "review.language", json!({"lang": "PL"}));
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "pl");
    run(&mut s, "review.language", json!({}));
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], "en-US");
    // A language without proofing tools: no marks.
    run(&mut s, "review.language", json!({"lang": "fr-FR"}));
    assert_eq!(run(&mut s, "review.issues", json!({})), json!([]));
}

/// The editing language (set by the front end from the interface language): new blank
/// documents and opened plain text are written in it; the untouched first document switches,
/// a document with changes never does, and templates stay English.
#[test]
fn new_documents_follow_the_editing_language() {
    polish_test_dictionary();
    let mut s = s();
    s.set_editing_language(Some("pl-PL"));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("pl-PL"), "the untouched blank document switches");
    assert!(!s.dirty && !s.can_undo(), "no change to save or undo");
    run(&mut s, "text.insert", json!({"text": "Zażółć gęślą jaźń."}));
    assert_eq!(run(&mut s, "review.issues", json!({})), json!([]));
    s.set_editing_language(None);
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("pl-PL"), "a document with changes keeps its language");
    s.set_editing_language(Some("pl"));
    run(&mut s, "file.new", json!({}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("pl"));
    run(&mut s, "file.new", json!({"template": "letter"}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("en-US"), "templates are English text");
    let data = crate::cmd::insert::base64_encode("Płynie rzeka.".as_bytes());
    run(&mut s, "file.open", json!({"path": "notatka.txt", "data": data}));
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("pl"));
    assert_eq!(run(&mut s, "review.issues", json!({})), json!([]));
    // Malformed tags fall back to US English.
    s.set_editing_language(Some("not a tag"));
    assert_eq!(s.editing_lang, None);
}

/// #2882: Set As Default survives preference serialization and overrides the interface language.
#[test]
fn default_proofing_language_survives_a_restart() {
    let mut first = s();
    first.set_editing_language(Some("en-US"));
    run(&mut first, "review.language", json!({"lang": "FR_fr", "default": true}));
    let saved = serde_json::to_vec(&first.prefs).unwrap();
    run(&mut first, "review.language", json!({"lang": "en-GB"}));
    run(&mut first, "edit.undo", json!({}));
    run(&mut first, "file.new", json!({}));
    assert_eq!(first.doc.styles.default_chr.lang.as_deref(), Some("fr-FR"));

    let mut second = s();
    second.prefs = serde_json::from_slice(&saved).unwrap();
    second.set_editing_language(Some("pl-PL"));
    assert_eq!(second.doc.styles.default_chr.lang.as_deref(), Some("fr-FR"));
    assert!(!second.dirty && !second.can_undo());
    run(&mut second, "file.new", json!({}));
    assert_eq!(second.doc.styles.default_chr.lang.as_deref(), Some("fr-FR"));
    let data = crate::cmd::insert::base64_encode(b"Bonjour");
    run(&mut second, "file.open", json!({"path": "note.txt", "data": data}));
    assert_eq!(second.doc.styles.default_chr.lang.as_deref(), Some("fr-FR"));

    let mut english = wordcraft_doc::Document::from_text("English document");
    english.styles.default_chr.lang = Some("en-GB".into());
    let bytes = crate::io::save_bytes("english.docx", &english).unwrap();
    let data = crate::cmd::insert::base64_encode(&bytes);
    run(&mut second, "file.open", json!({"path": "english.docx", "data": data}));
    second.set_editing_language(Some("ru-RU"));
    assert_eq!(second.doc.styles.default_chr.lang.as_deref(), Some("en-GB"));
    run(&mut second, "file.new", json!({"template": "letter"}));
    assert_eq!(second.doc.styles.default_chr.lang.as_deref(), Some("en-US"));
}

/// Older or malformed settings retain the interface fallback and unrelated preferences.
#[test]
fn default_proofing_language_ignores_damaged_preferences() {
    for value in [json!(null), json!("not a tag"), json!(17), json!([])] {
        let mut session = s();
        session.prefs = serde_json::from_value(json!({"defaultLanguage": value, "detectLanguage": false})).unwrap();
        session.set_editing_language(Some("pl-PL"));
        assert_eq!(session.doc.styles.default_chr.lang.as_deref(), Some("pl-PL"));
        assert!(!session.prefs.detect_language);
    }
    let mut old = s();
    old.prefs = serde_json::from_str("{}").unwrap();
    old.set_editing_language(Some("ru-RU"));
    assert_eq!(old.doc.styles.default_chr.lang.as_deref(), Some("ru-RU"));
}

/// #2882: a final protection rejection rolls back the saved default as well as the document.
#[test]
fn rejected_default_proofing_language_does_not_survive_a_restart() {
    for saved in [None, Some("pl-PL")] {
        let mut session = Session::new(wordcraft_doc::Document::from_text("Locked intro.\nEditable text.\nLocked end."));
        session.prefs.default_language = saved.map(str::to_string);
        session.set_editing_language(Some("en-US"));
        run(&mut session, "select.text", json!({"text": "Editable text."}));
        run(&mut session, "review.restrict", json!({"exception": "add"}));
        run(&mut session, "review.restrict", json!({"mode": "readOnly"}));
        run(&mut session, "select.text", json!({"text": "Editable"}));
        // Selection-only language formatting is allowed inside the exception.
        run(&mut session, "review.language", json!({"lang": "en-GB"}));
        let before = session.doc.clone();
        let prefs = serde_json::to_value(&session.prefs).unwrap();
        let undo = session.undo_labels();
        let error = session.run("review.language", &json!({"lang": "fr-FR", "default": true})).unwrap_err();
        assert!(error.to_string().contains("only the editable regions"), "{error}");
        assert_eq!(session.doc, before);
        assert_eq!(session.undo_labels(), undo);
        assert_eq!(serde_json::to_value(&session.prefs).unwrap(), prefs);
        let mut restarted = s();
        restarted.prefs = serde_json::from_value(prefs).unwrap();
        restarted.set_editing_language(Some("en-US"));
        run(&mut restarted, "file.new", json!({}));
        assert_eq!(restarted.doc.styles.default_chr.lang.as_deref(), Some(saved.unwrap_or("en-US")));
    }
}

/// A Polish document keeps its languages through .docx: the default (`w:docDefaults`) and an
/// English run (`w:lang`) come back, and so does the proofing.
#[test]
fn proofing_languages_survive_a_docx_round_trip() {
    polish_test_dictionary();
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Płynie żeka. Hello wrold."}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "pl-PL", "default": true}));
    run(&mut s, "select.text", json!({"text": "Hello wrold."}));
    run(&mut s, "review.language", json!({"lang": "en-GB"}));
    let bytes = crate::io::save_bytes("polski.docx", &s.doc).unwrap();
    let doc = crate::io::open_bytes("polski.docx", &bytes).unwrap();
    assert_eq!(doc.styles.default_chr.lang.as_deref(), Some("pl-PL"));
    let mut back = Session::new(doc);
    let issues = run(&mut back, "review.issues", json!({}));
    let found: Vec<(String, String)> =
        issues.as_array().unwrap().iter().map(|i| (i["text"].as_str().unwrap().to_string(), i["lang"].as_str().unwrap().to_string())).collect();
    assert_eq!(found, [("żeka".to_string(), "pl".to_string()), ("wrold".to_string(), "en".to_string())], "{issues}");
}

#[test]
fn hide_white_space_toggles_view_state_only() {
    let mut s = s();
    let before = s.doc.body.len();
    assert!(!s.view.hide_white_space);
    assert_eq!(run(&mut s, "view.hideWhiteSpace", json!({}))["value"], true);
    assert!(s.view.hide_white_space);
    assert_eq!(run(&mut s, "view.hideWhiteSpace", json!({"value": false}))["value"], false);
    assert_eq!(s.doc.body.len(), before, "a view option never edits the document");
}

/// The first SmartArt (or other graphic) in the body, selected.
fn select_graphic(s: &mut Session) {
    let (path, off) = (0..s.doc.body.len() as u32)
        .find_map(|b| {
            let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path(vec![b]))?;
            let off = p.object_offsets().into_iter().find(|o| matches!(p.object_at(*o), Some(wordcraft_doc::InlineObject::Graphic { .. })))?;
            Some((wordcraft_doc::Path(vec![b]), off))
        })
        .expect("a graphic");
    let at = Pos { story: StoryRef::Body, path, off };
    let end = Pos { off: off + wordcraft_doc::para::OBJ.len_utf8(), ..at.clone() };
    s.sel = crate::Selection { anchor: at, focus: end };
}

/// #497: Insert › SmartArt saves as diagram parts (data, layout, style, colours, drawing) and
/// reopens as the same editable graphic; an edited data part stays read-only.
#[test]
fn smart_art_round_trips_through_docx() {
    use std::io::Read;
    let mut s = s();
    let r = run(
        &mut s,
        "insert.smartArt",
        json!({"layout": "hierarchy", "items": ["Root", "\tLeft", {"text": "Right", "level": 1}], "colors": "colorful"}),
    );
    assert_eq!(r["smartArt"]["layout"], "hierarchy");
    let spec = cmd::smart_art::selected_smart_art(&s).expect("selected after insert");
    let bytes = crate::io::save_bytes("sa.docx", &s.doc).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    for part in ["data1", "layout1", "quickStyle1", "colors1", "drawing1"] {
        assert!(zip.by_name(&format!("word/diagrams/{part}.xml")).is_ok(), "{part} written");
    }
    let mut ct = String::new();
    zip.by_name("[Content_Types].xml").unwrap().read_to_string(&mut ct).unwrap();
    assert!(ct.contains("diagramDrawing+xml") && ct.contains("diagramData+xml"), "{ct}");
    let mut s2 = Session::new(crate::io::open_bytes("sa.docx", &bytes).unwrap());
    select_graphic(&mut s2);
    assert_eq!(cmd::smart_art::selected_smart_art(&s2).as_deref(), Some(&*spec), "reopens editable");
    run(&mut s2, "smartArt.layout", json!({"layout": "venn"}));
    // Saved again, and once more after reopening: still the same graphic.
    let again = crate::io::save_bytes("sa.docx", &s2.doc).unwrap();
    let mut s3 = Session::new(crate::io::open_bytes("sa.docx", &again).unwrap());
    select_graphic(&mut s3);
    assert_eq!(cmd::smart_art::selected_smart_art(&s3).map(|g| g.layout), Some(wordcraft_doc::smart_art::SmartArtLayout::Venn));
    // A data part another program rewrote: shown, not editable.
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&again)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        if name.ends_with("data1.xml") {
            buf = String::from_utf8(buf).unwrap().replace("<dgm:bg/>", "<dgm:bg><a:noFill/></dgm:bg>").into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut out, &buf).unwrap();
    }
    let foreign = out.finish().unwrap().into_inner();
    let mut s4 = Session::new(crate::io::open_bytes("sa.docx", &foreign).unwrap());
    select_graphic(&mut s4);
    assert!(cmd::smart_art::selected_smart_art(&s4).is_none());
    assert!(s4.run("smartArt.layout", &json!({"layout": "cycle"})).is_err());
}

/// #497: item, layout and colour edits are one undo step each; promote and demote move an item
/// with the items under it.
#[test]
fn smart_art_edits_undo_one_step_each() {
    let mut s = s();
    run(&mut s, "insert.smartArt", json!({"layout": "process"}));
    let items = |s: &Session| cmd::smart_art::selected_smart_art(s).unwrap().items.iter().map(|i| (i.text.clone(), i.level)).collect::<Vec<_>>();
    let before = items(&s);
    run(&mut s, "smartArt.items", json!({"items": ["One", "Two", "\tTwo a"]}));
    assert_eq!(items(&s), [("One".to_string(), 0), ("Two".to_string(), 0), ("Two a".to_string(), 1)]);
    run(&mut s, "smartArt.demote", json!({"index": 1}));
    assert_eq!(items(&s)[1..], [("Two".to_string(), 1), ("Two a".to_string(), 2)]);
    run(&mut s, "smartArt.promote", json!({"index": 1}));
    run(&mut s, "smartArt.addShape", json!({"after": 0, "text": "New"}));
    assert_eq!(items(&s)[1], ("New".to_string(), 0));
    run(&mut s, "smartArt.colors", json!({"variant": "accent4"}));
    run(&mut s, "smartArt.layout", json!({"layout": "pyramid"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(cmd::smart_art::selected_smart_art(&s).unwrap().layout, wordcraft_doc::smart_art::SmartArtLayout::Process);
    for _ in 0..5 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(items(&s), before);
    assert!(s.run("smartArt.promote", &json!({"index": 0})).is_err(), "already at the top");
    run(&mut s, "picture.size", json!({"width": 200, "height": 120, "lockAspect": false}));
}

/// #497: hostile SmartArt params are capped or refused, never a panic.
#[test]
fn smart_art_hostile_params_are_capped() {
    use wordcraft_doc::smart_art::{MAX_ITEMS, MAX_LEVEL, MAX_TEXT};
    let mut s = s();
    let many: Vec<serde_json::Value> = (0..MAX_ITEMS * 3).map(|i| json!({"text": "x".repeat(10_000), "level": i * 1000})).collect();
    run(&mut s, "insert.smartArt", json!({"layout": "hierarchy", "items": many, "width": 1e30, "height": -5}));
    let g = cmd::smart_art::selected_smart_art(&s).unwrap();
    assert_eq!(g.items.len(), MAX_ITEMS);
    assert!(g.items.iter().all(|i| i.text.chars().count() <= MAX_TEXT && i.level <= MAX_LEVEL));
    for bad in [json!({"items": []}), json!({"items": "x"}), json!({"items": [5]}), json!({"layout": "spiral"}), json!({"layout": 3})] {
        assert!(s.run("insert.smartArt", &bad).is_err(), "{bad}");
    }
    let junk = [json!(null), json!({}), json!({"index": -1, "after": 1e300, "items": [{"level": "deep"}], "variant": [], "layout": {}})];
    for id in [
        "smartArt.items",
        "smartArt.addShape",
        "smartArt.promote",
        "smartArt.demote",
        "smartArt.layout",
        "smartArt.colors",
        "smartArt.reset",
        "smartArt.textPane",
    ] {
        for j in &junk {
            let _ = s.run(id, j);
        }
    }
    for _ in 0..MAX_ITEMS {
        let _ = s.run("smartArt.addShape", &json!({}));
    }
    assert_eq!(cmd::smart_art::selected_smart_art(&s).unwrap().items.len(), MAX_ITEMS);
    let _ = s.layout();
}

/// Outline view of: Heading 1 "One", body "a", Heading 2 "Two", body "b", Heading 1 "Three", body "c".
fn outline_doc() -> Session {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "One\na\nTwo\nb\nThree\nc"}));
    for (i, style) in [(0, "Heading1"), (2, "Heading2"), (4, "Heading1")] {
        s.sel = crate::Selection::caret(Pos::body(i, 0));
        run(&mut s, "para.style", json!({"style": style}));
    }
    run(&mut s, "view.outline", json!({}));
    s
}

fn outline_shown(s: &mut Session) -> Vec<u64> {
    let st = run(s, "outline.state", json!({}));
    st["shown"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect()
}

/// The body blocks the outline layout draws.
fn laid_out_blocks(s: &mut Session) -> Vec<u32> {
    let l = s.layout();
    let mut v: Vec<u32> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Lines { story: StoryRef::Body, path, .. } => path.0.first().copied(),
            _ => None,
        })
        .collect();
    v.dedup();
    v
}

#[test]
fn outline_level_changes_are_undoable() {
    let mut s = outline_doc();
    let level = |s: &mut Session, i: usize| run(s, "outline.state", json!({}))["levels"][i].as_u64().unwrap();
    s.sel = crate::Selection::caret(Pos::body(1, 0));
    run(&mut s, "outline.level", json!({"level": 3}));
    assert_eq!(level(&mut s, 1), 3);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(level(&mut s, 1), 10, "body text again");
    s.sel = crate::Selection::caret(Pos::body(2, 0));
    run(&mut s, "outline.promoteToHeading1", json!({}));
    assert_eq!(level(&mut s, 2), 1);
    run(&mut s, "outline.demoteToBody", json!({}));
    assert_eq!(level(&mut s, 2), 10);
    assert!(s.run("outline.level", &json!({"level": 12})).is_err());
    assert!(s.run("outline.level", &json!({})).is_err());
    // Every outline command survives junk parameters in Outline view.
    for spec in cmd::registry().all().iter().filter(|c| c.id.starts_with("outline.")) {
        for j in [json!(null), json!({"block": 99999, "level": "x", "value": 5}), json!({"block": -1, "level": 0})] {
            let mut s = outline_doc();
            let _ = s.run(spec.id, &j);
            let _ = s.layout();
        }
    }
}

#[test]
fn outline_show_level_filters_paragraphs() {
    let mut s = outline_doc();
    assert_eq!(outline_shown(&mut s), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(laid_out_blocks(&mut s), vec![0, 1, 2, 3, 4, 5]);
    run(&mut s, "outline.showLevel", json!({"level": 1}));
    assert_eq!(outline_shown(&mut s), vec![0, 4]);
    assert_eq!(laid_out_blocks(&mut s), vec![0, 4]);
    run(&mut s, "outline.showLevel2", json!({}));
    assert_eq!(outline_shown(&mut s), vec![0, 2, 4]);
    run(&mut s, "outline.showAll", json!({}));
    assert_eq!(outline_shown(&mut s), vec![0, 1, 2, 3, 4, 5]);
    // Word's keys, and only in Outline view.
    let reg = cmd::registry();
    assert_eq!(reg.by_shortcut("Alt+Shift+1").map(|c| c.id), Some("outline.showLevel1"));
    assert_eq!(reg.by_shortcut("Alt+Shift+A").map(|c| c.id), Some("outline.showAll"));
    assert_eq!(reg.by_shortcut("Alt+Shift+=").map(|c| c.id), Some("outline.expand"));
    assert_eq!(reg.by_shortcut("Alt+Shift+-").map(|c| c.id), Some("outline.collapse"));
    run(&mut s, "outline.close", json!({}));
    assert!(s.run("outline.showLevel1", &json!({})).is_err());
}

#[test]
fn outline_collapse_hides_descendants_and_expand_restores() {
    let mut s = outline_doc();
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    // The deepest open level closes first, then the heading itself.
    run(&mut s, "outline.collapse", json!({}));
    assert_eq!(outline_shown(&mut s), vec![0, 1, 2, 4, 5]);
    run(&mut s, "outline.collapse", json!({}));
    assert_eq!(outline_shown(&mut s), vec![0, 4, 5]);
    assert_eq!(laid_out_blocks(&mut s), vec![0, 4, 5]);
    // A caret moving into the collapsed text goes on to the next shown paragraph.
    run(&mut s, "caret.end", json!({}));
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus, Pos::body(4, 0));
    // Moving "Three" and "c" up passes "One" with everything collapsed under it, in one undo step.
    s.sel = crate::Selection { anchor: Pos::body(4, 0), focus: Pos::body(5, 1) };
    run(&mut s, "outline.moveUp", json!({}));
    assert_eq!(text(&s), "Three\nc\nOne\na\nTwo\nb");
    assert_eq!(outline_shown(&mut s), vec![0, 1, 2], "the collapsed heading moved and stays collapsed");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "One\na\nTwo\nb\nThree\nc");
    assert_eq!(outline_shown(&mut s), vec![0, 4, 5]);
    // Expand opens one level at a time; double-clicking the symbol (toggle) opens it all.
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "outline.expand", json!({}));
    assert_eq!(outline_shown(&mut s), vec![0, 1, 2, 4, 5]);
    run(&mut s, "outline.expand", json!({}));
    assert_eq!(outline_shown(&mut s), vec![0, 1, 2, 3, 4, 5]);
    run(&mut s, "outline.toggle", json!({"block": 0}));
    assert_eq!(outline_shown(&mut s), vec![0, 4, 5]);
    run(&mut s, "outline.toggle", json!({"block": 0}));
    assert_eq!(outline_shown(&mut s), vec![0, 1, 2, 3, 4, 5]);
    // Clicking a symbol selects the heading with what is under it.
    run(&mut s, "outline.selectSubtree", json!({"block": 0}));
    assert_eq!(s.selected_text(), "One\na\nTwo\nb");
}

/// #3621: with thousands of collapsed headings, an Enter above them (which shifts every index)
/// finds them again without scanning the document once per heading, and a caret move doesn't
/// redo the matching.
#[test]
fn outline_many_collapsed_headings_stay_fast_after_edits() {
    const N: usize = 8000;
    let text: Vec<String> = (0..N).flat_map(|i| [format!("Heading {i}"), format!("body {i}")]).collect();
    let mut d = wordcraft_doc::Document::from_text(&text.join("\n"));
    for i in 0..N {
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(2 * i)).unwrap();
        p.props.style = Some("Heading1".into());
    }
    let mut s = Session::new(d);
    run(&mut s, "view.outline", json!({}));
    run(&mut s, "outline.showLevel1", json!({}));
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "outline.collapse", json!({}));
    let collapsed_texts =
        |s: &Session| -> Vec<String> { s.view.outline.collapsed.iter().map(|(i, _)| s.doc.body[*i].as_para().unwrap().text.clone()).collect() };
    let before = collapsed_texts(&s);
    assert_eq!(before.len(), N, "every heading is collapsed");
    s.sel = crate::Selection::caret(Pos::body(1, 0));
    for _ in 0..10 {
        run(&mut s, "text.newParagraph", json!({}));
    }
    for _ in 0..20 {
        run(&mut s, "caret.right", json!({}));
    }
    assert_eq!(collapsed_texts(&s), before, "the same headings stay collapsed");
}

/// #745: layout keeps a picture's size, so pictures read from HTML, Markdown, ODT or RTF at
/// their pixel size are fitted to the text width when the file is opened (and when pasted),
/// keeping their proportions; small pictures keep their size.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn imported_pictures_fit_the_text_width() {
    let png = |w: u32, h: u32| {
        let mut b = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png)
            .unwrap();
        cmd::insert::base64_encode(&b)
    };
    let sizes = |doc: &wordcraft_doc::Document| -> Vec<(f32, f32)> {
        doc.body
            .iter()
            .filter_map(|b| b.as_para())
            .flat_map(|p| p.objects.iter())
            .filter_map(|o| match o {
                wordcraft_doc::InlineObject::Image { w, h, .. } => Some((*w, *h)),
                _ => None,
            })
            .collect()
    };
    let md = format!("![wide](data:image/png;base64,{})\n\n![small](data:image/png;base64,{})\n", png(1200, 300), png(40, 20));
    let doc = crate::io::open_bytes("p.md", md.as_bytes()).unwrap();
    let max_w = crate::io::text_width(&doc);
    let got = sizes(&doc);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!((got[0].0 - max_w).abs() < 0.01, "wide picture fits: {got:?} (text width {max_w})");
    assert!((got[0].1 - max_w / 4.0).abs() < 0.01, "and keeps its proportions: {got:?}");
    assert!((got[1].0 - 30.0).abs() < 0.01 && (got[1].1 - 15.0).abs() < 0.01, "small picture keeps its size: {got:?}");

    // Pasting HTML fits our text width too.
    let mut s = Session::new(wordcraft_doc::Document::new());
    let html = format!("<p><img src=\"data:image/png;base64,{}\"></p>", png(1200, 300));
    s.run("edit.pasteSpecial", &json!({"as": "html", "html": html})).unwrap();
    let max_w = crate::io::text_width(&s.doc);
    let got = sizes(&s.doc);
    assert!(got.iter().any(|(w, _)| (w - max_w).abs() < 0.01), "pasted picture fits: {got:?}");
}

#[test]
fn format_font_sets_the_asian_and_latin_fonts_separately() {
    // #505: Word's Font dialog sets an Asian text font and a Latin text font of their own.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Hello 你好"}));
    run(&mut s, "select.all", json!({}));
    let fonts = |s: &Session| {
        let c = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props_of_char(0).clone();
        (c.font, c.font_ea)
    };
    let before = fonts(&s);
    run(&mut s, "format.font", json!({"name": "Georgia", "eastAsia": "SimSun"}));
    assert_eq!(fonts(&s), (Some("Georgia".into()), Some("SimSun".into())));
    assert_eq!(run(&mut s, "format.state", json!({}))["fontEastAsia"], "SimSun");
    // The Asian font alone leaves the Latin font as it is.
    run(&mut s, "format.font", json!({"eastAsia": "Noto Sans CJK SC"}));
    assert_eq!(fonts(&s), (Some("Georgia".into()), Some("Noto Sans CJK SC".into())));
    // Each change is one undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fonts(&s), (Some("Georgia".into()), Some("SimSun".into())));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fonts(&s), before);
    assert!(s.run("format.font", &json!({})).is_err() && s.run("format.font", &json!({"eastAsia": " "})).is_err());
}

/// The result of the formula field in cell `c` of row `r` of the first table, if there is one.
fn formula_result(s: &Session, r: usize, c: usize) -> Option<String> {
    let t = s.doc.body.iter().find_map(|b| b.as_table())?;
    t.rows.get(r)?.cells.get(c)?.blocks.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
        wordcraft_doc::InlineObject::Field { instr, result, .. } if instr.starts_with('=') => Some(result.clone()),
        _ => None,
    })
}

/// A 4 × 4 matrix (values 1 to 9 under the header) with an empty fifth row; the caret in its
/// second cell.
fn formula_table() -> Session {
    let mut s = s();
    run(&mut s, "table.quick", json!({"kind": "matrix"}));
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, 3, 1, 0], "off": 0}}));
    run(&mut s, "table.insertRowBelow", json!({}));
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, 4, 1, 0], "off": 0}}));
    s
}

#[test]
fn table_formula_reads_cell_references_and_functions() {
    let mut s = formula_table();
    let r = run(&mut s, "table.formula", json!({"formula": "=B2+B3*2"}));
    assert_eq!(r["result"], "9");
    assert_eq!(formula_result(&s, 4, 1).as_deref(), Some("9"));
    let mut s = formula_table();
    assert_eq!(run(&mut s, "table.formula", json!({"formula": "=IF(SUM(ABOVE)>10, 1, 0)"}))["result"], "1");
    let mut s = formula_table();
    assert_eq!(run(&mut s, "table.formula", json!({"formula": "=SUM(B2:C3) \\# \"0.00\""}))["result"], "12.00");
}

#[test]
fn table_formula_with_an_unknown_function_is_an_error_and_inserts_nothing() {
    let mut s = formula_table();
    let err = s.run("table.formula", &json!({"formula": "=MEDIAN(ABOVE)"})).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("MEDIAN"), "{err}");
    assert_eq!(formula_result(&s, 4, 1), None);
    assert!(s.run("table.formula", &json!({"formula": "=SUM()"})).is_err(), "empty parentheses are a syntax error, as in Word");
}

#[test]
fn table_formula_bare_function_name_takes_its_direction_from_the_cell() {
    let mut s = formula_table();
    let r = run(&mut s, "table.formula", json!({"formula": "=SUM"}));
    assert_eq!((r["formula"].as_str(), r["result"].as_str()), (Some("=SUM(ABOVE)"), Some("12")));
}

/// Update Field (F9) works a formula out again from the cells it reads.
#[test]
fn update_fields_recalculates_a_table_formula() {
    let mut s = formula_table();
    run(&mut s, "table.formula", json!({"formula": "=SUM(ABOVE)"}));
    assert_eq!(formula_result(&s, 4, 1).as_deref(), Some("12"));
    // Row 1's 1 becomes 10.
    run(
        &mut s,
        "select.range",
        json!({"anchor": {"story": "body", "path": [1, 1, 1, 0], "off": 0}, "focus": {"story": "body", "path": [1, 1, 1, 0], "off": 1}}),
    );
    run(&mut s, "text.insert", json!({"text": "10"}));
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(formula_result(&s, 4, 1).as_deref(), Some("21"));
}

/// A formula's `\*` switches format its result when the formula is inserted and when Update Field
/// works it out again.
#[test]
fn table_formula_results_take_the_format_switches() {
    for (formula, inserted, updated) in [
        (r"=SUM(ABOVE) \* CardText", "twelve", "twenty-one"),
        (r"=SUM(ABOVE) \* CardText \* Caps", "Twelve", "Twenty-one"),
        (r"=SUM(ABOVE) \* OrdText \* Upper", "TWELFTH", "TWENTY-FIRST"),
        (r"=SUM(ABOVE) \* MERGEFORMAT", "12", "21"),
    ] {
        let mut s = formula_table();
        let r = run(&mut s, "table.formula", json!({"formula": formula}));
        assert_eq!((r["formula"].as_str(), r["result"].as_str()), (Some(formula), Some(inserted)));
        run(
            &mut s,
            "select.range",
            json!({"anchor": {"story": "body", "path": [1, 1, 1, 0], "off": 0}, "focus": {"story": "body", "path": [1, 1, 1, 0], "off": 1}}),
        );
        run(&mut s, "text.insert", json!({"text": "10"}));
        run(&mut s, "references.updateFields", json!({}));
        assert_eq!(formula_result(&s, 4, 1).as_deref(), Some(updated), "{formula}");
    }
}

#[test]
fn table_formula_results_take_dollar_text_and_hex() {
    for (formula, inserted, updated) in [
        (r"=SUM(ABOVE) \* DollarText", "twelve and 00/100", "twenty-one and 00/100"),
        (r"=SUM(ABOVE) \* DollarText \* Upper", "TWELVE AND 00/100", "TWENTY-ONE AND 00/100"),
        (r"=SUM(ABOVE) \* Hex", "C", "15"),
    ] {
        let mut s = formula_table();
        let r = run(&mut s, "table.formula", json!({"formula": formula}));
        assert_eq!((r["formula"].as_str(), r["result"].as_str()), (Some(formula), Some(inserted)));
        run(
            &mut s,
            "select.range",
            json!({"anchor": {"story": "body", "path": [1, 1, 1, 0], "off": 0}, "focus": {"story": "body", "path": [1, 1, 1, 0], "off": 1}}),
        );
        run(&mut s, "text.insert", json!({"text": "10"}));
        run(&mut s, "references.updateFields", json!({}));
        assert_eq!(formula_result(&s, 4, 1).as_deref(), Some(updated), "{formula}");
    }
}

/// A one-column table (no header row) holding `cells`, in a document whose default language
/// is `lang`; the caret ends in the last cell.
fn column_table(lang: &str, cells: &[&str]) -> Session {
    let mut s = s();
    run(&mut s, "review.language", json!({"lang": lang, "default": true}));
    run(&mut s, "insert.table", json!({"rows": cells.len(), "cols": 1}));
    for (k, c) in cells.iter().enumerate() {
        if k > 0 {
            run(&mut s, "text.tab", json!({}));
        }
        if !c.is_empty() {
            run(&mut s, "text.insert", json!({"text": c}));
        }
    }
    s
}

fn column_texts(s: &Session) -> Vec<String> {
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    t.rows.iter().map(|r| r.cells[0].blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<String>()).collect()
}

/// #836: table sort reads numbers in the text's language: `1,5` is one and a half in Polish.
#[test]
fn table_sort_reads_decimal_comma_in_polish() {
    let mut s = column_table("pl-PL", &["12", "1,5", "2"]);
    run(&mut s, "table.sort", json!({"column": 0, "header": false}));
    assert_eq!(column_texts(&s), ["1,5", "2", "12"]);
    run(&mut s, "table.sort", json!({"column": 0, "header": false, "descending": true}));
    assert_eq!(column_texts(&s), ["12", "2", "1,5"]);
    // English keeps the comma as a thousands separator.
    let mut s = column_table("en-US", &["1,250", "12.5", "2"]);
    run(&mut s, "table.sort", json!({"column": 0, "header": false}));
    assert_eq!(column_texts(&s), ["2", "12.5", "1,250"]);
}

/// #836: table formulas read each cell in its language and write the result with its decimal
/// separator.
#[test]
fn table_formula_reads_and_writes_decimal_comma_in_polish() {
    let mut s = column_table("pl-PL", &["3,50", "2,25", ""]);
    assert_eq!(run(&mut s, "table.formula", json!({"formula": "=SUM(ABOVE)"}))["result"], "5,75");
    let mut s = column_table("pl-PL", &["1 250,00", "100", ""]);
    assert_eq!(run(&mut s, "table.formula", json!({"formula": "=SUM(ABOVE)"}))["result"], "1350");
    let mut s = column_table("pl-PL", &["1\u{a0}250,50 zł", "100", ""]);
    assert_eq!(run(&mut s, "table.formula", json!({"formula": "=SUM(ABOVE)"}))["result"], "1350,50");
    let mut s = column_table("en-US", &["$1,250.50", "3.25", ""]);
    assert_eq!(run(&mut s, "table.formula", json!({"formula": "=SUM(ABOVE)"}))["result"], "1253.75");
}

/// #837: text sorts follow the language's alphabet: Polish `ł` after `l`, `ć` after `c`.
#[test]
fn table_and_paragraph_sort_use_polish_alphabet() {
    let mut s = column_table("pl-PL", &["Zakopane", "Łódź", "Kraków"]);
    run(&mut s, "table.sort", json!({"column": 0, "header": false}));
    assert_eq!(column_texts(&s), ["Kraków", "Łódź", "Zakopane"]);
    let mut s = Session::new(wordcraft_doc::Document::from_text("zebra\nćma\ndom"));
    run(&mut s, "review.language", json!({"lang": "pl-PL", "default": true}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "para.sort", json!({}));
    assert_eq!(text(&s), "ćma\ndom\nzebra");
    // Swedish puts å ä ö after z; German sorts ä with a.
    let mut s = column_table("sv-SE", &["ödla", "zebra", "åka", "apa"]);
    run(&mut s, "table.sort", json!({"column": 0, "header": false}));
    assert_eq!(column_texts(&s), ["apa", "zebra", "åka", "ödla"]);
    let mut s = column_table("de-DE", &["Zug", "Äpfel", "Bach"]);
    run(&mut s, "table.sort", json!({"column": 0, "header": false}));
    assert_eq!(column_texts(&s), ["Äpfel", "Bach", "Zug"]);
}

/// #837: index entries and letter headings follow the language: `Łódź` files under its own
/// `Ł` heading between `K` and `Z` in Polish.
#[test]
fn index_sorts_polish_entries_and_headings() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Zakopane.\nŁódź.\nKraków."));
    run(&mut s, "review.language", json!({"lang": "pl-PL", "default": true}));
    for w in ["Zakopane", "Łódź", "Kraków"] {
        run(&mut s, "select.text", json!({"text": w}));
        run(&mut s, "references.markEntry", json!({}));
    }
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "references.index", json!({}));
    let lines: Vec<String> =
        s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some("ListEntry")).map(|p| p.plain_text()).collect();
    let order: Vec<&str> = lines.iter().map(|l| l.split([',', '\t']).next().unwrap_or("")).collect();
    let pos = |w: &str| order.iter().position(|x| *x == w).unwrap_or(usize::MAX);
    assert!(pos("Kraków") < pos("Łódź") && pos("Łódź") < pos("Zakopane"), "{lines:?}");
}

/// Review of #844: a numbered column (`1.`, `2.`, `10.`) sorts as numbers; text with no language
/// at all follows the editing language (a Polish interface reads `3,50` as 3.5).
#[test]
fn ordinals_and_untagged_text_sort_and_sum_as_numbers() {
    let mut t = column_table("pl-PL", &["10.", "2.", "1."]);
    run(&mut t, "table.sort", json!({"column": 0, "header": false}));
    assert_eq!(column_texts(&t), ["1.", "2.", "10."]);

    let mut t = column_table("en-US", &["10", "(50)", "20"]);
    run(&mut t, "table.sort", json!({"column": 0, "header": false}));
    assert_eq!(column_texts(&t), ["(50)", "10", "20"]);

    let mut s = s();
    s.set_editing_language(Some("pl-PL"));
    s.doc.styles.default_chr.lang = None;
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 1}));
    run(&mut s, "text.insert", json!({"text": "3,50"}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.insert", json!({"text": "2,25"}));
    run(&mut s, "text.tab", json!({}));
    assert_eq!(run(&mut s, "table.formula", json!({"formula": "=SUM(ABOVE)"}))["result"], "5,75");
}

/// Review of #844: the bibliography lists Polish authors in the Polish alphabet.
#[test]
fn bibliography_sorts_polish_authors() {
    let mut s = s();
    run(&mut s, "review.language", json!({"lang": "pl-PL", "default": true}));
    for author in ["Mazur, Jan", "Łukasiewicz, Jan", "Lis, Anna"] {
        run(&mut s, "references.citation", json!({"source": {"author": author, "title": format!("Książka {author}"), "year": "2020"}}));
        run(&mut s, "text.insert", json!({"text": " "}));
    }
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "references.bibliography", json!({}));
    let all = text(&s);
    // The list, not the citations before it.
    let t = all.split_once("References").map_or(all.as_str(), |(_, list)| list);
    let at = |name: &str| t.find(&format!("{name}, ")).unwrap_or_else(|| panic!("{name} in {all}"));
    assert!(at("Lis") < at("Łukasiewicz") && at("Łukasiewicz") < at("Mazur"), "{t}");
}

/// A new blank document takes its paper from the region, and lengths are shown in the region's
/// unit with its decimal separator; an explicit Measurement units choice still wins (#838).
#[test]
fn new_documents_follow_the_region() {
    // Poland: A4, centimetres, decimal comma.
    let mut s = s();
    run(&mut s, "file.options", json!({"region": "pl-PL"}));
    run(&mut s, "file.new", json!({}));
    let sp = &s.doc.last_section;
    assert_eq!(wordcraft_geom::paper_name(sp.page_w, sp.page_h), Some("A4"));
    assert_eq!(sp.margin_left, 72.0);
    assert_eq!(s.units(), wordcraft_geom::Unit::Centimeters);
    assert_eq!(s.format_length(sp.margin_left), "2,54 cm");
    let o = run(&mut s, "file.options", json!({"pane": "advanced", "units": "auto"}));
    assert_eq!((o["units"].as_str(), o["unitsFromRegion"].as_bool()), (Some("centimeters"), Some(true)));
    assert_eq!((o["region"]["paper"].as_str(), o["region"]["decimal"].as_str()), (Some("A4"), Some(",")));

    // The user's own choice wins over the region, and `auto` gives the region's back.
    run(&mut s, "file.options", json!({"units": "inches"}));
    assert_eq!(s.units(), wordcraft_geom::Unit::Inches);
    assert_eq!(s.format_length(108.0), "1,5\"");
    run(&mut s, "file.options", json!({"units": "auto"}));
    assert_eq!(s.units(), wordcraft_geom::Unit::Centimeters);

    // A chosen region is a saved preference over the system's; `auto` returns to the system's.
    assert_eq!(s.prefs.region.as_deref(), Some("pl-PL"));
    let saved: serde_json::Value = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["region"], "pl-PL");
    s.system_region = wordcraft_geom::Region::from_tag("ja-JP");
    assert_eq!(s.region().territory, "PL", "the choice wins over the system");
    let o = run(&mut s, "file.options", json!({"region": "auto"}));
    assert_eq!((o["regionFromSystem"].as_bool(), o["region"]["territory"].as_str()), (Some(true), Some("JP")));
    assert_eq!(s.prefs.region, None);
    run(&mut s, "file.options", json!({"region": "de_AT.UTF-8@euro"}));
    assert_eq!(s.prefs.region.as_deref(), Some("de-AT"), "kept as a short tag");
    run(&mut s, "file.options", json!({"region": ""}));
    assert_eq!(s.prefs.region, None);

    // The United States: Letter and inches.
    let mut us = Session::new(wordcraft_doc::Document::new());
    run(&mut us, "file.options", json!({"region": "en-US"}));
    run(&mut us, "file.new", json!({"template": "blank"}));
    let sp = &us.doc.last_section;
    assert_eq!((sp.page_w, sp.page_h), (612.0, 792.0));
    assert_eq!(us.units(), wordcraft_geom::Unit::Inches);
    assert_eq!(us.format_length(72.0), "1\"");
    run(&mut us, "file.options", json!({"units": "centimeters"}));
    assert_eq!(us.format_length(72.0), "2.54 cm");

    // Templates keep their own size; a headless session is en-US; junk is refused or read as en-US.
    let mut t = Session::new(wordcraft_doc::Document::new());
    run(&mut t, "file.options", json!({"region": "de-DE"}));
    run(&mut t, "file.new", json!({"template": "letter"}));
    assert_eq!(t.doc.last_section.page_w, crate::sample::letter().last_section.page_w);
    assert_eq!(Session::new(wordcraft_doc::Document::new()).blank_document().last_section.page_w, 612.0);
    assert!(t.run("file.options", &json!({"region": 7})).is_err());
    assert!(t.run("file.options", &json!({"units": "furlongs"})).is_err());
    run(&mut t, "file.options", json!({"region": "\u{0}\u{FFFF}-zz-ZZZZZZZZ"}));
    assert_eq!(t.region(), wordcraft_geom::Region::default());
    assert_eq!(t.prefs.region.as_deref(), Some("en-US"), "junk is saved as en-US");
}

/// Automatic (HTML-style) paragraph spacing: `para.spacing` turns it on and off, and a number
/// replaces it, as typing one in Word's Paragraph dialog does; Ctrl+0 and Remove Space After work
/// on it too.
#[test]
fn spacing_commands_handle_automatic_spacing() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Pasted from the web"}));
    let rp = |s: &Session| s.doc.styles.resolve_para(&s.doc.para_at(&s.sel.focus).unwrap().props);
    run(&mut s, "para.spacing", json!({"beforeAuto": true, "afterAuto": true}));
    assert_eq!((rp(&s).space_before, rp(&s).space_after, rp(&s).before_auto), (14.0, 14.0, true));
    run(&mut s, "para.spacing", json!({"before": 6}));
    assert_eq!((rp(&s).space_before, rp(&s).before_auto, rp(&s).after_auto), (6.0, false, true), "a number replaces Auto");
    run(&mut s, "para.removeSpaceAfter", json!({}));
    assert_eq!((rp(&s).space_after, rp(&s).after_auto), (0.0, false));
    run(&mut s, "para.spacing", json!({"beforeAuto": true}));
    run(&mut s, "para.addSpaceBefore", json!({}));
    assert_eq!((rp(&s).space_before, rp(&s).before_auto), (0.0, false), "Ctrl+0 removes the automatic space");
    assert!(s.run("para.spacing", &json!({"beforeAuto": "yes"})).is_ok(), "junk is ignored, never a panic");
}

/// #712, as in Word: Ctrl+F (`edit.find` with no text) opens the Navigation pane and asks the UI
/// to put the keyboard in its search box, not the dialog; Ctrl+H (`edit.replace` with nothing to
/// replace) opens Find and Replace on its Replace tab instead of failing; Go To moves by pages from
/// the caret's page (`pageBy`).
#[test]
fn find_replace_and_go_to_open_like_word() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "cat dog cat"}));
    run(&mut s, "edit.find", json!({}));
    assert!(s.view.nav_pane);
    assert_eq!(s.ui_requests.last(), Some(&json!({"focus": "navSearch"})));
    assert!(!s.ui_requests.iter().any(|r| r.get("open").is_some()), "no dialog: {:?}", s.ui_requests);
    s.dirty = false;
    run(&mut s, "edit.findReplace", json!({}));
    assert_eq!(s.ui_requests.last(), Some(&json!({"open": "replace"})));
    assert!(!s.dirty, "opening the dialog changes nothing");
    assert!(s.run("edit.findReplace", &json!({"tab": "nope"})).is_err());
    let reg = s.registry.clone();
    assert_eq!(s.keymap.resolve(&reg, "Mod+H").map(|c| c.id), Some("edit.findReplace"));
    run(&mut s, "edit.goto", json!({}));
    assert_eq!(s.ui_requests.last(), Some(&json!({"open": "goto"})));
    // Replace with a "with" and the last search still replaces, as before.
    run(&mut s, "edit.find", json!({"text": "dog"}));
    run(&mut s, "edit.replace", json!({"with": "cow"}));
    assert_eq!(text(&s), "cat cow cat");

    // A replace with no `text` and no earlier search is a mistake, not "0 matches" (#3271).
    let mut s = super::tests::s();
    for (id, params) in [("edit.replaceAll", json!({"find": "cat", "replace": "dog"})), ("edit.replace", json!({})), ("edit.replaceAll", json!({}))] {
        let e = s.run(id, &params).unwrap_err().to_string();
        assert!(e.contains("`text` (string) is required"), "{id}: {e}");
    }
    run(&mut s, "edit.replaceAll", json!({"text": "", "with": "x"}));

    // Five pages; Go To +2 / -1 from where the caret is, clamped to the document.
    let mut s = super::tests::s();
    for i in 0..5 {
        if i > 0 {
            run(&mut s, "insert.pageBreak", json!({}));
        }
        run(&mut s, "text.insert", json!({"text": format!("Page {}", i + 1)}));
    }
    let page = |s: &mut Session| s.layout().caret_on(&s.sel.focus, s.page_hint).unwrap().page + 1;
    run(&mut s, "edit.goto", json!({"page": 1}));
    assert_eq!(page(&mut s), 1);
    run(&mut s, "edit.goto", json!({"pageBy": 2}));
    assert_eq!(page(&mut s), 3);
    run(&mut s, "edit.goto", json!({"pageBy": -1}));
    assert_eq!(page(&mut s), 2);
    run(&mut s, "edit.goto", json!({"pageBy": 99}));
    assert_eq!(page(&mut s), 5, "past the end: the last page");
    run(&mut s, "edit.goto", json!({"pageBy": -99}));
    assert_eq!(page(&mut s), 1, "before the start: the first page");
    assert!(s.run("edit.goto", &json!({"pageBy": i64::MIN})).is_ok(), "hostile numbers never panic");
}

/// #802: Insert › Picture takes Windows Metafiles (and EMF) at their own frame size, stored under
/// their own extension so a save writes them as WMF/EMF parts.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn insert_picture_takes_a_windows_metafile() {
    use wordcraft_doc::para::InlineObject;
    // One WMF record: size in words, function, parameters.
    let rec = |func: u16, params: &[i16]| {
        let mut v = ((3 + params.len()) as u32).to_le_bytes().to_vec();
        v.extend(func.to_le_bytes());
        params.iter().for_each(|p| v.extend(p.to_le_bytes()));
        v
    };
    // A placeable WMF, 1440 × 720 units at 1440 per inch (72 × 36 points), drawing a rectangle.
    let body = [rec(0x041B, &[720, 1440, 0, 0]), rec(0, &[])].concat();
    let mut wmf = 0x9AC6_CDD7u32.to_le_bytes().to_vec();
    for v in [0i16, 0, 0, 1440, 720] {
        wmf.extend(v.to_le_bytes());
    }
    wmf.extend(1440u16.to_le_bytes());
    wmf.extend([0u8; 6]);
    let words = ((wmf.len() + 18 + body.len()) / 2) as u32;
    for v in [1u16, 9, 0x300] {
        wmf.extend(v.to_le_bytes());
    }
    wmf.extend(words.to_le_bytes());
    wmf.extend(0u16.to_le_bytes());
    wmf.extend(5u32.to_le_bytes());
    wmf.extend(0u16.to_le_bytes());
    wmf.extend(body);
    let dir = scratch_dir("insert-wmf");
    let path = dir.join("logo.wmf");
    std::fs::write(&path, &wmf).unwrap();

    let mut s = Session::new(wordcraft_doc::Document::new());
    let r = run(&mut s, "insert.picture", json!({"path": path.to_string_lossy()}));
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(72.0), Some(36.0)), "{r}");
    let key = r["media"].as_str().unwrap().to_string();
    assert!(key.ends_with(".wmf"), "{key}");
    assert_eq!(s.doc.media.get(&key).map(|b| b.len()), Some(wmf.len()));
    let mut placed = false;
    s.doc.objects_in_reading_order(&mut |o| placed |= matches!(o, InlineObject::Image { media, .. } if *media == key));
    assert!(placed, "the picture is in the text");

    // Neither a picture nor a metafile: the same error as before, naming the formats.
    std::fs::write(&path, b"not a picture").unwrap();
    let err = s.run("insert.picture", &json!({"path": path.to_string_lossy()})).unwrap_err();
    assert!(err.to_string().contains("WMF, EMF"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Change Picture must keep the replacement's own format: the part's name and content type
/// come from the extension, so a JPEG stored as `imageN.png` saves declared `image/png` (Word
/// and LibreOffice then show a broken picture).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn picture_change_keeps_the_replacement_format() {
    let dir = scratch_dir("picture-change-format");
    let first = dir.join("first.png");
    std::fs::write(&first, tiny_png()).unwrap();
    let mut jpeg = Vec::new();
    image::RgbImage::from_pixel(2, 2, image::Rgb([30, 30, 200])).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
    let second = dir.join("second.jpg");
    std::fs::write(&second, &jpeg).unwrap();

    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "insert.picture", json!({"path": first.to_string_lossy()}));
    run(&mut s, "picture.change", json!({"path": second.to_string_lossy()}));
    let mut media = String::new();
    s.doc.objects_in_reading_order(&mut |o| {
        if let wordcraft_doc::para::InlineObject::Image { media: m, .. } = o {
            media = m.clone();
        }
    });
    assert!(media.ends_with(".jpeg") || media.ends_with(".jpg"), "the JPEG keeps its format: {media}");
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(back.media.get(&media).map(|b| b.as_slice()), Some(jpeg.as_slice()), "the saved part is the JPEG");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn json_open_repairs_hostile_run_lengths() {
    use wordcraft_doc::{Block, Document};
    // "héllo": é is two bytes (1..3); a run ending at 2 splits it, usize::MAX overflows a sum.
    for lens in [vec![2, 4], vec![usize::MAX, usize::MAX], vec![1], vec![], vec![0, 99]] {
        let mut d = Document::from_text("héllo");
        if let Some(Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) {
            p.runs = lens.iter().map(|&len| wordcraft_doc::para::Run { len, props: Default::default() }).collect();
        }
        let bytes = serde_json::to_vec(&d).unwrap();
        let (doc, _) = crate::io::open_bytes_with("x.json", &bytes, None).unwrap();
        let Some(Block::Para(p)) = doc.body.first().map(|b| &**b) else { panic!("no paragraph") };
        let mut end = 0;
        for (r, _) in p.run_ranges() {
            assert!(p.text.is_char_boundary(r.end) && r.end > r.start, "{lens:?}");
            end = r.end;
        }
        assert_eq!(end, p.text.len(), "{lens:?}");
    }
}

#[test]
fn outline_aliases_add_one_undo_step() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "one"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "two"}));
    for id in ["outline.promote", "outline.demote", "outline.moveUp", "outline.moveDown"] {
        let (doc, sel, depth) = (s.doc.clone(), s.sel.clone(), s.undo_depth());
        run(&mut s, id, json!({}));
        assert_ne!(s.doc, doc, "{id} changed nothing");
        assert_eq!(s.undo_depth(), depth + 1, "{id}");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc, doc, "{id}: one undo restores the document");
        assert_eq!(s.sel, sel, "{id}: one undo restores the selection");
        run(&mut s, "edit.redo", json!({}));
    }
}

/// Insert › Link › Place in This Document (#1234): a link to a bookmark, a heading or the top of
/// the document, the list of those places, and following the link.
#[test]
fn links_to_places_in_this_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Intro"}));
    run(&mut s, "para.style", json!({"style": "Heading1"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "para.style", json!({"style": "Normal"}));
    run(&mut s, "text.insert", json!({"text": "Body text here"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Details"}));
    run(&mut s, "para.style", json!({"style": "Heading2"}));
    run(&mut s, "select.text", json!({"text": "Body"}));
    run(&mut s, "insert.bookmark", json!({"name": "Mark1"}));
    let links = |s: &Session| -> Vec<(String, String)> {
        s.doc
            .body
            .iter()
            .filter_map(|b| b.as_para())
            .flat_map(|p| p.run_ranges().filter_map(|(r, c)| Some((c.link.clone()?, p.text.get(r)?.to_string()))).collect::<Vec<_>>())
            .collect()
    };
    // The places: the top, both headings with their levels, and the one visible bookmark.
    let t = run(&mut s, "link.targets", json!({}));
    assert_eq!(t["top"], true);
    let heads: Vec<(u64, String)> =
        t["headings"].as_array().unwrap().iter().map(|h| (h["level"].as_u64().unwrap(), h["text"].as_str().unwrap().to_string())).collect();
    assert_eq!(heads, vec![(1, "Intro".to_string()), (2, "Details".to_string())]);
    assert_eq!(t["bookmarks"], json!(["Mark1"]));
    // A bookmark link on the selection.
    run(&mut s, "select.text", json!({"text": "here"}));
    run(&mut s, "insert.link", json!({"bookmark": "Mark1"}));
    assert_eq!(links(&s), vec![("#Mark1".to_string(), "here".to_string())]);
    // A heading link at a caret inserts the heading's text; it reuses the heading's hidden bookmark.
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    run(&mut s, "insert.link", json!({"heading": t["headings"][1]["block"]}));
    let l = links(&s);
    assert_eq!(l.len(), 2, "{l:?}");
    let (hidden, shown) = l.iter().find(|(a, _)| a.starts_with("#_Toc")).cloned().unwrap();
    assert_eq!(shown, "Details", "{l:?}");
    let t2 = run(&mut s, "link.targets", json!({}));
    assert_eq!(t2["bookmarks"], json!(["Mark1"]), "the hidden bookmark is not listed");
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    run(&mut s, "insert.link", json!({"heading": t["headings"][1]["block"], "text": "again"}));
    assert!(links(&s).iter().all(|(a, _)| a == "#Mark1" || *a == hidden), "one bookmark for the heading: {:?}", links(&s));
    // The top of the document, and following the links.
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    run(&mut s, "insert.link", json!({"top": true}));
    assert!(links(&s).iter().any(|(a, t)| a == "#_top" && t == "Top of the Document"), "{:?}", links(&s));
    run(&mut s, "edit.goto", json!({"bookmark": "Mark1"}));
    assert_ne!(s.sel.focus, Pos::body(0, 0));
    run(&mut s, "edit.goto", json!({"bookmark": "_top"}));
    assert_eq!(s.sel.focus, Pos::body(0, 0));
    // Refusals change nothing.
    let rev = s.rev();
    assert!(s.run("insert.link", &json!({"bookmark": "Nope"})).is_err());
    assert!(s.run("insert.link", &json!({"heading": 1})).is_err(), "a body paragraph is not a heading");
    assert_eq!(s.rev(), rev);
}

/// A DOCX built in code: a tracked deletion inside a field result, as Word writes when a
/// formula field's result is edited (#1037). The first field's result is split across the field
/// end; the second has its whole result deleted.
fn tracked_field_result_docx() -> Vec<u8> {
    use std::io::Write;
    const WHO: &str = r#"w:author="Tester" w:date="2026-10-12T00:00:00Z""#;
    let field = |instr: &str, inside: &str, after: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instr} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{inside}<w:r><w:fldChar w:fldCharType="end"/></w:r>{after}"#
        )
    };
    let del = |id: u32, t: &str| format!(r#"<w:del w:id="{id}" {WHO}><w:r><w:delText>{t}</w:delText></w:r></w:del>"#);
    let ins = |id: u32, t: &str| format!(r#"<w:ins w:id="{id}" {WHO}><w:r><w:t>{t}</w:t></w:r></w:ins>"#);
    let p1 = field("=SUM(ABOVE)", &del(1, "$"), &(del(2, "150.00") + &ins(3, "$140.00")));
    let p2 = field("=2+3", &del(4, "5"), &ins(5, "five"));
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p>{p1}</w:p><w:p>{p2}</w:p></w:body></w:document>"#
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, value) in [
        ("word/document.xml", xml.as_str()),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn tracked_deletion_inside_a_field_result_is_a_revision_and_is_saved() {
    let doc = wordcraft_docx::read(&tracked_field_result_docx()).unwrap();
    // Revisions with one author and date are shared: one deletion, one insertion.
    let kinds: Vec<_> = doc.revisions.iter().map(|r| r.kind).collect();
    assert_eq!(kinds, [wordcraft_doc::RevisionKind::Delete, wordcraft_doc::RevisionKind::Insert]);
    // The deleted `$` is still in the text, marked deleted (not ordinary result text).
    let p = doc.body[0].as_para().unwrap();
    assert_eq!(p.text.replace(wordcraft_doc::para::OBJ, ""), "$150.00$140.00");
    // Saving keeps the deletion inside the result, with its text as delText.
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    let sep = xml.find(r#"w:fldCharType="separate""#).unwrap();
    let end = xml.find(r#"w:fldCharType="end""#).unwrap();
    let inside = &xml[sep..end];
    assert!(inside.contains("<w:del ") && inside.contains("<w:delText") && inside.contains('$'), "{inside}");
}

/// Paragraph texts without the field markers' placeholder characters.
fn visible_texts(s: &Session) -> Vec<String> {
    body_texts(s).iter().map(|t| t.replace(wordcraft_doc::para::OBJ, "")).collect()
}

#[test]
fn accept_and_reject_all_handle_a_tracked_deletion_inside_a_field_result() {
    let open = || Session::new(wordcraft_docx::read(&tracked_field_result_docx()).unwrap());
    let mut s = open();
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(visible_texts(&s), ["$140.00", "five"]);
    assert!(s.doc.revisions.is_empty());
    let mut s = open();
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(visible_texts(&s), ["$150.00", "5"]);
}

#[test]
fn rename_document_preserves_saved_contents_and_rejects_collisions() {
    let dir = std::env::temp_dir().join(format!("wordcraft-rename-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("original.docm");
    let renamed = dir.join("Notes.docm");
    let existing = dir.join("existing.docm");
    std::fs::write(&original, b"original bytes").unwrap();
    std::fs::write(&existing, b"keep these bytes").unwrap();
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.path = Some(original.clone());
    s.dirty = true;
    assert!(s.run("file.rename", &json!({"name": "existing"})).is_err());
    assert_eq!(s.path.as_ref(), Some(&original));
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep these bytes");
    let roots = crate::file_access::AutomationRoots::new(Some(&dir), None).unwrap();
    assert!(crate::file_access::confine(roots.as_ref(), || s.run("file.rename", &json!({"name": "Notes"}))).is_err());
    assert!(original.exists());
    assert!(!renamed.exists());
    s.run("file.rename", &json!({"name": " Notes "})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&renamed));
    assert!(s.dirty);
    assert!(!original.exists());
    assert_eq!(std::fs::read(&renamed).unwrap(), b"original bytes");
    s.run("file.rename", &json!({"name": "Notes"})).unwrap();
    s.run("file.rename", &json!({"name": "notes"})).unwrap();
    let lower = dir.join("notes.docm");
    assert_eq!(s.path.as_ref(), Some(&lower));
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(names.contains(&"notes.docm".into()) && !names.contains(&"Notes.docm".into()), "{names:?}");
    assert_eq!(std::fs::read(&lower).unwrap(), b"original bytes");
    assert!(s.run("file.rename", &json!({"name": "existing"})).is_err());
    assert_eq!(s.path.as_ref(), Some(&lower));
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep these bytes");

    // A name given with the document's extension is the whole file name: an agent or a script
    // passes one, and appending the extension again made "Report.docm.docm" (#2983).
    s.run("file.rename", &json!({"name": "Report.docm"})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&dir.join("Report.docm")));
    assert!(!dir.join("Report.docm.docm").exists());
    // Another extension, or the extension alone, is a stem and still gets one.
    s.run("file.rename", &json!({"name": "Report.2026"})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&dir.join("Report.2026.docm")));
    s.run("file.rename", &json!({"name": ".docm"})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&dir.join(".docm.docm")));
    assert_eq!(std::fs::read(dir.join(".docm.docm")).unwrap(), b"original bytes");
    // A name whose bytes do not line up with the extension's length: comparing by slicing at
    // `len - ext.len()` could land inside a character and panic (#2983 review).
    s.run("file.rename", &json!({"name": "abécde"})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&dir.join("abécde.docm")));
    s.run("file.rename", &json!({"name": "résumé.docm"})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&dir.join("résumé.docm")));
    // Whatever its case; the file is named as typed, so the extension follows the name.
    s.run("file.rename", &json!({"name": "Report.DOCM"})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&dir.join("Report.DOCM")));
    assert!(!dir.join("Report.DOCM.docm").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rename_unsaved_document_names_the_file_not_the_document() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.doc.core.title = "Quarterly report".into();
    for name in ["", "  ", "../report", "a/b", "a\\b", "a\nb", "name."] {
        assert!(s.run("file.rename", &json!({"name": name})).is_err(), "{name:?}");
        assert!(s.file_name.is_none());
    }
    s.run("file.rename", &json!({"name": " Résumé notes "})).unwrap();
    assert_eq!(s.file_name.as_deref(), Some("Résumé notes"));
    assert_eq!(s.doc.core.title, "Quarterly report");
    assert!(s.path.is_none());
    assert!(!s.dirty);
    assert!(!s.can_undo());
    s.run("file.new", &json!({})).unwrap();
    assert!(s.file_name.is_none());
    s.run("file.rename", &json!({"name": "Draft"})).unwrap();
    s.run("file.open", &json!({"path": "other.txt", "data": "aGk="})).unwrap();
    assert!(s.file_name.is_none());
    s.run("file.new", &json!({})).unwrap();
    s.run("file.rename", &json!({"name": "Draft"})).unwrap();
    s.run("review.compare", &json!({"text": "changed", "showIn": "new"})).unwrap();
    assert!(s.file_name.is_none() && s.path.is_none());
}

/// A version snapshot belongs to the document it was taken from: opening another document (or
/// starting a new one) starts a new version history, so a restore can't put the previous
/// document's content into the open one (#1327 follow-up).
#[test]
fn a_version_snapshot_does_not_survive_a_document_switch() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("First document."));
    s.run("file.versions", &json!({"save": "snapshot"})).unwrap();
    assert_eq!(s.versions.len(), 1);

    // A new document: the snapshot of the previous one is gone with it.
    s.run("file.new", &json!({})).unwrap();
    assert!(s.versions.is_empty(), "the snapshot of the previous document survived the switch");
    assert!(s.run("file.versions", &json!({"restore": 0})).is_err(), "the old snapshot is still restorable");
    assert_ne!(s.doc.plain_text(StoryRef::Body), "First document.", "the old document's content came back");

    // The same for an opened file.
    s.run("file.versions", &json!({"save": "snapshot"})).unwrap();
    s.run("file.open", &json!({"path": "other.txt", "data": "aGk="})).unwrap();
    assert!(s.versions.is_empty(), "the snapshot of the previous document survived the switch");
    assert!(s.run("file.versions", &json!({"restore": 0})).is_err(), "the old snapshot is still restorable");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "hi");

    // A snapshot of the open document still restores into it.
    s.run("file.versions", &json!({"save": "snapshot"})).unwrap();
    s.run("text.insert", &json!({"text": " there"})).unwrap();
    s.run("file.versions", &json!({"restore": 0})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "hi");
}

/// A comparison that opens the original replaces the open document, so the version snapshots
/// of the document it replaces go with it: a restore must not put the previous document back
/// over the comparison result (#1327 follow-up).
#[test]
fn a_version_snapshot_does_not_survive_a_comparison() {
    let dir = std::env::temp_dir().join(format!("wordcraft-compare-versions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("original.docx");
    let mut other = Session::new(wordcraft_doc::Document::from_text("Original document."));
    other.run("file.save", &json!({"path": original.to_string_lossy()})).unwrap();

    let mut s = Session::new(wordcraft_doc::Document::from_text("Current document."));
    s.run("file.versions", &json!({"save": "snapshot"})).unwrap();
    assert_eq!(s.versions.len(), 1);
    s.run("review.compare", &json!({"original": original.to_string_lossy(), "text": "Revised document."})).unwrap();
    assert!(s.versions.is_empty(), "the snapshot of the previous document survived the comparison");
    assert!(s.run("file.versions", &json!({"restore": 0})).is_err(), "the old snapshot is still restorable");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Compare's new and revised results are untitled documents even without an `original` path.
#[test]
fn a_version_snapshot_does_not_survive_an_untitled_comparison() {
    for show_in in ["new", "revised"] {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Current document."));
        s.run("file.versions", &json!({"save": "snapshot"})).unwrap();
        s.run("review.compare", &json!({"text": "Revised document.", "showIn": show_in})).unwrap();
        assert!(s.versions.is_empty(), "old snapshots survived a {show_in} comparison");
        assert!(s.run("file.versions", &json!({"restore": 0})).is_err());
        assert!(s.path.is_none() && s.file_name.is_none());
    }
}

/// A comparison with an `original` replaces the open document in place — the handler loads the
/// original into `s.doc` without `set_document` — so `document_id` does not move and two
/// identical comparisons produce identical documents. A comparison is an edit of its own all
/// the same: the `replaces_document` guard keeps its undo step (#4004 review).
#[test]
fn a_repeated_comparison_is_still_an_edit() {
    let dir = std::env::temp_dir().join(format!("wordcraft-compare-twice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("original.docx");
    let mut other = Session::new(wordcraft_doc::Document::from_text("Original document."));
    other.run("file.save", &json!({"path": original.to_string_lossy()})).unwrap();

    let mut s = Session::new(wordcraft_doc::Document::from_text("Current document."));
    let compare = json!({"original": original.to_string_lossy(), "text": "Revised document."});
    s.run("review.compare", &compare).unwrap();
    let depth = s.undo_depth();
    assert!(s.dirty);
    s.run("review.compare", &compare).unwrap();
    assert_eq!(s.undo_depth(), depth + 1, "the second comparison is an edit of its own");
    assert!(s.dirty, "the comparison result is unsaved");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rejected comparison leaves the current document and its snapshots together.
#[test]
fn a_rejected_comparison_keeps_the_current_versions() {
    use wordcraft_doc::{CharProps, InlineObject};

    let mut s = Session::new(wordcraft_doc::Document::from_text("Editable. Locked."));
    s.path = Some(std::path::PathBuf::from("original.docx"));
    s.file_name = Some("original.docx".into());
    s.run("file.versions", &json!({"save": "snapshot"})).unwrap();
    let end = s.doc.end_of(StoryRef::Body);
    s.doc.insert_object(&end, InlineObject::PermEnd { id: 1 }, &CharProps::default()).unwrap();
    let start = s.doc.start_of(StoryRef::Body);
    s.doc.insert_object(&start, InlineObject::PermStart { id: 1, group: String::new(), editor: s.author.clone() }, &CharProps::default()).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    let before = s.doc.clone();

    assert!(s.run("review.compare", &json!({"text": "Different document.", "showIn": "new"})).is_err());
    assert_eq!(s.doc, before);
    assert_eq!(s.versions.len(), 1, "a rejected comparison discarded the current document's snapshot");
    assert_eq!(s.path.as_deref(), Some(std::path::Path::new("original.docx")));
    assert_eq!(s.file_name.as_deref(), Some("original.docx"));
}

/// A mailing result opens beside a protected source document, which keeps its save target,
/// versions, password and text.
#[test]
fn a_mailing_result_beside_a_protected_document_keeps_its_file_identity() {
    use wordcraft_doc::{CharProps, InlineObject};

    let mut s = Session::new(wordcraft_doc::Document::from_text("Protected source."));
    s.path = Some(std::path::PathBuf::from("source.docx"));
    s.file_name = Some("source.docx".into());
    s.run("file.versions", &json!({"save": "snapshot"})).unwrap();
    let end = s.doc.end_of(StoryRef::Body);
    s.doc.insert_object(&end, InlineObject::PermEnd { id: 1 }, &CharProps::default()).unwrap();
    let start = s.doc.start_of(StoryRef::Body);
    s.doc.insert_object(&start, InlineObject::PermStart { id: 1, group: String::new(), editor: s.author.clone() }, &CharProps::default()).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    let before = s.doc.clone();
    let before_id = s.document_id();
    let before_dirty = s.dirty;
    s.password = Some(crate::io::Password::new("synthetic file password"));
    s.bib_style = "Chicago".into();

    s.run("mailings.envelopes", &json!({"delivery": "Recipient"})).unwrap();
    assert_ne!(s.document_id(), before_id, "the envelope is a new document");
    s.activate(before_id).unwrap();
    assert_eq!(s.doc, before);
    assert_eq!(s.versions.len(), 1);
    assert_eq!(s.path.as_deref(), Some(std::path::Path::new("source.docx")));
    assert_eq!(s.file_name.as_deref(), Some("source.docx"));
    assert_eq!(s.document_id(), before_id);
    assert_eq!(s.dirty, before_dirty);
    assert_eq!(s.password.as_ref().map(crate::io::Password::as_str), Some("synthetic file password"));
    assert_eq!(s.bib_style, "Chicago");
}

/// #1344: File › Info › Inspect Document finds hidden text by its effective formatting (direct,
/// character style, paragraph style) in every story — body, header, footer, footnote, comment,
/// text box — and Remove All deletes exactly that text, in one undo step, through a DOCX save.
#[test]
fn inspect_document_hidden_text_in_every_story_and_from_styles() {
    use wordcraft_doc::props::{CharProps, ParaProps};
    use wordcraft_doc::styles::{Style, StyleKind};
    use wordcraft_doc::{Paragraph, PartKind, para_block};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Body words"}));
    run(&mut s, "review.newComment", json!({"text": "c"}));
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "insert.textBox", json!({"text": "t"}));
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "references.footnote", json!({"text": "n"}));
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "references.endnote", json!({"text": "e"}));
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "insert.header", json!({"text": "h"}));
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "insert.footer", json!({"text": "f"}));
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    let hidden = CharProps { hidden: Some(true), ..Default::default() };
    s.doc.styles.upsert(Style { id: "HideC".into(), name: "Hide C".into(), kind: StyleKind::Character, chr: hidden.clone(), ..Default::default() });
    s.doc.styles.upsert(Style {
        id: "HideP".into(),
        name: "Hide P".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        chr: hidden.clone(),
        ..Default::default()
    });
    let plain = CharProps::default();
    let blocks = |tag: &str| {
        let mut a = Paragraph::with_text(&format!("keep-{tag}-1 "), plain.clone());
        for (t, c) in [
            (format!("hide-{tag}-direct"), hidden.clone()),
            (format!(" keep-{tag}-2 "), plain.clone()),
            (format!("hide-{tag}-cstyle"), CharProps { style: Some("HideC".into()), ..Default::default() }),
            (format!(" keep-{tag}-3"), plain.clone()),
        ] {
            let at = a.len();
            a.insert_text(at, &t, &c).unwrap();
        }
        let mut b = Paragraph::with_text(&format!("hide-{tag}-pstyle"), plain.clone());
        b.props = ParaProps { style: Some("HideP".into()), ..Default::default() };
        // Shown again by direct formatting over the hidden paragraph style: it stays.
        let mut c = Paragraph::with_text(&format!("keep-{tag}-4"), CharProps { hidden: Some(false), ..Default::default() });
        c.props = ParaProps { style: Some("HideP".into()), ..Default::default() };
        vec![para_block(a), para_block(b), para_block(c)]
    };
    let tag = |k: PartKind| match k {
        PartKind::Header => "header",
        PartKind::Footer => "footer",
        PartKind::Footnote => "footnote",
        PartKind::Endnote => "endnote",
        PartKind::Comment => "comment",
        PartKind::TextBox => "textBox",
    };
    s.doc.body.extend(blocks("body"));
    // A table cell in the body, with the same pieces.
    let mut table = wordcraft_doc::table::Table::new(1, 2, 200.0);
    table.rows[0].cells[0].blocks = blocks("cell");
    s.doc.body.push(std::sync::Arc::new(wordcraft_doc::Block::Table(table)));
    s.doc.body.push(para_block(Paragraph::new()));
    let mut tags = vec![("body", "body"), ("cell", "body")];
    for part in s.doc.parts.values_mut() {
        part.blocks.extend(blocks(tag(part.kind)));
        tags.push((tag(part.kind), tag(part.kind)));
    }
    tags.sort();
    assert_eq!(tags.iter().map(|t| t.0).collect::<Vec<_>>(), ["body", "cell", "comment", "endnote", "footer", "footnote", "header", "textBox"]);
    let stories = |d: &wordcraft_doc::Document| std::iter::once(StoryRef::Body).chain(d.parts.keys().map(|k| StoryRef::Part(*k))).collect::<Vec<_>>();
    let para_counts = |d: &wordcraft_doc::Document| stories(d).into_iter().map(|st| (st, d.para_paths(st).len())).collect::<Vec<_>>();
    let counts_before = para_counts(&s.doc);
    let all_text = |d: &wordcraft_doc::Document| -> String {
        std::iter::once(StoryRef::Body).chain(d.parts.keys().map(|k| StoryRef::Part(*k))).map(|st| d.plain_text(st)).collect::<Vec<_>>().join("\n")
    };

    // Inspect reports every hidden piece, in its story.
    let found = run(&mut s, "file.inspect", json!({}))["found"].clone();
    assert_eq!(found["hiddenText"], true);
    let items = found["hidden"].as_array().cloned().unwrap_or_default();
    for (t, story) in &tags {
        for how in ["direct", "cstyle", "pstyle"] {
            let want = format!("hide-{t}-{how}");
            assert!(
                items.iter().any(|i| i["story"] == *story && i["text"].as_str().is_some_and(|x| x.contains(&want))),
                "{want} not reported: {items:?}"
            );
        }
    }
    assert!(!items.iter().any(|i| i["text"].as_str().is_some_and(|x| x.contains("keep-"))), "visible text reported: {items:?}");

    // Remove All: every hidden piece goes, every visible one stays, in one undo step.
    let before = all_text(&s.doc);
    let undo_len = s.undo_labels().len();
    let out = run(&mut s, "file.inspect", json!({"remove": ["hidden"]}));
    assert_eq!(out["residual"], json!([]), "{out}");
    let after = all_text(&s.doc);
    for (t, _) in &tags {
        for n in 1..=4 {
            assert!(after.contains(&format!("keep-{t}-{n}")), "keep-{t}-{n} lost:\n{after}");
        }
    }
    assert!(!after.contains("hide-"), "hidden text left:\n{after}");
    // The paragraphs hidden through their style went, marks and all (the body lost two: its own
    // and the table cell's); the paragraph shown again by direct formatting stayed.
    let expected: Vec<_> = counts_before.iter().map(|(st, n)| (*st, n - if *st == StoryRef::Body { 2 } else { 1 })).collect();
    assert_eq!(para_counts(&s.doc), expected);
    assert_eq!(s.undo_labels().len(), undo_len + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(all_text(&s.doc), before, "one undo brings it all back");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(all_text(&s.doc), after);
    assert_eq!(run(&mut s, "file.inspect", json!({}))["found"]["hiddenText"], false);

    // Save and reopen: none of the hidden strings is in the file.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let reopened = all_text(&back);
    assert!(!reopened.contains("hide-"), "hidden text in the saved file:\n{reopened}");
    assert!(reopened.contains("keep-header-4") && reopened.contains("keep-comment-3"), "{reopened}");
    assert!(reopened.contains("keep-endnote-2") && reopened.contains("keep-cell-4"), "{reopened}");
}

fn hidden_props() -> wordcraft_doc::props::CharProps {
    wordcraft_doc::props::CharProps { hidden: Some(true), ..Default::default() }
}

/// #1344: document defaults that hide text hide it everywhere they aren't overridden.
#[test]
fn inspect_document_removes_text_hidden_by_document_defaults() {
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Paragraph, para_block};
    let mut s = s();
    s.doc.styles.default_chr.hidden = Some(true);
    s.doc.body = vec![
        para_block(Paragraph::with_text("hide-default", CharProps::default())),
        para_block(Paragraph::with_text("keep-default", CharProps { hidden: Some(false), ..Default::default() })),
    ];
    let found = run(&mut s, "file.inspect", json!({}))["found"].clone();
    assert!(found["hidden"].as_array().unwrap().iter().any(|i| i["text"] == "hide-default"), "{found}");
    let out = run(&mut s, "file.inspect", json!({"remove": ["hidden"]}));
    assert_eq!(out["residual"], json!([]), "{out}");
    assert_eq!(text(&s), "keep-default");
    assert_eq!(s.doc.para_paths(StoryRef::Body).len(), 1);
}

/// #1344: a hidden piece holding a content control's end keeps the marker (its start is outside).
#[test]
fn inspect_document_keeps_content_control_markers() {
    use wordcraft_doc::para::InlineObject;
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Paragraph, para_block};
    let mut s = s();
    let mut p = Paragraph::with_text("keep-a ", CharProps::default());
    let plain = CharProps::default();
    p.insert_object(p.len(), InlineObject::ControlStart { control: Box::default() }, &plain).unwrap();
    p.insert_text(p.len(), "keep-cc ", &plain).unwrap();
    p.insert_text(p.len(), "hide-cc", &hidden_props()).unwrap();
    p.insert_object(p.len(), InlineObject::ControlEnd, &hidden_props()).unwrap();
    p.insert_text(p.len(), "hide-after", &hidden_props()).unwrap();
    p.insert_text(p.len(), " keep-b", &plain).unwrap();
    s.doc.body = vec![para_block(p)];
    let out = run(&mut s, "file.inspect", json!({"remove": ["hidden"]}));
    assert_eq!(out["residual"], json!([]), "{out}");
    let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(p.text.replace('\u{FFFC}', ""), "keep-a keep-cc  keep-b");
    let objs: Vec<_> = p.object_offsets().into_iter().filter_map(|o| p.object_at(o)).collect();
    assert!(matches!(objs.as_slice(), [InlineObject::ControlStart { .. }, InlineObject::ControlEnd]), "{objs:?}");
}

/// #1344: text hidden by a table style isn't resolved by the inspector, so it says so: the table
/// is listed as unsupported and the removal's residual isn't empty.
#[test]
fn inspect_document_lists_tables_whose_style_hides_text() {
    use wordcraft_doc::styles::{Style, StyleKind, TableStyleParts};
    let mut s = s();
    s.doc.styles.upsert(Style {
        id: "HideHeader".into(),
        name: "Hide Header".into(),
        kind: StyleKind::Table,
        table: Some(TableStyleParts { header_chr: hidden_props(), ..Default::default() }),
        ..Default::default()
    });
    let mut table = wordcraft_doc::table::Table::new(2, 2, 200.0);
    table.props.style = Some("HideHeader".into());
    s.doc.body.insert(0, std::sync::Arc::new(wordcraft_doc::Block::Table(table)));
    let found = run(&mut s, "file.inspect", json!({}))["found"].clone();
    let unsupported = found["unsupported"].as_array().cloned().unwrap_or_default();
    assert!(unsupported.iter().any(|u| u["object"] == "table" && u["story"] == "body" && u["style"] == "HideHeader"), "{found}");
    let out = run(&mut s, "file.inspect", json!({"remove": ["hidden"]}));
    assert!(out["residual"].as_array().is_some_and(|r| r.iter().any(|u| u["object"] == "table")), "{out}");
}

/// #1344: removing a hidden footnote reference removes its note too, as deleting a reference does,
/// and reports the note's text.
#[test]
fn inspect_document_removes_the_note_of_a_hidden_reference() {
    use wordcraft_doc::PartKind;
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Body"}));
    let id = run(&mut s, "references.footnote", json!({"text": "note-visible"}))["id"].as_u64().unwrap() as u32;
    let path = wordcraft_doc::Path::top(0);
    let p = s.doc.para_mut(StoryRef::Body, &path).unwrap();
    let off = p.object_offsets()[0];
    p.format(off, off + '\u{FFFC}'.len_utf8(), &|c| c.hidden = Some(true)).unwrap();
    let out = run(&mut s, "file.inspect", json!({"remove": ["hidden"]}));
    assert_eq!(out["residual"], json!([]), "{out}");
    assert!(!s.doc.parts.contains_key(&id), "orphan note part left");
    let notes = out["removedNotes"].as_array().cloned().unwrap_or_default();
    assert!(notes.iter().any(|n| n["kind"] == "footnote" && n["text"].as_str().is_some_and(|t| t.contains("note-visible"))), "{out}");
    assert_eq!(text(&s), "Body");
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert!(!back.parts.values().any(|p| p.kind == PartKind::Footnote), "note in the saved file");
    assert_eq!(back.plain_text(StoryRef::Body), "Body");
}

/// #1344: many hidden pieces (index entries are hidden text) are removed quickly, and the
/// found list is capped.
#[test]
fn inspect_document_removes_thousands_of_hidden_pieces() {
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Paragraph, para_block};
    let mut s = s();
    s.doc.body = (0..2000)
        .map(|i| {
            let mut p = Paragraph::with_text(&format!("word{i} "), CharProps::default());
            p.insert_text(p.len(), &format!("XE \"entry{i}\""), &hidden_props()).unwrap();
            p.insert_text(p.len(), " more", &CharProps::default()).unwrap();
            para_block(p)
        })
        .collect();
    let t = std::time::Instant::now();
    let found = run(&mut s, "file.inspect", json!({}))["found"].clone();
    assert_eq!(found["hiddenCount"], 2000);
    assert_eq!(found["hidden"].as_array().map(Vec::len), Some(1000));
    assert_eq!(found["truncated"], true);
    let out = run(&mut s, "file.inspect", json!({"remove": ["hidden"]}));
    assert_eq!(out["residual"], json!([]));
    assert!(!text(&s).contains("XE"));
    assert!(text(&s).contains("word1999  more"));
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
}

/// #1344: a hidden paragraph between two tables stays (emptied, its mark still hidden), or the
/// tables would join into one when the file is reopened.
#[test]
fn inspect_document_keeps_a_hidden_paragraph_between_tables() {
    use wordcraft_doc::{Block, Paragraph, para_block};
    let mut s = s();
    let table = || std::sync::Arc::new(Block::Table(wordcraft_doc::table::Table::new(1, 1, 100.0)));
    let mut p = Paragraph::with_text("hide-between", hidden_props());
    p.mark.hidden = Some(true);
    s.doc.body = vec![table(), para_block(p), table(), para_block(Paragraph::with_text("keep-end", Default::default()))];
    let out = run(&mut s, "file.inspect", json!({"remove": ["hidden"]}));
    assert_eq!(out["residual"], json!([]), "{out}");
    let kinds = |d: &wordcraft_doc::Document| d.body.iter().map(|b| matches!(**b, Block::Table(_))).collect::<Vec<_>>();
    assert_eq!(kinds(&s.doc), [true, false, true, false]);
    let kept = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap();
    assert!(kept.is_empty() && kept.mark.hidden == Some(true), "emptied, mark still hidden: {kept:?}");
    assert!(!s.doc.plain_text(StoryRef::Body).contains("hide-"));
    assert_eq!(run(&mut s, "file.inspect", json!({}))["found"]["hiddenText"], false);
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(kinds(&back).iter().filter(|t| **t).count(), 2, "the tables stay apart");
}

/// #1381: the reporter's sample HTML (synthetic, 1.7 KB).
const SAMPLE_1381: &str = r##"<html><head><meta charset="utf-8"><title>Testo iniziale</title></head><body>
<h2 style="color:#1c3a70;">Premessa</h2>
<p>Il presente documento descrive l&rsquo;organizzazione della <strong>struttura sanitaria di prova</strong> e le misure
adottate per la <em>gestione del rischio clinico</em>. I contenuti sono <span style="color:#c00000;">sintetici</span>
e servono solo a verificare la resa della conversione.</p>
<p style="text-align:justify;">Le attività sono svolte secondo le raccomandazioni ministeriali vigenti e sono oggetto
di revisione annuale da parte del comitato per la valutazione dei sinistri.</p>
<ul>
<li>Identificazione dei rischi per reparto</li>
<li>Analisi degli eventi avversi e dei <em>near miss</em>
<ul><li>segnalazione spontanea</li><li>revisione delle cartelle cliniche</li></ul></li>
<li>Piano annuale di miglioramento</li>
</ul>
<ol>
<li>Raccolta delle segnalazioni</li>
<li>Classificazione dell&rsquo;evento</li>
<li>Azioni correttive</li>
</ol>
<table border="1" cellpadding="4" cellspacing="0" style="width:100%;border-collapse:collapse;">
<thead><tr style="background-color:#e8f0fa;"><th>Indicatore</th><th>Valore atteso</th><th>Valore rilevato</th></tr></thead>
<tbody>
<tr><td>Segnalazioni analizzate entro 30 giorni</td><td style="text-align:right;">&ge; 90%</td><td style="text-align:right;">93%</td></tr>
<tr><td>Audit clinici svolti</td><td style="text-align:right;">4</td><td style="text-align:right;">5</td></tr>
<tr><td colspan="2">Totale azioni correttive aperte</td><td style="text-align:right;">12</td></tr>
</tbody></table>
<p style="font-size:9pt;color:#595959;">Fonte: elaborazione interna su dati sintetici.</p>
</body></html>"##;

/// The first table's grid width in points.
fn first_table_width(doc: &wordcraft_doc::Document) -> f32 {
    doc.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t.grid.iter().sum::<f32>()) } else { None }).unwrap()
}

#[test]
fn opened_html_text_and_markdown_get_the_region_paper() {
    // #1381: a file that gives no page size opens on the region's paper, as a new blank document
    // does (#838): A4 in Italy, Letter in the US. Tables fit that page.
    let dir = std::env::temp_dir().join(format!("wordcraft-region-paper-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let html = dir.join("S05.html");
    std::fs::write(&html, SAMPLE_1381).unwrap();
    for (tag, size) in [("it-IT", (595.28, 841.89)), ("en-US", (612.0, 792.0))] {
        let mut s = s();
        s.system_region = wordcraft_geom::Region::from_tag(tag);
        // By path (pictures load from beside the file) and by bytes (the web, MCP).
        run(&mut s, "file.open", json!({"path": html.to_string_lossy()}));
        let sp = s.doc.last_section.clone();
        assert_eq!((sp.page_w, sp.page_h), size, "{tag}: HTML by path");
        assert!(first_table_width(&s.doc) <= sp.text_width() + 0.5, "{tag}: the table fits the page");
        for (name, body) in [("S05.html", SAMPLE_1381), ("notes.txt", "Hello"), ("notes.md", "# Title\n\n|a|b|\n|-|-|\n|1|2|\n")] {
            let data = crate::cmd::insert::base64_encode(body.as_bytes());
            run(&mut s, "file.open", json!({"path": name, "data": data}));
            let sp = &s.doc.last_section;
            assert_eq!((sp.page_w, sp.page_h), size, "{tag}: {name}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opened_documents_with_their_own_page_size_keep_it() {
    // #1381: only files without a page size take the region's paper.
    for (w, h) in [(500.0, 700.0), (612.0, 792.0)] {
        let mut doc = wordcraft_doc::Document::from_text("Own size");
        doc.last_section.page_w = w;
        doc.last_section.page_h = h;
        for name in ["own.docx", "own.rtf", "own.odt"] {
            let bytes = crate::io::save_bytes(name, &doc).unwrap();
            let mut s = s();
            s.system_region = wordcraft_geom::Region::from_tag("it-IT");
            run(&mut s, "file.open", json!({"path": name, "data": crate::cmd::insert::base64_encode(&bytes)}));
            let sp = &s.doc.last_section;
            assert_eq!((sp.page_w.round(), sp.page_h.round()), (w, h), "{name}");
        }
    }
}

#[test]
fn html_converted_to_docx_keeps_bold_headers_and_right_aligned_cells() {
    // #1381: `wordcraft-cli convert S05.html out.docx`: the <th> runs are bold and the
    // `text-align:right` cells' paragraphs carry `w:jc w:val="right"`.
    use std::io::Read;
    let mut s = s();
    run(&mut s, "file.open", json!({"path": "S05.html", "data": crate::cmd::insert::base64_encode(SAMPLE_1381.as_bytes())}));
    let bytes = crate::io::save_bytes("out.docx", &s.doc).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    let cell = |text: &str| {
        let at = xml.find(&format!(">{text}<")).unwrap_or_else(|| panic!("{text} in {xml}"));
        let start = xml[..at].rfind("<w:tc>").or_else(|| xml[..at].rfind("<w:tc ")).unwrap();
        let end = at + xml[at..].find("</w:tc>").unwrap();
        xml[start..end].to_string()
    };
    for head in ["Indicatore", "Valore atteso", "Valore rilevato"] {
        assert!(cell(head).contains("<w:b/>"), "{head}: {}", cell(head));
    }
    for right in ["93%", "4", "5", "12"] {
        assert!(cell(right).contains(r#"<w:jc w:val="right"/>"#), "{right}: {}", cell(right));
    }
    let left = cell("Audit clinici svolti");
    assert!(!left.contains(r#"<w:jc w:val="right"/>"#) && !left.contains("<w:b/>"), "{left}");
}

#[test]
fn exported_page_images_draw_headers_at_full_strength() {
    // file.exportPng (and `wordcraft-cli render`) drew headers and footers dimmed, as the screen
    // does while the body is edited; Word's exported pages and WordCraft's PDF don't.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Body"}));
    let mut hp = wordcraft_doc::Paragraph::with_text("HEADER", wordcraft_doc::props::CharProps { size: Some(28.0), ..Default::default() });
    hp.props.align = Some(wordcraft_doc::props::Align::Left);
    let id = s.doc.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
    s.doc.last_section.headers.default = Some(id);
    let dir = std::env::temp_dir().join(format!("wordcraft-png-header-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.png");
    s.run("file.exportPng", &json!({"path": path.to_string_lossy(), "scale": 1.0})).unwrap();
    let img = image::open(&path).unwrap().to_luma8();
    // The darkest pixel in the top tenth of the page (the header) is black, not grey.
    let top = img.height() / 10;
    let darkest = (0..top).flat_map(|y| (0..img.width()).map(move |x| (x, y))).map(|(x, y)| img.get_pixel(x, y)[0]).min().unwrap();
    assert!(darkest < 60, "header drawn dimmed: darkest pixel {darkest}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Shape Format › Shape Fill with transparency and gradients (#1500): each form sets the fill of
/// every selected shape in one undo step; a colour alone keeps the transparency; hostile values
/// are clamped; the old `{"color"}` form still works.
#[test]
fn shape_fill_transparency_and_gradients() {
    use wordcraft_doc::fill::{Fill, GradientKind, PathShape};
    use wordcraft_doc::props::Rgb;
    let mut s = s();
    run(&mut s, "insert.shape", json!({"kind": "rectangle"}));
    run(&mut s, "select.collapse", json!({"end": true}));
    run(&mut s, "insert.shape", json!({"kind": "ellipse"}));
    run(&mut s, "select.objects", json!({"index": 0}));
    run(&mut s, "select.addObject", json!({"index": 1}));
    let fills = |s: &Session| -> Vec<Option<Fill>> {
        cmd::objects::all_objects(s)
            .into_iter()
            .map(|(_, o)| match o {
                wordcraft_doc::InlineObject::Shape { fill, .. } => fill,
                o => panic!("{o:?}"),
            })
            .collect()
    };
    let before = fills(&s);
    run(&mut s, "shape.fill", json!({"color": "112233", "transparency": 40}));
    let solid = Fill::Solid { color: Rgb(0x11, 0x22, 0x33), transparency: 40.0 };
    assert_eq!(fills(&s), [Some(solid.clone()), Some(solid)], "both selected shapes");
    run(&mut s, "shape.fill", json!({"color": "445566"}));
    assert_eq!(fills(&s)[1], Some(Fill::Solid { color: Rgb(0x44, 0x55, 0x66), transparency: 40.0 }), "a colour alone keeps the transparency");
    run(
        &mut s,
        "shape.fill",
        json!({"gradient": {"type": "rectangular", "focus": [50, 50, 50, 50], "stops": [{"pos": 100, "color": "FFFFFF", "transparency": 30}, {"pos": 0, "color": "000000"}]}}),
    );
    let Some(Fill::Gradient(g)) = &fills(&s)[0] else { panic!("{:?}", fills(&s)) };
    assert_eq!(g.kind, GradientKind::Path { shape: PathShape::Rect, focus: [50.0; 4] });
    assert_eq!(g.stops.iter().map(|st| (st.pos, st.color, st.transparency)).collect::<Vec<_>>(), [(0.0, Rgb::BLACK, 0.0), (100.0, Rgb::WHITE, 30.0)]);
    assert!(g.rot_with_shape, "turns with the shape, as Word's new gradients do");
    run(
        &mut s,
        "shape.fill",
        json!({"gradient": {"type": "linear", "angle": -90, "stops": [{"pos": -5, "color": "FF0000", "transparency": 500}, {"pos": 1e9, "color": "0000FF"}]}}),
    );
    let Some(Fill::Gradient(g)) = &fills(&s)[1] else { panic!() };
    assert_eq!(g.kind, GradientKind::Linear { angle: 270.0, scaled: false });
    assert_eq!((g.stops[0].pos, g.stops[0].transparency, g.stops[1].pos), (0.0, 100.0, 100.0));
    // One undo step per command, for every shape.
    run(&mut s, "edit.undo", json!({}));
    assert!(matches!(fills(&s)[1], Some(Fill::Gradient(_))));
    for _ in 0..3 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(fills(&s), before);
    run(&mut s, "shape.fill", json!({"color": null}));
    assert_eq!(fills(&s), [None, None]);
    for bad in [
        json!({"gradient": {"stops": []}}),
        json!({"gradient": {"type": "spiral", "stops": [{"pos": 0, "color": "000000"}, {"pos": 100, "color": "FFFFFF"}]}}),
        json!({"gradient": 5}),
        json!({"color": "nope"}),
    ] {
        assert!(s.run("shape.fill", &bad).is_err(), "{bad}");
    }
}

/// A drag in the Format Shape pane applies its value as it goes and undoes in one step.
#[test]
fn steps_since_a_mark_merge_into_one_undo_step() {
    let mut s = s();
    run(&mut s, "insert.shape", json!({"kind": "rectangle"}));
    let fill = |s: &Session| match cmd::objects::selected(s) {
        Some((_, wordcraft_doc::InlineObject::Shape { fill, .. })) => fill,
        o => panic!("{o:?}"),
    };
    let before = fill(&s);
    let mark = s.undo_mark();
    for t in [10, 20, 30] {
        run(&mut s, "shape.fill", json!({"color": "112233", "transparency": t}));
    }
    s.merge_undo_since(mark);
    assert_eq!(s.undo_mark(), mark + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fill(&s), before);
    s.merge_undo_since(mark + 5);
    assert_eq!(s.undo_mark(), mark, "nothing newer: nothing merged");
}

/// Text typed right after a footnote or endnote number is plain body text, not another raised number.
#[test]
fn typing_after_a_note_number_is_not_superscript() {
    for (cmd_id, style) in [("references.footnote", "FootnoteReference"), ("references.endnote", "EndnoteReference")] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "One"}));
        run(&mut s, cmd_id, json!({"text": "Note"}));
        s.sel = crate::Selection::caret(s.doc.end_of(StoryRef::Body));
        assert_eq!(s.doc.para_at(&s.sel.focus).and_then(|p| p.props_at(p.len()).style.clone()).as_deref(), Some(style));
        assert_eq!(s.typing_props().style, None, "{cmd_id}");
        run(&mut s, "text.insert", json!({"text": " after"}));
        let end = s.doc.end_of(StoryRef::Body);
        assert_eq!(s.doc.para_at(&end).map(|p| p.props_of_char(p.len() - 1).style.clone()), Some(None), "{cmd_id}");
    }
}

/// Gap 19, review: text inserted by one author and deleted by another keeps both marks through
/// an ODT save, so Reject All (undo the deletion, then the insertion) and Accept All both remove it.
#[test]
fn odt_inserted_then_deleted_text_round_trips() {
    use std::io::Write;
    let ns = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/""#;
    let info = |who: &str| format!("<office:change-info><dc:creator>{who}</dc:creator><dc:date>2024-03-01T10:00:00</dc:date></office:change-info>");
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:tracked-changes><text:changed-region text:id="i1"><text:insertion>{}</text:insertion></text:changed-region><text:changed-region text:id="d1"><text:deletion>{}<text:p><text:change-start text:change-id="i1"/>gone<text:change-end text:change-id="i1"/></text:p></text:deletion></text:changed-region></text:tracked-changes><text:p>Keep <text:change text:change-id="d1"/>this.</text:p></office:text></office:body></office:document-content>"#,
        info("Alice"),
        info("Bob")
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    let doc = crate::io::open_bytes("t.odt", &z.finish().unwrap().into_inner()).unwrap();
    let doc = crate::io::open_bytes("t.odt", &wordcraft_formats::odt::export(&doc).unwrap()).unwrap();
    for cmd in ["review.rejectAll", "review.acceptAll"] {
        let mut s = Session::new(doc.clone());
        run(&mut s, cmd, json!({}));
        assert_eq!(text(&s), "Keep this.", "{cmd}");
    }
}

/// Gap 19: an ODT's tracked insertion and multi-paragraph deletion come in as revisions that
/// Reject All turns back into the original and Accept All into the edited text.
#[test]
fn odt_tracked_changes_accept_and_reject_all() {
    use std::io::Write;
    let ns = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/""#;
    let info = |who: &str| format!("<office:change-info><dc:creator>{who}</dc:creator><dc:date>2024-03-01T10:00:00</dc:date></office:change-info>");
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:tracked-changes><text:changed-region text:id="i1"><text:insertion>{}</text:insertion></text:changed-region><text:changed-region text:id="d1"><text:deletion>{}<text:p>tail</text:p><text:p>head<text:s/></text:p></text:deletion></text:changed-region></text:tracked-changes><text:p>First <text:change text:change-id="d1"/>rest <text:change-start text:change-id="i1"/>added<text:change-end text:change-id="i1"/>.</text:p><text:p>End.</text:p></office:text></office:body></office:document-content>"#,
        info("Alice"),
        info("Bob")
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    let doc = crate::io::open_bytes("t.odt", &bytes).unwrap();

    let mut sr = Session::new(doc.clone());
    run(&mut sr, "review.rejectAll", json!({}));
    assert_eq!(text(&sr), "First tail\nhead rest .\nEnd.");
    assert!(sr.doc.revisions.is_empty() || run(&mut sr, "review.changes", json!({})).as_array().unwrap().is_empty());

    let mut sa = Session::new(doc);
    run(&mut sa, "review.acceptAll", json!({}));
    assert_eq!(text(&sa), "First rest added.\nEnd.");
    assert!(run(&mut sa, "review.changes", json!({})).as_array().unwrap().is_empty());
}

/// Gap 19, review 2: a paragraph break deleted inside an insertion range is not itself
/// inserted, so Reject All keeps the two paragraphs (after an ODT save, too).
#[test]
fn odt_deleted_mark_inside_an_insertion_survives_reject_all() {
    use std::io::Write;
    let ns = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/""#;
    let info = |who: &str| format!("<office:change-info><dc:creator>{who}</dc:creator><dc:date>2024-03-01T10:00:00</dc:date></office:change-info>");
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:tracked-changes><text:changed-region text:id="i1"><text:insertion>{}</text:insertion></text:changed-region><text:changed-region text:id="d1"><text:deletion>{}<text:p/><text:p/></text:deletion></text:changed-region></text:tracked-changes><text:p>One <text:change-start text:change-id="i1"/>A<text:change text:change-id="d1"/>C<text:change-end text:change-id="i1"/> two</text:p></office:text></office:body></office:document-content>"#,
        info("Alice"),
        info("Bob")
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    let doc = crate::io::open_bytes("t.odt", &z.finish().unwrap().into_inner()).unwrap();
    let saved = crate::io::open_bytes("t.odt", &wordcraft_formats::odt::export(&doc).unwrap()).unwrap();
    for d in [doc, saved] {
        let mut s = Session::new(d);
        run(&mut s, "review.rejectAll", json!({}));
        assert_eq!(text(&s), "One \n two");
    }
}

/// #1320: `layout.verticalAlign` sets the section's vertical alignment, Justified included
/// ("justify", or "both"/"justified"); a value it doesn't know is Top, as before. Section JSON
/// for `layout.pageSetup` keeps its names.
#[test]
fn vertical_align_command_sets_justified() {
    use wordcraft_doc::section::PageVAlign;
    let mut s = s();
    let va = |s: &Session| crate::cmd::page::sect(s).valign;
    for (value, want) in [
        ("justify", PageVAlign::Justify),
        ("center", PageVAlign::Center),
        ("both", PageVAlign::Justify),
        ("bottom", PageVAlign::Bottom),
        ("justified", PageVAlign::Justify),
        ("top", PageVAlign::Top),
        ("sideways", PageVAlign::Top),
    ] {
        run(&mut s, "layout.verticalAlign", json!({"value": value}));
        assert_eq!(va(&s), want, "{value}");
    }
    run(&mut s, "layout.verticalAlign", json!({"value": "justify"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(va(&s), PageVAlign::Top, "one Undo step");
    let spec = s.registry.get("layout.verticalAlign").unwrap();
    assert!(spec.params.contains("justify"), "documents its values: {}", spec.params);
    // Section JSON: today's names still read, and Justified is "justify".
    let mut props = serde_json::to_value(crate::cmd::page::sect(&s)).unwrap();
    assert_eq!(props["valign"], json!("top"));
    for (name, want) in [("center", PageVAlign::Center), ("bottom", PageVAlign::Bottom), ("justify", PageVAlign::Justify), ("top", PageVAlign::Top)] {
        props["valign"] = json!(name);
        run(&mut s, "layout.pageSetup", json!({"section": props.clone()}));
        assert_eq!(va(&s), want, "{name}");
        assert_eq!(serde_json::to_value(crate::cmd::page::sect(&s)).unwrap()["valign"], json!(name));
    }
}

/// #1876 review: only Web Layout wraps to the window's `web_width`. Draft and Outline keep a fixed
/// 800 pt column, also after a visit to a wide Web Layout, and the window width is never serialized.
#[test]
fn draft_keeps_its_column_after_web_layout() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Draft text wraps in a fixed column. ".repeat(80)}));
    run(&mut s, "view.draft", json!({}));
    let before = s.layout();
    assert_eq!(before.pages[0].w, 800.0);
    run(&mut s, "view.webLayout", json!({}));
    s.view.web_width = 1400.0;
    assert_eq!(s.layout().pages[0].w, 1400.0, "Web Layout follows the window");
    run(&mut s, "view.draft", json!({}));
    assert_eq!(s.layout().pages[0].w, 800.0, "Draft never sees Web Layout's window width");
    run(&mut s, "view.outline", json!({}));
    assert_eq!(s.layout().pages[0].w, 800.0, "nor does Outline");
    let serialized = serde_json::to_value(&s.view).unwrap();
    assert!(serialized.get("webWidth").is_none(), "a window width is not view state to keep");
    let restored: crate::ViewState = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored.web_width, 800.0);
}

/// Insert › Quick Parts and AutoText list, insert and save the document's own building blocks
/// (its glossary, kept in the file); saving with `"in": "document"` is undoable.
#[test]
fn quick_parts_use_the_documents_building_blocks() {
    use wordcraft_doc::{BuildingBlock, CharProps, Paragraph, para_block};
    let entry = |name: &str, gallery: &str, text: &str| BuildingBlock {
        name: name.into(),
        gallery: gallery.into(),
        category: "General".into(),
        blocks: vec![para_block(Paragraph::with_text(text, CharProps::default()))],
        ..Default::default()
    };
    let mut d = wordcraft_doc::Document::new();
    d.glossary = vec![entry("Signature", "docParts", "Kind regards, Ann"), entry("Disclaimer", "autoTxt", "No warranty.")];
    let bytes = wordcraft_docx::write(&d).unwrap();
    let mut s = Session::new(crate::io::open_bytes("glossary.docx", &bytes).unwrap());
    let listed = run(&mut s, "insert.quickParts", json!({}));
    assert_eq!(listed["document"], json!(["Signature"]), "{listed}");
    assert_eq!(listed["parts"], json!(["Signature"]), "{listed}");
    assert_eq!(run(&mut s, "insert.autoText", json!({}))["document"], json!(["Disclaimer"]));
    run(&mut s, "insert.quickParts", json!({"insert": "Signature"}));
    assert!(text(&s).contains("Kind regards, Ann"), "{}", text(&s));
    run(&mut s, "insert.autoText", json!({"insert": "Disclaimer"}));
    assert!(text(&s).contains("No warranty."), "{}", text(&s));

    // Saved into the document: undoable, and kept in its file.
    run(&mut s, "select.text", json!({"text": "Kind regards"}));
    s.dirty = false;
    let listed = run(&mut s, "insert.quickParts", json!({"save": "Closing", "in": "document"}));
    assert_eq!(listed["document"], json!(["Signature", "Closing"]), "{listed}");
    assert!(s.building_blocks.is_empty(), "not in the session's store");
    assert!(s.dirty);
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc.glossary.iter().any(|e| e.name == "Closing"));
    run(&mut s, "edit.redo", json!({}));
    let back = crate::io::open_bytes("glossary.docx", &wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let closing = back.glossary.iter().find(|e| e.name == "Closing").expect("saved entry");
    assert_eq!(closing.gallery, "docParts");
    assert_eq!(closing.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.as_str()).collect::<String>(), "Kind regards");
    assert_eq!(back.glossary.len(), 3);

    // Without "in", saving stays in the session; deleting removes the name where it lives.
    run(&mut s, "insert.quickParts", json!({"save": "Scratch"}));
    assert!(s.building_blocks.contains_key("Scratch") && !s.doc.glossary.iter().any(|e| e.name == "Scratch"));
    run(&mut s, "insert.quickParts", json!({"delete": "Scratch"}));
    run(&mut s, "insert.quickParts", json!({"delete": "Closing"}));
    assert!(s.building_blocks.is_empty());
    assert_eq!(s.doc.glossary.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Signature", "Disclaimer"]);
    assert!(s.run("insert.quickParts", &json!({"save": " ", "in": "document"})).is_err());
    assert!(s.run("insert.quickParts", &json!({"insert": "Closing"})).is_err());
}

#[test]
fn toc_collects_cell_headings() {
    use std::sync::Arc;
    use wordcraft_doc::{Block, CharProps, InlineObject, Paragraph, Path, Table, para_block};
    let mut earlier = Table::new(1, 1, 180.0);
    earlier.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("Earlier", CharProps::default()).styled("Heading1"))];
    let mut nested = Table::new(1, 1, 180.0);
    nested.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("Nested", CharProps::default()).styled("Heading3"))];
    let mut later = Paragraph::with_text("Later", CharProps::default());
    later.props.outline_level = Some(1);
    let mut table = Table::new(1, 1, 180.0);
    table.rows[0].cells[0].blocks =
        vec![para_block(later), Arc::new(Block::Table(nested)), para_block(Paragraph::with_text("Unlisted", CharProps::default()))];
    let mut toc = Paragraph::with_text("Contents", CharProps::default()).styled("TOCHeading");
    toc.insert_object(
        toc.len(),
        InlineObject::Field {
            instr: r#"TOC \o "1-3" \h \u"#.into(),
            result: String::new(),
            locked: false,
            dirty: true,
            code: false,
            form: String::new(),
        },
        &CharProps::default(),
    )
    .unwrap();
    let mut d = wordcraft_doc::Document::new();
    d.body = vec![
        Arc::new(Block::Table(earlier)),
        para_block(toc),
        para_block(Paragraph::with_text("\u{000C}", CharProps::default())),
        Arc::new(Block::Table(table)),
    ];
    let mut s = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap());
    for command in ["references.updateFields", "references.updateToc"] {
        run(&mut s, command, json!({}));
        let entries: Vec<_> = s
            .doc
            .body
            .iter()
            .filter_map(|b| b.as_para())
            .filter(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
            .cloned()
            .collect();
        assert_eq!(entries.len(), 3, "{command}: {}", text(&s));
        let layout = s.layout();
        let bookmarks = s.doc.bookmarks();
        let mut pages = Vec::new();
        for (p, title) in entries.iter().zip(["Earlier", "Later", "Nested"]) {
            let value = p.plain_text();
            assert!(value.starts_with(title), "{value}");
            let shown: u32 = value.rsplit_once('\t').unwrap().1.parse().unwrap();
            let name = p.props_of_char(0).link.as_ref().unwrap().strip_prefix('#').unwrap();
            let (_, target) = bookmarks.iter().find(|(n, _)| n == name).unwrap();
            assert!(target.path.depth() > 0);
            let paragraph = s.doc.para(StoryRef::Body, &target.path).unwrap();
            assert_eq!(paragraph.plain_text(), title);
            let caret = layout.caret(target).unwrap();
            assert_eq!(shown, layout.pages[caret.page].number, "{title}");
            pages.push(caret.page);
        }
        assert!(pages[1] > pages[0]);
        assert_eq!(bookmarks.iter().filter(|(n, _)| n.starts_with("_Toc")).count(), 3);
        // The block-index-only link API must not misidentify a cell paragraph as a body block.
        let targets = s.run("link.targets", &json!({})).unwrap();
        assert_eq!(targets["headings"].as_array().unwrap().len(), 0);
        assert!(s.doc.para(StoryRef::Body, &Path::top(1)).is_some());
    }
}

#[test]
fn toc_keeps_direct_number_tabs() {
    use wordcraft_doc::props::{CharProps, NumRef, TabAlign, TabLeader, TabStop};
    let left = TabStop { pos: 24.0, align: TabAlign::Left, leader: TabLeader::None };
    let right = TabStop { pos: 179.5, align: TabAlign::Right, leader: TabLeader::Dot };
    let mut d = wordcraft_doc::Document::from_text("Contents\n\n1.\tAlpha\t9\nAlpha\nBeta");
    d.last_section.page_w = 220.0;
    d.last_section.margin_left = 20.0;
    d.last_section.margin_right = 20.0;
    d.styles.get_mut("TOC2").unwrap().para.tabs = Some(vec![right]);
    let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    {
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        p.props.style = Some("TOCHeading".into());
        p.insert_object(
            p.len(),
            wordcraft_doc::InlineObject::Field {
                instr: r#"TOC \o "1-2""#.into(),
                result: String::new(),
                locked: false,
                dirty: true,
                code: false,
                form: String::new(),
            },
            &CharProps::default(),
        )
        .unwrap();
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap();
        p.props.style = Some("TOC2".into());
        p.props.tabs = Some(vec![right]);
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(2)).unwrap();
        p.props.style = Some("TOC2".into());
        p.props.tabs = Some(vec![left, right]);
    }
    for i in [3, 4] {
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(i)).unwrap();
        p.props.style = Some("Heading2".into());
        p.props.numbering = Some(NumRef { num, level: 0 });
    }
    let bytes = wordcraft_docx::write(&d).unwrap();
    let mut s = Session::new(wordcraft_docx::read(&bytes).unwrap());
    for command in ["references.updateFields", "references.updateToc", "references.toc"] {
        run(&mut s, command, json!({}));
        let layout = s.layout();
        let mut entries = 0;
        for (i, b) in s.doc.body.iter().enumerate() {
            let Some(p) = b.as_para().filter(|p| p.props.style.as_deref() == Some("TOC2")) else { continue };
            entries += 1;
            assert!(p.props.tabs.as_ref().is_some_and(|tabs| tabs.contains(&left)), "{command}: {:?}", p.props.tabs);
            let title = p.text.find('\t').unwrap() + 1;
            let page = p.text.rfind('\t').unwrap() + 1;
            let start = layout.caret(&Pos::body(i, title)).unwrap();
            let end = layout.caret(&Pos::body(i, page)).unwrap();
            assert_eq!((start.page, start.top), (end.page, end.top), "{command}: {:?}", p.plain_text());
            assert!((start.x - 44.0).abs() < 1.0, "{command}: {}", start.x);
            assert!(end.x > 180.0, "{command}: {}", end.x);
        }
        assert_eq!(entries, 2);
    }
}

/// #2601: Insert › Drop Cap's menu and Drop Cap Options. Dropped and In margin take the
/// documented defaults (3 lines, distance 0, the text's font); the dialog's values (lines,
/// distance, font) are kept and clamped; None takes the drop cap away with its font; each is one
/// undo step and the caret stays where it was. The command is off where Word disables it: an
/// empty paragraph, a table cell, a paragraph that starts with a space.
#[test]
fn drop_cap_options_set_position_lines_distance_and_font() {
    let props = |s: &Session| s.doc.body.first().and_then(|b| b.as_para()).map(|p| p.props.clone()).unwrap_or_default();
    let cap = |s: &Session| {
        let p = props(s);
        (p.drop_cap, p.drop_cap_margin, p.drop_cap_distance, p.drop_cap_font)
    };
    let mut s = s();
    assert!(s.run("insert.dropCap", &json!({})).is_err(), "off in an empty paragraph");
    run(&mut s, "text.insert", json!({"text": "Once upon a time"}));
    let caret = s.sel.clone();
    run(&mut s, "insert.dropCap", json!({"position": "margin"}));
    assert_eq!(cap(&s), (Some(3), Some(true), None, None));
    assert_eq!(s.sel, caret, "the caret stays");
    run(&mut s, "insert.dropCap", json!({"position": "dropped", "lines": 5, "distance": 7.2, "font": "Liberation Mono"}));
    assert_eq!(cap(&s), (Some(5), None, Some(7.2), Some("Liberation Mono".to_string())));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(cap(&s), (Some(3), Some(true), None, None), "one undo step");
    run(&mut s, "insert.dropCap", json!({"lines": 99, "distance": 1e30, "font": "  "}));
    assert_eq!(cap(&s), (Some(10), None, Some(wordcraft_doc::resolve::MAX_DROP_CAP_DISTANCE), None), "clamped");
    run(&mut s, "insert.dropCap", json!({"lines": -4, "distance": -3, "position": "sideways"}));
    assert_eq!(cap(&s), (Some(1), None, None, None), "junk falls back to the nearest valid value");
    run(&mut s, "insert.dropCap", json!({"font": "Liberation Mono", "distance": 3}));
    run(&mut s, "insert.dropCap", json!({"position": "none"}));
    assert_eq!(cap(&s), (None, None, None, None), "None takes the letter's font back too");
    assert!(props(&s).is_empty());
    run(&mut s, "insert.dropCap", json!({"lines": 2}));
    run(&mut s, "insert.dropCap", json!({"lines": 0}));
    assert_eq!(cap(&s), (None, None, None, None), "0 lines is None, as before");

    let mut s = self::s();
    run(&mut s, "text.insert", json!({"text": " leading space"}));
    assert!(s.run("insert.dropCap", &json!({})).is_err(), "off when the paragraph starts with a space");
    let mut s = self::s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    run(&mut s, "text.insert", json!({"text": "Cell"}));
    assert!(s.sel.focus.path.cell().is_some());
    assert!(s.run("insert.dropCap", &json!({})).is_err(), "off in a table cell");
}

#[test]
fn rtf_list_round_trip_keeps_text() {
    for cmd in ["para.bullets", "para.numbering", "para.multilevel", "para.defineBullet"] {
        let mut s = Session::new(wordcraft_doc::Document::from_text("first\nsecond"));
        run(&mut s, "select.all", json!({}));
        run(&mut s, cmd, json!({}));
        let bytes = crate::io::save_bytes("x.rtf", &s.doc).unwrap();
        let d = crate::io::open_bytes("x.rtf", &bytes).unwrap();
        assert_eq!(d.plain_text(StoryRef::Body), "first\nsecond", "{cmd}");
    }
}

/// #3490: `table.cellMargins` preserves existing margins for any sides not passed.
#[test]
fn table_cell_margins_preserves_unpassed_sides() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    let (tp, _, _) = s.sel.focus.path.cell().unwrap();

    // 1. Updating 'top' only retains default left (5.4), bottom (0.0), and right (5.4).
    run(&mut s, "table.cellMargins", json!({"top": 20.0}));
    let margins = s.doc.table(StoryRef::Body, &tp).unwrap().props.cell_margins.unwrap();
    assert_eq!(margins, [20.0, 5.4, 0.0, 5.4]);

    // 2. Updating 'left' subsequently retains previously set top (20.0), bottom (0.0), and right (5.4).
    run(&mut s, "table.cellMargins", json!({"left": 10.0}));
    let margins = s.doc.table(StoryRef::Body, &tp).unwrap().props.cell_margins.unwrap();
    assert_eq!(margins, [20.0, 10.0, 0.0, 5.4]);

    // 3. Updating 'bottom' retains top (20.0), left (10.0), and right (5.4).
    run(&mut s, "table.cellMargins", json!({"bottom": 8.0}));
    let margins = s.doc.table(StoryRef::Body, &tp).unwrap().props.cell_margins.unwrap();
    assert_eq!(margins, [20.0, 10.0, 8.0, 5.4]);

    // 4. Updating 'right' retains top (20.0), left (10.0), and bottom (8.0).
    run(&mut s, "table.cellMargins", json!({"right": 15.0}));
    let margins = s.doc.table(StoryRef::Body, &tp).unwrap().props.cell_margins.unwrap();
    assert_eq!(margins, [20.0, 10.0, 8.0, 15.0]);

    // 5. Updating multiple sides at once retains unpassed sides.
    run(&mut s, "table.cellMargins", json!({"top": 0.0, "bottom": 0.0}));
    let margins = s.doc.table(StoryRef::Body, &tp).unwrap().props.cell_margins.unwrap();
    assert_eq!(margins, [0.0, 10.0, 0.0, 15.0]);
}

#[test]
fn headless_ui_requests_stay_bounded() {
    let mut s = s();
    let big = "x".repeat(1 << 20);
    for _ in 0..200 {
        run(&mut s, "edit.pasteSpecial", json!({"text": big}));
        run(&mut s, "edit.goto", json!({}));
    }
    assert!(s.ui_requests.len() <= 64, "len = {}", s.ui_requests.len());
    let size: usize = s.ui_requests.iter().map(|r| r.to_string().len()).sum();
    assert!(size < 4 << 20, "queued {size} bytes");
}

/// `file.exportPng` writes only to `.png` paths, so automation can't replace other files with
/// page images (#3696).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_png_refuses_a_path_that_is_not_png() {
    let dir = scratch_dir("export-png-extension");
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    for name in ["notes.txt", "config", "report.docx", "page.png.txt"] {
        let target = dir.join(name);
        std::fs::write(&target, b"precious user notes").unwrap();
        let r = s.run("file.exportPng", &json!({"path": target.to_string_lossy()}));
        assert!(matches!(r, Err(crate::CmdError::Params(_))), "{name}: {r:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"precious user notes", "{name} was changed");
    }
    // A .png path still works, whatever its case.
    for name in ["page.png", "PAGE.PNG"] {
        let target = dir.join(name);
        s.run("file.exportPng", &json!({"path": target.to_string_lossy()})).unwrap();
        assert!(std::fs::read(&target).unwrap().starts_with(b"\x89PNG"), "{name} is not a PNG");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The body's IME replacement epoch advances when the whole body is swapped (setText, Undo/Redo)
/// and stays put for ordinary edits; an equation editing session cannot ride across a replacement.
#[test]
fn ime_body_epoch_marks_body_replacements_not_edits() {
    let mut s = s();
    let id = s.document_id();
    run(&mut s, "text.insert", json!({"text": "one"}));
    let first = s.ime_body_epoch();
    run(&mut s, "text.insert", json!({"text": " two"}));
    assert_eq!(s.ime_body_epoch(), first, "typing is not a replacement");

    run(&mut s, "document.setText", json!({"text": "same text"}));
    let replaced = s.ime_body_epoch();
    assert!(replaced > first, "setText replaces the body under the same document id");
    assert_eq!(s.document_id(), id, "the document id itself is unchanged");

    // The same text again still replaces the body: a composition must not survive on content
    // equality.
    run(&mut s, "document.setText", json!({"text": "same text"}));
    assert!(s.ime_body_epoch() > replaced);

    // Undo swaps the body back.
    let before_undo = s.ime_body_epoch();
    assert!(s.undo());
    assert!(s.ime_body_epoch() > before_undo, "Undo replaces the body");
}

/// Replacing the document body while an equation is being edited ends that equation editing
/// session.
#[test]
fn a_body_replacement_ends_equation_editing() {
    let mut s = s();
    run(&mut s, "insert.equation", json!({"linear": "x+y"}));
    assert!(s.math.is_some());
    run(&mut s, "document.setText", json!({"text": "x+y"}));
    assert!(s.math.is_none(), "the old equation's editing session is gone");
}

/// A failed command that cleared the equation session puts it back with the document.
#[test]
fn a_failed_replacement_keeps_equation_editing() {
    let mut s = s();
    run(&mut s, "insert.equation", json!({"linear": "x+y"}));
    let session = s.math.as_ref().map(|m| m.session);
    assert!(session.is_some());
    // Missing `text` is rejected before any mutation.
    assert!(s.run("document.setText", &json!({})).is_err());
    assert_eq!(s.math.as_ref().map(|m| m.session), session);
}

/// Track Changes: deleting a range whose text is already marked deleted is a real no-op. It
/// creates no revision record, no undo-visible change and no IME revision.
#[test]
fn deleting_an_already_deleted_tracked_range_is_a_classified_noop() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "abc"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 1) };
    run(&mut s, "text.backspace", json!({}));
    let revisions = s.doc.revisions.len();
    let revision = s.ime_state_revision().expect("an IME revision");
    s.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 1) };
    run(&mut s, "text.backspace", json!({}));
    let outcome = s.ime_edit_outcome();
    assert!(outcome.classified, "{outcome:?}");
    assert!(!outcome.content_changed && !outcome.state_changed, "{outcome:?}");
    assert_eq!(s.doc.revisions.len(), revisions, "no unused Delete revision is appended");
    assert_eq!(s.ime_state_revision(), Some(revision), "the no-op did not move the settled caret");
    s.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 1) };
    assert!(s.ime_state_revision() > Some(revision), "re-selecting moves the caret, not the document");
}

/// `text.insert` classifies the automatic rewrites its own handler performs: smart quotes,
/// list autoformat and AutoCorrect are `transformed`, not the IME's literal insert.
#[test]
fn text_insert_marks_automatic_rewrites_as_transformed() {
    let mut session = s();
    run(&mut session, "text.insert", json!({"text": "\""}));
    assert!(session.ime_edit_outcome().transformed, "smart quote: {:?}", session.ime_edit_outcome());
    assert_eq!(text(&session), "\u{201C}");

    let mut session = s();
    for c in ["W", "o", "r", "d", " ", "(", "c", ")", " "] {
        run(&mut session, "text.insert", json!({"text": c}));
    }
    assert!(session.ime_edit_outcome().transformed, "AutoCorrect: {:?}", session.ime_edit_outcome());

    let mut session = s();
    run(&mut session, "text.insert", json!({"text": "*"}));
    run(&mut session, "text.insert", json!({"text": " "}));
    assert!(session.ime_edit_outcome().transformed, "list AutoFormat: {:?}", session.ime_edit_outcome());

    let mut session = s();
    run(&mut session, "text.insert", json!({"text": "A--b"}));
    run(&mut session, "text.insert", json!({"text": " "}));
    assert!(session.ime_edit_outcome().transformed, "dash AutoFormat: {:?}", session.ime_edit_outcome());
    assert_eq!(text(&session), "A\u{2014}b ");

    let mut session = s();
    run(&mut session, "tools.autocorrect", json!({"doubleSpacePeriod": true}));
    for c in "Hello  ".chars() {
        run(&mut session, "text.insert", json!({"text": c.to_string()}));
    }
    let outcome = session.ime_edit_outcome();
    assert!(outcome.transformed && outcome.content_changed && outcome.state_changed, "double-space period: {outcome:?}");
    assert_eq!(text(&session), "Hello. ");
    run(&mut session, "edit.undo", json!({}));
    assert_eq!(text(&session), "Hello  ", "undo restores the spaces in one step");
}

#[test]
fn style_set_keeps_imported_styles_latent_styles_and_defaults() {
    use wordcraft_doc::props::{Rgb, TextColor};
    use wordcraft_doc::styles::{Style, StyleKind};
    for name in ["basic", "lines", "shaded", "casual", "centered", "minimalist", "title"] {
        let mut s = s();
        let mut body = s.doc.styles.get("Normal").unwrap().clone();
        body.id = "BodyText".into();
        body.name = "Body Text".into();
        body.builtin = true;
        body.based_on = Some("Normal".into());
        body.chr.size = Some(20.0);
        body.chr.color = Some(TextColor::Rgb(Rgb(0xFF, 0, 0)));
        let grid = Style { id: "GridTable4".into(), name: "Grid Table 4".into(), kind: StyleKind::Table, builtin: true, ..Default::default() };
        s.doc.styles.styles.push(body.clone());
        s.doc.styles.styles.push(grid.clone());
        s.doc.styles.latent_styles = Some("<w:latentStyles/>".into());
        s.doc.styles.default_chr.lang = Some("de-DE".into());
        s.doc.styles.default_chr.size = Some(11.0);
        run(&mut s, "design.styleSet", json!({ "name": name }));
        assert_eq!(s.doc.styles.get("BodyText"), Some(&body), "{name}: BodyText");
        assert_eq!(s.doc.styles.get("GridTable4"), Some(&grid), "{name}: GridTable4");
        assert_eq!(s.doc.styles.latent_styles.as_deref(), Some("<w:latentStyles/>"), "{name}");
        assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("de-DE"), "{name}");
        assert_eq!(s.doc.styles.default_chr.size, Some(11.0), "{name}");
        assert_eq!(s.doc.styles.styles.iter().filter(|x| x.id == "Heading1").count(), 1, "{name}");
        let saved = crate::io::save_bytes("x.docx", &s.doc).unwrap();
        let back = crate::io::open_bytes("x.docx", &saved).unwrap();
        assert!(back.styles.get("BodyText").is_some(), "{name}: BodyText after round trip");
    }
    let mut s = s();
    run(&mut s, "design.styleSet", json!({ "name": "title" }));
    assert_eq!(s.doc.styles.get("Title").unwrap().chr.size, Some(36.0));
}

fn style_size(s: &Session, id: &str) -> f32 {
    s.doc.styles.resolve_char(Some(id), &Default::default()).size
}

#[test]
fn scale_font_sizes_adds_points_to_every_style() {
    let mut s = Session::new(crate::sample::sample_document());
    let ids: Vec<String> = s.doc.styles.styles.iter().map(|x| x.id.clone()).collect();
    let before: Vec<f32> = ids.iter().map(|i| style_size(&s, i)).collect();
    let base = s.doc.styles.resolve_char(None, &Default::default()).size;
    let runs = serde_json::to_string(&s.doc.body).unwrap();
    run(&mut s, "design.scaleFontSizes", json!({"delta": 2}));
    // Every style (explicit or inherited) grows by exactly 2, once.
    for (i, b) in ids.iter().zip(&before) {
        assert_eq!(style_size(&s, i), b + 2.0, "{i}");
    }
    assert_eq!(s.doc.styles.resolve_char(None, &Default::default()).size, base + 2.0);
    assert_eq!(serde_json::to_string(&s.doc.body).unwrap(), runs, "runs untouched");
    run(&mut s, "edit.undo", json!({}));
    for (i, b) in ids.iter().zip(&before) {
        assert_eq!(style_size(&s, i), *b, "undo {i}");
    }
}

#[test]
fn scale_font_sizes_percent_and_clamp_and_errors() {
    let mut s = Session::new(crate::sample::sample_document());
    let n = style_size(&s, "Normal");
    run(&mut s, "design.scaleFontSizes", json!({"percent": 10}));
    assert_eq!(style_size(&s, "Normal"), (n * 1.1 * 2.0).round() / 2.0);
    run(&mut s, "design.scaleFontSizes", json!({"delta": -500}));
    assert_eq!(style_size(&s, "Normal"), 1.0);
    for bad in [
        json!({}),
        json!({"delta": 1e9}),
        json!({"percent": -100}),
        json!({"percent": 1e9}),
        json!({"delta": "x"}),
        json!({"delta": 0}),
        json!({"percent": 0}),
        json!({"delta": 1, "percent": 10}),
        json!({"delta": 2, "percent": "x"}),
        json!({"percent": 10, "delta": null}),
    ] {
        assert!(s.run("design.scaleFontSizes", &bad).is_err(), "{bad}");
    }
}

#[test]
fn scale_font_sizes_reaches_char_table_and_cs_sizes_and_unset_default() {
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Style, StyleKind, styles::TablePart};
    let mut s = s();
    s.doc.styles.default_chr.size = None;
    s.doc.styles.default_chr.size_cs = Some(10.0);
    let mut ch = Style { id: "Big".into(), name: "Big".into(), kind: StyleKind::Character, ..Default::default() };
    ch.chr.size = Some(20.0);
    ch.chr.size_cs = Some(22.0);
    let mut tb = Style { id: "Grid".into(), name: "Grid".into(), kind: StyleKind::Table, ..Default::default() };
    let mut parts = wordcraft_doc::styles::TableStyleParts::default();
    parts.header_chr.size = Some(14.0);
    parts.regions.entry(TablePart::Band2Horz).or_default().chr.size = Some(8.0);
    tb.table = Some(parts);
    s.doc.styles.styles.extend([ch, tb]);
    run(&mut s, "design.scaleFontSizes", json!({"delta": 2}));
    let st = &s.doc.styles;
    assert_eq!(st.default_chr.size, Some(13.0), "unset default is 11 pt, made explicit");
    assert_eq!(st.default_chr.size_cs, Some(12.0));
    let big = CharProps { style: Some("Big".into()), ..Default::default() };
    assert_eq!(st.resolve_char(None, &big).size, 22.0);
    assert_eq!(st.resolve_char(None, &big).size_cs, 24.0);
    let grid = st.styles.iter().find(|x| x.id == "Grid").and_then(|x| x.table.as_ref()).unwrap();
    assert_eq!(grid.header_chr.size, Some(16.0));
    assert_eq!(grid.regions.get(&TablePart::Band2Horz).unwrap().chr.size, Some(10.0));
}

/// #3676: a 1-based page parameter becomes a 0-based index without wrapping.
/// On 32-bit and wasm the old cast truncated 2^32 to 0 and `0 - 1` panicked.
#[test]
fn page_index_saturates_instead_of_wrapping_to_page_one() {
    assert_eq!(crate::p::page_index(0), 0);
    assert_eq!(crate::p::page_index(1), 0);
    assert_eq!(crate::p::page_index(2), 1);
    assert_ne!(crate::p::page_index(1u64 << 32), 0, "2^32 must not wrap to page 1");
    assert_eq!(crate::p::page_index(u64::MAX), usize::MAX - 1);
}

/// #3676: absolute page numbers that cannot be an index are rejected, not silently
/// turned into page 1. `edit.goto` clamps to the last page, as it does past the end.
#[test]
fn hostile_absolute_pages_never_land_on_page_one() {
    let mut s = s();
    for i in 0..5 {
        if i > 0 {
            run(&mut s, "insert.pageBreak", json!({}));
        }
        run(&mut s, "text.insert", json!({"text": format!("Page {}", i + 1)}));
    }
    let page = |s: &mut Session| s.layout().caret_on(&s.sel.focus, s.page_hint).unwrap().page + 1;
    run(&mut s, "edit.goto", json!({"page": 1u64 << 32}));
    assert_eq!(page(&mut s), 5, "an out-of-range page lands on the last page");
    run(&mut s, "edit.goto", json!({"page": u64::MAX}));
    assert_eq!(page(&mut s), 5, "u64::MAX lands on the last page");
    run(&mut s, "edit.goto", json!({"page": 0}));
    assert_eq!(page(&mut s), 1, "page 0 is the first page");
}

/// #3676: `caret.set` takes a 0-based page; an oversized page is rejected instead of
/// truncating to page 0 on 32-bit and wasm.
#[test]
fn caret_set_rejects_a_page_number_that_does_not_fit() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let r = s.run("caret.set", &json!({"page": 1u64 << 32, "x": 100.0, "y": 100.0}));
    assert!(matches!(r, Err(crate::CmdError::Params(_))), "page 2^32: {r:?}");
}

/// #3676: `file.exportPng` rejects a page number that cannot be an index instead of
/// exporting page 1 (the 32-bit truncation) and writing it to the requested path.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_png_rejects_a_page_number_that_does_not_fit() {
    let dir = scratch_dir("export-png-hostile-page");
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let target = dir.join("page.png");
    let r = s.run("file.exportPng", &json!({"path": target.to_string_lossy(), "page": 1u64 << 32}));
    assert!(matches!(r, Err(crate::CmdError::Params(_))), "page 2^32: {r:?}");
    assert!(!target.exists(), "no file is written for an invalid page");
    let r = s.run("file.exportPng", &json!({"path": target.to_string_lossy(), "page": u64::MAX}));
    assert!(matches!(r, Err(crate::CmdError::Params(_))), "page u64::MAX: {r:?}");
    assert!(!target.exists(), "no file is written for an invalid page");
    let _ = std::fs::remove_dir_all(&dir);
}
