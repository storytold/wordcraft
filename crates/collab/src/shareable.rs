//! The list of commands an admitted peer may replay into a document.
//!
//! This is the feature's security boundary: `CollabHost` dispatches a peer's
//! command through the app's own command bus (`Session::run`), which is the
//! same door the user's click, the CLI, MCP and the control port use. So the
//! question is not "which commands look dangerous" but "which commands may a
//! stranger run in my document", and the answer is an **allowlist by id**.
//!
//! The rule, and it is deliberate that it is inconvenient:
//!
//! - explicit ids only — no prefix wildcard, no family match;
//! - a command added upstream tomorrow is **refused** until someone adds it
//!   here on purpose, with a reason;
//! - everything a peer sends that is not on this list is refused and counted
//!   ([`crate::Collab::refused`]), because a refusal the person cannot see
//!   looks like a lost edit.
//!
//! Two lists, and every registered id of the families is in
//! exactly one of them — [`tests::every_id_in_the_document_families_is_accounted_for`]
//! computes that from the registry rather than trusting the text here.
//!
//! The families are the ground the list is enumerated from; the ids
//! outside them (`file.`, `tools.`, `mailings.`, `view.`, `caret.`,
//! `select.`, `edit.`, `protect.`, `options.`, `inspector.`, `references.`,
//! `fields.`, `developer.`, `readAloud.`, `speech.` and the tracked-change
//! half of `review.`) are never candidates.

/// The command ids a peer may replay: the document model and the person's
/// text, and nothing that reads a path,embeds foreign bytes, opens a dialog,
/// answers a query or moves the local view.
pub const SHAREABLE: &[&str] = &[
    "arrange.align", "arrange.bounds", "arrange.bringForward",
    "arrange.copyTo", "arrange.group", "arrange.moveWithText",
    "arrange.nudge", "arrange.position", "arrange.rotate",
    "arrange.rotation", "arrange.sendBackward", "arrange.ungroup",
    "arrange.wrap", "arrange.wrapText", "equation.backspace",
    "equation.convert", "equation.delete", "equation.deleteSurrounding",
    "equation.display", "equation.edit", "equation.enter",
    "equation.exit", "equation.inputFormat", "equation.insertStructure",
    "equation.insertSymbol", "equation.justify", "equation.move",
    "equation.normalText", "equation.number", "equation.set",
    "equation.structure", "equation.structureActions", "equation.type",
    "format.allCaps", "format.bold", "format.border",
    "format.changeCase", "format.charStyle", "format.clear",
    "format.color", "format.doubleStrikethrough", "format.doubleUnderline",
    "format.emboss", "format.enclose", "format.engrave",
    "format.font", "format.growFont", "format.growFont1",
    "format.hidden", "format.highlight", "format.italic",
    "format.outline", "format.phonetic", "format.position",
    "format.resetChar", "format.scale", "format.set",
    "format.shading", "format.shadow", "format.shrinkFont",
    "format.shrinkFont1", "format.size", "format.smallCaps",
    "format.spacing", "format.strikethrough", "format.subscript",
    "format.superscript", "format.textEffects", "format.underline",
    "format.underlineColor", "format.wordUnderline", "insert.autoText",
    "insert.blankPage", "insert.bookmark", "insert.canvas",
    "insert.closeHeader", "insert.coverPage", "insert.createFooter",
    "insert.createHeader", "insert.dateTime", "insert.deleteBookmark",
    "insert.dropCap", "insert.editFooter", "insert.editHeader",
    "insert.equation", "insert.footer", "insert.header",
    "insert.horizontalLine", "insert.icon", "insert.link",
    "insert.pageBreak", "insert.pageNumber", "insert.quickParts",
    "insert.removeEmptyHeaders", "insert.removeFooter", "insert.removeHeader",
    "insert.removeLink", "insert.shape", "insert.signatureLine",
    "insert.symbol", "insert.table", "insert.textBox",
    "insert.wordArt", "layout.break", "layout.differentFirstPage",
    "layout.differentOddEven", "layout.documentGrid", "layout.hyphenation",
    "layout.lineNumbers", "layout.linkToPrevious", "layout.margins",
    "layout.orientation", "layout.section", "layout.sectionRtl",
    "layout.size", "layout.textDirection", "layout.verticalAlign",
    "outline.demote", "outline.demoteToBody", "outline.level",
    "outline.moveDown", "outline.moveUp", "outline.promote",
    "outline.promoteToHeading1", "para.addSpaceBefore", "para.align",
    "para.alignCenter", "para.alignLeft", "para.alignRight",
    "para.asianTypography", "para.borders", "para.bullets",
    "para.defineBullet", "para.defineNumber", "para.demote",
    "para.distribute", "para.double", "para.hangingIndent",
    "para.heading1", "para.heading2", "para.heading3",
    "para.indent", "para.indents", "para.justify",
    "para.keepLines", "para.keepNext", "para.lineSpacing",
    "para.listLevel", "para.ltr", "para.moveDown",
    "para.moveUp", "para.multilevel", "para.normal",
    "para.numbering", "para.oneAndHalf", "para.outdent",
    "para.outlineLevel", "para.pageBreakBefore", "para.promote",
    "para.removeHanging", "para.removeSpaceAfter", "para.reset",
    "para.restartNumbering", "para.rtl", "para.set",
    "para.setNumberingValue", "para.shading", "para.single",
    "para.sort", "para.spacing", "para.style",
    "para.tabs", "para.widowControl", "review.comments",
    "review.deleteComment", "review.editComment", "review.newComment",
    "review.reply", "review.resolveComment", "styles.addToGallery",
    "styles.apply", "styles.create", "styles.delete",
    "styles.modify", "styles.rename", "styles.setVisibility",
    "styles.updateToMatch", "table.altText", "table.autofit",
    "table.borders", "table.cellAlign", "table.cellMargins",
    "table.columnWidth", "table.deleteCells", "table.deleteColumn",
    "table.deleteRow", "table.deleteStyle", "table.deleteTable",
    "table.distributeColumns", "table.distributeRows", "table.fitColumn",
    "table.formula", "table.fromText", "table.insertAt",
    "table.insertColumnLeft", "table.insertColumnRight", "table.insertRowAbove",
    "table.insertRowBelow", "table.look", "table.merge",
    "table.modifyStyle", "table.newStyle", "table.repeatHeader",
    "table.rowHeight", "table.rtl", "table.shading",
    "table.sort", "table.split", "table.splitTable",
    "table.style", "table.textDirection", "table.toText",
    "text.backTab", "text.backspace", "text.columnBreak",
    "text.copyright", "text.delete", "text.deleteWordBack",
    "text.deleteWordForward", "text.ellipsis", "text.emDash",
    "text.insert", "text.insertTab", "text.lineBreak",
    "text.nbHyphen", "text.nbsp", "text.newParagraph",
    "text.optionalHyphen", "text.overtype", "text.pageBreak",
    "text.registered", "text.tab", "text.trademark",];

/// The command ids inside those families that are **not** shareable, with
/// the reason in the section comment above each group. They live here, rather
/// than simply being left out of [`SHAREABLE`], so that a registered id can
/// never go missing by accident: the test below requires the two lists to
/// cover the families exactly.
pub const REFUSED_IN_FAMILY: &[&str] = &[
    // Reads or writes a local path.
    "insert.textFromFile", "styles.copyFrom",
    // Embeds foreign bytes (an image, an OLE object, a chart, a diagram).
    "insert.chart", "insert.picture", "insert.object", "insert.smartArt", "insert.spreadsheet",
    // Fields, deferred by the design note on storytold/wordcraft#4639.
    "insert.crossReference", "insert.dateField", "insert.docProperty", "insert.field", "insert.pageField", "insert.timeField",
    // Selection, view and drag state: local, and a peer cannot see the screen it refers to.
    "arrange.selectionPane",
    "outline.close", "outline.collapse", "outline.expand", "outline.firstLineOnly", "outline.selectSubtree",
    "outline.showAll", "outline.showFormatting", "outline.showLevel", "outline.showLevel1", "outline.showLevel2",
    "outline.showLevel3", "outline.showLevel4", "outline.showLevel5", "outline.showLevel6", "outline.showLevel7",
    "outline.showLevel8", "outline.showLevel9", "outline.state", "outline.toggle",
    "table.borderPainter", "table.draw", "table.eraser", "table.moveDrag", "table.resizeDrag",
    "table.selectAt", "table.selectCell", "table.selectColumn", "table.selectRow", "table.selectTable", "table.viewGridlines",
    "text.toggleUnicode", "text.typeCode",
    // Dialogs: UI-only, a param shape the app itself never sends.
    "equation.gallery", "format.fontDialog", "layout.columns", "layout.manualHyphenation", "layout.pageNumberFormat",
    "layout.pageSetup", "para.dialog", "styles.organizer", "styles.pane", "table.properties", "table.quick",
    // Queries: they return data and do not change a document.
    "equation.get", "format.state", "styles.inspect", "styles.inspector", "styles.inspectorClear", "styles.list", "styles.manage",
    // The rest of `review.`: tracked changes, proofing, language, compare and
    // read-aloud. Only a comment's own ids (in `SHAREABLE`) travel; two
    // authors' revisions interacting is undefined, and the proofing and
    // read-aloud half is local machinery.
    "review.accept", "review.acceptAll", "review.addToDictionary", "review.applySuggestion", "review.changes", "review.combine",
    "review.compare", "review.editor", "review.hideInk", "review.ignoreAll", "review.issues", "review.language", "review.markup",
    "review.nextChange", "review.nextComment", "review.previousChange", "review.previousComment", "review.proofing", "review.readAloud",
    "review.reject", "review.rejectAll", "review.restrict", "review.showMarkup", "review.spelling", "review.suggestions",
    "review.thesaurus", "review.trackChanges", "review.trackingOptions", "review.wordCount",
];

/// The families the two lists above are enumerated from.
pub const FAMILIES: &[&str] = &["para.", "insert.", "text.", "table.", "format.", "styles.", "equation.", "outline.", "layout.", "arrange.", "review."];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{FAMILIES, REFUSED_IN_FAMILY, SHAREABLE};

    fn registered() -> BTreeSet<&'static str> {
        wordcraft_engine::cmd::registry().all().iter().map(|s| s.id).collect()
    }

    fn in_families(ids: impl Iterator<Item = &'static str>) -> BTreeSet<&'static str> {
        ids.filter(|id| FAMILIES.iter().any(|f| id.starts_with(f))).collect()
    }

    #[test]
    fn every_id_in_the_document_families_is_accounted_for() {
        let registered = registered();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut dupes: Vec<&str> = Vec::new();
        for id in SHAREABLE.iter().chain(REFUSED_IN_FAMILY) {
            if !seen.insert(id) {
                dupes.push(id);
            }
        }
        assert!(dupes.is_empty(), "an id is in both lists (or twice): {dupes:?}");

        let unknown: Vec<&&str> = seen.iter().filter(|id| !registered.contains(**id)).collect();
        assert!(unknown.is_empty(), "these are on a list but are not registered commands: {unknown:?}");

        let family_ids = in_families(registered.iter().copied());
        let missing: Vec<&&str> = family_ids.iter().filter(|id| !seen.contains(**id)).collect();
        assert!(missing.is_empty(), "registered ids of the document families that are on neither list: {missing:?}");
    }

    #[test]
    fn the_shareable_list_holds_no_id_outside_the_document_families() {
        let outside: Vec<&&str> = SHAREABLE.iter().filter(|id| !in_families(std::iter::once(**id)).contains(**id)).collect();
        assert!(outside.is_empty(), "shareable ids outside the ten families: {outside:?}");
    }
}
