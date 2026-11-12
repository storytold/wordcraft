//! Save-picker rejection policy, shared with native tests of the browser fallback.

use wordcraft_ui_egui::async_saves::SaveOutcome;

/// A cancelled picker ends the save. Other picker failures try the browser download path.
/// Write/close errors do not pass through this function: they retain their own failure.
pub(crate) fn picker_rejected(name: &str, download: impl FnOnce() -> Result<(), String>) -> SaveOutcome {
    if name == "AbortError" {
        SaveOutcome::Cancelled
    } else {
        match download() {
            Ok(()) => SaveOutcome::Saved,
            Err(error) => SaveOutcome::Failed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_picker_downloads_once() {
        for name in ["SecurityError", "NotSupportedError", "TypeError", ""] {
            let mut calls = 0;
            let outcome = picker_rejected(name, || {
                calls += 1;
                Ok(())
            });
            assert_eq!(outcome, SaveOutcome::Saved);
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn cancelled_picker_never_downloads() {
        let outcome = picker_rejected("AbortError", || panic!("cancel must not download"));
        assert_eq!(outcome, SaveOutcome::Cancelled);
    }

    #[test]
    fn failed_fallback_remains_a_failed_save() {
        let outcome = picker_rejected("SecurityError", || Err("download unavailable".into()));
        assert_eq!(outcome, SaveOutcome::Failed("download unavailable".into()));
    }
}
