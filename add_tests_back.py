import io, sys

p = 'crates/ui-egui/src/lib.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

# Add tests at the end of the tests module (before the final '}')
TESTS = '''
    /// #3433: a template the platform hands the app (a double-click in the file manager, the
    /// command line, a drop on the window) starts a NEW document based on it. The document has no
    /// path of its own, so Save asks where to write instead of overwriting the template.
    #[test]
    fn a_template_the_platform_hands_the_app_starts_a_new_document_and_the_template_is_never_written_over() {
        let dir = crate::test_support::scratch("template-launch");
        let path = dir.join("Letter.dotx");
        let mut s = Session::new(wordcraft_doc::Document::from_text("Template body.\\n"));
        s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        let before = std::fs::read(&path).unwrap();

        let mut app = crate::test_support::app();
        app.open_from_shell(&path.to_string_lossy(), None).unwrap();

        // The document is built from the template but is not the template: no path of its own.
        assert_eq!(app.session.path, None, "the new document must have no path to save over");
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Template body.\\n", "the template's content came along");
        assert_eq!(app.session.doc.settings.attached_template.as_deref(), Some(path.to_string_lossy().as_ref()), "the template is attached");

        // Save asks where to write, and the template on disk is byte for byte the same.
        let out = dir.join("Letter.docx");
        app.run("file.saveAs", json!({"path": out.to_string_lossy()})).unwrap();
        assert!(out.exists(), "the document was saved as its own file");
        assert_eq!(std::fs::read(&path).unwrap(), before, "the template on disk was not touched");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// A document the platform hands the app opens as itself: the same file, editable, and Save
    /// writes back to it (#3433).
    #[test]
    fn a_document_the_platform_hands_the_app_opens_as_itself() {
        let dir = crate::test_support::scratch("document-launch");
        let path = dir.join("Letter.docx");
        let mut s = Session::new(wordcraft_doc::Document::from_text("Hello.\\n"));
        s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();

        let mut app = crate::test_support::app();
        app.open_from_shell(&path.to_string_lossy(), None).unwrap();
        assert_eq!(app.session.path.as_deref(), Some(path.as_path()), "the document keeps its own path");
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello.\\n");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// #3433: a protected template the platform hands the app asks for its password, and the
    /// file is then opened with it as a NEW document -- not as the template itself, which is
    /// what the password dialog's retry would do if it always ran `file.open`.
    #[test]
    fn a_protected_template_the_platform_hands_the_app_asks_for_its_password_and_stays_a_new_document() {
        let dir = crate::test_support::scratch("protected-template-launch");
        let path = dir.join("Secret.dotx");
        let mut s = Session::new(wordcraft_doc::Document::from_text("Secret.\\n"));
        s.run("file.save", &json!({"path": path.to_string_lossy(), "password": "x"})).unwrap();

        let mut app = crate::test_support::app();
        let r = app.open_from_shell(&path.to_string_lossy(), None);
        assert!(r.is_ok() && r.unwrap().get("pending").is_some(), "password was asked");

        // The retry must be the command that asked (file.newFromTemplate), not file.open.
        let dialog = app.dialog.as_ref().expect("the password dialog");
        assert_eq!(dialog.name(), "password");
        let (command, params) = dialog.as_password().expect("password dialog");
        assert_eq!(command, "file.newFromTemplate", "the retry must be the command that asked");
        assert!(params.get("path").is_some(), "params carry the template path");

        // Enter the password; the document opens as a new document, not the template.
        app.execute(&command, params).unwrap();
        assert_eq!(app.session.path, None, "the new document has no path");
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Secret.\\n", "content came along");
        assert_eq!(app.session.doc.settings.attached_template.as_deref(), Some(path.to_string_lossy().as_ref()), "template attached");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// A new document made from a macro-enabled template carries the template's VBA project, so
    /// Save suggests a .docm: a .docx would leave the macros out.
    #[test]
    fn a_document_from_a_macro_template_is_saved_as_a_docm() {
        let mut doc = wordcraft_doc::Document::new();
        doc.passthrough.insert("word/vbaProject.bin".into(), std::sync::Arc::new(vec![0xD0, 0xCF, 0x11, 0xE0]));
        doc.settings.attached_template = Some("/templates/Letter.dotm".into());
        let a = WordApp::new(Session::new(doc), Default::default());
        assert_eq!(a.default_save_name(), "Document1.docm", "a .docm keeps the macros");
    }

    /// A document made from an ordinary template has no macros, so it is saved as a .docx.
    #[test]
    fn a_document_from_an_ordinary_template_is_saved_as_a_docx() {
        let mut doc = wordcraft_doc::Document::new();
        doc.settings.attached_template = Some("/templates/Letter.dotx".into());
        let a = WordApp::new(Session::new(doc), Default::default());
        assert_eq!(a.default_save_name(), "Document1.docx", "an ordinary template gives a .docx");
    }
}'''

# Find the last '}' of the tests module and insert before it
idx = s.rfind("}\n")
if idx == -1:
    sys.exit("could not find module end")
s = s[:idx] + TESTS.replace("\n", eol) + s[idx:]
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("tests added")