use std::sync::Arc;

use wordcraft_doc::graphic::{Embedded, EmbeddedRef, Graphic};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, InlineObject};

/// Original synthetic model data: sample formatting/tables/footer plus picture and source parts.
fn complete_document() -> Document {
    let mut doc = wordcraft_engine::sample::sample_document();
    let picture = Arc::new(vec![0, 1, 127, 128, 254, 255]);
    doc.media.insert("synthetic.png".into(), picture);
    doc.passthrough.insert("word/embeddings/synthetic.bin".into(), Arc::new(vec![9, 0, 255, 7]));
    let source = Arc::new(Embedded {
        xml: "<synthetic r:id=\"r1\"/>".into(),
        refs: vec![EmbeddedRef {
            id: "r1".into(),
            kind: "urn:synthetic:embedding".into(),
            target: "word/embeddings/synthetic.bin".into(),
            external: false,
        }],
    });
    let Block::Para(p) = Arc::make_mut(doc.body.first_mut().unwrap()) else { panic!("sample starts with a paragraph") };
    p.insert_object(
        0,
        InlineObject::Image {
            media: "synthetic.png".into(),
            w: 12.0,
            h: 12.0,
            alt: "Original synthetic picture".into(),
            float: Default::default(),
            crop: Default::default(),
            ole: Some(source.clone()),
        },
        &CharProps::default(),
    )
    .unwrap();
    p.insert_object(
        0,
        InlineObject::Graphic {
            w: 24.0,
            h: 18.0,
            alt: "Original synthetic chart".into(),
            float: Default::default(),
            graphic: Arc::new(Graphic { source: Some(source), ..Default::default() }),
        },
        &CharProps::default(),
    )
    .unwrap();
    doc
}

#[test]
fn native_json_preserves_the_entire_document() {
    let doc = complete_document();
    let bytes = wordcraft_engine::io::save_bytes("recovery.json", &doc).unwrap();
    let restored = wordcraft_engine::io::open_bytes("recovery.json", &bytes).unwrap();
    assert_eq!(restored.media, doc.media, "picture bytes must survive");
    assert_eq!(restored.passthrough, doc.passthrough, "opaque parts must survive");
    assert_eq!(restored, doc, "object markup and relationships, stories and formatting must survive");
}

#[test]
fn existing_native_json_without_byte_stores_still_opens() {
    let mut value = serde_json::to_value(Document::from_text("Old native document")).unwrap();
    value.as_object_mut().unwrap().remove("media");
    value.as_object_mut().unwrap().remove("passthrough");
    let doc = wordcraft_engine::io::open_bytes("old.json", &serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(doc.plain_text(wordcraft_doc::StoryRef::Body), "Old native document");
}

/// Picture and part bytes are one base64 string each in native JSON (`.json` files and recovery
/// copies, #586). An array of numbers took about 3.6 times their size, and over ten times in a
/// pretty-printed `.json` file.
#[test]
fn native_json_stores_bytes_as_base64() {
    let mut doc = Document::from_text("Synthetic");
    let compact = |d: &Document| serde_json::to_vec(d).unwrap().len();
    let pretty = |d: &Document| wordcraft_engine::io::save_bytes("doc.json", d).unwrap();
    let (without, without_pretty) = (compact(&doc), pretty(&doc).len());
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    doc.media.insert("big.png".into(), Arc::new(bytes.clone()));
    doc.passthrough.insert("customXml/item1.xml".into(), Arc::new(b"<synthetic/>".to_vec()));
    let saved = pretty(&doc);
    // Base64 is 4/3 of the bytes; allow a little for names and punctuation.
    for (with, before) in [(compact(&doc), without), (saved.len(), without_pretty)] {
        assert!(with - before < bytes.len() * 14 / 10, "{} bytes for {}", with - before, bytes.len());
    }
    let value: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(value["passthrough"]["customXml/item1.xml"], "PHN5bnRoZXRpYy8+");
    let reopened = wordcraft_engine::io::open_bytes("doc.json", &saved).unwrap();
    assert_eq!((reopened.media, reopened.passthrough), (doc.media, doc.passthrough));
}

/// `.json` files and recovery copies written before kept bytes as arrays of numbers: they still
/// open, with the same bytes.
#[test]
fn native_json_with_byte_arrays_still_opens() {
    let doc = complete_document();
    let mut value = serde_json::to_value(&doc).unwrap();
    for (store, parts) in [("media", &doc.media), ("passthrough", &doc.passthrough)] {
        for (name, bytes) in parts {
            value[store][name] = serde_json::json!(bytes.as_slice());
        }
    }
    assert!(value["media"]["synthetic.png"].is_array());
    let reopened = wordcraft_engine::io::open_bytes("old.json", &serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    assert_eq!(reopened, doc);
}

/// Bytes that are neither base64 nor an array of bytes make the file fail to open, never panic.
#[test]
fn native_json_with_bad_bytes_is_an_error() {
    let saved = wordcraft_engine::io::save_bytes("doc.json", &complete_document()).unwrap();
    for bad in [serde_json::json!("not base64!"), serde_json::json!([1, 2, 300]), serde_json::json!({"a": 1}), serde_json::json!(7)] {
        let mut value: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        value["media"]["synthetic.png"] = bad.clone();
        assert!(wordcraft_engine::io::open_bytes("doc.json", &serde_json::to_vec(&value).unwrap()).is_err(), "{bad}");
    }
}

/// Measures native JSON with a large picture: the compact form a recovery copy wraps, and a
/// pretty-printed `.json` file. Run on demand:
/// `cargo test --release -p wordcraft-engine --test it recovery_format:: -- --ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn native_json_size_with_a_large_picture() {
    let mut doc = Document::from_text("Synthetic document with one large picture.");
    // Incompressible bytes, as a photograph's are.
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let noise: Vec<u8> = (0..27u32 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    doc.media.insert("media/image1.png".into(), Arc::new(noise));
    let mib = |n: usize| n as f64 / f64::from(1u32 << 20);
    let t0 = std::time::Instant::now();
    let compact = serde_json::to_vec(&doc).unwrap();
    let written = t0.elapsed();
    let t0 = std::time::Instant::now();
    let back: Document = serde_json::from_slice(&compact).unwrap();
    println!("27 MiB picture, compact (recovery copy): {:.1} MiB, written in {written:?}, read in {:?}", mib(compact.len()), t0.elapsed());
    assert_eq!(back.media, doc.media);
    let t0 = std::time::Instant::now();
    let pretty = wordcraft_engine::io::save_bytes("doc.json", &doc).unwrap();
    let written = t0.elapsed();
    let t0 = std::time::Instant::now();
    let back = wordcraft_engine::io::open_bytes("doc.json", &pretty).unwrap();
    println!("27 MiB picture, .json file (pretty): {:.1} MiB, written in {written:?}, read in {:?}", mib(pretty.len()), t0.elapsed());
    assert_eq!(back.media, doc.media);
}
