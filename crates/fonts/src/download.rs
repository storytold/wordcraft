//! Fonts a document may ask for that their publisher gives away, and where: when one isn't
//! installed, the app can say where to get it instead of only naming the substitute.
//!
//! Only a publisher's own page is listed, and only for fonts it offers to anyone free of
//! charge. The app links to the page; it never downloads, bundles or redistributes the fonts.
//! Most Office fonts (Calibri, Cambria, Consolas, Segoe UI…) come only with Windows or Office,
//! so they are not here.

/// Where a font family can be downloaded free of charge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontDownload {
    /// The name of the download: the family or collection the page offers.
    pub name: &'static str,
    /// Who publishes the font and hosts the page.
    pub publisher: &'static str,
    /// The publisher's download page.
    pub url: &'static str,
}

/// Each download and the family it covers: the family itself and every name that continues it
/// after a space ("Aptos" covers "Aptos Display", "Aptos Narrow", "Aptos Mono"…).
const DOWNLOADS: &[FontDownload] = &[
    // Microsoft Download Center, "Microsoft Aptos Fonts": the whole Aptos family.
    FontDownload { name: "Aptos", publisher: "Microsoft", url: "https://www.microsoft.com/download/details.aspx?id=106087" },
    // Microsoft's own repository; SIL Open Font License.
    FontDownload { name: "Cascadia Code", publisher: "Microsoft", url: "https://github.com/microsoft/cascadia-code/releases" },
    FontDownload { name: "Cascadia Mono", publisher: "Microsoft", url: "https://github.com/microsoft/cascadia-code/releases" },
];

/// Where `family` can be downloaded free of charge from its publisher, if it can. The name is
/// matched whatever its case, with a charset suffix ("Aptos CE") or alternates ("Aptos;Arial")
/// as a document may write it.
pub fn download_for(family: &str) -> Option<&'static FontDownload> {
    family.split(';').map(str::trim).filter(|f| !f.is_empty()).find_map(|f| {
        let f = crate::word::charset_base_family(f).trim();
        DOWNLOADS.iter().find(|d| {
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
        assert_eq!(download_for("Cascadia Code").map(|d| d.name), Some("Cascadia Code"));
        assert_eq!(download_for("Cascadia Mono PL").map(|d| d.name), Some("Cascadia Mono"));
    }

    #[test]
    fn other_fonts_have_none() {
        // Fonts that come only with Windows or Office, look-alike names, and junk.
        for family in ["Calibri", "Cambria", "Consolas", "Segoe UI", "Arial", "Aptosa", "Apt", "", ";", "Aptosé", "é", "Cascadia"] {
            assert_eq!(download_for(family), None, "{family}");
        }
    }

    #[test]
    fn every_page_is_https_on_the_publishers_site() {
        for d in DOWNLOADS {
            assert!(d.url.starts_with("https://www.microsoft.com/") || d.url.starts_with("https://github.com/microsoft/"), "{}", d.url);
        }
    }
}
