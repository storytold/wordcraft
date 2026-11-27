use super::arrange::group_menu_command;
use super::language::{PROOFING_LANGUAGES, caret_language, language_checks, language_name};
use super::tabs::contextual_tabs;
use crate::WordApp;
use egui::vec2;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::{Value, json};
use wordcraft_engine::Session;

type Harness = egui_kittest::Harness<'static, WordApp>;

/// An English app window of `size`, settled and ready for clicks through the real frame.
fn app_harness(mut app: WordApp, size: egui::Vec2) -> Harness {
    app.ui.language = "en".into();
    let mut h = crate::test_harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    // The language menu names languages in their own script; the CI runner has no CJK font.
    crate::test_harness::cjk_stand_in(&h.ctx);
    crate::test_harness::frames(&mut h, 4);
    h
}

/// A document window on `tab`, wide enough that no tab hides groups behind its scroll arrows.
fn tab_harness(app: WordApp, tab: &str) -> Harness {
    let mut app = app;
    app.run("ui.tab", json!({"tab": tab})).unwrap();
    app_harness(app, vec2(2560.0, 900.0))
}

/// Click the ribbon button named `name` (role Button, exact name first).
fn click_button(h: &mut Harness, name: &str) {
    let node = h
        .query_all(By::new().role(egui::accesskit::Role::Button).label(name))
        .next()
        .or_else(|| h.query_all(By::new().role(egui::accesskit::Role::Button).label_contains(name)).next())
        .unwrap_or_else(|| panic!("a button named {name}"));
    node.click();
    crate::test_harness::frames(h, 3);
}

/// The menu rows (plain, radio or check) whose label contains `label`: the same words also name
/// controls outside the menu, e.g. the status bar's language button.
fn menu_row(label: &str) -> By<'_> {
    use egui::accesskit::Role;
    By::new().predicate(move |n| {
        matches!(n.role(), Role::MenuItem | Role::MenuItemRadio | Role::MenuItemCheckBox) && n.label().is_some_and(|l| l.contains(label))
    })
}

/// Click the menu row or gallery tile named `name`: framework menu rows are not buttons, so no role filter.
fn click_row(h: &mut Harness, name: &str) {
    let node = h
        .query_all(menu_row(name))
        .next()
        .or_else(|| h.query_all(By::new().label(name)).next())
        .or_else(|| h.query_all(By::new().label_contains(name)).next())
        .unwrap_or_else(|| panic!("a row named {name}"));
    node.click();
    crate::test_harness::frames(h, 3);
}

/// The names of the checked rows and buttons now shown.
fn checked(h: &Harness) -> Vec<String> {
    h.query_all(By::new().predicate(|n| n.toggled() == Some(egui::accesskit::Toggled::True))).filter_map(|n| n.accesskit_node().label()).collect()
}

/// The size of the selected object, in points.
fn object_size(a: &WordApp) -> Option<(f32, f32)> {
    wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h)))
}

/// #2601: Dropped / In margin send just the position and `keep`, so every selected paragraph keeps
/// its own lines, distance and font; None sends only the position.
#[test]
fn drop_cap_menu_keeps_the_options() {
    use super::insert::drop_cap_payload;
    assert_eq!(drop_cap_payload("dropped"), json!({"position": "dropped", "keep": true}));
    assert_eq!(drop_cap_payload("margin"), json!({"position": "margin", "keep": true}));
    assert_eq!(drop_cap_payload("none"), json!({"position": "none"}), "None takes it all away");
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Once")), crate::Services::default());
    app.run("insert.dropCap", json!({"position": "dropped", "lines": 5, "distance": 7.0, "font": "Liberation Mono"})).unwrap();
    app.run("insert.dropCap", drop_cap_payload("margin")).unwrap();
    let state = wordcraft_engine::cmd::insert::drop_cap_state(&app.session);
    assert_eq!((state.position, state.lines, state.distance, state.font.as_deref()), ("margin", 5, 7.0, Some("Liberation Mono")));
}

/// Two clicks arm and disarm the painter; the double-click handling then leaves it sticky (#2785).
#[test]
fn double_click_on_format_painter_makes_it_sticky() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    app.run("edit.formatPainter", json!({})).unwrap();
    assert!(matches!(app.session.painter, Some((_, _, false))), "one click arms it for one use");
    app.run("edit.formatPainter", json!({})).unwrap();
    assert!(app.session.painter.is_none(), "the second click of a double-click turns it off");
    super::home::painter_double_click(&mut app);
    assert!(matches!(app.session.painter, Some((_, _, true))), "the double-click leaves it sticky");
}

/// A 1440 pt window shows the Home tab's Paragraph group whole, not collapsed into one button: its font box and
/// size box are a ribbon's short fields and its buttons sit flat and close.
#[test]
fn home_shows_the_paragraph_group_whole_in_a_1440_point_window() {
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    let h = app_harness(app, vec2(1440.0, 900.0));
    let shown = |name: &str| h.query_all(By::new().label(name)).next().is_some();
    assert!(shown("Bullets"), "the Paragraph group shows its Bullets button");
    assert!(shown("Borders"), "and its Borders button");
}

fn png_data() -> String {
    let img = image::RgbaImage::from_fn(20, 10, |x, y| {
        if x > 2 && x < 17 && y > 2 && y < 7 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
    });
    let b = wordcraft_geom::picture::encode_format(&image::DynamicImage::ImageRgba8(img).clone(), image::ImageFormat::Png).unwrap();
    wordcraft_engine::cmd::insert::base64_encode(&b)
}

/// #3917 review: Shape Outline › Weight on a drawing canvas reads the canvas's border, so its
/// weight is checked and a new weight keeps the border's colour instead of turning it black.
#[test]
fn shape_outline_weight_keeps_a_canvas_borders_colour() {
    use super::shape::selected_outline;
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("insert.canvas", json!({})).unwrap();
    a.run("shape.outline", json!({"color": "1F4E79", "width": 1.5})).unwrap();
    let blue = wordcraft_doc::Rgb::parse("1F4E79");
    assert_eq!(selected_outline(&a.session), blue.map(|c| (c, 1.5)), "the canvas's border");
    let mut h = tab_harness(a, "Shape Format");
    click_button(&mut h, "Shape Outline");
    click_row(&mut h, "Weight");
    assert!(checked(&h).iter().any(|l| l.contains("1.5 pt")), "the border's weight is checked: {:?}", checked(&h));
    click_row(&mut h, "3 pt");
    assert_eq!(selected_outline(&h.state().session), blue.map(|c| (c, 3.0)), "a new weight keeps the border blue");
}

/// #4229: with a group selected the Group button stays on, so Ungroup in its menu can be
/// reached; with nothing selected it is off, and with two objects it offers Group.
#[test]
fn the_group_menu_stays_on_for_a_selected_group() {
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::from_text("one\ntwo\nthree")), crate::Services::default());
    assert!(a.session.why_disabled(group_menu_command(&a)).is_some(), "nothing selected");
    a.run("insert.shape", json!({"kind": "rectangle", "width": 80, "height": 40})).unwrap();
    a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 150})).unwrap();
    a.run("select.collapse", json!({"end": true})).unwrap();
    a.run("insert.shape", json!({"kind": "ellipse", "width": 80, "height": 40})).unwrap();
    a.run("arrange.bounds", json!({"page": 0, "x": 300, "y": 150})).unwrap();
    a.run("select.addObject", json!({"index": 0})).unwrap();
    assert_eq!(group_menu_command(&a), "arrange.group");
    assert!(a.session.why_disabled(group_menu_command(&a)).is_none(), "two objects to group");
    // Layout › Arrange holds the Group button; Shape Format's Arrange group has none.
    let mut h = tab_harness(a, "Layout");
    let group_button_off = |h: &Harness| h.get_by_role_and_label(egui::accesskit::Role::Button, "Group").accesskit_node().is_disabled();
    assert!(!group_button_off(&h), "two objects: the Group button is on");
    h.state_mut().run("arrange.group", json!({})).unwrap();
    crate::test_harness::frames(&mut h, 3);
    let a = h.state();
    assert!(a.session.why_disabled("arrange.group").is_some(), "one group: nothing to group");
    assert_eq!(group_menu_command(a), "arrange.ungroup");
    assert!(a.session.why_disabled(group_menu_command(a)).is_none(), "the menu reaches Ungroup");
    assert!(!group_button_off(&h), "a selected group: the Group button stays on to reach Ungroup");
}

/// #1206: Margins, Orientation, Size and Columns mark the caret section's setting, as Word
/// does. Every value differs from a new document's, so a default can't satisfy it.
#[test]
fn page_setup_menus_mark_the_current_setting() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    app.run("layout.margins", json!({"preset": "narrow"})).unwrap();
    app.run("layout.orientation", json!({"value": "landscape"})).unwrap();
    app.run("layout.size", json!({"name": "A4"})).unwrap();
    app.run("layout.columns", json!({"preset": "right"})).unwrap();
    let mut h = tab_harness(app, "Layout");
    for (button, want) in [("Margins", "Narrow"), ("Orientation", "Landscape"), ("Size", "A4"), ("Columns", "Right")] {
        click_button(&mut h, button);
        let rows = checked(&h);
        let first_words: Vec<&str> = rows.iter().filter_map(|l| l.trim_start_matches('✓').split_whitespace().next()).collect();
        assert_eq!(first_words.iter().filter(|w| **w == want).count(), 1, "{button}: {want} is the checked row: {rows:?}");
        h.key_press(egui::Key::Escape);
        crate::test_harness::frames(&mut h, 3);
    }
}

/// The Language menu's built-in short list, as `language_checks` takes it.
fn short() -> Vec<(String, String)> {
    PROOFING_LANGUAGES.iter().map(|(t, n)| ((*t).to_string(), (*n).to_string())).collect()
}

/// Check marks with only `tag` checked.
fn listed_only(tag: &str) -> Vec<bool> {
    PROOFING_LANGUAGES.iter().map(|(t, _)| *t == tag).collect()
}

/// CP4.3: the menu's check marks are worked out one way (the tag, or the same proofing language
/// named alike), not by a case per language: a bare or ISO 639-2 tag checks its language, and a
/// region WordCraft proofs like another (New Zealand like the UK) doesn't check that one.
#[test]
fn language_menu_checks_by_tag_or_proofing_language() {
    for (current, tag) in [
        ("pl", "pl-PL"),
        ("pt", "pt-BR"),
        ("it", "it-IT"),
        ("hr", "hr-HR"),
        ("tr", "tr-TR"),
        ("ces", "cs-CZ"),
        ("in", "id-ID"),
        ("es", "es-ES"),
        ("en", "en-US"),
        ("RU-ru", "ru-RU"),
    ] {
        let c = language_checks(current, &short());
        assert_eq!((c.extra, c.listed), (None, listed_only(tag)), "{current}");
    }
    let c = language_checks("en-NZ", &short());
    assert_eq!(c.extra.as_deref(), Some("English (New Zealand)"));
    assert!(!c.listed.contains(&true), "English (New Zealand) doesn't check English (United Kingdom)");
    // A pack language in the list (once installed) is checked for its bare tag too.
    let mut entries = short();
    entries.push(("bg-BG".into(), "Bulgarian".into()));
    let c = language_checks("bg", &entries);
    assert_eq!((c.extra, c.listed.last().copied()), (None, Some(true)));
    // CP4.3 review: a bare pack-language tag checks the regioned row of the same pack, however
    // the packs are known (`nl` and `nl-NL` are both the `nl` pack's), and shows no second Dutch.
    entries.push(("nl-NL".into(), "Dutch".into()));
    for current in ["nl", "NL-nl"] {
        let c = language_checks(current, &entries);
        assert_eq!((c.extra, c.listed.last().copied()), (None, Some(true)), "{current}");
    }
    let c = language_checks("nl-BE", &entries);
    assert_eq!((c.extra.as_deref(), c.listed.last().copied()), (Some("Dutch (Belgium)"), Some(false)));
    assert!(super::language::same_language("sv", "sv-SE") && !super::language::same_language("sv-FI", "sv-SE"));
    // Nothing set (mixed selection): nothing checked, nothing above.
    let c = language_checks("", &entries);
    assert_eq!((c.extra, c.listed.contains(&true)), (None, false));
}

/// CP4.3 review: rows are matched by tag, never by the name shown. Serbian in Latin and in
/// Cyrillic letters of Bosnia show under the same name but are two languages, so neither checks
/// the other's row; a name that is translated or different doesn't matter to the match.
#[test]
fn rows_are_matched_by_tag_not_by_display_name() {
    use super::language::same_language;
    assert_eq!(language_name(Some("sr-Latn-BA")), language_name(Some("sr-Cyrl-BA")), "same display name");
    assert!(!same_language("sr-Latn-BA", "sr-Cyrl-BA"));
    let entries = vec![("sr-Latn-BA".to_string(), "Serbian".to_string()), ("sr-Cyrl-BA".to_string(), "Serbian".to_string())];
    assert_eq!(language_checks("sr-Cyrl-BA", &entries).listed, [false, true]);
    // A variant on an otherwise equal tag is the row without it, unless a pack or the lists name
    // the variant (Valencian is not Catalan); a language WordCraft has nothing for keeps its regions apart.
    assert!(same_language("de-DE-1996", "de-DE") && same_language("de-DE", "de-DE-1996"));
    assert!(!same_language("ca-ES-valencia", "ca-ES"));
    assert!(!same_language("xx", "xx-AA"), "xx vs xx-AA");
    assert!(same_language("xx-AA", "XX_aa"), "xx-AA vs XX_aa");
    assert_eq!(language_checks("de-DE-1996", &[("de-DE".to_string(), "German (Germany)".to_string())]).listed, [true]);
    // Two unknown languages never merge, though neither has a name but its tag.
    assert!(!same_language("xx", "yy") && !same_language("xx-AA", "yy-AA"));
    // The row's name can be anything (translated): the tag decides.
    let renamed = vec![("pl-PL".to_string(), "Polski".to_string())];
    assert_eq!(language_checks("pl", &renamed).listed, [true]);
    assert_eq!(language_checks("PL_pl", &renamed).listed, [true]);
    assert_eq!(language_checks("pt", &[("pt-PT".to_string(), "Portuguese (Brazil)".to_string())]).listed, [false]);
}

/// CP4.3 review: a name built from a language and a region (`Swedish (Finland)`) has no catalog row
/// of its own, so the language, the region and the `{language} ({region})` template are looked up
/// separately and joined. English stays as `language_name` builds it.
#[test]
fn a_language_and_region_name_is_translated_in_parts() {
    use super::language::display_name_with;
    let fake = |s: &str| -> String {
        match s {
            "Swedish" => "X".into(),
            "Finland" => "Y".into(),
            "{language} ({region})" => "{language} \u{2013} {region}".into(),
            other => other.to_string(),
        }
    };
    assert_eq!(display_name_with(Some("sv-FI"), &fake), "X \u{2013} Y");
    // A whole name is looked up as it is; the template isn't used for it.
    assert_eq!(display_name_with(Some("sv-SE"), &fake), "X");
    // Without a catalogue (English) the name is byte-identical to the English name.
    for tag in ["sv-FI", "es-CU", "fr-CA", "de-CH", "pt-AO", "en-SG", "sr-Latn-BA", "pl-PL", "nl", "xx-YY", "xx", "ca-valencia"] {
        assert_eq!(super::language::display_name(Some(tag)), language_name(Some(tag)), "{tag}");
    }
    assert_eq!(language_name(Some("sv-FI")), "Swedish (Finland)");
    // Matching never uses a translated name: Finnish Swedish and Swedish of Sweden stay two rows.
    assert!(!super::language::same_language("sv-FI", "sv-SE"));
}

/// #860 review: a tag the language list doesn't have shows by language and region, not as
/// the tag; Portuguese (Angola) isn't Brazil's, and no name shows twice in the menu.
#[test]
fn other_regions_show_by_name_once() {
    assert_eq!(language_name(Some("fr-CA")), "French (Canada)");
    assert_eq!(language_name(Some("en-AU")), "English (Australia)");
    assert_eq!(language_name(Some("de_AT")), "German (Austria)");
    assert_eq!(language_name(Some("pt-AO")), "Portuguese (Angola)");
    assert_eq!(language_name(Some("sr-Latn-BA")), "Serbian (Bosnia and Herzegovina)");
    assert_eq!(language_name(Some("en-ZZ")), "English (ZZ)");
    assert_eq!(language_name(Some("xx-YY")), "xx-YY");
    for tag in ["fr-CA", "pt-AO", "en-SG"] {
        let c = language_checks(tag, &short());
        let name = c.extra.expect("shown above the list");
        assert!(!PROOFING_LANGUAGES.iter().any(|(_, n)| *n == name), "{name} is listed once");
        assert!(!c.listed.contains(&true), "{tag}: nothing in the list checked");
    }
    // A tag named like a list entry checks that entry instead of showing it again.
    let c = language_checks("PT-br-x-test", &short());
    assert_eq!(c.extra, None);
    assert_eq!(c.listed, listed_only("pt-BR"));
}

/// Aliases (ISO 639-2 codes, bare languages, other case) name and check the same row as the
/// listed tag, and checking marks only that row; a region the list lacks shows above it.
#[test]
fn language_names_resolve_aliases_and_tags() {
    for (canonical, aliases) in [
        ("id-ID", &["id", "ID-id", "ind", "in"][..]),
        ("cs-CZ", &["cs", "ces", "CZE"][..]),
        ("pt-BR", &["pt"][..]),
        ("tr-TR", &["tr"][..]),
        ("es-ES", &["es", "ES-es"][..]),
    ] {
        assert!(PROOFING_LANGUAGES.iter().any(|(t, _)| *t == canonical), "{canonical} is in the list");
        for alias in aliases {
            assert_eq!(language_name(Some(alias)), language_name(Some(canonical)), "{alias} names {canonical}");
            let c = language_checks(alias, &short());
            assert_eq!((c.extra, c.listed), (None, listed_only(canonical)), "{alias} checks only {canonical}");
        }
    }
    // Every listed tag, in lower case, checks its own row and no other.
    for (tag, _) in PROOFING_LANGUAGES {
        let c = language_checks(&tag.to_ascii_lowercase(), &short());
        assert_eq!((c.extra, c.listed), (None, listed_only(tag)), "{tag}");
    }
    // A Spanish of another country is shown above the list and checks no row.
    let c = language_checks("es-AR", &short());
    assert!(c.extra.is_some() && !c.listed.contains(&true));
}

/// #1110: the strip's drawn buttons have names for screen readers: Share, and the "»" that
/// holds the tabs that don't fit ("More tabs", not a glyph).
#[test]
fn the_tab_strip_buttons_have_names() {
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    let mut h = app_harness(app, vec2(760.0, 700.0));
    h.get_by_role_and_label(egui::accesskit::Role::Button, tl!("Share"));
    let more = h.get_by_role_and_label(egui::accesskit::Role::Button, tl!("More tabs"));
    more.click();
    crate::test_harness::frames(&mut h, 2);
    // The menu lists a tab that didn't fit (View is among the last on a narrow strip).
    assert!(h.query(By::new().role(egui::accesskit::Role::MenuItem).label(tl!("View"))).is_some(), "the » menu lists the tabs left out");
}

/// Each contextual tab shows on the tab strip while its object is selected and leaves with the
/// selection.
#[test]
fn contextual_tabs_follow_the_selection() {
    let cases: [(&str, Vec<(&str, Value)>); 4] = [
        ("Picture Format", vec![("insert.picture", json!({"data": png_data()}))]),
        ("Shape Format", vec![("insert.shape", json!({"kind": "rectangle"}))]),
        ("SmartArt Design", vec![("insert.smartArt", json!({"layout": "cycle"}))]),
        ("Chart Design", vec![("insert.chart", json!({"type": "doughnut"}))]),
    ];
    for (tab, setup) in cases {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        assert!(!contextual_tabs(&app.session).contains(&tab), "{tab}: none before the selection");
        for (command, params) in setup {
            app.run(command, params).unwrap();
        }
        assert!(contextual_tabs(&app.session).contains(&tab), "{tab}: offered with its object");
        app.run("ui.tab", json!({"tab": tab})).unwrap();
        let mut h = app_harness(app, vec2(1440.0, 900.0));
        let name = crate::i18n::tr(crate::i18n::Lang::EN, tab);
        let on_strip = |h: &Harness| h.query(By::new().role(egui::accesskit::Role::Tab).label(name)).is_some();
        assert!(on_strip(&h), "{tab} is on the strip");
        h.state_mut().run("select.collapse", json!({"end": true})).unwrap();
        h.state_mut().run("text.insert", json!({"text": "x"})).unwrap();
        // The tab leaves through its presence; step until it has gone, within a bound.
        for _ in 0..300 {
            if !on_strip(&h) {
                break;
            }
            h.step();
        }
        assert!(!contextual_tabs(&h.state().session).contains(&tab), "{tab}: gone with the selection");
        assert!(!on_strip(&h), "{tab} left the strip");
    }
}

/// No group's content runs into its label: View › Show stacked four check boxes in a column
/// made for three, so "Hide White Space" was drawn under "Show". Every tab, every language: each
/// control lies inside its group's access node.
#[test]
fn ribbon_groups_fit_above_their_labels() {
    fn rects(node: &egui_kittest::Node<'_>, out: &mut Vec<egui::Rect>) {
        for child in node.children() {
            // The framework's parts (a split button's arrow, a field's adornments) can be nodes with no
            // bounds of their own; they have no place to lie outside of, so only their children count.
            if child.accesskit_node().raw_bounds().is_some() {
                out.push(child.rect());
            }
            rects(&child, out);
        }
    }
    let app = WordApp::new(Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
    let mut h = crate::test_harness::builder().with_size(vec2(2560.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    // The ribbon names languages in their own script; a CI runner has no CJK font.
    crate::test_harness::cjk_stand_in(&h.ctx);
    let mut outside = Vec::new();
    // Menu and popup bodies the framework measures in a hidden layer below the window carry access nodes under the
    // app's groups; they are never drawn, so they cannot run into a label. Only what lies on screen is checked.
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(2560.0, 900.0));
    for lang in crate::i18n::Lang::all() {
        h.state_mut().run("ui.language", json!({"value": lang.code()})).unwrap();
        for tab in super::TABS.iter().filter(|t| **t != "File") {
            h.state_mut().run("ui.tab", json!({"tab": tab})).unwrap();
            crate::test_harness::frames(&mut h, 3);
            // The ribbon's groups: every labelled container is a group node (the document, its page text, the task
            // pane area), and those below the ribbon are not what this checks.
            let ribbon_bottom = h.state().canvas.canvas_rect().map_or(f32::INFINITY, |c| c.top());
            let groups = h
                .query_all(By::new().role(egui::accesskit::Role::Group))
                .filter(|g| g.accesskit_node().raw_bounds().is_some() && g.rect().bottom() <= ribbon_bottom + 1.0);
            for group in groups {
                let frame = group.rect();
                let mut inner = Vec::new();
                rects(&group, &mut inner);
                for r in inner.into_iter().filter(|r| r.width() > 0.0 && r.height() > 0.0 && screen.contains_rect(*r)) {
                    if !frame.contains_rect(r) {
                        outside.push(format!("{} {tab} › {:?}: {r:?} outside {frame:?}", lang.code(), group.accesskit_node().label()));
                    }
                }
            }
        }
    }
    outside.sort();
    outside.dedup();
    assert!(outside.is_empty(), "controls outside their group: {outside:#?}");
}

/// #860: Review › Language (and the status bar) reach every proofing language, not just the
/// short list: Set Proofing Language… opens the Language dialog (French, Set As Default),
/// Language Preferences… opens Options › Language, and French text shows by name, checked.
#[test]
fn language_menu_reaches_every_language() {
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Bonjour")), crate::Services::default());
    let mut h = tab_harness(app, "Review");
    click_button(&mut h, "Language");
    fn item(label: &str) -> By<'_> {
        menu_row(label)
    }
    let french = language_name(Some("fr-FR"));
    assert!(h.query(item(french.as_str())).is_none(), "the short list only, on an English document");
    assert!(!h.get(item("Set Proofing Language…")).accesskit_node().is_disabled(), "never greyed out on a fresh document");
    assert!(!h.get(item("English (United States)")).accesskit_node().is_disabled());
    click_row(&mut h, "Set Proofing Language…");
    let open = h.state().dialog_host.open_dialogs();
    assert_eq!(open.first().and_then(|d| d.key.as_deref()), Some("language"), "the Language dialog opened");
    let ctx = h.ctx.clone();
    for d in open {
        h.state_mut().dialog_host.cancel_by_id(&ctx, d.id);
    }
    crate::test_harness::frames(&mut h, 3);
    // French for the whole document: the menu and the status bar name it.
    h.state_mut().run("review.language", json!({"lang": "fr-FR", "default": true})).unwrap();
    crate::test_harness::frames(&mut h, 3);
    assert_eq!(caret_language(h.state()), french);
    click_button(&mut h, "Language");
    assert!(h.query(item(french.as_str())).is_some(), "the current language is listed (only while it is the current one)");
    click_row(&mut h, "Language Preferences…");
    assert!(h.state().ui.backstage, "File › Options opened");
    assert_eq!(h.state().ui.options_pane, "language");
}

/// #1281: every short-list entry (the four Englishes, Indonesian…) is picked in the menu:
/// clicking it sets the selection's proofing language, and the entry is then the checked one.
#[test]
fn every_proofing_language_is_picked_from_the_language_menu() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Colour")), crate::Services::default());
    app.run("select.all", json!({})).unwrap();
    let mut h = tab_harness(app, "Review");
    for (tag, name) in PROOFING_LANGUAGES {
        click_button(&mut h, "Language");
        click_row(&mut h, name);
        let st = h.state_mut().session.run("format.state", &json!({})).unwrap();
        assert_eq!(st.get("lang").and_then(Value::as_str), Some(tag), "{name}");
        click_button(&mut h, "Language");
        for (_, other) in PROOFING_LANGUAGES {
            let ticked = h.get(menu_row(other)).accesskit_node().toggled();
            assert_eq!(ticked == Some(egui::accesskit::Toggled::True), other == name, "{name}: {other} ticked {ticked:?}");
        }
        h.key_press(egui::Key::Escape);
        crate::test_harness::frames(&mut h, 3);
    }
}

/// #2573, #1901: Picture Format › Size's width shows the interface unit and takes a size typed
/// in pixels, keeping the picture's proportions, as one undo step; Esc throws a typed size away
/// with no undo step.
#[test]
fn picture_width_typed_in_pixels_keeps_the_proportions() {
    use egui::accesskit::Role;
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("file.options", json!({"units": "centimeters"})).unwrap();
    a.run("insert.picture", json!({"data": png_data()})).unwrap();
    assert_eq!(object_size(&a), Some((15.0, 7.5)), "20×10 px at 96 ppi");
    let mut h = tab_harness(a, "Picture Format");
    let undo = h.state().session.undo_depth();
    // The unit is the field's own adornment, so it shows in the number: 15 pt is 0.53 cm.
    let shown = h.get_by_role_and_label(Role::SpinButton, "W:").accesskit_node().numeric_value().unwrap_or(f64::NAN);
    assert!((shown - 15.0 * 2.54 / 72.0).abs() < 0.01, "the interface unit: {shown:?}");
    h.get_by_role_and_label(Role::SpinButton, "W:").click();
    crate::test_harness::frames(&mut h, 2);
    h.get_by_role_and_label(Role::SpinButton, "W:").type_text("300 px");
    crate::test_harness::frames(&mut h, 2);
    h.key_press(egui::Key::Enter);
    crate::test_harness::frames(&mut h, 3);
    let (w, ht) = object_size(h.state()).unwrap();
    assert!((w - 225.0).abs() < 0.01 && (ht - 112.5).abs() < 0.01, "300 px = 225 pt, the height follows: {w}×{ht}");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(object_size(h.state()), Some((15.0, 7.5)), "one undo step");
    // Esc throws a typed size away.
    let undo_before = h.state().session.undo_depth();
    h.get_by_role_and_label(Role::SpinButton, "W:").click();
    crate::test_harness::frames(&mut h, 2);
    h.get_by_role_and_label(Role::SpinButton, "W:").type_text("500 px");
    h.key_press(egui::Key::Escape);
    crate::test_harness::frames(&mut h, 3);
    assert_eq!(object_size(h.state()), Some((15.0, 7.5)), "Esc cancels the typing");
    assert_eq!(h.state().session.undo_depth(), undo_before, "and leaves no undo step");
    assert!(undo_before <= undo + 1);
}

/// #4196: Shape Format › Size shows the interface unit, in every unit, and takes a size typed in
/// any unit once typed (Enter, Tab), each side on its own, as one undo step; Esc throws it away.
/// A field focused and left without typing changes nothing, in Picture Format too, where the
/// size it shows is rounded.
#[test]
fn shape_size_fields_use_the_interface_unit() {
    use egui::accesskit::Role;
    let harness = |units: &str, picture: Option<&str>, wh: Option<(f32, f32)>| {
        let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("file.options", json!({"units": units})).unwrap();
        if let Some(data) = picture {
            a.run("insert.picture", json!({"data": data})).unwrap();
        } else {
            a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        }
        if let Some((w, h)) = wh {
            a.run("picture.size", json!({"width": w, "height": h, "lockAspect": false})).unwrap();
        }
        tab_harness(a, if picture.is_some() { "Picture Format" } else { "Shape Format" })
    };
    // The unit is the field's own adornment (drawn after the number, not part of its text), so it
    // shows in the number the field holds: the size in points scaled to the interface unit.
    for (units, per_pt) in
        [("centimeters", 2.54 / 72.0), ("inches", 1.0 / 72.0), ("millimeters", 25.4 / 72.0), ("points", 1.0), ("picas", 1.0 / 12.0)]
    {
        let h = harness(units, None, None);
        let (w, ht) = object_size(h.state()).unwrap();
        for (side, pt) in [("W:", w), ("H:", ht)] {
            let shown = h.get_by_role_and_label(Role::SpinButton, side).accesskit_node().numeric_value().unwrap_or(f64::NAN);
            let want = f64::from(pt) * per_pt;
            assert!((shown - want).abs() <= want * 0.01 + 0.005, "{units}, {side} {shown:?}, want about {want}");
        }
    }
    // Sizes the fields show rounded: 100×50 pt is 1.39"×0.69", a 3000×1 pt line's height
    // shows 0.01" (below the fields' 1 pt), a 20×10 px picture is 0.21"×0.1".
    for units in ["inches", "centimeters", "picas"] {
        for (picture, wh) in [(false, Some((100.0, 50.0))), (false, Some((3000.0, 1.0))), (true, None)] {
            let mut h = harness(units, picture.then(png_data).as_deref(), wh);
            let (before, undo) = (object_size(h.state()), h.state().session.undo_depth());
            assert!(wh.is_none() || before == wh, "showing the tab changes nothing: {before:?}");
            for (side, key) in [("W:", egui::Key::Enter), ("H:", egui::Key::Tab)] {
                h.get_by_role_and_label(Role::SpinButton, side).click();
                crate::test_harness::frames(&mut h, 2);
                h.key_press(key);
                crate::test_harness::frames(&mut h, 3);
            }
            let after = (object_size(h.state()), h.state().session.undo_depth());
            assert_eq!(after, (before, undo), "{units}, picture {picture}, {wh:?}: left without typing");
        }
    }
    let mut h = harness("centimeters", None, None);
    let (w0, h0) = object_size(h.state()).unwrap();
    let undo = h.state().session.undo_depth();
    for (side, typed, key, want) in [("W:", "3 cm", egui::Key::Enter, (85.04, h0)), ("H:", "300 px", egui::Key::Tab, (85.04, 225.0))] {
        h.get_by_role_and_label(Role::SpinButton, side).click();
        crate::test_harness::frames(&mut h, 2);
        h.get_by_role_and_label(Role::SpinButton, side).type_text(typed);
        crate::test_harness::frames(&mut h, 2);
        h.key_press(key);
        crate::test_harness::frames(&mut h, 3);
        let (w, ht) = object_size(h.state()).unwrap();
        assert!((w - want.0).abs() < 0.01 && (ht - want.1).abs() < 0.01, "{side} {typed}: {w}×{ht}, want {want:?}");
    }
    assert_eq!(h.state().session.undo_depth(), undo + 2, "one undo step a side");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(object_size(h.state()), Some((w0, h0)));
    crate::test_harness::frames(&mut h, 2);
    h.get_by_role_and_label(Role::SpinButton, "W:").click();
    crate::test_harness::frames(&mut h, 2);
    h.get_by_role_and_label(Role::SpinButton, "W:").type_text("10 cm");
    h.key_press(egui::Key::Escape);
    crate::test_harness::frames(&mut h, 3);
    assert_eq!(object_size(h.state()), Some((w0, h0)), "Esc cancels the typing");
    // A size typed at the range's bound applies although the field shows that bound: a 1×1 px
    // picture (0.75 pt, below the fields' 1 pt) typed to 1 pt.
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
    let png = wordcraft_geom::picture::encode_format(&image::DynamicImage::from(img.clone()), image::ImageFormat::Png).unwrap();
    let mut h = harness("points", Some(&wordcraft_engine::cmd::insert::base64_encode(&png)), None);
    assert_eq!(object_size(h.state()), Some((0.75, 0.75)));
    h.get_by_role_and_label(Role::SpinButton, "W:").click();
    crate::test_harness::frames(&mut h, 2);
    h.get_by_role_and_label(Role::SpinButton, "W:").type_text("1");
    h.key_press(egui::Key::Enter);
    crate::test_harness::frames(&mut h, 3);
    assert_eq!(object_size(h.state()), Some((1.0, 1.0)), "1 pt typed at the bound");
}

/// #2573 review: showing Picture Format never resizes a picture whose size is outside the
/// fields' range (a 1×1 px spacer is 0.75 pt): the field shows the value unsettled and writes nothing.
#[test]
fn picture_format_leaves_a_tiny_picture_alone() {
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
    let png = wordcraft_geom::picture::encode_format(&image::DynamicImage::from(img.clone()), image::ImageFormat::Png).unwrap();
    a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png)})).unwrap();
    assert_eq!(object_size(&a), Some((0.75, 0.75)));
    let undo = a.session.undo_depth();
    let mut h = tab_harness(a, "Picture Format");
    crate::test_harness::frames(&mut h, 6);
    assert_eq!(object_size(h.state()), Some((0.75, 0.75)), "shown, not resized");
    assert_eq!(h.state().session.undo_depth(), undo, "no undo step");
}

/// #3242: Align used to run `arrange.align` directly with no `value`, which always centers
/// (`crates/engine/src/cmd/objects.rs`) — there was no way to reach Left or Right from the
/// ribbon short of dragging. Align now opens a menu, like Word's own Align dropdown.
#[test]
fn align_menu_reaches_left_and_right_not_just_center() {
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("insert.picture", json!({"data": png_data()})).unwrap();
    let frame = |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, f.x)));
    assert!(matches!(frame(&a), Some((wordcraft_doc::para::Wrap::Inline, _))), "a freshly inserted picture is inline");
    let mut h = tab_harness(a, "Picture Format");
    click_button(&mut h, "Align");
    assert!(h.query(By::new().label_contains("Align Left")).is_some(), "Align opens a menu, instead of running arrange.align with no value");
    click_row(&mut h, "Align Right");
    let (wrap, right_x) = frame(h.state()).unwrap();
    assert_eq!(wrap, wordcraft_doc::para::Wrap::Square, "aligning floats an inline picture, as arrange.align already does");
    // A positive x alone doesn't pin Right down: Center's x is positive too whenever the
    // picture is narrower than the text column, so Right could silently dispatch Center.
    click_button(&mut h, "Align");
    click_row(&mut h, "Align Center");
    let center_x = frame(h.state()).unwrap().1;
    click_button(&mut h, "Align");
    click_row(&mut h, "Align Left");
    let left_x = frame(h.state()).unwrap().1;
    assert!(left_x < center_x && center_x < right_x, "Left, Center and Right each land at a distinct x: {left_x} < {center_x} < {right_x}");
}

/// #2573 review: Shape Format and Chart Design leave sizes outside their fields' ranges alone
/// too (a 0.5 pt high line; a chart narrower than 36 pt), as Picture Format does.
#[test]
fn shape_and_chart_formats_leave_out_of_range_sizes_alone() {
    let shown = |a: crate::WordApp, tab: &str| {
        let mut h = tab_harness(a, tab);
        crate::test_harness::frames(&mut h, 6);
        let a = h.state();
        (object_size(a), a.session.undo_depth())
    };
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    a.run("picture.size", json!({"width": 3000.0, "height": 1.0, "lockAspect": false})).unwrap();
    let before = (object_size(&a), a.session.undo_depth());
    assert_eq!(before.0, Some((3000.0, 1.0)));
    assert_eq!(shown(a, "Shape Format"), before, "Shape Format shown, the shape untouched");

    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("insert.chart", json!({"type": "doughnut"})).unwrap();
    a.run("picture.size", json!({"width": 20.0, "height": 2500.0, "lockAspect": false})).unwrap();
    let before = (object_size(&a), a.session.undo_depth());
    assert_eq!(before.0, Some((20.0, 2500.0)));
    assert_eq!(shown(a, "Chart Design"), before, "Chart Design shown, the chart untouched");
}

/// The Shading arrow's name: "Shading options", in the interface language (#3172).
fn shading_options() -> String {
    crate::i18n::fmt(tl!("{name} options"), &[("name", tl!("Shading"))])
}

#[test]
fn table_shading_label_is_painted_beside_its_arrow() {
    use egui::accesskit::Role;
    // Vietnamese and German labels need the embedded catalogs (CI has them); English always runs in both themes.
    let cases: &[(&str, egui::Theme)] = if crate::i18n::catalogs_embedded() {
        &[("en", egui::Theme::Light), ("en", egui::Theme::Dark), ("vi", egui::Theme::Light), ("de", egui::Theme::Light)]
    } else {
        &[("en", egui::Theme::Light), ("en", egui::Theme::Dark)]
    };
    let ctx = crate::test_harness::context();
    ctx.enable_accesskit();
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 100.0));
    for (idx, &(language, theme)) in cases.iter().enumerate() {
        app.run("ui.language", json!({"value": language})).unwrap();
        app.run("ui.theme", json!({"value": if theme == egui::Theme::Dark { "dark" } else { "light" }})).unwrap();
        ctx.set_theme(theme);
        let raw = egui::RawInput { focused: true, time: Some(idx as f64 * 0.1), screen_rect: Some(screen), ..Default::default() };
        craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
        let mut out = ctx.run_ui(raw, |ui| {
            let _ = crate::colors::shading_split(
                ui,
                &mut app,
                tl!("Shading").to_string(),
                crate::colors::Target::cmd("table.shading").remember(crate::colors::Remember::Shading),
            );
        });
        out.textures_delta.clear();
        assert_eq!(ctx.global_style().visuals.dark_mode, theme == egui::Theme::Dark, "{language}/{theme:?}: palette active");
        let label = tl!("Shading");
        let arrow_label = shading_options();
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("accesskit update").nodes;
        let button_rect = |name: &str| {
            nodes
                .iter()
                .find_map(|(_, n)| {
                    (n.role() == Role::Button && n.label() == Some(name))
                        .then(|| {
                            n.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
                        })
                        .flatten()
                })
                .unwrap_or_else(|| panic!("{language}: missing button {name:?}"))
        };
        let main = button_rect(label);
        let arrow = button_rect(&arrow_label);
        assert!((main.top() - arrow.top()).abs() < 0.01 && (main.bottom() - arrow.bottom()).abs() < 0.01);
        assert!((main.right() - arrow.right()).abs() < 0.01 && arrow.left() > main.left(), "{language}: {main:?}, {arrow:?}");
        let text = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t),
                _ => None,
            })
            .expect("Shading is drawn as text, not just a hover tip");
        let text_rect = egui::Rect::from_min_size(text.pos, text.galley.size());
        assert!(main.contains_rect(text_rect), "{language}: {text_rect:?} outside {main:?}");
        assert!(text_rect.left() >= main.left() + 24.0, "the label starts after the icon");
        assert!(text_rect.right() <= arrow.left(), "{language}: label {text_rect:?} overlaps arrow {arrow:?}");
    }
    crate::i18n::apply_language(crate::i18n::Lang::EN);
}

fn first_cell_shading(app: &WordApp) -> Option<wordcraft_doc::Rgb> {
    app.session.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows[0].cells[0].props.shading
}

/// #1726: the arrow opens the real colour menu without applying the remembered colour;
/// picking a swatch remembers it, and the command half reapplies it after one-step undo.
#[test]
fn table_shading_split_keeps_palette_and_apply_separate() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
    app.canvas.last_shading = "0070C0".into();
    let mut h = tab_harness(app, "Table Design");
    let palette_open = |h: &Harness| h.query_by_label_contains("No Color").is_some();
    // A swatch of the document theme's palette (the Craft theme has no pure red); swatches are named by their hex.
    let red = wordcraft_doc::Rgb(0xE9, 0x71, 0x32);
    let original = first_cell_shading(h.state());
    click_button(&mut h, &shading_options());
    assert_eq!(first_cell_shading(h.state()), original, "opening the palette must not apply the last colour");
    assert!(palette_open(&h), "the arrow opens the shading palette");
    click_row(&mut h, "E97132");
    assert_eq!(first_cell_shading(h.state()), Some(red));
    assert_eq!(h.state().canvas.last_shading, "E97132");
    assert!(!palette_open(&h), "a colour pick closes the palette");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    crate::test_harness::frames(&mut h, 2);
    assert_eq!(first_cell_shading(h.state()), original);
    click_button(&mut h, "Shading");
    assert_eq!(first_cell_shading(h.state()), Some(red));
    assert!(!palette_open(&h), "the command half must not open the palette");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(first_cell_shading(h.state()), original);
    click_button(&mut h, "Shading");
    assert_eq!(first_cell_shading(h.state()), Some(red));
    click_button(&mut h, &shading_options());
    click_row(&mut h, "No Color");
    assert_eq!(first_cell_shading(h.state()), None);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(first_cell_shading(h.state()), Some(red), "No Color is undone in one step");
}

#[test]
fn table_styles_gallery_uses_large_landscape_thumbnails() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{By, Queryable};
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
    let mut h = tab_harness(app, "Design");
    let style_set_height = h
        .query_all(By::new().role(Role::RadioButton).label_contains("Style set:"))
        .next()
        .expect("Design › Style Sets renders its large gallery tiles")
        .rect()
        .height();

    h.state_mut().run("ui.tab", json!({"tab": "Table Design"})).unwrap();
    crate::test_harness::frames(&mut h, 3);

    let mut styles: Vec<(bool, String)> = h
        .state()
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| s.kind == wordcraft_doc::StyleKind::Table && !s.hidden)
        .map(|s| (s.builtin, crate::i18n::style_name(s)))
        .collect();
    styles.sort_by_key(|(builtin, _)| *builtin);
    let expected: Vec<String> = styles.into_iter().take(7).map(|(_, name)| name).collect();
    assert_eq!(expected.len(), 7, "built-in document has at least seven table styles");

    let tiles: Vec<(String, egui::Rect)> = expected
        .iter()
        .filter_map(|name| h.query_all(By::new().role(Role::RadioButton).label(name)).next().map(|node| (name.clone(), node.rect())))
        .collect();
    assert_eq!(tiles.len(), expected.len(), "all seven Table Styles tiles render at full ribbon width: {tiles:?}");

    for (name, rect) in &tiles {
        assert!(
            rect.height() >= style_set_height - 1.0,
            "Table Styles tile `{name}` height should be at least Design › Style Sets' Large thumbnail height ({style_set_height}), got {rect:?}"
        );
        let ratio = rect.width() / rect.height();
        assert!(
            (1.25..=1.75).contains(&ratio),
            "Table Styles tile `{name}` should use a landscape aspect ratio matching its 3:2 table preview, got {ratio:.2} ({rect:?})"
        );
    }
    assert!(tiles.windows(2).all(|w| w[0].1.max.x <= w[1].1.min.x + 0.01), "Table Styles tiles sit side by side without overlapping: {tiles:?}");
}

/// #7308: the compact menu and split buttons of Home › Font and Home › Paragraph show their icons
/// alone (keeping their names for access and tooltips) even in a wide window, leaving room for all
/// six tiles of the Styles gallery at 1920 pt.
#[test]
fn home_font_and_paragraph_buttons_stay_icon_only_and_styles_shows_six_tiles() {
    use egui::accesskit::Role;

    let button_width = |h: &Harness, name: &str| {
        h.query_all(By::new().role(Role::Button).label(name)).next().unwrap_or_else(|| panic!("a button named {name}")).rect().width()
    };

    // Wide enough that the ribbon keeps every group at GroupLevel::Full: without `icon_only`, both
    // Font and Paragraph render their full text labels, making longer names much wider than shorter
    // ones and wider than text-labelled buttons like Replace.
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    let wide = tab_harness(app, "Home");
    let replace_w = button_width(&wide, "Replace");

    let menu_buttons = ["Change Case", "Text Effects and Typography", "Line and Paragraph Spacing"];
    let menu_widths: Vec<(&str, f32)> = menu_buttons.iter().map(|&n| (n, button_width(&wide, n))).collect();
    let base_menu_w = menu_widths[0].1;
    for &(name, w) in &menu_widths {
        assert!((w - base_menu_w).abs() < 1.0, "{name} ({w:.1} pt) differs from icon-only menu width ({base_menu_w:.1} pt): {menu_widths:?}");
        assert!(w < replace_w, "{name} ({w:.1} pt) should be narrower than a labelled button like Replace ({replace_w:.1} pt)");
    }

    let split_buttons = ["Underline", "Text Highlight Color", "Font Color", "Bullets", "Numbering", "Multilevel List", "Shading", "Borders"];
    let split_widths: Vec<(&str, f32)> = split_buttons.iter().map(|&n| (n, button_width(&wide, n))).collect();
    let base_split_w = split_widths[0].1;
    for &(name, w) in &split_widths {
        assert!((w - base_split_w).abs() < 1.0, "{name} ({w:.1} pt) differs from icon-only split width ({base_split_w:.1} pt): {split_widths:?}");
        assert!(w < replace_w, "{name} ({w:.1} pt) should be narrower than a labelled button like Replace ({replace_w:.1} pt)");
    }

    // At 1920 pt, Font no longer starves the Styles gallery: all six default quick styles show.
    let app = WordApp::new(Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
    let full_hd = app_harness(app, vec2(1920.0, 900.0));
    let shown = |name: &str| full_hd.query_all(By::new().label(name)).next().is_some();
    for style in ["Normal", "No Spacing", "Heading 1", "Title", "Heading 2", "Subtitle"] {
        assert!(shown(style), "Home › Styles should show {style} at 1920 pt");
    }
}

/// #7323: the Home › Bullets dropdown renders the Bullet Library grid as compact
/// square preview tiles (~22×22 pt), rather than expanding to full card dimensions (~132 pt).
#[test]
fn bullets_dropdown_renders_compact_bullet_tiles() {
    use egui::accesskit::Role;

    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Sample")), crate::Services::default());
    let mut h = tab_harness(app, "Home");
    h.ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);

    // The SplitButton has two button nodes: the command part and the arrow part.
    let arrow = h.query_all(By::new().role(Role::Button).label("Bullets")).nth(1).expect("Bullets split-button menu arrow");
    arrow.click();
    crate::test_harness::frames(&mut h, 3);

    let bullet_tile = h.query_all(By::new().role(Role::ListItem).label("•")).next().expect("bullet tile •");
    let rect = bullet_tile.rect();
    assert!(
        rect.width() >= 16.0 && rect.width() <= 32.0,
        "bullet tile width ({:.1} pt) should be compact (expected ~22 pt, bound [16, 32])",
        rect.width()
    );
    assert!(
        rect.height() >= 16.0 && rect.height() <= 32.0,
        "bullet tile height ({:.1} pt) should be compact (expected ~22 pt, bound [16, 32])",
        rect.height()
    );
    assert!((rect.width() - rect.height()).abs() <= 2.0, "bullet tile should be square: {}x{}", rect.width(), rect.height());
}

/// #7310: unimplemented stub ribbon buttons (Dictate, Ink to Math, Translate, Block Authors)
/// resolve disabled in the ribbon UI and are known to the command catalog without warnings.
#[test]
fn unimplemented_stub_ribbon_buttons_are_disabled() {
    use egui::accesskit::Role;

    let cases = [
        ("Home", "Dictate", "tools.dictate"),
        ("Draw", "Ink to Math", "draw.inkToMath"),
        ("Review", "Translate", "review.translate"),
        ("Review", "Block Authors", "review.blockAuthors"),
    ];

    for (tab, expected_label, id) in cases {
        let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
        let h = tab_harness(app, tab);

        let keymap = craft::ui::runtime::commands::keymap(&h.ctx).expect("command board has a keymap");
        assert!(keymap.catalog().iter().any(|def| def.command.id == id), "{tab}: stub command {id} should be registered in the app catalog");

        let button = h
            .query_all(By::new().role(Role::Button))
            .find(|n| n.accesskit_node().label().is_some_and(|l| l.split_whitespace().collect::<Vec<_>>().join(" ") == expected_label))
            .unwrap_or_else(|| panic!("{tab}: ribbon button '{expected_label}' should exist"));
        assert!(button.accesskit_node().is_disabled(), "{tab}: stub button {:?} ({id}) must be disabled", button.accesskit_node().label());
    }
}

/// Is the ribbon button `name` disabled? (helpers for the table ribbon gate tests below.)
fn table_button_disabled(h: &Harness, name: &str) -> bool {
    use egui::accesskit::Role;
    h.query_all(By::new().role(Role::Button).label(name)).next().unwrap_or_else(|| panic!("button {name} not found")).accesskit_node().is_disabled()
}

fn table_button_hint(h: &Harness, name: &str) -> String {
    use egui::accesskit::Role;
    h.query_all(By::new().role(Role::Button).label(name))
        .next()
        .unwrap_or_else(|| panic!("button {name} not found"))
        .accesskit_node()
        .description()
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// A 2x2 table app on `tab`; `protected` sets read-only protection.
fn table_tab_harness(tab: &str, protected: bool) -> Harness {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
    if protected {
        app.session.doc.settings.protection = Some("readOnly".into());
    }
    tab_harness(app, tab)
}

/// #7324, #7457: without an active table the Table ribbon menu buttons disable. The Table tabs are
/// configured explicitly so they show with the cursor outside a table.
#[test]
fn table_ribbon_menu_buttons_disable_when_unselected() {
    for tab in ["Table Design", "Table Layout"] {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
        app.session.ribbon.tabs = vec![
            wordcraft_engine::ribbon::TabEntry { name: "Table Design".to_string(), custom: false, hidden: false, groups: Vec::new() },
            wordcraft_engine::ribbon::TabEntry { name: "Table Layout".to_string(), custom: false, hidden: false, groups: Vec::new() },
        ];
        let h = tab_harness(app, tab);
        if tab == "Table Design" {
            assert!(table_button_disabled(&h, "Styles"), "Styles button should be disabled when not in table");
            assert!(table_button_disabled(&h, "Borders"), "Borders button should be disabled when not in table");
        } else {
            assert!(table_button_disabled(&h, "Select"), "Select button should be disabled when not in table");
            assert!(table_button_disabled(&h, "Delete"), "Delete button should be disabled when not in table");
            assert!(table_button_disabled(&h, "AutoFit"), "AutoFit button should be disabled when not in table");
            assert!(
                table_button_hint(&h, "Select").to_ascii_lowercase().contains("the cursor isn't in a table"),
                "Select should indicate outside table in disabled hint"
            );
        }
    }
}

/// With an active, unprotected table the Table Design buttons are enabled.
#[test]
fn table_design_menu_buttons_enabled_in_table() {
    let h = table_tab_harness("Table Design", false);
    assert!(!table_button_disabled(&h, "Styles"), "Styles button should be enabled in table");
    assert!(!table_button_disabled(&h, "Borders"), "Borders button should be enabled in table");
}

/// With an active, unprotected table the Table Layout buttons are enabled and open their menus
/// (the controls for the shut-menu checks under protection).
#[test]
fn table_layout_menu_buttons_enabled_in_table() {
    let mut h = table_tab_harness("Table Layout", false);
    assert!(!table_button_disabled(&h, "Select"), "Select button should be enabled in table");
    assert!(!table_button_disabled(&h, "Delete"), "Delete button should be enabled in table");
    assert!(!table_button_disabled(&h, "AutoFit"), "AutoFit button should be enabled in table");
    click_button(&mut h, "Delete");
    assert!(h.query_all(menu_row("Delete Rows")).next().is_some(), "an enabled Delete opens its menu");
}

/// Positive control for the AutoFit shut-menu check under protection: an enabled AutoFit opens its menu.
#[test]
fn table_layout_autofit_opens_menu_in_table() {
    let mut h = table_tab_harness("Table Layout", false);
    click_button(&mut h, "AutoFit");
    assert!(h.query_all(menu_row("AutoFit Window")).next().is_some(), "an enabled AutoFit opens its menu");
}

/// Under read-only protection the Table Design buttons disable and say why.
#[test]
fn table_design_menu_buttons_disable_when_protected() {
    let h = table_tab_harness("Table Design", true);
    assert!(table_button_disabled(&h, "Styles"), "Styles button should be disabled under protection");
    assert!(table_button_disabled(&h, "Borders"), "Borders button should be disabled under protection");
    assert!(table_button_hint(&h, "Styles").contains("protected"), "Styles should indicate protection in disabled hint");
}

/// Under read-only protection the mutating Table Layout buttons disable, say why, and stay shut
/// when clicked (C2-5, #7824).
#[test]
fn table_layout_menu_buttons_disable_when_protected() {
    let mut h = table_tab_harness("Table Layout", true);
    assert!(!table_button_disabled(&h, "Select"), "Select button should remain enabled under protection");
    assert!(table_button_disabled(&h, "Delete"), "Delete button should be disabled under protection");
    assert!(table_button_disabled(&h, "AutoFit"), "AutoFit button should be disabled under protection");
    assert!(table_button_hint(&h, "Delete").contains("protected"), "Delete should indicate protection in disabled hint");
    click_button(&mut h, "Delete");
    assert!(h.query_all(menu_row("Delete Rows")).next().is_none(), "a disabled Delete opens no menu");
    click_button(&mut h, "AutoFit");
    assert!(h.query_all(menu_row("AutoFit Window")).next().is_none(), "a disabled AutoFit opens no menu");
}

/// #7374: the Decorative Text / WordArt Styles dropdown menu renders compact tiles (~40×36 pt)
/// rather than expanding to default card thumbnail size (~132×119 pt).
#[test]
fn decorative_text_dropdown_renders_compact_wordart_tiles() {
    use egui::accesskit::Role;

    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Sample")), crate::Services::default());
    let mut h = tab_harness(app, "Insert");
    click_button(&mut h, "Decorative Text");

    let tile = h.query_all(By::new().role(Role::ListItem).label("Solid Accent")).next().expect("Solid Accent WordArt tile");
    let rect = tile.rect();
    assert!(
        rect.width() >= 32.0 && rect.width() <= 50.0,
        "WordArt tile width ({:.1} pt) should be compact (expected ~40 pt, bound [32, 50]), got {:.1}",
        rect.width(),
        rect.width()
    );
    assert!(
        rect.height() >= 28.0 && rect.height() <= 46.0,
        "WordArt tile height ({:.1} pt) should be compact (expected ~36 pt, bound [28, 46]), got {:.1}",
        rect.height(),
        rect.height()
    );
}

/// #7691: Table Design › Table Style Options aligns its six checkboxes in two columns with a gap
/// between them and keeps all three rows above the group caption.
#[test]
fn table_style_options_align_in_two_columns() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
    let h = tab_harness(app, "Table Design");
    let rect = |label: &str| h.get_by_role_and_label(Role::CheckBox, label).rect();
    let header = rect("Header Row");
    let first_col = rect("First Column");
    let total = rect("Total Row");
    let last_col = rect("Last Column");
    let banded = rect("Banded Rows");
    let banded_col = rect("Banded Columns");
    // Left column checkboxes share one left edge, and right column checkboxes share another.
    assert!(
        (header.left() - total.left()).abs() < 0.5 && (header.left() - banded.left()).abs() < 0.5,
        "left column misaligned: {header:?}, {total:?}, {banded:?}"
    );
    assert!(
        (first_col.left() - last_col.left()).abs() < 0.5 && (first_col.left() - banded_col.left()).abs() < 0.5,
        "right column misaligned: {first_col:?}, {last_col:?}, {banded_col:?}"
    );
    // Each row's pair shares the same top edge and stacks vertically with tight spacing.
    let env = craft::ui::runtime::env::Env::of_root(&h.ctx);
    let tight_gap = env.tokens.space.gap.tight;
    let min_col_gap = env.tokens.space.gap.stack;
    assert!((header.top() - first_col.top()).abs() < 0.5, "row 1 misaligned: {header:?}, {first_col:?}");
    assert!((total.top() - last_col.top()).abs() < 0.5, "row 2 misaligned: {total:?}, {last_col:?}");
    assert!((banded.top() - banded_col.top()).abs() < 0.5, "row 3 misaligned: {banded:?}, {banded_col:?}");
    assert!(header.bottom() <= total.top() && total.bottom() <= banded.top(), "rows overlap: {header:?}, {total:?}, {banded:?}");
    let row_gap_1 = total.top() - header.bottom();
    let row_gap_2 = banded.top() - total.bottom();
    assert!(
        (row_gap_1 - tight_gap).abs() <= 0.5 && (row_gap_2 - tight_gap).abs() <= 0.5,
        "rows should remain Gap::Tight ({tight_gap} pt) apart, got {row_gap_1} and {row_gap_2}"
    );
    // A horizontal gap at least `space.gap.stack` separates the widest left-column label from the second column.
    let text_rect = |label: &str| {
        h.output()
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("painted text for {label:?}"))
    };
    let left_label_max_right =
        ["Header Row", "Total Row", "Banded Rows"].into_iter().map(|label| text_rect(label).right()).fold(f32::NEG_INFINITY, f32::max);
    let col_gap = first_col.left() - left_label_max_right;
    assert!(
        col_gap >= min_col_gap,
        "column gap between widest left label and right column too narrow: got {col_gap:.2} pt, expected >= {min_col_gap:.2} pt (left_label_max_right={left_label_max_right}, first_col={first_col:?})"
    );
    // All three rows stay above the group's caption.
    let caption = text_rect("Table Style Options");
    assert!(banded.bottom() <= caption.top(), "checkboxes collide with group caption: {banded:?} vs {caption:?}");
}

struct StylesGalleryLayout {
    gallery: egui::Rect,
    tiles: Vec<(String, egui::Rect, Option<egui::accesskit::Toggled>)>,
    up: egui::Rect,
    down: egui::Rect,
    more: egui::Rect,
}

/// Lay out the Home tab in LTR (`rtl == false`) or Arabic RTL (`rtl == true`) using `fixture_lang`
/// so the test never skips when `CRAFT_I18N_DIR` is unset, and collect the Styles gallery's
/// container, visible tiles, and stepper/More buttons.
fn styles_gallery_layout(rtl: bool) -> StylesGalleryLayout {
    use egui::accesskit::Role;

    struct ResetLang;
    impl Drop for ResetLang {
        fn drop(&mut self) {
            crate::i18n::apply_language(crate::i18n::Lang::EN);
        }
    }
    let _reset = ResetLang;

    let lang = if rtl {
        crate::i18n::fixture_lang("ar", "\tHome\tالرئيسية\n\tStyles\tالأنماط\n\tNormal\tعادي\n")
    } else {
        crate::i18n::Lang::EN
    };

    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.ui.language = "en".into();
    app.run("ui.tab", json!({"tab": "Home"})).unwrap();

    let mut h = crate::test_harness::builder().with_size(vec2(2560.0, 900.0)).build_ui_state(
        move |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            if rtl {
                ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                crate::i18n::apply_language(lang);
                ctx.global_style_mut(|s| s.right_to_left = true);
                craft::ui::runtime::env::with_env(ui, |o| o.direction = Some(craft::i18n::Direction::Rtl), |ui| app.ui(ui));
                ctx.global_style_mut(|s| s.right_to_left = false);
                crate::i18n::apply_language(crate::i18n::Lang::EN);
            } else {
                app.ui(ui);
            }
        },
        app,
    );
    crate::test_harness::frames(&mut h, 4);

    crate::i18n::apply_language(lang);
    let gallery_label = crate::i18n::t("Styles").to_string();
    let names: Vec<String> = h.state().session.doc.styles.gallery().iter().take(6).map(|s| crate::i18n::style_name(s)).collect();
    crate::i18n::apply_language(crate::i18n::Lang::EN);

    let gallery = h
        .query_all(By::new().role(Role::RadioGroup).label(gallery_label.as_str()))
        .next()
        .unwrap_or_else(|| panic!("Styles radio group '{gallery_label}' not found"))
        .rect();
    assert!(gallery.is_positive(), "Styles gallery rect must be positive: {gallery:?}");

    let in_gallery = |rect: egui::Rect| gallery.contains_rect(rect);
    let tiles: Vec<_> = names
        .iter()
        .filter_map(|name| {
            h.query_all(By::new().role(Role::RadioButton).label(name.as_str()))
                .find(|node| in_gallery(node.rect()))
                .map(|node| (name.clone(), node.rect(), node.accesskit_node().toggled()))
        })
        .collect();
    assert_eq!(tiles.len(), names.len(), "expected all {} visible Styles tiles inside {gallery:?}: {tiles:?} vs {names:?}", names.len());

    let button = |label: &str| {
        h.query_all(By::new().role(Role::Button).label(label))
            .find(|node| in_gallery(node.rect()))
            .unwrap_or_else(|| panic!("gallery button '{label}' not found inside {gallery:?}"))
            .rect()
    };

    StylesGalleryLayout { gallery, tiles, up: button("Previous row"), down: button("Next row"), more: button("More") }
}

/// #5566, #7753: in a right-to-left interface the Styles gallery starts at the right, as the rest
/// of the ribbon does: the first style (Normal) is the rightmost tile, and the scroll arrows and
/// More button sit in a column at the gallery's left end. Left to right is unchanged.
#[test]
fn the_styles_gallery_reads_from_the_right_in_a_right_to_left_interface() {
    use egui::accesskit::Toggled;

    let assert_common = |layout: &StylesGalleryLayout, expected_first: &str, dir: &str| {
        assert_eq!(layout.tiles.len(), 6, "{dir}: expected 6 visible gallery tiles: {:?}", layout.tiles);
        let (first_name, first_rect, first_toggled) = &layout.tiles[0];
        assert_eq!(first_name, expected_first, "{dir}: first tile should be localized Normal");
        assert_eq!(*first_toggled, Some(Toggled::True), "{dir}: active Normal style should be checked");
        assert!(
            layout
                .tiles
                .iter()
                .all(|(_, r, _)| r.is_positive() && layout.gallery.contains_rect(*r) && (r.center().y - first_rect.center().y).abs() <= 1.0),
            "{dir}: gallery tiles must sit on one horizontal row inside {:?}: {:?}",
            layout.gallery,
            layout.tiles
        );

        let controls = [("Previous row", layout.up), ("Next row", layout.down), ("More", layout.more)];
        for (name, rect) in controls {
            assert!(rect.is_positive(), "{dir}: {name} rect must be positive: {rect:?}");
            assert!(layout.gallery.contains_rect(rect), "{dir}: {name} {rect:?} must sit inside gallery {:?}", layout.gallery);
        }
        assert!(
            (layout.up.center().x - layout.down.center().x).abs() <= 1.0 && (layout.down.center().x - layout.more.center().x).abs() <= 1.0,
            "{dir}: up, down, and More buttons must align in one column: up={:?}, down={:?}, more={:?}",
            layout.up,
            layout.down,
            layout.more
        );
        assert!(
            layout.up.max.y <= layout.down.min.y + 1.0 && layout.down.max.y <= layout.more.min.y + 1.0,
            "{dir}: buttons must stack top-to-bottom (up -> down -> More): up={:?}, down={:?}, more={:?}",
            layout.up,
            layout.down,
            layout.more
        );
    };

    // 1. Left-to-right (English)
    let ltr = styles_gallery_layout(false);
    assert_common(&ltr, "Normal", "LTR");
    assert!(
        ltr.tiles.windows(2).all(|w| w[0].1.center().x < w[1].1.center().x && w[0].1.max.x <= w[1].1.min.x + 1.0),
        "LTR: tiles must run left to right: {:?}",
        ltr.tiles
    );
    for (name, ctrl) in [("Previous row", ltr.up), ("Next row", ltr.down), ("More", ltr.more)] {
        assert!(
            ltr.tiles.iter().all(|(_, tile, _)| ctrl.min.x >= tile.max.x),
            "LTR: {name} ({ctrl:?}) must sit to the right of every tile: {:?}",
            ltr.tiles
        );
    }

    // 2. Right-to-left (Arabic)
    let rtl = styles_gallery_layout(true);
    assert_common(&rtl, "عادي", "RTL");
    assert!(
        rtl.tiles.windows(2).all(|w| w[0].1.center().x > w[1].1.center().x && w[0].1.min.x + 1.0 >= w[1].1.max.x),
        "RTL: tiles must run right to left from the rightmost Normal tile: {:?}",
        rtl.tiles
    );
    for (name, ctrl) in [("Previous row", rtl.up), ("Next row", rtl.down), ("More", rtl.more)] {
        assert!(
            rtl.tiles.iter().all(|(_, tile, _)| ctrl.max.x <= tile.min.x),
            "RTL: {name} ({ctrl:?}) must sit to the left of every tile: {:?}",
            rtl.tiles
        );
    }
}

#[test]
fn a_combo_field_reached_with_tab_shows_its_hint_after_a_delay() {
    use egui::accesskit::Role;

    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Sample")), crate::Services::default());
    let mut h = tab_harness(app, "Home");

    // Focus the Font box right before Font Size, then Tab onto Font Size.
    h.get(By::new().role(Role::ComboBox).label("Font")).focus();
    h.step();
    for _ in 0..4 {
        h.key_press(egui::Key::Tab);
        h.step();
        if h.query(By::new().role(Role::ComboBox).label("Font Size")).is_some_and(|n| n.is_focused()) {
            break;
        }
    }
    assert!(h.get(By::new().role(Role::ComboBox).label("Font Size")).is_focused(), "Tab should focus Font Size");
    assert_eq!(h.query_all_by_label("Font Size").count(), 1, "hint must not show immediately before delay");

    let frames = ((craft::ui::contract::tooltip::DELAY * 60.0).ceil() as usize) + 5;
    for _ in 0..frames {
        h.step();
    }
    assert!(h.query_all_by_label("Font Size").count() >= 2, "hint must show after tooltip delay");
}

/// #7165: the Symbol drop-down's cells are compact (a thumbnail-small step, not a card) and the hover text names
/// the character, not just the glyph.
#[test]
fn symbol_menu_cells_are_compact_and_the_tooltip_names_the_character() {
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    let mut h = tab_harness(app, "Insert");
    click_button(&mut h, "Symbol");
    let cell_y = |h: &Harness| h.get_by_label("\u{A9}").rect().min.y;
    let cell = h.get_by_label("\u{A9}").rect();
    assert!(cell.width() <= 80.0 && cell.height() <= 80.0, "a symbol cell is compact, got {:?}", cell.size());
    // Eight symbols fit one row, so the menu does not cover the page.
    assert_eq!(h.get_by_label("\u{A5}").rect().min.y, cell_y(&h), "the eighth symbol is on the first row");
    h.get_by_label("\u{A9}").hover();
    crate::test_harness::frames(&mut h, 40);
    assert!(h.query_by_label_contains("COPYRIGHT SIGN (U+00A9)").is_some(), "the tooltip names the character");
    // A screen reader gets the name without hovering: the tile's label is the glyph, its description the name.
    let desc = h.get_by_label("\u{A9}").accesskit_node().description().map(|d| d.to_string()).unwrap_or_default();
    assert!(desc.contains("COPYRIGHT SIGN"), "the tile's accessible description names the character, got {desc:?}");
}

/// Characters outside the common list still say what they are: code point and block.
#[test]
fn symbol_tip_names_known_characters_and_falls_back_to_the_code_point() {
    use super::insert::symbol_tip;
    let named = symbol_tip("\u{A9}", "");
    assert!(named.contains("COPYRIGHT SIGN") && named.contains("00A9"), "named character: {named}");
    let with_font = symbol_tip("\u{3B1}", "Symbol");
    assert!(with_font.contains("GREEK SMALL LETTER ALPHA") && with_font.contains("03B1") && with_font.contains("Symbol"), "{with_font}");
    assert!(!named.contains("Symbol"), "no font, none named");
    let unnamed = symbol_tip("\u{2602}", "");
    assert!(unnamed.contains("2602") && unnamed.contains("Miscellaneous Symbols"), "unnamed character: {unnamed}");
    assert_eq!(symbol_tip("", ""), "");
}
