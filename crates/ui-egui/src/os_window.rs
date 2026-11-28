//! The app id every WordCraft OS window carries and the channel its title shows: the root window
//! (`main.rs`), the runtime's windows (`InstallSpec::app_id`) and the document windows
//! (`craft::window`), so docks show the WordCraft icon on them and Wayland compositors' window
//! rules can match them. Without one the app id is empty.

/// Matches the `.desktop` file and hicolor icon name, so Wayland docks pick up the icon.
pub const APP_ID: &str = "ai.storyteller.wordcraft";

/// Immutable startup identity. The desktop selects it before any UI/runtime service is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchIdentity {
    Stable,
    NightlyV1,
}

static IDENTITY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn select_identity(identity: LaunchIdentity) -> Result<(), String> {
    let value = match identity {
        LaunchIdentity::Stable => 1,
        LaunchIdentity::NightlyV1 => 2,
    };
    IDENTITY
        .compare_exchange(0, value, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| "the window launch identity was already selected".into())
}

pub fn identity() -> LaunchIdentity {
    if IDENTITY.load(std::sync::atomic::Ordering::Acquire) == 2 { LaunchIdentity::NightlyV1 } else { LaunchIdentity::Stable }
}

pub const fn id_for(identity: LaunchIdentity) -> &'static str {
    match identity {
        LaunchIdentity::Stable => APP_ID,
        LaunchIdentity::NightlyV1 => "ai.storyteller.wordcraft.nightly-v1",
    }
}

pub fn app_id() -> &'static str {
    id_for(identity())
}

pub const fn name_for(identity: LaunchIdentity) -> &'static str {
    match identity {
        LaunchIdentity::Stable => "WordCraft",
        LaunchIdentity::NightlyV1 => "WordCraft Nightly",
    }
}

pub const fn stable_services_for(identity: LaunchIdentity) -> bool {
    matches!(identity, LaunchIdentity::Stable)
}

pub fn stable_services_allowed() -> bool {
    stable_services_for(identity())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nightly_identity_is_distinct_and_never_enables_stable_services() {
        assert_eq!(id_for(LaunchIdentity::Stable), APP_ID);
        assert_ne!(id_for(LaunchIdentity::NightlyV1), APP_ID);
        assert!(stable_services_for(LaunchIdentity::Stable));
        assert!(!stable_services_for(LaunchIdentity::NightlyV1));
    }

    #[test]
    fn the_channel_names_are_distinct_and_keep_the_stable_name() {
        assert_eq!(name_for(LaunchIdentity::Stable), "WordCraft");
        assert_eq!(name_for(LaunchIdentity::NightlyV1), "WordCraft Nightly");
    }
}
