//! References: citations and bibliography (APA, MLA, Chicago, IEEE, Harvard, ISO 690, Turabian,
//! GB/T 7714, GOST, SIST02), source manager, index, table of figures, cross-references, table
//! of authorities, footnote options.

use serde_json::{Value, json};
use wordcraft_doc::collate::{SortKey, heading_letter, sort_key};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::{Block, Document, Paragraph, Path, Pos, Source, StoryRef};

use super::references::FieldCode;
use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

mod cross_refs;
mod csl_styles;
mod generated;
mod render;
mod sources;
mod styles;

use cross_refs::cross_ref;
pub use cross_refs::{caption_labels, ref_targets};
pub use csl_styles::CslStyleEntry;
use csl_styles::{csl_styles, find_csl_entry, session_style};
#[cfg(test)]
use generated::ENTRY_STYLE;
pub(crate) use generated::generated_list;
use generated::table_of_figures;
pub use generated::{figures_instr, update_generated};
pub use render::{cite, entry};
pub use sources::update_citations;
use sources::{citation, import_library, sources};
pub use styles::{Cite, CiteStyle, Name, Names, Order, STYLES, find_style, style};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("references.sources", "Manage Sources", "References › Citations & Bibliography", sources)
            .params(r#"{"add"?: Source, "remove"?: tag, "import"?: path (a Word bibliography file, `b:Sources` / `Sources.xml`)} → list"#),
        CommandSpec::new("references.importSources", "Import Sources Library", "References › Citations & Bibliography", import_library).params(
            r#"{"path": string, "format"?: "bibtex|ris|csl-json|auto" (default: the extension, else the content)} → {"added": n, "updated": n, "skipped": [{"tag"?, "reason"}], "total": n} (entries the file held)"#,
        ),
        CommandSpec::new("references.cslStyles", "Citation Styles (CSL)", "References › Citations & Bibliography", csl_styles).params(
            r#"{} → {"styles": [{"id", "title", "file"}]}; {"add": path} → {"id", "warnings": […]} (the file is copied into the styles folder; one that doesn't parse is a parameter error); {"remove": id} → {"removed": true}"#,
        ),
        CommandSpec::new("references.citation", "Insert Citation", "References › Citations & Bibliography", citation).params(r#"{"tag"?: string, "source"?: Source (added if new), "pages"?: string}"#),
        CommandSpec::new("references.citationStyle", "Style", "References › Citations & Bibliography", |s, v| {
            let asked = p::str(v, "style").unwrap_or("APA");
            // A built-in (our name, or Word's style-sheet name), then the installed CSL
            // styles (`csl:<id>` or the style's title); a `csl:` style that is gone says so
            // rather than falling back, and any other unknown name is the usual list.
            let stored = if let Some(st) = find_style(asked) {
                st.name.to_string()
            } else if let Some(id) = find_csl_entry(s, asked).map(|e| e.id().to_string()) {
                format!("csl:{id}")
            } else if let Some(id) = asked.trim().strip_prefix("csl:") {
                return Err(CmdError::Params(format!("CSL style {id} is not installed; re-add it in References ▸ Style")));
            } else {
                return Err(CmdError::Params(format!(
                    "style must be one of {:?}, or a Citation Styles (CSL) style",
                    STYLES.iter().map(|x| x.name).collect::<Vec<_>>()
                )));
            };
            s.bib_style = stored.clone();
            // Recorded in the document too, so a save keeps it.
            s.doc.settings.bib_style = Some(stored);
            update_citations(s)?;
            Ok(json!({"style": s.bib_style}))
        })
        .params(
            r#"{"style": "APA|Chicago|GB/T 7714|GOST (name sort)|GOST (title sort)|Harvard (Anglia)|IEEE|ISO 690 (author-date)|ISO 690 (numerical)|MLA|SIST02|Turabian" (Word's style names work too), or a CSL style: "csl:<id>" or its title}"#,
        ),
        CommandSpec::new("references.bibliography", "Bibliography", "References › Citations & Bibliography", |s, v| {
            let title = match p::str(v, "title") {
                Some(t) => t.to_string(),
                // The style's own heading; CSL styles don't name theirs, so "Bibliography".
                None => session_style(s)?.bib_title(),
            };
            generated_list(s, "BIBLIOGRAPHY", &title)?;
            sel_result(s)
        })
        .params(r#"{"title"?: string}"#),
        CommandSpec::new("references.tableOfFigures", "Insert Table of Figures", "References › Captions", table_of_figures).params(
            r#"{"label"?: "Figure|Table|Equation" or a caption label of your own, "includeLabel"?: bool, "pageNumbers"?: bool, "rightAlign"?: bool, "tabLeader"?: "dot|hyphen|underscore|none"}"#,
        ),
        CommandSpec::new("references.updateFigures", "Update Table", "References › Captions", |s, _| {
            update_generated(s)?;
            sel_result(s)
        }),
        CommandSpec::new("references.markCitation", "Mark Citation", "References › Table of Authorities", |s, v| {
            let entry = p::str(v, "entry").map(str::to_string).unwrap_or_else(|| s.selected_text().trim().to_string());
            if entry.is_empty() {
                return Err(CmdError::Params("select a citation or give `entry`".into()));
            }
            let (_, b) = s.sel.ordered();
            s.doc.insert_object(&b, InlineObject::Field { instr: format!("TA \\l \"{}\"", entry.replace('"', "")), result: String::new(), locked: false, dirty: false, code: false, form: String::new() }, &CharProps { hidden: Some(true), ..Default::default() })?;
            Ok(json!({"entry": entry}))
        })
        .params(r#"{"entry"?: string}"#)
        .key(super::non_mac("Alt+Shift+I")),
        CommandSpec::new("references.tableOfAuthorities", "Insert Table of Authorities", "References › Table of Authorities", |s, _| {
            generated_list(s, "TOA", "Table of Authorities")?;
            sel_result(s)
        }),
        CommandSpec::new("references.updateAuthorities", "Update Table", "References › Table of Authorities", |s, _| {
            update_generated(s)?;
            sel_result(s)
        }),
        CommandSpec::new("insert.crossReference", "Cross-reference", "Insert › Links", cross_ref)
            .params(r#"{"to": "heading|bookmark|figure|table|equation|label:<Name> (a caption label of your own)", "target": string (text / name / number), "show"?: "text|page|number|aboveBelow"} or {} to list targets"#),
        CommandSpec::new("references.noteOptions", "Footnote and Endnote", "References › Footnotes", super::references::note_options).params(
            r#"{"kind"?: "footnote|endnote", "pos"?: "pageBottom|beneathText" (footnotes) | "sectEnd|docEnd" (endnotes), "numFmt"?: "decimal|lowerRoman|upperRoman|lowerLetter|upperLetter|…", "numStart"?: 1-32767, "numRestart"?: "continuous|eachSect|eachPage" (each page: footnotes only), "scope"?: "document|section", "footnoteFormat"?: …, "endnoteFormat"?: …} — no settings: the options in effect at the caret"#,
        ),
        CommandSpec::new("references.researcher", "Researcher", "References › Research", |s, _| {
            s.status = "Researcher needs an online service; WordCraft keeps your documents offline.".into();
            Ok(json!({"available": false}))
        })
        .pure(),
    ]
}

#[cfg(test)]
mod tests;
