//! Citation packs: versioned JSON rule sets (Missouri and federal ship with the crate).

use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Schema version for citation pack files.
pub const PACK_SCHEMA: u32 = 1;

const MAX_PACK_BYTES: usize = 512 * 1024;
const MAX_RULES: usize = 200;
const MAX_REGEX_LEN: usize = 800;
const MAX_MESSAGE_LEN: usize = 500;

/// One rule in a citation pack. `detect` finds a span; when `valid` is set, a finding is
/// reported only if the matched text does not match `valid`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PackRule {
    pub id: String,
    pub kind: String,
    pub source: String,
    pub detect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extend_tail: bool,
    /// When true, valid matches (see `valid`) are included in the table-of-authorities finder.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub finder: bool,
    pub message: String,
}

/// A citation pack referenced by court profiles (`citationPack`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CitationPack {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub jurisdiction: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effective: String,
    pub rules: Vec<PackRule>,
}

/// A compiled rule ready to run against document text.
pub struct CompiledRule {
    pub id: String,
    pub kind: String,
    pub source: String,
    pub message: String,
    pub detect: Regex,
    pub valid: Option<Regex>,
    /// When true, extend the match with an optional pin cite and parenthetical before `valid`.
    pub extend_tail: bool,
    pub finder: bool,
}

/// A loaded pack with compiled regexes.
pub struct CompiledPack {
    pub id: String,
    pub name: String,
    pub jurisdiction: String,
    pub rules: Vec<CompiledRule>,
}

impl CitationPack {
    pub fn from_json(text: &str) -> Result<Self, Vec<String>> {
        if text.len() > MAX_PACK_BYTES {
            return Err(vec![format!("citation pack is larger than {} bytes", MAX_PACK_BYTES)]);
        }
        let v: CitationPack = serde_json::from_str(text).map_err(|e| vec![e.to_string()])?;
        let problems = problems(&v);
        if problems.is_empty() { Ok(v) } else { Err(problems) }
    }

    pub fn compile(self) -> Result<CompiledPack, Vec<String>> {
        let mut rules = Vec::new();
        for r in self.rules {
            let detect = compile_regex(&r.detect, "detect")?;
            let valid = match &r.valid {
                Some(s) => Some(compile_regex(s, "valid")?),
                None => None,
            };
            rules.push(CompiledRule {
                id: r.id,
                kind: r.kind,
                source: r.source,
                message: r.message,
                detect,
                valid,
                extend_tail: r.extend_tail,
                finder: r.finder,
            });
        }
        Ok(CompiledPack { id: self.id, name: self.name, jurisdiction: self.jurisdiction, rules })
    }
}

fn compile_regex(pat: &str, label: &str) -> Result<Regex, Vec<String>> {
    if pat.len() > MAX_REGEX_LEN {
        return Err(vec![format!("`{label}` regex is too long (max {MAX_REGEX_LEN} characters)")]);
    }
    Regex::new(pat).map_err(|e| vec![format!("`{label}` regex is invalid: {e}")])
}

pub fn problems(p: &CitationPack) -> Vec<String> {
    let mut out = Vec::new();
    if p.schema != PACK_SCHEMA {
        out.push(format!("`schema` must be {PACK_SCHEMA} (got {})", p.schema));
    }
    check_id(&mut out, "`id`", &p.id);
    check_len(&mut out, "`name`", &p.name, 200);
    check_len(&mut out, "`jurisdiction`", &p.jurisdiction, 200);
    if p.rules.len() > MAX_RULES {
        out.push(format!("too many rules (max {MAX_RULES})"));
    }
    for (i, r) in p.rules.iter().enumerate() {
        let who = format!("rules[{i}]");
        check_id(&mut out, &format!("{who}.id"), &r.id);
        check_len(&mut out, &format!("{who}.kind"), &r.kind, 64);
        check_len(&mut out, &format!("{who}.source"), &r.source, 300);
        if r.detect.is_empty() {
            out.push(format!("{who}.detect is required"));
        } else if r.detect.len() > MAX_REGEX_LEN {
            out.push(format!("{who}.detect is too long"));
        }
        if let Some(v) = &r.valid
            && v.len() > MAX_REGEX_LEN
        {
            out.push(format!("{who}.valid is too long"));
        }
        if r.message.is_empty() {
            out.push(format!("{who}.message is required"));
        } else if r.message.len() > MAX_MESSAGE_LEN {
            out.push(format!("{who}.message is too long"));
        }
    }
    out
}

fn check_id(out: &mut Vec<String>, field: &str, id: &str) {
    if id.is_empty() {
        out.push(format!("{field} is required"));
        return;
    }
    if id.len() > 64 {
        out.push(format!("{field} is too long (max 64)"));
    }
    if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.') {
        out.push(format!("{field} has invalid characters"));
    }
}

fn check_len(out: &mut Vec<String>, field: &str, s: &str, max: usize) {
    if s.is_empty() {
        out.push(format!("{field} is required"));
    } else if s.len() > max {
        out.push(format!("{field} is too long (max {max})"));
    }
}

/// Citation packs bundled with LawCraft.
pub fn bundled() -> Vec<CompiledPack> {
    [include_str!("../packs/missouri.json"), include_str!("../packs/federal.json")]
        .iter()
        .filter_map(|text| match CitationPack::from_json(text) {
            Ok(p) => match p.compile() {
                Ok(c) => Some(c),
                Err(e) => {
                    log::error!("bundled citation pack failed to compile: {}", e.join("; "));
                    None
                }
            },
            Err(e) => {
                log::error!("bundled citation pack is not valid: {}", e.join("; "));
                None
            }
        })
        .collect()
}

/// Lookup table for bundled packs.
pub fn bundled_map() -> BTreeMap<String, CompiledPack> {
    bundled().into_iter().map(|p| (p.id.clone(), p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_packs_are_valid() {
        let all = bundled();
        assert_eq!(all.len(), 2, "Missouri and federal");
        assert!(all.iter().any(|p| p.id == "missouri"));
        assert!(all.iter().any(|p| p.id == "federal"));
    }
}
