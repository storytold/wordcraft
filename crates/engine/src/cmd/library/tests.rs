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
    assert_eq!(ss.library.sizes().3, 0, "no picture was opened");
    let path = s(&dir.join("huge.png"));
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
    for bad in [json!({}), json!({"query": "  "}), json!({"query": "x".repeat(300)}), json!({"query": "a", "limit": 0})] {
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
