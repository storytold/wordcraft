//! Fonts for text in SVG pictures: the faces WordCraft documents draw with (`wordcraft-fonts`,
//! with Word's font substitution), handed to usvg on demand. Nothing else is loaded: usvg's
//! database starts empty and system fonts are never scanned through it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use usvg::fontdb::{Database, ID, Source};
use usvg::{FontFamily, FontResolver, FontStyle};
use wordcraft_fonts::{FaceRef, FontDb, FontFace};

/// A loaded face's bytes, shared with `wordcraft-fonts` without a copy (faces are never unloaded).
struct FaceBytes(&'static [u8]);

impl AsRef<[u8]> for FaceBytes {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

/// Faces already in this parse's database, by `wordcraft-fonts` face id.
type Loaded = Arc<Mutex<HashMap<u32, ID>>>;

/// The face `face` in `db`, added the first time.
fn load(face: &'static FontFace, db: &mut Arc<Database>, loaded: &Loaded) -> Option<ID> {
    let mut map = loaded.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(id) = map.get(&face.id()) {
        return Some(*id);
    }
    let ids = Arc::make_mut(db).load_font_source(Source::Binary(Arc::new(FaceBytes(face.data()))));
    let id = ids.iter().copied().find(|id| db.face(*id).is_some_and(|f| f.index == face.index())).or_else(|| ids.first().copied())?;
    map.insert(face.id(), id);
    Some(id)
}

/// The document family a CSS generic family maps to (the faces Word would use).
fn generic(f: &FontFamily) -> &str {
    match f {
        FontFamily::Serif => "Times New Roman",
        FontFamily::SansSerif => "Arial",
        FontFamily::Monospace => "Courier New",
        FontFamily::Cursive => "Comic Sans MS",
        FontFamily::Fantasy => "Impact",
        FontFamily::Named(n) => n,
    }
}

/// usvg's font resolver over `wordcraft-fonts`: the first listed family that is installed or
/// bundled, else the first one through Word's substitution; characters it lacks fall back as in
/// documents.
pub(crate) fn resolver() -> FontResolver<'static> {
    let loaded: Loaded = Arc::default();
    let for_fallback = loaded.clone();
    FontResolver {
        select_font: Box::new(move |font, db| {
            let fonts = FontDb::global();
            let family = font
                .families()
                .iter()
                .map(generic)
                .find(|f| fonts.has_family(f))
                .or_else(|| font.families().first().map(generic))
                .unwrap_or("Times New Roman");
            let bold = font.weight() >= 600;
            let italic = font.style() != FontStyle::Normal;
            let face = wordcraft_fonts::word::resolve(family, bold, italic).face;
            load(face.get(), db, &loaded)
        }),
        select_fallback: Box::new(move |c, _used, db| {
            let face = FontDb::global().fallback_for(c, u32::MAX)?;
            load(FaceRef::of(&face).get(), db, &for_fallback)
        }),
    }
}
