import io, sys

p = 'crates/ui-egui/src/lib.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

OLD = '''    /// #3433: a protected template the platform hands the app asks for its password, and the
    /// file is then opened with it as a NEW document -- not as the template itself, which is
    /// what the password dialog's retry would do if it always ran `file.open`.
    #[test]
    fn a_protected_template_the_platform_hands_the_app_asks_for_its_password_and_stays_a_new_document() {
        let dir = scratch("protected-template-launch");
        let path = dir.join("Secret.dotx");
        let mut s = Session::new(wordcraft_doc::Document::from_text("Secret.\\n"));
        s.run("file.save", &json!({"path": path.to_string_lossy(), "password": "x"})).unwrap();

        let mut app = app();
        let r = app.open_from_shell(&path.to_string_lossy(), None);
        assert!(r.is_ok() && r.unwrap().get("pending").is_some(), "password was asked");

        // The retry must be the command that asked (file.newFromTemplate), not file.open.
        let Some(dialogs::Dialog::Password { command, params, .. }) = app.dialog.as_ref().cloned() else {
            panic!("expected password dialog");
        };
        assert_eq!(app.dialog.as_ref().map(dialogs::Dialog::name), Some("password"), "dialog is password");
        assert_eq!(command, "file.newFromTemplate", "the retry must be the command that asked");
        assert!(params.get("path").is_some(), "params carry the template path");

        // Enter the password; the document opens as a new document, not the template.
        app.execute(&command, params).unwrap();
        assert_eq!(app.session.path, None, "the new document has no path");
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Secret.\\n", "content came along");
        assert_eq!(app.session.doc.settings.attached_template.as_deref(), Some(path.to_string_lossy().as_ref()), "template attached");

        let _ = std::fs::remove_dir_all(dir);
    }'''

NEW = '''    /// #3433: a protected template the platform hands the app asks for its password, and the
    /// file is then opened with it as a NEW document -- not as the template itself, which is
    /// what the password dialog's retry would do if it always ran `file.open`.
    #[test]
    fn a_protected_template_the_platform_hands_the_app_asks_for_its_password_and_stays_a_new_document() {
        let dir = scratch("protected-template-launch");
        let path = dir.join("Secret.dotx");
        let mut s = Session::new(wordcraft_doc::Document::from_text("Secret.\\n"));
        s.run("file.save", &json!({"path": path.to_string_lossy(), "password": "x"})).unwrap();

        let mut app = app();
        let r = app.open_from_shell(&path.to_string_lossy(), None);
        assert!(r.is_ok() && r.unwrap().get("pending").is_some(), "password was asked");

        // The retry must be the command that asked (file.newFromTemplate), not file.open.
        let Some(dialogs::Dialog::Password { command, params, .. }) = app.dialog.as_ref().cloned() else {
            panic!("expected password dialog");
        };
        assert_eq!(app.dialog.as_ref().map(dialogs::Dialog::name), Some("password"), "dialog is password");
        assert_eq!(command, "file.newFromTemplate", "the retry must be the command that asked");
        assert!(params.get("path").is_some(), "params carry the template path");

        // The dialog inserts the password into params when OK is clicked; simulate that.
        let mut p = params;
        p.as_object_mut().unwrap().insert("password".into(), json!("x"));
        app.execute(&command, p).unwrap();

        assert_eq!(app.session.path, None, "the new document has no path");
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Secret.\\n", "content came along");
        assert_eq!(app.session.doc.settings.attached_template.as_deref(), Some(path.to_string_lossy().as_ref()), "template attached");

        let _ = std::fs::remove_dir_all(dir);
    }'''

o = OLD.replace("\n", eol); n = NEW.replace("\n", eol)
if s.count(o) != 1:
    sys.exit(f"the anchor count is {s.count(o)}")
s = s.replace(o, n, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("fixed the test")