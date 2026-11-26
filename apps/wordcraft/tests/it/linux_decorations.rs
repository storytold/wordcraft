//! Integration test for Linux Wayland client-side window decoration controls (#5814, #5375).
//!
//! Ensures that on Linux, client-side window controls use font-free Adwaita vector paths
//! so the close button is rendered as a clean vector cross rather than a square fallback,
//! and that sctk-adwaita has proper license attribution in NOTICE without bundling font files.

use std::path::Path;

/// Strips an unquoted TOML `#` comment from a single line.
fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    for (idx, ch) in line.char_indices() {
        match ch {
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            '#' if !in_double && !in_single => return &line[..idx],
            _ => {}
        }
    }
    line
}

/// Extracts the comment-stripped body of a TOML table section by exact `[header]`.
fn toml_section(content: &str, header: &str) -> String {
    let mut in_section = false;
    let mut out = String::new();
    for raw_line in content.lines() {
        let line = strip_toml_comment(raw_line).trim();
        if line.starts_with('[') && line.ends_with(']') {
            if in_section {
                break;
            }
            in_section = line == header;
            continue;
        }
        if in_section && !line.is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    assert!(in_section, "expected target section {header} in apps/wordcraft/Cargo.toml");
    out
}

/// Parses the `features = [...]` string array from the `winit = { ... }` dependency entry in a TOML section.
fn winit_features(section: &str) -> Vec<&str> {
    let winit_start = section.find("winit").expect("target section must declare a winit dependency");
    let after_winit = &section[winit_start..];
    let table_end = after_winit.find('}').expect("winit inline table must close with '}'");
    let winit_table = &after_winit[..=table_end];
    let features_start = winit_table.find("features").expect("winit dependency must declare features");
    let after_features = &winit_table[features_start..];
    let open_bracket = after_features.find('[').expect("winit features must open with '['");
    let after_open = &after_features[open_bracket + 1..];
    let close_bracket = after_open.find(']').expect("winit features must close with ']'");
    let array_body = &after_open[..close_bracket];
    array_body.split(',').map(|item| item.trim().trim_matches('"')).filter(|item| !item.is_empty()).collect()
}

/// Verifies that `apps/wordcraft/Cargo.toml` enables `wayland-csd-adwaita-notitle` for Linux.
#[test]
fn linux_manifest_enables_wayland_csd_adwaita_notitle() {
    let cargo_toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let content = std::fs::read_to_string(&cargo_toml).expect("read apps/wordcraft/Cargo.toml");

    let linux_section = toml_section(&content, "[target.'cfg(target_os = \"linux\")'.dependencies]");
    let linux_features = winit_features(&linux_section);
    assert!(
        linux_features.contains(&"x11") && linux_features.contains(&"wayland-csd-adwaita-notitle"),
        "Linux winit dependency must enable x11 and wayland-csd-adwaita-notitle, got: {linux_features:?}"
    );

    let other_unix_section = toml_section(&content, "[target.'cfg(all(unix, not(any(target_os = \"macos\", target_os = \"linux\"))))'.dependencies]");
    let other_unix_features = winit_features(&other_unix_section);
    assert!(
        other_unix_features.contains(&"x11") && !other_unix_features.contains(&"wayland-csd-adwaita-notitle"),
        "Non-Linux Unix winit dependency must enable x11 without wayland-csd-adwaita-notitle, got: {other_unix_features:?}"
    );
}

/// Verifies that `NOTICE` documents sctk-adwaita and its MIT licence.
#[test]
fn notice_documents_sctk_adwaita_attribution() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspace root");
    let notice = std::fs::read_to_string(workspace_root.join("NOTICE")).expect("read NOTICE");
    assert!(notice.contains("Linux Wayland window decorations"), "NOTICE must include 'Linux Wayland window decorations'");
    assert!(notice.contains("sctk-adwaita"), "NOTICE must document sctk-adwaita");
    assert!(notice.contains("Bartłomiej Maryńczak"), "NOTICE must retain sctk-adwaita's author copyright");
}

/// Parses `[[package]]` entries from `Cargo.lock` as `(name, version, dependencies)`.
fn parse_lockfile_packages(lockfile: &str) -> Vec<(&str, &str, Vec<&str>)> {
    lockfile
        .split("[[package]]")
        .skip(1)
        .filter_map(|chunk| {
            let mut name = None;
            let mut version = None;
            let mut deps = Vec::new();
            let mut in_deps = false;
            for raw_line in chunk.lines() {
                let line = raw_line.trim();
                if in_deps {
                    if line == "]" {
                        in_deps = false;
                        continue;
                    }
                    let dep = line.trim_end_matches(',').trim().trim_matches('"');
                    if !dep.is_empty() {
                        deps.push(dep);
                    }
                    continue;
                }
                if let Some(rest) = line.strip_prefix("name = ") {
                    name = Some(rest.trim_matches('"'));
                } else if let Some(rest) = line.strip_prefix("version = ") {
                    version = Some(rest.trim_matches('"'));
                } else if line == "dependencies = [" {
                    in_deps = true;
                }
            }
            Some((name?, version?, deps))
        })
        .collect()
}

/// Verifies that WordCraft's resolved Linux `winit` -> `sctk-adwaita` dependency path in
/// `Cargo.lock` does not pull in any title-font renderer crates.
#[test]
fn cargo_lock_contains_font_free_sctk_adwaita() {
    use std::collections::{HashSet, VecDeque};

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspace root");
    let lockfile = std::fs::read_to_string(workspace_root.join("Cargo.lock")).expect("read Cargo.lock");
    let packages = parse_lockfile_packages(&lockfile);

    let has_dep = |pkg_name: &str, target: &str| {
        packages.iter().any(|(name, _, deps)| *name == pkg_name && deps.iter().any(|spec| spec.split_whitespace().next() == Some(target)))
    };
    assert!(has_dep("winit", "sctk-adwaita"), "winit in Cargo.lock must depend on sctk-adwaita");

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    for (idx, (name, _, _)) in packages.iter().enumerate() {
        if *name == "sctk-adwaita" && visited.insert(idx) {
            queue.push_back(idx);
        }
    }
    assert!(!queue.is_empty(), "Cargo.lock must include sctk-adwaita");

    let forbidden = ["fontdue", "ab_glyph", "cosmic-text", "crossfont"];
    while let Some(idx) = queue.pop_front() {
        let (name, _, deps) = &packages[idx];
        assert!(!forbidden.contains(name), "sctk-adwaita's resolved dependency path must not include title-font renderer {name}");
        for spec in deps {
            let mut parts = spec.split_whitespace();
            let dep_name = parts.next().expect("dependency name");
            let dep_ver = parts.next();
            for (next_idx, (cand_name, cand_ver, _)) in packages.iter().enumerate() {
                if *cand_name == dep_name && dep_ver.is_none_or(|v| v == *cand_ver) && visited.insert(next_idx) {
                    queue.push_back(next_idx);
                }
            }
        }
    }
}
