//! Portable Quick Access Toolbar files (#5890), separate from ribbon and document settings.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{CmdError, CmdResult, Session, p};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolbarFile {
    format: String,
    version: u32,
    commands: Vec<String>,
}

/// File operations are exclusive, so an invalid import cannot partially apply other changes.
pub(super) fn transfer(s: &mut Session, v: &Value) -> CmdResult {
    let importing = v.get("qatImport").is_some();
    let key = if importing { "qatImport" } else { "qatExport" };
    if v.as_object().is_none_or(|o| o.len() != 1) {
        return Err(CmdError::Params("toolbar file operations cannot be combined with other changes".into()));
    }
    let path = p::req_str(v, key)?;
    if importing {
        import(s, path)?;
    } else {
        let file = ToolbarFile { format: "wordcraft-qat".into(), version: 1, commands: s.ribbon.qat().into_iter().map(str::to_string).collect() };
        let bytes = serde_json::to_vec_pretty(&file).map_err(|e| CmdError::Failed(e.to_string()))?;
        crate::io::write_file(path, &bytes).map_err(CmdError::Failed)?;
    }
    Ok(json!({"qat": s.ribbon.qat()}))
}

#[cfg(not(target_arch = "wasm32"))]
fn import(s: &mut Session, path: &str) -> Result<(), CmdError> {
    // A toolbar has at most 200 short IDs; cap the file before decoding untrusted JSON.
    let bytes = crate::io::read_file_capped(path, 1024 * 1024, "toolbar file").map_err(CmdError::Failed)?;
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err(CmdError::Params("toolbar file must be a JSON object".into()));
    }
    let file: ToolbarFile = serde_json::from_slice(&bytes).map_err(|e| CmdError::Params(format!("invalid toolbar file: {e}")))?;
    if file.format != "wordcraft-qat" || file.version != 1 {
        return Err(CmdError::Params("unsupported toolbar file format or version".into()));
    }
    // Use the toolbar's existing validation and capacity limit, but only replace it once all
    // entries pass. Unlike preference recovery, imports must not silently discard commands.
    let mut layout = crate::RibbonLayout { qat: Some(Vec::new()), ..Default::default() };
    for id in &file.commands {
        if id.len() > 128 || !layout.qat_add(&s.registry, id, None).map_err(CmdError::Params)? {
            return Err(CmdError::Params("toolbar command IDs must be unique and at most 128 bytes".into()));
        }
    }
    s.ribbon.qat = layout.qat;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn import(_s: &mut Session, _path: &str) -> Result<(), CmdError> {
    Err(CmdError::Failed("toolbar files are only available on the desktop".into()))
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::json;
    use wordcraft_doc::Document;

    #[test]
    fn qat_file_round_trips_defaults_custom_order_and_empty_toolbar() {
        let path = std::env::temp_dir().join(format!("wc-qat-roundtrip-{}.json", std::process::id()));
        for ids in [vec!["file.save", "edit.undo", "edit.redo"], vec!["format.bold", "file.save"], vec![]] {
            let mut source = Session::new(Document::new());
            source.run("tools.customizeRibbon", &json!({"layout": {"qat": ids}})).unwrap();
            source.run("tools.customizeRibbon", &json!({"qatExport": path})).unwrap();
            let bytes = std::fs::read(&path).expect("export must write a file");
            let file: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(file, json!({"format": "wordcraft-qat", "version": 1, "commands": ids}));
            let mut dest = Session::new(Document::from_text("Keep my document"));
            dest.run("tools.customizeRibbon", &json!({"newTab": {"name": "Mine"}, "hide": "Draw"})).unwrap();
            let tabs = dest.ribbon.tabs.clone();
            dest.run("tools.customizeRibbon", &json!({"qatImport": path})).unwrap();
            assert_eq!(dest.ribbon.qat(), ids);
            assert_eq!(dest.ribbon.tabs, tabs);
            assert_eq!(dest.doc.plain_text(wordcraft_doc::StoryRef::Body), "Keep my document");
            assert!(!dest.dirty);
            assert!(!dest.can_undo());
            assert!(dest.ui_requests.is_empty(), "programmatic import opens no dialog");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn qat_file_operations_respect_automation_roots() {
        use crate::file_access::{AutomationRoots, confine};
        let dir = std::env::temp_dir().join(format!("wc-qat-roots-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("toolbar.json");
        let read_only = AutomationRoots::new(Some(&dir), None).unwrap();
        let write_only = AutomationRoots::new(None, Some(&dir)).unwrap();
        let mut s = Session::new(Document::new());
        confine(read_only.as_ref(), || {
            assert!(s.run("tools.customizeRibbon", &json!({"qatExport": path})).is_err());
        });
        assert!(!path.exists());
        confine(write_only.as_ref(), || {
            s.run("tools.customizeRibbon", &json!({"qatExport": path})).unwrap();
            assert!(s.run("tools.customizeRibbon", &json!({"qatImport": path})).is_err());
        });
        confine(read_only.as_ref(), || {
            s.run("tools.customizeRibbon", &json!({"qatImport": path})).unwrap();
        });
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn qat_file_rejects_invalid_imports_without_mutation() {
        let path = std::env::temp_dir().join(format!("wc-qat-invalid-{}.json", std::process::id()));
        let mut s = Session::new(Document::new());
        s.run("tools.customizeRibbon", &json!({"qatAdd": "format.bold"})).unwrap();
        let before = s.ribbon.clone();
        for file in [
            json!({}),
            json!({"format": "other", "version": 1, "commands": []}),
            json!({"format": "wordcraft-qat", "version": 2, "commands": []}),
            json!({"format": "wordcraft-qat", "version": 1, "commands": ["gone.command"]}),
            json!({"format": "wordcraft-qat", "version": 1, "commands": ["edit.undo", "edit.undo"]}),
            json!({"format": "wordcraft-qat", "version": 1, "commands": [42]}),
            json!({"format": "wordcraft-qat", "version": 1, "commands": ["x".repeat(129)]}),
            json!({"format": "wordcraft-qat", "version": 1, "commands": [], "tabs": []}),
            json!({"format": "wordcraft-qat", "version": 1, "commands": vec!["edit.undo"; 201]}),
        ] {
            std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
            assert!(s.run("tools.customizeRibbon", &json!({"qatImport": path})).is_err(), "{file}");
            assert_eq!(s.ribbon, before);
        }
        for bytes in [b"not json".to_vec(), br#"["wordcraft-qat",1,[]]"#.to_vec(), vec![b' '; 1024 * 1024 + 1]] {
            std::fs::write(&path, bytes).unwrap();
            assert!(s.run("tools.customizeRibbon", &json!({"qatImport": path})).is_err());
            assert_eq!(s.ribbon, before);
        }
        for params in [
            json!({"qatImport": 3}),
            json!({"qatExport": false}),
            json!({"qatImport": path, "reset": true}),
            json!({"qatImport": path, "qatExport": path}),
        ] {
            assert!(s.run("tools.customizeRibbon", &params).is_err());
            assert_eq!(s.ribbon, before);
        }
        std::fs::remove_file(path).unwrap();
    }
}
