//! File tab (Backstage): new, open, save, export, print, properties; document inspection.

use serde_json::{Value, json};
use wordcraft_doc::{Block, Document, StoryRef};

use super::{pos_json, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("file.new", "New", "File", new)
            .key("Mod+N")
            .params(r#"{"template"?: "blank|sample|letter|resume|report", "locale"?: "nb-NO|nn-NO|en-US"}"#)
            .pure(),
        CommandSpec::new("file.open", "Open", "File", open).key("Mod+O").params(r#"{"path": string}"#).pure(),
        CommandSpec::new("file.save", "Save", "File", save).key("Mod+S").params(r#"{"path"?: string}"#).pure(),
        CommandSpec::new("file.saveAs", "Save As", "File", save_as).key("F12").params(r#"{"path": string}"#).pure(),
        CommandSpec::new("file.exportPdf", "Export PDF", "File › Export", |s, v| {
            let path = p::req_str(v, "path")?;
            let path = if path.to_ascii_lowercase().ends_with(".pdf") { path.to_string() } else { format!("{path}.pdf") };
            crate::io::save_path(std::path::Path::new(&path), &s.doc).map_err(CmdError::Failed)?;
            Ok(json!({"path": path}))
        })
        .params(r#"{"path": string}"#)
        .pure(),
        CommandSpec::new("file.exportPng", "Export Page as PNG", "File › Export", export_png)
            .params(r#"{"path": string, "page"?: n (1-based), "scale"?: px per pt}"#)
            .pure(),
        CommandSpec::new("file.print", "Print", "File", |s, _| {
            s.ui_requests.push(json!({"open": "print"}));
            sel_result(s)
        })
        .key("Mod+P")
        .pure(),
        CommandSpec::new("file.close", "Close", "File", |s, _| {
            s.ui_requests.push(json!({"close": true}));
            sel_result(s)
        })
        .key("Mod+W")
        .pure(),
        CommandSpec::new("file.properties", "Properties", "File › Info", properties)
            .params(r#"{"title"?, "subject"?, "author"?, "keywords"?, "comments"?, "category"?}"#),
        CommandSpec::new("file.info", "Info", "File", info).pure(),
        CommandSpec::new("file.options", "Options", "File", |s, _| {
            s.ui_requests.push(json!({"open": "options"}));
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("file.setAuthor", "User Name", "File › Options › General", |s, v| {
            s.author = p::req_str(v, "name")?.to_string();
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("document.inspect", "Inspect Document", "Agents", inspect).params(r#"{"text"?: bool}"#).pure(),
        CommandSpec::new("document.text", "Document Text", "Agents", |s, v| {
            let story = super::story_param(s, v);
            Ok(json!({"text": s.doc.plain_text(story)}))
        })
        .pure(),
        CommandSpec::new("document.paragraph", "Paragraph Details", "Agents", paragraph).params(r#"{"path": [n], "story"?: Story}"#).pure(),
        CommandSpec::new("document.selection", "Selection", "Agents", |s, _| {
            let mut r = sel_result(s)?;
            if let Some(o) = r.as_object_mut() {
                o.insert("text".into(), json!(s.selected_text()));
                o.insert("format".into(), super::format::state(s));
            }
            Ok(r)
        })
        .pure(),
        CommandSpec::new("document.layout", "Layout Summary", "Agents", layout_summary).pure(),
        CommandSpec::new("document.setText", "Replace Document Text", "Agents", |s, v| {
            let text = p::req_str(v, "text")?;
            let d = Document::from_text(text);
            s.doc.body = d.body;
            s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
            sel_result(s)
        })
        .params(r#"{"text": string}"#),
    ]
}

fn new(s: &mut Session, v: &Value) -> CmdResult {
    let mut doc = match p::str(v, "template").unwrap_or("blank") {
        "sample" => crate::sample::sample_document(),
        "letter" => crate::sample::letter(),
        "resume" => crate::sample::resume(),
        "report" => crate::sample::report(),
        _ => Document::new(),
    };
    let locale = p::str(v, "locale").unwrap_or("").to_ascii_lowercase().replace('_', "-");
    let norwegian = match locale.as_str() {
        "nb" | "nb-no" => Some("nb-NO"),
        "nn" | "nn-no" => Some("nn-NO"),
        _ => None,
    };
    if let Some(language) = norwegian {
        doc.last_section.page_w = 21.0 * wordcraft_geom::PT_PER_CM;
        doc.last_section.page_h = 29.7 * wordcraft_geom::PT_PER_CM;
        doc.styles.default_chr.lang = Some(language.into());
    }
    s.set_document(doc);
    s.path = None;
    sel_result(s)
}

fn open(s: &mut Session, v: &Value) -> CmdResult {
    let Some(path) = p::str(v, "path") else {
        s.ui_requests.push(json!({"open": "openFile"}));
        return sel_result(s);
    };
    let doc = if let Some(data) = p::str(v, "data") {
        let bytes = super::insert::base64_decode(data).ok_or_else(|| CmdError::Params("bad base64".into()))?;
        crate::io::open_bytes(path, &bytes).map_err(CmdError::Failed)?
    } else {
        crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?
    };
    s.set_document(doc);
    s.path = Some(path.into());
    Ok(json!({"path": path, "paragraphs": s.doc.paragraph_count(), "words": s.doc.word_count()}))
}

fn save(s: &mut Session, v: &Value) -> CmdResult {
    let path = match p::str(v, "path").map(std::path::PathBuf::from).or_else(|| s.path.clone()) {
        Some(p) => p,
        None => {
            s.ui_requests.push(json!({"open": "saveAs"}));
            return Ok(json!({"saved": false}));
        }
    };
    s.doc.core.modified = super::now_iso();
    if s.doc.core.created.is_empty() {
        s.doc.core.created = s.doc.core.modified.clone();
    }
    s.doc.core.last_modified_by = s.author.clone();
    s.doc.core.revision = s.doc.core.revision.saturating_add(1);
    crate::io::save_path(&path, &s.doc).map_err(CmdError::Failed)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ["docx", "odt", "rtf", "json"].contains(&ext.as_str()) {
        s.path = Some(path.clone());
        s.dirty = false;
    }
    Ok(json!({"saved": true, "path": path.to_string_lossy()}))
}

fn save_as(s: &mut Session, v: &Value) -> CmdResult {
    if p::str(v, "path").is_none() {
        s.ui_requests.push(json!({"open": "saveAs"}));
        return Ok(json!({"saved": false}));
    }
    save(s, v)
}

fn export_png(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    let page = p::u64(v, "page").unwrap_or(1).max(1) as usize - 1;
    let scale = p::f32(v, "scale").unwrap_or(2.0).clamp(0.1, 8.0);
    let l = s.export_layout();
    let pg = l.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {}", page + 1)))?;
    let img = wordcraft_render::render_page(&s.doc, pg, scale, &Default::default());
    let png = img.to_png();
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::write(path, &png).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    Ok(json!({"path": path, "width": img.width, "height": img.height, "bytes": png.len()}))
}

fn properties(s: &mut Session, v: &Value) -> CmdResult {
    let c = &mut s.doc.core;
    let set = |dst: &mut String, k: &str| {
        if let Some(x) = p::str(v, k) {
            *dst = x.to_string();
        }
    };
    set(&mut c.title, "title");
    set(&mut c.subject, "subject");
    set(&mut c.creator, "author");
    set(&mut c.keywords, "keywords");
    set(&mut c.description, "comments");
    set(&mut c.category, "category");
    Ok(serde_json::to_value(&s.doc.core).unwrap_or(Value::Null))
}

fn info(s: &mut Session, _: &Value) -> CmdResult {
    let l = s.layout();
    Ok(json!({
        "path": s.path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "dirty": s.dirty,
        "pages": l.pages.len(),
        "words": s.doc.word_count(),
        "paragraphs": s.doc.paragraph_count(),
        "properties": s.doc.core,
        "trackChanges": s.doc.settings.track_changes,
        "comments": s.doc.comments.len(),
        "sections": s.doc.sections().len(),
        "layoutMs": l.ms,
    }))
}

/// Structure for agents: blocks with text and formatting summary, selection, stats.
fn inspect(s: &mut Session, v: &Value) -> CmdResult {
    let with_text = p::bool(v, "text").unwrap_or(true);
    let mut blocks = Vec::new();
    for (i, b) in s.doc.body.iter().enumerate().take(2000) {
        match &**b {
            Block::Para(p) => {
                let style = p.props.style.clone().unwrap_or_else(|| "Normal".into());
                let mut o = json!({"index": i, "type": "paragraph", "style": style, "len": p.len()});
                if with_text {
                    o["text"] = json!(p.plain_text());
                }
                if let Some(n) = p.props.numbering.filter(|n| n.num != 0) {
                    o["list"] = json!({"num": n.num, "level": n.level});
                }
                if !p.props.is_empty() {
                    o["props"] = serde_json::to_value(&p.props).unwrap_or(Value::Null);
                }
                let runs: Vec<Value> = p
                    .run_ranges()
                    .filter(|(_, c)| !c.is_empty())
                    .map(|(r, c)| json!({"start": r.start, "end": r.end, "props": c}))
                    .take(50)
                    .collect();
                if !runs.is_empty() {
                    o["runs"] = json!(runs);
                }
                if !p.objects.is_empty() {
                    o["objects"] = serde_json::to_value(&p.objects).unwrap_or(Value::Null);
                }
                if p.section.is_some() {
                    o["sectionBreak"] = json!(true);
                }
                blocks.push(o);
            }
            Block::Table(t) => {
                let cells: Vec<Vec<String>> = t
                    .rows
                    .iter()
                    .map(|r| {
                        r.cells
                            .iter()
                            .map(|c| c.blocks.iter().filter_map(|b| b.as_para().map(|p| p.plain_text())).collect::<Vec<_>>().join("\n"))
                            .collect()
                    })
                    .collect();
                blocks.push(json!({"index": i, "type": "table", "rows": t.rows.len(), "cols": t.cols(), "style": t.props.style, "cells": cells}));
            }
        }
    }
    let l = s.layout();
    let parts: Vec<Value> =
        s.doc.parts.iter().map(|(id, p)| json!({"id": id, "kind": p.kind, "text": s.doc.plain_text(StoryRef::Part(*id))})).collect();
    Ok(json!({
        "blocks": blocks,
        "parts": parts,
        "selection": {"anchor": pos_json(&s.sel.anchor), "focus": pos_json(&s.sel.focus), "text": s.selected_text()},
        "pages": l.pages.len(),
        "words": s.doc.word_count(),
        "sections": s.doc.sections().iter().map(|(end, sp)| json!({"endBlock": end, "props": sp})).collect::<Vec<_>>(),
        "comments": s.doc.comments.len(),
        "trackChanges": s.doc.settings.track_changes,
        "properties": s.doc.core,
        "canUndo": s.can_undo(),
        "canRedo": s.can_redo(),
    }))
}

fn paragraph(s: &mut Session, v: &Value) -> CmdResult {
    let path: Vec<u32> = serde_json::from_value(v.get("path").cloned().unwrap_or(json!([0]))).map_err(|e| CmdError::Params(e.to_string()))?;
    let story = super::story_param(s, v);
    let p = s.doc.para(story, &wordcraft_doc::Path(path)).ok_or_else(|| CmdError::Params("no paragraph there".into()))?;
    let resolved = s.doc.styles.resolve_para(&p.props);
    Ok(json!({"paragraph": p, "resolved": resolved}))
}

fn layout_summary(s: &mut Session, _: &Value) -> CmdResult {
    let l = s.layout();
    let pages: Vec<Value> = l
        .pages
        .iter()
        .map(|p| {
            let lines: usize = p.items.iter().map(|it| if let wordcraft_layout::Placed::Lines { l0, l1, .. } = it { l1 - l0 } else { 0 }).sum();
            json!({"number": p.number, "w": p.w, "h": p.h, "section": p.section, "lines": lines, "firstBlock": p.first_block, "header": p.header_story, "footer": p.footer_story})
        })
        .collect();
    let caret = l.caret_on(&s.sel.focus, s.page_hint).map(|c| json!({"page": c.page, "x": c.x, "y": c.top, "h": c.height}));
    Ok(json!({"pages": pages, "caret": caret, "ms": l.ms}))
}
