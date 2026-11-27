//! Table row/cell tracked insertion, deletion, and `w:tblPrExChange` preservation around `craft-docx`.

use craft_xml::{El, Node};
use wordcraft_doc::props::{PropChange, TableProps};
use wordcraft_doc::{Block, Document, Revision, RevisionKind};

const ZIP_LIMITS: craft_zip::Limits = craft_zip::Limits { max_entries: 10_000, max_total_bytes: 256 << 20 };

#[derive(Clone, Debug, Default)]
struct RevAttr {
    author: String,
    date: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct CellRev {
    ins: Option<RevAttr>,
    del: Option<RevAttr>,
}

#[derive(Clone, Debug, Default)]
struct RowRev {
    ex_change: Option<(RevAttr, TableProps)>,
    ins: Option<RevAttr>,
    del: Option<RevAttr>,
    cells: Vec<CellRev>,
}

fn rev_attr(el: &El) -> RevAttr {
    RevAttr { author: el.attr_or_plain("w:author").unwrap_or_default().to_string(), date: el.attr_or_plain("w:date").map(str::to_string) }
}

fn collect_tables_revs(el: &El, tables: &mut Vec<Vec<RowRev>>) {
    if el.name == "w:tbl" {
        let mut rows = Vec::new();
        for tr in el.children("w:tr") {
            let mut r = RowRev::default();
            if let Some(ex_ch) = tr.child("w:tblPrEx").and_then(|e| e.child("w:tblPrExChange")).or_else(|| tr.find("w:tblPrExChange")) {
                let mut tp = TableProps::default();
                if let Some(tblw) = ex_ch.find("w:tblW") {
                    let w_type = tblw.attr_or_plain("w:type").unwrap_or_default();
                    if let Some(w) = tblw.attr_or_plain("w:w").and_then(|s| s.parse::<f32>().ok()) {
                        if w_type == "pct" {
                            tp.width_pct = Some(w / 50.0);
                        } else {
                            tp.width = Some(w / 20.0);
                        }
                    }
                }
                r.ex_change = Some((rev_attr(ex_ch), tp));
            }
            if let Some(trpr) = tr.child("w:trPr") {
                if let Some(ins) = trpr.child("w:ins") {
                    r.ins = Some(rev_attr(ins));
                }
                if let Some(del) = trpr.child("w:del") {
                    r.del = Some(rev_attr(del));
                }
            }
            for tc in tr.children("w:tc") {
                let mut c = CellRev::default();
                if let Some(tcpr) = tc.child("w:tcPr") {
                    if let Some(ins) = tcpr.child("w:cellIns") {
                        c.ins = Some(rev_attr(ins));
                    }
                    if let Some(del) = tcpr.child("w:cellDel") {
                        c.del = Some(rev_attr(del));
                    }
                }
                r.cells.push(c);
            }
            rows.push(r);
        }
        tables.push(rows);
        return;
    }
    for ch in el.elements() {
        collect_tables_revs(ch, tables);
    }
}

fn push_rev(doc: &mut Document, kind: RevisionKind, attr: &RevAttr) -> u32 {
    let id = doc.revisions.len() as u32;
    doc.revisions.push(Revision { kind, author: attr.author.clone(), date: attr.date.clone().unwrap_or_default(), ..Default::default() });
    id
}

pub(crate) fn apply_read_table_revs(bytes: &[u8], doc: &mut Document) {
    let Ok(mut archive) = craft_zip::Archive::open(bytes, ZIP_LIMITS) else {
        return;
    };
    let Some(doc_xml) = archive.read("word/document.xml", craft_docx::MAX_XML_FILE as u64).ok().flatten() else {
        return;
    };
    let Ok(s) = std::str::from_utf8(&doc_xml) else {
        return;
    };
    if !s.contains("w:tbl") {
        return;
    }
    let Ok(root) = craft_xml::parse(&doc_xml) else {
        return;
    };
    let mut parsed = Vec::new();
    collect_tables_revs(&root, &mut parsed);
    if parsed.is_empty() {
        return;
    }
    let mut t_idx = 0usize;
    let body = std::mem::take(&mut doc.body);
    let mut new_body = Vec::with_capacity(body.len());
    for mut block in body {
        if let Block::Table(t) = std::sync::Arc::make_mut(&mut block) {
            if let Some(t_revs) = parsed.get(t_idx) {
                for (row, r_rev) in t.rows.iter_mut().zip(t_revs.iter()) {
                    if let Some((attr, old_tp)) = &r_rev.ex_change {
                        let rev_id = push_rev(doc, RevisionKind::Format, attr);
                        let ex = row.props.exceptions.get_or_insert_with(|| Box::new(TableProps::default()));
                        ex.fmt_change = PropChange::boxed(rev_id, old_tp.clone());
                    }
                    if let Some(attr) = &r_rev.ins {
                        row.props.ins = Some(push_rev(doc, RevisionKind::Insert, attr));
                    }
                    if let Some(attr) = &r_rev.del {
                        row.props.del = Some(push_rev(doc, RevisionKind::Delete, attr));
                    }
                    for (cell, c_rev) in row.cells.iter_mut().zip(r_rev.cells.iter()) {
                        if let Some(attr) = &c_rev.ins {
                            cell.props.ins = Some(push_rev(doc, RevisionKind::Insert, attr));
                        }
                        if let Some(attr) = &c_rev.del {
                            cell.props.del = Some(push_rev(doc, RevisionKind::Delete, attr));
                        }
                    }
                }
            }
            t_idx += 1;
        }
        new_body.push(block);
    }
    doc.body = new_body;
}

fn change_attr_vec(doc: &Document, rev: u32, next_id: &mut u32) -> Vec<(String, String)> {
    let id = *next_id;
    *next_id += 1;
    let r = doc.revisions.get(rev as usize);
    let author = r.map_or("Unknown", |x| x.author.as_str());
    let mut attrs = vec![("w:id".into(), id.to_string()), ("w:author".into(), author.to_string())];
    if let Some(d) = r.map(|x| x.date.as_str()).filter(|d| !d.is_empty()) {
        attrs.push(("w:date".into(), d.to_string()));
    }
    attrs
}

fn tblprex_inner_children(t: &TableProps) -> Vec<Node> {
    if let Some(p) = t.width_pct {
        let w = ((p.clamp(0.0, 1000.0) * 50.0).round() as i64).to_string();
        vec![Node::El(El { name: "w:tblW".into(), attrs: vec![("w:w".into(), w), ("w:type".into(), "pct".into())], children: Vec::new() })]
    } else if let Some(v) = t.width {
        let w = wordcraft_geom::to_twips(v).to_string();
        vec![Node::El(El { name: "w:tblW".into(), attrs: vec![("w:w".into(), w), ("w:type".into(), "dxa".into())], children: Vec::new() })]
    } else {
        Vec::new()
    }
}

fn patch_tables_el(el: &mut El, doc: &Document, tables: &[&wordcraft_doc::table::Table], t_idx: &mut usize, next_id: &mut u32) -> bool {
    if el.name == "w:tbl" {
        let idx = *t_idx;
        *t_idx += 1;
        let Some(tbl) = tables.get(idx) else {
            return false;
        };
        let mut changed = false;
        let mut r_idx = 0usize;
        for tr in el.elements_mut() {
            if tr.name != "w:tr" {
                continue;
            }
            let Some(row) = tbl.rows.get(r_idx) else {
                r_idx += 1;
                continue;
            };
            if let Some(ch) = row.props.exceptions.as_ref().and_then(|x| x.fmt_change.as_ref())
                && let Some(ex_el) = tr.child_mut("w:tblPrEx")
            {
                let attrs = change_attr_vec(doc, ch.rev, next_id);
                let inner = tblprex_inner_children(&ch.old);
                ex_el.children.push(Node::El(El {
                    name: "w:tblPrExChange".into(),
                    attrs,
                    children: vec![Node::El(El { name: "w:tblPrEx".into(), attrs: Vec::new(), children: inner })],
                }));
                changed = true;
            }
            if row.props.ins.is_some() || row.props.del.is_some() {
                if tr.child("w:trPr").is_none() {
                    let pos = tr.children.iter().position(|n| matches!(n, Node::El(c) if c.name == "w:tc")).unwrap_or(0);
                    tr.children.insert(pos, Node::El(El { name: "w:trPr".into(), attrs: Vec::new(), children: Vec::new() }));
                }
                if let Some(trpr) = tr.child_mut("w:trPr") {
                    if let Some(ins) = row.props.ins {
                        let attrs = change_attr_vec(doc, ins, next_id);
                        trpr.children.push(Node::El(El { name: "w:ins".into(), attrs, children: Vec::new() }));
                        changed = true;
                    }
                    if let Some(del) = row.props.del {
                        let attrs = change_attr_vec(doc, del, next_id);
                        trpr.children.push(Node::El(El { name: "w:del".into(), attrs, children: Vec::new() }));
                        changed = true;
                    }
                }
            }
            let mut c_idx = 0usize;
            for tc in tr.elements_mut() {
                if tc.name != "w:tc" {
                    continue;
                }
                if let Some(cell) = row.cells.get(c_idx)
                    && let Some(tcpr) = tc.child_mut("w:tcPr")
                {
                    if let Some(ins) = cell.props.ins {
                        let attrs = change_attr_vec(doc, ins, next_id);
                        tcpr.children.push(Node::El(El { name: "w:cellIns".into(), attrs, children: Vec::new() }));
                        changed = true;
                    } else if let Some(del) = cell.props.del {
                        let attrs = change_attr_vec(doc, del, next_id);
                        tcpr.children.push(Node::El(El { name: "w:cellDel".into(), attrs, children: Vec::new() }));
                        changed = true;
                    }
                }
                c_idx += 1;
            }
            r_idx += 1;
        }
        return changed;
    }
    let mut changed = false;
    for ch in el.elements_mut() {
        if patch_tables_el(ch, doc, tables, t_idx, next_id) {
            changed = true;
        }
    }
    changed
}

fn rewrite_table_xml(data: &[u8], doc: &Document) -> Option<Vec<u8>> {
    let tables: Vec<&wordcraft_doc::table::Table> = doc.body.iter().filter_map(|b| b.as_table()).collect();
    let has_any = tables.iter().any(|t| {
        t.rows.iter().any(|r| {
            r.props.ins.is_some()
                || r.props.del.is_some()
                || r.props.exceptions.as_ref().is_some_and(|x| x.fmt_change.is_some())
                || r.cells.iter().any(|c| c.props.ins.is_some() || c.props.del.is_some())
        })
    });
    if !has_any {
        return None;
    }
    let mut root = craft_xml::parse(data).ok()?;
    let mut t_idx = 0usize;
    let mut next_id = 100u32;
    if !patch_tables_el(&mut root, doc, &tables, &mut t_idx, &mut next_id) {
        return None;
    }
    let mut out = String::with_capacity(data.len() + 512);
    out.push_str(craft_xml::DECL_STANDALONE);
    root.write_xml(&mut out);
    Some(out.into_bytes())
}

pub(crate) fn apply_write_table_revs(doc: &Document, bytes: Vec<u8>) -> Vec<u8> {
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
            && let Some(rewritten) = rewrite_table_xml(&data, doc)
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
