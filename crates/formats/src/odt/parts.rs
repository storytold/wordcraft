//! Package parts WordCraft doesn't read (ODF 1.3 Part 2 §3: any file a package lists), kept as
//! they are and written back: macros and dialogs (`Basic/`, `Dialogs/`, `Scripts/`),
//! LibreOffice's interface configuration (`Configurations2/`), metadata (`manifest.rdf`) and
//! files of other applications.
//!
//! Not kept: what WordCraft reads and writes itself (content, styles, meta, settings, pictures,
//! embedded objects), the thumbnail (it would show the document as it was), and digital
//! signatures (`META-INF/*signatures*.xml`): they sign the package as it was, so a saved, changed
//! document can't carry them.

use std::sync::Arc;

use wordcraft_doc::Document;

/// `Document::passthrough` key prefix of a kept part (its path follows), and the key of the list
/// of kept paths with their media types (`path\tmedia type` lines; directories end in `/`).
pub(super) const PREFIX: &str = "odf-part/";
pub(super) const LIST: &str = "odf-parts";

/// Most parts kept, and most bytes in all.
const MAX_PARTS: usize = 1000;
const MAX_BYTES: u64 = 64 << 20;

/// Whether package path `name` is one WordCraft reads or writes itself, or drops.
fn own(name: &str, manifest: &std::collections::HashMap<String, String>) -> bool {
    const OWN: [&str; 6] = ["mimetype", "content.xml", "styles.xml", "meta.xml", "settings.xml", "META-INF/manifest.xml"];
    if OWN.contains(&name) || name.ends_with('/') {
        return true;
    }
    if ["Pictures/", "Thumbnails/", "ObjectReplacements/"].iter().any(|d| name.starts_with(d)) {
        return true;
    }
    if name.starts_with("META-INF/") && name.contains("signatures") {
        return true;
    }
    // An embedded object's files: its directory is listed as an OpenDocument.
    let mut dir = name;
    while let Some((up, _)) = dir.rsplit_once('/') {
        if manifest
            .get(&format!("{up}/"))
            .is_some_and(|m| m.starts_with("application/vnd.oasis.opendocument.") || m.starts_with("application/vnd.sun.xml.math"))
        {
            return true;
        }
        dir = up;
    }
    false
}

/// Whether `name` is a path safe to write back: relative, without `.` or `..` steps or empty
/// ones, and of no more than 512 bytes.
fn safe(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 512
        && !name.starts_with('/')
        && !name.contains('\\')
        && name.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
        && !name.chars().any(char::is_control)
}

/// The parts of a package WordCraft doesn't read itself: each file's path and bytes, and the
/// list of their paths with media types (see [`LIST`]).
#[derive(Default)]
pub(super) struct Kept {
    files: Vec<(String, Vec<u8>)>,
    list: String,
}

impl Kept {
    /// Keeps them in `doc`, for the next save.
    pub(super) fn store(self, doc: &mut Document) {
        for (name, bytes) in self.files {
            doc.passthrough.insert(format!("{PREFIX}{name}"), Arc::new(bytes));
        }
        if !self.list.is_empty() {
            doc.passthrough.insert(LIST.into(), Arc::new(self.list.into_bytes()));
        }
    }
}

/// Reads the parts of package `z` (entries `entries`) WordCraft doesn't read itself, with their
/// media types from `manifest`.
pub(super) fn read(
    z: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    entries: &[String],
    manifest: &std::collections::HashMap<String, String>,
) -> Kept {
    use std::io::Read;
    let mut kept = Kept::default();
    let mut budget = MAX_BYTES;
    let mut dirs = std::collections::BTreeSet::new();
    let clean = |m: &str| m.replace(['\t', '\n', '\r'], " ");
    for name in entries {
        if kept.files.len() >= MAX_PARTS || own(name, manifest) || !safe(name) {
            continue;
        }
        let Ok(f) = z.by_name(name) else { continue };
        if f.size() > budget {
            continue;
        }
        let mut bytes = Vec::new();
        if f.take(budget).read_to_end(&mut bytes).is_err() {
            continue;
        }
        budget = budget.saturating_sub(bytes.len() as u64);
        let mime = manifest.get(name.as_str()).map_or("", String::as_str);
        kept.list.push_str(&format!("{name}\t{}\n", clean(mime)));
        // The directories it is in that the manifest lists (a Basic library, a configuration).
        let mut dir = name.as_str();
        while let Some((up, _)) = dir.rsplit_once('/') {
            let d = format!("{up}/");
            if let Some(m) = manifest.get(&d) {
                dirs.insert((d, clean(m)));
            }
            dir = up;
        }
        kept.files.push((name.clone(), bytes));
    }
    for (d, m) in dirs {
        kept.list.push_str(&format!("{d}\t{m}\n"));
    }
    kept
}

/// The parts kept in `doc` to write back: (path, bytes) of each file, and (path, media type) of
/// each manifest entry (directories too). Paths WordCraft writes itself are left out.
pub(super) fn kept(doc: &Document) -> (Vec<(String, Arc<Vec<u8>>)>, Vec<(String, String)>) {
    let mut files = Vec::new();
    let mut entries = Vec::new();
    let Some(list) = doc.passthrough.get(LIST).and_then(|l| std::str::from_utf8(l).ok()) else { return (files, entries) };
    let empty = std::collections::HashMap::new();
    for line in list.lines().take(MAX_PARTS * 2) {
        let (path, mime) = line.split_once('\t').unwrap_or((line, ""));
        let dir = path.ends_with('/');
        let name = path.trim_end_matches('/');
        if !safe(name) || own(path, &empty) && !dir {
            continue;
        }
        if dir {
            entries.push((path.to_owned(), mime.to_owned()));
        } else if let Some(b) = doc.passthrough.get(&format!("{PREFIX}{path}")) {
            // Every file has its entry (ODF 1.3 Part 2 §4.3; LibreOffice refuses a package with a
            // file its manifest doesn't list), its media type empty when the file's was.
            files.push((path.to_owned(), b.clone()));
            entries.push((path.to_owned(), mime.to_owned()));
        }
    }
    (files, entries)
}
