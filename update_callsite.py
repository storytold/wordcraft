import io, sys

p = 'crates/ui-egui/src/lib.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

OLD = '            None if self.from_a_macro_template() => format!("{}.docm", self.title_stem()),'
NEW = '            None if self.is_from_macro_template() => format!("{}.docm", self.title_stem()),'

o = OLD.replace("\n", eol); n = NEW.replace("\n", eol)
if s.count(o) != 1:
    sys.exit(f"the anchor count is {s.count(o)}")
s = s.replace(o, n, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("updated call site")