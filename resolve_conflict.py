import io, sys

p = 'crates/engine/src/cmd/tools.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

OLD = '''    #[test]
<<<<<<< Updated upstream
    fn protection_blocks_edits() {
        let mut s = Session::new(Document::from_text("locked"));
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        // The "none" mode is the shorthand for stopping the protection, and it doesn't stop a
        // protected document: only the explicit stop does (#1327 follow-up).
        assert!(s.run("review.restrict", &json!({"mode": "none"})).is_err());
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        s.run("review.restrict", &json!({"stop": true})).unwrap();
        assert!(s.run("text.insert", &json!({"text": "x"})).is_ok());
    }

    #[test]
=======
>>>>>>> Stashed changes
    fn accessibility_and_quick_parts() {'''

NEW = '''    #[test]
    fn protection_blocks_edits() {
        let mut s = Session::new(Document::from_text("locked"));
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        // The "none" mode is the shorthand for stopping the protection, and it doesn't stop a
        // protected document: only the explicit stop does (#1327 follow-up).
        assert!(s.run("review.restrict", &json!({"mode": "none"})).is_err());
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        s.run("review.restrict", &json!({"stop": true})).unwrap();
        assert!(s.run("text.insert", &json!({"text": "x"})).is_ok());
    }

    #[test]
    fn accessibility_and_quick_parts() {'''

o = OLD.replace("\n", eol); n = NEW.replace("\n", eol)
if s.count(o) != 1:
    sys.exit(f"the anchor count is {s.count(o)}")
s = s.replace(o, n, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("resolved conflict")