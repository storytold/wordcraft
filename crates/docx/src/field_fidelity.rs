//! Range-field `w:ffData` (`form`) and result `w:rPrChange` preservation around `craft-docx`.

use craft_xml::{El, Node};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Block, Blocks, Document};

const RANGE_MARKER: &str = "ADDIN __WC_RANGE__ ";
const ZIP_LIMITS: craft_zip::Limits = craft_zip::Limits { max_entries: 10_000, max_total_bytes: 256 << 20 };

#[derive(Default)]
struct FieldScan {
    form: String,
    in_result: bool,
    has_rpr_change: bool,
}

fn scan_fields_el(el: &El, stack: &mut Vec<FieldScan>, forms: &mut Vec<String>, needs_range_marker: &mut Vec<bool>) {
    match el.name.as_str() {
        "w:fldChar" => match el.attr_or_plain("w:fldCharType") {
            Some("begin") => {
                let form = el.child("w:ffData").map_or_else(String::new, |ff| ff.to_xml());
                stack.push(FieldScan { form, in_result: false, has_rpr_change: false });
            }
            Some("separate") => {
                if let Some(top) = stack.last_mut() {
                    top.in_result = true;
                }
            }
            Some("end") => {
                if let Some(done) = stack.pop() {
                    forms.push(done.form);
                    needs_range_marker.push(done.has_rpr_change);
                }
            }
            _ => {}
        },
        "w:rPrChange" => {
            if let Some(top) = stack.last_mut()
                && top.in_result
            {
                top.has_rpr_change = true;
            }
        }
        _ => {}
    }
    for ch in el.elements() {
        scan_fields_el(ch, stack, forms, needs_range_marker);
    }
}

fn mark_fields_el(el: &mut El, needs_range_marker: &[bool], field_idx: &mut usize, active_marked: &mut bool) -> bool {
    let mut changed = false;
    match el.name.as_str() {
        "w:fldChar" if el.attr_or_plain("w:fldCharType") == Some("begin") => {
            *active_marked = needs_range_marker.get(*field_idx).copied().unwrap_or(false);
            *field_idx += 1;
        }
        "w:instrText" if *active_marked => {
            if let Some(Node::Text(t)) = el.children.first_mut() {
                *t = format!("{RANGE_MARKER}{t}");
            } else {
                el.children.insert(0, Node::Text(RANGE_MARKER.to_string()));
            }
            *active_marked = false;
            changed = true;
        }
        _ => {}
    }
    for ch in el.elements_mut() {
        if mark_fields_el(ch, needs_range_marker, field_idx, active_marked) {
            changed = true;
        }
    }
    changed
}

fn inspect_and_mark_fields_xml(data: &[u8]) -> (Vec<String>, Option<Vec<u8>>) {
    let Ok(s) = std::str::from_utf8(data) else {
        return (Vec::new(), None);
    };
    if !s.contains("fldChar") {
        return (Vec::new(), None);
    }
    let Ok(mut root) = craft_xml::parse(data) else {
        return (Vec::new(), None);
    };
    let mut stack = Vec::new();
    let mut forms = Vec::new();
    let mut needs_range_marker = Vec::new();
    scan_fields_el(&root, &mut stack, &mut forms, &mut needs_range_marker);
    if !needs_range_marker.iter().any(|&b| b) {
        return (forms, None);
    }
    let mut field_idx = 0usize;
    let mut active_marked = false;
    if !mark_fields_el(&mut root, &needs_range_marker, &mut field_idx, &mut active_marked) {
        return (forms, None);
    }
    let mut out = String::with_capacity(data.len() + 64);
    out.push_str(craft_xml::DECL_STANDALONE);
    root.write_xml(&mut out);
    (forms, Some(out.into_bytes()))
}

pub(crate) fn preprocess_read(bytes: &[u8]) -> (Vec<String>, Option<Vec<u8>>) {
    let Ok(mut archive) = craft_zip::Archive::open(bytes, ZIP_LIMITS) else {
        return (Vec::new(), None);
    };
    let mut entries = Vec::with_capacity(archive.len());
    let mut forms = Vec::new();
    let mut changed = false;
    for idx in 0..archive.len() {
        let Some(name) = archive.entry_name(idx).ok().flatten() else {
            return (Vec::new(), None);
        };
        if archive.entry_is_dir(idx).ok().flatten().unwrap_or(false) {
            continue;
        }
        let stored = archive.is_stored(&name).unwrap_or(false);
        let Some(mut data) = archive.read(&name, craft_docx::MAX_XML_FILE as u64).ok().flatten() else {
            return (Vec::new(), None);
        };
        if name == "word/document.xml" {
            let (f, rewritten) = inspect_and_mark_fields_xml(&data);
            forms = f;
            if let Some(r) = rewritten {
                data = r;
                changed = true;
            }
        }
        let comp = if stored { craft_zip::Compression::Stored } else { craft_zip::Compression::Deflated };
        entries.push((name, data, comp));
    }
    if !changed {
        return (forms, None);
    }
    let mut out = craft_zip::Writer::new();
    for (name, data, comp) in entries {
        if out.add(&name, &data, comp).is_err() {
            return (forms, None);
        }
    }
    (forms, out.finish().ok())
}

fn apply_forms_to_blocks(blocks: &mut Blocks, forms: &mut std::slice::Iter<'_, String>) {
    for block in blocks.iter_mut() {
        match std::sync::Arc::make_mut(block) {
            Block::Para(p) => {
                for obj in &mut p.objects {
                    match obj {
                        InlineObject::FieldStart { instr, form, .. } => {
                            if let Some(rest) = instr.strip_prefix(RANGE_MARKER) {
                                *instr = rest.trim().to_string();
                            }
                            if let Some(f) = forms.next()
                                && form.is_empty()
                                && !f.is_empty()
                            {
                                *form = f.clone();
                            }
                        }
                        InlineObject::Field { form, .. } => {
                            if let Some(f) = forms.next()
                                && form.is_empty()
                                && !f.is_empty()
                            {
                                *form = f.clone();
                            }
                        }
                        _ => {}
                    }
                }
            }
            Block::Table(t) => {
                for r in &mut t.rows {
                    for c in &mut r.cells {
                        apply_forms_to_blocks(&mut c.blocks, forms);
                    }
                }
            }
        }
    }
}

pub(crate) fn postprocess_read(doc: &mut Document, forms: &[String]) {
    let mut iter = forms.iter();
    apply_forms_to_blocks(&mut doc.body, &mut iter);
}

fn collect_forms_from_blocks(blocks: &Blocks, out: &mut Vec<String>) {
    for block in blocks.iter() {
        match block.as_ref() {
            Block::Para(p) => {
                for obj in &p.objects {
                    match obj {
                        InlineObject::FieldStart { form, .. } | InlineObject::Field { form, .. } => {
                            out.push(form.clone());
                        }
                        _ => {}
                    }
                }
            }
            Block::Table(t) => {
                for r in &t.rows {
                    for c in &r.cells {
                        collect_forms_from_blocks(&c.blocks, out);
                    }
                }
            }
        }
    }
}

fn inject_forms_el(el: &mut El, forms: &[String], idx: &mut usize) -> bool {
    let mut changed = false;
    if el.name == "w:fldChar" && el.attr_or_plain("w:fldCharType") == Some("begin") {
        let cur_form = forms.get(*idx).map_or("", String::as_str);
        *idx += 1;
        if !cur_form.is_empty()
            && el.child("w:ffData").is_none()
            && let Ok(ff_el) = craft_xml::parse(cur_form.as_bytes())
        {
            el.children.push(Node::El(ff_el));
            changed = true;
        }
    }
    for ch in el.elements_mut() {
        if inject_forms_el(ch, forms, idx) {
            changed = true;
        }
    }
    changed
}

fn inject_forms_into_xml(data: &[u8], forms: &[String]) -> Option<Vec<u8>> {
    if !forms.iter().any(|f| !f.is_empty()) {
        return None;
    }
    let mut root = craft_xml::parse(data).ok()?;
    let mut idx = 0usize;
    if !inject_forms_el(&mut root, forms, &mut idx) {
        return None;
    }
    let mut out = String::with_capacity(data.len() + 256);
    out.push_str(craft_xml::DECL_STANDALONE);
    root.write_xml(&mut out);
    Some(out.into_bytes())
}

pub(crate) fn postprocess_write(doc: &Document, bytes: Vec<u8>) -> Vec<u8> {
    let mut forms = Vec::new();
    collect_forms_from_blocks(&doc.body, &mut forms);
    if !forms.iter().any(|f| !f.is_empty()) {
        return bytes;
    }
    let Ok(mut archive) = craft_zip::Archive::open(&bytes, ZIP_LIMITS) else {
        return bytes;
    };
    let mut entries = Vec::with_capacity(archive.len());
    let mut changed = false;
    for idx in 0..archive.len() {
        let Some(name) = archive.entry_name(idx).ok().flatten() else {
            return bytes;
        };
        if archive.entry_is_dir(idx).ok().flatten().unwrap_or(false) {
            continue;
        }
        let stored = archive.is_stored(&name).unwrap_or(false);
        let Some(mut data) = archive.read(&name, craft_docx::MAX_XML_FILE as u64).ok().flatten() else {
            return bytes;
        };
        if name == "word/document.xml"
            && let Some(rewritten) = inject_forms_into_xml(&data, &forms)
        {
            data = rewritten;
            changed = true;
        }
        let comp = if stored { craft_zip::Compression::Stored } else { craft_zip::Compression::Deflated };
        entries.push((name, data, comp));
    }
    if !changed {
        return bytes;
    }
    let mut out = craft_zip::Writer::new();
    for (name, data, comp) in entries {
        if out.add(&name, &data, comp).is_err() {
            return bytes;
        }
    }
    out.finish().unwrap_or(bytes)
}
