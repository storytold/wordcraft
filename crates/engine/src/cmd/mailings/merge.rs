use super::{
    Block, Blocks, CmdError, CmdResult, Document, InlineObject, Paragraph, Part, PartKind, SectionProps, SectionStart, Session, StoryRef, Value,
    json, p, para_block,
};

pub(super) fn step(s: &mut Session, d: i64) -> CmdResult {
    go_to(s, (s.merge.record as i64).saturating_add(d))
}

/// Preview record `idx` (0-based), clamped to the list.
pub(super) fn go_to(s: &mut Session, idx: i64) -> CmdResult {
    let n = s.merge.rows.len().max(1) as i64;
    s.merge.record = idx.clamp(0, n - 1) as usize;
    s.merge.preview = true;
    refresh(s);
    Ok(json!({"record": s.merge.record + 1, "of": n}))
}

/// Field names used by MERGEFIELD fields in the document (body, headers and footers).
pub(super) fn merge_fields(doc: &Document) -> Vec<String> {
    let mut v = Vec::new();
    for (story, path) in merge_stories(doc).into_iter().flat_map(|st| doc.para_paths(st).into_iter().map(move |p| (st, p))) {
        if let Some(p) = doc.para(story, &path) {
            for o in &p.objects {
                if let InlineObject::Field { instr, .. } = o
                    && let Some(name) = mergefield_name(instr)
                    && !v.contains(&name)
                {
                    v.push(name);
                }
            }
        }
    }
    v
}

pub(super) fn mergefield_name(instr: &str) -> Option<String> {
    // The field type ignores case and ends at a space or a quote, so `MERGEFIELDX` is another field.
    let t = instr.trim();
    let (kind, rest) = t.split_at_checked(t.find(|c: char| c.is_whitespace() || c == '"').unwrap_or(t.len()))?;
    if !kind.eq_ignore_ascii_case("MERGEFIELD") {
        return None;
    }
    let rest = rest.trim();
    let name = if let Some(r) = rest.strip_prefix('"') { r.split('"').next().unwrap_or("") } else { rest.split_whitespace().next().unwrap_or("") };
    Some(name.to_string())
}

/// Which columns hold address parts.
pub(super) fn address_map(headers: &[String]) -> Value {
    let find = |names: &[&str]| headers.iter().find(|h| names.iter().any(|n| h.replace([' ', '_'], "").eq_ignore_ascii_case(n))).cloned();
    json!({
        "firstName": find(&["FirstName", "First", "GivenName"]),
        "lastName": find(&["LastName", "Last", "Surname", "FamilyName"]),
        "company": find(&["Company", "Organization", "CompanyName"]),
        "address": find(&["Address", "AddressLine1", "Address1", "Street"]),
        "city": find(&["City", "Town"]),
        "state": find(&["State", "Province", "Region"]),
        "postal": find(&["PostalCode", "ZIP", "ZipCode", "Postcode"]),
        "country": find(&["Country"]),
        "title": find(&["Title", "CourtesyTitle"]),
    })
}

fn value(s: &Session, row: usize, field: &str) -> String {
    let Some(i) = s.merge.headers.iter().position(|h| h.eq_ignore_ascii_case(field)) else { return String::new() };
    s.merge.rows.get(row).and_then(|r| r.get(i)).cloned().unwrap_or_default()
}

pub(super) fn address_block(s: &Session, row: usize) -> String {
    let m = address_map(&s.merge.headers);
    let g = |k: &str| m.get(k).and_then(Value::as_str).map(|f| value(s, row, f)).unwrap_or_default();
    let name = [g("title"), g("firstName"), g("lastName")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
    let city_line = [g("city"), [g("state"), g("postal")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ")]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    [name, g("company"), g("address"), city_line, g("country")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n")
}

fn greeting(s: &Session, row: usize) -> String {
    let m = address_map(&s.merge.headers);
    let g = |k: &str| m.get(k).and_then(Value::as_str).map(|f| value(s, row, f)).unwrap_or_default();
    let name = [g("firstName"), g("lastName")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
    if name.is_empty() { "Dear Sir or Madam,".into() } else { format!("Dear {name},") }
}

/// What a merge-related field does for the current record.
enum Act {
    /// Show this text.
    Text(String),
    /// NEXT / a true NEXTIF: later fields in this merged document use the next record.
    Next,
    /// A true SKIPIF: drop this merged document and go on with the next record.
    Skip,
}

/// The merge field a rule tests: the name in its first nested field, if that is a MERGEFIELD.
fn rule_field(t: &str) -> Option<String> {
    mergefield_name(t.split_once('{').and_then(|(_, r)| r.split('}').next())?)
}

/// Comparison of a rule condition `{ MERGEFIELD X } op "value" …` (IF, SKIPIF, NEXTIF) for
/// `row`: whether it holds, and the quoted texts after the value (IF's true / false text).
fn condition<'a>(s: &Session, t: &'a str, row: usize) -> (bool, Vec<&'a str>) {
    let field = rule_field(t).unwrap_or_default();
    let rest = t.split_once('}').map(|(_, r)| r).unwrap_or("");
    let op: String = rest.trim_start().chars().take_while(|c| matches!(c, '<'..='>')).collect();
    let mut quoted = rest.split('"').skip(1).step_by(2);
    let cmp = quoted.next().unwrap_or("");
    let v = value(s, row, &field);
    // Numbers compare as numbers (Word: "7" = "7.0"), everything else as text.
    let ord = match (v.trim().parse::<f64>(), cmp.trim().parse::<f64>()) {
        (Ok(a), Ok(b)) => a.partial_cmp(&b),
        _ => Some(v.as_str().cmp(cmp)),
    };
    use std::cmp::Ordering::{Equal, Greater, Less};
    let holds = match op.as_str() {
        "<>" => ord != Some(Equal),
        "<" => ord == Some(Less),
        ">" => ord == Some(Greater),
        "<=" => matches!(ord, Some(Less | Equal)),
        ">=" => matches!(ord, Some(Greater | Equal)),
        _ => ord == Some(Equal),
    };
    (holds, quoted.collect())
}

/// The first word of a field instruction, upper-cased.
fn keyword(instr: &str) -> String {
    instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase()
}

/// Evaluate a merge-related field for a record (None = not a merge field).
fn act(s: &Session, instr: &str, row: usize) -> Option<Act> {
    let t = instr.trim();
    if let Some(n) = mergefield_name(t) {
        return Some(Act::Text(value(s, row, &n)));
    }
    Some(match keyword(t).as_str() {
        "ADDRESSBLOCK" => Act::Text(address_block(s, row)),
        "GREETINGLINE" => Act::Text(greeting(s, row)),
        "MERGEREC" => Act::Text(row.saturating_add(1).to_string()),
        "NEXT" => Act::Next,
        "NEXTIF" if condition(s, t, row).0 => Act::Next,
        "SKIPIF" if condition(s, t, row).0 => Act::Skip,
        "NEXTIF" | "SKIPIF" => Act::Text(String::new()),
        // IF { MERGEFIELD X } = "v" "then" "else" (an IF on other fields isn't ours to touch).
        "IF" if rule_field(t).is_some() => {
            let (holds, texts) = condition(s, t, row);
            Act::Text(texts.get(if holds { 0 } else { 1 }).copied().unwrap_or("").to_string())
        }
        _ => return None,
    })
}

/// What a merge field shows outside Preview Results (None = not a merge field).
pub(super) fn placeholder(instr: &str) -> Option<String> {
    if let Some(n) = mergefield_name(instr) {
        return Some(format!("«{n}»"));
    }
    match keyword(instr).as_str() {
        "ADDRESSBLOCK" => Some("«AddressBlock»".into()),
        "GREETINGLINE" => Some("«GreetingLine»".into()),
        "NEXT" | "NEXTIF" | "SKIPIF" | "MERGEREC" => Some(String::new()),
        "IF" if rule_field(instr).is_some() => Some(String::new()),
        _ => None,
    }
}

/// The stories a merge resolves: the body, then every header and footer.
fn merge_stories(doc: &Document) -> Vec<StoryRef> {
    std::iter::once(StoryRef::Body).chain(header_footer_ids(doc).into_iter().map(StoryRef::Part)).collect()
}

fn header_footer_ids(doc: &Document) -> Vec<u32> {
    doc.parts.iter().filter(|(_, p)| matches!(p.kind, PartKind::Header | PartKind::Footer)).map(|(id, _)| *id).collect()
}

/// Update merge field results: record values when previewing, «Field» placeholders otherwise.
/// NEXT moves later fields of the previewed document to the next record, as Finish & Merge does.
pub fn refresh(s: &mut Session) {
    let preview = s.merge.preview && !s.merge.rows.is_empty();
    let last = s.merge.rows.len();
    for story in merge_stories(&s.doc) {
        let mut cur = s.merge.record;
        for path in s.doc.para_paths(story) {
            let updates: Vec<(usize, String)> = match s.doc.para(story, &path) {
                Some(p) => p
                    .objects
                    .iter()
                    .enumerate()
                    .filter_map(|(k, o)| {
                        let InlineObject::Field { instr, .. } = o else { return None };
                        let shown = placeholder(instr)?;
                        if !preview {
                            return Some((k, shown));
                        }
                        Some((
                            k,
                            match act(s, instr, cur) {
                                Some(Act::Text(t)) => t,
                                Some(Act::Next) => {
                                    cur = cur.saturating_add(1).min(last);
                                    String::new()
                                }
                                Some(Act::Skip) | None => String::new(),
                            },
                        ))
                    })
                    .collect(),
                None => continue,
            };
            if updates.is_empty() {
                continue;
            }
            if let Ok(p) = s.doc.para_mut(story, &path) {
                for (k, r) in updates {
                    if let Some(InlineObject::Field { result, .. }) = p.objects.get_mut(k) {
                        *result = r;
                    }
                }
                p.touch();
            }
        }
    }
    s.touch();
}

/// The records Finish & Merge and Print Documents take (0-based, end exclusive): `records`
/// all or current, or `from` and `to` by record number. A range with none is an error.
pub fn merge_range(s: &Session, v: &Value) -> Result<(usize, usize), CmdError> {
    let n = s.merge.rows.len();
    let (from, to) = match p::str(v, "records").map(str::trim) {
        Some("current") => (s.merge.record.min(n.saturating_sub(1)), s.merge.record.saturating_add(1).min(n)),
        None | Some("all") => {
            ((p::u64(v, "from").unwrap_or(1).max(1) as usize).saturating_sub(1), (p::u64(v, "to").unwrap_or(n as u64).min(n as u64)) as usize)
        }
        Some(_) => return Err(CmdError::Params("`records` must be all or current".into())),
    };
    if from >= to {
        return Err(CmdError::Failed(format!("no records from {} to {to} (the list has {n})", from.saturating_add(1))));
    }
    Ok((from, to))
}

/// Most merged documents' worth of body blocks Finish & Merge builds.
const MAX_MERGED_BLOCKS: usize = 200_000;

/// Build one merged copy per record: fields become plain text; records separated by page breaks
/// (letters) or new paragraphs (directory). A true SKIPIF drops the record's copy; NEXT moves the
/// rest of a copy to the next record, and the following copy starts after it. Headers and footers
/// are merged too: with merge fields in them, each letter becomes its own section with its own
/// header/footer copies.
pub(super) fn finish(s: &mut Session, v: &Value) -> CmdResult {
    let (from, to) = merge_range(s, v)?;
    let directory = s.merge.kind == "directory";
    let hf: Vec<u32> =
        header_footer_ids(&s.doc).into_iter().filter(|id| s.doc.story(StoryRef::Part(*id)).is_some_and(|b| has_merge_fields(b, 0))).collect();
    let per_record = !directory && !hf.is_empty();
    let mut out = s.doc.clone();
    out.body.clear();
    let mut next_id = s.doc.parts.keys().next_back().map_or(1, |k| k.saturating_add(1));
    let mut start = from;
    let mut count = 0usize;
    let mut first_record = None;
    // The section that ends the previous letter (per-record sections only).
    let mut prev_section: Option<SectionProps> = None;
    while start < to {
        let mut body = s.doc.body.clone();
        let record = start;
        let mut cur = start;
        let keep = merge_blocks(s, &mut body, &mut cur, to, 0);
        start = cur.saturating_add(1);
        if !keep {
            continue;
        }
        first_record.get_or_insert(record);
        if per_record {
            // This letter's own header/footer copies, merged with its record.
            let mut map = Vec::new();
            for id in &hf {
                let Some(part) = s.doc.parts.get(id) else { continue };
                if next_id == u32::MAX {
                    return Err(CmdError::Failed("too many headers and footers to merge".into()));
                }
                let mut blocks = part.blocks.clone();
                let mut c = record;
                merge_blocks(s, &mut blocks, &mut c, to, 0);
                out.parts.insert(next_id, Part { kind: part.kind, blocks, body: part.body });
                map.push((*id, next_id));
                next_id += 1;
            }
            let mut first = true;
            for b in &mut body {
                if let Block::Para(p) = std::sync::Arc::make_mut(b)
                    && let Some(sp) = p.section.as_deref_mut()
                {
                    remap_headers(sp, &map, first);
                    first = false;
                }
            }
            let mut last = s.doc.last_section.clone();
            remap_headers(&mut last, &map, first);
            if let Some(prev) = prev_section.replace(last) {
                if !matches!(out.body.last().map(|b| &**b), Some(Block::Para(_))) {
                    out.body.push(para_block(Paragraph::new()));
                }
                if let Some(Block::Para(p)) = out.body.last_mut().map(std::sync::Arc::make_mut) {
                    p.section = Some(Box::new(prev));
                }
            }
        } else if count > 0
            && !directory
            && let Some(Block::Para(p)) = out.body.last_mut().map(std::sync::Arc::make_mut)
        {
            let end = p.len();
            let props = p.props_at(end).clone();
            let _ = p.insert_text(end, "\u{000C}", &props);
        }
        out.body.extend(body);
        count += 1;
        if out.body.len() > MAX_MERGED_BLOCKS {
            break;
        }
    }
    if let Some(last) = prev_section {
        // The template's own header/footer stories are replaced by the letters' copies.
        for id in &hf {
            out.parts.remove(id);
        }
        out.last_section = last;
    } else {
        // One shared header/footer (directory, or no letters): merged with the first record.
        let row = first_record.unwrap_or(from);
        for id in &hf {
            if let Some(part) = out.parts.get_mut(id) {
                let mut c = row;
                merge_blocks(s, &mut part.blocks, &mut c, to, 0);
            }
        }
    }
    out.ensure_nonempty();
    if let Some(path) = p::str(v, "path") {
        crate::io::save_path(std::path::Path::new(path), &out).map_err(CmdError::Failed)?;
        return Ok(json!({"records": count, "path": path}));
    }
    s.add_document(out);
    s.dirty = true;
    Ok(json!({"records": count}))
}

/// Point a letter's section at its own header/footer copies; the letter's first section starts
/// on a new page.
fn remap_headers(sp: &mut SectionProps, map: &[(u32, u32)], first: bool) {
    for slot in
        [&mut sp.headers.default, &mut sp.headers.first, &mut sp.headers.even, &mut sp.footers.default, &mut sp.footers.first, &mut sp.footers.even]
    {
        if let Some(id) = slot
            && let Some((_, new)) = map.iter().find(|(old, _)| old == id)
        {
            *id = *new;
        }
    }
    if first {
        sp.start = SectionStart::NextPage;
    }
}

/// Nesting depth of tables the merge descends into.
const MAX_DEPTH: usize = 32;

fn has_merge_fields(blocks: &Blocks, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && blocks.iter().any(|b| match &**b {
            Block::Para(p) => p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if placeholder(instr).is_some())),
            Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).any(|c| has_merge_fields(&c.blocks, depth + 1)),
        })
}

/// Replace the merge fields in `blocks` by their text, in document order, starting with record
/// `*cur`. NEXT moves `*cur` on (records from `end` on are blank). Returns false when a SKIPIF
/// drops this merged document.
fn merge_blocks(s: &Session, blocks: &mut Blocks, cur: &mut usize, end: usize, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    for b in blocks.iter_mut() {
        match std::sync::Arc::make_mut(b) {
            Block::Para(p) => {
                let mut repl = Vec::new();
                for off in p.object_offsets() {
                    let Some(InlineObject::Field { instr, .. }) = p.object_at(off) else { continue };
                    let row = if *cur < end { *cur } else { s.merge.rows.len() };
                    match act(s, instr, row) {
                        Some(Act::Text(t)) => repl.push((off, t)),
                        Some(Act::Next) => {
                            *cur = cur.saturating_add(1).min(end);
                            repl.push((off, String::new()));
                        }
                        Some(Act::Skip) => return false,
                        None => {}
                    }
                }
                for (off, val) in repl.into_iter().rev() {
                    let props = p.props_of_char(off).clone();
                    let _ = p.delete(off, off + wordcraft_doc::para::OBJ.len_utf8());
                    let _ = p.insert_text(off, &val, &props);
                }
            }
            Block::Table(t) => {
                for c in t.rows.iter_mut().flat_map(|r| &mut r.cells) {
                    if !merge_blocks(s, &mut c.blocks, cur, end, depth + 1) {
                        return false;
                    }
                }
            }
        }
    }
    true
}
