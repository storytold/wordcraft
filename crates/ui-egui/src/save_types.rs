//! The file types Save As offers, in the order the dialog lists them.

use craft::host::FileFilter;

/// The extensions Save As offers: every format the engine saves, with the suggested name's own
/// format first. A picker makes a filter's first extension the dialog's default, which the
/// panels append to a file name typed without one; with no filter at all the name came back bare
/// and the save failed, since the format is chosen by extension (#689).
fn save_exts(name: &str) -> Vec<String> {
    let suggested = std::path::Path::new(name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase)
        // A .markdown file is under Markdown's one entry in the list.
        .map(|e| if e == "markdown" { "md".to_string() } else { e })
        .filter(|e| wordcraft_engine::io::SAVE_EXTS.contains(&e.as_str()))
        .unwrap_or_else(|| "docx".to_string());
    let mut exts: Vec<String> = Vec::with_capacity(wordcraft_engine::io::SAVE_EXTS.len());
    exts.push(suggested.clone());
    exts.extend(wordcraft_engine::io::SAVE_EXTS.iter().filter(|e| **e != suggested).map(|e| (*e).to_string()));
    exts
}

/// The "Save as type" list (#1399): one named entry per format, the suggested one first (see
/// [`save_exts`] for why the order matters). Markdown's entry also takes `.markdown`.
pub fn save_filters(name: &str) -> Vec<FileFilter> {
    save_exts(name)
        .into_iter()
        .map(|e| {
            let label = format!("{} (*.{e})", format_name(&e));
            if e == "md" { FileFilter::new(label, &["md", "markdown"]) } else { FileFilter::new(label, &[e.as_str()]) }
        })
        .collect()
}

/// A readable name for a save format's extension.
fn format_name(ext: &str) -> String {
    match ext {
        "docx" => "Word Document".into(),
        "docm" => "Word Macro-Enabled Document".into(),
        "dotx" => "Word Template".into(),
        "dotm" => "Word Macro-Enabled Template".into(),
        "pdf" => "PDF".into(),
        "txt" => "Plain Text".into(),
        "md" => "Markdown".into(),
        "html" => "Web Page".into(),
        "rtf" => "Rich Text Format".into(),
        "odt" => "OpenDocument Text".into(),
        "ott" => "OpenDocument Text Template".into(),
        "fodt" => "Flat OpenDocument Text".into(),
        "tex" => "LaTeX".into(),
        "png" => "PNG Image".into(),
        "json" => "JSON".into(),
        "xml" => "XML".into(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::save_filters;

    /// #1399, #689: the type list has one entry per format, the suggested name's own format first
    /// (a name typed without an extension then gets it), every savable extension exactly once.
    #[test]
    fn save_dialog_lists_one_named_type_per_format() {
        let filters = save_filters("report.odt");
        assert_eq!(filters.len(), wordcraft_engine::io::SAVE_EXTS.len());
        assert_eq!(filters[0].extensions, vec!["odt".to_string()], "the suggested format is first");
        let all: Vec<&String> = filters.iter().flat_map(|f| &f.extensions).collect();
        for e in wordcraft_engine::io::SAVE_EXTS {
            assert_eq!(all.iter().filter(|x| x.as_str() == *e).count(), 1, "{e}");
        }
        // A .markdown name is Markdown's own entry, first, and nothing lists the extension twice.
        let markdown = save_filters("notes.markdown");
        assert_eq!(markdown[0].extensions.first().map(String::as_str), Some("md"));
        assert!(markdown[0].extensions.iter().any(|e| e == "markdown"));
        let every: Vec<&String> = markdown.iter().flat_map(|f| &f.extensions).collect();
        assert_eq!(every.iter().filter(|x| x.as_str() == "markdown").count(), 1);
        // Unknown names fall back to Word's format first.
        assert_eq!(save_filters("no extension")[0].extensions, vec!["docx".to_string()]);
        assert_eq!(save_filters("mystery.rtf2")[0].extensions, vec!["docx".to_string()]);
    }
}
