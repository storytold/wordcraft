use super::actions::{customize_keyboard, customize_ribbon, hf_nav, language, macros, quick_parts, search_commands, select_similar};
use super::autocorrect::autocorrect_options;
use super::compare::{COMPARE_PARAMS, compare};
use super::inspect::{accessibility, inspect_doc};
use super::{
    Block, CharProps, CmdError, CommandSpec, InlineObject, NumRef, Paragraph, Selection, StoryRef, Value, delete_selection, json, p, para_block,
    sel_result,
};

/// Builds the tools, review, file and insertion command table: Repeat and macros; AutoCorrect,
/// keyboard and ribbon customization and search; Compare, accessibility, inspection, protection,
/// versions, recovery, templates and autosave; compatibility, sharing, Quick Parts, fields,
/// signatures, objects and spreadsheets; header/footer, table and numbering settings; language,
/// markup, editor and selection commands; and design and view options.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("edit.repeat", "Repeat", "Quick Access Toolbar", |s, _| {
            let Some((id, params)) = s.last_command.clone() else { return Err(CmdError::Failed("nothing to repeat".into())) };
            s.run(&id, &params)
        })
        .key("F4")
        .pure(),
        CommandSpec::new("tools.recordMacro", "Record Macro", "View › Macros", |s, v| match s.recording.take() {
            Some((name, steps)) => {
                let n = steps.len();
                s.macros.insert(name.clone(), steps);
                Ok(json!({"recording": false, "saved": name, "steps": n}))
            }
            None => {
                let name = p::str(v, "name").unwrap_or("Macro1").to_string();
                s.recording = Some((name.clone(), Vec::new()));
                Ok(json!({"recording": true, "name": name}))
            }
        })
        .params(r#"{"name"?: string} (call again to stop)"#)
        .pure(),
        CommandSpec::new("tools.macros", "Macros", "View › Macros", macros)
            .key("Alt+F8")
            .params(r#"{"run"?: name, "define"?: {"name": string, "steps": [{"command", "params"}]}, "delete"?: name}"#)
            .pure(),
        CommandSpec::new("tools.autocorrect", "AutoCorrect Options", "File › Options › Proofing", autocorrect_options)
            .params(
                r#"{"enabled"?: bool (all of AutoCorrect), "replaceText"|"twoInitialCaps"|"capSentences"|"capCells"|"capDays"|"capsLock"|"smartQuotes"|"fractions"|"ordinals"|"dashes"|"links"|"bullets"|"numbering"|"borderLines"?: bool, "boldItalic"?: bool (*bold* and _italic_ with real formatting; default false), "tabIndent"?: bool (Set left- and first-indent with tabs and backspaces; default false), "headingStyles"?: bool (Built-in Heading styles; default false), "doubleSpacePeriod"?: bool (Add a period with double-space; default false), "autoAddFirstLetter"|"autoAddInitialCaps"?: bool (Automatically add words to the exception lists when an automatic capital is undone; default true), "add"?: {"from": string, "to": string} (adds or replaces an entry), "delete"?: string (an entry's "from", built in or added), "addException"?: string, "deleteException"?: string, "addInitialCapsException"?: string (keeps TWo INitial CApitals, like "IDs"), "deleteInitialCapsException"?: string} → options, entries and exceptions"#,
            )
            .pure(),
        CommandSpec::new("tools.customizeKeyboard", "Customize Keyboard", "File › Options", customize_keyboard)
            .params(
                r#"{"command"?: id (its keys), "key"?: "Mod+Shift+K" (the command it runs), "assign"?: {"command": id, "key": string}, "remove"?: {"command": id, "key": string}, "reset"?: true (built-in keys only)} → {custom: {assigned: {key: id}, removed: [key]}, …}; no params opens the dialog"#,
            )
            .pure(),
        CommandSpec::new("tools.customizeRibbon", "Customize Ribbon", "File › Options", customize_ribbon)
            .params(
                r#"{"hide"?: tab, "show"?: tab, "newTab"?: {"name"?, "after"?: tab}, "newGroup"?: {"tab", "name"?}, "add"?: {"tab", "group", "command", "after"?: id}, "remove"?: {"tab", "group"?, "command"?}, "rename"?: {"tab", "group"?, "to"}, "move"?: {"tab", "group"?, "command"?, "by": int}, "qatAdd"?: id, "qatRemove"?: id, "qatMove"?: {"command", "by": int}, "reset"?: true | "ribbon" | "qat", "layout"?: object} → {tabs: [{name, custom?, hidden?, groups?: [{name, commands}]}], qat: [id]}; only custom tabs and groups can be renamed or removed (built-in tabs can be hidden and moved); no params opens the dialog"#,
            )
            .pure(),
        // Word's Search box in the title bar ("Tell me what you want to do"), Alt+Q (#819).
        CommandSpec::new("tools.search", "Search", "Title Bar", search_commands)
            .key(crate::cmd::non_mac("Alt+Q"))
            .params(r#"{"query"?: string} → {matches: [{id, label, location, shortcut}]} (every word of the query in the label, location or id; at most 50); no query opens the Search box"#)
            .pure(),
        CommandSpec::new("review.compare", "Compare", "Review › Compare", compare).params(COMPARE_PARAMS),
        CommandSpec::new("review.combine", "Combine", "Review › Compare", compare).params(COMPARE_PARAMS),
        CommandSpec::new("file.accessibility", "Check Accessibility", "Review › Accessibility", accessibility).pure(),
        CommandSpec::new("file.inspect", "Inspect Document", "File › Info", inspect_doc)
            .params(r#"{"remove"?: ["comments", "revisions", "properties", "hidden", "headers"]} → {found: {comments, revisions, properties, hiddenText, hiddenCount, hidden: [{story: "body|header|footer|footnote|endnote|comment|textBox", part?, text}] (at most 1000), truncated, unsupported: [{story, object: "table|opaque", ..}] (hidden text not inspected: table styles, verbatim objects), headers}, removed, residual: [..] (hidden text a removal couldn't reach), removedNotes: [{kind, part, text}] (notes whose hidden reference was removed)}; whole hidden paragraphs go with their marks; the removals are refused while the document tracks changes"#),
        CommandSpec::new("file.protect", "Protect Document", "File › Info", |s, v| {
            let mode = p::str(v, "mode").unwrap_or("readOnly");
            s.run("review.restrict", &json!({"mode": mode}))
        })
        .params(r#"{"mode": "none|readOnly|comments|trackedChanges"} (starting enforcement; "none" doesn't stop a protected document — use review.restrict {"stop": true})"#),
        CommandSpec::new("file.versions", "Version History", "File › Info", |s, v| {
            // Validate a restore before anything is saved: a rejected command must leave the
            // version list as it was. A `restore` that is there but not a non-negative integer
            // is refused, not read as absent.
            let restore = if v.get("restore").is_some_and(|x| !x.is_null()) {
                let i = p::u64(v, "restore").ok_or_else(|| CmdError::Params("restore (non-negative integer) is required".into()))?;
                // A `u64` that doesn't fit in `usize` names no version: `as usize` would wrap it
                // to a small index on a 32-bit target and restore the wrong document.
                let i = usize::try_from(i).map_err(|_| CmdError::Params("no such version".into()))?;
                let version = s.versions.get(i).cloned().ok_or_else(|| CmdError::Params("no such version".into()))?;
                // The restore replaces the whole document, including the content the protection
                // kept read-only, so the protection is stopped first (with its password, when it
                // has one) — a restore must not go around it.
                crate::cmd::protect::check_version_restore(s)?;
                Some(version)
            } else {
                None
            };
            if let Some(label) = p::str(v, "save") {
                s.versions.push((label.to_string(), crate::cmd::now_iso(), s.doc.clone()));
                if s.versions.len() > 50 {
                    s.versions.remove(0);
                }
            }
            if let Some((_, _, d)) = restore {
                s.checkpoint("Restore Version");
                // A snapshot's own protection is stale: it was taken at some earlier moment, and
                // restoring it must not bring a protection back that the live document doesn't
                // have (#1327 follow-up).
                let protection = s.doc.settings.protection.clone();
                let protect_hash = s.doc.settings.protect_hash.clone();
                let protect_formatting = s.doc.settings.protect_formatting;
                s.doc = d;
                s.doc.settings.protection = protection;
                s.doc.settings.protect_hash = protect_hash;
                s.doc.settings.protect_formatting = protect_formatting;
                // The restored document reuses the open document's id; the body and any equation
                // editing session are replaced.
                s.note_body_replaced();
                s.math = None;
                s.touch();
                s.clamp_selection();
            }
            Ok(Value::Array(s.versions.iter().enumerate().map(|(i, (l, d, _))| json!({"index": i, "label": l, "date": d})).collect()))
        })
        .params(r#"{"save"?: label, "restore"?: index}"#)
        .pure(),
        CommandSpec::new("file.recover", "Recover Unsaved Documents", "File › Info", |s, v| s.recover(v))
        .params(r#"{"restore"?: recoveryId, "discard"?: recoveryId, "refresh"?: true}; desktop only; returns {enabled,busy,documents,error}; restore loads asynchronously without a file path"#)
        .pure(),
        CommandSpec::new("file.newFromTemplate", "New from Template", "File › New", |s, v| {
            let path = p::req_str(v, "path")?;
            let mut doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
            // As in Word, the new document is attached to the template it came from (#2852).
            doc.settings.attached_template = Some(path.to_string());
            s.add_document(doc);
            s.dirty = true;
            sel_result(s)
        })
        .params(r#"{"path": string} → a new, untitled document from the template, attached to it"#)
        .pure(),
        CommandSpec::new("file.saveTemplate", "Save as Template", "File › Save As", |s, v| {
            let path = p::req_str(v, "path")?;
            // A .dotm keeps its name (#2852); anything else becomes a .dotx.
            let path = if crate::cmd::user_templates::is_template_name(path) { path.to_string() } else { format!("{path}.dotx") };
            crate::io::save_path(std::path::Path::new(&path), &s.doc).map_err(CmdError::Failed)?;
            Ok(json!({"path": path}))
        })
        .params(r#"{"path": string}"#)
        .pure(),
        CommandSpec::new("file.autosave", "AutoSave", "Quick Access Toolbar", |s, v| {
            s.autosave = p::bool(v, "value").unwrap_or(!s.autosave);
            Ok(json!({"value": s.autosave}))
        })
        .pure(),
        CommandSpec::new("file.compatibility", "Check Compatibility", "File › Info", |s, _| {
            let mut issues = Vec::new();
            if s.doc
                .para_paths(StoryRef::Body)
                .iter()
                .any(|p| s.doc.para(StoryRef::Body, p).is_some_and(|x| x.objects.iter().any(|o| matches!(o, InlineObject::Opaque { .. }))))
            {
                issues.push("Some embedded objects from the original file are shown as text only.");
            }
            // Headers and footers too: `{ IF { PAGE } … }` usually lives there.
            if super::inspect::all_stories(&s.doc).into_iter().any(|story| {
                s.doc.para_paths(story).iter().any(|p| {
                    s.doc
                        .para(story, p)
                        .is_some_and(|x| x.objects.iter().any(|o| matches!(o, InlineObject::Field { code_xml, .. } if !code_xml.is_empty())))
                })
            }) {
                issues.push("Fields nested in other fields show their last result and aren't updated.");
            }
            Ok(json!({"issues": issues}))
        })
        .pure(),
        CommandSpec::new("file.share", "Share", "File", |s, _| {
            s.push_ui_request(json!({"open": "saveAs"}));
            Ok(json!({"hint": "Save or export a copy (PDF, Word document) to share it."}))
        })
        .pure(),
        CommandSpec::new("insert.quickParts", "Quick Parts", "Insert › Text", |s, v| quick_parts(s, v, wordcraft_doc::QUICK_PARTS_GALLERY)).params(
            r#"{"save"?: name (from the selection), "in"?: "document" (save into the document's own building blocks, kept in its file; else this session's), "insert"?: name, "delete"?: name} → {"parts": names (this session's, then the document's Quick Parts), "document": the document's Quick Parts}"#,
        ),
        CommandSpec::new("insert.autoText", "AutoText", "Insert › Text › Quick Parts", |s, v| quick_parts(s, v, wordcraft_doc::AUTO_TEXT_GALLERY))
            .key(crate::cmd::non_mac("Alt+F3"))
            .params(r#"{"save"?: name, "in"?: "document", "insert"?: name, "delete"?: name} → {"parts": names, "document": the document's AutoText}"#),
        CommandSpec::new("insert.docProperty", "Document Property", "Insert › Text › Quick Parts", |s, v| {
            let name = p::str(v, "name").unwrap_or("Title").to_string();
            let c = &s.doc.core;
            let val = match name.to_lowercase().as_str() {
                "title" => c.title.clone(),
                "author" | "creator" => c.creator.clone(),
                "subject" => c.subject.clone(),
                "keywords" => c.keywords.clone(),
                "comments" | "description" => c.description.clone(),
                "category" => c.category.clone(),
                _ => return Err(CmdError::Params("unknown property".into())),
            };
            let instr = match name.to_lowercase().as_str() {
                "author" | "creator" => "AUTHOR".to_string(),
                "title" => "TITLE".to_string(),
                other => format!("DOCPROPERTY {other}"),
            };
            let props = s.typing_props();
            let at = delete_selection(s)?;
            let end = s.doc.insert_object(&at, InlineObject::Field { instr, result: val, locked: false, dirty: false, code: false, form: String::new(), code_xml: String::new(), }, &props)?;
            s.sel = Selection::caret(end);
            sel_result(s)
        })
        .params(r#"{"name": "Title|Author|Subject|Keywords|Comments|Category"}"#),
        CommandSpec::new("insert.signatureLine", "Signature Line", "Insert › Text", |s, v| {
            let signer = p::str(v, "signer").unwrap_or("").to_string();
            let title = p::str(v, "title").unwrap_or("").to_string();
            let at = delete_selection(s)?;
            let mut lines = vec!["X ______________________________".to_string()];
            if !signer.is_empty() {
                lines.push(signer);
            }
            if !title.is_empty() {
                lines.push(title);
            }
            let frag = wordcraft_doc::edit::Fragment {
                blocks: lines.iter().map(|l| Block::Para(Paragraph::with_text(l, CharProps::default()).styled("NoSpacing"))).collect(),
                ..Default::default()
            };
            let end = s.doc.insert_fragment(&at, &frag)?;
            s.sel = Selection::caret(end);
            sel_result(s)
        })
        .params(r#"{"signer"?: string, "title"?: string}"#),
        CommandSpec::new("insert.object", "Object", "Insert › Text", |s, v| {
            let path = p::req_str(v, "path")?;
            s.run("insert.textFromFile", &json!({"path": path}))
        })
        .params(r#"{"path": string}"#),
        CommandSpec::new("insert.spreadsheet", "Spreadsheet Table", "Insert › Tables", |s, v| {
            let csv = p::str(v, "csv").unwrap_or("A,B,C\n1,2,3\n4,5,6");
            let rows = crate::cmd::mailings::parse_csv(csv);
            let cols = rows.iter().map(Vec::len).max().unwrap_or(1).clamp(1, 63);
            s.run("insert.table", &json!({"rows": rows.len().max(1), "cols": cols, "style": "GridTable1Light"}))?;
            let Some((tp, _, _)) = s.sel.focus.path.cell() else { return sel_result(s) };
            let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
            for (r, row) in rows.iter().enumerate() {
                for (c, val) in row.iter().enumerate() {
                    if let Some(cell) = t.rows.get_mut(r).and_then(|x| x.cells.get_mut(c)) {
                        cell.blocks = vec![para_block(Paragraph::with_text(val, CharProps::default()))];
                    }
                }
            }
            sel_result(s)
        })
        .params(r#"{"csv"?: string}"#),
        CommandSpec::new("layout.linkToPrevious", "Link to Previous", "Header & Footer › Navigation", |s, v| {
            let header = p::bool(v, "footer").map(|f| !f).unwrap_or(true);
            let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
            let sect = s.doc.section_mut(block);
            if header {
                sect.headers = Default::default();
            } else {
                sect.footers = Default::default();
            }
            Ok(json!({"linked": true}))
        })
        .key(crate::cmd::non_mac("Alt+Shift+R"))
        .params(r#"{"footer"?: bool}"#),
        CommandSpec::new("hf.next", "Next Section", "Header & Footer › Navigation", |s, _| hf_nav(s, 1)).pure(),
        CommandSpec::new("hf.previous", "Previous Section", "Header & Footer › Navigation", |s, _| hf_nav(s, -1)).pure(),
        CommandSpec::new("hf.position", "Header/Footer Position", "Header & Footer › Position", |s, v| {
            let (h, f) = (p::f32(v, "header"), p::f32(v, "footer"));
            let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
            let sect = s.doc.section_mut(block);
            if let Some(h) = h {
                sect.header = h.clamp(0.0, 300.0);
            }
            if let Some(f) = f {
                sect.footer = f.clamp(0.0, 300.0);
            }
            Ok(json!({"header": sect.header, "footer": sect.footer}))
        })
        .params(r#"{"header"?: pt, "footer"?: pt}"#),
        CommandSpec::new("table.cellMargins", "Cell Margins", "Table Layout › Alignment", |s, v| {
            let Some((tp, _, _)) = s.sel.focus.path.cell() else { return Err(CmdError::Disabled("not in a table".into())) };
            let story = s.sel.focus.story;
            let current = {
                let table = s.doc.table(story, &tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
                let style = table.props.style.as_deref().and_then(|id| s.doc.styles.table_style(id));
                table.props.cell_margins.or(style.and_then(|st| st.parts.cell_margins)).unwrap_or([0.0, 5.4, 0.0, 5.4])
            };
            let m = [
                p::f32(v, "top").unwrap_or(current[0]),
                p::f32(v, "left").unwrap_or(current[1]),
                p::f32(v, "bottom").unwrap_or(current[2]),
                p::f32(v, "right").unwrap_or(current[3]),
            ]
            .map(|x| x.clamp(0.0, 144.0));
            s.doc.table_mut(story, &tp)?.props.cell_margins = Some(m);
            for path in s.doc.para_paths(story).into_iter().filter(|p| p.0.starts_with(&tp.0) && p.0.len() > tp.0.len()) {
                s.doc.para_mut(story, &path)?.touch();
            }
            sel_result(s)
        })
        .params(r#"{"top"?, "left"?, "bottom"?, "right"? (pt)}"#),
        CommandSpec::new("table.borderPainter", "Border Painter", "Table Design › Borders", |s, v| {
            s.run(
                "table.borders",
                &json!({"kind": p::str(v, "kind").unwrap_or("all"), "width": p::f32(v, "width").unwrap_or(0.5), "color": p::str(v, "color")}),
            )
        }),
        CommandSpec::new("para.setNumberingValue", "Set Numbering Value", "Home › Paragraph › Numbering", |s, v| {
            let value = p::u64(v, "value").unwrap_or(1).clamp(0, 100_000) as u32;
            let f = s.sel.focus.clone();
            let n = s
                .doc
                .para_at(&f)
                .and_then(|x| x.props.numbering)
                .filter(|n| n.num != 0)
                .ok_or_else(|| CmdError::Failed("not in a numbered list".into()))?;
            let new = s.doc.numbering.restart(n.num).ok_or_else(|| CmdError::Failed("bad list".into()))?;
            if let Some(num) = s.doc.numbering.nums.iter_mut().find(|x| x.id == new) {
                num.start_overrides = vec![(n.level, value)];
            }
            let paths: Vec<_> = s.doc.para_paths(f.story).into_iter().filter(|q| *q >= f.path).collect();
            for path in paths {
                let para = s.doc.para_mut(f.story, &path)?;
                match para.props.numbering {
                    Some(x) if x.num == n.num => para.props.numbering = Some(NumRef { num: new, level: x.level }),
                    _ => break,
                }
                para.touch();
            }
            sel_result(s)
        })
        .params(r#"{"value": n}"#),
        CommandSpec::new("para.defineBullet", "Define New Bullet", "Home › Paragraph › Bullets", |s, v| {
            s.run("para.bullets", &json!({"kind": p::str(v, "char").unwrap_or("★")}))
        })
        .params(r#"{"char": string}"#),
        CommandSpec::new("para.defineNumber", "Define New Number Format", "Home › Paragraph › Numbering", |s, v| {
            let fmt = wordcraft_doc::section::NumFormat::from_ooxml(p::str(v, "format").unwrap_or("decimal"));
            let text = p::str(v, "text").unwrap_or("%1.").to_string();
            let start = p::u64(v, "start").unwrap_or(1).min(100_000) as u32;
            let id = s.doc.numbering.try_add_list(wordcraft_doc::ListKind::Numbered)?;
            if let Some(a) = s.doc.numbering.abstract_of(id).map(|a| a.id)
                && let Some(abs) = s.doc.numbering.abstracts.iter_mut().find(|x| x.id == a)
                && let Some(l) = abs.levels.first_mut()
            {
                l.format = fmt;
                l.text = text;
                l.start = start;
            }
            crate::cmd::para::fmt(s, &|pp| {
                pp.numbering = Some(NumRef { num: id, level: 0 });
                pp.style = Some("ListParagraph".into());
            })
        })
        .params(r#"{"format": "decimal|upperRoman|lowerLetter…", "text"?: "%1.", "start"?: n}"#),
        CommandSpec::new("review.language", "Language", "Review › Language", language).params(
            r#"{"lang"?: "en-US|en-GB|pl-PL|fr-FR|…" (BCP 47; default en-US), "noProof"?: bool, "detect"?: bool (detect language automatically, a preference), "default"?: bool (also the document's default and the saved language for future blank documents)}"#,
        ),
        CommandSpec::new("review.showMarkup", "Show Markup", "Review › Tracking", |s, v| {
            s.view.show_markup = p::bool(v, "value").unwrap_or(!s.view.show_markup);
            s.relayout();
            Ok(json!({"value": s.view.show_markup}))
        })
        .pure(),
        CommandSpec::new("review.editor", "Editor", "Home › Editor", |s, v| s.run("review.spelling", v)).pure(),
        CommandSpec::new("select.similar", "Select Text with Similar Formatting", "Home › Editing › Select", select_similar).pure(),
        CommandSpec::new("select.extend", "Extend Selection", "Editing › Selection", |s, _| {
            // F8 grows the selection: word → sentence → paragraph → document.
            let (a, b) = s.sel.ordered();
            let para_len = s.doc.para_at(&a).map(|x| x.len()).unwrap_or(0);
            let id = if a == b {
                "select.word"
            } else if a.path == b.path && (a.off > 0 || b.off < para_len) {
                if s.doc.para_at(&a).map(|x| x.sentence_at(a.off)) == Some((a.off, b.off)) { "select.paragraph" } else { "select.sentence" }
            } else {
                "select.all"
            };
            let params = if id == "select.all" { json!({"whole": true}) } else { json!({}) };
            s.run(id, &params)
        })
        .key("F8")
        .pure(),
        CommandSpec::new("design.effects", "Effects", "Design › Document Formatting", |s, v| {
            s.doc.settings.theme_name =
                format!("{} ({})", s.doc.settings.theme_name.split(" (").next().unwrap_or("Craft"), p::str(v, "name").unwrap_or("Subtle"));
            sel_result(s)
        }),
        CommandSpec::new("view.immersive", "Immersive Reader", "View › Immersive", |s, _| {
            s.view.read_mode = !s.view.read_mode;
            Ok(json!({"value": s.view.read_mode}))
        })
        .pure(),
        CommandSpec::new("view.vertical", "Vertical", "View › Page Movement", |s, _| {
            s.view.multi_page = false;
            s.view.fit.clear();
            Ok(json!({"pageMovement": "vertical"}))
        })
        .pure(),
        CommandSpec::new("view.sideToSide", "Side to Side", "View › Page Movement", |s, _| {
            s.view.multi_page = true;
            s.view.single_column = false;
            s.view.fit = "multiplePages".into();
            Ok(json!({"pageMovement": "sideToSide"}))
        })
        .pure(),
    ]
}
