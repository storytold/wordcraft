use serde_json::json;

use super::*;

fn host() -> HostInfo {
    HostInfo {
        id: "wordcraft".into(),
        name: "WordCraft".into(),
        version: Version::new(0, 7, 0),
        protocol: Version::new(2, 0, 0),
        features: vec!["dialogs".into(), "events".into()],
        api_sets: vec![("WordApi".into(), Version::new(1, 4, 0)), ("Settings".into(), Version::new(1, 1, 0))],
        methods: vec!["Document.getSelectedDataAsync".into()],
    }
}

fn node(id: &str, version: &str, requires: serde_json::Value) -> Node {
    Node {
        id: id.into(),
        name: id.trim_start_matches("org.").into(),
        version: version.into(),
        enabled: true,
        failed: false,
        requires: serde_json::from_value(requires).unwrap(),
        office: None,
    }
}

fn needs(id: &str, range: &str) -> serde_json::Value {
    json!({"addins": [{"id": id, "version": range, "github": "sirmmo/craft-vba", "why": "Runs the report macros"}]})
}

fn messages(r: &Resolution, id: &str) -> Vec<String> {
    r.blocked.get(id).map(|b| b.iter().map(|x| x.message.clone()).collect()).unwrap_or_default()
}

#[test]
fn a_satisfied_requirement_starts_the_required_add_in_first() {
    let nodes = [node("org.reports", "1.4.0", needs("org.vba", "^0.1.4")), node("org.vba", "0.1.5", json!({}))];
    let r = resolve(&nodes, &host());
    assert!(r.blocked.is_empty(), "{:?}", r.blocked);
    assert_eq!(r.order, ["org.vba", "org.reports"]);
    assert_eq!(r.required_by.get("org.vba").unwrap(), &["org.reports"]);
}

#[test]
fn missing_out_of_range_off_and_failed_add_ins_block() {
    let reports = node("org.reports", "1.4.0", needs("org.vba", "^0.1.4"));
    // Missing: named after its repository.
    let r = resolve(std::slice::from_ref(&reports), &host());
    assert_eq!(messages(&r, "org.reports"), ["Needs craft-vba, which isn't installed"]);
    let b = &r.blocked["org.reports"][0];
    assert_eq!((b.kind, b.id.as_deref(), b.need.as_str(), b.have.as_deref()), (Kind::Addin, Some("org.vba"), "^0.1.4", None));
    // Out of range.
    let r = resolve(&[reports.clone(), node("org.vba", "0.1.3", json!({}))], &host());
    assert_eq!(messages(&r, "org.reports"), ["Needs vba ^0.1.4 (installed: 0.1.3)"]);
    assert_eq!(r.blocked["org.reports"][0].have.as_deref(), Some("0.1.3"));
    let r = resolve(&[reports.clone(), node("org.vba", "0.2.0", json!({}))], &host());
    assert_eq!(messages(&r, "org.reports"), ["Needs vba ^0.1.4 (installed: 0.2.0)"]);
    // Turned off.
    let mut off = node("org.vba", "0.1.4", json!({}));
    off.enabled = false;
    let r = resolve(&[reports.clone(), off], &host());
    assert_eq!(messages(&r, "org.reports"), ["Needs vba, which is turned off"]);
    assert!(!r.is_blocked("org.vba"), "turned off isn't blocked");
    // Failed.
    let mut failed = node("org.vba", "0.1.4", json!({}));
    failed.failed = true;
    let r = resolve(&[reports.clone(), failed], &host());
    assert_eq!(messages(&r, "org.reports"), ["Needs vba, which has failed"]);
    // A version that isn't one only matches `*`.
    let r = resolve(&[reports, node("org.vba", "", json!({}))], &host());
    assert_eq!(messages(&r, "org.reports"), ["Needs vba ^0.1.4 (installed: ?)"]);
    let r = resolve(&[node("org.reports", "1", json!({"addins": [{"id": "org.vba"}]})), node("org.vba", "", json!({}))], &host());
    assert!(r.blocked.is_empty());
}

#[test]
fn a_blocked_dependency_blocks_its_dependents_in_turn() {
    let nodes = [
        node("org.top", "1.0.0", json!({"addins": [{"id": "org.mid"}]})),
        node("org.mid", "1.0.0", json!({"addins": [{"id": "org.low", "version": ">=2"}]})),
        node("org.low", "1.0.0", json!({})),
    ];
    let r = resolve(&nodes, &host());
    assert_eq!(messages(&r, "org.mid"), ["Needs low >=2 (installed: 1.0.0)"]);
    assert_eq!(messages(&r, "org.top"), ["Needs mid, which is blocked (Needs low >=2 (installed: 1.0.0))"]);
    assert_eq!(r.order, ["org.low", "org.mid", "org.top"]);
    assert!(!r.is_blocked("org.low"));
}

#[test]
fn optional_add_ins_never_block() {
    let nodes = [node(
        "org.reports",
        "1.0.0",
        json!({"addins": [{"id": "org.mqtt", "version": ">=0.1, <0.3", "optional": true}, {"id": "org.other", "version": "9", "optional": true}]}),
    )];
    let mut mqtt = node("org.mqtt", "0.5.0", json!({}));
    mqtt.enabled = false;
    let r = resolve(&[nodes[0].clone(), mqtt], &host());
    assert!(r.blocked.is_empty(), "{:?}", r.blocked);
    assert!(r.required_by.is_empty(), "optional add-ins aren't required by anyone");
    let m = missing(&nodes[0], &nodes);
    assert_eq!(m.len(), 2);
    assert!(m.iter().all(|x| x.optional));
}

#[test]
fn app_protocol_and_feature_requirements() {
    let n =
        node("org.x", "1.0.0", json!({"apps": {"wordcraft": ">=0.8", "gridcraft": ">=9"}, "protocol": ">=3.0", "features": ["dialogs", "teleport"]}));
    let r = resolve(&[n], &host());
    assert_eq!(
        messages(&r, "org.x"),
        [
            "Needs WordCraft 0.8 or later (this is 0.7.0)",
            "Needs plugin protocol 3.0 or later (this WordCraft has 2.0)",
            "Needs `teleport`, which this WordCraft doesn't have"
        ]
    );
    let kinds: Vec<Kind> = r.blocked["org.x"].iter().map(|b| b.kind).collect();
    assert_eq!(kinds, [Kind::App, Kind::Protocol, Kind::Feature]);
    // Other apps' constraints don't matter here, and met ones don't block.
    let n = node("org.y", "1.0.0", json!({"apps": {"wordcraft": "^0.7", "gridcraft": ">=9"}, "protocol": ">=1.2", "features": ["events"]}));
    assert!(resolve(&[n], &host()).blocked.is_empty());
    // Cargo's caret keeps protocol 2.0 out of `^1.2`: write `>=1.2`.
    let n = node("org.z", "1.0.0", json!({"protocol": "^1.2"}));
    assert_eq!(resolve(&[n], &host()).blocked["org.z"][0].kind, Kind::Protocol);
}

#[test]
fn ranges_that_dont_parse_block_and_never_crash() {
    let n =
        node("org.x", "1.0.0", json!({"apps": {"wordcraft": "soon"}, "protocol": ">>2", "addins": [{"id": "org.y", "version": "latest please"}]}));
    let r = resolve(&[n, node("org.y", "1.0.0", json!({}))], &host());
    let b = &r.blocked["org.x"];
    assert!(b.iter().all(|x| x.kind == Kind::Range), "{b:?}");
    assert_eq!(b.len(), 3);
    assert!(b[2].message.contains("isn't a version range"), "{}", b[2].message);
}

#[test]
fn cycles_block_every_add_in_in_them() {
    let nodes = [
        node("org.a", "1.0.0", json!({"addins": [{"id": "org.b"}]})),
        node("org.b", "1.0.0", json!({"addins": [{"id": "org.c"}]})),
        node("org.c", "1.0.0", json!({"addins": [{"id": "org.a"}]})),
        node("org.d", "1.0.0", json!({"addins": [{"id": "org.a"}]})),
        node("org.e", "1.0.0", json!({})),
    ];
    let r = resolve(&nodes, &host());
    assert_eq!(messages(&r, "org.a"), ["Requirement cycle: a → b → c → a"]);
    assert_eq!(messages(&r, "org.b"), ["Requirement cycle: b → c → a → b"]);
    assert_eq!(r.blocked["org.c"][0].kind, Kind::Cycle);
    assert_eq!(r.blocked["org.c"][0].need, "org.c → org.a → org.b → org.c");
    assert!(messages(&r, "org.d")[0].starts_with("Needs a, which is blocked (Requirement cycle"));
    assert!(!r.is_blocked("org.e"));
    // Blocked add-ins never start; the order still lists everyone, the cycle last.
    assert_eq!(r.order, ["org.d", "org.e", "org.a", "org.b", "org.c"]);
}

#[test]
fn chains_deeper_than_eight_are_blocked() {
    // a0 → a1 → … → a9: a0 is 9 links deep, a1 8.
    let nodes: Vec<Node> = (0..10)
        .map(|i| node(&format!("org.a{i}"), "1.0.0", if i < 9 { json!({"addins": [{"id": format!("org.a{}", i + 1)}]}) } else { json!({}) }))
        .collect();
    let r = resolve(&nodes, &host());
    assert_eq!(r.blocked.keys().collect::<Vec<_>>(), ["org.a0"]);
    assert_eq!(r.blocked["org.a0"][0].kind, Kind::Depth);
    assert_eq!(r.order.first().map(String::as_str), Some("org.a9"));
    assert_eq!(r.order.last().map(String::as_str), Some("org.a0"));
}

#[test]
fn installing_the_missing_add_in_unblocks() {
    let reports = node("org.reports", "1.4.0", needs("org.vba", "^0.1.4"));
    assert!(resolve(std::slice::from_ref(&reports), &host()).is_blocked("org.reports"));
    assert!(!resolve(&[reports, node("org.vba", "0.1.9", json!({}))], &host()).is_blocked("org.reports"));
}

#[test]
fn installing_out_of_a_dependents_range_conflicts() {
    let nodes = [node("org.reports", "1.4.0", needs("org.vba", "^0.1.4")), node("org.vba", "0.1.5", json!({}))];
    let c = conflicts(&nodes, "org.vba", "craft-vba", "0.2.0");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].message, "reports needs craft-vba ^0.1.4; 0.2.0 doesn't match");
    assert!(conflicts(&nodes, "org.vba", "craft-vba", "0.1.7").is_empty());
    let mut off = nodes.clone();
    off[0].enabled = false;
    assert!(conflicts(&off, "org.vba", "craft-vba", "0.2.0").is_empty(), "a turned-off dependent doesn't hold an update back");
}

#[test]
fn office_requirement_sets_and_methods() {
    let o = OfficeNeeds {
        sets: vec![
            ("WordApi".into(), "1.9".into()),
            ("wordapi".into(), "1.3".into()),
            ("ExcelApi".into(), "1.1".into()),
            ("Settings".into(), "x".into()),
        ],
        methods: vec!["Document.getSelectedDataAsync".into(), "Document.goToByIdAsync".into()],
    };
    let b = office_blockers(&o, &host());
    let m: Vec<&str> = b.iter().map(|x| x.message.as_str()).collect();
    assert_eq!(
        m,
        [
            "Needs WordApi 1.9; WordCraft supports up to 1.4",
            "Needs ExcelApi 1.1, which WordCraft doesn't support",
            "Its manifest's requirement Settings x isn't a version",
            "Needs Document.goToByIdAsync, which WordCraft doesn't support",
        ]
    );
    let mut n = node("office.abc", "1.0.0.0", json!({}));
    n.office = Some(OfficeNeeds { sets: vec![("WordApi".into(), "1.4".into())], methods: vec![] });
    assert!(resolve(&[n], &host()).blocked.is_empty());
}

#[test]
fn releases_are_picked_newest_in_range() {
    let releases = json!([
        {"tag_name": "v0.2.0", "draft": false, "prerelease": false},
        {"tag_name": "v0.1.9-beta.1", "prerelease": true},
        {"tag_name": "0.1.7", "draft": true},
        {"tag_name": "v0.1.5"},
        {"tag_name": "nightly"},
        {"tag_name": "v0.1.6", "prerelease": true},
        {"tag_name": "0.1.4"},
        {"name": "no tag"},
    ]);
    let list = releases.as_array().unwrap();
    let pick = |range: &str| pick_release(list, &Range::parse(range).unwrap()).map(|(i, v)| (i, v.to_string()));
    assert_eq!(pick("^0.1.4"), Some((3, "0.1.5".into())), "not the latest (0.2.0), not drafts or pre-releases");
    assert_eq!(pick("*"), Some((0, "0.2.0".into())));
    assert_eq!(pick(">=0.1, <0.3"), Some((0, "0.2.0".into())));
    assert_eq!(pick("=0.1.4"), Some((6, "0.1.4".into())));
    assert_eq!(pick("^0.3"), None);
    assert_eq!(pick(">=0.1.9-beta.1, <0.2"), Some((1, "0.1.9-beta.1".into())), "a range naming a pre-release may pick one");
    assert_eq!(pick_release(&[], &Range::any()), None);
}

#[test]
fn manifests_check_their_requirements() {
    let base = |requires: serde_json::Value| {
        json!({"id": "org.reports", "name": "Reports", "version": "1.4.0", "run": {"command": "x"}, "requires": requires}).to_string()
    };
    let m = crate::Manifest::parse(&base(json!({
        "apps": {"wordcraft": ">=0.7.0", "gridcraft": ">=0.7.0"},
        "protocol": ">=1.2",
        "features": ["dialogs", "events"],
        "addins": [
            {"id": "org.craft.vba", "version": "^0.1.4", "github": "sirmmo/craft-vba", "why": "Runs the report macros"},
            {"id": "org.craft.mqtt", "version": ">=0.1, <0.3", "github": "sirmmo/craft-collab", "optional": true, "why": "Live editing"}
        ]
    })))
    .unwrap();
    assert_eq!(m.requires.addins.len(), 2);
    assert_eq!(m.requires.addins[0].label(), "craft-vba");
    assert!(m.requires.addins[1].optional);
    let too_many: Vec<_> = (0..33).map(|i| json!({"id": format!("org.x{i}")})).collect();
    for (bad, why) in [
        (json!({"addins": too_many}), "at most 32"),
        (json!({"addins": [{"id": "bad id"}]}), "must be"),
        (json!({"addins": [{"id": "org.reports"}]}), "requires itself"),
        (json!({"addins": [{"id": "org.a"}, {"id": "org.a"}]}), "twice"),
        (json!({"addins": [{"id": "org.a", "why": "x".repeat(201)}]}), "200"),
        (json!({"addins": [{"id": "org.a", "github": "https://gitlab.com/a/b"}]}), "owner/repo"),
        (json!({"addins": [{"id": "org.a", "github": "a/b/releases/tag/v1"}]}), "owner/repo"),
        (json!({"apps": {"not an id": "1"}}), "app id"),
        (json!({"features": [""]}), "features"),
        (json!({"addins": "nope"}), "invalid type"),
    ] {
        let e = crate::Manifest::parse(&base(bad.clone())).unwrap_err();
        assert!(e.contains(why), "{bad}: {e}");
    }
    // A bad range doesn't stop the manifest from loading: it blocks the add-in instead.
    assert!(crate::Manifest::parse(&base(json!({"addins": [{"id": "org.a", "version": "whenever"}]}))).is_ok());
    assert!(crate::Manifest::parse(&base(json!({"addins": [{"id": "org.a", "github": "sirmmo/craft-vba"}]}))).is_ok());
}
