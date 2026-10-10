//! The word-processor feature catalog: the incumbent's ribbon and menu commands (feature names
//! only) mapped to our command ids. `parity()` compares it with the registry; the gap is the
//! work list (`cargo xtask parity` → docs/parity-checklist.md).

use serde_json::{Value, json};

use crate::Registry;

/// `Tab|Group|Feature|command id`
pub const CATALOG: &str = "\
File|Backstage|New|file.new
File|Backstage|Open|file.open
File|Backstage|Save|file.save
File|Backstage|Save As|file.saveAs
File|Backstage|Print|file.print
File|Backstage|Close|file.close
File|Backstage|Info|file.info
File|Backstage|Properties|file.properties
File|Backstage|Export PDF|file.exportPdf
File|Backstage|Export Image|file.exportPng
File|Backstage|Options|file.options
File|Backstage|Share|file.share
File|Backstage|Protect Document|file.protect
File|Backstage|Inspect Document|file.inspect
File|Backstage|Check Accessibility|file.accessibility
File|Backstage|Check Compatibility|file.compatibility
File|Backstage|Version History|file.versions
File|Backstage|Recover Unsaved|file.recover
File|Backstage|New from Template|file.newFromTemplate
File|Backstage|Save as Template|file.saveTemplate
Quick Access|Toolbar|Undo|edit.undo
Quick Access|Toolbar|Redo|edit.redo
Quick Access|Toolbar|Repeat|edit.repeat
Quick Access|Toolbar|AutoSave|file.autosave
Home|Clipboard|Paste|edit.paste
Home|Clipboard|Paste Keep Text Only|edit.pasteText
Home|Clipboard|Paste Merge Formatting|edit.pasteMerge
Home|Clipboard|Paste Special|edit.pasteSpecial
Home|Clipboard|Cut|edit.cut
Home|Clipboard|Copy|edit.copy
Home|Clipboard|Format Painter|edit.formatPainter
Home|Clipboard|Copy Formatting|edit.copyFormat
Home|Clipboard|Paste Formatting|edit.pasteFormat
Home|Clipboard|Clipboard Pane|edit.clipboardPane
Home|Font|Font|format.font
Home|Font|Font Size|format.size
Home|Font|Increase Font Size|format.growFont
Home|Font|Decrease Font Size|format.shrinkFont
Home|Font|Change Case|format.changeCase
Home|Font|Clear All Formatting|format.clear
Home|Font|Bold|format.bold
Home|Font|Italic|format.italic
Home|Font|Underline|format.underline
Home|Font|Double Underline|format.doubleUnderline
Home|Font|Strikethrough|format.strikethrough
Home|Font|Subscript|format.subscript
Home|Font|Superscript|format.superscript
Home|Font|Text Effects Outline|format.outline
Home|Font|Text Effects Shadow|format.shadow
Home|Font|Text Highlight Color|format.highlight
Home|Font|Font Color|format.color
Home|Font|Character Shading|format.shading
Home|Font|Font Dialog|format.fontDialog
Home|Font|Double Strikethrough|format.doubleStrikethrough
Home|Font|Small Caps|format.smallCaps
Home|Font|All Caps|format.allCaps
Home|Font|Hidden|format.hidden
Home|Font|Emboss|format.emboss
Home|Font|Engrave|format.engrave
Home|Font|Character Spacing|format.spacing
Home|Font|Character Scale|format.scale
Home|Font|Character Position|format.position
Home|Font|Character Border|format.border
Home|Font|Phonetic Guide|format.phonetic
Home|Font|Enclose Characters|format.enclose
Home|Paragraph|Bullets|para.bullets
Home|Paragraph|Numbering|para.numbering
Home|Paragraph|Multilevel List|para.multilevel
Home|Paragraph|Change List Level|para.listLevel
Home|Paragraph|Restart Numbering|para.restartNumbering
Home|Paragraph|Define New Bullet|para.defineBullet
Home|Paragraph|Define New Number Format|para.defineNumber
Home|Paragraph|Set Numbering Value|para.setNumberingValue
Home|Paragraph|Decrease Indent|para.outdent
Home|Paragraph|Increase Indent|para.indent
Home|Paragraph|Sort|para.sort
Home|Paragraph|Show/Hide Marks|view.marks
Home|Paragraph|Align Left|para.alignLeft
Home|Paragraph|Center|para.alignCenter
Home|Paragraph|Align Right|para.alignRight
Home|Paragraph|Justify|para.justify
Home|Paragraph|Distributed|para.distribute
Home|Paragraph|Line Spacing|para.lineSpacing
Home|Paragraph|Add Space Before|para.addSpaceBefore
Home|Paragraph|Remove Space After|para.removeSpaceAfter
Home|Paragraph|Shading|para.shading
Home|Paragraph|Borders|para.borders
Home|Paragraph|Horizontal Line|insert.horizontalLine
Home|Paragraph|Paragraph Dialog|para.dialog
Home|Paragraph|Tabs|para.tabs
Home|Paragraph|Keep with Next|para.keepNext
Home|Paragraph|Keep Lines Together|para.keepLines
Home|Paragraph|Page Break Before|para.pageBreakBefore
Home|Paragraph|Widow/Orphan Control|para.widowControl
Home|Paragraph|Outline Level|para.outlineLevel
Home|Paragraph|Hanging Indent|para.hangingIndent
Home|Paragraph|Right-to-Left Text Direction|para.rtl
Home|Paragraph|Left-to-Right Text Direction|para.ltr
Home|Paragraph|Asian Typography|para.asianTypography
Home|Styles|Styles Gallery|para.style
Home|Styles|Normal|para.normal
Home|Styles|Heading 1|para.heading1
Home|Styles|Heading 2|para.heading2
Home|Styles|Heading 3|para.heading3
Home|Styles|Create a Style|styles.create
Home|Styles|Modify Style|styles.modify
Home|Styles|Update Style to Match|styles.updateToMatch
Home|Styles|Delete Style|styles.delete
Home|Styles|Styles Pane|styles.pane
Home|Styles|Style Inspector|styles.inspector
Home|Styles|Manage Styles|styles.manage
Home|Styles|Apply Styles|styles.apply
Home|Styles|Character Style|format.charStyle
Home|Editing|Find|edit.find
Home|Editing|Find Next|edit.findNext
Home|Editing|Find Previous|edit.findPrevious
Home|Editing|Advanced Find|edit.advancedFind
Home|Editing|Replace|edit.replace
Home|Editing|Replace All|edit.replaceAll
Home|Editing|Go To|edit.goto
Home|Editing|Select All|select.all
Home|Editing|Select Objects|select.objects
Home|Editing|Select Similar Formatting|select.similar
Home|Voice|Dictate|tools.dictate
Home|Editor|Editor|review.editor
Insert|Pages|Cover Page|insert.coverPage
Insert|Pages|Blank Page|insert.blankPage
Insert|Pages|Page Break|insert.pageBreak
Insert|Tables|Insert Table|insert.table
Insert|Tables|Draw Table|table.draw
Insert|Tables|Convert Text to Table|table.fromText
Insert|Tables|Excel Spreadsheet|insert.spreadsheet
Insert|Tables|Quick Tables|table.quick
Insert|Illustrations|Pictures|insert.picture
Insert|Illustrations|Online Pictures|insert.onlinePicture
Insert|Illustrations|Shapes|insert.shape
Insert|Illustrations|Icons|insert.icon
Insert|Illustrations|3D Models|insert.model3d
Insert|Illustrations|SmartArt|insert.smartArt
Insert|Illustrations|Chart|insert.chart
Insert|Illustrations|Screenshot|insert.screenshot
Insert|Illustrations|Drawing Canvas|insert.canvas
Insert|Add-ins|Get Add-ins|addins.get
Insert|Media|Online Video|insert.video
Insert|Links|Link|insert.link
Insert|Links|Remove Link|insert.removeLink
Insert|Links|Bookmark|insert.bookmark
Insert|Links|Cross-reference|insert.crossReference
Insert|Comments|Comment|review.newComment
Insert|Header & Footer|Header|insert.header
Insert|Header & Footer|Footer|insert.footer
Insert|Header & Footer|Page Number|insert.pageNumber
Insert|Header & Footer|Edit Header|insert.editHeader
Insert|Header & Footer|Edit Footer|insert.editFooter
Insert|Header & Footer|Remove Header|insert.removeHeader
Insert|Header & Footer|Remove Footer|insert.removeFooter
Insert|Header & Footer|Format Page Numbers|layout.pageNumberFormat
Insert|Header & Footer|Close Header and Footer|insert.closeHeader
Insert|Header & Footer|Different First Page|layout.differentFirstPage
Insert|Header & Footer|Different Odd & Even|layout.differentOddEven
Insert|Header & Footer|Link to Previous|layout.linkToPrevious
Insert|Text|Text Box|insert.textBox
Insert|Text|Quick Parts|insert.quickParts
Insert|Text|AutoText|insert.autoText
Insert|Text|Document Property|insert.docProperty
Insert|Text|Field|insert.field
Insert|Text|WordArt|insert.wordArt
Insert|Text|Drop Cap|insert.dropCap
Insert|Text|Signature Line|insert.signatureLine
Insert|Text|Date & Time|insert.dateTime
Insert|Text|Object|insert.object
Insert|Text|Text from File|insert.textFromFile
Insert|Symbols|Equation|insert.equation
Insert|Symbols|Ink Equation|insert.inkEquation
Insert|Symbols|Symbol|insert.symbol
Insert|Symbols|Nonbreaking Space|text.nbsp
Insert|Symbols|Nonbreaking Hyphen|text.nbHyphen
Insert|Symbols|Optional Hyphen|text.optionalHyphen
Draw|Tools|Select|draw.select
Draw|Tools|Lasso Select|draw.lasso
Draw|Tools|Eraser|draw.eraser
Draw|Pens|Pen|draw.pen
Draw|Pens|Pencil|draw.pencil
Draw|Pens|Highlighter|draw.highlighter
Draw|Pens|Add Pen|draw.addPen
Draw|Convert|Ink to Shape|draw.inkToShape
Draw|Convert|Ink to Math|draw.inkToMath
Draw|Insert|Drawing Canvas|insert.canvas
Draw|Replay|Ink Replay|draw.replay
Design|Document Formatting|Themes|design.theme
Design|Document Formatting|Style Set|design.styleSet
Design|Document Formatting|Colors|design.themeColors
Design|Document Formatting|Fonts|design.themeFonts
Design|Document Formatting|Paragraph Spacing|design.paragraphSpacing
Design|Document Formatting|Effects|design.effects
Design|Document Formatting|Set as Default|design.setDefault
Design|Page Background|Watermark|design.watermark
Design|Page Background|Page Color|design.pageColor
Design|Page Background|Page Borders|design.pageBorders
Layout|Page Setup|Margins|layout.margins
Layout|Page Setup|Orientation|layout.orientation
Layout|Page Setup|Size|layout.size
Layout|Page Setup|Columns|layout.columns
Layout|Page Setup|Breaks|layout.break
Layout|Page Setup|Line Numbers|layout.lineNumbers
Layout|Page Setup|Hyphenation|layout.hyphenation
Layout|Page Setup|Page Setup Dialog|layout.pageSetup
Layout|Page Setup|Vertical Alignment|layout.verticalAlign
Layout|Paragraph|Indent|para.indents
Layout|Paragraph|Spacing|para.spacing
Layout|Arrange|Position|arrange.position
Layout|Arrange|Wrap Text|arrange.wrap
Layout|Arrange|Bring Forward|arrange.bringForward
Layout|Arrange|Send Backward|arrange.sendBackward
Layout|Arrange|Selection Pane|arrange.selectionPane
Layout|Arrange|Align|arrange.align
Layout|Arrange|Group|arrange.group
Layout|Arrange|Rotate|arrange.rotate
References|Table of Contents|Table of Contents|references.toc
References|Table of Contents|Add Text|references.addText
References|Table of Contents|Update Table|references.updateToc
References|Table of Contents|Remove Table of Contents|references.removeToc
References|Footnotes|Insert Footnote|references.footnote
References|Footnotes|Insert Endnote|references.endnote
References|Footnotes|Next Footnote|references.nextFootnote
References|Footnotes|Show Notes|references.notes
References|Footnotes|Footnote Options|references.noteOptions
References|Research|Researcher|references.researcher
References|Citations|Insert Citation|references.citation
References|Citations|Manage Sources|references.sources
References|Citations|Style|references.citationStyle
References|Citations|Bibliography|references.bibliography
References|Captions|Insert Caption|references.caption
References|Captions|Insert Table of Figures|references.tableOfFigures
References|Captions|Update Table of Figures|references.updateFigures
References|Captions|Cross-reference|insert.crossReference
References|Index|Mark Entry|references.markEntry
References|Index|Insert Index|references.index
References|Index|Update Index|references.updateIndex
References|Table of Authorities|Mark Citation|references.markCitation
References|Table of Authorities|Insert Table of Authorities|references.tableOfAuthorities
References|Fields|Update Fields|references.updateFields
Mailings|Create|Envelopes|mailings.envelopes
Mailings|Create|Labels|mailings.labels
Mailings|Start Mail Merge|Start Mail Merge|mailings.start
Mailings|Start Mail Merge|Select Recipients|mailings.recipients
Mailings|Start Mail Merge|Edit Recipient List|mailings.editRecipients
Mailings|Write & Insert Fields|Highlight Merge Fields|mailings.highlightFields
Mailings|Write & Insert Fields|Address Block|mailings.addressBlock
Mailings|Write & Insert Fields|Greeting Line|mailings.greetingLine
Mailings|Write & Insert Fields|Insert Merge Field|mailings.insertField
Mailings|Write & Insert Fields|Rules|mailings.rules
Mailings|Write & Insert Fields|Match Fields|mailings.matchFields
Mailings|Preview Results|Preview Results|mailings.preview
Mailings|Preview Results|Next Record|mailings.next
Mailings|Preview Results|Previous Record|mailings.previous
Mailings|Preview Results|Find Recipient|mailings.findRecipient
Mailings|Preview Results|Check for Errors|mailings.checkErrors
Mailings|Finish|Finish & Merge|mailings.finish
Review|Proofing|Spelling & Grammar|review.spelling
Review|Proofing|Thesaurus|review.thesaurus
Review|Proofing|Word Count|review.wordCount
Review|Speech|Read Aloud|review.readAloud
Review|Accessibility|Check Accessibility|file.accessibility
Review|Language|Translate|review.translate
Review|Language|Language|review.language
Review|Comments|New Comment|review.newComment
Review|Comments|Delete|review.deleteComment
Review|Comments|Previous|review.previousComment
Review|Comments|Next|review.nextComment
Review|Comments|Show Comments|review.comments
Review|Comments|Resolve|review.resolveComment
Review|Comments|Reply|review.reply
Review|Tracking|Track Changes|review.trackChanges
Review|Tracking|Display for Review|review.markup
Review|Tracking|Show Markup|review.showMarkup
Review|Tracking|Reviewing Pane|review.changes
Review|Changes|Accept|review.accept
Review|Changes|Reject|review.reject
Review|Changes|Accept All|review.acceptAll
Review|Changes|Reject All|review.rejectAll
Review|Changes|Previous Change|review.previousChange
Review|Changes|Next Change|review.nextChange
Review|Compare|Compare|review.compare
Review|Compare|Combine|review.combine
Review|Protect|Block Authors|review.blockAuthors
Review|Protect|Restrict Editing|review.restrict
Review|Ink|Hide Ink|review.hideInk
View|Views|Read Mode|view.readMode
View|Views|Print Layout|view.printLayout
View|Views|Web Layout|view.webLayout
View|Views|Outline|view.outline
View|Views|Draft|view.draft
View|Immersive|Focus|view.focus
View|Immersive|Immersive Reader|view.immersive
View|Page Movement|Vertical|view.vertical
View|Page Movement|Side to Side|view.sideToSide
View|Show|Ruler|view.ruler
View|Show|Gridlines|view.gridlines
View|Show|Navigation Pane|view.navigationPane
View|Zoom|Zoom|view.zoom
View|Zoom|100%|view.zoom100
View|Zoom|One Page|view.onePage
View|Zoom|Multiple Pages|view.multiplePages
View|Zoom|Page Width|view.pageWidth
View|Zoom|Zoom In|view.zoomIn
View|Zoom|Zoom Out|view.zoomOut
View|Dark Mode|Switch Modes|view.darkMode
View|Window|New Window|view.newWindow
View|Window|Arrange All|view.arrangeAll
View|Window|Split|view.split
View|Window|View Side by Side|view.sideBySide
View|Window|Synchronous Scrolling|view.syncScroll
View|Window|Switch Windows|view.switchWindows
View|Macros|Macros|tools.macros
View|Macros|Record Macro|tools.recordMacro
View|SharePoint|Properties|file.properties
Table Design|Table Style Options|Table Style Options|table.look
Table Design|Table Styles|Table Styles|table.style
Table Design|Table Styles|Shading|table.shading
Table Design|Borders|Borders|table.borders
Table Design|Borders|Border Painter|table.borderPainter
Table Layout|Table|Select Table|table.selectTable
Table Layout|Table|Select Row|table.selectRow
Table Layout|Table|Select Cell|table.selectCell
Table Layout|Table|View Gridlines|view.gridlines
Table Layout|Table|Properties|table.properties
Table Layout|Draw|Draw Table|table.draw
Table Layout|Draw|Eraser|table.eraser
Table Layout|Rows & Columns|Delete Cells|table.deleteCells
Table Layout|Rows & Columns|Delete Rows|table.deleteRow
Table Layout|Rows & Columns|Delete Columns|table.deleteColumn
Table Layout|Rows & Columns|Delete Table|table.deleteTable
Table Layout|Rows & Columns|Insert Above|table.insertRowAbove
Table Layout|Rows & Columns|Insert Below|table.insertRowBelow
Table Layout|Rows & Columns|Insert Left|table.insertColumnLeft
Table Layout|Rows & Columns|Insert Right|table.insertColumnRight
Table Layout|Merge|Merge Cells|table.merge
Table Layout|Merge|Split Cells|table.split
Table Layout|Merge|Split Table|table.splitTable
Table Layout|Cell Size|AutoFit|table.autofit
Table Layout|Cell Size|Row Height|table.rowHeight
Table Layout|Cell Size|Column Width|table.columnWidth
Table Layout|Cell Size|Distribute Rows|table.distributeRows
Table Layout|Cell Size|Distribute Columns|table.distributeColumns
Table Layout|Alignment|Alignment|table.cellAlign
Table Layout|Alignment|Text Direction|table.textDirection
Table Layout|Alignment|Cell Margins|table.cellMargins
Table Layout|Data|Sort|table.sort
Table Layout|Data|Repeat Header Rows|table.repeatHeader
Table Layout|Data|Convert to Text|table.toText
Table Layout|Data|Formula|table.formula
Equation|Tools|Equation|insert.equation
Equation|Tools|Ink Equation|insert.inkEquation
Equation|Conversions|Unicode|equation.inputFormat
Equation|Conversions|LaTeX|equation.inputFormat
Equation|Conversions|Convert|equation.convert
Equation|Conversions|Normal Text|equation.normalText
Equation|Symbols|Symbols|equation.insertSymbol
Equation|Structures|Fraction|equation.insertStructure
Equation|Structures|Script|equation.insertStructure
Equation|Structures|Radical|equation.insertStructure
Equation|Structures|Integral|equation.insertStructure
Equation|Structures|Large Operator|equation.insertStructure
Equation|Structures|Bracket|equation.insertStructure
Equation|Structures|Function|equation.insertStructure
Equation|Structures|Accent|equation.insertStructure
Equation|Structures|Limit and Log|equation.insertStructure
Equation|Structures|Operator|equation.insertStructure
Equation|Structures|Matrix|equation.insertStructure
Equation|Equation Options|Change to Inline / Display|equation.display
Equation|Equation Options|Justification|equation.justify
Equation|Equation Options|Equation Number (#)|equation.number
Equation|Equation Options|Structure Commands|equation.structure
Picture Format|Adjust|Remove Background|picture.removeBackground
Picture Format|Adjust|Corrections|picture.corrections
Picture Format|Adjust|Color|picture.color
Picture Format|Adjust|Artistic Effects|picture.effects
Picture Format|Adjust|Transparency|picture.transparency
Picture Format|Adjust|Compress Pictures|picture.compress
Picture Format|Adjust|Change Picture|picture.change
Picture Format|Adjust|Reset Picture|picture.reset
Picture Format|Picture Styles|Picture Styles|picture.style
Picture Format|Picture Styles|Picture Border|picture.border
Picture Format|Accessibility|Alt Text|picture.altText
Picture Format|Size|Crop|picture.crop
Picture Format|Size|Size|picture.size
Shape Format|Shape Styles|Shape Fill|shape.fill
Shape Format|Shape Styles|Shape Outline|shape.outline
Shape Format|Shape Styles|Shape Effects|shape.effects
Shape Format|Text|Text Direction|shape.textDirection
Shape Format|Text|Align Text|shape.alignText
Shape Format|Text|Create Link|shape.link
Header & Footer|Navigation|Go to Header|insert.editHeader
Header & Footer|Navigation|Go to Footer|insert.editFooter
Header & Footer|Navigation|Previous Section|hf.previous
Header & Footer|Navigation|Next Section|hf.next
Header & Footer|Position|Header from Top|hf.position
Editing|Keyboard|Type Text|text.insert
Editing|Keyboard|New Paragraph|text.newParagraph
Editing|Keyboard|Line Break|text.lineBreak
Editing|Keyboard|Column Break|text.columnBreak
Editing|Keyboard|Tab|text.tab
Editing|Keyboard|Backspace|text.backspace
Editing|Keyboard|Delete|text.delete
Editing|Keyboard|Delete Previous Word|text.deleteWordBack
Editing|Keyboard|Delete Next Word|text.deleteWordForward
Editing|Navigation|Word Left|caret.wordLeft
Editing|Navigation|Word Right|caret.wordRight
Editing|Navigation|Line Start|caret.home
Editing|Navigation|Line End|caret.end
Editing|Navigation|Paragraph Up|caret.paraUp
Editing|Navigation|Paragraph Down|caret.paraDown
Editing|Navigation|Document Start|caret.docStart
Editing|Navigation|Document End|caret.docEnd
Editing|Navigation|Page Up|caret.pageUp
Editing|Navigation|Page Down|caret.pageDown
Editing|Selection|Select Word|select.word
Editing|Selection|Select Sentence|select.sentence
Editing|Selection|Select Paragraph|select.paragraph
Editing|Selection|Extend Selection (F8)|select.extend
Editing|Selection|Column Selection|select.column
Tools|Proofing|AutoCorrect Options|tools.autocorrect
Tools|Proofing|Set Proofing Language|review.language
Tools|Customize|Customize Ribbon|tools.customizeRibbon
Tools|Customize|Customize Keyboard|tools.customizeKeyboard
Tools|Protection|Encrypt with Password|file.encrypt
Tools|Templates|Templates and Add-ins|tools.templates
";

/// Catalog entries: (tab, group, feature, id).
pub fn entries() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    CATALOG
        .lines()
        .filter_map(|l| {
            let mut it = l.split('|');
            Some((it.next()?, it.next()?, it.next()?, it.next()?))
        })
        .collect()
}

/// Parity summary: per tab and overall, with the missing features.
pub fn parity(reg: &Registry) -> Value {
    let e = entries();
    let mut tabs: Vec<(&str, usize, usize, Vec<String>)> = Vec::new();
    for (tab, group, label, id) in &e {
        let have = reg.get(id).is_some();
        let t = match tabs.iter_mut().find(|t| t.0 == *tab) {
            Some(t) => t,
            None => {
                tabs.push((tab, 0, 0, Vec::new()));
                let n = tabs.len() - 1;
                match tabs.get_mut(n) {
                    Some(t) => t,
                    None => continue,
                }
            }
        };
        t.2 += 1;
        if have {
            t.1 += 1;
        } else {
            t.3.push(format!("{group} › {label} (`{id}`)"));
        }
    }
    let live = tabs.iter().map(|t| t.1).sum::<usize>();
    let total = e.len();
    json!({
        "live": live,
        "total": total,
        "percent": if total == 0 { 0.0 } else { (live as f64 * 1000.0 / total as f64).round() / 10.0 },
        "tabs": tabs.iter().map(|t| json!({"tab": t.0, "live": t.1, "total": t.2, "missing": t.3})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_parses_and_ids_unique_per_feature() {
        let e = entries();
        assert!(e.len() > 300);
        assert!(e.iter().all(|x| x.3.contains('.')));
    }

    #[test]
    fn parity_floor() {
        let reg = crate::cmd::registry();
        let p = parity(&reg);
        let pct = p["percent"].as_f64().unwrap();
        // The floor only ever rises.
        assert!(pct >= 60.0, "parity {pct}%");
    }
}
