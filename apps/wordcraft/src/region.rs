//! The system region new documents follow (#838): the paper File › New starts on, the default
//! measurement unit and the decimal separator.
//!
//! - Windows: the preferred language with the home location from Settings › Time & language ›
//!   Region (`GlobalizationPreferences.HomeGeographicRegion`, a safe Windows Runtime API), since
//!   the language list alone says `en-US` for an English interface in Poland.
//! - Elsewhere: the locale the UI crate reads ([`wordcraft_ui_egui::i18n::system_region`]).

/// The system region.
pub fn system() -> wordcraft_engine::Region {
    // Runs without preferences (agents' and screenshot runs) behave the same on every machine:
    // en-US unless `WORDCRAFT_REGION` says otherwise.
    if std::env::var_os("WORDCRAFT_NO_PREFS").is_some() && std::env::var_os("WORDCRAFT_REGION").is_none() {
        return wordcraft_engine::Region::default();
    }
    let region = wordcraft_ui_egui::i18n::system_region();
    // Not on the Windows 7 build (docs/windows7.md): the API is part of the Windows Runtime.
    #[cfg(all(windows, not(target_vendor = "win7")))]
    {
        if std::env::var_os("WORDCRAFT_REGION").is_none() {
            return match windows::System::UserProfile::GlobalizationPreferences::HomeGeographicRegion() {
                Ok(home) => region.with_territory(&home.to_string()),
                Err(e) => {
                    log::debug!("system home region: {e}");
                    region
                }
            };
        }
    }
    region
}
