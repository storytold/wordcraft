import io, sys

p = 'crates/ui-egui/src/dialogs.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

OLD = '''                match app.execute(&command, p) {'''
NEW = '''                match app.execute(command, p) {'''

o = OLD.replace("\n", eol); n = NEW.replace("\n", eol)
if s.count(o) != 1:
    sys.exit(f"the anchor count is {s.count(o)}")
s = s.replace(o, n, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("fixed needless borrow")