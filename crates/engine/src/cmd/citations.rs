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

mod authorities;
mod cross_refs;
mod generated;
mod render;
mod sources;
mod styles;

use authorities::{insert_authorities, mark_citation};
use cross_refs::cross_ref;
pub use cross_refs::{caption_labels, ref_targets};
#[cfg(test)]
use generated::ENTRY_STYLE;
pub(crate) use generated::generated_list;
use generated::table_of_figures;
pub use generated::{figures_instr, update_generated};
pub use render::{cite, entry};
pub use sources::update_citations;
use sources::{citation, sources};
pub use styles::{Cite, CiteStyle, Name, Names, Order, STYLES, find_style, style};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("references.sources", "Manage Sources", "References › Citations & Bibliography", sources)
            .params(r#"{"add"?: Source, "remove"?: tag, "import"?: path (a Word bibliography file, `b:Sources` / `Sources.xml`)} → list"#),
        CommandSpec::new("references.citation", "Insert Citation", "References › Citations & Bibliography", citation).params(r#"{"tag"?: string, "source"?: Source (added if new), "pages"?: string}"#),
        CommandSpec::new("references.citationStyle", "Style", "References › Citations & Bibliography", |s, v| {
            let st = p::str(v, "style").unwrap_or("APA");
            let st = find_style(st).ok_or_else(|| {
                CmdError::Params(format!("style must be one of {:?}", STYLES.iter().map(|x| x.name).collect::<Vec<_>>()))
            })?;
            s.bib_style = st.name.to_string();
            // Recorded in the document too, so a save keeps it.
            s.doc.settings.bib_style = Some(st.name.to_string());
            update_citations(s)?;
            Ok(json!({"style": s.bib_style}))
        })
        .params(
            r#"{"style": "APA|Chicago|GB/T 7714|GOST (name sort)|GOST (title sort)|Harvard (Anglia)|IEEE|ISO 690 (author-date)|ISO 690 (numerical)|MLA|SIST02|Turabian" (Word's style names work too)}"#,
        ),
        CommandSpec::new("references.bibliography", "Bibliography", "References › Citations & Bibliography", |s, v| {
            let title = p::str(v, "title").unwrap_or(style(&s.bib_style).bib_title).to_string();
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
        CommandSpec::new("references.markCitation", "Mark Citation", "References › Table of Authorities", mark_citation)
            .params(
                r#"{"long"?: string (the citation as the table lists it; default: the selected text), "short"?: string (default: the long citation), "category"?: 1-16 (1 Cases, 2 Statutes, 3 Other Authorities, 4 Rules, 5 Treatises, 6 Regulations, 7 Constitutional Provisions; default 1), "all"?: bool (Mark All: also every other place the long or short citation appears, exactly as typed; at most 1000 marks, `capped` in the result says more were left), "entry"?: string (same as long)}"#,
            )
            .key(super::non_mac("Alt+Shift+I")),
        CommandSpec::new("references.tableOfAuthorities", "Insert Table of Authorities", "References › Table of Authorities", insert_authorities)
            .params(r#"{"category"?: 0-16 (0, the default: All, one table per category that has citations, as Word inserts them), "passim"?: bool (Use passim: five pages or more read "passim"; default true)}"#),
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
