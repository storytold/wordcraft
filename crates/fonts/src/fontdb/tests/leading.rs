use super::super::*;

#[test]
fn east_asian_raw_ascent_guard() {
    // Copy only bundled public-font bytes; no installed face or fixture is changed.
    for (ascent, descent, gap, win_ascent, expected) in [
        (-100i16, -1000i16, 0i16, 800u16, (800.0, 200.0)),
        (0, -1000, 0, 800, (800.0, 200.0)),
        (900, -300, 0, 800, (1080.0, 480.0)),
        (1200, -600, 0, 800, (1200.0, 600.0)),
        (-100, -1000, 200, 800, (100.0, 1000.0)),
        (-100, -1000, 0, 0, (700.0, 100.0)),
    ] {
        let mut bytes = BUNDLED[0].to_vec();
        let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
        let offset = |tag: &[u8; 4]| {
            let entry = bytes[12..12 + count * 16].as_chunks::<16>().0.iter().find(|e| &e[..4] == tag).unwrap();
            u32::from_be_bytes(entry[8..12].try_into().unwrap()) as usize
        };
        let head = offset(b"head");
        let hhea = offset(b"hhea");
        let os2 = offset(b"OS/2");
        bytes[head + 18..head + 20].copy_from_slice(&1000u16.to_be_bytes());
        bytes[hhea + 4..hhea + 6].copy_from_slice(&ascent.to_be_bytes());
        bytes[hhea + 6..hhea + 8].copy_from_slice(&descent.to_be_bytes());
        bytes[hhea + 8..hhea + 10].copy_from_slice(&gap.to_be_bytes());
        bytes[os2 + 74..os2 + 76].copy_from_slice(&win_ascent.to_be_bytes());
        bytes[os2 + 76..os2 + 78].copy_from_slice(&200u16.to_be_bytes());
        bytes[os2 + 78..os2 + 82].copy_from_slice(&(1u32 << 17).to_be_bytes());
        let mut face =
            make_face(FontBytes::Owned(Arc::new(bytes)), 0, "Raw ascent control".into(), "Regular".into(), "Raw ascent control".into(), vec![])
                .unwrap();
        face.ascent = 700.0;
        face.descent = 100.0;
        assert!(face.is_east_asian());
        let fixed = crate::word::fixed_line_metrics(&face);
        let actual = crate::word::line_metrics(&face);
        assert_eq!(actual, expected, "raw ascent={ascent}, descent={descent}, gap={gap}, win={win_ascent}");
        assert_eq!(crate::word::line_metrics(&face), expected, "cached result");
        if ascent <= 0 || ascent == 1200 {
            assert_eq!(actual, fixed, "invalid raw metrics or expanded total use the unchanged fallback");
        }
    }
}

#[test]
fn line_metrics_follow_code_pages() {
    // Vary only metadata in a bundled OFL face; installed fonts cannot affect this proof.
    for (upem, ascent, descent, gap, east_asian) in [
        (1000u16, 1000i16, -300i16, 200i16, (1195.0, 495.0)),
        (2048, 2189, -555, 0, (2600.6, 966.6)),
        (1000, 1060, -340, 0, (1270.0, 550.0)),
        (1000, 900, -300, 0, (1080.0, 480.0)),
        (1000, 880, -120, 500, (1030.0, 270.0)),
        (2048, 1820, -692, 0, (2196.8, 1068.8)),
    ] {
        let standard = (f64::from(ascent) + f64::from(gap.max(0)), f64::from(descent).abs());
        for (pages, expected) in [(1u32, standard), (1 << 17, east_asian)] {
            let mut bytes = BUNDLED[0].to_vec();
            let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
            let offset = |tag: &[u8; 4]| {
                let entry = bytes[12..12 + count * 16].as_chunks::<16>().0.iter().find(|e| &e[..4] == tag).unwrap();
                u32::from_be_bytes(entry[8..12].try_into().unwrap()) as usize
            };
            let head = offset(b"head");
            let hhea = offset(b"hhea");
            let os2 = offset(b"OS/2");
            bytes[head + 18..head + 20].copy_from_slice(&upem.to_be_bytes());
            bytes[hhea + 4..hhea + 6].copy_from_slice(&ascent.to_be_bytes());
            bytes[hhea + 6..hhea + 8].copy_from_slice(&descent.to_be_bytes());
            bytes[hhea + 8..hhea + 10].copy_from_slice(&gap.to_be_bytes());
            bytes[os2 + 78..os2 + 82].copy_from_slice(&pages.to_be_bytes());
            let face =
                make_face(FontBytes::Owned(Arc::new(bytes)), 0, "Metric proof".into(), "Regular".into(), "Metric proof".into(), vec![]).unwrap();
            assert_eq!(face.is_east_asian(), pages != 1);
            assert!(!face.covers('中'), "code pages, not glyph coverage, select the policy");
            assert_eq!(crate::word::fixed_line_metrics(&face), standard);
            let actual = crate::word::line_metrics(&face);
            assert!((actual.0 - expected.0).abs() < 1e-9 && (actual.1 - expected.1).abs() < 1e-9, "{actual:?} != {expected:?}");
        }
    }
}
