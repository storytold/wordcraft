//! WordCraft's icon set: SVG documents on a 20×20 view box, registered as the app icon set beside
//! the framework's catalog, plus the Insert › Icons library glyph.

use std::borrow::Cow;

use craft::behavior::IconId;
use craft::ui::primitives::{AppIcons, Glyph, IconSpec, register_app_icons};
use craft::ui::style::UiTokens;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

/// One row per file in `crates/ui-egui/icons/`, in sorted order: the icon's name and its document.
///
/// The names come from the exporter's file names, so a reader names an icon exactly as its file does.
static WORDCRAFT_ICONS_TABLE: &[(&str, &[u8])] = &[
    ("accept", include_bytes!("../icons/accept.svg")),
    ("accessibility", include_bytes!("../icons/accessibility.svg")),
    ("addNote", include_bytes!("../icons/addNote.svg")),
    ("addPen", include_bytes!("../icons/addPen.svg")),
    ("addShape", include_bytes!("../icons/addShape.svg")),
    ("addText", include_bytes!("../icons/addText.svg")),
    ("addins", include_bytes!("../icons/addins.svg")),
    ("addressBlock", include_bytes!("../icons/addressBlock.svg")),
    ("align", include_bytes!("../icons/align.svg")),
    ("alignCenter", include_bytes!("../icons/alignCenter.svg")),
    ("alignLeft", include_bytes!("../icons/alignLeft.svg")),
    ("alignRight", include_bytes!("../icons/alignRight.svg")),
    ("alignText", include_bytes!("../icons/alignText.svg")),
    ("altText", include_bytes!("../icons/altText.svg")),
    ("arrangeAll", include_bytes!("../icons/arrangeAll.svg")),
    ("autofit", include_bytes!("../icons/autofit.svg")),
    ("bibliography", include_bytes!("../icons/bibliography.svg")),
    ("blankPage", include_bytes!("../icons/blankPage.svg")),
    ("blockAuthors", include_bytes!("../icons/blockAuthors.svg")),
    ("bold", include_bytes!("../icons/bold.svg")),
    ("wordcraft.bookmark", include_bytes!("../icons/bookmark.svg")),
    ("borderPainter", include_bytes!("../icons/borderPainter.svg")),
    ("borders", include_bytes!("../icons/borders.svg")),
    ("breaks", include_bytes!("../icons/breaks.svg")),
    ("bringForward", include_bytes!("../icons/bringForward.svg")),
    ("bullets", include_bytes!("../icons/bullets.svg")),
    ("canvas", include_bytes!("../icons/canvas.svg")),
    ("caption", include_bytes!("../icons/caption.svg")),
    ("case", include_bytes!("../icons/case.svg")),
    ("ccCheckBox", include_bytes!("../icons/ccCheckBox.svg")),
    ("ccComboBox", include_bytes!("../icons/ccComboBox.svg")),
    ("ccDate", include_bytes!("../icons/ccDate.svg")),
    ("ccDropDown", include_bytes!("../icons/ccDropDown.svg")),
    ("ccGallery", include_bytes!("../icons/ccGallery.svg")),
    ("ccPicture", include_bytes!("../icons/ccPicture.svg")),
    ("ccPlainText", include_bytes!("../icons/ccPlainText.svg")),
    ("ccRepeat", include_bytes!("../icons/ccRepeat.svg")),
    ("ccRichText", include_bytes!("../icons/ccRichText.svg")),
    ("cellAlign", include_bytes!("../icons/cellAlign.svg")),
    ("cellMargins", include_bytes!("../icons/cellMargins.svg")),
    ("changeChartType", include_bytes!("../icons/changeChartType.svg")),
    ("charborder", include_bytes!("../icons/charborder.svg")),
    ("chart", include_bytes!("../icons/chart.svg")),
    ("chartData", include_bytes!("../icons/chartData.svg")),
    ("chartTitle", include_bytes!("../icons/chartTitle.svg")),
    ("wordcraft.check", include_bytes!("../icons/check.svg")),
    ("checkErrors", include_bytes!("../icons/checkErrors.svg")),
    ("chevronDoubleRight", include_bytes!("../icons/chevronDoubleRight.svg")),
    ("chevronDown", include_bytes!("../icons/chevronDown.svg")),
    ("chevronLeft", include_bytes!("../icons/chevronLeft.svg")),
    ("chevronRight", include_bytes!("../icons/chevronRight.svg")),
    ("chevronUp", include_bytes!("../icons/chevronUp.svg")),
    ("citation", include_bytes!("../icons/citation.svg")),
    ("clear", include_bytes!("../icons/clear.svg")),
    ("wordcraft.close", include_bytes!("../icons/close.svg")),
    ("closeOutline", include_bytes!("../icons/closeOutline.svg")),
    ("colWidth", include_bytes!("../icons/colWidth.svg")),
    ("colors", include_bytes!("../icons/colors.svg")),
    ("columns", include_bytes!("../icons/columns.svg")),
    ("combine", include_bytes!("../icons/combine.svg")),
    ("comment", include_bytes!("../icons/comment.svg")),
    ("compare", include_bytes!("../icons/compare.svg")),
    ("wordcraft.copy", include_bytes!("../icons/copy.svg")),
    ("coverPage", include_bytes!("../icons/coverPage.svg")),
    ("wordcraft.crop", include_bytes!("../icons/crop.svg")),
    ("crossRef", include_bytes!("../icons/crossRef.svg")),
    ("cut", include_bytes!("../icons/cut.svg")),
    ("darkMode", include_bytes!("../icons/darkMode.svg")),
    ("dataLabels", include_bytes!("../icons/dataLabels.svg")),
    ("dateTime", include_bytes!("../icons/dateTime.svg")),
    ("deleteComment", include_bytes!("../icons/deleteComment.svg")),
    ("deleteTable", include_bytes!("../icons/deleteTable.svg")),
    ("demote", include_bytes!("../icons/demote.svg")),
    ("designMode", include_bytes!("../icons/designMode.svg")),
    ("dictate", include_bytes!("../icons/dictate.svg")),
    ("discord", include_bytes!("../icons/discord.svg")),
    ("discordPill", include_bytes!("../icons/discordPill.svg")),
    ("distributeCols", include_bytes!("../icons/distributeCols.svg")),
    ("distributeRows", include_bytes!("../icons/distributeRows.svg")),
    ("docPrefs", include_bytes!("../icons/docPrefs.svg")),
    ("documentTemplate", include_bytes!("../icons/documentTemplate.svg")),
    ("draft", include_bytes!("../icons/draft.svg")),
    ("draw", include_bytes!("../icons/draw.svg")),
    ("drawTable", include_bytes!("../icons/drawTable.svg")),
    ("dropCap", include_bytes!("../icons/dropCap.svg")),
    ("dropdown", include_bytes!("../icons/dropdown.svg")),
    ("editRecipients", include_bytes!("../icons/editRecipients.svg")),
    ("editor", include_bytes!("../icons/editor.svg")),
    ("effects", include_bytes!("../icons/effects.svg")),
    ("effectsDesign", include_bytes!("../icons/effectsDesign.svg")),
    ("enclose", include_bytes!("../icons/enclose.svg")),
    ("endnote", include_bytes!("../icons/endnote.svg")),
    ("envelope", include_bytes!("../icons/envelope.svg")),
    ("equation", include_bytes!("../icons/equation.svg")),
    ("wordcraft.eraser", include_bytes!("../icons/eraser.svg")),
    ("export", include_bytes!("../icons/export.svg")),
    ("wordcraft.file", include_bytes!("../icons/file.svg")),
    ("find", include_bytes!("../icons/find.svg")),
    ("finish", include_bytes!("../icons/finish.svg")),
    ("first", include_bytes!("../icons/first.svg")),
    ("fitOnePage", include_bytes!("../icons/fitOnePage.svg")),
    ("focus", include_bytes!("../icons/focus.svg")),
    ("wordcraft.folder", include_bytes!("../icons/folder.svg")),
    ("fontMissing", include_bytes!("../icons/fontMissing.svg")),
    ("fontcolor", include_bytes!("../icons/fontcolor.svg")),
    ("fontdialog", include_bytes!("../icons/fontdialog.svg")),
    ("fonts", include_bytes!("../icons/fonts.svg")),
    ("footer", include_bytes!("../icons/footer.svg")),
    ("footnote", include_bytes!("../icons/footnote.svg")),
    ("formula", include_bytes!("../icons/formula.svg")),
    ("fullScreen", include_bytes!("../icons/fullScreen.svg")),
    ("greetingLine", include_bytes!("../icons/greetingLine.svg")),
    ("gridlines", include_bytes!("../icons/gridlines.svg")),
    ("group", include_bytes!("../icons/group.svg")),
    ("grow", include_bytes!("../icons/grow.svg")),
    ("header", include_bytes!("../icons/header.svg")),
    ("help", include_bytes!("../icons/help.svg")),
    ("hideInk", include_bytes!("../icons/hideInk.svg")),
    ("highlight", include_bytes!("../icons/highlight.svg")),
    ("highlightFields", include_bytes!("../icons/highlightFields.svg")),
    ("highlightNoColor", include_bytes!("../icons/highlightNoColor.svg")),
    ("homeFolder", include_bytes!("../icons/homeFolder.svg")),
    ("hyphenation", include_bytes!("../icons/hyphenation.svg")),
    ("icons", include_bytes!("../icons/icons.svg")),
    ("immersive", include_bytes!("../icons/immersive.svg")),
    ("indent", include_bytes!("../icons/indent.svg")),
    ("indentLeft", include_bytes!("../icons/indentLeft.svg")),
    ("indentRight", include_bytes!("../icons/indentRight.svg")),
    ("index", include_bytes!("../icons/index.svg")),
    ("wordcraft.info", include_bytes!("../icons/info.svg")),
    ("inkToMath", include_bytes!("../icons/inkToMath.svg")),
    ("inkToShape", include_bytes!("../icons/inkToShape.svg")),
    ("insertAbove", include_bytes!("../icons/insertAbove.svg")),
    ("insertBelow", include_bytes!("../icons/insertBelow.svg")),
    ("insertLeft", include_bytes!("../icons/insertLeft.svg")),
    ("insertRight", include_bytes!("../icons/insertRight.svg")),
    ("interfaceTheme", include_bytes!("../icons/interfaceTheme.svg")),
    ("italic", include_bytes!("../icons/italic.svg")),
    ("justify", include_bytes!("../icons/justify.svg")),
    ("labels", include_bytes!("../icons/labels.svg")),
    ("language", include_bytes!("../icons/language.svg")),
    ("wordcraft.lasso", include_bytes!("../icons/lasso.svg")),
    ("last", include_bytes!("../icons/last.svg")),
    ("launcher", include_bytes!("../icons/launcher.svg")),
    ("legend", include_bytes!("../icons/legend.svg")),
    ("lineNumbers", include_bytes!("../icons/lineNumbers.svg")),
    ("lineSpacing", include_bytes!("../icons/lineSpacing.svg")),
    ("wordcraft.link", include_bytes!("../icons/link.svg")),
    ("macros", include_bytes!("../icons/macros.svg")),
    ("mailMerge", include_bytes!("../icons/mailMerge.svg")),
    ("margins", include_bytes!("../icons/margins.svg")),
    ("markCitation", include_bytes!("../icons/markCitation.svg")),
    ("markEntry", include_bytes!("../icons/markEntry.svg")),
    ("markdownSource", include_bytes!("../icons/markdownSource.svg")),
    ("markup", include_bytes!("../icons/markup.svg")),
    ("matchFields", include_bytes!("../icons/matchFields.svg")),
    ("merge", include_bytes!("../icons/merge.svg")),
    ("mergeField", include_bytes!("../icons/mergeField.svg")),
    ("wordcraft.minus", include_bytes!("../icons/minus.svg")),
    ("models3d", include_bytes!("../icons/models3d.svg")),
    ("wordcraft.more", include_bytes!("../icons/more.svg")),
    ("multilevel", include_bytes!("../icons/multilevel.svg")),
    ("multiplePages", include_bytes!("../icons/multiplePages.svg")),
    ("navPane", include_bytes!("../icons/navPane.svg")),
    ("newComment", include_bytes!("../icons/newComment.svg")),
    ("newWindow", include_bytes!("../icons/newWindow.svg")),
    ("next", include_bytes!("../icons/next.svg")),
    ("nextChange", include_bytes!("../icons/nextChange.svg")),
    ("nextComment", include_bytes!("../icons/nextComment.svg")),
    ("nextFootnote", include_bytes!("../icons/nextFootnote.svg")),
    ("numbering", include_bytes!("../icons/numbering.svg")),
    ("object", include_bytes!("../icons/object.svg")),
    ("onePage", include_bytes!("../icons/onePage.svg")),
    ("onlinePicture", include_bytes!("../icons/onlinePicture.svg")),
    ("wordcraft.open", include_bytes!("../icons/open.svg")),
    ("options", include_bytes!("../icons/options.svg")),
    ("orientation", include_bytes!("../icons/orientation.svg")),
    ("outdent", include_bytes!("../icons/outdent.svg")),
    ("outline", include_bytes!("../icons/outline.svg")),
    ("outlineBody", include_bytes!("../icons/outlineBody.svg")),
    ("outlineCollapse", include_bytes!("../icons/outlineCollapse.svg")),
    ("outlineDemote", include_bytes!("../icons/outlineDemote.svg")),
    ("outlineDown", include_bytes!("../icons/outlineDown.svg")),
    ("outlineExpand", include_bytes!("../icons/outlineExpand.svg")),
    ("outlinePromote", include_bytes!("../icons/outlinePromote.svg")),
    ("outlineTop", include_bytes!("../icons/outlineTop.svg")),
    ("outlineUp", include_bytes!("../icons/outlineUp.svg")),
    ("pageBorders", include_bytes!("../icons/pageBorders.svg")),
    ("pageBreak", include_bytes!("../icons/pageBreak.svg")),
    ("pageColor", include_bytes!("../icons/pageColor.svg")),
    ("pageNumber", include_bytes!("../icons/pageNumber.svg")),
    ("pageWidth", include_bytes!("../icons/pageWidth.svg")),
    ("painter", include_bytes!("../icons/painter.svg")),
    ("paraSpacing", include_bytes!("../icons/paraSpacing.svg")),
    ("pastCitation", include_bytes!("../icons/pastCitation.svg")),
    ("paste", include_bytes!("../icons/paste.svg")),
    ("pen", include_bytes!("../icons/pen.svg")),
    ("wordcraft.pencil", include_bytes!("../icons/pencil.svg")),
    ("phonetic", include_bytes!("../icons/phonetic.svg")),
    ("picture", include_bytes!("../icons/picture.svg")),
    ("wordcraft.pilcrow", include_bytes!("../icons/pilcrow.svg")),
    ("wordcraft.pin", include_bytes!("../icons/pin.svg")),
    ("wordcraft.plus", include_bytes!("../icons/plus.svg")),
    ("position", include_bytes!("../icons/position.svg")),
    ("prevChange", include_bytes!("../icons/prevChange.svg")),
    ("prevComment", include_bytes!("../icons/prevComment.svg")),
    ("preview", include_bytes!("../icons/preview.svg")),
    ("previous", include_bytes!("../icons/previous.svg")),
    ("print", include_bytes!("../icons/print.svg")),
    ("printLayout", include_bytes!("../icons/printLayout.svg")),
    ("promote", include_bytes!("../icons/promote.svg")),
    ("properties", include_bytes!("../icons/properties.svg")),
    ("protect", include_bytes!("../icons/protect.svg")),
    ("quickParts", include_bytes!("../icons/quickParts.svg")),
    ("readAloud", include_bytes!("../icons/readAloud.svg")),
    ("readMode", include_bytes!("../icons/readMode.svg")),
    ("recipients", include_bytes!("../icons/recipients.svg")),
    ("recover", include_bytes!("../icons/recover.svg")),
    ("wordcraft.redo", include_bytes!("../icons/redo.svg")),
    ("reject", include_bytes!("../icons/reject.svg")),
    ("repeatHeader", include_bytes!("../icons/repeatHeader.svg")),
    ("replace", include_bytes!("../icons/replace.svg")),
    ("replay", include_bytes!("../icons/replay.svg")),
    ("replayForward", include_bytes!("../icons/replayForward.svg")),
    ("replayPause", include_bytes!("../icons/replayPause.svg")),
    ("replayPlay", include_bytes!("../icons/replayPlay.svg")),
    ("replayRewind", include_bytes!("../icons/replayRewind.svg")),
    ("researcher", include_bytes!("../icons/researcher.svg")),
    ("resetGraphic", include_bytes!("../icons/resetGraphic.svg")),
    ("resolve", include_bytes!("../icons/resolve.svg")),
    ("restrict", include_bytes!("../icons/restrict.svg")),
    ("reviewingPane", include_bytes!("../icons/reviewingPane.svg")),
    ("rotate", include_bytes!("../icons/rotate.svg")),
    ("rowHeight", include_bytes!("../icons/rowHeight.svg")),
    ("wordcraft.ruler", include_bytes!("../icons/ruler.svg")),
    ("rules", include_bytes!("../icons/rules.svg")),
    ("wordcraft.save", include_bytes!("../icons/save.svg")),
    ("saveAs", include_bytes!("../icons/saveAs.svg")),
    ("screenshot", include_bytes!("../icons/screenshot.svg")),
    ("wordcraft.search", include_bytes!("../icons/search.svg")),
    ("select", include_bytes!("../icons/select.svg")),
    ("selectTable", include_bytes!("../icons/selectTable.svg")),
    ("selectionPane", include_bytes!("../icons/selectionPane.svg")),
    ("sendBackward", include_bytes!("../icons/sendBackward.svg")),
    ("setDefault", include_bytes!("../icons/setDefault.svg")),
    ("shading", include_bytes!("../icons/shading.svg")),
    ("shadingNoBar", include_bytes!("../icons/shadingNoBar.svg")),
    ("shape.arrow", include_bytes!("../icons/shape.arrow.svg")),
    ("shape.curvedArrowConnector", include_bytes!("../icons/shape.curvedArrowConnector.svg")),
    ("shape.curvedConnector", include_bytes!("../icons/shape.curvedConnector.svg")),
    ("shape.diamond", include_bytes!("../icons/shape.diamond.svg")),
    ("shape.elbowArrowConnector", include_bytes!("../icons/shape.elbowArrowConnector.svg")),
    ("shape.elbowConnector", include_bytes!("../icons/shape.elbowConnector.svg")),
    ("shape.ellipse", include_bytes!("../icons/shape.ellipse.svg")),
    ("shape.heart", include_bytes!("../icons/shape.heart.svg")),
    ("shape.line", include_bytes!("../icons/shape.line.svg")),
    ("shape.rectangle", include_bytes!("../icons/shape.rectangle.svg")),
    ("shape.roundedRectangle", include_bytes!("../icons/shape.roundedRectangle.svg")),
    ("shape.star", include_bytes!("../icons/shape.star.svg")),
    ("shape.straightArrowConnector", include_bytes!("../icons/shape.straightArrowConnector.svg")),
    ("shape.straightConnector", include_bytes!("../icons/shape.straightConnector.svg")),
    ("shape.triangle", include_bytes!("../icons/shape.triangle.svg")),
    ("shapeEffects", include_bytes!("../icons/shapeEffects.svg")),
    ("shapes", include_bytes!("../icons/shapes.svg")),
    ("share", include_bytes!("../icons/share.svg")),
    ("showComments", include_bytes!("../icons/showComments.svg")),
    ("showNotes", include_bytes!("../icons/showNotes.svg")),
    ("shrink", include_bytes!("../icons/shrink.svg")),
    ("sideBySide", include_bytes!("../icons/sideBySide.svg")),
    ("sideToSide", include_bytes!("../icons/sideToSide.svg")),
    ("signature", include_bytes!("../icons/signature.svg")),
    ("size", include_bytes!("../icons/size.svg")),
    ("smartArt", include_bytes!("../icons/smartArt.svg")),
    ("smartArtColors", include_bytes!("../icons/smartArtColors.svg")),
    ("sort", include_bytes!("../icons/sort.svg")),
    ("sources", include_bytes!("../icons/sources.svg")),
    ("spaceAfter", include_bytes!("../icons/spaceAfter.svg")),
    ("spaceBefore", include_bytes!("../icons/spaceBefore.svg")),
    ("spelling", include_bytes!("../icons/spelling.svg")),
    ("wordcraft.split", include_bytes!("../icons/split.svg")),
    ("splitCells", include_bytes!("../icons/splitCells.svg")),
    ("splitTable", include_bytes!("../icons/splitTable.svg")),
    ("strike", include_bytes!("../icons/strike.svg")),
    ("styles", include_bytes!("../icons/styles.svg")),
    ("stylesPane", include_bytes!("../icons/stylesPane.svg")),
    ("subscript", include_bytes!("../icons/subscript.svg")),
    ("superscript", include_bytes!("../icons/superscript.svg")),
    ("switchWindows", include_bytes!("../icons/switchWindows.svg")),
    ("symbol", include_bytes!("../icons/symbol.svg")),
    ("syncScroll", include_bytes!("../icons/syncScroll.svg")),
    ("table", include_bytes!("../icons/table.svg")),
    ("tableOfAuthorities", include_bytes!("../icons/tableOfAuthorities.svg")),
    ("tableOfFigures", include_bytes!("../icons/tableOfFigures.svg")),
    ("textBox", include_bytes!("../icons/textBox.svg")),
    ("textBoxLink", include_bytes!("../icons/textBoxLink.svg")),
    ("textBoxUnlink", include_bytes!("../icons/textBoxUnlink.svg")),
    ("textDirection", include_bytes!("../icons/textDirection.svg")),
    ("textFill", include_bytes!("../icons/textFill.svg")),
    ("textLtr", include_bytes!("../icons/textLtr.svg")),
    ("textOutline", include_bytes!("../icons/textOutline.svg")),
    ("textPane", include_bytes!("../icons/textPane.svg")),
    ("textRtl", include_bytes!("../icons/textRtl.svg")),
    ("themes", include_bytes!("../icons/themes.svg")),
    ("thesaurus", include_bytes!("../icons/thesaurus.svg")),
    ("thickness", include_bytes!("../icons/thickness.svg")),
    ("toText", include_bytes!("../icons/toText.svg")),
    ("toc", include_bytes!("../icons/toc.svg")),
    ("trackChanges", include_bytes!("../icons/trackChanges.svg")),
    ("transform", include_bytes!("../icons/transform.svg")),
    ("translate", include_bytes!("../icons/translate.svg")),
    ("underline", include_bytes!("../icons/underline.svg")),
    ("wordcraft.undo", include_bytes!("../icons/undo.svg")),
    ("unlinkCitations", include_bytes!("../icons/unlinkCitations.svg")),
    ("update", include_bytes!("../icons/update.svg")),
    ("updateTable", include_bytes!("../icons/updateTable.svg")),
    ("user", include_bytes!("../icons/user.svg")),
    ("vertical", include_bytes!("../icons/vertical.svg")),
    ("video", include_bytes!("../icons/video.svg")),
    ("watermark", include_bytes!("../icons/watermark.svg")),
    ("webLayout", include_bytes!("../icons/webLayout.svg")),
    ("wordArt", include_bytes!("../icons/wordArt.svg")),
    ("wordCount", include_bytes!("../icons/wordCount.svg")),
    ("wrapText", include_bytes!("../icons/wrapText.svg")),
    ("zoom", include_bytes!("../icons/zoom.svg")),
    ("zoom100", include_bytes!("../icons/zoom100.svg")),
    ("zoomIn", include_bytes!("../icons/zoomIn.svg")),
    ("zoomOut", include_bytes!("../icons/zoomOut.svg")),
];

/// The app icon set. Every document is drawn with the pen width 1.25 of the 20-unit view box,
/// which the framework scales to its own stroke.
pub static WORDCRAFT_ICONS: AppIcons = AppIcons::new("wordcraft", Some(1.25), WORDCRAFT_ICONS_TABLE);

/// Registers the app icon set beside the framework's catalog; runs before the first pass.
pub fn register() {
    register_app_icons(&WORDCRAFT_ICONS);
}

/// The app icons whose names the framework's catalog also has. The catalog wins a clash, so the
/// app registers its own drawings of these as `wordcraft.<name>`, and [`id`] names them so.
const OWN_OVER_CATALOG: &[(&str, &str)] = &[
    ("bookmark", "wordcraft.bookmark"),
    ("check", "wordcraft.check"),
    ("close", "wordcraft.close"),
    ("copy", "wordcraft.copy"),
    ("crop", "wordcraft.crop"),
    ("eraser", "wordcraft.eraser"),
    ("file", "wordcraft.file"),
    ("folder", "wordcraft.folder"),
    ("info", "wordcraft.info"),
    ("lasso", "wordcraft.lasso"),
    ("link", "wordcraft.link"),
    ("minus", "wordcraft.minus"),
    ("more", "wordcraft.more"),
    ("open", "wordcraft.open"),
    ("pencil", "wordcraft.pencil"),
    ("pilcrow", "wordcraft.pilcrow"),
    ("pin", "wordcraft.pin"),
    ("plus", "wordcraft.plus"),
    ("redo", "wordcraft.redo"),
    ("ruler", "wordcraft.ruler"),
    ("save", "wordcraft.save"),
    ("search", "wordcraft.search"),
    ("split", "wordcraft.split"),
    ("undo", "wordcraft.undo"),
];

/// The icon WordCraft draws for `name`: its own drawing where the framework's catalog has the
/// same name, else `name` itself.
pub fn id(name: &'static str) -> IconId {
    match OWN_OVER_CATALOG.iter().find(|(plain, _)| *plain == name) {
        Some((_, own)) => IconId::new(own),
        None => IconId::new(name),
    }
}

/// The icon a command's button shows outside its own ribbon group (custom groups, the Quick
/// Access Toolbar): the one its ribbon button uses, which is usually named after the id's last
/// part. A name the set doesn't have draws the framework's fallback tile.
pub fn for_command(id: &str) -> IconSpec<'static> {
    const ICONS: &[(&str, &str)] = &[
        ("arrange.wrap", "wrapText"),
        ("draw.highlighter", "highlight"),
        ("format.border", "charborder"),
        ("format.color", "fontcolor"),
        ("format.growFont", "grow"),
        ("format.shrinkFont", "shrink"),
        ("format.strikethrough", "strike"),
        ("format.highlight", "highlight"),
        ("insert.signatureLine", "signature"),
        ("mailings.envelopes", "envelope"),
        ("mailings.insertField", "mergeField"),
        ("para.ltr", "textLtr"),
        ("para.rtl", "textRtl"),
        ("para.bullets", "bullets"),
        ("para.numbering", "numbering"),
        ("table.insertRowAbove", "insertAbove"),
        ("view.commentsPane", "showComments"),
        ("view.marks", "pilcrow"),
        ("edit.repeat", "redo"),
        ("file.new", "blankPage"),
        ("tools.customizeRibbon", "more"),
    ];
    let name: Cow<'static, str> = match ICONS.iter().find(|(c, _)| *c == id) {
        Some((_, icon)) => Cow::Borrowed(icon),
        None => Cow::Owned(id.rsplit('.').next().unwrap_or(id).to_owned()),
    };
    let name = match OWN_OVER_CATALOG.iter().find(|(plain, _)| *plain == name) {
        Some((_, own)) => Cow::Borrowed(*own),
        None => name,
    };
    IconSpec::svg(IconId::from(name))
}

/// An Insert › Icons library icon as a value glyph, drawn from the same path data the inserted
/// picture is rendered from (`wordcraft_render::icon_lib`).
#[derive(craft::ui::contract::MemoProps)]
pub struct LibraryGlyph {
    #[memo_props(address = "library icons are items of the static `ICONS`")]
    icon: &'static wordcraft_render::icon_lib::Icon,
}

/// The glyph of a library icon; bind it to a local so `IconSpec::glyph(&glyph)` can borrow it.
pub fn library_glyph(icon: &'static wordcraft_render::icon_lib::Icon) -> impl Glyph {
    LibraryGlyph { icon }
}

impl Glyph for LibraryGlyph {
    fn paint(&self, p: &Painter, _tokens: &UiTokens, r: Rect, c: Color32, _mirrored: bool) {
        use wordcraft_render::icon_lib::{GRID, STROKE};
        let s = r.width().min(r.height()) / GRID;
        let o = r.center() - vec2(GRID / 2.0 * s, GRID / 2.0 * s);
        let stroke = Stroke::new(STROKE * s, c);
        for piece in self.icon.pieces() {
            let pts: Vec<Pos2> = piece.points.iter().map(|q| pos2(o.x + q[0] * s, o.y + q[1] * s)).collect();
            if piece.fill {
                p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
            } else if piece.closed {
                p.add(Shape::closed_line(pts, stroke));
            } else {
                // Round caps, as on the inserted picture.
                for end in [pts.first(), pts.last()].into_iter().flatten() {
                    p.circle_filled(*end, stroke.width / 2.0, c);
                }
                p.add(Shape::line(pts, stroke));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers of the outline path's `d` attribute, in order (the arrow's paths are only M, L, Z). The
    /// icon fills the tail and the head as two translucent paths and ends with the stroked outline of the
    /// whole arrow, which is the last.
    fn path_points(svg: &str) -> Vec<(f32, f32)> {
        let d = svg.split(" d=\"").last().and_then(|rest| rest.split('"').next()).expect("the arrow has a path");
        let nums: Vec<f32> = d
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter(|t| !t.is_empty())
            .map(|t| t.parse().expect("a number"))
            .collect();
        nums.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect()
    }

    /// Every app drawing is registered under a name the framework's catalog does not have, so none
    /// is refused and replaced by the catalog's icon of the same name.
    #[test]
    fn no_app_icon_is_shadowed_by_the_catalog() {
        let catalog = craft::ui::primitives::icon_names();
        let names: Vec<&str> = WORDCRAFT_ICONS.icons.iter().map(|(name, _)| *name).collect();
        let shadowed: Vec<&&str> = names.iter().filter(|name| catalog.binary_search(name).is_ok()).collect();
        assert!(shadowed.is_empty(), "the catalog has these names: {shadowed:?}");
        for (plain, own) in OWN_OVER_CATALOG {
            assert!(names.contains(own), "{own} is not registered");
            assert_eq!(id(plain).name(), *own);
        }
        let undo = match for_command("edit.undo").source {
            craft::ui::primitives::IconSource::Svg(id) => id.name().to_owned(),
            _ => String::new(),
        };
        assert_eq!(undo, "wordcraft.undo");
    }

    /// #6768: the Insert › Shapes Arrow tile is the block arrow that tile inserts, outlined as the
    /// document draws it, not a thin line with a head.
    #[test]
    fn the_arrow_tile_is_the_block_arrow_it_inserts() {
        let (_, bytes) = WORDCRAFT_ICONS.icons.iter().find(|(name, _)| *name == "shape.arrow").expect("shape.arrow is registered");
        let svg = std::str::from_utf8(bytes).expect("an SVG document is text");
        let got = path_points(svg);
        let want = wordcraft_doc::wrap::arrow_outline(wordcraft_doc::para::ShapeKind::Arrow, 14.0, 10.0);
        assert_eq!(got.len(), want.len(), "{got:?}");
        for ((x, y), (u, v)) in got.iter().zip(want) {
            let (ex, ey) = (3.0 + u * 14.0, 5.0 + v * 10.0);
            assert!((x - ex).abs() < 0.05 && (y - ey).abs() < 0.05, "({x}, {y}) vs ({ex}, {ey})");
        }
    }
}
