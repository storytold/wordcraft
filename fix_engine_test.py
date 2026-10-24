import io, sys

p = 'crates/engine/src/cmd/tools.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

OLD = '''    #[test]
    /// #3433: a template the platform hands the app (command line, file manager, drop) starts a
    /// new document and leaves the template alone.
    #[test]
    fn a_template_opened_from_the_shell_starts_a_new_document_and_leaves_the_template_alone() {
        let dir = crate::test_support::scratch("template-launch");
        let path = dir.join("Letter.dotx");
        let mut s = Session::new(Document::from_text("Template body.\\n"));
        s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        let before = std::fs::read(&path).unwrap();

        let mut s = Session::new(Document::new());
        s.run("file.newFromTemplate", &json!({"path": path.to_string_lossy()})).unwrap();

        // The document is built from the template but is not the template: no path of its own.
        assert_eq!(s.path, None, "the new document must have no path to save over");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Template body.\\n", "the template's content came along");
        assert_eq!(s.doc.settings.attached_template.as_deref(), Some(path.to_string_lossy().as_ref()), "the template is attached");

        // Save asks where to write, and the template on disk is byte for byte the same.
        let out = dir.join("Letter.docx");
        s.run("file.saveAs", &json!({"path": out.to_string_lossy()})).unwrap();
        assert!(out.exists(), "the document was saved as its own file");
        assert_eq!(std::fs::read(&path).unwrap(), before, "the template on disk was not touched");

        let _ = std::fs::remove_dir_all(dir);
    }

    fn protection_blocks_edits() {'''

NEW = '''    #[test]
    /// #3433: a template the platform hands the app (command line, file manager, drop) starts a
    /// new document and leaves the template alone.
    #[test]
    fn a_template_opened_from_the_shell_starts_a_new_document_and_leaves_the_template_alone() {
        let dir = std::env::temp_dir().join(format!("wc-engine-template-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Letter.dotx");
        let mut s = Session::new(Document::from_text("Template body.\\n"));
        s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        let before = std::fs::read(&path).unwrap();

        let mut s = Session::new(Document::new());
        s.run("file.newFromTemplate", &json!({"path": path.to_string_lossy()})).unwrap();

        // The document is built from the template but is not the template: no path of its own.
        assert_eq!(s.path, None, "the new document must have no path to save over");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Template body.\\n", "the template's content came along");
        assert_eq!(s.doc.settings.attached_template.as_deref(), Some(path.to_string_lossy().as_ref()), "the template is attached");

        // Save asks where to write, and the template on disk is byte for byte the same.
        let out = dir.join("Letter.docx");
        s.run("file.saveAs", &json!({"path": out.to_string_lossy()})).unwrap();
        assert!(out.exists(), "the document was saved as its own file");
        assert_eq!(std::fs::read(&path).unwrap(), before, "the template on disk was not touched");

        let _ = std::fs::remove_dir_all(dir);
    }

    fn protection_blocks_edits() {'''

o = OLD.replace("\n", eol); n = NEW.replace("\n", eol)
if s.count(o) != 1:
    sys.exit(f"the anchor count is {s.count(o)}")
s = s.replace(o, n, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("engine test fixed")