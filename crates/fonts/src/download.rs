//! Fonts a document may ask for that their publisher gives away, and where: when one isn't
//! installed, the app can say where to get it instead of only naming the substitute.
//!
//! Only a publisher's own page is listed, for fonts it offers for download at no charge. The app
//! links to the page; it never downloads, bundles or redistributes the fonts, and following the
//! publisher's licence terms is the user's part. A font whose publisher limits who may use it
//! is marked [`FontDownload::restricted`], so the app can say so beside the link. Most Office
//! fonts (Calibri, Cambria, Consolas, Segoe UI…) come only with Windows or Office, so they are
//! not here.
//!
//! The list is `font_downloads.json` beside this file, built into the app: adding a font is a
//! new entry there.

/// The list: `{"downloads": [{"family", "publisher", "url", "restricted"?, "note"?}]}`.
const LIST: &str = include_str!("font_downloads.json");

/// Most entries read from the list.
const MAX_DOWNLOADS: usize = 1024;

/// Where a font family can be downloaded at no charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontDownload {
    /// The name of the download: the family or collection the page offers. It covers the
    /// family itself and every name that continues it after a space ("Aptos" covers "Aptos
    /// Display", "Aptos Narrow", "Aptos Mono"…).
    pub name: String,
    /// Who publishes the font and hosts the page.
    pub publisher: String,
    /// The publisher's download page.
    pub url: String,
    /// The publisher limits who may use the font or what for (its design resources for one
    /// platform, say): worth saying beside the link.
    pub restricted: bool,
}

/// The entries of `list` that name a family, a publisher and an `https` page; others are left
/// out, and a list that isn't JSON has none.
fn parse(list: &str) -> Vec<FontDownload> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(list) else {
        log::warn!("font downloads: the list isn't JSON");
        return Vec::new();
    };
    let entries = json.get("downloads").and_then(|d| d.as_array()).map(Vec::as_slice).unwrap_or_default();
    entries
        .iter()
        .take(MAX_DOWNLOADS)
        .filter_map(|e| {
            let field = |k: &str| e.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
            let (name, publisher, url) = (field("family")?, field("publisher")?, field("url")?);
            let restricted = e.get("restricted").and_then(|v| v.as_bool()).unwrap_or(false);
            url.starts_with("https://").then(|| FontDownload { name: name.into(), publisher: publisher.into(), url: url.into(), restricted })
        })
        .collect()
}

/// The built-in list, read once.
fn downloads() -> &'static [FontDownload] {
    static DOWNLOADS: std::sync::OnceLock<Vec<FontDownload>> = std::sync::OnceLock::new();
    DOWNLOADS.get_or_init(|| parse(LIST))
}

/// Where `family` can be downloaded free of charge from its publisher, if it can. The name is
/// matched whatever its case, with a charset suffix ("Aptos CE") or alternates ("Aptos;Arial")
/// as a document may write it.
pub fn download_for(family: &str) -> Option<&'static FontDownload> {
    family.split(';').map(str::trim).filter(|f| !f.is_empty()).find_map(|f| {
        let f = crate::word::charset_base_family(f).trim();
        downloads().iter().find(|d| {
            // Compared as bytes: `get` on a `str` would refuse a split inside a character.
            let (name, f) = (d.name.as_bytes(), f.as_bytes());
            f.get(..name.len()).is_some_and(|head| head.eq_ignore_ascii_case(name)) && matches!(f.get(name.len()), None | Some(b' '))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsoft_fonts_offered_free_of_charge_have_a_download_page() {
        let aptos = download_for("Aptos").unwrap();
        assert_eq!(aptos.publisher, "Microsoft");
        assert!(aptos.url.starts_with("https://www.microsoft.com/"));
        for family in ["aptos", "Aptos Display", "APTOS NARROW", "Aptos Mono", "Aptos Serif", " Aptos ", "Aptos;Arial", "Aptos CE"] {
            assert_eq!(download_for(family), Some(aptos), "{family}");
        }
        assert_eq!(download_for("Cascadia Code").map(|d| d.name.as_str()), Some("Cascadia Code"));
        assert_eq!(download_for("Cascadia Mono PL").map(|d| d.name.as_str()), Some("Cascadia Mono"));
        assert_eq!(download_for("Selawik Semibold").map(|d| (d.name.as_str(), d.restricted)), Some(("Selawik", false)));
        assert!(!aptos.restricted);
    }

    /// Fonts offered with limits on their use are listed, and say so. Plain Segoe UI isn't
    /// offered for download, and the Segoe UI Variable entry doesn't stand in for it.
    #[test]
    fn fonts_with_limited_licences_are_marked() {
        for family in ["Segoe UI Variable", "Segoe UI Variable Text", "segoe fluent icons", "Segoe MDL2 Assets"] {
            let d = download_for(family).unwrap_or_else(|| panic!("{family}"));
            assert!(d.restricted, "{family}");
            assert!(d.url.starts_with("https://learn.microsoft.com/"), "{}", d.url);
        }
        for family in ["Segoe UI", "Segoe UI Semibold", "Segoe UI Symbol", "Segoe Print"] {
            assert_eq!(download_for(family), None, "{family}");
        }
    }

    #[test]
    fn other_fonts_have_none() {
        // Fonts that come only with Windows or Office, look-alike names, and junk.
        for family in ["Calibri", "Cambria", "Consolas", "Arial", "Aptosa", "Apt", "", ";", "Aptosé", "é", "Cascadia"] {
            assert_eq!(download_for(family), None, "{family}");
        }
    }

    #[test]
    fn every_page_is_https_on_the_publishers_site() {
        // Every entry of the file is read: none is left out for a missing field.
        let entries = serde_json::from_str::<serde_json::Value>(LIST).unwrap()["downloads"].as_array().unwrap().len();
        assert_eq!(downloads().len(), entries);
        assert!(entries >= 3);
        for d in downloads() {
            let hosts = ["https://www.microsoft.com/", "https://learn.microsoft.com/", "https://github.com/microsoft/"];
            assert!(hosts.iter().any(|h| d.url.starts_with(h)), "{}", d.url);
        }
    }

    #[test]
    fn a_broken_list_has_no_downloads() {
        assert!(parse("not json").is_empty());
        assert!(parse("{}").is_empty());
        assert!(parse(r#"{"downloads": 7}"#).is_empty());
        // Entries without a family, a publisher or an https page are left out.
        let list = r#"{"downloads": [
            {"family": "A", "publisher": "P", "url": "http://plain.example"},
            {"family": "", "publisher": "P", "url": "https://x.example"},
            {"publisher": "P", "url": "https://x.example"},
            {"family": 3, "publisher": "P", "url": "https://x.example"},
            "junk",
            {"family": " Kept ", "publisher": "P", "url": "https://x.example"}
        ]}"#;
        let read = parse(list);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].name, "Kept");
        assert!(!read[0].restricted, "not restricted unless it says so");
        let marked = parse(r#"{"downloads": [{"family": "A", "publisher": "P", "url": "https://x.example", "restricted": true}]}"#);
        assert!(marked[0].restricted);
    }
}
