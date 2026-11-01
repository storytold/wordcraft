//! Checks on `manifest.webmanifest`, which the browser reads and no Rust code does.

#[cfg(test)]
mod tests {
    const MANIFEST: &str = include_str!("../manifest.webmanifest");

    fn manifest() -> serde_json::Value {
        serde_json::from_str(MANIFEST).unwrap()
    }

    /// The installed app is offered for these files in "Open with"; one the engine can't open
    /// would arrive and fail with an error.
    #[test]
    fn every_handled_extension_is_one_the_engine_opens() {
        let handlers = manifest()["file_handlers"].as_array().unwrap().clone();
        assert!(!handlers.is_empty());
        for handler in handlers {
            for exts in handler["accept"].as_object().unwrap().values() {
                for ext in exts.as_array().unwrap() {
                    let ext = ext.as_str().unwrap();
                    let bare = ext.strip_prefix('.').unwrap_or_else(|| panic!("{ext} must start with a dot"));
                    assert!(wordcraft_engine::io::OPEN_EXTS.contains(&bare), "{ext} is handled but doesn't open");
                }
            }
        }
    }

    /// The site runs under any prefix (`public_url = "./"`): an absolute URL, or an `id` (which
    /// resolves against the origin), would make copies under different prefixes one app.
    #[test]
    fn urls_stay_under_the_hosting_prefix() {
        let m = manifest();
        assert!(m.get("id").is_none());
        assert_eq!(m["start_url"], "./");
        assert_eq!(m["scope"], "./");
        let urls =
            m["icons"].as_array().unwrap().iter().map(|i| &i["src"]).chain(m["file_handlers"].as_array().unwrap().iter().map(|h| &h["action"]));
        for url in urls {
            let url = url.as_str().unwrap();
            assert!(!url.starts_with('/') && !url.contains(':') && !url.contains(".."), "{url}");
        }
    }
}
