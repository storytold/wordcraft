//! Plug-ins: the `plugin.*` commands (list, install from a file, base64 data, a GitHub repository
//! or a catalog's package, update, remove and run plug-ins, and the catalogs they come from) and
//! Insert › Add-ins › Get Add-ins, which opens the Plug-ins window in the app. The runtime and the
//! rules a run follows are in [`crate::plugin`] (RFC-0046).

use serde_json::{Value, json};

use crate::plugin::{self, Request, bad};
use crate::{CmdResult, CommandSpec, Session, p};

/// Where the commands live: Word's Add-ins group on the Insert tab.
const LOC: &str = "Insert › Add-ins";

fn list(s: &mut Session, _p: &Value) -> CmdResult {
    let plugins: Vec<Value> = s.plugins.list().iter().map(plugin::InstalledPlugin::describe).collect();
    let last = s.plugins.last.as_ref().map(|(id, command, _)| json!({"id": id, "command": command}));
    Ok(json!({
        "plugins": plugins,
        "problems": s.plugins.problems(),
        "catalogs": s.plugins.prefs.catalogs,
        "stored": s.plugins.has_store(),
        "last": last,
    }))
}

fn install(s: &mut Session, p: &Value) -> CmdResult {
    let cmd = "plugin.install";
    let request = plugin::install_request(p);
    let wanted = p::str(p, "sha256").map(str::to_string);
    let c = match wanted.as_deref().and_then(|h| plugin::take_held(s, h, &request)) {
        Some(c) => c,
        None => plugin::obtain(s, cmd, &Request::parse(cmd, p)?)?,
    };
    let (_, mut info) = plugin::review(s, cmd, &c)?;
    let sha256 = info["sha256"].as_str().unwrap_or_default().to_string();
    if let Some(w) = &wanted
        && !w.eq_ignore_ascii_case(&sha256)
    {
        return Err(bad(cmd, format!("the bundle's SHA-256 is {sha256}, not the {w} that was reviewed: review it again")));
    }
    if p::bool(p, "review").unwrap_or(false) {
        plugin::hold(s, request, c);
        info["installed"] = json!(false);
        return Ok(info);
    }
    plugin::install(s, cmd, c)?;
    info["installed"] = json!(true);
    s.status = format!("Installed {}", info["name"].as_str().unwrap_or_default());
    Ok(info)
}

fn update(s: &mut Session, p: &Value) -> CmdResult {
    let cmd = "plugin.update";
    let only = p::str(p, "id");
    if let Some(id) = only
        && s.plugins.get(id).is_none()
    {
        return Err(bad(cmd, format!("no plug-in `{id}` is installed")));
    }
    let apply = p::bool(p, "apply").unwrap_or(false);
    let targets: Vec<plugin::InstalledPlugin> =
        s.plugins.list().iter().filter(|x| only.is_none_or(|id| x.manifest.id.eq_ignore_ascii_case(id))).cloned().collect();
    let (mut updates, mut updated, mut errors) = (vec![], vec![], vec![]);
    for old in targets {
        let id = old.manifest.id.clone();
        let (params, available) = match plugin::with_fetch(s, |f| plugin::update_for(f, &old)) {
            Ok(Some(found)) => found,
            Ok(None) => continue,
            Err(e) => {
                errors.push(json!({"id": id, "error": e}));
                continue;
            }
        };
        let mut entry = json!({"id": id, "installed": old.manifest.version, "available": available, "install": params});
        if apply {
            match apply_update(s, cmd, &params) {
                Ok(true) => updated.push(json!(id)),
                Ok(false) => entry["permissionsChanged"] = json!(true),
                Err(e) => errors.push(json!({"id": id, "error": e.to_string()})),
            }
        }
        updates.push(entry);
    }
    Ok(json!({"updates": updates, "updated": updated, "errors": errors}))
}

/// Install the update `params` name, unless it asks for other permissions (then it waits for its
/// confirmation, `plugin.install`): whether it was installed.
fn apply_update(s: &mut Session, cmd: &str, params: &Value) -> Result<bool, crate::CmdError> {
    let c = plugin::obtain(s, cmd, &Request::parse(cmd, params)?)?;
    let (_, info) = plugin::review(s, cmd, &c)?;
    if info["permissionsChanged"].as_bool().unwrap_or(true) {
        return Ok(false);
    }
    plugin::install(s, cmd, c).map(|_| true)
}

fn remove(s: &mut Session, p: &Value) -> CmdResult {
    let cmd = "plugin.remove";
    let id = p::str(p, "id").ok_or_else(|| bad(cmd, "`id` required (see plugin.list)"))?.to_string();
    if !plugin::remove(s, cmd, &id)? {
        return Err(bad(cmd, format!("no plug-in `{id}` is installed")));
    }
    Ok(json!({"removed": id}))
}

fn run(s: &mut Session, p: &Value) -> CmdResult {
    let id = p::str(p, "id").ok_or_else(|| bad("plugin.run", "`id` required (see plugin.list)"))?.to_string();
    let command = p::str(p, "command").map(str::to_string);
    let fuel = p.get("fuel").and_then(Value::as_u64);
    plugin::run(s, &id, command.as_deref(), p.get("params").unwrap_or(&Value::Null), fuel)
}

fn catalog_list(s: &mut Session, p: &Value) -> CmdResult {
    let urls = match p::str(p, "url") {
        Some(url) => vec![url.to_string()],
        None => s.plugins.prefs.catalogs.clone(),
    };
    let (mut packages, mut warnings) = (vec![], vec![]);
    for url in &urls {
        match plugin::with_fetch(s, |f| plugin::catalog_packages_with(f, url)) {
            Ok((found, w)) => {
                packages.extend(found);
                warnings.extend(w.into_iter().map(|w| format!("{url}: {w}")));
            }
            Err(e) => warnings.push(format!("{url}: {e}")),
        }
    }
    for pkg in &mut packages {
        let installed = pkg["id"].as_str().and_then(|id| s.plugins.get(id)).map(|p| p.manifest.version.clone());
        pkg["installed"] = json!(installed);
    }
    Ok(json!({"catalogs": urls, "packages": packages, "warnings": warnings}))
}

fn catalog_add(s: &mut Session, p: &Value) -> CmdResult {
    let cmd = "plugin.catalog.add";
    let url = p::str(p, "url").map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| bad(cmd, "`url` required"))?.to_string();
    if p::bool(p, "remove").unwrap_or(false) {
        s.plugins.prefs.catalogs.retain(|u| *u != url);
    } else if !s.plugins.prefs.catalogs.contains(&url) {
        plugin::check_catalog_url(&url).map_err(|e| bad(cmd, e))?;
        if s.plugins.prefs.catalogs.len() >= plugin::MAX_CATALOGS {
            return Err(bad(cmd, format!("at most {} catalogs", plugin::MAX_CATALOGS)));
        }
        s.plugins.prefs.catalogs.push(url);
    }
    s.plugins.save_prefs();
    Ok(json!({"catalogs": s.plugins.prefs.catalogs}))
}

/// Insert › Add-ins › Get Add-ins: the plug-ins installed and the catalogs to get more from (the
/// app opens the Plug-ins window on its catalogs). Reads nothing over the network.
fn get_addins(s: &mut Session, _p: &Value) -> CmdResult {
    let mut r = list(s, &Value::Null)?;
    r["pending"] = json!("plugins");
    Ok(r)
}

/// Only `plugin.run` is an undo step: it records one, named after the plug-in, for the commands
/// it runs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("addins.get", "Get Add-ins", LOC, get_addins)
            .params(r#"{} → {"plugins", "catalogs", "stored", "problems", "last"} (from the ribbon, the app shows the Plug-ins window; see plugin.catalog.list for a catalog's packages)"#)
            .pure(),
        CommandSpec::new("plugin.list", "Plug-ins", LOC, list)
            .params(r#"{} → {"plugins": [{"id", "name", "version", "kind", "permissions", "commands", "source", "sha256", "stored"}], "problems", "catalogs", "stored" (kept in the Plug-ins folder; else this session only), "last"}"#)
            .pure(),
        CommandSpec::new("plugin.install", "Install Plug-in", LOC, install)
            .params(r#"{"path": .craftplugin file} | {"data": base64 .craftplugin, "name"?} | {"repo": "owner/repo" or "owner/repo@tag" (its release's .craftplugin, checked against SHA256SUMS)} | {"catalog": index.json URL, "id", "version"? (default: the newest for WordCraft)}; each with "review"?: bool (check and describe it without installing: the confirmation) and "sha256"? (install only that bundle, after a review, without downloading it again) → {"id", "name", "version", "kind", "permissions", "commands", "source", "sha256", "inCatalog", "replaces", "permissionsChanged", "stored", "installed"}"#)
            .pure(),
        CommandSpec::new("plugin.update", "Update Plug-ins", LOC, update)
            .params(r#"{"id"?, "apply"?: bool (install the updates that ask for no other permissions)} — checks each plug-in's catalog or repository → {"updates": [{"id", "installed", "available", "install" (plugin.install params), "permissionsChanged"?}], "updated", "errors"}"#)
            .pure(),
        CommandSpec::new("plugin.remove", "Remove Plug-in", LOC, remove).params(r#"{"id"} — removes it and its settings → {"removed"}"#).pure(),
        CommandSpec::new("plugin.run", plugin::RUN_LABEL, LOC, run)
            .params(r#"{"id", "command"? (default: the first), "params"? (the command's parameters), "fuel"? (a lower instruction budget)} — runs the commands it calls on the active document as one undo step named after the plug-in, rolled back when it fails; it sees the selection (`selection` in its input) → {"plugin", "command", "label", "output", "notes", "logs", "calls", "fuel", "panel"}"#),
        CommandSpec::new("plugin.catalog.list", "Plug-in Catalogs", LOC, catalog_list)
            .params(r#"{"url"?} — the packages WordCraft can run in that catalog, or in every catalog added → {"catalogs", "packages": [{"catalog", "id", "name", "version", "author", "description", "license", "repo", "kind", "sha256", "installed"}], "warnings"}"#)
            .pure(),
        CommandSpec::new("plugin.catalog.add", "Add Plug-in Catalog", LOC, catalog_add)
            .params(r#"{"url": https:// on GitHub or file:// index.json, "remove"?: bool} → {"catalogs"}"#)
            .pure(),
    ]
}
