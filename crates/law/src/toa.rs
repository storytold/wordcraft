//! Table of authorities from the citation finder. LawCraft does the part Word leaves to the
//! person: it finds the brief's citations and marks each one with Word's own citation mark (a
//! hidden `TA` field, as References › Mark Citation inserts it) in the category its kind belongs
//! to. The table itself is WordCraft's: one `TOA` field per category, built and updated by
//! References › Table of Authorities, so Update Table keeps it current and Word reads it too.

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Paragraph, Path, Pos, StoryRef};
use wordcraft_engine::cmd::citations::{ENTRY_STYLE, ta_field};
use wordcraft_engine::{CmdError, Selection, Session, p};

use crate::checks;
use crate::citations::{CitationEntry, find_entries, group_heading};
use crate::profile::Profile;

/// Word's table-of-authorities categories for the authorities the finder knows, in the order a
/// brief lists them.
const CATEGORIES: [(u8, &str); 5] = [(1, "Cases"), (7, "Constitutional Provisions"), (2, "Statutes"), (4, "Rules"), (3, "Other Authorities")];

/// The Word category (TA `\c`) for a finder kind.
fn category(kind: &str) -> u8 {
    let heading = group_heading(kind);
    CATEGORIES.iter().find(|(_, h)| *h == heading).map_or(3, |(c, _)| *c)
}

/// `law.toa.build`: mark the citations the finder accepts, then insert the table (at the profile's
/// Table of Authorities section, or the caret) or update the one already there.
pub fn build(s: &mut Session, v: &Value) -> Result<Value, CmdError> {
    let pack_id = pack_id(s, v)?;
    let pack = crate::cmd::pack_by_id(&pack_id)?;
    let (profile, doc_type) = applied_profile(s, v);
    let spec = profile.as_ref().and_then(|p| doc_type.as_ref().and_then(|d| p.docs.get(d)));
    let passim = p::bool(v, "passim").unwrap_or_else(|| spec.and_then(|d| d.toa_passim).unwrap_or(true));
    let at_cursor = match p::str(v, "at") {
        None => false,
        Some("cursor") => true,
        Some(other) => return Err(CmdError::Params(format!("`at` is \"cursor\" or left out, not `{other}`"))),
    };

    let entries = find_entries(&s.doc, pack)?;
    let marked = mark(s, &entries)?;
    // The marks change the text the layout measured: lay it out again before the table.
    s.touch();

    let existing = table_categories(s);
    let used: Vec<u8> = CATEGORIES.iter().map(|(c, _)| *c).filter(|c| entries.iter().any(|e| category(&e.kind) == *c)).collect();
    let inserted = if existing.is_empty() {
        if used.is_empty() {
            Vec::new()
        } else {
            let at = if at_cursor {
                caret_block(s)?
            } else if let Some(i) = spec.and_then(|d| checks::section_body_index(&s.doc, d, "tableOfAuthorities")) {
                i + 1
            } else {
                caret_block(s)?
            };
            insert_tables(s, at, &used, passim)?;
            used
        }
    } else {
        // A category the table doesn't have yet (the brief's first statute, say) gets its own
        // table after the last one; then every table is brought up to date.
        let have: Vec<u8> = existing.iter().filter_map(|instr| field_category(instr)).collect();
        let missing: Vec<u8> = used.iter().copied().filter(|c| !have.contains(c)).collect();
        if !missing.is_empty()
            && let Some(at) = after_last_table(s)
        {
            insert_tables(s, at, &missing, passim)?;
        }
        s.run("references.updateAuthorities", &json!({}))?;
        missing
    };
    s.touch();
    Ok(json!({
        "pack": pack_id,
        "authorities": entries.len(),
        "marked": marked,
        "inserted": inserted.iter().map(|c| json!(c)).collect::<Vec<_>>(),
        "updated": !existing.is_empty(),
    }))
}

/// Mark every occurrence the finder accepted that isn't marked yet: a hidden `TA` field right
/// after the citation, in its kind's category. Later occurrences go first, so the earlier ones'
/// offsets stay put. Returns how many marks were added.
fn mark(s: &mut Session, entries: &[CitationEntry]) -> Result<usize, CmdError> {
    let mut occ: Vec<(&CitationEntry, Pos)> = entries.iter().flat_map(|e| e.occurrences.iter().map(move |o| (e, o.end.clone()))).collect();
    occ.sort_by(|a, b| (b.1.path.0.as_slice(), b.1.off).cmp(&(a.1.path.0.as_slice(), a.1.off)));
    let mut added = 0;
    for (entry, end) in occ {
        let already = s.doc.para(end.story, &end.path).and_then(|p| p.object_at(end.off)).is_some_and(is_ta);
        if already {
            continue;
        }
        s.doc.insert_object(&end, ta_field(&entry.label, Some(category(&entry.kind))), &CharProps { hidden: Some(true), ..Default::default() })?;
        added += 1;
    }
    Ok(added)
}

fn is_ta(o: &InlineObject) -> bool {
    matches!(o, InlineObject::Field { instr, .. } if instr.trim_start().starts_with("TA "))
}

/// The categories of the tables of authorities already in the body (empty: none yet).
fn table_categories(s: &Session) -> Vec<String> {
    let mut out = Vec::new();
    for b in &s.doc.body {
        let Some(p) = b.as_para() else { continue };
        for o in &p.objects {
            if let InlineObject::Field { instr, .. } = o
                && instr.trim_start().starts_with("TOA")
            {
                out.push(instr.trim().to_string());
            }
        }
    }
    out
}

/// The category a `TOA` field lists (`\c "2"` → 2).
fn field_category(instr: &str) -> Option<u8> {
    let rest = instr.split("\\c").nth(1)?;
    rest.trim_start().trim_start_matches('"').chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()
}

/// The top-level block just after the last table of authorities' entries.
fn after_last_table(s: &Session) -> Option<usize> {
    let last = s.doc.body.iter().rposition(|b| {
        b.as_para().is_some_and(|p| p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if instr.trim_start().starts_with("TOA"))))
    })?;
    let mut at = last + 1;
    while s.doc.body.get(at).and_then(|b| b.as_para()).is_some_and(|p| p.props.style.as_deref() == Some(ENTRY_STYLE)) {
        at += 1;
    }
    Some(at)
}

/// Insert one table of authorities per category at top-level block `at`, in order. They are
/// built in a blank paragraph that moves down as each list goes in before it, and is removed
/// at the end.
fn insert_tables(s: &mut Session, at: usize, categories: &[u8], passim: bool) -> Result<(), CmdError> {
    s.doc.insert_block(StoryRef::Body, &Path::top(at), Block::Para(Paragraph::new()))?;
    let mut blank = at;
    for c in categories {
        let title = CATEGORIES.iter().find(|(k, _)| k == c).map_or("Other Authorities", |(_, t)| t);
        s.sel = Selection::caret(Pos { story: StoryRef::Body, path: Path::top(blank), off: 0 });
        s.run("references.tableOfAuthorities", &json!({"category": c, "passim": passim, "title": title}))?;
        // The list's heading took the blank's place; its entries follow it.
        blank += 1;
        while s.doc.body.get(blank).and_then(|b| b.as_para()).is_some_and(|p| p.props.style.as_deref() == Some(ENTRY_STYLE)) {
            blank += 1;
        }
    }
    if s.doc.body.get(blank).and_then(|b| b.as_para()).is_some_and(|p| p.is_empty()) {
        s.doc.remove_block(StoryRef::Body, &Path::top(blank))?;
    }
    Ok(())
}

/// The top-level body block holding the caret.
fn caret_block(s: &Session) -> Result<usize, CmdError> {
    let focus = &s.sel.focus;
    if focus.story != StoryRef::Body {
        return Err(CmdError::Disabled("place the caret in the document body".into()));
    }
    focus.path.0.first().map(|b| *b as usize).ok_or_else(|| CmdError::Failed("caret path is invalid".into()))
}

fn pack_id(s: &Session, v: &Value) -> Result<String, CmdError> {
    if let Some(id) = p::str(v, "pack") {
        return Ok(id.to_string());
    }
    let profile_id = s
        .doc
        .custom_prop(checks::PROP_PROFILE)
        .ok_or_else(|| CmdError::Params("`pack` (citation pack id) is required when no court profile is applied".into()))?;
    let store = crate::cmd::store();
    let profile = store.get(profile_id).ok_or_else(|| CmdError::Failed(format!("no court profile `{profile_id}`")))?;
    profile.citation_pack.clone().ok_or_else(|| CmdError::Params(format!("court profile `{profile_id}` has no `citationPack`")))
}

fn applied_profile(s: &Session, v: &Value) -> (Option<Profile>, Option<String>) {
    let profile_id = p::str(v, "id").map(str::to_string).or_else(|| s.doc.custom_prop(checks::PROP_PROFILE).map(str::to_string));
    let doc_type = p::str(v, "doc").map(str::to_string).or_else(|| s.doc.custom_prop(checks::PROP_DOC_TYPE).map(str::to_string));
    let profile = profile_id.and_then(|id| crate::cmd::store().get(&id).cloned());
    (profile, doc_type)
}
