//! Library commands against folders of pictures drawn here in code (no third-party files).

use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::{Value, json};

use crate::file_access::{AutomationRoots, confine};
use crate::library::{MAX_DEPTH, MAX_ENTRIES, MAX_FILE_BYTES, MAX_FOLDERS, MAX_THUMB_HEIGHT};
use crate::{CmdError, Prefs, Session};

/// A fresh, empty folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-library-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A `w` × `h` picture of one colour, encoded as `format`.
fn picture(w: u32, h: u32, format: ImageFormat) -> Vec<u8> {
    let img = match format {
        // JPEG has no alpha channel.
        ImageFormat::Jpeg => DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, Rgb([20, 120, 200]))),
        _ => DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, Rgba([200, 60, 20, 255]))),
    };
    wordcraft_geom::picture::encode_format(&img, format).unwrap()
}

fn png(w: u32, h: u32) -> Vec<u8> {
    picture(w, h, ImageFormat::Png)
}

/// A session with an empty document and `dir` in the Library.
fn session_with(dir: &Path) -> Session {
    let mut ss = Session::new(wordcraft_doc::Document::new());
    ss.run("library.addFolder", &json!({"path": s(dir)})).unwrap();
    ss
}

fn list(ss: &mut Session, p: Value) -> Vec<Value> {
    ss.run("library.list", &p).unwrap().as_array().cloned().unwrap_or_default()
}

/// The file names `library.list` gives, in its order.
fn names(files: &[Value]) -> Vec<String> {
    files.iter().map(|f| f["file"].as_str().and_then(|f| Path::new(f).file_name()).unwrap().to_string_lossy().into_owned()).collect()
}

/// The pictures inserted in the document: their media bytes.
fn inserted(ss: &Session) -> Vec<Vec<u8>> {
    ss.doc.media.values().map(|m| m.to_vec()).collect()
}

fn decode_png(b64: &str) -> image::RgbaImage {
    let bytes = crate::cmd::insert::base64_decode(b64).unwrap();
    image::load_from_memory_with_format(&bytes, ImageFormat::Png).unwrap().to_rgba8()
}

#[test]
fn folders_are_added_listed_and_removed() {
    let dir = folder("basics");
    std::fs::write(dir.join("Sunrise.png"), png(30, 20)).unwrap();
    std::fs::write(dir.join("sea.JPG"), picture(8, 8, ImageFormat::Jpeg)).unwrap();
    std::fs::create_dir_all(dir.join("more")).unwrap();
    std::fs::write(dir.join("more/leaf.gif"), picture(4, 4, ImageFormat::Gif)).unwrap();
    // Not pictures the tab lists: other files, hidden files, Office lock files.
    for name in ["notes.txt", "drawing.svg", ".hidden.png", "~$lock.png", "report.docx"] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    let mut ss = session_with(&dir);
    let real = s(&std::fs::canonicalize(&dir).unwrap());
    assert_eq!(ss.prefs.library_folders, vec![real.clone()]);
    // Adding it again, however it is spelled, keeps one.
    let again = ss.run("library.addFolder", &json!({"path": format!("{}/more/..", s(&dir))})).unwrap();
    assert_eq!(again["added"], json!(false));
    assert_eq!(ss.prefs.library_folders.len(), 1);
    let f = ss.run("library.folders", &json!({})).unwrap();
    assert_eq!(f[0]["path"], json!(real));
    assert_eq!(f[0]["files"], json!(3), "{f}");

    let files = list(&mut ss, json!({}));
    assert_eq!(names(&files), ["sea.JPG", "Sunrise.png", "leaf.gif"], "a folder's own files by name in any case, then its subfolders'");
    let sunrise = &files[1];
    assert_eq!(sunrise["title"], json!("Sunrise"), "the file's title is its name without the extension");
    assert_eq!(sunrise["pictures"], json!([{"index": 0, "title": "Sunrise.png", "hidden": false}]), "one picture, titled by its file name");
    assert_eq!(sunrise["pictureCount"], json!(1));
    assert!(sunrise.get("error").is_none(), "{sunrise}");
    // A picture is known from its name, so the quick listing has it too.
    let quick = list(&mut ss, json!({"pictures": false}));
    assert_eq!(quick, files, "listed alike with or without reading");
    // One folder, one file.
    assert_eq!(list(&mut ss, json!({"folder": s(&dir)})).len(), 3);
    assert_eq!(list(&mut ss, json!({"file": s(&dir.join("sea.JPG"))})).len(), 1);

    let removed = ss.run("library.removeFolder", &json!({"path": s(&dir)})).unwrap();
    assert_eq!(removed["folders"], json!([]));
    assert!(ss.prefs.library_folders.is_empty());
    assert!(dir.join("Sunrise.png").exists(), "removing a folder leaves its files alone");
    assert!(ss.run("library.removeFolder", &json!({"path": s(&dir)})).is_err(), "no longer a Library folder");
    let missing = ss.run("library.addFolder", &json!({"path": s(&dir.join("missing"))}));
    assert!(missing.is_err(), "{missing:?}");
    let file = ss.run("library.addFolder", &json!({"path": s(&dir.join("Sunrise.png"))})).unwrap_err();
    assert!(file.to_string().contains("not a folder"), "{file}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn at_most_32_folders() {
    let base = folder("many");
    let mut ss = Session::new(wordcraft_doc::Document::new());
    for i in 0..=MAX_FOLDERS {
        let d = base.join(format!("f{i}"));
        std::fs::create_dir_all(&d).unwrap();
        let r = ss.run("library.addFolder", &json!({"path": s(&d)}));
        if i < MAX_FOLDERS {
            r.unwrap();
        } else {
            assert!(r.unwrap_err().to_string().contains("at most 32"));
        }
    }
    assert_eq!(ss.prefs.library_folders.len(), MAX_FOLDERS);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_folder_is_scanned_to_5000_entries() {
    let dir = folder("entries");
    for i in 0..=MAX_ENTRIES {
        std::fs::write(dir.join(format!("f{i:05}.txt")), "").unwrap();
    }
    let mut ss = session_with(&dir);
    let f = ss.run("library.folders", &json!({})).unwrap();
    assert_eq!(f[0]["entries"], json!(MAX_ENTRIES), "{f}");
    assert_eq!(f[0]["truncated"], json!(true));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn folders_deeper_than_8_are_not_entered() {
    let dir = folder("depth");
    let mut at = dir.clone();
    let bytes = png(2, 2);
    for level in 0..=MAX_DEPTH + 1 {
        std::fs::write(at.join(format!("level{level}.png")), &bytes).unwrap();
        at = at.join(format!("d{level}"));
        std::fs::create_dir_all(&at).unwrap();
    }
    let mut ss = session_with(&dir);
    let files = list(&mut ss, json!({"pictures": false}));
    let found = names(&files);
    assert_eq!(found.len(), MAX_DEPTH + 1, "levels 0 to 8: {found:?}");
    assert!(found.contains(&format!("level{MAX_DEPTH}.png")) && !found.contains(&format!("level{}.png", MAX_DEPTH + 1)));
    let f = ss.run("library.folders", &json!({})).unwrap();
    assert_eq!(f[0]["skippedDeep"], json!(1), "{f}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn files_over_256_mb_are_listed_with_an_error_and_never_read() {
    let dir = folder("large");
    // Sparse: no disk space is used.
    let big = std::fs::File::create(dir.join("huge.png")).unwrap();
    big.set_len(MAX_FILE_BYTES + 1).unwrap();
    drop(big);
    let mut ss = session_with(&dir);
    for read in [true, false] {
        let files = list(&mut ss, json!({"pictures": read}));
        assert_eq!(files.len(), 1);
        assert!(files[0]["error"].as_str().is_some_and(|e| e.contains("256 MB")), "{}", files[0]);
    }
    // Refresh looks at the failed file again: still too large, still not read.
    let files = list(&mut ss, json!({"refresh": true}));
    assert!(files[0]["error"].as_str().is_some_and(|e| e.contains("256 MB")), "{}", files[0]);
    let path = s(&dir.join("huge.png"));
    // The pane searches files that failed too: this one is still not read.
    assert!(ss.run("library.search", &json!({"query": "huge", "file": path})).is_ok());
    assert_eq!(ss.library.sizes().3, 0, "no picture was opened");
    assert!(ss.run("library.thumbnail", &json!({"file": path, "picture": 0})).is_err());
    assert!(ss.run("library.insert", &json!({"file": path})).is_err());
    assert!(inserted(&ss).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn links_are_not_followed_out_of_a_folder() {
    let base = folder("links");
    let (lib, outside) = (base.join("lib"), base.join("outside"));
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.png"), png(3, 3)).unwrap();
    std::os::unix::fs::symlink(&outside, lib.join("away")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.png"), lib.join("linked.png")).unwrap();
    let mut ss = session_with(&lib);
    assert!(list(&mut ss, json!({})).is_empty());
    let f = ss.run("library.folders", &json!({})).unwrap();
    assert_eq!(f[0]["skippedLinks"], json!(2), "{f}");
    // Naming the link doesn't reach the file outside either.
    for (id, p) in [
        ("library.thumbnail", json!({"file": s(&lib.join("linked.png")), "picture": 0})),
        ("library.insert", json!({"file": s(&lib.join("linked.png"))})),
    ] {
        let e = ss.run(id, &p).unwrap_err();
        assert!(e.to_string().contains("not in a Library folder"), "{id}: {e}");
    }
    assert!(inserted(&ss).is_empty());
    let _ = std::fs::remove_dir_all(base);
}

/// A PNG whose header says it is `w` × `h` with no pixel data behind it.
fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0]);
    b.extend([0, 0, 0, 0]);
    b
}

/// A 4 × 4 RGB TIFF that says its strips are JPEG (#3645): WordCraft never decodes one.
fn jpeg_strip_tiff() -> Vec<u8> {
    let mut t = picture(4, 4, ImageFormat::Tiff);
    let ifd = u32::from_le_bytes(t[4..8].try_into().unwrap()) as usize;
    let n = usize::from(u16::from_le_bytes([t[ifd], t[ifd + 1]]));
    let e = (0..n).map(|i| ifd + 2 + 12 * i).find(|&e| t[e..e + 2] == 259u16.to_le_bytes()).unwrap();
    t[e + 8..e + 10].copy_from_slice(&7u16.to_le_bytes());
    t
}

#[test]
fn corrupt_and_hostile_pictures_have_no_thumbnail_and_are_not_inserted() {
    let dir = folder("corrupt");
    let good = png(5, 5);
    // Cut off inside its pixel data: the header reads, the pixels don't.
    let noisy = RgbaImage::from_fn(64, 64, |x, y| Rgba([(x * 37 % 256) as u8, (y * 91 % 256) as u8, ((x ^ y) * 13 % 256) as u8, 255]));
    let mut truncated = wordcraft_geom::picture::encode_format(&DynamicImage::ImageRgba8(noisy), ImageFormat::Png).unwrap();
    let idat = truncated.windows(4).position(|w| w == b"IDAT").unwrap();
    truncated.truncate(idat + 40);
    let files: [(&str, Vec<u8>); 7] = [
        ("good.png", good.clone()),
        ("noise.png", (0..512u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect()),
        ("empty.jpg", vec![]),
        ("half.png", truncated),
        // Declares more pixels than any budget: refused from the header, nothing allocated.
        ("vast.png", png_header(1_000_000, 1_000_000)),
        ("strips.tif", jpeg_strip_tiff()),
        // A PNG named as a JPEG is still the picture it is.
        ("named-wrong.jpg", png(2, 2)),
    ];
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    let mut ss = session_with(&dir);
    let listed = list(&mut ss, json!({}));
    // Listing reads no picture, so each is listed by its name, damaged or not.
    assert_eq!(listed.len(), files.len(), "every file is listed: {listed:?}");
    assert!(listed.iter().all(|f| f.get("error").is_none() && f["pictureCount"] == json!(1)), "{listed:?}");
    // Reading one for its thumbnail or an insert says what is wrong with it.
    for name in ["noise.png", "empty.jpg", "vast.png", "strips.tif", "half.png"] {
        let path = s(&dir.join(name));
        let r = ss.run("library.thumbnail", &json!({"file": path, "picture": 0}));
        assert!(matches!(&r, Err(CmdError::Failed(m)) if !m.is_empty()), "{name}: {r:?}");
        if name != "half.png" {
            assert!(ss.run("library.insert", &json!({"file": path})).is_err(), "{name}");
        }
    }
    assert!(inserted(&ss).is_empty(), "nothing went in");
    // The good ones still work beside them.
    ss.run("library.thumbnail", &json!({"file": s(&dir.join("named-wrong.jpg")), "picture": 0})).unwrap();
    ss.run("library.insert", &json!({"file": s(&dir.join("good.png"))})).unwrap();
    assert_eq!(inserted(&ss), vec![good]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Listing and searching read no picture: a file that can't be read at all is still listed and
/// found by its name; only drawing or inserting it reads it, and says why that failed.
#[cfg(unix)]
#[test]
fn listing_and_search_read_no_picture() {
    use std::os::unix::fs::PermissionsExt;
    let dir = folder("unread");
    let locked = dir.join("locked.png");
    std::fs::write(&locked, b"not a picture either").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let mut ss = session_with(&dir);
    let listed = list(&mut ss, json!({}));
    assert_eq!(names(&listed), ["locked.png"]);
    assert_eq!(listed[0]["pictures"], json!([{"index": 0, "title": "locked.png", "hidden": false}]), "{}", listed[0]);
    assert!(listed[0].get("error").is_none(), "{}", listed[0]);
    assert_eq!(ss.run("library.search", &json!({"query": "LOCKED"})).unwrap().as_array().map(Vec::len), Some(1));
    assert_eq!(ss.library.sizes().3, 0, "no picture was opened");
    let path = s(&locked);
    assert!(ss.run("library.thumbnail", &json!({"file": path, "picture": 0})).is_err());
    assert!(ss.run("library.insert", &json!({"file": path})).is_err());
    assert!(inserted(&ss).is_empty());
    let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn search_finds_file_names_and_formats_in_any_case() {
    let dir = folder("search");
    std::fs::create_dir_all(dir.join("Trips")).unwrap();
    std::fs::write(dir.join("Harbour at dawn.png"), png(4, 4)).unwrap();
    std::fs::write(dir.join("Trips/harbour-wall.jpg"), picture(4, 4, ImageFormat::Jpeg)).unwrap();
    std::fs::write(dir.join("kiln.bmp"), picture(4, 4, ImageFormat::Bmp)).unwrap();
    let mut ss = session_with(&dir);
    let hits = ss.run("library.search", &json!({"query": "HARBOUR"})).unwrap();
    let found: Vec<&str> = hits.as_array().unwrap().iter().map(|h| h["title"].as_str().unwrap()).collect();
    assert_eq!(found, ["Harbour at dawn.png", "harbour-wall.jpg"], "{hits}");
    assert!(hits.as_array().unwrap().iter().all(|h| h["picture"] == json!(0) && h["match"] == json!("title")), "{hits}");
    // The format is searched too: a .jpg is a JPEG picture.
    let jpeg = ss.run("library.search", &json!({"query": "jpeg"})).unwrap();
    assert_eq!(jpeg.as_array().map(Vec::len), Some(1), "{jpeg}");
    assert_eq!(jpeg[0]["match"], json!("text"));
    // In one folder only, with a limit.
    let trips = s(&dir.join("Trips"));
    ss.run("library.addFolder", &json!({"path": trips})).unwrap();
    let one = ss.run("library.search", &json!({"query": "harbour", "folder": trips})).unwrap();
    assert_eq!(one.as_array().map(Vec::len), Some(1), "{one}");
    // In one file only, as the pane searches a few files a frame.
    let dawn = s(&dir.join("Harbour at dawn.png"));
    let one = ss.run("library.search", &json!({"query": "harbour", "file": dawn})).unwrap();
    assert_eq!(names(one.as_array().unwrap()), ["Harbour at dawn.png"], "{one}");
    assert_eq!((&one[0]["picture"], &one[0]["hidden"], &one[0]["match"]), (&json!(0), &json!(false), &json!("title")), "{one}");
    assert_eq!(ss.run("library.search", &json!({"query": "kiln", "file": dawn})).unwrap(), json!([]));
    let limited = ss.run("library.search", &json!({"query": "a", "limit": 1})).unwrap();
    assert_eq!(limited.as_array().map(Vec::len), Some(1));
    assert_eq!(ss.run("library.search", &json!({"query": "nothing like it"})).unwrap(), json!([]));
    // A query is at most 256 characters, as the pane's search field takes.
    assert_eq!(ss.run("library.search", &json!({"query": "x".repeat(256)})).unwrap(), json!([]));
    for bad in [json!({}), json!({"query": "  "}), json!({"query": "x".repeat(257)}), json!({"query": "a", "limit": 0})] {
        assert!(ss.run("library.search", &bad).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thumbnails_are_bounded_drawn_from_the_picture_and_cached() {
    let dir = folder("thumbs");
    std::fs::write(dir.join("wide.png"), png(1000, 10)).unwrap();
    std::fs::write(dir.join("tall.png"), png(4, 6000)).unwrap();
    std::fs::write(dir.join("small.png"), png(20, 10)).unwrap();
    let mut ss = session_with(&dir);
    let at = |name: &str| s(&dir.join(name));
    let wide = ss.run("library.thumbnail", &json!({"file": at("wide.png"), "picture": 0})).unwrap();
    assert_eq!((wide["width"].clone(), wide["height"].clone()), (json!(160), json!(2)), "the default width");
    let img = decode_png(wide["png"].as_str().unwrap());
    assert_eq!(img.dimensions(), (160, 2));
    assert_eq!(img.get_pixel(80, 1).0, [200, 60, 20, 255], "the picture's own colour");
    let tall = ss.run("library.thumbnail", &json!({"file": at("tall.png"), "picture": 0, "width": 512})).unwrap();
    assert_eq!(tall["height"], json!(MAX_THUMB_HEIGHT), "{tall}");
    assert!(tall["width"].as_u64().unwrap() <= 4);
    let small = ss.run("library.thumbnail", &json!({"file": at("small.png"), "picture": 0, "width": 512})).unwrap();
    assert_eq!((small["width"].clone(), small["height"].clone()), (json!(20), json!(10)), "not enlarged");
    // Drawn once, then from the cache.
    let (thumbs, ..) = ss.library.sizes();
    ss.run("library.thumbnail", &json!({"file": at("wide.png"), "picture": 0})).unwrap();
    assert_eq!(ss.library.sizes().0, thumbs);
    for bad in [
        json!({"file": at("wide.png"), "picture": 1}),
        json!({"file": at("wide.png")}),
        json!({"file": at("wide.png"), "picture": 0, "width": 4}),
        json!({"file": at("wide.png"), "picture": 0, "width": 100_000}),
    ] {
        assert!(ss.run("library.thumbnail", &bad).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_changed_file_is_read_again() {
    let dir = folder("changed");
    let path = dir.join("swap.png");
    std::fs::write(&path, png(40, 40)).unwrap();
    let mut ss = session_with(&dir);
    let p = json!({"file": s(&path), "picture": 0, "width": 512});
    assert_eq!(ss.run("library.thumbnail", &p).unwrap()["width"], json!(40));
    // Another size, so the stamp differs even where file times are coarse.
    std::fs::write(&path, png(64, 8)).unwrap();
    let again = ss.run("library.thumbnail", &p).unwrap();
    assert_eq!((again["width"].clone(), again["height"].clone()), (json!(64), json!(8)));
    ss.run("library.insert", &json!({"file": s(&path)})).unwrap();
    assert_eq!(inserted(&ss), vec![png(64, 8)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn inserting_a_picture_is_one_undo_step() {
    let dir = folder("insert");
    let bytes = png(96, 48);
    std::fs::write(dir.join("logo.png"), &bytes).unwrap();
    let mut ss = session_with(&dir);
    ss.run("text.insert", &json!({"text": "Before "})).unwrap();
    let depth = ss.undo_depth();
    let r = ss.run("library.insert", &json!({"file": s(&dir.join("logo.png")), "picture": 0, "alt": "Our logo"})).unwrap();
    // Natural size at 96 ppi, as Insert › Pictures gives it.
    assert_eq!((r["width"].clone(), r["height"].clone()), (json!(72.0), json!(36.0)), "{r}");
    assert!(r["file"].as_str().is_some_and(|f| f.ends_with("logo.png")), "{r}");
    assert_eq!(inserted(&ss), vec![bytes.clone()], "the file's own bytes");
    assert_eq!(ss.undo_depth(), depth + 1, "one undo step");
    assert_eq!(ss.undo_labels().first().map(String::as_str), Some("Insert from Library"));
    assert!(ss.dirty);
    let para = ss.doc.body.first().and_then(|b| b.as_para()).unwrap();
    let alt = para.objects.iter().find_map(|o| match o {
        wordcraft_doc::para::InlineObject::Image { alt, .. } => Some(alt.clone()),
        _ => None,
    });
    assert_eq!(alt.as_deref(), Some("Our logo"));
    assert!(ss.undo());
    assert!(ss.doc.body.first().and_then(|b| b.as_para()).unwrap().objects.is_empty(), "undo takes it out");
    // A width in points, fitted like Insert › Pictures; the index may be left out.
    let r = ss.run("library.insert", &json!({"file": s(&dir.join("logo.png")), "width": 36})).unwrap();
    assert_eq!((r["width"].clone(), r["height"].clone()), (json!(36.0), json!(18.0)), "{r}");
    let e = ss.run("library.insert", &json!({"file": s(&dir.join("logo.png")), "picture": 1})).unwrap_err();
    assert!(matches!(e, CmdError::Params(_)), "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_pictures_in_a_library_folder_are_inserted() {
    let base = folder("elsewhere");
    let (lib, other) = (base.join("lib"), base.join("other"));
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("stray.png"), png(2, 2)).unwrap();
    std::fs::write(lib.join("notes.txt"), b"text").unwrap();
    let mut ss = session_with(&lib);
    let e = ss.run("library.insert", &json!({"file": s(&other.join("stray.png"))})).unwrap_err();
    assert!(e.to_string().contains("not in a Library folder"), "{e}");
    let e = ss.run("library.insert", &json!({"file": s(&lib.join("notes.txt"))})).unwrap_err();
    assert!(e.to_string().contains("is not a .png"), "{e}");
    let e = ss.run("library.insert", &json!({"file": s(&base.join("lib/../other/stray.png"))})).unwrap_err();
    assert!(e.to_string().contains("not in a Library folder"), "{e}");
    assert!(inserted(&ss).is_empty());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_read_root_confines_every_library_command() {
    let base = folder("confined");
    let (inside, outside) = (base.join("in"), base.join("out"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(inside.join("ok.png"), png(3, 3)).unwrap();
    std::fs::write(outside.join("secret.png"), png(3, 3)).unwrap();
    // The person at the keyboard added both folders earlier.
    let mut ss = session_with(&inside);
    ss.run("library.addFolder", &json!({"path": s(&outside)})).unwrap();
    let roots = AutomationRoots::new(Some(&inside), None).unwrap().unwrap();
    confine(Some(&roots), || {
        let other = base.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let e = ss.run("library.addFolder", &json!({"path": s(&other)})).unwrap_err();
        assert!(e.to_string().contains("outside the read root"), "{e}");
        let f = ss.run("library.folders", &json!({})).unwrap();
        assert!(f[0].get("error").is_none(), "{f}");
        assert!(f[1]["error"].as_str().is_some_and(|e| e.contains("outside the read root")), "{f}");
        let files = list(&mut ss, json!({}));
        assert_eq!(names(&files), ["ok.png"], "only the folder inside the root");
        assert!(ss.run("library.list", &json!({"folder": s(&outside)})).is_err());
        assert_eq!(ss.run("library.search", &json!({"query": "secret"})).unwrap(), json!([]));
        let secret = s(&outside.join("secret.png"));
        for (id, p) in [
            ("library.thumbnail", json!({"file": secret, "picture": 0})),
            ("library.insert", json!({"file": secret})),
            ("library.list", json!({"file": secret})),
            ("library.search", json!({"query": "secret", "file": secret})),
        ] {
            let e = ss.run(id, &p).unwrap_err();
            assert!(e.to_string().contains("outside the read root"), "{id}: {e}");
        }
        assert!(inserted(&ss).is_empty());
        let ok = s(&inside.join("ok.png"));
        ss.run("library.thumbnail", &json!({"file": ok, "picture": 0})).unwrap();
        ss.run("library.insert", &json!({"file": ok})).unwrap();
    });
    assert_eq!(inserted(&ss).len(), 1);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn library_folders_survive_a_fresh_session() {
    let dir = folder("persist");
    std::fs::write(dir.join("kept.png"), png(2, 2)).unwrap();
    let ss = session_with(&dir);
    let saved = serde_json::to_string(&ss.prefs).unwrap();
    assert!(saved.contains("libraryFolders"), "{saved}");
    let mut fresh = Session::new(wordcraft_doc::Document::new());
    fresh.prefs = serde_json::from_str::<Prefs>(&saved).unwrap();
    assert_eq!(names(&list(&mut fresh, json!({}))), ["kept.png"]);
    // Preferences from before the Library have none.
    let old: Prefs = serde_json::from_str(r#"{"countNotes": false}"#).unwrap();
    assert!(old.library_folders.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reading_and_folder_commands_are_not_edits() {
    let dir = folder("query");
    std::fs::write(dir.join("a.png"), png(2, 2)).unwrap();
    let path = s(&dir.join("a.png"));
    let mut ss = Session::new(wordcraft_doc::Document::new());
    ss.run("tools.recordMacro", &json!({"name": "Steps"})).unwrap();
    for (id, p) in [
        ("library.addFolder", json!({"path": s(&dir)})),
        ("library.folders", json!({})),
        ("library.list", json!({})),
        ("library.search", json!({"query": "a"})),
        ("library.thumbnail", json!({"file": path, "picture": 0})),
        ("view.libraryPane", json!({"value": true})),
        ("library.removeFolder", json!({"path": s(&dir)})),
    ] {
        ss.run(id, &p).unwrap();
        assert!(!ss.can_undo(), "{id} is not an undo step");
        assert!(!ss.dirty, "{id} leaves no unsaved change");
    }
    assert!(ss.view.library_pane);
    let steps = ss.recording.as_ref().map(|(_, steps)| steps.len());
    assert_eq!(steps, Some(0), "nor a macro step");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hostile_library_params_are_errors_not_panics() {
    let dir = folder("hostile");
    std::fs::write(dir.join("a.png"), png(3, 3)).unwrap();
    let file = s(&dir.join("a.png"));
    let mut ss = session_with(&dir);
    let long = "x".repeat(100_000);
    let values = [
        json!(null),
        json!(""),
        json!("../../../../etc/passwd"),
        json!("a\u{0}b.png"),
        json!(long),
        json!(-1),
        json!(1e308),
        json!(u64::MAX),
        json!(f64::MIN_POSITIVE),
        json!([file]),
        json!({"file": file}),
        json!(file),
        json!(s(&dir)),
    ];
    let reg = crate::cmd::registry();
    let ids: Vec<&str> = reg.all().iter().map(|c| c.id).filter(|id| id.starts_with("library.") && *id != "library.removeFolder").collect();
    assert_eq!(ids.len(), 6, "{ids:?}");
    for id in &ids {
        for v in &values {
            for key in ["path", "file", "folder", "query", "picture", "pictures", "width", "limit", "alt"] {
                let mut p = json!({"file": file, "picture": 0, "query": "a", "path": s(&dir)});
                p[key] = v.clone();
                let r = ss.run(id, &p);
                assert!(!matches!(&r, Err(CmdError::Failed(m)) if m.contains(" panicked: ")), "{id} {key}={v}: {r:?}");
            }
        }
    }
    assert!(ss.prefs.library_folders.len() <= MAX_FOLDERS);
    // Every insert that went through is one whole picture; nothing else changed.
    assert!(inserted(&ss).iter().all(|m| *m == png(3, 3)));
    ss.run("library.removeFolder", &json!({"path": s(&dir)})).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// One WMF record (MS-WMF 2.3): its size in 16-bit words, its function, its parameters.
fn wmf_rec(func: u16, params: &[u16]) -> Vec<u8> {
    let mut v = ((3 + params.len()) as u32).to_le_bytes().to_vec();
    v.extend(func.to_le_bytes());
    v.extend(params.iter().flat_map(|p| p.to_le_bytes()));
    v
}

/// A placeable WMF (MS-WMF 2.3.2.3) whose bounds are `right` × `bottom` at `inch` units an inch:
/// `records`, then the end record.
fn wmf(right: u16, bottom: u16, inch: u16, records: &[Vec<u8>]) -> Vec<u8> {
    let mut f = 0x9AC6_CDD7u32.to_le_bytes().to_vec();
    // Handle, bounds (left, top, right, bottom), units an inch, reserved, checksum.
    for v in [0, 0, 0, right, bottom, inch, 0, 0, 0] {
        f.extend(v.to_le_bytes());
    }
    let body = [records.concat(), wmf_rec(0, &[])].concat();
    let words = (18 + body.len()) / 2;
    // Memory metafile, 9-word header, version 3, size in words, objects, largest record, members.
    for v in [1u16, 9, 0x300, words as u16, (words >> 16) as u16, 1, 7, 0, 0] {
        f.extend(v.to_le_bytes());
    }
    f.extend(body);
    f
}

/// A solid brush of RGB (0, 160, 0), selected, and a rectangle given as bottom, right, top, left.
fn green_rect(rect: [u16; 4]) -> [Vec<u8>; 3] {
    [wmf_rec(0x02FC, &[0, 0xA000, 0x0000, 0]), wmf_rec(0x012D, &[0]), wmf_rec(0x041B, &rect)]
}

/// A WMF two inches by one, green over its left half.
fn green_left_wmf() -> Vec<u8> {
    wmf(2880, 1440, 1440, &green_rect([1440, 1440, 0, 0]))
}

/// One EMF record (MS-EMF 2.3): its type, its size in bytes, its payload.
fn emf_rec(typ: u32, payload: &[i32]) -> Vec<u8> {
    let mut v = typ.to_le_bytes().to_vec();
    v.extend(((8 + 4 * payload.len()) as u32).to_le_bytes());
    v.extend(payload.iter().flat_map(|p| p.to_le_bytes()));
    v
}

/// An EMF (MS-EMF 2.3.4.2) framed by `frame` (hundredths of a millimetre) on a device of 4 units a
/// millimetre: `records`, then the end record.
fn emf(frame: [i32; 4], records: &[Vec<u8>]) -> Vec<u8> {
    let body = [records.concat(), emf_rec(14, &[0, 0, 0])].concat();
    let mut h = Vec::new();
    // Type, header size, bounds (device units), frame.
    for v in [1, 88, 0, 0, frame[2] / 25, frame[3] / 25, frame[0], frame[1], frame[2], frame[3]] {
        h.extend(i32::to_le_bytes(v));
    }
    // " EMF", version, file size, records, handles and reserved, description, palette.
    let size = (88 + body.len()) as i32;
    let records = records.len() as i32 + 2;
    for v in [0x464D_4520, 0x0001_0000, size, records, 2, 0, 0, 0] {
        h.extend(i32::to_le_bytes(v));
    }
    // The device: 1,000 units over 250 mm each way.
    for v in [1000, 1000, 250, 250] {
        h.extend(i32::to_le_bytes(v));
    }
    [h, body].concat()
}

/// An EMF 50 mm by 25 mm (200 × 100 device units), blue (RGB 20, 100, 200) over its top half.
fn blue_top_emf() -> Vec<u8> {
    let brush = emf_rec(39, &[1, 0, 0x00C8_6414, 0]);
    emf([0, 0, 5000, 2500], &[brush, emf_rec(37, &[1]), emf_rec(43, &[0, 0, 200, 50])])
}

/// The picture objects in the document's first paragraph: (media key, width, height, alt).
fn pictures(ss: &Session) -> Vec<(String, f32, f32, String)> {
    let Some(para) = ss.doc.body.first().and_then(|b| b.as_para()) else { return vec![] };
    para.objects
        .iter()
        .filter_map(|o| match o {
            wordcraft_doc::para::InlineObject::Image { media, w, h, alt, .. } => Some((media.clone(), *w, *h, alt.clone())),
            _ => None,
        })
        .collect()
}

/// #2868: Windows Metafiles and Enhanced Metafiles (built here from the public specs) are listed,
/// drawn as thumbnails as pages draw them, and inserted as Insert › Pictures inserts them.
#[test]
fn metafiles_are_listed_thumbnailed_and_inserted() {
    let dir = folder("metafiles");
    let (wmf, emf) = (green_left_wmf(), blue_top_emf());
    std::fs::write(dir.join("Chart.wmf"), &wmf).unwrap();
    std::fs::write(dir.join("diagram.EMF"), &emf).unwrap();
    let mut ss = session_with(&dir);
    let at = |name: &str| s(&dir.join(name));
    let files = list(&mut ss, json!({}));
    assert_eq!(names(&files), ["Chart.wmf", "diagram.EMF"]);
    for f in &files {
        assert!(f.get("error").is_none(), "{f}");
        assert_eq!(f["pictureCount"], json!(1), "{f}");
    }
    let f = ss.run("library.folders", &json!({})).unwrap();
    assert_eq!(f[0]["files"], json!(2), "{f}");
    for (query, title) in [("WMF", "Chart.wmf"), ("emf", "diagram.EMF")] {
        let hits = ss.run("library.search", &json!({"query": query})).unwrap();
        assert_eq!(hits.as_array().map(Vec::len), Some(1), "{query}: {hits}");
        assert_eq!(hits[0]["title"], json!(title));
    }

    // Thumbnails: the requested width, the picture's shape, its own colours, nothing behind it.
    let t = ss.run("library.thumbnail", &json!({"file": at("Chart.wmf"), "picture": 0})).unwrap();
    assert_eq!((t["width"].clone(), t["height"].clone()), (json!(160), json!(80)), "{t}");
    let img = decode_png(t["png"].as_str().unwrap());
    assert_eq!(img.dimensions(), (160, 80));
    assert_eq!(img.get_pixel(40, 40).0, [0, 160, 0, 255], "green on the left");
    assert_eq!(img.get_pixel(120, 40).0[3], 0, "nothing on the right");
    // A vector picture is drawn the full width asked for, even past its own size: it stays sharp.
    let t = ss.run("library.thumbnail", &json!({"file": at("diagram.EMF"), "picture": 0, "width": 512})).unwrap();
    assert_eq!((t["width"].clone(), t["height"].clone()), (json!(512), json!(256)), "{t}");
    let img = decode_png(t["png"].as_str().unwrap());
    assert_eq!(img.get_pixel(256, 64).0, [20, 100, 200, 255], "blue on top");
    assert_eq!(img.get_pixel(256, 192).0[3], 0, "nothing below");
    let small = ss.run("library.thumbnail", &json!({"file": at("diagram.EMF"), "picture": 0, "width": 32})).unwrap();
    assert_eq!((small["width"].clone(), small["height"].clone()), (json!(32), json!(16)), "{small}");

    // Inserted as Insert › Pictures inserts the same file: its own frame, its own bytes.
    let mut direct = Session::new(wordcraft_doc::Document::new());
    ss.run("text.insert", &json!({"text": "Figure "})).unwrap();
    for (name, bytes, ext, alt) in [("Chart.wmf", &wmf, "wmf", "Sales"), ("diagram.EMF", &emf, "emf", "Flow")] {
        let depth = ss.undo_depth();
        let r = ss.run("library.insert", &json!({"file": at(name), "alt": alt})).unwrap();
        let d = direct.run("insert.picture", &json!({"path": at(name)})).unwrap();
        assert_eq!((r["width"].clone(), r["height"].clone()), (d["width"].clone(), d["height"].clone()), "{name}: {r} {d}");
        assert!(r["media"].as_str().is_some_and(|m| m.ends_with(&format!(".{ext}"))), "{name}: {r}");
        assert!(r["file"].as_str().is_some_and(|f| f.ends_with(name)), "{r}");
        assert_eq!(ss.undo_depth(), depth + 1, "{name}: one undo step");
        assert_eq!(ss.undo_labels().first().map(String::as_str), Some("Insert from Library"));
        assert_eq!(ss.doc.media.get(r["media"].as_str().unwrap()).map(|m| m.to_vec()).as_ref(), Some(bytes), "{name}: the file's own bytes");
    }
    let placed = pictures(&ss);
    assert_eq!(placed.len(), 2, "{placed:?}");
    assert_eq!((placed[0].1, placed[0].2), (144.0, 72.0), "a placeable WMF's frame: two inches by one");
    assert!((placed[1].1 - 141.73).abs() < 0.01 && (placed[1].2 - 70.87).abs() < 0.01, "an EMF's frame: 50 mm by 25 mm, {placed:?}");
    assert_eq!((placed[0].3.as_str(), placed[1].3.as_str()), ("Sales", "Flow"));
    assert!(ss.undo());
    assert_eq!(pictures(&ss).len(), 1, "undo takes the last one out");
    let _ = std::fs::remove_dir_all(dir);
}

/// Damaged metafiles, and one with no size at all, are listed with why and draw and insert
/// nothing; hostile frame sizes are drawn and inserted within the bounds pictures have. Nothing
/// panics.
#[test]
fn damaged_and_hostile_metafiles_are_listed_with_their_error() {
    let dir = folder("bad-metafiles");
    let (good, emf_good) = (green_left_wmf(), blue_top_emf());
    let noise: Vec<u8> = (0..512u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    let mut long_record = good[..40].to_vec();
    long_record.extend([0xFF, 0xFF, 0xFF, 0xFF, 0xFC, 0x02]);
    let mut long_header = emf_good.clone();
    long_header[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let rect = || [emf_rec(39, &[1, 0, 0x00C8_6414, 0]), emf_rec(37, &[1]), emf_rec(43, &[0, 0, 200, 50])];
    // (name, bytes, can it be drawn?)
    let files: [(&str, Vec<u8>, bool); 12] = [
        ("header-alone.emf", emf_good[..88].to_vec(), false),
        ("cut-in-header.wmf", good[..30].to_vec(), false),
        ("cut-in-record.wmf", good[..44].to_vec(), false),
        ("record-past-the-end.wmf", long_record, false),
        ("header-past-the-end.emf", long_header, false),
        ("noise.emf", noise.clone(), false),
        // No frame, no bounds and nothing drawn (SETBKMODE alone): no picture at all.
        ("no-size.emf", emf([0, 0, 0, 0], &[emf_rec(18, &[1])]), false),
        // A frame with no height: what the file draws stands in for it.
        ("flat-frame.emf", emf([0, 0, 5000, 0], &rect()), true),
        ("vast.emf", emf([0, 0, i32::MAX, i32::MAX], &rect()), true),
        ("widest.emf", emf([i32::MIN, i32::MIN, i32::MAX, i32::MAX], &rect()), true),
        ("tall.wmf", wmf(1, 32767, 1, &green_rect([32767, 1, 0, 0])), true),
        ("good.wmf", good.clone(), true),
    ];
    for (name, bytes, _) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    let mut ss = session_with(&dir);
    let listed = list(&mut ss, json!({}));
    assert_eq!(listed.len(), files.len(), "every file is listed: {listed:?}");
    for f in &listed {
        let name = Path::new(f["file"].as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned();
        let (_, _, ok) = files.iter().find(|(n, ..)| *n == name).unwrap();
        assert_eq!(f.get("error").is_none(), *ok, "{f}");
        if !ok {
            let e = f["error"].as_str().unwrap();
            let why =
                if name == "noise.emf" { "not a supported picture (PNG, JPEG, GIF, WebP, BMP, TIFF, WMF or EMF)" } else { "the metafile is damaged" };
            assert!(e.starts_with(why), "{name}: {e}");
            assert_eq!(f["pictures"], json!([]), "{f}");
        }
    }
    let max_width = ss.doc.sections().first().map(|(_, sp)| sp.text_width()).unwrap();
    for (name, bytes, ok) in &files {
        let path = s(&dir.join(name));
        for width in [32, 160, 512] {
            let r = ss.run("library.thumbnail", &json!({"file": path, "picture": 0, "width": width}));
            match r {
                Ok(t) if *ok => {
                    let (w, h) = (t["width"].as_u64().unwrap(), t["height"].as_u64().unwrap());
                    assert!((1..=width).contains(&w) && (1..=u64::from(MAX_THUMB_HEIGHT)).contains(&h), "{name} at {width}: {t}");
                    assert_eq!(decode_png(t["png"].as_str().unwrap()).dimensions(), (w as u32, h as u32), "{name}");
                }
                r => assert!(!ok && matches!(r, Err(CmdError::Failed(_))), "{name} at {width}: {r:?}"),
            }
        }
        let before = inserted(&ss).len();
        let r = ss.run("library.insert", &json!({"file": path}));
        match r {
            Ok(r) if *ok => {
                let (w, h) = (r["width"].as_f64().unwrap(), r["height"].as_f64().unwrap());
                assert!(w > 0.0 && w <= f64::from(max_width) + 0.01 && h > 0.0 && h <= 1584.0, "{name}: {r}");
                assert_eq!(inserted(&ss).len(), before + 1, "{name}");
                assert!(inserted(&ss).contains(bytes), "{name}");
            }
            r => {
                assert!(!ok && r.is_err(), "{name}: {r:?}");
                assert_eq!(inserted(&ss).len(), before, "{name}: nothing went in");
            }
        }
    }
    // Searching and listing without reading still work beside them.
    assert_eq!(list(&mut ss, json!({"pictures": false})).len(), files.len());
    let hits = ss.run("library.search", &json!({"query": "wmf"})).unwrap();
    assert_eq!(hits.as_array().map(Vec::len), Some(2), "the two readable WMFs: {hits}");
    let _ = std::fs::remove_dir_all(dir);
}
