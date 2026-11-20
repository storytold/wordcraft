//! Bounded, non-materializing admission for the validator's TIFF pixel check.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque, hash_map::Entry};

use craft_image_codecs::CodecError;
use craft_tiff::{ByteOrder, EntrySpan, FieldType, ParseOptions, Tiff, directory_entries, entry_span, tags};

/// A bound of the validator's own admission (depth, directory count, entry count) was reached: the graph was not
/// assessed, which is no evidence that it is malformed, so it is a lost assessment (`LimitExceeded`), never a grade.
fn bound(what: &str) -> CodecError {
    CodecError::LimitExceeded(format!("TIFF directory {what} bound"))
}

/// What the codec will read for a complete TIFF: everything up to its last referenced byte (bytes after that are
/// padding of any content and any length), with the directories of its main chain in order: the first directory and
/// each `next` target. This is the ONE authority for which directories are main-chain pages; the page the validator
/// assesses is chosen from it (a directory first reached as a SubIFD stays a main-chain page).
pub(crate) fn payload(bytes: &[u8], mut reserve: impl FnMut(u64) -> Result<(), CodecError>) -> Result<Option<(&[u8], Vec<u64>)>, CodecError> {
    observation_contract! {
        rules: "-";
        collections: "type.HashMap*2,type.Vec*1";
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
        value_spans: HashMap::new(),
        entries: 0,
        end: if big { 16 } else { 8 },
        reserve: &mut reserve,
    };
    let malformed_root =
        || CodecError::Malformed { format: craft_image_codecs::Format::Tiff, message: "the first TIFF directory is missing or cut off".into() };
    let first = walk.offset(if big { 8 } else { 4 }).filter(|&offset| offset != 0).ok_or_else(malformed_root)?;
    directory_entries(bytes, first, order, big, walk.limits.max_entries).map_err(|_| malformed_root())?;
    let Some(main_chain) = walk.admit(first)? else {
        return Ok(None);
    };
    // Every range the codec reads is named by the walk, so what follows its end is padding of any content.
    let end = usize::try_from(walk.end).unwrap_or(usize::MAX).min(bytes.len());
    Ok(bytes.get(..end).map(|payload| (payload, main_chain)))
}

struct DirectoryWalk<'a, R> {
    bytes: &'a [u8],
    order: ByteOrder,
    big: bool,
    limits: ParseOptions,
    /// Directories already read and charged, with their tables and next pointers.
    parsed: HashMap<u64, (BTreeMap<u16, EntrySpan>, u64)>,
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

    /// The main chain of the directory graph reachable from `first` when it can be admitted: every directory reads,
    /// the directory, entry and depth budgets hold, and no directory is reachable from itself. `None` when the graph is
    /// malformed or cyclic; a budget that does not hold is `Err(LimitExceeded)`, a lost assessment.
    ///
    /// This is the ONLY cycle computation. The graph's edges are every SubIFD, Exif, GPS and Interop
    /// pointer, and the `next` pointer of every directory that is read as a chain member (the first
    /// directory, a `next` target or a SubIFD target; Exif, GPS and Interop directories are single
    /// directories whose `next` nothing follows). Reaching a directory again is free whatever route
    /// it took, deeper routes included; only a directory that reaches itself rejects. Reachability is explored first, so
    /// the verdict depends on the graph and not on the order its edges are visited in.
    fn admit(&mut self, first: u64) -> Result<Option<Vec<u64>>, CodecError> {
        observation_contract! {
            rules: "-";
            collections: "field.parsed*1,type.HashMap*8,type.HashSet*1,type.Vec*3";
            scope: "the TIFF directory graph within the directory, entry, depth and work bounds";
            loss: "directory cycle, malformed directory or metadata budget";
            fallback: "leave the image unassessed";
            package_loss: "independent";
        }

        // Phase 1: reach every directory once per role (chain member or single), nearest first.
        // `next` keeps the depth (a chain is one level); a pointer to a child adds one.
        let mut queue = VecDeque::from([(first, true, 0usize)]);
        let mut visited = HashSet::new();
        // The depth at which each directory was first reached in any role: the queue pops nearest first, so it is
        // the directory's minimum depth, and the depth bound is decided on that alone.
        let mut nearest: HashMap<u64, usize> = HashMap::new();
        let mut children_of: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut chain_next: HashMap<u64, u64> = HashMap::new();
        while let Some((offset, chain, depth)) = queue.pop_front() {
            if !visited.insert((offset, chain)) {
                continue;
            }
            if *nearest.entry(offset).or_insert(depth) > self.limits.max_depth {
                return Err(bound("depth"));
            }
            let Some((tags, next)) = self.directory(offset)? else { return Ok(None) };
            if let Entry::Vacant(slot) = children_of.entry(offset) {
                self.record_image_data(&tags)?;
                let Some(children) = self.children(&tags)? else { return Ok(None) };
                slot.insert(children.iter().map(|&(child, _)| child).collect());
                queue.extend(children.into_iter().map(|(child, chain)| (child, chain, depth + 1)));
            }
            if chain && next != 0 {
                chain_next.insert(offset, next);
                queue.push_front((next, true, depth));
            }
        }
        // Phase 2: a directory with a path back to itself is a cycle (depth-first, three colours).
        let successors =
            |offset: u64| -> Vec<u64> { children_of.get(&offset).into_iter().flatten().copied().chain(chain_next.get(&offset).copied()).collect() };
        // `false`: open on the current path; `true`: finished, with no path back to itself.
        let mut colour: HashMap<u64, bool> = HashMap::from([(first, false)]);
        let mut stack = vec![(first, successors(first).into_iter())];
        while let Some((offset, rest)) = stack.last_mut() {
            let offset = *offset;
            match rest.next().map(|target| (target, colour.get(&target).copied())) {
                Some((_, Some(false))) => return Ok(None),
                Some((target, None)) => {
                    colour.insert(target, false);
                    stack.push((target, successors(target).into_iter()));
                }
                Some(_) => {}
                None => {
                    colour.insert(offset, true);
                    stack.pop();
                }
            }
        }
        // The main chain: the first directory and each `next` target. The graph is acyclic here.
        let mut main_chain = vec![first];
        while let Some(&next) = main_chain.last().and_then(|at| chain_next.get(at)) {
            if main_chain.len() > self.parsed.len() {
                break;
            }
            main_chain.push(next);
        }
        Ok(Some(main_chain))
    }

    fn directory(&mut self, offset: u64) -> Result<Option<(BTreeMap<u16, EntrySpan>, u64)>, CodecError> {
        observation_contract! {
            rules: "-";
            collections: "field.bytes*2,field.declared*5,field.entries*3,field.parsed*3,field.value_spans*1,type.BTreeMap*2";
            scope: "TIFF directory table and physical value spans, bounded before directory or decoder allocations";
            loss: "malformed directory, incomplete value or pointer, directory cycle, work/byte budget";
            fallback: "do not assess pixels from an incomplete preflight";
            package_loss: "independent";
        }

        if let Some(known) = self.parsed.get(&offset) {
            return Ok(Some(known.clone()));
        }
        if self.parsed.len() >= self.limits.max_ifds {
            return Err(bound("count"));
        }
        (self.reserve)(512)?;
        let Ok(dir) = directory_entries(self.bytes, offset, self.order, self.big, self.limits.max_entries) else { return Ok(None) };
        self.entries = self.entries.saturating_add(dir.read());
        if self.entries > self.limits.max_entries_total || (dir.read() >= self.limits.max_entries && dir.declared() > dir.read() as u64) {
            return Err(bound("entry"));
        }
        if dir.declared() != dir.read() as u64 {
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

    /// The directories a directory points at, each with whether it is read as a chain (SubIFD) or as a
    /// single directory (Exif, GPS, Interop). `None` when a pointer is unreadable or malformed.
    fn children(&mut self, tags: &BTreeMap<u16, EntrySpan>) -> Result<Option<Vec<(u64, bool)>>, CodecError> {
        observation_contract! {
            rules: "-";
            collections: "type.BTreeMap*1,type.Vec*2";
            scope: "SubIFD, Exif, GPS and Interop pointers under directory, depth and work bounds";
            loss: "unreadable child pointer or incomplete child directory";
            fallback: "withhold the complete directory inventory";
            package_loss: "independent";
        }

        let mut out = Vec::new();
        for tag in [tags::SUB_IFDS, tags::EXIF_IFD, tags::GPS_IFD, tags::INTEROP_IFD] {
            let Some(children) = tags.get(&tag) else { continue };
            if tag != tags::SUB_IFDS && children.count != 1 {
                return Ok(None);
            }
            (self.reserve)(children.count.saturating_mul(8))?;
            for index in 0..children.count {
                let Some(child) = self.uint(*children, index) else { return Ok(None) };
                if child != 0 {
                    out.push((child, tag == tags::SUB_IFDS));
                }
            }
        }
        Ok(Some(out))
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

/// The stored `(offset, length)` ranges of the page's strips or tiles, from its offset and byte-count tables: at
/// most `limit` of them, in order.
pub(crate) fn chunks(bytes: &[u8], page: &craft_image_codecs::TiffPage, limit: usize) -> Vec<(usize, usize)> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*4";
        scope: "the selected TIFF directory's offset and byte-count tables";
        loss: "unreadable directory or table";
        fallback: "return no ranges";
        package_loss: "independent";
    }

    let Some((order, big)) = Tiff::sniff(bytes) else { return Vec::new() };
    let Ok(dir) = directory_entries(bytes, page.ifd_offset, order, big, ParseOptions::default().max_entries) else { return Vec::new() };
    let (offsets_tag, counts_tag) =
        if page.tiled { (tags::TILE_OFFSETS, tags::TILE_BYTE_COUNTS) } else { (tags::STRIP_OFFSETS, tags::STRIP_BYTE_COUNTS) };
    let (mut offsets, mut counts) = (None, None);
    for pos in dir.positions() {
        let Some(span) = entry_span(bytes, pos, order, big, 0) else { continue };
        if span.tag == offsets_tag {
            offsets = Some(span);
        } else if span.tag == counts_tag {
            counts = Some(span);
        }
    }
    let (Some(offsets), Some(counts)) = (offsets, counts) else { return Vec::new() };
    (0..offsets.count.min(counts.count).min(limit as u64))
        .filter_map(|index| {
            let start = usize::try_from(span_uint(bytes, order, offsets, index)?).ok()?;
            Some((start, usize::try_from(span_uint(bytes, order, counts, index)?).ok()?))
        })
        .collect()
}

/// Bytes one band-decoder worker holds while it opens the selected page, beyond its scratch
/// buffers. The codec builds a `tiff` decoder per worker (`bands.rs` `decode_bands`, through
/// `Prepared::open`), and that decoder re-reads the directory:
/// - the whole directory as a `BTreeMap` of entries, kept for the decoder's lifetime
///   (`Decoder::read_ifd`, about 64 bytes per entry with node overhead);
/// - every multi-valued tag it interprets as a `Vec<Value>` of 32-byte values that is then
///   narrowed into a vector of the final type (`Value::into_*_vec`), so the strip or tile offset
///   and byte-count tables peak at 8 + (32 + 8) bytes per chunk once both are read, and the other
///   lists at 32 + 8 bytes per value (`Image::from_reader`);
/// - a fixed reader, cycle-tracking maps and one-entry vectors.
pub(crate) fn worker_open_bytes(bytes: &[u8], page: &craft_image_codecs::TiffPage) -> u64 {
    observation_contract! {
        rules: "-";
        collections: "field.declared*1";
        scope: "the selected TIFF directory's entry table and value counts";
        loss: "unreadable directory";
        fallback: "charge only the fixed decoder state";
        package_loss: "independent";
    }

    const VALUE: u64 = 32;
    let fixed = 4096;
    let Some((order, big)) = Tiff::sniff(bytes) else { return fixed };
    let Ok(dir) = directory_entries(bytes, page.ifd_offset, order, big, ParseOptions::default().max_entries) else { return fixed };
    let mut total = fixed.saturating_add(dir.declared().min(1 << 20).saturating_mul(64));
    for pos in dir.positions() {
        let Some(span) = entry_span(bytes, pos, order, big, 0) else { continue };
        let values = span.count;
        total = total.saturating_add(match span.tag {
            // Offsets are retained while the byte counts expand.
            tags::STRIP_OFFSETS | tags::TILE_OFFSETS => values.saturating_mul(8 + VALUE + 8),
            // BitsPerSample, ExtraSamples, SampleFormat, JPEGTables, ChromaSubsampling.
            258 | 338 | 339 | 347 | 530 if values > 1 => values.saturating_mul(VALUE + 8),
            _ => 0,
        });
    }
    total
}
