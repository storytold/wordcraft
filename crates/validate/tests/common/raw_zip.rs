//! Synthetic stored ZIP records, preserving names/metadata that ZipWriter normalizes or rejects.
#![allow(dead_code)]

pub struct Record {
    pub name: Vec<u8>,
    pub flags: u16,
    pub extra: Vec<u8>,
    pub body: Vec<u8>,
}
impl Record {
    pub fn new(name: impl AsRef<[u8]>, flags: u16, body: impl AsRef<[u8]>) -> Self {
        Self { name: name.as_ref().to_vec(), flags, extra: Vec::new(), body: body.as_ref().to_vec() }
    }
    pub fn unicode(mut self, version: u8, current: &[u8], replacement: &[u8]) -> Self {
        self.extra.extend(0x7075u16.to_le_bytes());
        self.extra.extend(u16::try_from(5 + replacement.len()).unwrap().to_le_bytes());
        self.extra.push(version);
        self.extra.extend(crc32(current).to_le_bytes());
        self.extra.extend(replacement);
        self
    }
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 { crc >> 1 } else { (crc >> 1) ^ 0xedb88320 };
        }
    }
    !crc
}

pub fn build(records: &[Record]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for r in records {
        let offset = u32::try_from(out.len()).unwrap();
        let size = u32::try_from(r.body.len()).unwrap();
        let crc = crc32(&r.body);
        let n = u16::try_from(r.name.len()).unwrap();
        let e = u16::try_from(r.extra.len()).unwrap();
        out.extend(0x04034b50u32.to_le_bytes());
        for value in [20, r.flags, 0, 0, 0] {
            out.extend(value.to_le_bytes());
        }
        for value in [crc, size, size] {
            out.extend(value.to_le_bytes());
        }
        for value in [n, e] {
            out.extend(value.to_le_bytes());
        }
        out.extend(&r.name);
        out.extend(&r.extra);
        out.extend(&r.body);
        central.extend(0x02014b50u32.to_le_bytes());
        for value in [20, 20, r.flags, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        for value in [crc, size, size] {
            central.extend(value.to_le_bytes());
        }
        for value in [n, e, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend(0u32.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(&r.name);
        central.extend(&r.extra);
    }
    let offset = u32::try_from(out.len()).unwrap();
    let size = u32::try_from(central.len()).unwrap();
    out.extend(central);
    out.extend(0x06054b50u32.to_le_bytes());
    let count = u16::try_from(records.len()).unwrap();
    for value in [0, 0, count, count] {
        out.extend(value.to_le_bytes());
    }
    for value in [size, offset] {
        out.extend(value.to_le_bytes());
    }
    out.extend(0u16.to_le_bytes());
    out
}
