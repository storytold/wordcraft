//! A read-only look inside `word/vbaProject.bin` ([MS-OVBA]): which modules it holds and what
//! source they carry. The project is never run; parsing exists so agents and users can audit
//! what a document would have run elsewhere (steps 1–2 of the macro work: banner + source
//! audit, no execution — #5444).
//!
//! The project is a compound file ([MS-CFB]) whose `/VBA/dir` stream lists modules; each
//! module's stream holds a performance cache, then the source at [`Module::text_offset`],
//! compressed with the MS-OVBA container format ([`decompress`]). The reader is lenient
//! (unknown dir records are skipped; a module whose stream can't be read is listed without
//! source) and bounded (stream and source sizes are capped), so hostile input yields an error
//! or a best-effort report, never a panic.
//!
//! Names and source are MBCS in the project's code page ([`Project::code_page`]); they are
//! decoded as UTF-8 with replacement, which is exact for the ASCII that macro source almost
//! always is. A code page other than 1252 is recorded, not translated.

use std::io::{Cursor, Read, Seek};

use crate::DocxError;

fn bad(msg: impl Into<String>) -> DocxError {
    DocxError::NotWord(msg.into())
}

/// How large a `/VBA/dir` stream may be ([`MAX_DIR`] bytes; real ones are a few KiB).
pub const MAX_DIR: u64 = 1 << 20;
/// How many modules a project may list before the report says `truncated`.
pub const MAX_MODULES: usize = 256;
/// How large one module's source may be before the module says `truncated`.
pub const MAX_SOURCE: u64 = 1 << 20;

/// What [MS-OVBA] §2.4.1.1.5 names a compressed chunk's signature: bits 12..15 of the header.
const CHUNK_SIGNATURE: u16 = 0xB000;
/// The container's first byte ([MS-OVBA] §2.4.1.1.4).
const CONTAINER_SIGNATURE: u8 = 0x01;
/// How many bytes an uncompressed (raw) chunk carries.
const RAW_CHUNK: usize = 4096;

/// Dir record ids ([MS-OVBA] §2.3.4.2). Only the ones the audit reads are named.
const REC_CODE_PAGE: u16 = 0x0003;
const REC_PROJECT_VERSION: u16 = 0x0009;
const REC_MODULES_COUNT: u16 = 0x000F;
const REC_MODULE_NAME: u16 = 0x0019;
const REC_MODULE_STREAM_NAME: u16 = 0x001A;
const REC_MODULE_PROCEDURAL: u16 = 0x0021;
const REC_MODULE_DOCUMENT: u16 = 0x0022;
const REC_MODULE_READ_ONLY: u16 = 0x0025;
const REC_MODULE_PRIVATE: u16 = 0x0028;
const REC_MODULE_END: u16 = 0x002B;
const REC_MODULE_OFFSET: u16 = 0x0031;
const REC_MODULE_NAME_UNICODE: u16 = 0x0047;

/// A VBA project: the modules its `dir` stream lists, each with its source when it could be
/// read. [`Project::truncated`] says the project lists more than [`MAX_MODULES`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    /// The modules in `dir` order.
    pub modules: Vec<Module>,
    /// The project's code page (the MBCS encoding its names and source use).
    pub code_page: u16,
    /// The project lists more modules than [`MAX_MODULES`] (the rest are left out).
    pub truncated: bool,
}

/// One VBA module: where its stream lives and what source it carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Module {
    /// The module's name (`MODULENAME`, or its Unicode variant when the project has one).
    pub name: String,
    /// The name of the stream under `/VBA/` that holds the module.
    pub stream_name: String,
    /// Where in the stream the source starts (the performance cache comes first).
    pub text_offset: u64,
    /// Is the module procedural (rather than document or class)?
    pub procedural: bool,
    /// The module is marked read-only.
    pub read_only: bool,
    /// The module is marked private.
    pub private: bool,
    /// The module's source, decoded as UTF-8 with replacement ([`MAX_SOURCE`] cap);
    /// `None` when the stream could not be read.
    pub source: Option<String>,
    /// The source is longer than [`MAX_SOURCE`] (it is cut at the cap).
    pub truncated: bool,
}

/// The project inside `word/vbaProject.bin`. Nothing here runs anything.
pub fn project(bytes: &[u8]) -> Result<Project, DocxError> {
    wordcraft_doc::cfb_guard::check(bytes).map_err(|e| bad(format!("not a readable compound file ({e})")))?;
    let mut comp = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|e| bad(format!("not a readable compound file ({e})")))?;
    let dir = stream(&mut comp, "/VBA/dir", MAX_DIR)?;
    let dir = decompress(&dir, MAX_DIR as usize)?;
    let mut project = walk_dir(&dir)?;
    for module in &mut project.modules {
        // A stream name is a single path segment; anything else can't name a module stream.
        if module.stream_name.is_empty() || module.stream_name.contains(['/', '\\', '\0']) {
            module.source = None;
            continue;
        }
        let path = format!("/VBA/{}", module.stream_name);
        let bytes = match stream(&mut comp, &path, MAX_SOURCE) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!("docx: can't read the VBA module {path:?} ({e})");
                module.source = None;
                continue;
            }
        };
        read_source(module, &bytes);
    }
    Ok(project)
}

/// A module's stream: a performance cache, then the source at [`Module::text_offset`] —
/// compressed when it starts with the container signature ([MS-OVBA] §2.4.1.1.4), raw
/// otherwise. Lenient: an offset past the stream lists the module without source.
fn read_source(module: &mut Module, stream: &[u8]) {
    let at = module.text_offset.min(stream.len() as u64) as usize;
    let rest = &stream[at..];
    let source = if rest.first() == Some(&CONTAINER_SIGNATURE) {
        match decompress(rest, MAX_SOURCE as usize) {
            Ok(raw) => raw,
            Err(e) => {
                log::warn!("docx: can't decompress the VBA module {:?} ({e})", module.stream_name);
                module.source = None;
                return;
            }
        }
    } else if rest.len() as u64 > MAX_SOURCE {
        module.source = Some(decode(&rest[..MAX_SOURCE as usize]));
        module.truncated = true;
        return;
    } else {
        rest.to_vec()
    };
    module.truncated = source.len() as u64 > MAX_SOURCE;
    module.source = Some(decode(&source[..source.len().min(MAX_SOURCE as usize)]));
}

/// The `dir` stream: a flat list of `id + size + payload` records ([MS-OVBA] §2.3.4.2).
/// Unknown records are skipped; a cut record ends the walk (lenient).
fn walk_dir(dir: &[u8]) -> Result<Project, DocxError> {
    let mut project = Project::default();
    let mut modules: Vec<Module> = Vec::new();
    let mut count: Option<u32> = None;
    let mut in_module = false;
    let mut skipping = false;
    let mut at = 0;
    while at + 6 <= dir.len() {
        let (Some(id), Some(size)) = (u16_at(dir, at), u32_at(dir, at + 2)) else { break };
        let size = size as usize;
        // PROJECTVERSION: id, reserved size (4), then 4 + 2 bytes of version; not a sized
        // payload ([MS-OVBA] §2.3.4.2.1.11), so the generic walk would stop on real projects.
        if id == REC_PROJECT_VERSION {
            at = at.checked_add(12).ok_or_else(|| bad("the dir stream is too large"))?;
            continue;
        }
        let Some(end) = at.checked_add(6).and_then(|a| a.checked_add(size)) else { break };
        let Some(payload) = dir.get(at + 6..end) else { break };
        at = end;
        // A module beyond the cap is skipped whole: its records must not touch the last
        // kept one.
        match (id, in_module && !skipping) {
            (REC_CODE_PAGE, _) => project.code_page = u16_at(payload, 0).unwrap_or(0),
            (REC_MODULES_COUNT, false) => count = u32_at(payload, 0),
            (REC_MODULE_NAME, _) => {
                in_module = true;
                if modules.len() < MAX_MODULES {
                    skipping = false;
                    modules.push(Module { name: decode(payload), ..Module::default() });
                } else {
                    skipping = true;
                    project.truncated = true;
                }
            }
            (REC_MODULE_NAME_UNICODE, true) => {
                if let Some(module) = modules.last_mut() {
                    module.name = decode_utf16(payload).unwrap_or_else(|| module.name.clone());
                }
            }
            (REC_MODULE_STREAM_NAME, true) => {
                if let Some(module) = modules.last_mut() {
                    module.stream_name = decode(payload);
                }
            }
            (REC_MODULE_OFFSET, true) => {
                if let Some(module) = modules.last_mut() {
                    module.text_offset = u32_at(payload, 0).unwrap_or(0) as u64;
                }
            }
            (REC_MODULE_PROCEDURAL, true) => {
                if let Some(module) = modules.last_mut() {
                    module.procedural = true;
                }
            }
            (REC_MODULE_DOCUMENT, true) => {
                if let Some(module) = modules.last_mut() {
                    module.procedural = false;
                }
            }
            (REC_MODULE_READ_ONLY, true) => {
                if let Some(module) = modules.last_mut() {
                    module.read_only = true;
                }
            }
            (REC_MODULE_PRIVATE, true) => {
                if let Some(module) = modules.last_mut() {
                    module.private = true;
                }
            }
            (REC_MODULE_END, _) => {
                in_module = false;
                skipping = false;
            }
            _ => {}
        }
    }
    if count.is_some_and(|c| c as usize > MAX_MODULES) {
        project.truncated = true;
    }
    project.modules = modules;
    Ok(project)
}

/// The MS-OVBA compressed-container format ([MS-OVBA] §2.4.1): a signature byte, then chunks —
/// each a header (signature bits, and how many bytes the chunk carries) followed by flag bytes
/// with literals and copy tokens. `cap` bounds the total decompressed size; a container that
/// decompresses past it says [`DocxError::Limit`].
pub fn decompress(bytes: &[u8], cap: usize) -> Result<Vec<u8>, DocxError> {
    if bytes.first() != Some(&CONTAINER_SIGNATURE) {
        return Err(bad("the compressed container doesn't start with the signature byte"));
    }
    let mut out = Vec::new();
    let mut at = 1;
    while at < bytes.len() {
        let Some(header) = u16_at(bytes, at) else { return Err(bad("the chunk header is cut")) };
        at += 2;
        if header & 0xF000 == CHUNK_SIGNATURE {
            // Bits 0..11 are the chunk size - 3, counting the header: data = size - 2.
            let size = (header & 0x0FFF) as usize + 3;
            let end = at.checked_add(size.saturating_sub(2)).ok_or_else(|| bad("the chunk is too large"))?;
            let chunk = bytes.get(at..end).ok_or_else(|| bad("the compressed chunk is cut"))?;
            at = end;
            decompress_chunk(chunk, &mut out, cap)?;
        } else {
            // An uncompressed chunk carries RAW_CHUNK literal bytes.
            let chunk =
                bytes.get(at..at.checked_add(RAW_CHUNK).ok_or_else(|| bad("the chunk is too large"))?).ok_or_else(|| bad("the raw chunk is cut"))?;
            out.extend_from_slice(chunk);
            at += RAW_CHUNK;
            if out.len() > cap {
                return Err(DocxError::Limit("the decompressed source exceeds the cap".into()));
            }
        }
    }
    Ok(out)
}

/// One compressed chunk: flag bytes whose bits name literals (bit clear) and copy tokens
/// (bit set) ([MS-OVBA] §2.4.1.3.7). Copy tokens reach back into the chunk only.
fn decompress_chunk(chunk: &[u8], out: &mut Vec<u8>, cap: usize) -> Result<(), DocxError> {
    let base = out.len();
    let mut at = 0;
    while at < chunk.len() {
        let flags = chunk[at];
        at += 1;
        for bit in 0..8u8 {
            if at >= chunk.len() {
                break;
            }
            if flags & (1 << bit) == 0 {
                out.push(chunk[at]);
                at += 1;
                if out.len() > cap {
                    return Err(DocxError::Limit("the decompressed source exceeds the cap".into()));
                }
            } else {
                let Some(token) = u16_at(chunk, at) else { return Err(bad("the copy token is cut")) };
                at += 2;
                let difference = out.len() - base;
                let (length_bits, offset_bits) = split(difference);
                let length_mask = 0xFFFFu16 >> offset_bits;
                let offset = (token >> length_bits) as usize + 1;
                let length = (token & length_mask) as usize + 3;
                let from = base
                    .checked_add(difference.checked_sub(offset).ok_or_else(|| bad("the copy token reaches past the chunk"))?)
                    .ok_or_else(|| bad("the copy token reaches past the chunk"))?;
                for i in 0..length {
                    // The source advances with the copy: overlapping tokens copy runs.
                    let byte = out.get(from + i).copied().ok_or_else(|| bad("the copy token reaches past the chunk"))?;
                    out.push(byte);
                    if out.len() > cap {
                        return Err(DocxError::Limit("the decompressed source exceeds the cap".into()));
                    }
                }
            }
        }
    }
    Ok(())
}

/// How a copy token splits into length and offset bits, per how much the chunk has already
/// decompressed ([MS-OVBA] §2.4.1.3.19.4): the offset takes `max(ceil(log2(difference)), 4)`
/// bits, capped at 12, and the length takes the other 16.
fn split(difference: usize) -> (u32, u32) {
    let mut offset_bits = 4u32;
    while offset_bits < 12 && (1usize << offset_bits) < difference {
        offset_bits += 1;
    }
    (16 - offset_bits, offset_bits)
}

/// A root-level stream, at most `cap` bytes (the same shape the encrypted reader uses).
fn stream<F: Read + Seek>(comp: &mut cfb::CompoundFile<F>, path: &str, cap: u64) -> Result<Vec<u8>, DocxError> {
    let name = path.trim_start_matches('/');
    let s = comp.open_stream(path).map_err(|e| bad(format!("can't read {name} ({e})")))?;
    let mut v = Vec::new();
    s.take(cap.saturating_add(1)).read_to_end(&mut v).map_err(|e| bad(format!("can't read {name} ({e})")))?;
    if v.len() as u64 > cap {
        return Err(DocxError::Limit(format!("{name} is too large")));
    }
    Ok(v)
}

/// MBCS text as UTF-8 with replacement (exact for ASCII; the code page is in the report).
fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// UTF-16LE text ([MS-OVBA] §2.3.4.2.3), `None` when the bytes are cut mid-pair.
fn decode_utf16(bytes: &[u8]) -> Option<String> {
    let (units, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    Some(String::from_utf16_lossy(&units.iter().map(|b| u16::from_le_bytes(*b)).collect::<Vec<u16>>()))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
