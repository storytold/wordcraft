//! X11 scale factor for winit (`WINIT_X11_SCALE_FACTOR`).
//!
//! winit's X11 backend guesses the scale from each output's physical size
//! (EDID dimensions). An X server can deliberately run a dense panel at 96
//! DPI, in which case only winit applications appear zoomed. Prefer the
//! desktop's configured DPI instead, before the window is created:
//! `Xft.dpi` when set, otherwise the server screen's DPI; unknown values leave winit's own guess.
//! An explicit `WINIT_X11_SCALE_FACTOR` in the environment is always kept.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// Re-exec with `WINIT_X11_SCALE_FACTOR` set from the desktop when on X11 and unset.
pub fn apply() {
    #[cfg(all(unix, not(target_os = "macos")))]
    apply_impl();
}

#[cfg(all(unix, not(target_os = "macos")))]
fn apply_impl() {
    use std::os::unix::process::CommandExt as _;
    if std::env::var_os("DISPLAY").is_none_or(|v| v.is_empty()) {
        return;
    }
    // Native Wayland ignores `WINIT_X11_SCALE_FACTOR`; ask only when winit will use X11.
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) && !matches!(std::env::var("WINIT_UNIX_BACKEND").as_deref(), Ok("x11")) {
        return;
    }
    if let Ok(current) = std::env::var("WINIT_X11_SCALE_FACTOR")
        && !current.trim().is_empty()
    {
        return;
    }
    let xrdb = run_query("xrdb", &["-query"]);
    let xdpy = run_query("xdpyinfo", &[]);
    if parse_xft_dpi(&xrdb).is_none() && xsettings_manager_present() {
        return;
    }
    let Some(factor) = scale_factor_from_outputs_opt(&xrdb, &xdpy) else {
        return;
    };
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

#[cfg(all(unix, not(target_os = "macos")))]
fn run_query(program: &str, args: &[&str]) -> String {
    use std::io::Read as _;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let Ok(mut child) = Command::new(program).args(args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return String::new();
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return String::new();
    };
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0_u8; 4096];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) if out.len() < 64 * 1024 => out.extend_from_slice(&buf[..n.min(64 * 1024 - out.len())]),
                Ok(_) => {}
            }
        }
        out
    });
    let deadline = Instant::now() + Duration::from_millis(750);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader.join().unwrap_or_default();
                return if status.success() { String::from_utf8_lossy(&bytes).into_owned() } else { String::new() };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return String::new();
            }
        }
    }
}

fn xsettings_manager_present() -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("x11-settings".into()).spawn(move || {
        let _ = tx.send(xsettings_owner_present());
    });
    if spawned.is_err() {
        return false;
    }
    rx.recv_timeout(std::time::Duration::from_millis(750)).unwrap_or(false)
}

fn xsettings_owner_present() -> bool {
    use x11rb::protocol::xproto::ConnectionExt as _;

    let Ok((connection, screen)) = x11rb::connect(None) else { return false };
    let name = format!("_XSETTINGS_S{screen}");
    let Ok(cookie) = connection.intern_atom(false, name.as_bytes()) else { return false };
    let Ok(atom) = cookie.reply() else { return false };
    let Ok(cookie) = connection.get_selection_owner(atom.atom) else { return false };
    let Ok(owner) = cookie.reply() else { return false };
    selection_owner_present(owner.owner)
}

fn selection_owner_present(owner: u32) -> bool {
    owner != 0
}

fn valid_scale_factor(factor: f64) -> bool {
    factor.is_finite() && (0.5..=4.0).contains(&factor)
}

fn format_factor(factor: f64) -> String {
    let mut s = std::format!("{factor:.4}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s.is_empty() { "1".to_string() } else { s }
}

/// Combine `xrdb -query` and `xdpyinfo` outputs into a scale factor.
#[allow(dead_code)]
pub fn scale_factor_from_outputs(xrdb: &str, xdpyinfo: &str) -> f64 {
    scale_factor_from_outputs_opt(xrdb, xdpyinfo).unwrap_or(1.0)
}

fn scale_factor_from_outputs_opt(xrdb: &str, xdpyinfo: &str) -> Option<f64> {
    if let Some(dpi) = parse_xft_dpi(xrdb) {
        let factor = dpi / 96.0;
        if valid_scale_factor(factor) {
            return Some(factor);
        }
    }
    if let Some(dpi) = parse_xdpyinfo_resolution(xdpyinfo) {
        let factor = dpi / 96.0;
        if valid_scale_factor(factor) {
            return Some(factor);
        }
    }
    if let Some((pixels, mm)) = parse_xdpyinfo_dimensions(xdpyinfo) {
        return Some(calc_dpi_factor(pixels, mm));
    }
    None
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
        assert_eq!(scale_factor_from_outputs_opt("", ""), None);
        assert_eq!(scale_factor_from_outputs("", "screen #0:\n"), 1.0);
    }

    #[test]
    fn xsettings_owner_zero_means_no_manager() {
        assert!(!selection_owner_present(0));
        assert!(selection_owner_present(0x200001));
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
