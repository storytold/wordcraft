import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# 1. apps/wordcraft/src/main.rs - non-Unicode template path fix (matches new upstream)
patch('apps/wordcraft/src/main.rs',
'''fn open_non_unicode(app: &mut WordApp, path: &std::path::Path) {
    let name = path.to_string_lossy();
    let opened = wordcraft_engine::io::read_document(path)
        .and_then(|bytes| app.run("file.open", serde_json::json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)})));
    match opened {
        // Not when it waits for a password: that open names the document itself.
        Ok(r) if r.get("pending").is_none() => app.session.path = Some(path.into()),
        Ok(_) => {}
        Err(e) => log::warn!("{name}: {e}"),
    }
}''',
'''fn open_non_unicode(app: &mut WordApp, path: &std::path::Path) {
    let name = path.to_string_lossy();
    // A template opens as a new document based on it, which has no path of its own (#3433): the
    // template is never what a later save writes to.
    let is_template = wordcraft_engine::io::is_template_name(&name);
    let opened = wordcraft_engine::io::read_document(path)
        .and_then(|bytes| app.run("file.open", serde_json::json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)})));
    match opened {
        // Not when it waits for a password: that open names the document itself.
        Ok(r) if r.get("pending").is_none() && !is_template => app.session.path = Some(path.into()),
        Ok(_) => {}
        Err(e) => log::warn!("{name}: {e}"),
    }
}''')

print("patched main.rs")