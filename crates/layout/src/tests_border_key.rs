use super::*;

const GEOMETRY: (u32, u32, u32) = (0, 100.0f32.to_bits(), 20.0f32.to_bits());

#[test]
fn border_fields_stay_distinct() {
    let base = Border::single(1.0);
    let variants = [
        Border { style: BorderStyle::Dotted, ..base },
        Border { width: 2.0, ..base },
        Border { color: Some(Rgb::BLACK), ..base },
        Border { space: 1.0, ..base },
    ];
    let mut edges = HashSet::new();
    assert!(keep_rule_edge(&mut edges, GEOMETRY, base));
    assert!(!keep_rule_edge(&mut edges, GEOMETRY, base));
    for border in variants {
        assert_ne!(border, base);
        assert!(keep_rule_edge(&mut edges, GEOMETRY, border));
        assert!(!keep_rule_edge(&mut edges, GEOMETRY, border));
    }
    assert_eq!(edges.len(), variants.len() + 1);
}

#[test]
fn border_zero_matches_eq() {
    let base = Border::single(0.0);
    let mut edges = HashSet::new();
    assert!(keep_rule_edge(&mut edges, GEOMETRY, base));
    for width in [0.0, -0.0] {
        for space in [0.0, -0.0] {
            let border = Border { width, space, ..base };
            assert_eq!(border, base);
            assert_eq!(rule_edge_key(GEOMETRY, border), rule_edge_key(GEOMETRY, base));
            assert!(!keep_rule_edge(&mut edges, GEOMETRY, border));
        }
    }
    assert_eq!(edges.len(), 1);
}

#[test]
fn border_nan_is_never_equal() {
    let base = Border::single(1.0);
    let mut edges = HashSet::new();
    for bits in [0x7fc0_0000, 0x7fc0_0001, 0xffc0_0001] {
        let nan = f32::from_bits(bits);
        for border in [Border { width: nan, ..base }, Border { space: nan, ..base }, Border { width: nan, space: nan, ..base }] {
            let copied = border;
            assert_ne!(border, copied);
            assert!(rule_edge_key(GEOMETRY, border).is_none());
            assert!(keep_rule_edge(&mut edges, GEOMETRY, border));
            assert!(keep_rule_edge(&mut edges, GEOMETRY, border));
        }
    }
    assert!(edges.is_empty());
}

#[test]
fn border_infinity_matches_eq() {
    let base = Border::single(1.0);
    let mut edges = HashSet::new();
    for width in [f32::INFINITY, f32::NEG_INFINITY] {
        for space in [f32::INFINITY, f32::NEG_INFINITY] {
            let border = Border { width, space, ..base };
            let copied = border;
            assert_eq!(border, copied);
            assert!(keep_rule_edge(&mut edges, GEOMETRY, border));
            assert!(!keep_rule_edge(&mut edges, GEOMETRY, border));
        }
    }
    assert_eq!(edges.len(), 4);
}

#[test]
fn edge_bits_remain_distinct() {
    let border = Border::single(1.0);
    let negative_zero = (-0.0f32).to_bits();
    // Admission remains at the callers. The key itself keeps all three raw geometry fields.
    let geometries = [(0, 0, 0), (negative_zero, 0, 0), (0, negative_zero, 0), (0, 0, negative_zero), (1, 0, 0), (0, 1, 0), (0, 0, 1)];
    let mut edges = HashSet::new();
    for geometry in geometries {
        assert!(keep_rule_edge(&mut edges, geometry, border));
        assert!(!keep_rule_edge(&mut edges, geometry, border));
    }
    assert_eq!(edges.len(), geometries.len());
}

#[test]
fn nan_seed_never_matches() {
    let base = Border::single(1.0);
    let nan_border = Border { space: f32::from_bits(0x7fc0_0001), ..base };
    let mut edges: HashSet<_> = [base, nan_border].into_iter().filter_map(|border| rule_edge_key(GEOMETRY, border)).collect();
    assert_eq!(edges.len(), 1);
    assert!(keep_rule_edge(&mut edges, GEOMETRY, nan_border));
    assert!(!keep_rule_edge(&mut edges, GEOMETRY, base));
    assert_eq!(edges.len(), 1);
}
