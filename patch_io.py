import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# crates/engine/src/io.rs - is_template_name (after is_word_package)
patch('crates/engine/src/io.rs',
'''pub fn is_word_package(ext: &str) -> bool {
    wordcraft_docx::Flavor::from_ext(ext).is_some()
}''',
'''pub fn is_word_package(ext: &str) -> bool {
    wordcraft_docx::Flavor::from_ext(ext).is_some()
}

/// Is `name` a Word template? A template opened from the shell starts a new document based on
/// it, as in Word (#3433); one opened from File > Open is the template itself.
///
/// The OOXML templates (.dotx/.dotm) are Word's own kinds; `.dot` is the Word 97-2003 one,
/// which Word also opens as a new document. `.doc` is not a template: it is a document.
pub fn is_template_name(name: &str) -> bool {
    match ext_of(name).as_str() {
        "dot" => true,
        ext => matches!(wordcraft_docx::Flavor::from_ext(ext), Some(wordcraft_docx::Flavor::Template | wordcraft_docx::Flavor::MacroTemplate)),
    }
}''')

print("io.rs patched")