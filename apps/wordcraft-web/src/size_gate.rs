//! The size check applied to a file the browser hands over, before any of it is read (#1333).

/// Whether a browser `File` of `size` bytes (`File.size`, a JS number) may be read under a budget
/// of `max` bytes. The error names the file and the limit.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn check_size(name: &str, size: f64, max: u64) -> Result<(), String> {
    if !size.is_finite() || size < 0.0 {
        return Err(format!("{name}: the browser reported no usable size"));
    }
    if size > max as f64 {
        return Err(format!("{name}: larger than the limit of {}", wordcraft_engine::io::limit_text(max)));
    }
    Ok(())
}

/// The budget for a file named `name`: pictures (by extension, one of `image_exts`) get the
/// picture limit, fonts the font limit, anything else the document limit.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn budget_for(name: &str, image_exts: &[&str]) -> u64 {
    // The same rule the opener uses to tell a format: what follows the last dot of the file
    // name, so a name that is only ".png" counts as a picture there and here.
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if image_exts.contains(&ext.as_str()) {
        wordcraft_ui_egui::paste_picture::MAX_FILE_BYTES
    } else if wordcraft_fonts::FONT_FILE_EXTS.contains(&ext.as_str()) {
        wordcraft_fonts::MAX_FONT_FILE_BYTES as u64
    } else {
        wordcraft_engine::io::MAX_OPEN_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_over_budget_is_refused_by_name() {
        let err = check_size("big.docx", 101.0, 100).unwrap_err();
        assert!(err.contains("big.docx") && err.contains("100"), "{err}");
    }

    #[test]
    fn a_file_within_budget_is_allowed() {
        assert!(check_size("a.docx", 100.0, 100).is_ok());
        assert!(check_size("empty.txt", 0.0, 100).is_ok());
    }

    #[test]
    fn dropped_pictures_and_fonts_get_their_own_budgets() {
        let images = ["png", "jpg"];
        assert_eq!(budget_for("Photo.PNG", &images), wordcraft_ui_egui::paste_picture::MAX_FILE_BYTES);
        assert_eq!(budget_for("face.ttf", &images), wordcraft_fonts::MAX_FONT_FILE_BYTES as u64);
        assert_eq!(budget_for("letter.docx", &images), wordcraft_engine::io::MAX_OPEN_BYTES);
        assert_eq!(budget_for("noextension", &images), wordcraft_engine::io::MAX_OPEN_BYTES);
        // Only the last extension counts; a name that is just ".png" is a picture, as the opener sees it.
        assert_eq!(budget_for("scan.png.docx", &images), wordcraft_engine::io::MAX_OPEN_BYTES);
        assert_eq!(budget_for(".png", &images), wordcraft_ui_egui::paste_picture::MAX_FILE_BYTES);
    }

    #[test]
    fn the_refusal_says_2_gb_for_the_document_limit() {
        let err = check_size("big.docx", 3.0e9, wordcraft_engine::io::MAX_OPEN_BYTES).unwrap_err();
        assert!(err.contains("2 GB"), "{err}");
    }

    #[test]
    fn a_nonsense_size_is_refused() {
        assert!(check_size("x", f64::NAN, 100).is_err());
        assert!(check_size("x", -1.0, 100).is_err());
        assert!(check_size("x", f64::INFINITY, 100).is_err());
    }
}
