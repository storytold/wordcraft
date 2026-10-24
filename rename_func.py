import io, sys

p = 'crates/ui-egui/src/lib.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

OLD = '''    /// Was this document made from a macro-enabled template? It carries the template's VBA
    /// project, which a .docx cannot hold.
    fn from_a_macro_template(&self) -> bool {
        self.session
            .doc
            .settings
            .attached_template
            .as_deref()
            .is_some_and(|t| std::path::Path::new(t).extension().is_some_and(|e| e.eq_ignore_ascii_case("dotm")))
    }'''

NEW = '''    /// Was this document made from a macro-enabled template? It carries the template's VBA
    /// project, which a .docx cannot hold.
    fn is_from_macro_template(&self) -> bool {
        self.session
            .doc
            .settings
            .attached_template
            .as_deref()
            .is_some_and(|t| std::path::Path::new(t).extension().is_some_and(|e| e.eq_ignore_ascii_case("dotm")))
    }'''

o = OLD.replace("\n", eol); n = NEW.replace("\n", eol)
if s.count(o) != 1:
    sys.exit(f"the anchor count is {s.count(o)}")
s = s.replace(o, n, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("renamed function")