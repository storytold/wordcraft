//! Custom theme colours, fonts and themes; Save Current Theme and Browse for Themes (#1869).

use std::path::PathBuf;

use serde_json::{Value, json};
use wordcraft_doc::props::{Rgb, TextColor};

use crate::Session;

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Twelve distinct colours: dk1, lt1, dk2, lt2, accent1–6, hlink, folHlink.
fn twelve(seed: u8) -> Vec<String> {
    (0..12u8).map(|i| format!("#{:02X}{:02X}{:02X}", seed, i * 20, 0x80 + i)).collect()
}

fn rgb(h: &str) -> Rgb {
    Rgb::parse(h).unwrap()
}

/// A folder of its own for one test.
fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wordcraft-themes-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn heading1_color(s: &Session) -> Option<TextColor> {
    s.doc.styles.styles.iter().find(|st| st.id == "Heading1").and_then(|st| st.chr.color)
}

#[test]
fn custom_colors_create_and_apply_all_twelve() {
    let mut s = s();
    let c = twelve(0x10);
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": c}));
    let want: Vec<Rgb> = c.iter().map(|h| rgb(h)).collect();
    assert_eq!(s.doc.settings.theme_colors, want);
    assert_eq!(heading1_color(&s), Some(TextColor::Rgb(want[4])), "headings take accent 1");
    let list = run(&mut s, "design.customThemes", json!({}));
    assert_eq!(list["colors"][0]["name"], "Harbour");
    assert_eq!(list["colors"][0]["colors"].as_array().unwrap().len(), 12);
}

#[test]
fn custom_colors_name_taken_needs_replace() {
    let mut s = s();
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": twelve(1)}));
    assert!(s.run("design.customColors", &json!({"name": "harbour", "colors": twelve(2)})).is_err(), "same name (any case) without replace");
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": twelve(3), "replace": true}));
    let list = run(&mut s, "design.customThemes", json!({}));
    assert_eq!(list["colors"].as_array().unwrap().len(), 1);
    assert_eq!(list["colors"][0]["colors"][0], twelve(3)[0].trim_start_matches('#'));
}

#[test]
fn custom_fonts_create_and_apply() {
    let mut s = s();
    run(&mut s, "design.customFonts", json!({"name": "Bookish", "heading": "Palatino", "body": "Optima"}));
    assert_eq!(s.doc.settings.major_font, "Palatino");
    assert_eq!(s.doc.settings.minor_font, "Optima");
    assert_eq!(s.doc.styles.default_chr.font.as_deref(), Some("Optima"));
    let list = run(&mut s, "design.customThemes", json!({}));
    assert_eq!(list["fonts"][0], json!({"name": "Bookish", "heading": "Palatino", "body": "Optima"}));
}

#[test]
fn theme_colors_and_fonts_apply_custom_sets_by_name() {
    let mut s = s();
    let c = twelve(0x22);
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": c}));
    run(&mut s, "design.customFonts", json!({"name": "Bookish", "heading": "Palatino", "body": "Optima"}));
    run(&mut s, "design.theme", json!({"name": "Studio"}));
    assert_ne!(s.doc.settings.theme_colors[4], rgb(&c[4]));
    run(&mut s, "design.themeColors", json!({"name": "Harbour"}));
    assert_eq!(s.doc.settings.theme_colors[0], rgb(&c[0]));
    assert_eq!(s.doc.settings.theme_colors[11], rgb(&c[11]));
    run(&mut s, "design.themeFonts", json!({"name": "Bookish"}));
    assert_eq!((s.doc.settings.major_font.as_str(), s.doc.settings.minor_font.as_str()), ("Palatino", "Optima"));
    // The old form still works.
    run(&mut s, "design.themeFonts", json!({"heading": "Georgia"}));
    assert_eq!(s.doc.settings.minor_font, "Georgia");
    assert!(s.run("design.themeFonts", &json!({"name": "Nope"})).is_err());
}

/// A built-in theme or colour set after a custom one replaces all 12 colours, not only accent 1:
/// otherwise the custom set's other 11 colours would linger in the saved theme part.
#[test]
fn built_in_colors_after_custom_ones_replace_all_twelve() {
    let studio = crate::cmd::design::THEMES.iter().find(|t| t.0 == "Studio").unwrap().3;
    let gallery = crate::cmd::design::THEMES.iter().find(|t| t.0 == "Gallery").unwrap().3;
    let mut s = s();
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": twelve(0x66)}));
    run(&mut s, "design.theme", json!({"name": "Studio"}));
    let mut want = wordcraft_doc::THEME_COLORS.to_vec();
    want.splice(4..10, studio);
    assert_eq!(s.doc.settings.theme_colors, want);
    run(&mut s, "design.themeColors", json!({"name": "Harbour"}));
    run(&mut s, "design.themeColors", json!({"name": "Gallery"}));
    want.splice(4..10, gallery);
    assert_eq!(s.doc.settings.theme_colors, want);
    assert_eq!(heading1_color(&s), Some(TextColor::Rgb(gallery[0])));
}

#[test]
fn delete_custom_removes_and_unknown_is_an_error() {
    let mut s = s();
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": twelve(1)}));
    run(&mut s, "design.customFonts", json!({"name": "Bookish", "heading": "A", "body": "B"}));
    run(&mut s, "design.deleteCustom", json!({"kind": "colors", "name": "Harbour"}));
    run(&mut s, "design.deleteCustom", json!({"kind": "fonts", "name": "bookish"}));
    let list = run(&mut s, "design.customThemes", json!({}));
    assert!(list["colors"].as_array().unwrap().is_empty());
    assert!(list["fonts"].as_array().unwrap().is_empty());
    assert!(s.run("design.themeColors", &json!({"name": "Harbour"})).is_err());
    assert!(s.run("design.deleteCustom", &json!({"kind": "colors", "name": "Harbour"})).is_err());
    assert!(s.run("design.deleteCustom", &json!({"kind": "effects", "name": "x"})).is_err());
}

#[test]
fn custom_sets_persist_through_the_themes_folder() {
    let dir = temp_dir("persist");
    let mut a = s();
    a.load_custom_themes(&dir);
    run(&mut a, "design.customColors", json!({"name": "Harbour", "colors": twelve(5)}));
    run(&mut a, "design.customFonts", json!({"name": "Bookish", "heading": "Palatino", "body": "Optima"}));
    run(&mut a, "design.customFonts", json!({"name": "Gone", "heading": "X", "body": "Y"}));
    run(&mut a, "design.deleteCustom", json!({"kind": "fonts", "name": "Gone"}));
    run(&mut a, "design.themeFonts", json!({"name": "Bookish"}));
    run(&mut a, "design.saveTheme", json!({"name": "Seaside"}));
    assert!(dir.join("Seaside.thmx").is_file(), "Save Current Theme without a path goes to the themes folder");

    let mut b = s();
    b.load_custom_themes(&dir);
    let list = run(&mut b, "design.customThemes", json!({}));
    assert_eq!(list["colors"][0]["name"], "Harbour");
    assert_eq!(list["fonts"].as_array().unwrap().len(), 1);
    assert_eq!(list["fonts"][0]["name"], "Bookish");
    assert_eq!(list["themes"], json!(["Seaside"]));
    run(&mut b, "design.theme", json!({"name": "Seaside"}));
    assert_eq!(b.doc.settings.theme_colors[4], rgb(&twelve(5)[4]));
    assert_eq!(b.doc.settings.major_font, "Palatino");
    assert_eq!(b.doc.settings.theme_name, "Seaside");

    run(&mut b, "design.deleteCustom", json!({"kind": "theme", "name": "Seaside"}));
    assert!(!dir.join("Seaside.thmx").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_then_open_theme_restores_colours_fonts_and_name() {
    let dir = temp_dir("saveopen");
    let path = dir.join("Mine.thmx");
    let mut s = s();
    let c = twelve(0x33);
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": c}));
    run(&mut s, "design.customFonts", json!({"name": "Bookish", "heading": "Palatino", "body": "Optima"}));
    s.doc.settings.theme_name = "Mine".into();
    run(&mut s, "design.saveTheme", json!({"path": path.to_string_lossy()}));
    assert!(path.is_file());

    let mut t = Session::new(wordcraft_doc::Document::new());
    run(&mut t, "design.openTheme", json!({"path": path.to_string_lossy()}));
    let want: Vec<Rgb> = c.iter().map(|h| rgb(h)).collect();
    assert_eq!(t.doc.settings.theme_colors, want);
    assert_eq!((t.doc.settings.major_font.as_str(), t.doc.settings.minor_font.as_str()), ("Palatino", "Optima"));
    assert_eq!(t.doc.settings.theme_name, "Mine");
    assert_eq!(heading1_color(&t), Some(TextColor::Rgb(want[4])));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_theme_reads_a_docx_and_base64_data() {
    let mut src = s();
    run(&mut src, "design.customColors", json!({"name": "Harbour", "colors": twelve(0x44)}));
    let bytes = wordcraft_docx::write(&src.doc).unwrap();
    let data = crate::cmd::insert::base64_encode(&bytes);
    let mut t = s();
    run(&mut t, "design.openTheme", json!({"path": "Other.docx", "data": data}));
    assert_eq!(t.doc.settings.theme_colors[7], rgb(&twelve(0x44)[7]));
    assert!(t.run("design.openTheme", &json!({"path": "x.thmx", "data": "bm90IGEgemlw"})).is_err(), "not a package");
}

#[test]
fn docx_saved_after_a_custom_theme_carries_it() {
    let mut s = s();
    let c = twelve(0x55);
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": c}));
    run(&mut s, "design.customFonts", json!({"name": "Bookish", "heading": "Palatino", "body": "Optima"}));
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let t = wordcraft_docx::read_theme(&bytes).unwrap();
    let want: Vec<Rgb> = c.iter().map(|h| rgb(h)).collect();
    assert_eq!(t.colors.to_vec(), want);
    assert_eq!((t.major_font.as_str(), t.minor_font.as_str()), ("Palatino", "Optima"));
}

#[test]
fn hostile_custom_theme_params_are_errors() {
    let mut s = s();
    let mut eleven = twelve(1);
    eleven.pop();
    let mut bad = twelve(1);
    bad[3] = "#GG0000".into();
    let cases = [
        ("design.customColors", json!({"name": "A", "colors": eleven})),
        ("design.customColors", json!({"name": "A", "colors": bad})),
        ("design.customColors", json!({"name": "A", "colors": [1, 2, 3]})),
        ("design.customColors", json!({"name": "", "colors": twelve(1)})),
        ("design.customColors", json!({"name": "   ", "colors": twelve(1)})),
        ("design.customColors", json!({"name": "../evil", "colors": twelve(1)})),
        ("design.customColors", json!({"name": "a/b", "colors": twelve(1)})),
        ("design.customColors", json!({"name": "a\\b", "colors": twelve(1)})),
        ("design.customColors", json!({"name": "x".repeat(10_000), "colors": twelve(1)})),
        ("design.customColors", json!({"name": "Craft", "colors": twelve(1)})),
        ("design.customFonts", json!({"name": "F", "heading": "", "body": "B"})),
        ("design.customFonts", json!({"name": "..", "heading": "A", "body": "B"})),
        ("design.customFonts", json!({"name": "F", "heading": "A".repeat(10_000), "body": "B"})),
        ("design.deleteCustom", json!({"kind": "colors"})),
        ("design.saveTheme", json!({"name": "x"})),
        ("design.saveTheme", json!({"name": "../../evil"})),
        ("design.openTheme", json!({"path": ""})),
        ("design.openTheme", json!({"path": "/no/such/file.thmx"})),
    ];
    for (id, v) in cases {
        assert!(s.run(id, &v).is_err(), "{id} {v} should be an error");
    }
    let list = run(&mut s, "design.customThemes", json!({}));
    assert!(list["colors"].as_array().unwrap().is_empty());
    assert!(list["fonts"].as_array().unwrap().is_empty());
}

#[test]
fn hostile_themes_folder_is_survived() {
    let dir = temp_dir("hostile");
    std::fs::write(dir.join(crate::cmd::themes::CUSTOM_FILE), b"{not json").unwrap();
    std::fs::write(dir.join("Broken.thmx"), b"PK\x03\x04garbage").unwrap();
    let mut s = s();
    s.load_custom_themes(&dir);
    let list = run(&mut s, "design.customThemes", json!({}));
    assert!(list["colors"].as_array().unwrap().is_empty());
    assert!(list["themes"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Names are matched case-insensitively, and so is the file behind a saved theme: deleting or
/// re-saving "seaside" finds `Seaside.thmx` even on a case-sensitive file system.
#[test]
fn theme_files_are_found_whatever_the_case_of_the_name() {
    let dir = temp_dir("case");
    let mut s = s();
    s.load_custom_themes(&dir);
    run(&mut s, "design.saveTheme", json!({"name": "Seaside"}));
    run(&mut s, "design.saveTheme", json!({"name": "seaside"}));
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(files.iter().filter(|f| f.ends_with(".thmx")).count(), 1, "{files:?}");
    assert_eq!(run(&mut s, "design.customThemes", json!({}))["themes"], json!(["Seaside"]));
    run(&mut s, "design.deleteCustom", json!({"kind": "theme", "name": "SEASIDE"}));
    let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "thmx")).collect();
    assert!(left.is_empty(), "the theme file was removed: {left:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A saved theme can't take a built-in theme's name (the built-in would win when applied), and a
/// file named like one in the themes folder is skipped.
#[test]
fn saved_themes_cant_shadow_built_ins() {
    let dir = temp_dir("builtin");
    let mut s = s();
    s.load_custom_themes(&dir);
    assert!(s.run("design.saveTheme", &json!({"name": "studio"})).is_err());
    assert!(!dir.join("studio.thmx").exists());
    let bytes = wordcraft_docx::write_thmx(&wordcraft_docx::Theme::of(&s.doc)).unwrap();
    std::fs::write(dir.join("Studio.thmx"), &bytes).unwrap();
    std::fs::write(dir.join("Mine.thmx"), &bytes).unwrap();
    let mut t = Session::new(wordcraft_doc::Document::new());
    t.load_custom_themes(&dir);
    assert_eq!(run(&mut t, "design.customThemes", json!({}))["themes"], json!(["Mine"]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Names that can't be file names on some system: Windows' reserved device names, a trailing dot
/// or space, more than 255 bytes.
#[test]
fn names_that_cant_be_files_anywhere_are_errors() {
    let mut s = s();
    let long = "ä".repeat(128);
    for name in ["CON", "nul", "Com1", "LPT9", "aux.thmx", "Name.", long.as_str()] {
        assert!(s.run("design.customColors", &json!({"name": name, "colors": twelve(1)})).is_err(), "{name}");
    }
    run(&mut s, "design.customColors", json!({"name": "Console", "colors": twelve(1)}));
}

/// Deleting a custom set changes no document: it isn't an edit to repeat or refuse.
#[test]
fn delete_custom_is_not_a_document_edit() {
    let mut s = s();
    run(&mut s, "design.customFonts", json!({"name": "Bookish", "heading": "A", "body": "B"}));
    s.dirty = false;
    run(&mut s, "design.deleteCustom", json!({"kind": "fonts", "name": "Bookish"}));
    assert!(!s.dirty);
    assert_ne!(s.last_command.as_ref().map(|c| c.0.as_str()), Some("design.deleteCustom"));
}

/// A rename is one command: the new set is in and the old one out, in memory and in `custom.json`.
#[test]
fn renaming_a_set_replaces_the_old_one_in_one_step() {
    let dir = temp_dir("rename");
    let mut s = s();
    s.load_custom_themes(&dir);
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": twelve(1)}));
    run(&mut s, "design.customFonts", json!({"name": "Bookish", "heading": "Palatino"}));
    run(&mut s, "design.customColors", json!({"name": "Dock", "colors": twelve(2), "renameFrom": "harbour"}));
    run(&mut s, "design.customFonts", json!({"name": "Booky", "heading": "Palatino", "renameFrom": "Bookish"}));
    let list = run(&mut s, "design.customThemes", json!({}));
    assert_eq!(list["colors"].as_array().unwrap().len(), 1);
    assert_eq!(list["colors"][0]["name"], "Dock");
    assert_eq!(list["fonts"].as_array().unwrap().len(), 1);
    assert_eq!(list["fonts"][0]["name"], "Booky");
    let mut b = self::s();
    b.load_custom_themes(&dir);
    let saved = run(&mut b, "design.customThemes", json!({}));
    assert_eq!(saved["colors"][0]["name"], "Dock", "custom.json holds the new set only");
    assert_eq!(saved["colors"].as_array().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rename that can't go through changes nothing: the old set stays.
#[test]
fn a_refused_rename_keeps_the_old_set() {
    let mut s = s();
    run(&mut s, "design.customColors", json!({"name": "Harbour", "colors": twelve(1)}));
    run(&mut s, "design.customColors", json!({"name": "Dock", "colors": twelve(2)}));
    assert!(s.run("design.customColors", &json!({"name": "Dock", "colors": twelve(3), "renameFrom": "Harbour"})).is_err(), "Dock is taken");
    assert!(s.run("design.customColors", &json!({"name": "Pier", "colors": twelve(3), "renameFrom": "Nowhere"})).is_err(), "no such set");
    assert!(s.run("design.customColors", &json!({"name": "Craft", "colors": twelve(3), "renameFrom": "Harbour"})).is_err(), "built-in name");
    let list = run(&mut s, "design.customThemes", json!({}));
    let names: Vec<&str> = list["colors"].as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
    assert_eq!(names, vec!["Harbour", "Dock"]);
}

/// A theme whose file can't be removed isn't deleted: it would come back from the folder.
#[test]
fn a_theme_whose_file_wont_go_stays() {
    let dir = temp_dir("delfail");
    let mut s = s();
    s.load_custom_themes(&dir);
    run(&mut s, "design.saveTheme", json!({"name": "Seaside"}));
    let file = dir.join("Seaside.thmx");
    assert!(file.exists());
    // Swap the file for a non-empty folder of the same name: removing it as a file fails.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir_all(file.join("inner")).unwrap();
    assert!(s.run("design.deleteCustom", &json!({"kind": "theme", "name": "Seaside"})).is_err());
    let list = run(&mut s, "design.customThemes", json!({}));
    assert_eq!(list["themes"], json!(["Seaside"]), "still listed after the failed delete");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn oversized_theme_data_is_refused_before_decoding() {
    let mut t = Session::new(wordcraft_doc::Document::new());
    let big = "A".repeat(wordcraft_docx::MAX_THEME_FILE.div_ceil(3) * 4 + 4);
    let err = t.run("design.openTheme", &json!({"path": "x.thmx", "data": big})).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
}

#[test]
fn saving_to_a_built_in_name_by_path_in_the_themes_folder_is_refused() {
    let dir = temp_dir("builtin-path");
    let mut t = s();
    t.load_custom_themes(&dir);
    let file = dir.join("Craft.thmx");
    let err = t.run("design.saveTheme", &json!({"path": file.to_string_lossy()})).unwrap_err();
    assert!(err.to_string().contains("built-in"), "{err}");
    assert!(!file.exists(), "nothing was written");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_themes_folder_is_recognised_however_the_path_is_spelled() {
    let dir = temp_dir("spelled");
    let mut t = s();
    t.load_custom_themes(&dir);
    let sneaky = dir.join("..").join(dir.file_name().unwrap()).join("Craft.thmx");
    assert!(t.run("design.saveTheme", &json!({"path": sneaky.to_string_lossy()})).is_err(), "built-in name via `..`");
    assert!(!dir.join("Craft.thmx").exists());
    let wrong_ext = dir.join("Mine.txt");
    let err = t.run("design.saveTheme", &json!({"path": wrong_ext.to_string_lossy()})).unwrap_err();
    assert!(err.to_string().contains(".thmx"), "{err}");
    assert!(!wrong_ext.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_link_followed_by_dotdot_still_finds_the_themes_folder() {
    let dir = temp_dir("linkdots");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let link = std::env::temp_dir().join(format!("wordcraft-themes-link-{}", std::process::id()));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&sub, &link).unwrap();
    let mut t = s();
    t.load_custom_themes(&dir);
    // `link/..` is the themes folder itself, because `link` points into it.
    let sneaky = link.join("..").join("Craft.thmx");
    assert!(t.run("design.saveTheme", &json!({"path": sneaky.to_string_lossy()})).is_err());
    assert!(!dir.join("Craft.thmx").exists());
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&dir);
}
