use super::*;

fn v(s: &str) -> Version {
    Version::parse(s).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn versions_parse_with_missing_parts_four_parts_pre_releases_and_builds() {
    let table: &[(&str, &str)] = &[
        ("1", "1.0.0"),
        ("1.2", "1.2.0"),
        ("1.2.3", "1.2.3"),
        (" 1.2.3 ", "1.2.3"),
        ("1.0.0.0", "1.0.0"),
        ("16.0.1234.5678", "16.0.1234"),
        ("1.2.3-beta.1", "1.2.3-beta.1"),
        ("1.2.3+build.5", "1.2.3"),
        ("1.2.3-rc.1+abc", "1.2.3-rc.1"),
        ("0.0.0", "0.0.0"),
    ];
    for (text, want) in table {
        assert_eq!(v(text).to_string(), *want, "{text}");
    }
    for bad in [
        "",
        " ",
        "a",
        "1.",
        ".1",
        "1..2",
        "1.2.3.4.5",
        "01.2.3",
        "1.2.3-",
        "1.2.3-beta..1",
        "1.2.3+",
        "-1.0.0",
        "1.2.x",
        "v1.2.3",
        "1.2.3 4",
        "99999999999999999999.0.0",
    ] {
        assert!(Version::parse(bad).is_err(), "{bad:?} should not parse");
    }
    assert!(Version::parse(&"1".repeat(500)).is_err());
}

#[test]
fn versions_order_pre_releases_before_releases() {
    let ordered = [
        "0.9.9",
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "1.0.1",
        "1.10.0",
        "2.0.0",
    ];
    for w in ordered.windows(2) {
        assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
    }
    assert_eq!(v("1.2.3+a"), v("1.2.3+b"), "build metadata is ignored");
    assert_eq!(v("1.0.0.0"), v("1.0.0.9"), "the fourth part is ignored");
}

#[test]
fn tags_name_versions() {
    assert_eq!(Version::from_tag("v0.1.5"), Some(v("0.1.5")));
    assert_eq!(Version::from_tag("0.1.5"), Some(v("0.1.5")));
    assert_eq!(Version::from_tag("V2.0.0-rc.1"), Some(v("2.0.0-rc.1")));
    assert_eq!(Version::from_tag("release-1"), None);
    assert_eq!(Version::from_tag("latest"), None);
    assert_eq!(Version::from_tag("vnext"), None);
}

/// (range, matching versions, versions that don't match)
const TABLE: &[(&str, &[&str], &[&str])] = &[
    ("^1.2.3", &["1.2.3", "1.9.0", "1.2.4"], &["1.2.2", "2.0.0", "0.9.0", "2.0.0-alpha"]),
    ("1.2.3", &["1.2.3", "1.99.0"], &["2.0.0", "1.2.2"]),
    ("^1.2", &["1.2.0", "1.5.0"], &["1.1.9", "2.0.0"]),
    ("^1", &["1.0.0", "1.9.9"], &["0.9.0", "2.0.0"]),
    ("^0.1.4", &["0.1.4", "0.1.99"], &["0.1.3", "0.2.0", "1.0.0"]),
    ("0.1.4", &["0.1.4", "0.1.5"], &["0.2.0"]),
    ("^0.0.3", &["0.0.3"], &["0.0.4", "0.0.2", "0.1.0"]),
    ("^0.0", &["0.0.0", "0.0.9"], &["0.1.0"]),
    ("^0", &["0.0.0", "0.9.9"], &["1.0.0"]),
    ("~1.2.3", &["1.2.3", "1.2.9"], &["1.3.0", "1.2.2"]),
    ("~1.2", &["1.2.0", "1.2.9"], &["1.3.0"]),
    ("~1", &["1.0.0", "1.9.0"], &["2.0.0"]),
    ("~0.1.4", &["0.1.4", "0.1.9"], &["0.2.0"]),
    (">=0.1", &["0.1.0", "5.0.0"], &["0.0.9"]),
    (">=0.1, <0.3", &["0.1.0", "0.2.9"], &["0.3.0", "0.0.1"]),
    (">= 0.1 , < 0.3", &["0.2.0"], &["0.3.0"]),
    (">1.2.3", &["1.2.4"], &["1.2.3"]),
    (">1.2", &["1.3.0"], &["1.2.9"]),
    (">1", &["2.0.0"], &["1.9.9"]),
    ("<1.2", &["1.1.9"], &["1.2.0"]),
    ("<=1.2.3", &["1.2.3"], &["1.2.4"]),
    ("<=1.2", &["1.2.9"], &["1.3.0"]),
    ("=1.2.3", &["1.2.3"], &["1.2.4", "1.2.3-beta"]),
    ("=1.2", &["1.2.0", "1.2.7"], &["1.3.0"]),
    ("1.*", &["1.0.0", "1.9.0"], &["2.0.0", "0.9.0"]),
    ("1.2.*", &["1.2.0", "1.2.9"], &["1.3.0"]),
    ("1.x", &["1.4.0"], &["2.0.0"]),
    ("0.*", &["0.0.1", "0.9.0"], &["1.0.0"]),
    ("*", &["0.0.0", "99.0.0", "1.0.0-beta"], &[]),
    ("", &["1.0.0", "2.0.0-rc.1"], &[]),
    // Pre-releases match only comparators naming a pre-release of the same version.
    (">=1.0.0-beta.2", &["1.0.0-beta.2", "1.0.0-rc.1", "1.0.0", "1.5.0"], &["1.0.0-beta.1", "1.1.0-beta"]),
    ("^1.2.3-beta", &["1.2.3-beta", "1.2.3-beta.4", "1.2.3", "1.3.0"], &["1.3.0-alpha", "1.2.3-alpha"]),
    (">=1.0", &["1.0.0"], &["1.0.1-beta", "2.0.0-rc.1"]),
    // Four-part Office versions compare on three parts.
    (">=1.0", &["1.0.0.0", "1.0.0.7"], &["0.9.0.0"]),
];

#[test]
fn ranges_follow_cargo() {
    for (range, yes, no) in TABLE {
        let r = Range::parse(range).unwrap_or_else(|e| panic!("{range}: {e}"));
        for x in *yes {
            assert!(r.matches(&v(x)), "{range} should match {x}");
        }
        for x in *no {
            assert!(!r.matches(&v(x)), "{range} should not match {x}");
        }
    }
}

#[test]
fn garbage_ranges_are_errors_not_crashes() {
    let long = "1".repeat(129);
    let many = vec!["1"; 40].join(",");
    let bad = [
        "abc",
        ">=",
        "^",
        "~",
        ">=a",
        "1.2.3.4",
        "1,",
        ",",
        "1,,2",
        ">=1.*.3",
        "1.*.3",
        "^*",
        ">=*",
        "=>1.0",
        "1.2-beta",
        ">=1 2",
        "1 || 2",
        "1.2.3-",
        "01",
        "18446744073709551616",
        "^18446744073709551615",
        "~1.18446744073709551615",
        &long,
        &many,
        "💥",
        "\u{0}",
    ];
    for b in bad {
        let r = Range::parse(b);
        assert!(r.is_err(), "{b:?} should be refused, got {r:?}");
        assert!(r.unwrap_err().contains("isn't a version range"));
    }
    // 128 characters is still fine.
    let ok = vec![">=1.0.0"; 16].join(",");
    assert!(ok.len() <= 128 && Range::parse(&ok).is_ok(), "{ok}");
}

#[test]
fn ranges_describe_themselves() {
    assert_eq!(Range::parse(">=0.8").unwrap().describe(), "0.8 or later");
    assert_eq!(Range::parse("^0.1.4").unwrap().describe(), "^0.1.4");
    assert_eq!(Range::parse("").unwrap().as_str(), "*");
    assert!(Range::parse("").unwrap().is_any() && Range::parse("*").unwrap().is_any());
    assert!(!Range::parse("^1").unwrap().allows_prereleases());
    assert!(Range::parse(">=1.0.0-rc.1").unwrap().allows_prereleases());
}
