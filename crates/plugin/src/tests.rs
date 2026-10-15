use serde_json::json;

use crate::{ButtonSize, Manifest, MenuKind, parse_reply};

#[test]
fn manifests_are_validated() {
    let m = Manifest::parse(r#"{"id": "t.plugin", "name": "T", "run": {"command": "true"}, "commands": [{"id": "go", "label": "Go"}]}"#).unwrap();
    assert_eq!(m.command("go").map(|c| c.label.as_str()), Some("Go"));
    assert!(m.ui().ribbon.is_empty() && m.ui().menus.is_empty());
    for bad in [
        r#"{"id": "bad id", "name": "x", "run": {"command": "x"}}"#,
        r#"{"id": ".hidden", "name": "x", "run": {"command": "x"}}"#,
        r#"{"id": "x", "name": " ", "run": {"command": "x"}}"#,
        r#"{"id": "x", "name": "x", "run": {"command": ""}}"#,
        r#"{"id": "x", "name": "x", "run": {"command": "x"}, "commands": [{"id": "a", "label": "A"}, {"id": "a", "label": "B"}]}"#,
        r#"{"id": "x", "name": "x", "run": {"command": "x"}, "homepage": "javascript:alert(1)"}"#,
        "not json",
    ] {
        assert!(Manifest::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn ui_declarations_must_name_the_plugins_commands() {
    let with_ui = |ui: serde_json::Value| {
        json!({"id": "t.ui", "name": "T", "run": {"command": "true"}, "commands": [{"id": "go", "label": "Go"}], "ui": ui}).to_string()
    };
    let m = Manifest::parse(&with_ui(json!({
        "ribbon": [{"tab": "Review", "group": "Mine", "buttons": [{"command": "go", "size": "small", "params": {"n": 1}}]}],
        "menus": [{"command": "go"}, {"menu": "table", "command": "go", "label": "Go here"}]
    })))
    .unwrap();
    let ui = m.ui();
    assert_eq!(ui.ribbon[0].buttons[0].size, ButtonSize::Small);
    assert_eq!(ui.menus[0].menu, MenuKind::Text);
    assert_eq!(ui.menus[1].menu, MenuKind::Table);
    for bad in [
        json!({"ribbon": [{"tab": "Review", "group": "Mine", "buttons": [{"command": "nope"}]}]}),
        json!({"ribbon": [{"tab": "Review", "group": "Mine", "buttons": []}]}),
        json!({"ribbon": [{"tab": "", "group": "Mine", "buttons": [{"command": "go"}]}]}),
        json!({"ribbon": [{"tab": "Review", "group": "x".repeat(65), "buttons": [{"command": "go"}]}]}),
        json!({"menus": [{"command": "nope"}]}),
        json!({"menus": [{"menu": "cell", "command": "go"}]}),
    ] {
        assert!(Manifest::parse(&with_ui(bad.clone())).is_err(), "{bad}");
    }
}

#[test]
fn one_manifest_serves_several_apps() {
    // GridCraft's functions and cell menus mean nothing here; `uiFor.wordcraft` is what shows.
    let both = json!({
        "id": "t.both", "name": "Both", "run": {"command": "true"}, "apps": ["gridcraft", "wordcraft"],
        "functions": [{"name": "T.TWICE", "minArgs": 1, "maxArgs": 1}],
        "commands": [{"id": "tidy", "label": "Tidy"}],
        "ui": {"menus": [{"menu": "cell", "command": "tidy"}]},
        "uiFor": {"wordcraft": {"menus": [{"command": "tidy"}], "ribbon": [{"tab": "Tools", "group": "Tidy", "buttons": [{"command": "tidy"}]}]}}
    });
    let m = Manifest::parse(&both.to_string()).unwrap();
    assert_eq!(m.ui().menus.len(), 1);
    assert_eq!(m.ui().menus[0].menu, MenuKind::Text);
    assert_eq!(m.ui().ribbon[0].tab, "Tools");
    assert_eq!(m.functions.len(), 1);
    let only_grid = json!({"id": "t.grid", "name": "Grid", "run": {"command": "true"}, "apps": ["GridCraft"]});
    assert!(Manifest::parse(&only_grid.to_string()).unwrap_err().contains("not WordCraft"));
    let any_case = json!({"id": "t.w", "name": "W", "run": {"command": "true"}, "apps": ["WordCraft"]});
    assert!(Manifest::parse(&any_case.to_string()).is_ok());
}

#[test]
fn homepages_must_be_web_links() {
    let with = |h: &str| json!({"id": "t.h", "name": "T", "run": {"command": "true"}, "homepage": h}).to_string();
    assert!(Manifest::parse(&with("https://example.org/docs")).is_ok());
    assert!(Manifest::parse(&with("http://example.org")).is_ok());
    for bad in ["file:///etc/passwd", "example.org", "https://exa mple.org"] {
        assert!(Manifest::parse(&with(bad)).is_err(), "{bad}");
    }
}

#[test]
fn command_replies_are_checked() {
    let r = parse_reply(
        json!({"commands": [{"command": "text.insert", "params": {"text": "x"}}, {"command": "format.bold"}], "message": "ok", "result": 3}),
    )
    .unwrap();
    assert_eq!(r.commands.len(), 2);
    assert_eq!(r.commands[1], ("format.bold".to_string(), serde_json::Value::Null));
    assert_eq!(r.message.as_deref(), Some("ok"));
    assert_eq!(r.result, json!(3));
    assert!(parse_reply(json!({"commands": "text.insert"})).is_err());
    assert!(parse_reply(json!({"commands": [{"params": {}}]})).is_err());
    assert!(parse_reply(json!({})).unwrap().commands.is_empty());
}

#[test]
fn discover_finds_plugin_folders() {
    let root = std::env::temp_dir().join(format!("wordcraft-discover-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for name in ["b", "a"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("plugin.json"), "{}").unwrap();
    }
    std::fs::create_dir_all(root.join("not-a-plugin")).unwrap();
    assert_eq!(crate::discover(std::slice::from_ref(&root)), vec![root.join("a"), root.join("b")]);
    assert_eq!(crate::discover(&[root.join("a")]), vec![root.join("a")]);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn turned_off_plugins_refuse_their_commands() {
    let m = Manifest::parse(r#"{"id": "t.off", "name": "Off", "run": {"command": "true"}, "commands": [{"id": "go", "label": "Go"}]}"#).unwrap();
    let p = crate::add(m, std::env::temp_dir());
    assert_eq!(p.status(), "ready");
    assert_eq!(crate::set_enabled("t.off", false), Ok(true));
    assert_eq!(crate::set_enabled("t.off", false), Ok(false));
    assert_eq!(p.status(), "off");
    assert!(crate::run_command("t.off", "go", json!({}), json!({})).unwrap_err().contains("turned off"));
    assert_eq!(p.info()["enabled"], false);
    assert_eq!(crate::set_enabled("t.off", true), Ok(true));
    assert!(crate::run_command("t.off", "nope", json!({}), json!({})).unwrap_err().contains("no command"));
    assert!(crate::unload("t.off"));
    assert!(crate::set_enabled("t.off", true).is_err());
}

mod install {
    use std::io::Write;

    use crate::install::{Repo, Version, parse_github_url, pick_asset, unpack};

    fn repo(owner: &str, name: &str, version: Version) -> Repo {
        Repo { owner: owner.into(), name: name.into(), version }
    }

    #[test]
    fn github_addresses_are_read() {
        for url in ["https://github.com/acme/fin-tools", "github.com/acme/fin-tools/", "http://www.github.com/acme/fin-tools.git", "acme/fin-tools"] {
            assert_eq!(parse_github_url(url), Ok(repo("acme", "fin-tools", Version::Latest)), "{url}");
        }
        assert_eq!(parse_github_url("https://github.com/acme/fin/releases/tag/v1.2.0"), Ok(repo("acme", "fin", Version::Release("v1.2.0".into()))));
        assert_eq!(parse_github_url("https://github.com/acme/fin/tree/feat/x"), Ok(repo("acme", "fin", Version::Tree("feat/x".into()))));
        assert_eq!(parse_github_url("https://github.com/acme/fin/releases/latest"), Ok(repo("acme", "fin", Version::Latest)));
        for bad in [
            "",
            "https://gitlab.com/acme/fin",
            "https://github.com/acme",
            "https://github.com/../fin",
            "https://github.com/acme/fin/blob/main/x",
            "https://github.com/acme/fin/tree/../../x",
            "ftp://github.com/acme/fin",
            "a/b/c",
        ] {
            assert!(parse_github_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_release_zip_for_this_platform_is_picked() {
        let names: Vec<String> = [
            "fin-1.0-linux-x86_64.zip",
            "fin-1.0-macos-x86_64.zip",
            "fin-1.0-macos-aarch64.zip",
            "fin-1.0-windows-x64.zip",
            "fin-1.0-macos-aarch64.tar.gz",
            "checksums.txt",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(pick_asset(&names, "macos", "aarch64"), Some(2));
        assert_eq!(pick_asset(&names, "macos", "x86_64"), Some(1));
        assert_eq!(pick_asset(&names, "windows", "x86_64"), Some(3));
        assert_eq!(pick_asset(&names, "linux", "aarch64"), None, "no build for it: install the source");
        let universal: Vec<String> = ["fin-darwin-universal.zip", "fin-linux-amd64.zip"].map(String::from).to_vec();
        assert_eq!(pick_asset(&universal, "macos", "aarch64"), Some(0));
        let any: Vec<String> = ["fin-plugin.zip", "fin-linux-arm64.zip"].map(String::from).to_vec();
        assert_eq!(pick_asset(&any, "macos", "aarch64"), Some(0), "a zip naming no platform suits every one");
        assert_eq!(pick_asset(&any, "linux", "aarch64"), Some(1), "this platform's build beats the generic zip");
        assert_eq!(pick_asset(&["darwin-tools-windows.zip".to_string()], "macos", "x86_64"), Some(0));
        assert_eq!(pick_asset(&[], "macos", "aarch64"), None);
    }

    fn zip(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut out);
        for (name, data, mode) in files {
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(*mode);
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        out.into_inner()
    }

    fn manifest(id: &str) -> Vec<u8> {
        serde_json::json!({"id": id, "name": "Fin", "run": {"command": "./run.sh"}}).to_string().into_bytes()
    }

    #[test]
    fn a_source_zip_unpacks_into_a_folder_named_after_the_plugin() {
        let root = std::env::temp_dir().join(format!("wordcraft-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let m = manifest("acme.fin");
        let bytes = zip(&[
            ("acme-fin-1a2b3c/README.md", b"readme", 0o644),
            ("acme-fin-1a2b3c/plugin.json", &m, 0o644),
            ("acme-fin-1a2b3c/run.sh", b"#!/bin/sh\n", 0o755),
            ("acme-fin-1a2b3c/lib/util.py", b"x = 1\n", 0o644),
            ("acme-fin-1a2b3c/examples/other/plugin.json", b"{}", 0o644),
            ("elsewhere/secret.txt", b"not part of the plugin", 0o644),
        ]);
        let dir = unpack(&bytes, &root).unwrap();
        assert_eq!(dir, root.join("acme.fin"));
        assert!(dir.join("plugin.json").is_file() && dir.join("lib/util.py").is_file() && dir.join("examples/other/plugin.json").is_file());
        assert!(!dir.join("elsewhere").exists() && !root.join("elsewhere").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
            assert_eq!(std::fs::metadata(dir.join("README.md")).unwrap().permissions().mode() & 0o777, 0o644);
        }
        crate::load(&dir).unwrap();
        assert!(crate::unload("acme.fin"));

        // Installing again replaces the earlier copy, leftovers included.
        std::fs::write(dir.join("stale.txt"), "old").unwrap();
        let again = unpack(&zip(&[("plugin.json", &m, 0o644), ("run.sh", b"new", 0o755)]), &root).unwrap();
        assert_eq!(again, dir);
        assert!(!dir.join("stale.txt").exists() && !dir.join("lib").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "no staging folders left behind");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unsafe_or_foreign_zips_are_refused() {
        let root = std::env::temp_dir().join(format!("wordcraft-unpack-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(unpack(b"not a zip", &root).unwrap_err().contains("not a zip"));
        assert!(unpack(&zip(&[("README.md", b"hi", 0o644)]), &root).unwrap_err().contains("plugin.json"));
        assert!(unpack(&zip(&[("plugin.json", &manifest(".."), 0o644)]), &root).is_err(), "an id can't climb out of the folder");
        assert!(unpack(&zip(&[("plugin.json", b"{", 0o644)]), &root).is_err());
        // Paths escaping the plugin folder are skipped, not written.
        let m = manifest("acme.safe");
        let dir = unpack(&zip(&[("plugin.json", &m, 0o644), ("../escaped.txt", b"x", 0o644), ("/abs.txt", b"x", 0o644)]), &root).unwrap();
        assert!(!root.join("escaped.txt").exists() && !std::env::temp_dir().join("escaped.txt").exists());
        assert!(dir.join("plugin.json").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }
}

mod office {
    use crate::office::{OfficeCommand, Permission, parse_manifest};
    use crate::{MenuKind, run_command};

    /// An add-in manifest written for these tests from the public schema: a default task pane, a
    /// button on the References tab, and a text menu item.
    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<OfficeApp xmlns="http://schemas.microsoft.com/office/appforoffice/1.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
  xmlns:bt="http://schemas.microsoft.com/office/officeappbasictypes/1.0" xsi:type="TaskPaneApp">
  <Id>0d5c7a1e-2222-4bcd-9eaf-0123456789ab</Id>
  <Version>1.0.0.0</Version>
  <ProviderName>Acme</ProviderName>
  <DefaultLocale>en-US</DefaultLocale>
  <DisplayName DefaultValue="Acme Cite"/>
  <Description DefaultValue="Citations from the web."/>
  <Hosts><Host Name="Document"/></Hosts>
  <Requirements><Sets DefaultMinVersion="1.1"><Set Name="WordApi" MinVersion="1.3"/></Sets></Requirements>
  <DefaultSettings><SourceLocation DefaultValue="https://acme.example/pane.html"/></DefaultSettings>
  <Permissions>ReadWriteDocument</Permissions>
  <VersionOverrides xmlns="http://schemas.microsoft.com/office/taskpaneappversionoverrides" xsi:type="VersionOverridesV1_0">
    <Hosts>
      <Host xsi:type="Document">
        <DesktopFormFactor>
          <ExtensionPoint xsi:type="PrimaryCommandSurface">
            <OfficeTab id="TabReferences">
              <Group id="Acme.Group"><Label resid="Group.Label"/>
                <Control xsi:type="Button" id="Acme.Pane"><Label resid="Button.Label"/>
                  <Action xsi:type="ShowTaskpane"><SourceLocation resid="Pane.Url"/></Action>
                </Control>
              </Group>
            </OfficeTab>
          </ExtensionPoint>
          <ExtensionPoint xsi:type="ContextMenu">
            <OfficeMenu id="ContextMenuText">
              <Control xsi:type="Button" id="Acme.Text"><Label resid="Text.Label"/>
                <Action xsi:type="ShowTaskpane"><SourceLocation resid="Pane.Url"/></Action>
              </Control>
            </OfficeMenu>
          </ExtensionPoint>
        </DesktopFormFactor>
      </Host>
    </Hosts>
    <Resources>
      <bt:Urls><bt:Url id="Pane.Url" DefaultValue="https://acme.example/pane.html"/></bt:Urls>
      <bt:ShortStrings>
        <bt:String id="Group.Label" DefaultValue="Acme"/>
        <bt:String id="Button.Label" DefaultValue="Cite"/>
        <bt:String id="Text.Label" DefaultValue="Cite This"/>
      </bt:ShortStrings>
    </Resources>
  </VersionOverrides>
</OfficeApp>"#;

    #[test]
    fn word_add_in_manifests_become_plugins_that_open_panes() {
        let m = parse_manifest(MANIFEST, "wordcraft").unwrap();
        assert_eq!(m.id, "office.0d5c7a1e-2222-4bcd-9eaf-0123456789ab");
        assert_eq!(m.apps, ["wordcraft"]);
        let o = m.office.as_ref().unwrap();
        assert_eq!(o.permissions, Permission::ReadWriteDocument);
        assert_eq!(o.requirements, [("WordApi".to_string(), "1.3".to_string())]);
        assert_eq!((m.ui().ribbon[0].tab.as_str(), m.ui().ribbon[0].group.as_str()), ("References", "Acme"));
        assert_eq!(m.ui().menus[0].menu, MenuKind::Text);
        assert!(parse_manifest(MANIFEST, "gridcraft").is_err(), "a Word add-in isn't Excel's");
        let id = m.id.clone();
        crate::add(m, std::env::temp_dir());
        let r = run_command(&id, "Acme.Text", serde_json::json!({}), serde_json::json!({})).unwrap();
        assert!(matches!(r.office, Some(OfficeCommand::ShowTaskpane { .. })));
        assert_eq!(crate::get(&id).unwrap().info()["kind"], "office");
        assert!(crate::unload(&id));
    }

    #[test]
    fn macro_plugins_are_listed_with_their_language() {
        let m = crate::Manifest::parse(r#"{"id": "t.vba", "name": "VBA", "macros": {"language": "VBA"}, "run": {"command": "./never"}}"#).unwrap();
        crate::add(m, std::env::temp_dir());
        assert!(crate::macro_plugins().contains(&("t.vba".to_string(), "VBA".to_string())));
        let e = crate::call_macros("t.vba", "commands/run", serde_json::json!({}), &mut |_, _| Err(crate::HostError::unknown())).unwrap_err();
        assert!(e.contains("doesn't run macros"), "{e}");
        crate::set_enabled("t.vba", false).unwrap();
        assert!(!crate::macro_plugins().iter().any(|(id, _)| id == "t.vba"));
        assert!(crate::unload("t.vba"));
    }
}

mod protocol_1_2 {
    use serde_json::json;

    use crate::dialog::{DialogKind, FieldKind, MAX_BUTTONS, MAX_FIELDS, MAX_ITEMS, MAX_PROMPT, MAX_TITLE, parse_dialog, parse_open};
    use crate::{Manifest, parse_reply};

    #[test]
    fn manifests_declare_events() {
        let m =
            Manifest::parse(r#"{"id": "t.ev", "name": "Ev", "run": {"command": "true"}, "events": ["documentSaved", "somethingLater"]}"#).unwrap();
        assert_eq!(m.events, ["documentSaved", "somethingLater"]);
        assert!(Manifest::parse(r#"{"id": "t.ev", "name": "Ev", "run": {"command": "true"}}"#).unwrap().events.is_empty());
        assert!(Manifest::parse(r#"{"id": "t.ev", "name": "Ev", "run": {"command": "true"}, "events": "documentSaved"}"#).is_err());
        let many = json!({"id": "t.ev", "name": "Ev", "run": {"command": "true"}, "events": vec!["documentSaved"; 65]});
        assert!(Manifest::parse(&many.to_string()).is_err());
        assert_eq!(crate::FEATURES, ["dialogs", "open", "events", "document"]);
    }

    #[test]
    fn list_dialogs_are_read_and_bounded() {
        let items: Vec<_> =
            (0..MAX_ITEMS + 5).map(|i| json!({"id": format!("/f{i}"), "label": "x".repeat(300), "detail": "Folder", "icon": "folder"})).collect();
        let d = parse_dialog(&json!({
            "title": "t".repeat(500), "kind": "list", "prompt": "p".repeat(5000), "items": items, "command": "browse",
            "params": {"account": "me"}, "multi": true,
            "buttons": (0..6).map(|i| json!({"label": format!("B{i}"), "command": "browse", "params": {"path": "/"}})).collect::<Vec<_>>(),
        }))
        .unwrap();
        assert_eq!(d.kind, DialogKind::List);
        assert_eq!(d.items.len(), MAX_ITEMS, "anything over a limit is cut");
        assert_eq!(d.items[0].label.chars().count(), 256);
        assert_eq!(d.title.chars().count(), MAX_TITLE);
        assert_eq!(d.prompt.chars().count(), MAX_PROMPT);
        assert_eq!(d.buttons.len(), MAX_BUTTONS);
        assert!(d.multi);
        assert_eq!(d.params, json!({"account": "me"}));
        assert_eq!(d.ok_label, None);
        assert_eq!(d.commands().count(), 1 + MAX_BUTTONS);
        // Text is cut at character boundaries.
        let d = parse_dialog(&json!({"kind": "message", "title": "é".repeat(300)})).unwrap();
        assert_eq!(d.title, "é".repeat(MAX_TITLE));
        assert_eq!(d.command, None, "a message may just close");
    }

    #[test]
    fn forms_are_read_with_typed_values() {
        let fields: Vec<_> = (0..MAX_FIELDS + 3).map(|i| json!({"id": format!("f{i}"), "label": "F"})).collect();
        let d = parse_dialog(&json!({"kind": "form", "command": "go", "fields": fields})).unwrap();
        assert_eq!(d.fields.len(), MAX_FIELDS);
        let d = parse_dialog(&json!({"kind": "form", "command": "go", "okLabel": "Sign in", "fields": [
            {"id": "user", "label": "User", "value": 12},
            {"id": "pw", "label": "Password", "kind": "secret"},
            {"id": "n", "label": "N", "kind": "number", "value": "3.5"},
            {"id": "ok", "label": "Remember", "kind": "checkbox", "value": true},
            {"id": "c", "label": "Server", "kind": "choice", "value": "nope", "options": [{"id": "a", "label": "A"}, {"id": "b"}]}
        ]}))
        .unwrap();
        let v: Vec<_> = d.fields.iter().map(|f| f.value.clone()).collect();
        assert_eq!(v, [json!("12"), json!(""), json!(3.5), json!(true), json!("a")]);
        assert_eq!(d.fields[1].kind, FieldKind::Secret);
        assert_eq!(d.fields[4].options[1].label, "b", "an option without a label shows its id");
        assert_eq!(d.ok_label.as_deref(), Some("Sign in"));
    }

    #[test]
    fn malformed_dialogs_fail_never_panic() {
        for bad in [
            json!("list"),
            json!({"title": "No kind"}),
            json!({"kind": "web", "command": "x"}),
            json!({"kind": "list", "items": []}),
            json!({"kind": "form", "fields": []}),
            json!({"kind": "list", "command": "x", "items": [{"label": "no id"}]}),
            json!({"kind": "list", "command": "x", "items": [{"id": 3, "label": "L"}]}),
            json!({"kind": "list", "command": "x", "items": "many"}),
            json!({"kind": "list", "command": "x y", "items": []}),
            json!({"kind": "list", "command": "x", "params": [1]}),
            json!({"kind": "list", "command": "x", "multi": "yes"}),
            json!({"kind": "form", "command": "x", "fields": [{"id": "a"}, {"id": "a"}]}),
            json!({"kind": "form", "command": "x", "fields": [{"id": "a", "kind": "slider"}]}),
            json!({"kind": "form", "command": "x", "fields": [{"id": "a", "kind": "choice"}]}),
            json!({"kind": "form", "command": "x", "fields": [{"id": "a", "kind": "checkbox", "value": "yes"}]}),
            json!({"kind": "form", "command": "x", "fields": [{"id": "a", "kind": "number", "value": "many"}]}),
            json!({"kind": "message", "buttons": [{"label": "Go"}]}),
            json!({"kind": "message", "title": 5}),
        ] {
            assert!(parse_dialog(&bad).is_err(), "{bad}");
        }
        // In a reply, a bad dialog fails the whole answer.
        assert!(parse_reply(json!({"dialog": {"kind": "nope"}})).is_err());
        assert!(parse_reply(json!({"dialog": null, "open": null})).is_ok());
    }

    #[test]
    fn open_requests_are_read() {
        let o = parse_open(&json!({"path": "/home/me/Budget.docx", "label": "L".repeat(300), "readOnly": true})).unwrap();
        assert_eq!(o.label.as_deref().map(|l| l.chars().count()), Some(MAX_TITLE));
        assert!(o.read_only);
        for bad in [
            json!({"label": "x"}),
            json!({"path": ""}),
            json!({"path": 3}),
            json!({"path": "a\u{0}b"}),
            json!({"path": "/a", "readOnly": 1}),
            json!("/a"),
        ] {
            assert!(parse_open(&bad).is_err(), "{bad}");
        }
        let r = parse_reply(json!({"open": {"path": "/x.docx"}, "message": "Opening"})).unwrap();
        assert_eq!(r.open.map(|o| o.path), Some("/x.docx".to_string()));
    }

    #[test]
    fn dialogs_may_only_run_the_plugins_commands() {
        // Checked against the manifest when the command answers (a fake that answers nothing
        // can't be run here; the engine's tests drive a real process).
        let d = parse_dialog(&json!({"kind": "list", "command": "browse", "items": [], "buttons": [{"label": "Up", "command": "up"}]})).unwrap();
        assert_eq!(d.commands().collect::<Vec<_>>(), ["browse", "up"]);
    }

    #[test]
    fn host_language_must_look_like_a_tag() {
        crate::set_host_language("pt-br");
        assert_eq!(crate::host_language().as_deref(), Some("pt-br"));
        crate::set_host_language("not a tag!");
        assert_eq!(crate::host_language().as_deref(), Some("pt-br"), "ignored");
    }
}
