//! Standard text/PAGE/IF fallback inside an identifiable content control. WordCraft restores
//! the original PAGE field on import, avoiding doubled prefixes on repeated saves.
use super::field::{FieldData, field_run};
use super::{CharProps, W, Writer};
use wordcraft_doc::fields::number::{field_number, format_switch, labelled_number};
use wordcraft_doc::page_label::PageLabelMode;
use wordcraft_doc::section::NumFormat;

impl Writer<'_> {
    pub(super) fn labelled_page_field(&mut self, w: &mut W, field: &FieldData<'_>, props: &CharProps) -> bool {
        if !field.instr.split_whitespace().next().is_some_and(|s| s.eq_ignore_ascii_case("PAGE")) {
            return false;
        }
        let Some((label, section_format, start)) = self.page_label.take() else { return false };
        let format = format_switch(field.instr).unwrap_or(section_format);
        let deleted = self.is_del(props);
        if !field.locked {
            self.rev_open(w, props);
        }
        let target = w;
        let mut content = W::default();
        let w = &mut content;
        w.open("w:sdtContent", &[]);
        let text =
            |w: &mut W, text: &str| field_run(w, props, &|w| w.leaf(if deleted { "w:delText" } else { "w:t" }, &[("xml:space", "preserve")], text));
        let code = |w: &mut W, text: &str| {
            field_run(w, props, &|w| w.leaf(if deleted { "w:delInstrText" } else { "w:instrText" }, &[("xml:space", "preserve")], text))
        };
        let mark = |w: &mut W, kind: &str| field_run(w, props, &|w| w.empty("w:fldChar", &[("w:fldCharType", kind)]));
        let prefix = if label.mode == PageLabelMode::TextNumber && !label.text.is_empty() {
            format!("{}{}", label.text, label.separator)
        } else {
            String::new()
        };
        // The complete replacement shares the original revision boundary. Nested fields
        // write their controls directly so revisions do not nest or omit the literal label.
        let page = |writer: &Self, w: &mut W, instr: &str, result: &str| {
            writer.field_start(w, instr, false, true, "", props);
            text(w, result);
            mark(w, "end");
        };
        let result = field_number(field.instr, format, start);
        if field.locked {
            // The context is taken above, so the ordinary writer preserves this locked field.
            self.field(w, FieldData { instr: field.instr, result: field.result, locked: true, dirty: field.dirty, form: field.form }, props);
        } else if !label.counts() {
            text(w, &labelled_number(field.instr, Some(&label), format, start));
        } else if matches!(format, NumFormat::UpperRoman | NumFormat::LowerRoman | NumFormat::UpperLetter | NumFormat::LowerLetter) {
            let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            mark(w, "begin");
            code(w, " IF ");
            page(self, w, "PAGE \\* ARABIC", &start.to_string());
            code(w, &format!(" = 0 \"{}\" \"{}", quote(&label.zero_label), quote(&prefix)));
            page(self, w, field.instr, &result);
            code(w, "\" ");
            mark(w, "separate");
            text(w, &labelled_number(field.instr, Some(&label), format, start));
            mark(w, "end");
        } else {
            if !prefix.is_empty() {
                text(w, &prefix);
            }
            page(self, w, field.instr, &result);
        }
        w.close("w:sdtContent");
        let fingerprint = crate::xml::parse(
            format!("<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</root>", content.s).as_bytes(),
        )
        .ok()
        .and_then(|root| crate::page_labels::content_hash(&root));
        let w = target;
        w.open("w:sdt", &[("wc:pageField", field.instr), ("mc:Ignorable", "wc"), ("wc:locked", if field.locked { "1" } else { "0" })]);
        w.open("w:sdtPr", &[]);
        let tag = self.page_label_metadata.insert(&label, field.instr, fingerprint).unwrap_or_else(|| "WordCraft.PageLabel".into());
        w.empty("w:tag", &[("w:val", &tag)]);
        w.close("w:sdtPr");
        w.raw(&content.s);
        w.close("w:sdt");
        if !field.locked {
            self.rev_close(w, props);
        }
        self.page_label = Some((label, section_format, start));
        true
    }
}
