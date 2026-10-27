use super::*;

fn to_source_type(kind: &str) -> &str {
    match kind.to_ascii_lowercase().as_str() {
        "book" => "Book",
        "booksection" | "chapter" => "BookSection",
        "article" | "journal" | "journalarticle" => "JournalArticle",
        "periodical" | "articleinaperiodical" => "ArticleInAPeriodical",
        "website" | "site" | "internetsite" => "InternetSite",
        "documentfrominternetsite" => "DocumentFromInternetSite",
        "electronicsource" => "ElectronicSource",
        "report" => "Report",
        "conference" | "proceedings" | "conferenceproceedings" => "ConferenceProceedings",
        "case" => "Case",
        "art" => "Art",
        "film" => "Film",
        "interview" => "Interview",
        "patent" => "Patent",
        "performance" => "Performance",
        "soundrecording" => "SoundRecording",
        "standard" => "Standard",
        "thesis" => "Thesis",
        "other" | "misc" => "Misc",
        _ => "Book",
    }
}

pub(crate) fn from_source_type(st: &str) -> String {
    match st.trim() {
        "Book" | "book" => "book".into(),
        "BookSection" | "booksection" => "bookSection".into(),
        "JournalArticle" | "journalarticle" | "article" => "article".into(),
        "ArticleInAPeriodical" | "articleinaperiodical" => "periodical".into(),
        "InternetSite" | "internetsite" | "website" => "website".into(),
        "DocumentFromInternetSite" | "documentfrominternetsite" => "website".into(),
        "ElectronicSource" | "electronicsource" => "electronicsource".into(),
        "Report" | "report" => "report".into(),
        "ConferenceProceedings" | "conferenceproceedings" => "conference".into(),
        "Case" | "case" => "case".into(),
        "SoundRecording" | "soundrecording" => "soundrecording".into(),
        "Film" | "film" => "film".into(),
        "Interview" | "interview" => "interview".into(),
        "Patent" | "patent" => "patent".into(),
        "Performance" | "performance" => "performance".into(),
        "Standard" | "standard" => "standard".into(),
        "Thesis" | "thesis" => "thesis".into(),
        "Misc" | "misc" => "other".into(),
        "" => "book".into(),
        other => other.to_ascii_lowercase(),
    }
}

/// The document's citation style, APA when it has none, and a style sheet file name made from
/// it (`\APA.XSL`, `\MLA.XSL`).
fn bib_style(doc: &Document) -> (&str, String) {
    let name = doc.settings.bib_style.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or("APA");
    let file: String = name.chars().filter(char::is_ascii_alphanumeric).collect();
    (name, format!("\\{}.XSL", if file.is_empty() { "APA" } else { &file }))
}

fn write_name_list(w: &mut W, names: &str) {
    w.open("b:NameList", &[]);
    let persons: Vec<_> = names.split(';').map(str::trim).filter(|s| !s.is_empty()).collect();
    for a in persons {
        w.open("b:Person", &[]);
        // Parse "Last, First" or "Last, First Middle"
        let (last, first_middle) = match a.split_once(',') {
            Some((l, f)) => (l.trim(), f.trim()),
            None => (a, ""),
        };
        w.leaf("b:Last", &[], last);
        if !first_middle.is_empty() {
            let parts: Vec<&str> = first_middle.split_whitespace().collect();
            if let Some(first) = parts.first() {
                w.leaf("b:First", &[], first);
            }
            if let Some(slice) = parts.get(1..).filter(|s| !s.is_empty()) {
                let mid = slice.join(" ");
                w.leaf("b:Middle", &[], &mid);
            }
        }
        w.close("b:Person");
    }
    w.close("b:NameList");
}

fn split_accessed_date(s: &str) -> (Option<&str>, Option<&str>, Option<&str>) {
    let s = s.trim();
    if s.is_empty() {
        return (None, None, None);
    }
    let parts: Vec<&str> = if s.contains('-') {
        s.split('-').collect()
    } else if s.contains('/') {
        s.split('/').collect()
    } else {
        Vec::new()
    };
    if let [y, m, d] = parts.as_slice() {
        (Some(y.trim()), Some(m.trim()), Some(d.trim()))
    } else if let [y, m] = parts.as_slice() {
        (Some(y.trim()), Some(m.trim()), None)
    } else {
        (Some(s), None, None)
    }
}

pub(super) fn bibliography_xml(doc: &Document) -> Vec<u8> {
    let (style_name, selected) = bib_style(doc);
    let mut w = W::new();
    w.open(
        "b:Sources",
        &[
            ("SelectedStyle", selected.as_str()),
            ("StyleName", style_name),
            ("xmlns:b", "http://schemas.openxmlformats.org/officeDocument/2006/bibliography"),
            ("xmlns", "http://schemas.openxmlformats.org/officeDocument/2006/bibliography"),
        ],
    );
    for src in &doc.sources {
        w.open("b:Source", &[]);
        w.leaf("b:Tag", &[], &src.tag);
        w.leaf("b:SourceType", &[], to_source_type(&src.kind));
        // Author & Editor
        if !src.author.is_empty() || !src.editor.is_empty() {
            w.open("b:Author", &[]);
            if !src.author.is_empty() {
                w.open("b:Author", &[]);
                write_name_list(&mut w, &src.author);
                w.close("b:Author");
            }
            if !src.editor.is_empty() {
                w.open("b:Editor", &[]);
                write_name_list(&mut w, &src.editor);
                w.close("b:Editor");
            }
            w.close("b:Author");
        }
        if !src.title.is_empty() {
            w.leaf("b:Title", &[], &src.title);
        }
        if !src.year.is_empty() {
            w.leaf("b:Year", &[], &src.year);
        }
        if !src.month.is_empty() {
            w.leaf("b:Month", &[], &src.month);
        }
        if !src.day.is_empty() {
            w.leaf("b:Day", &[], &src.day);
        }
        if !src.city.is_empty() {
            w.leaf("b:City", &[], &src.city);
        }
        if !src.publisher.is_empty() {
            w.leaf("b:Publisher", &[], &src.publisher);
        }
        if !src.journal.is_empty() {
            w.leaf("b:JournalName", &[], &src.journal);
        }
        if !src.edition.is_empty() {
            w.leaf("b:Edition", &[], &src.edition);
        }
        if !src.volume.is_empty() {
            w.leaf("b:Volume", &[], &src.volume);
        }
        if !src.issue.is_empty() {
            w.leaf("b:Issue", &[], &src.issue);
        }
        if !src.pages.is_empty() {
            w.leaf("b:Pages", &[], &src.pages);
        }
        if !src.url.is_empty() {
            w.leaf("b:URL", &[], &src.url);
        }
        if !src.accessed.is_empty() {
            let (y, m, d) = split_accessed_date(&src.accessed);
            if let Some(y) = y.filter(|s| !s.is_empty()) {
                w.leaf("b:YearAccessed", &[], y);
            }
            if let Some(m) = m.filter(|s| !s.is_empty()) {
                w.leaf("b:MonthAccessed", &[], m);
            }
            if let Some(d) = d.filter(|s| !s.is_empty()) {
                w.leaf("b:DayAccessed", &[], d);
            }
            w.leaf("b:Accessed", &[], &src.accessed);
        }
        if !src.doi.is_empty() {
            w.leaf("b:DOI", &[], &src.doi);
            w.leaf("b:StandardNumber", &[], &src.doi);
        }
        w.close("b:Source");
    }
    w.close("b:Sources");
    w.into_bytes()
}

pub(super) fn bibliography_props_xml() -> Vec<u8> {
    let mut w = W::new();
    w.open(
        "ds:datastoreItem",
        &[("xmlns:ds", "http://schemas.openxmlformats.org/officeDocument/2006/customXml"), ("ds:itemID", "{3315B9A8-2323-4554-B8B2-1234567890AB}")],
    );
    w.open("ds:schemaRefs", &[]);
    w.empty("ds:schemaRef", &[("ds:uri", "http://schemas.openxmlformats.org/officeDocument/2006/bibliography")]);
    w.close("ds:schemaRefs");
    w.close("ds:datastoreItem");
    w.into_bytes()
}
