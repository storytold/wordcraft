//! Original picture-name collision fixtures; no reference-application files or assets.

use std::io::Write;
use wordcraft_doc::{Document, InlineObject};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;
const RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn picture(i: usize) -> Vec<u8> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[(i % 251) as u8, ((i / 251) % 251) as u8, 72], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    bytes
}

fn package(names: &[String], refs: &[usize]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut part = |path: &str, bytes: &[u8]| {
        z.start_file(path, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(bytes).unwrap();
    };
    part("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{RELS}/officeDocument" Target="word/document.xml"/></Relationships>"#).as_bytes());
    let mut rels = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (i, name) in names.iter().enumerate() {
        part(&format!("word/{name}"), &picture(i));
        rels.push_str(&format!(r#"<Relationship Id="i{i}" Type="{RELS}/image" Target="{name}"/>"#));
    }
    rels.push_str("</Relationships>");
    part("word/_rels/document.xml.rels", rels.as_bytes());
    let mut body = format!(r#"<w:document {NS}><w:body><w:p>"#);
    for i in refs {
        body.push_str(&format!(r#"<w:r><w:drawing><wp:inline><wp:extent cx="12700" cy="12700"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="i{i}"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#));
    }
    body.push_str("</w:p></w:body></w:document>");
    part("word/document.xml", body.as_bytes());
    z.finish().unwrap().into_inner()
}

fn image_refs(doc: &Document) -> Vec<&str> {
    doc.body[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .map(|o| match o {
            InlineObject::Image { media, .. } => media.as_str(),
            other => panic!("expected picture, got {other:?}"),
        })
        .collect()
}

#[test]
fn colliding_media_names_keep_first_free_spelling_and_picture_identity() {
    let names = [
        "a/image_2.png",
        "b/image.png",
        "c/image.png",
        "d/image_4.png",
        "e/image.png",
        "f/image_2.png",
        "g/image.png",
        "h/é.png",
        "i/.png",
        "j/raw",
        "k/raw",
        "l/raw_2",
        "m/raw",
        "n/image.PNG",
        "o/image.png",
    ]
    .map(str::to_string);
    let refs: Vec<_> = (0..names.len()).chain([2, 0, 5, 7]).collect();
    let keys = [
        "image_2.png",
        "image.png",
        "image_3.png",
        "image_4.png",
        "image_5.png",
        "image_2_2.png",
        "image_6.png",
        "image_7.png",
        "image_8.png",
        "raw",
        "raw_2",
        "raw_2_2",
        "raw_3",
        "image.PNG",
        "image_9.png",
    ];
    let mut doc = wordcraft_docx::read(&package(&names, &refs)).unwrap();
    assert_eq!(doc.media.len(), names.len());
    assert_eq!(image_refs(&doc), refs.iter().map(|i| keys[*i]).collect::<Vec<_>>());
    let check = |doc: &Document| {
        assert_eq!(doc.media.len(), names.len());
        let actual = image_refs(doc);
        assert_eq!(actual.len(), refs.len());
        for (reference, index) in actual.iter().zip(&refs) {
            assert_eq!(doc.media[*reference].as_slice(), picture(*index));
        }
    };
    check(&doc);
    for _ in 0..3 {
        doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
        check(&doc);
    }
}

fn cpu_cost(mut f: impl FnMut()) -> f64 {
    let ran = || std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse::<u64>().ok();
    let (wall, before) = (std::time::Instant::now(), ran());
    f();
    match (before, ran()) {
        (Some(a), Some(b)) if b > a => (b - a) as f64 * 1e-9,
        _ => wall.elapsed().as_secs_f64(),
    }
}

#[test]
fn many_colliding_media_names_do_not_restart_the_suffix_search() {
    let input = |n| {
        let names: Vec<_> = (0..n).map(|i| format!("media/{i}/original.png")).collect();
        package(&names, &(0..n).collect::<Vec<_>>())
    };
    let (small, big) = (input(512), input(4096));
    let read = |bytes: &[u8]| drop(std::hint::black_box(wordcraft_docx::read(bytes).unwrap()));
    read(&small);
    let mut ratios = Vec::new();
    // Thread CPU time excludes scheduler pauses on Linux. Elsewhere use elapsed time;
    // alternating inputs and retrying ratios well beyond linear reduce timing noise.
    for _ in 0..3 {
        let (mut s, mut b) = (f64::INFINITY, f64::INFINITY);
        for _ in 0..2 {
            s = s.min(cpu_cost(|| (0..4).for_each(|_| read(&small))) / 4.0);
            b = b.min(cpu_cost(|| read(&big)));
        }
        let ratio = b / s.max(1e-9);
        eprintln!("media name collisions: {s:.4}s -> {b:.4}s, ratio {ratio:.2}");
        if ratio < 24.0 {
            break;
        }
        ratios.push(ratio);
    }
    assert!(ratios.len() < 3, "8x colliding image names took {ratios:?}x CPU: repeated suffix search");
}

#[test]
fn writing_media_names_keeps_case_insensitive_first_free_spelling() {
    let keys = ["...", "A.png", "A?.png", "A_2.png", "a.png", "a_4.png", "raw", "raw?", "é"];
    let names: Vec<_> = (0..keys.len()).map(|i| format!("media/p{i}.png")).collect();
    let refs: Vec<_> = (0..keys.len()).chain([2, 0, 8]).collect();
    let mut doc = wordcraft_docx::read(&package(&names, &refs)).unwrap();
    let old = std::mem::take(&mut doc.media);
    for (i, key) in keys.iter().enumerate() {
        doc.media.insert((*key).into(), old[&format!("p{i}.png")].clone());
    }
    let para = doc.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    for (object, index) in para.objects.iter_mut().zip(&refs) {
        if let InlineObject::Image { media, .. } = object {
            *media = keys[*index].into();
        }
    }
    let expected = ["image.png", "A.png", "A_2.png", "A_2_2.png", "a_3.png", "a_4.png", "raw.png", "raw_2.png", "image_2.png"];
    for _ in 0..3 {
        doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
        assert_eq!(doc.media.len(), keys.len());
        let actual = image_refs(&doc);
        assert_eq!(actual, refs.iter().map(|i| expected[*i]).collect::<Vec<_>>());
        for (key, index) in actual.iter().zip(&refs) {
            assert_eq!(doc.media[*key].as_slice(), picture(*index));
        }
    }
}

#[test]
fn writing_media_names_does_not_restart_the_suffix_search() {
    let input = |n| {
        let names: Vec<_> = (0..n).map(|i| format!("media/{}{i}.png", "L".repeat(80))).collect();
        wordcraft_docx::read(&package(&names, &(0..n).collect::<Vec<_>>())).unwrap()
    };
    let (small, big) = (input(512), input(4096));
    let write = |doc: &Document| drop(std::hint::black_box(wordcraft_docx::write(doc).unwrap()));
    write(&small);
    let mut ratios = Vec::new();
    for _ in 0..3 {
        let (mut s, mut b) = (f64::INFINITY, f64::INFINITY);
        for _ in 0..2 {
            s = s.min(cpu_cost(|| (0..4).for_each(|_| write(&small))) / 4.0);
            b = b.min(cpu_cost(|| write(&big)));
        }
        let ratio = b / s.max(1e-9);
        eprintln!("writing media name collisions: {s:.4}s -> {b:.4}s, ratio {ratio:.2}");
        if ratio < 24.0 {
            break;
        }
        ratios.push(ratio);
    }
    assert!(ratios.len() < 3, "8x colliding image names took {ratios:?}x CPU to write: repeated suffix search");
}
