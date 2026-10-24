import io, sys

p = 'crates/engine/src/cmd/tools.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

OLD = '''    fn protection_blocks_edits() {''' + eol + '''        let mut s = Session::new(Document::from_text("locked"));''' + eol + '''        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();''' + eol + '''        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());''' + eol + '''        s.run("review.restrict", &json!({"mode": "none"})).unwrap();''' + eol + '''        assert!(s.run("text.insert", &json!({"text": "x"})).is_ok());''' + eol + '''    }''' + eol + eol + '''    #[test]''' + eol + '''    fn accessibility_and_quick_parts() {'''

NEW = '''    #[test]''' + eol + '''    fn accessibility_and_quick_parts() {'''

o = OLD; n = NEW
if s.count(o) != 1:
    sys.exit(f"the anchor count is {s.count(o)}")
s = s.replace(o, n, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("removed unused function")