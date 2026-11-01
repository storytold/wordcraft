//! Page-label metadata in standard custom document properties (§22.3), referenced by short
//! content-control tags (§17.5.2.38). Word can discard ignorable attributes on save; these
//! identities follow the header/footer instead of depending on a physical page/section index.
use std::collections::{BTreeMap, HashMap, HashSet};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use wordcraft_doc::{CustomProp, page_label::PageLabel};

use crate::xml::{El, MAX_DEPTH};

const PREFIX: &str = "WordCraft.PageLabel1.";
const MAX_PROPERTIES: usize = 10_000;
const MAX_RECORD_BYTES: usize = 32_768;
const MAX_CHUNKS: usize = 128;
// Stay below Word's legacy string-property limit, counting UTF-16 code units.
const CHUNK_UNITS: usize = 240;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub label: PageLabel,
    pub instr: String,
    pub content_hash: Option<String>,
}

/// Hash literal labels and field structure/codes, not formatting, run boundaries or cached results.
pub(crate) fn content_hash(content: &El) -> Option<String> {
    fn walk(e: &El, depth: usize, fields: &mut Vec<bool>, hash: &mut Sha256) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        match e.name.as_str() {
            "w:fldChar" => {
                let kind = e.attr("w:fldCharType")?;
                if !fields.iter().any(|result| *result) || kind == "end" {
                    hash.update([0]);
                    hash.update(kind.as_bytes());
                    hash.update(e.attr("w:fldLock").unwrap_or("0").as_bytes());
                    hash.update([0]);
                }
                match kind {
                    "begin" => {
                        if fields.len() >= MAX_DEPTH {
                            return None;
                        }
                        fields.push(false);
                    }
                    "separate" => *fields.last_mut()? = true,
                    "end" => {
                        fields.pop()?;
                    }
                    _ => return None,
                }
            }
            "w:t" | "w:delText" | "w:instrText" | "w:delInstrText" if !fields.iter().any(|result| *result) => hash.update(e.text().as_bytes()),
            "w:tab" | "w:br" | "w:fldSimple" if !fields.iter().any(|result| *result) => {
                hash.update([0]);
                hash.update(e.name.as_bytes());
                hash.update(e.attr("w:instr").unwrap_or("").as_bytes());
                hash.update([0]);
            }
            _ => {}
        }
        for child in e.els() {
            walk(child, depth + 1, fields, hash)?;
        }
        Some(())
    }
    let mut hash = Sha256::new();
    let mut fields = Vec::new();
    walk(content, 0, &mut fields, &mut hash)?;
    fields.is_empty().then(|| URL_SAFE_NO_PAD.encode(hash.finalize()))
}

fn key(json: &str) -> String {
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(Sha256::digest(json.as_bytes())))
}

fn valid_instr(instr: &str) -> bool {
    instr.len() <= 8192 && instr.split_whitespace().next().is_some_and(|s| s.eq_ignore_ascii_case("PAGE"))
}

#[derive(Default)]
pub(crate) struct Export {
    reserved: HashSet<String>,
    records: BTreeMap<String, Vec<CustomProp>>,
    count: usize,
}

impl Export {
    pub fn new(existing: &[CustomProp]) -> Self {
        Self { reserved: existing.iter().map(|p| p.name.to_ascii_lowercase()).collect(), count: existing.len(), ..Default::default() }
    }

    pub fn insert(&mut self, label: &PageLabel, instr: &str, content_hash: Option<String>) -> Option<String> {
        if label.validate().is_err() || !valid_instr(instr) {
            return None;
        }
        let json = serde_json::to_string(&serde_json::json!({"label":label,"instr":instr,"contentHash":content_hash})).ok()?;
        if json.len() > MAX_RECORD_BYTES {
            return None;
        }
        let tag = key(&json);
        if self.records.contains_key(&tag) {
            return Some(tag);
        }
        let mut chunks = vec![String::new()];
        let mut units = 0;
        for c in json.chars() {
            if units + c.len_utf16() > CHUNK_UNITS {
                chunks.push(String::new());
                units = 0;
            }
            if let Some(chunk) = chunks.last_mut() {
                chunk.push(c);
            }
            units += c.len_utf16();
        }
        if chunks.len() > MAX_CHUNKS || self.count.saturating_add(chunks.len() + 1) > MAX_PROPERTIES {
            return None;
        }
        let property = |name: String, value: String| CustomProp { name, value, kind: "lpwstr".into(), ..Default::default() };
        let mut props = vec![property(tag.clone(), chunks.len().to_string())];
        props.extend(chunks.into_iter().enumerate().map(|(i, value)| property(format!("{tag}.{i}"), value)));
        if props.iter().any(|p| self.reserved.contains(&p.name.to_ascii_lowercase())) {
            return None;
        }
        self.count += props.len();
        self.records.insert(tag.clone(), props);
        Some(tag)
    }

    pub fn properties(&self) -> impl Iterator<Item = &CustomProp> {
        self.records.values().flatten()
    }
}

#[derive(Default)]
pub(crate) struct Import {
    records: HashMap<String, Record>,
    owned: HashSet<String>,
}

impl Import {
    pub fn read(props: &[CustomProp]) -> Self {
        let mut by_name = HashMap::new();
        let mut duplicate = HashSet::new();
        for p in props.iter().take(MAX_PROPERTIES) {
            if by_name.insert(p.name.as_str(), p).is_some() {
                duplicate.insert(p.name.as_str());
            }
        }
        let mut out = Self::default();
        for p in props.iter().take(MAX_PROPERTIES) {
            if !p.name.starts_with(PREFIX) || p.name.len() != PREFIX.len() + 43 || duplicate.contains(p.name.as_str()) {
                continue;
            }
            let Some(count) = p.value.parse::<usize>().ok().filter(|n| (1..=MAX_CHUNKS).contains(n)) else { continue };
            let mut json = String::new();
            let mut names = vec![p.name.clone()];
            for i in 0..count {
                let name = format!("{}.{i}", p.name);
                let Some(chunk) = by_name.get(name.as_str()).filter(|c| c.kind == "lpwstr" && !duplicate.contains(name.as_str())) else { break };
                if chunk.value.len() > CHUNK_UNITS * 4 || json.len().saturating_add(chunk.value.len()) > MAX_RECORD_BYTES {
                    break;
                }
                json.push_str(&chunk.value);
                names.push(name);
            }
            if names.len() != count + 1 || key(&json) != p.name {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else { continue };
            let Some(instr) = value.get("instr").and_then(|v| v.as_str()).filter(|i| valid_instr(i)) else { continue };
            let Some(label) = value.get("label").and_then(|v| serde_json::from_value::<PageLabel>(v.clone()).ok()).filter(|l| l.validate().is_ok())
            else {
                continue;
            };
            let content_hash = value.get("contentHash").and_then(|v| v.as_str()).map(str::to_owned);
            out.records.insert(p.name.clone(), Record { label, instr: instr.into(), content_hash });
            out.owned.extend(names);
        }
        out
    }

    pub fn owns(&self, name: &str) -> bool {
        self.owned.contains(name)
    }

    pub fn field(&self, control: &El) -> Option<&Record> {
        let tag = control.child("w:sdtPr")?.child_val("w:tag")?;
        self.records.get(tag)
    }

    pub fn content_matches(&self, control: &El) -> bool {
        self.field(control)
            .and_then(|record| record.content_hash.as_ref())
            .is_none_or(|expected| control.child("w:sdtContent").and_then(content_hash).as_ref() == Some(expected))
    }

    pub fn has_edits(&self, root: &El, depth: usize) -> bool {
        depth > MAX_DEPTH || (root.name == "w:sdt" && !self.content_matches(root)) || root.els().any(|child| self.has_edits(child, depth + 1))
    }

    /// A story may contain several PAGE fields. Conflicting records are not guessed at.
    pub fn story_label(&self, root: &El) -> Option<PageLabel> {
        fn walk<'a>(store: &'a Import, e: &El, depth: usize, label: &mut Option<&'a PageLabel>, conflict: &mut bool) {
            if depth > MAX_DEPTH || *conflict {
                return;
            }
            if e.name == "w:sdt"
                && let Some(record) = store.field(e)
            {
                if label.is_some_and(|l| *l != record.label) {
                    *conflict = true;
                    return;
                }
                *label = Some(&record.label);
            }
            for child in e.els() {
                walk(store, child, depth + 1, label, conflict);
            }
        }
        let (mut label, mut conflict) = (None, false);
        walk(self, root, 0, &mut label, &mut conflict);
        if conflict { None } else { label.cloned() }
    }
}

/// New sections added in another editor can inherit their labelled header/footer without
/// naming it again. Resolve that link after all section breaks have been read.
pub(crate) fn restore_inherited_sections(doc: &mut wordcraft_doc::Document, labels: &HashMap<u32, PageLabel>) {
    use wordcraft_doc::section::HeaderSet;
    let (mut headers, mut footers) = (HeaderSet::default(), HeaderSet::default());
    let mut updates = Vec::new();
    for (end, sec) in doc.sections() {
        for (effective, own) in [(&mut headers, sec.headers), (&mut footers, sec.footers)] {
            effective.default = own.default.or(effective.default);
            effective.first = own.first.or(effective.first);
            effective.even = own.even.or(effective.even);
        }
        if sec.page_label.is_some() {
            continue;
        }
        let ids = [
            headers.default,
            footers.default,
            sec.title_page.then_some(headers.first).flatten(),
            sec.title_page.then_some(footers.first).flatten(),
            doc.settings.even_odd_headers.then_some(headers.even).flatten(),
            doc.settings.even_odd_headers.then_some(footers.even).flatten(),
        ];
        let mut found = ids.into_iter().flatten().filter_map(|id| labels.get(&id));
        if let Some(label) = found.next()
            && found.all(|l| l == label)
        {
            updates.push((end, label.clone()));
        }
    }
    for (end, label) in updates {
        doc.section_mut(end).page_label = Some(label);
    }
}

#[cfg(test)]
mod tests;
