//! X11 scale factor for winit (`WINIT_X11_SCALE_FACTOR`).
//!
//! winit's X11 backend guesses the scale from each output's physical size
//! (EDID dimensions). An X server can deliberately run a dense panel at 96
//! DPI, in which case only winit applications appear zoomed. Prefer the
//! desktop's configured DPI instead, before the window is created:
//! `Xft.dpi` when set, otherwise the server screen's DPI, otherwise 1.0.
//! An explicit `WINIT_X11_SCALE_FACTOR` in the environment is always kept.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// Re-exec with `WINIT_X11_SCALE_FACTOR` set from the desktop when on X11 and unset.
pub fn apply() {
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    return;
    #[cfg(all(unix, not(target_os = "macos")))]
    apply_impl();
}

#[cfg(all(unix, not(target_os = "macos")))]
fn apply_impl() {
    use std::os::unix::process::CommandExt as _;
    if std::env::var_os("DISPLAY").is_none_or(|v| v.is_empty()) {
        return;
    }
    if let Ok(current) = std::env::var("WINIT_X11_SCALE_FACTOR")
        && !current.trim().is_empty()
    {
        return;
    }
    let xrdb = std::process::Command::new("xrdb")
        .arg("-query")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let xdpy = std::process::Command::new("xdpyinfo")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let factor = scale_factor_from_outputs(&xrdb, &xdpy);
    if !valid_scale_factor(factor) {
        return;
    }
    // `std::env::set_var` is `unsafe` in edition 2024 and `unsafe` is
    // forbidden here, so re-exec with the variable set instead. winit reads
    // it when the window is created in the child; the variable itself guards
    // against looping.
    let Some(exe) = std::env::current_exe().ok() else {
        return;
    };
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let err = std::process::Command::new(exe).args(args).env("WINIT_X11_SCALE_FACTOR", format_factor(factor)).exec();
    log::warn!("x11 scale: re-exec failed ({err}); continuing with winit defaults");
}

fn valid_scale_factor(factor: f64) -> bool {
    factor.is_finite() && (0.5..=4.0).contains(&factor)
}

fn format_factor(factor: f64) -> String {
    let rounded = (factor * 12.0).round() / 12.0;
    let mut s = std::format!("{rounded:.4}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s.is_empty() { "1".to_string() } else { s }
}

/// Combine `xrdb -query` and `xdpyinfo` outputs into a scale factor.
pub fn scale_factor_from_outputs(xrdb: &str, xdpyinfo: &str) -> f64 {
    if let Some(dpi) = parse_xft_dpi(xrdb) {
        return dpi / 96.0;
    }
    if let Some(dpi) = parse_xdpyinfo_resolution(xdpyinfo) {
        return dpi / 96.0;
    }
    if let Some((pixels, mm)) = parse_xdpyinfo_dimensions(xdpyinfo) {
        return calc_dpi_factor(pixels, mm);
    }
    1.0
}

/// Same quantization as winit's `calc_dpi_factor`: 1/12 steps, at least 1.0.
pub fn calc_dpi_factor((width_px, height_px): (u32, u32), (width_mm, height_mm): (u64, u64)) -> f64 {
    if width_mm == 0 || height_mm == 0 {
        return 1.0;
    }
    let ppmm = ((width_px as f64 * height_px as f64) / (width_mm as f64 * height_mm as f64)).sqrt();
    let dpi_factor = ((ppmm * (12.0 * 25.4 / 96.0)).round() / 12.0).max(1.0);
    if dpi_factor.is_finite() && dpi_factor <= 20.0 { dpi_factor } else { 1.0 }
}

pub fn parse_xft_dpi(xrdb: &str) -> Option<f64> {
    for line in xrdb.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("Xft.dpi:").or_else(|| line.strip_prefix("Xft/dpi:")) else {
            continue;
        };
        if let Ok(dpi) = rest.split_whitespace().next().unwrap_or("").parse::<f64>()
            && dpi.is_finite()
            && dpi > 0.0
            && dpi <= 768.0
        {
            return Some(dpi);
        }
    }
    None
}

pub fn parse_xdpyinfo_resolution(xdpyinfo: &str) -> Option<f64> {
    for line in xdpyinfo.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("resolution:") else {
            continue;
        };
        let first = rest.trim().split(['x', ' ', '\t']).next().unwrap_or("");
        if let Ok(dpi) = first.parse::<f64>()
            && dpi.is_finite()
            && dpi > 0.0
            && dpi <= 768.0
        {
            return Some(dpi);
        }
    }
    None
}

pub fn parse_xdpyinfo_dimensions(xdpyinfo: &str) -> Option<((u32, u32), (u64, u64))> {
    for line in xdpyinfo.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("dimensions:") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let pixels = parts.next().unwrap_or("");
        let mut px = pixels.split('x');
        let (Ok(w), Ok(h)): (Result<u32, _>, Result<u32, _>) = (px.next().unwrap_or("").parse(), px.next().unwrap_or("").parse()) else {
            continue;
        };
        let Some(mm_part) = parts.find(|p| p.contains('x') && p.chars().any(|c| c.is_ascii_digit())) else {
            continue;
        };
        let mm_clean: String = mm_part.chars().filter(|c| c.is_ascii_digit() || *c == 'x').collect();
        let mut mm = mm_clean.split('x');
        let (Ok(mw), Ok(mh)): (Result<u64, _>, Result<u64, _>) = (mm.next().unwrap_or("").parse(), mm.next().unwrap_or("").parse()) else {
            continue;
        };
        if w > 0 && h > 0 {
            return Some(((w, h), (mw, mh)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIXELS: (u32, u32) = (1920, 1080);
    const SCREEN_96: (u64, u64) = (508, 285);
    const SCREEN_144: (u64, u64) = (339, 190);
    const SCREEN_192: (u64, u64) = (254, 143);

    #[test]
    fn server_dpi_wins_over_the_panels_physical_density() {
        assert_eq!(calc_dpi_factor(PIXELS, SCREEN_96), 1.0);
        assert_eq!(calc_dpi_factor(PIXELS, SCREEN_144), 1.5);
        assert_eq!(calc_dpi_factor(PIXELS, SCREEN_192), 2.0);
    }

    #[test]
    fn xft_dpi_still_has_priority() {
        for dpi in [96.0, 120.0, 144.0, 192.0] {
            let xrdb = std::format!("Xft.dpi: {dpi}\n");
            let hidpi = "  dimensions:    1920x1080 pixels (254x143 millimeters)\n  resolution:    192x192 dots per inch\n";
            assert_eq!(scale_factor_from_outputs(&xrdb, hidpi), dpi / 96.0);
        }
    }

    #[test]
    fn missing_screen_dimensions_fall_back_to_one() {
        assert_eq!(calc_dpi_factor(PIXELS, (0, 285)), 1.0);
        assert_eq!(scale_factor_from_outputs("", ""), 1.0);
        assert_eq!(scale_factor_from_outputs("", "screen #0:\n"), 1.0);
    }

    #[test]
    fn xrdb_parsing_finds_xft_dpi() {
        assert_eq!(parse_xft_dpi("Xft.dpi:\t96\n"), Some(96.0));
        assert_eq!(parse_xft_dpi("Xft.dpi: 144\n"), Some(144.0));
        assert_eq!(parse_xft_dpi("*customization: -color\nXft.hintstyle: hintfull\n"), None);
        assert_eq!(parse_xft_dpi(""), None);
    }

    #[test]
    fn xdpyinfo_parsing_finds_resolution_and_dimensions() {
        let info = "screen #0:\n  dimensions:    1920x1080 pixels (508x285 millimeters)\n  resolution:    96x96 dots per inch\n";
        assert_eq!(parse_xdpyinfo_resolution(info), Some(96.0));
        assert_eq!(parse_xdpyinfo_dimensions(info), Some((PIXELS, SCREEN_96)));
        assert_eq!(scale_factor_from_outputs("", info), 1.0);
        let hidpi = "  dimensions:    1920x1080 pixels (254x143 millimeters)\n  resolution:    192x192 dots per inch\n";
        assert_eq!(scale_factor_from_outputs("", hidpi), 2.0);
        let xft = "Xft.dpi: 144\n";
        assert_eq!(scale_factor_from_outputs(xft, hidpi), 1.5);
    }
}
