//! Engine results are unchanged for scripts and MCP. Only their interface presentation uses
//! translated templates; numbers and the proofing rule's quoted word remain as supplied.

use super::{fmt, t};

/// A proofing explanation in the interface language, preserving the word from the document.
pub(crate) fn proof_message(message: &str) -> String {
    if let Some(word) = message.strip_prefix("Repeated word: \"").and_then(|s| s.strip_suffix('"')) {
        return fmt(t("Repeated word: \"{word}\""), &[("word", word)]);
    }
    t(message).to_string()
}

/// The checker's dynamic findings carry their values in the English message. Recognize only
/// its current numeric forms; a changed or unknown result retains the original wording.
pub(crate) fn accessibility_issue(rule: &str, message: &str) -> String {
    let translated = match rule {
        "headingOrder" => heading_order(message),
        "contrast" => contrast(message),
        "blankParagraphs" => numbered(message, "", " empty paragraphs in a row", "{n} empty paragraphs in a row", "n"),
        "blankRow" => {
            numbered(message, "Row ", " of a table is empty: it looks like spacing", "Row {row} of a table is empty: it looks like spacing", "row")
        }
        "blankColumn" => numbered(
            message,
            "Column ",
            " of a table is empty: it looks like spacing",
            "Column {column} of a table is empty: it looks like spacing",
            "column",
        ),
        _ => None,
    };
    translated.unwrap_or_else(|| t(message).to_string())
}

fn numbered(message: &str, prefix: &str, suffix: &str, template: &str, name: &str) -> Option<String> {
    let n = message.strip_prefix(prefix)?.strip_suffix(suffix)?;
    n.parse::<usize>().ok()?;
    Some(fmt(t(template), &[(name, n)]))
}

fn heading_order(message: &str) -> Option<String> {
    let (level, previous) = message.strip_prefix("Heading level skipped (Heading ")?.strip_suffix(')')?.split_once(" after Heading ")?;
    level.parse::<u8>().ok()?;
    previous.parse::<u8>().ok()?;
    Some(fmt(t("Heading level skipped (Heading {level} after Heading {previous})"), &[("level", level), ("previous", previous)]))
}

fn contrast(message: &str) -> Option<String> {
    let (ratio, needed) = message.strip_prefix("Text is hard to read: contrast ")?.strip_suffix(":1 needed")?.split_once(":1, ")?;
    if ![ratio, needed].iter().all(|s| s.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= 0.0)) {
        return None;
    }
    Some(fmt(t("Text is hard to read: contrast {ratio}:1, {needed}:1 needed"), &[("ratio", ratio), ("needed", needed)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, fixture_lang, set_current};
    use serde_json::json;
    use std::sync::Arc;
    use wordcraft_doc::props::{CharProps, Rgb, TextColor};
    use wordcraft_doc::{Block, Document, Paragraph, para_block};

    fn japanese() -> Lang {
        fixture_lang(
            "ja",
            "\tHeading level skipped (Heading {level} after Heading {previous})\t見出しレベルが飛んでいます (見出し {previous} の後に見出し {level})\n\t{n} empty paragraphs in a row\t{n} 個の空の段落が連続しています\n\tRow {row} of a table is empty: it looks like spacing\t表の {row} 行目が空です。間隔を空けるための行に見えます\n\tColumn {column} of a table is empty: it looks like spacing\t表の {column} 列目が空です。間隔を空けるための列に見えます\n\tText is hard to read: contrast {ratio}:1, {needed}:1 needed\t文字のコントラストが低いため読みづらくなっています ({ratio}:1、必要な比率 {needed}:1)\n\tRepeated word: \"{word}\"\t単語が重複しています: \"{word}\"\n",
        )
    }

    fn checker_document() -> Document {
        let mut doc = Document::new();
        let mut table = wordcraft_doc::table::Table::new(2, 2, 240.0);
        table.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("Data", CharProps::default()))];
        doc.body = vec![
            para_block(Paragraph::with_text("First", CharProps::default()).styled("Heading1")),
            para_block(Paragraph::with_text("Third", CharProps::default()).styled("Heading3")),
            para_block(Paragraph::new()),
            para_block(Paragraph::new()),
            para_block(Paragraph::new()),
            para_block(Paragraph::with_text("Light text", CharProps { color: Some(TextColor::Rgb(Rgb(230, 230, 230))), ..Default::default() })),
            Arc::new(Block::Table(table)),
        ];
        doc
    }

    #[test]
    fn presenting_checker_results_preserves_the_report_and_english_wording() {
        let doc = checker_document();
        let before = serde_json::to_value(&doc).unwrap();
        let report = wordcraft_engine::cmd::accessibility::check(&doc);
        for rule in ["headingOrder", "contrast", "blankParagraphs", "blankRow", "blankColumn"] {
            assert!(report.iter().any(|i| i["rule"] == rule), "the fixture exercises {rule}");
        }
        set_current(Lang::EN);
        for issue in &report {
            let message = issue["issue"].as_str().unwrap();
            assert_eq!(accessibility_issue(issue["rule"].as_str().unwrap(), message), message);
        }
        set_current(japanese());
        let shown = |rule: &str| {
            let issue = report.iter().find(|i| i["rule"] == rule).unwrap();
            accessibility_issue(rule, issue["issue"].as_str().unwrap())
        };
        assert_eq!(shown("headingOrder"), "見出しレベルが飛んでいます (見出し 1 の後に見出し 3)");
        assert_eq!(shown("blankParagraphs"), "3 個の空の段落が連続しています");
        assert!(shown("blankRow").starts_with("表の 2 行目"));
        assert!(shown("blankColumn").starts_with("表の 2 列目"));
        assert!(shown("contrast").contains("必要な比率 4.5:1"));
        set_current(Lang::EN);
        assert_eq!(wordcraft_engine::cmd::accessibility::check(&doc), report);
        assert_eq!(serde_json::to_value(&doc).unwrap(), before);
    }

    #[test]
    fn proofing_presentation_preserves_unicode_words_and_literal_placeholders() {
        let mut session = wordcraft_engine::Session::new(Document::from_text("café café"));
        let before = serde_json::to_value(&session.doc).unwrap();
        let result = session.run("review.issues", &json!({})).unwrap();
        let message = result.as_array().unwrap().iter().filter_map(|i| i["message"].as_str()).find(|m| m.starts_with("Repeated word:")).unwrap();
        set_current(japanese());
        assert_eq!(proof_message(message), "単語が重複しています: \"café\"");
        let word = "あア亜 {word} \"café\"";
        let supplied = format!("Repeated word: \"{word}\"");
        assert_eq!(proof_message(&supplied), format!("単語が重複しています: \"{word}\""));
        set_current(Lang::EN);
        assert_eq!(proof_message(message), message);
        assert_eq!(proof_message(&supplied), supplied);
        assert_eq!(session.run("review.issues", &json!({})).unwrap(), result);
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }

    #[test]
    fn changed_or_malformed_checker_messages_keep_their_original_text() {
        set_current(japanese());
        for (rule, message) in [
            ("headingOrder", "Heading level skipped (Heading {level} after Heading 1)"),
            ("blankRow", "Row あ of a table is empty: it looks like spacing"),
            ("blankColumn", "Column 2 of a table is empty: changed wording"),
            ("contrast", "Text is hard to read: contrast NaN:1, 4.5:1 needed"),
            ("futureRule", "ユーザーの値 {n}"),
        ] {
            assert_eq!(accessibility_issue(rule, message), message);
        }
        assert_eq!(proof_message("Repeated word: \"unfinished"), "Repeated word: \"unfinished");
        set_current(Lang::EN);
    }
}
