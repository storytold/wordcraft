//! Bounded, non-materializing admission for the validator's TIFF pixel check.

use std::collections::{BTreeMap, HashMap, HashSet};

use craft_image_codecs::CodecError;
use craft_tiff::{ByteOrder, EntrySpan, FieldType, ParseOptions, Tiff, directory_entries, entry_span, tags};

/// What the codec will read for a complete TIFF, excluding zero padding after
/// its last referenced byte. Keep nonzero trailers for the codec's format checks.
pub(crate) fn payload(bytes: &[u8], mut reserve: impl FnMut(u64) -> Result<(), CodecError>) -> Result<Option<&[u8]>, CodecError> {
    observation_contract! {
        rules: "-";
        collections: "type.HashMap*2,type.HashSet*3";
        scope: "the TIFF directory graph within the part byte and metadata limits";
        loss: "header failure, malformed directory or metadata budget";
        fallback: "leave the image unassessed";
        package_loss: "independent";
    }

    let Some((order, big)) = Tiff::sniff(bytes) else {
        return Err(CodecError::Malformed { format: craft_image_codecs::Format::Tiff, message: "not a TIFF or BigTIFF header".into() });
    };
    let mut walk = DirectoryWalk {
        bytes,
        order,
        big,
        limits: ParseOptions::default(),
        parsed: HashMap::new(),
        on_path: HashSet::new(),
        admitted: HashSet::new(),
        followed: HashSet::new(),
        value_spans: HashMap::new(),
        entries: 0,
        end: if big { 16 } else { 8 },
        reserve: &mut reserve,
    };
    let malformed_root =
        || CodecError::Malformed { format: craft_image_codecs::Format::Tiff, message: "the first TIFF directory is missing or cut off".into() };
    let first = walk.offset(if big { 8 } else { 4 }).filter(|&offset| offset != 0).ok_or_else(malformed_root)?;
    directory_entries(bytes, first, order, big, walk.limits.max_entries).map_err(|_| malformed_root())?;
    if !walk.chain(first, 0, true)? {
        return Ok(None);
    }
    let end = usize::try_from(walk.end).unwrap_or(usize::MAX).min(bytes.len());
    let suffix = bytes.get(end..).unwrap_or_default();
    let end = suffix.iter().rposition(|&byte| byte != 0).map_or(end, |last| end.saturating_add(last + 1));
    Ok(bytes.get(..end))
}

struct DirectoryWalk<'a, R> {
    bytes: &'a [u8],
    order: ByteOrder,
    big: bool,
    limits: ParseOptions,
    /// Directories already read and charged, with their tables and next pointers.
    parsed: HashMap<u64, (BTreeMap<u16, EntrySpan>, u64)>,
    /// Directories on the current traversal path. Reaching one again is a true cycle.
    on_path: HashSet<u64>,
    /// Directories whose pointers were all admitted. Sharing one costs nothing.
    admitted: HashSet<u64>,
    /// Directories whose next-directory chain was admitted in full.
    followed: HashSet<u64>,
    value_spans: HashMap<u64, u64>,
    entries: usize,
    end: u64,
    reserve: &'a mut R,
}

impl<R: FnMut(u64) -> Result<(), CodecError>> DirectoryWalk<'_, R> {
    fn offset(&self, at: usize) -> Option<u64> {
        observation_contract! {
            rules: "-";
            collections: "field.bytes*2";
            scope: "one classic TIFF or BigTIFF offset field";
            loss: "offset bytes outside the payload";
            fallback: "return no offset";
            package_loss: "independent";
        }

        if self.big { self.order.read_u64(self.bytes, at) } else { self.order.read_u32(self.bytes, at).map(u64::from) }
    }

    /// Admit the directory graph reachable from `start`. The outcome depends only on that graph:
    /// a directory reached again after it was admitted is free, and only a directory reached
    /// from itself while still open is a cycle. The directory and entry budgets bound the walk.
    fn chain(&mut self, start: u64, depth: usize, follow_next: bool) -> Result<bool, CodecError> {
        observation_contract! {
            rules: "-";
            collections: "field.followed*1,field.on_path*1,type.Vec*1";
            scope: "the directory path of one TIFF chain walk";
            loss: "directory cycle or an unadmitted directory";
            fallback: "leave the image unassessed";
            package_loss: "independent";
        }

        if depth > self.limits.max_depth {
            return Ok(false);
        }
        let mut path = Vec::new();
        let outcome = self.chain_path(start, depth, follow_next, &mut path);
        for opened in &path {
            self.on_path.remove(opened);
        }
        let admitted = outcome?;
        if admitted && follow_next {
            self.followed.extend(path);
        }
        Ok(admitted)
    }

    fn chain_path(&mut self, mut offset: u64, depth: usize, follow_next: bool, path: &mut Vec<u64>) -> Result<bool, CodecError> {
        observation_contract! {
            rules: "-";
            collections: "field.admitted*1,field.followed*1,field.on_path*1,type.Vec*1";
            scope: "one TIFF directory chain under directory, depth and work bounds";
            loss: "directory cycle, malformed directory or metadata budget";
            fallback: "leave the image unassessed";
            package_loss: "independent";
        }

        while offset != 0 {
            if follow_next && self.followed.contains(&offset) {
                break;
            }
            if !self.on_path.insert(offset) {
                return Ok(false);
            }
            path.push(offset);
            let Some((tags, next)) = self.directory(offset)? else { return Ok(false) };
            if self.admitted.insert(offset) {
                self.record_image_data(&tags)?;
                if !self.children(&tags, depth)? {
                    return Ok(false);
                }
            }
            offset = if follow_next { next } else { 0 };
        }
        Ok(true)
    }

    fn directory(&mut self, offset: u64) -> Result<Option<(BTreeMap<u16, EntrySpan>, u64)>, CodecError> {
        observation_contract! {
            rules: "-";
            collections: "field.bytes*2,field.declared*4,field.entries*3,field.parsed*3,field.value_spans*1,type.BTreeMap*2";
            scope: "TIFF directory table and physical value spans, bounded before directory or decoder allocations";
            loss: "malformed directory, incomplete value or pointer, directory cycle, work/byte budget";
            fallback: "do not assess pixels from an incomplete preflight";
            package_loss: "independent";
        }

        if let Some(known) = self.parsed.get(&offset) {
            return Ok(Some(known.clone()));
        }
        if self.parsed.len() >= self.limits.max_ifds {
            return Ok(None);
        }
        (self.reserve)(512)?;
        let Ok(dir) = directory_entries(self.bytes, offset, self.order, self.big, self.limits.max_entries) else { return Ok(None) };
        self.entries = self.entries.saturating_add(dir.read());
        if dir.declared() != dir.read() as u64 || self.entries > self.limits.max_entries_total {
            return Ok(None);
        }
        // Bound the indexes here and the directory inventory in the codec. Use actual
        // directory and entry counts, including overlaps, independently of payload size.
        (self.reserve)(160 * dir.declared())?;
        let mut tags = BTreeMap::new();
        for pos in dir.positions() {
            let Some(value) = entry_span(self.bytes, pos, self.order, self.big, 0) else { return Ok(None) };
            if tags.insert(value.tag, value).is_some() {
                return Ok(None);
            }
            let held = self.value_spans.entry(value.start).or_default();
            // Alias ranges with the same origin share physical bytes. A wider view
            // extends the reservation by just the bytes not seen at that offset.
            (self.reserve)(value.len.saturating_sub(*held).saturating_mul(8))?;
            *held = (*held).max(value.len);
            self.end = self.end.max(value.start.saturating_add(value.len));
        }
        if let Some(extra) = tags.get(&tags::EXTRA_SAMPLES) {
            // The codec describes each page separately and materializes its ExtraSamples.
            // Charge those vectors per page when several pages alias the same value bytes.
            let samples = tags.get(&tags::SAMPLES_PER_PIXEL).and_then(|span| self.uint(*span, 0)).unwrap_or(1).min(u64::from(u16::MAX));
            (self.reserve)(16 * extra.count.min(samples))?;
        }
        let table = if self.big { 8 + 20 * dir.declared() } else { 2 + 12 * dir.declared() };
        let Some(next_at) = offset.checked_add(table) else { return Ok(None) };
        let Some(next) = usize::try_from(next_at).ok().and_then(|at| self.offset(at)) else { return Ok(None) };
        self.end = self.end.max(next_at.saturating_add(if self.big { 8 } else { 4 }));
        self.parsed.insert(offset, (tags.clone(), next));
        Ok(Some((tags, next)))
    }

    fn record_image_data(&mut self, tags: &BTreeMap<u16, EntrySpan>) -> Result<(), CodecError> {
        observation_contract! {
            rules: "-";
            collections: "type.BTreeMap*1";
            scope: "ranges named by the directory’s checked offset and length tables";
            loss: "work budget or unreadable range";
            fallback: "preserve decoder handling of truncated image data";
            package_loss: "independent";
        }

        for (offsets, sizes) in [
            (tags::STRIP_OFFSETS, tags::STRIP_BYTE_COUNTS),
            (tags::TILE_OFFSETS, tags::TILE_BYTE_COUNTS),
            (tags::JPEG_INTERCHANGE_FORMAT, tags::JPEG_INTERCHANGE_FORMAT_LENGTH),
            (288, 289), // FreeOffsets and FreeByteCounts.
        ] {
            let (Some(offsets), Some(sizes)) = (tags.get(&offsets), tags.get(&sizes)) else { continue };
            let chunks = offsets.count.min(sizes.count);
            (self.reserve)(chunks.saturating_mul(8))?;
            for index in 0..chunks {
                if let (Some(start), Some(len)) = (self.uint(*offsets, index), self.uint(*sizes, index))
                    && len > 0
                {
                    // Keep any bytes the decoder may read, including a short last
                    // strip. The codec decides whether that strip is truncated.
                    self.end = self.end.max(start.saturating_add(len));
                }
            }
        }
        Ok(())
    }

    fn children(&mut self, tags: &BTreeMap<u16, EntrySpan>, depth: usize) -> Result<bool, CodecError> {
        observation_contract! {
            rules: "-";
            collections: "type.BTreeMap*1";
            scope: "SubIFD, Exif, GPS and Interop pointers under directory, depth and work bounds";
            loss: "unreadable child pointer or incomplete child directory";
            fallback: "withhold the complete directory inventory";
            package_loss: "independent";
        }

        for tag in [tags::SUB_IFDS, tags::EXIF_IFD, tags::GPS_IFD, tags::INTEROP_IFD] {
            let Some(children) = tags.get(&tag) else { continue };
            if tag != tags::SUB_IFDS && children.count != 1 {
                return Ok(false);
            }
            (self.reserve)(children.count.saturating_mul(8))?;
            for index in 0..children.count {
                let Some(child) = self.uint(*children, index) else { return Ok(false) };
                if child == 0 || !self.chain(child, depth + 1, tag == tags::SUB_IFDS)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn uint(&self, span: EntrySpan, index: u64) -> Option<u64> {
        observation_contract! {
            rules: "-";
            collections: "field.bytes*1";
            scope: "one checked unsigned value from a TIFF tag span";
            loss: "unreadable value or incompatible TIFF field type";
            fallback: "return no value";
            package_loss: "independent";
        }

        span_uint(self.bytes, self.order, span, index)
    }
}

fn span_uint(bytes: &[u8], order: ByteOrder, span: EntrySpan, index: u64) -> Option<u64> {
    let at = usize::try_from(span.start.checked_add(index.checked_mul(span.field_type.size() as u64)?)?).ok()?;
    match span.field_type {
        FieldType::Byte => bytes.get(at).copied().map(u64::from),
        FieldType::Short => order.read_u16(bytes, at).map(u64::from),
        FieldType::Long | FieldType::Ifd => order.read_u32(bytes, at).map(u64::from),
        FieldType::Long8 | FieldType::Ifd8 => order.read_u64(bytes, at),
        _ => None,
    }
}

/// Stored strip or tile dimensions of the selected decode page, including edge tile padding.
pub(crate) fn chunk_dimensions(bytes: &[u8], page: &craft_image_codecs::TiffPage) -> (u64, u64) {
    let w = u64::from(page.width);
    let h = u64::from(page.height);
    let Some((order, big)) = Tiff::sniff(bytes) else { return (w, h) };
    let Ok(dir) = directory_entries(bytes, page.ifd_offset, order, big, ParseOptions::default().max_entries) else { return (w, h) };
    let mut width = w;
    let mut height = h;
    for pos in dir.positions() {
        let Some(span) = entry_span(bytes, pos, order, big, 0) else { continue };
        match span.tag {
            tags::TILE_WIDTH if page.tiled => width = span_uint(bytes, order, span, 0).unwrap_or(w),
            tags::TILE_LENGTH if page.tiled => height = span_uint(bytes, order, span, 0).unwrap_or(h),
            tags::ROWS_PER_STRIP if !page.tiled => height = span_uint(bytes, order, span, 0).unwrap_or(h).min(h),
            _ => {}
        }
    }
    (width, height)
}
