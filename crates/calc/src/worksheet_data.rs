//! Compact, bounded source for imported worksheet matrices. The data travels with the
//! calculation and survives saving as DOCX without relying on the original linked file.

use std::io::{Read, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use craft_units::Quantity;
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use serde::{Deserialize, Serialize};

use crate::CalcError;
use crate::array::{self, Cell, Data, MAX_ELEMENTS};
use crate::array_funcs::Ctx;
use crate::eval::Val;

const MAX_BYTES: usize = 512 * 1024;
const CHUNK: usize = 960;

/// A cell supplied by a worksheet or spreadsheet reader.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub enum WorksheetCell {
    Number(f64),
    Text(String),
}

/// A calculation expression containing a snapshot of a rectangular range.
pub fn source(rows: usize, cols: usize, cells: &[WorksheetCell]) -> Option<String> {
    array::check_size(rows, cols, cells.len()).ok()?;
    let json = serde_json::to_vec(cells).ok()?;
    if json.len() > MAX_BYTES {
        return None;
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&json).ok()?;
    let payload = STANDARD.encode(encoder.finish().ok()?);
    let chunks = payload.as_bytes().chunks(CHUNK).map(|chunk| format!("\"{}\"", String::from_utf8_lossy(chunk))).collect::<Vec<_>>();
    let source = format!("worksheetData({rows}, {cols}, {})", chunks.join(", "));
    crate::parse(&format!("{source} =")).ok()?;
    Some(source)
}

pub(crate) fn evaluate(args: &[Data], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let invalid = || CalcError::Syntax("invalid imported worksheet data".into());
    let [Data::One(rows), Data::One(cols), parts @ ..] = args else { return Err(invalid()) };
    let size = |value: &Val| {
        let n = value.q.value;
        (value.q.dim == craft_units::Dim::ONE && n >= 1.0 && n <= MAX_ELEMENTS as f64 && n.fract() == 0.0).then_some(n as usize).ok_or_else(invalid)
    };
    let (rows, cols) = (size(rows)?, size(cols)?);
    let cells = rows.checked_mul(cols).filter(|n| *n <= MAX_ELEMENTS).ok_or(CalcError::TooLarge)?;
    cx.pass.charge(cells as u64)?;
    let mut encoded = String::new();
    for part in parts {
        let Data::Text(chunk) = part else { return Err(invalid()) };
        encoded.push_str(chunk);
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| invalid())?;
    let mut decoded = Vec::new();
    ZlibDecoder::new(bytes.as_slice()).take(MAX_BYTES as u64 + 1).read_to_end(&mut decoded).map_err(|_| invalid())?;
    if decoded.len() > MAX_BYTES {
        return Err(CalcError::TooLarge);
    }
    cx.pass.charge((decoded.len() / 32) as u64)?;
    let values: Vec<WorksheetCell> = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    array::check_size(rows, cols, values.len())?;
    array::cells_to_data(
        rows,
        cols,
        values
            .into_iter()
            .map(|cell| match cell {
                WorksheetCell::Number(value) => Cell::Number(Val::plain(Quantity::dimensionless(value))),
                WorksheetCell::Text(text) => Cell::Text(text),
            })
            .collect(),
    )
}
