import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# crates/engine/src/cmd/tools.rs - enhance file.newFromTemplate with data and password
patch('crates/engine/src/cmd/tools.rs',
'''        CommandSpec::new("file.newFromTemplate", "New from Template", "File › New", |s, v| {
            let path = p::req_str(v, "path")?;
            let mut doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
            // As in Word, the new document is attached to the template it came from (#2852).
            doc.settings.attached_template = Some(path.to_string());
            s.set_document(doc);
            s.path = None;
            s.dirty = true;
            sel_result(s)
        })
        .params(r#"{"path": string} -> a new, untitled document from the template, attached to it"#)
        .pure(),''',
'''        CommandSpec::new("file.newFromTemplate", "New from Template", "File › New", |s, v| {
            let path = p::req_str(v, "path")?;
            let password = p::str(v, "password");
            // The template's own styles and page setup are the document's, so it is opened like
            // a document and only the path is dropped.
            let doc = match p::str(v, "data") {
                Some(data) => {
                    let bytes = super::insert::base64_decode(data).ok_or_else(|| CmdError::Params("bad base64".into()))?;
                    crate::io::open_bytes_in(path, &bytes, password, &s.region()).map_err(CmdError::Failed)?.0
                }
                None => crate::io::open_path_with(std::path::Path::new(path), password).map_err(CmdError::Failed)?.0,
            };
            s.set_document(doc);
            // The template stays attached, as Word's does (#3433): the document is built from it,
            // and "Update Styles from Template" still finds it.
            s.doc.settings.attached_template = Some(path.to_string());
            s.path = None;
            s.dirty = true;
            sel_result(s)
        })
        .params(r#"{"path": string, "data"?: base64 (the template's bytes; `path` then only names it), "password"?: string (for a password-protected Word template)}"#)
        .pure(),''')

print("tools.rs patched")