//! Build-time identity and update policy for internal distributions.
//!
//! Cargo tracks `option_env!`, so changing WORDCRAFT_BUILD_LABEL rebuilds this crate.
//! An unset or empty label keeps the official identity and startup update policy.
//! Changing the environment when launching a compiled application has no effect.

use std::sync::OnceLock;

/// The compiled distribution label; empty for ordinary builds.
pub const BUILD_LABEL: &str = match option_env!("WORDCRAFT_BUILD_LABEL") {
    Some(label) => label,
    None => "",
};

/// Application name used by native windows, including the empty window.
pub fn app_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| name_for_label(BUILD_LABEL)).as_str()
}

fn name_for_label(label: &str) -> String {
    if label.is_empty() { "WordCraft".to_string() } else { format!("WordCraft ({label})") }
}

/// Internal builds keep automatic startup checks off. Manual checks are separate.
pub const fn startup_update_check(requested: bool) -> bool {
    update_policy(requested, BUILD_LABEL)
}

const fn update_policy(requested: bool, label: &str) -> bool {
    requested && label.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_update_policy_matches_the_compiled_build() {
        assert!(update_policy(true, ""));
        assert!(!update_policy(false, ""));
        assert!(!update_policy(true, "Preview"));
        assert!(!update_policy(false, "Preview"));
        assert!(!startup_update_check(false));
        assert_eq!(startup_update_check(true), BUILD_LABEL.is_empty());
        if !BUILD_LABEL.is_empty() {
            assert!(!startup_update_check(true), "an internal build must never enable the startup check");
        }
    }

    #[test]
    fn application_name_identifies_internal_builds() {
        assert_eq!(name_for_label(""), "WordCraft");
        assert_eq!(name_for_label("Preview"), "WordCraft (Preview)");
        assert_eq!(app_name(), name_for_label(BUILD_LABEL));
    }
}
