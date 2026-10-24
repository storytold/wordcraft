import io, sys

p = 'crates/ui-egui/src/lib.rs'
s = io.open(p, encoding='utf-8', newline='').read()
eol = "\r\n" if "\r\n" in s else "\n"

# Fix the test to use app() and scratch() from the module (not crate::test_support)
s = s.replace("crate::test_support::scratch", "scratch")
s = s.replace("crate::test_support::app()", "app()")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print("fixed test_support references")