//! Bounds-checked little-endian access to hostile input. Every read returns `None` past the end, so
//! a truncated or lying size field stops the parse instead of panicking.

#[derive(Clone, Copy)]
pub(crate) struct Bytes<'a>(pub(crate) &'a [u8]);

impl<'a> Bytes<'a> {
    pub fn slice(&self, off: usize, len: usize) -> Option<&'a [u8]> {
        self.0.get(off..off.checked_add(len)?)
    }

    pub fn tail(&self, off: usize) -> Option<Bytes<'a>> {
        self.0.get(off..).map(Bytes)
    }

    fn arr<const N: usize>(&self, off: usize) -> Option<[u8; N]> {
        self.slice(off, N)?.try_into().ok()
    }

    pub fn u16(&self, off: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.arr(off)?))
    }

    pub fn i16(&self, off: usize) -> Option<i16> {
        Some(i16::from_le_bytes(self.arr(off)?))
    }

    pub fn u32(&self, off: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.arr(off)?))
    }

    pub fn i32(&self, off: usize) -> Option<i32> {
        Some(i32::from_le_bytes(self.arr(off)?))
    }

    pub fn f32(&self, off: usize) -> Option<f32> {
        Some(f32::from_le_bytes(self.arr(off)?))
    }

    /// `n` consecutive (x, y) pairs of 16- or 32-bit signed integers starting at `off`.
    pub fn points(&self, off: usize, n: usize, wide: bool) -> Option<Vec<(f64, f64)>> {
        let sz = if wide { 8 } else { 4 };
        let raw = self.slice(off, n.checked_mul(sz)?)?;
        let mut out = Vec::with_capacity(n);
        for c in raw.chunks_exact(sz) {
            let p = if wide {
                let x = i32::from_le_bytes(c.get(0..4)?.try_into().ok()?);
                let y = i32::from_le_bytes(c.get(4..8)?.try_into().ok()?);
                (f64::from(x), f64::from(y))
            } else {
                let x = i16::from_le_bytes(c.get(0..2)?.try_into().ok()?);
                let y = i16::from_le_bytes(c.get(2..4)?.try_into().ok()?);
                (f64::from(x), f64::from(y))
            };
            out.push(p);
        }
        Some(out)
    }
}
