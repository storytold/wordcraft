//! Pure host-mode rules, shared by the bridge ([`crate::host`]) and testable without a browser
//! (like [`crate::prefs`]): what opts a page into hosting WordCraft in its own document.

/// Does this `data-host` value ask for page host mode? Only the exact value `page` counts;
/// anything else is ignored, so the page host mode stays opt-in. `?host=parent`, the parent
/// bridge's URL opt-in, is answered before this is asked.
pub fn page_host_requested(data_host: Option<&str>) -> bool {
    data_host == Some("page")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_page_value_opts_in() {
        assert!(page_host_requested(Some("page")));
        for off in [None, Some(""), Some("Page"), Some("PAGE"), Some(" page"), Some("page "), Some("parent")] {
            assert!(!page_host_requested(off), "{off:?} must keep the page host mode off");
        }
    }
}
