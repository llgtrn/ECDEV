//! The compact deterministic binary encoding of ECDEV governance state (`*.ecg`).
//!
//! Layout: `ECG` magic, one format byte, one record-type tag, then fields as LEB128 unsigned
//! integers and length-prefixed UTF-8 strings. Collections are written sorted by their caller,
//! so equal state always encodes to equal bytes, and a file's name is the digest of its bytes.

pub const MAGIC: &[u8; 3] = b"ECG";
/// The file extension of every content-addressed governance record.
pub const EXTENSION: &str = "ecg";
pub const FORMAT: u8 = 1;

#[derive(Default)]
pub struct Encoder {
    pub bytes: Vec<u8>,
}

impl Encoder {
    pub fn new(tag: u8) -> Self {
        let mut bytes = MAGIC.to_vec();
        bytes.push(FORMAT);
        bytes.push(tag);
        Encoder { bytes }
    }
    pub fn u64(&mut self, mut v: u64) -> &mut Self {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.bytes.push(byte);
                return self;
            }
            self.bytes.push(byte | 0x80);
        }
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.bytes.push(v);
        self
    }
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(v as u8)
    }
    pub fn str(&mut self, v: &str) -> &mut Self {
        self.u64(v.len() as u64);
        self.bytes.extend_from_slice(v.as_bytes());
        self
    }
    pub fn strs(&mut self, v: &[String]) -> &mut Self {
        self.u64(v.len() as u64);
        for s in v {
            self.str(s);
        }
        self
    }
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.u64(v.len() as u64);
        self.bytes.extend_from_slice(v);
        self
    }
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

pub struct Decoder<'a> {
    s: &'a [u8],
    i: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

type R<T> = Result<T, DecodeError>;

impl<'a> Decoder<'a> {
    /// Opens a record and checks magic, format and tag.
    pub fn open(s: &'a [u8], tag: u8) -> R<Self> {
        if s.len() < 5 || &s[..3] != MAGIC {
            return Err(DecodeError("not a governance record".into()));
        }
        if s[3] != FORMAT {
            return Err(DecodeError(format!("unsupported record format {}", s[3])));
        }
        if s[4] != tag {
            return Err(DecodeError(format!(
                "record tag {} where {} expected",
                s[4], tag
            )));
        }
        Ok(Decoder { s, i: 5 })
    }
    /// The record tag of an encoded record, if it is one.
    pub fn tag_of(s: &[u8]) -> Option<u8> {
        (s.len() >= 5 && &s[..3] == MAGIC && s[3] == FORMAT).then(|| s[4])
    }
    pub fn u64(&mut self) -> R<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let b = *self
                .s
                .get(self.i)
                .ok_or_else(|| DecodeError("truncated integer".into()))?;
            self.i += 1;
            if shift >= 64 {
                return Err(DecodeError("integer overflow".into()));
            }
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }
    pub fn u8(&mut self) -> R<u8> {
        let b = *self
            .s
            .get(self.i)
            .ok_or_else(|| DecodeError("truncated byte".into()))?;
        self.i += 1;
        Ok(b)
    }
    pub fn bool(&mut self) -> R<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn length(&mut self) -> R<usize> {
        let n = self.u64()? as usize;
        if n > self.s.len() - self.i.min(self.s.len()) && n > 0 {
            // A collection count may exceed the remaining bytes only for zero-sized items,
            // which ECDEV governance never writes.
            return Err(DecodeError("length exceeds record".into()));
        }
        Ok(n)
    }
    pub fn bytes(&mut self) -> R<&'a [u8]> {
        let n = self.length()?;
        let v = &self.s[self.i..self.i + n];
        self.i += n;
        Ok(v)
    }
    pub fn str(&mut self) -> R<String> {
        let b = self.bytes()?;
        String::from_utf8(b.to_vec()).map_err(|_| DecodeError("invalid UTF-8".into()))
    }
    pub fn strs(&mut self) -> R<Vec<String>> {
        let n = self.length()?;
        (0..n).map(|_| self.str()).collect()
    }
    pub fn end(&self) -> R<()> {
        if self.i == self.s.len() {
            Ok(())
        } else {
            Err(DecodeError("trailing bytes in record".into()))
        }
    }
    /// Reads a vocabulary word by its rank.
    pub fn word<T: Copy>(&mut self, all: &[T]) -> R<T> {
        let r = self.u8()? as usize;
        all.get(r)
            .copied()
            .ok_or_else(|| DecodeError(format!("vocabulary rank {r} out of range")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_rejection() {
        let mut e = Encoder::new(7);
        e.u64(0)
            .u64(127)
            .u64(128)
            .u64(u64::MAX)
            .str("héllo")
            .strs(&["a".into(), "".into()])
            .bool(true);
        let bytes = e.finish();
        let mut d = Decoder::open(&bytes, 7).unwrap();
        assert_eq!(d.u64().unwrap(), 0);
        assert_eq!(d.u64().unwrap(), 127);
        assert_eq!(d.u64().unwrap(), 128);
        assert_eq!(d.u64().unwrap(), u64::MAX);
        assert_eq!(d.str().unwrap(), "héllo");
        assert_eq!(d.strs().unwrap(), vec!["a".to_string(), String::new()]);
        assert!(d.bool().unwrap());
        d.end().unwrap();
        assert!(Decoder::open(&bytes, 8).is_err());
        assert!(Decoder::open(b"XYZ\x01\x07", 7).is_err());
        let mut t = Decoder::open(&bytes[..8], 7).unwrap();
        t.u64().unwrap();
        t.u64().unwrap();
        assert!(t.u64().is_err());
    }
}
