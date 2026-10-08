//! Bounded gzip (RFC 1952) over DEFLATE (RFC 1951), written from the two RFCs. ECDEV reads
//! compressed sitemaps with it; measured on live robots.txt files, 6 of 178 sites that declare
//! sitemaps publish only `.gz` ones (2026-10-07).
//!
//! Strict by design: the output may never pass `max_out` bytes (a small compressed body can
//! inflate a thousandfold, so the cap bounds memory and time, not the input), every member's
//! CRC-32 and length are verified, and a truncated or corrupt stream is an error, never partial
//! data. (Scrapy's gunzip returns whatever it decoded before the error; a sitemap read halfway
//! would read as a complete one.) Concatenated members are accepted, trailing zero padding too.

/// The error codes are stable strings, carried into the caller's own error reasons.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum GzipError {
    NotGzip,
    UnsupportedMethod,
    ReservedFlags,
    Truncated,
    Corrupt(&'static str),
    HeaderChecksum,
    Checksum,
    Length,
    TooLarge,
    TrailingData,
}

impl GzipError {
    pub fn code(&self) -> &'static str {
        match self {
            GzipError::NotGzip => "GZIP_NOT_GZIP",
            GzipError::UnsupportedMethod => "GZIP_UNSUPPORTED_METHOD",
            GzipError::ReservedFlags => "GZIP_RESERVED_FLAGS",
            GzipError::Truncated => "GZIP_TRUNCATED",
            GzipError::Corrupt(_) => "GZIP_CORRUPT_STREAM",
            GzipError::HeaderChecksum => "GZIP_HEADER_CHECKSUM",
            GzipError::Checksum => "GZIP_CRC32_MISMATCH",
            GzipError::Length => "GZIP_LENGTH_MISMATCH",
            GzipError::TooLarge => "GZIP_OUTPUT_LIMIT_EXCEEDED",
            GzipError::TrailingData => "GZIP_TRAILING_DATA",
        }
    }
}

pub fn is_gzip(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b])
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}
static CRC: [u32; 256] = crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |c, &b| {
        CRC[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8)
    })
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    count: u32,
}

impl Bits<'_> {
    fn take(&mut self, need: u32) -> Result<u32, GzipError> {
        while self.count < need {
            let byte = *self.data.get(self.pos).ok_or(GzipError::Truncated)?;
            self.pos += 1;
            self.buf |= (byte as u32) << self.count;
            self.count += 8;
        }
        let value = self.buf & ((1u32 << need) - 1);
        self.buf >>= need;
        self.count -= need;
        Ok(value)
    }
    /// Drops the bits left in the current byte (before a stored block's lengths).
    fn align(&mut self) {
        self.buf = 0;
        self.count = 0;
    }
}

const MAX_BITS: usize = 15;

struct Huffman {
    count: [u16; MAX_BITS + 1],
    symbol: Vec<u16>,
}

impl Huffman {
    /// Canonical code from code lengths (RFC 1951 3.2.2). Returns the table and whether the
    /// code is incomplete; an over-subscribed code is an error.
    fn build(lengths: &[u8]) -> Result<(Huffman, bool), GzipError> {
        let mut count = [0u16; MAX_BITS + 1];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut left: i32 = 1;
        for &n in &count[1..=MAX_BITS] {
            left <<= 1;
            left -= n as i32;
            if left < 0 {
                return Err(GzipError::Corrupt("over-subscribed code"));
            }
        }
        let mut offs = [0u16; MAX_BITS + 1];
        for len in 1..MAX_BITS {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Ok((Huffman { count, symbol }, left > 0))
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, GzipError> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=MAX_BITS {
            code |= bits.take(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(GzipError::Corrupt("invalid code"))
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

struct Out {
    bytes: Vec<u8>,
    start: usize,
    limit: usize,
}

impl Out {
    fn room(&self, extra: usize) -> Result<(), GzipError> {
        if self.bytes.len() + extra > self.limit {
            Err(GzipError::TooLarge)
        } else {
            Ok(())
        }
    }
}

fn codes(bits: &mut Bits, out: &mut Out, lit: &Huffman, dist: &Huffman) -> Result<(), GzipError> {
    loop {
        let sym = lit.decode(bits)? as usize;
        if sym < 256 {
            out.room(1)?;
            out.bytes.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let sym = sym - 257;
            if sym >= 29 {
                return Err(GzipError::Corrupt("invalid length code"));
            }
            let len = LEN_BASE[sym] as usize + bits.take(LEN_EXTRA[sym] as u32)? as usize;
            let dsym = dist.decode(bits)? as usize;
            if dsym >= 30 {
                return Err(GzipError::Corrupt("invalid distance code"));
            }
            let d = DIST_BASE[dsym] as usize + bits.take(DIST_EXTRA[dsym] as u32)? as usize;
            // A match may reach back only into this member's own output.
            if d > out.bytes.len() - out.start {
                return Err(GzipError::Corrupt("distance too far back"));
            }
            out.room(len)?;
            for _ in 0..len {
                let b = out.bytes[out.bytes.len() - d];
                out.bytes.push(b);
            }
        }
    }
}

fn stored(bits: &mut Bits, out: &mut Out) -> Result<(), GzipError> {
    bits.align();
    let d = bits.data;
    let p = bits.pos;
    let head = d.get(p..p + 4).ok_or(GzipError::Truncated)?;
    let len = u16::from_le_bytes([head[0], head[1]]);
    let nlen = u16::from_le_bytes([head[2], head[3]]);
    if len != !nlen {
        return Err(GzipError::Corrupt("stored block length check"));
    }
    let len = len as usize;
    let body = d.get(p + 4..p + 4 + len).ok_or(GzipError::Truncated)?;
    out.room(len)?;
    out.bytes.extend_from_slice(body);
    bits.pos = p + 4 + len;
    Ok(())
}

fn fixed() -> Result<(Huffman, Huffman), GzipError> {
    let mut lengths = [0u8; 288];
    for (i, l) in lengths.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let (lit, _) = Huffman::build(&lengths)?;
    let (dist, _) = Huffman::build(&[5u8; 30])?;
    Ok((lit, dist))
}

fn dynamic(bits: &mut Bits) -> Result<(Huffman, Huffman), GzipError> {
    let nlen = bits.take(5)? as usize + 257;
    let ndist = bits.take(5)? as usize + 1;
    let ncode = bits.take(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(GzipError::Corrupt("too many length or distance codes"));
    }
    let mut lengths = [0u8; 320];
    for &slot in CODE_LENGTH_ORDER.iter().take(ncode) {
        lengths[slot] = bits.take(3)? as u8;
    }
    let (code_lengths, incomplete) = Huffman::build(&lengths[..19])?;
    if incomplete {
        return Err(GzipError::Corrupt("incomplete code length code"));
    }
    let mut lengths = [0u8; 320];
    let mut index = 0;
    while index < nlen + ndist {
        let sym = code_lengths.decode(bits)?;
        if sym < 16 {
            lengths[index] = sym as u8;
            index += 1;
        } else {
            let (fill, repeat) = match sym {
                16 => {
                    if index == 0 {
                        return Err(GzipError::Corrupt("repeat with no previous length"));
                    }
                    (lengths[index - 1], 3 + bits.take(2)? as usize)
                }
                17 => (0, 3 + bits.take(3)? as usize),
                _ => (0, 11 + bits.take(7)? as usize),
            };
            if index + repeat > nlen + ndist {
                return Err(GzipError::Corrupt("length repeat past the table"));
            }
            for _ in 0..repeat {
                lengths[index] = fill;
                index += 1;
            }
        }
    }
    if lengths[256] == 0 {
        return Err(GzipError::Corrupt("no end-of-block code"));
    }
    let (lit, lit_incomplete) = Huffman::build(&lengths[..nlen])?;
    // An incomplete code is legal only as a single one-bit code (RFC 1951 3.2.7).
    if lit_incomplete && nlen != (lit.count[0] + lit.count[1]) as usize {
        return Err(GzipError::Corrupt("incomplete literal code"));
    }
    let (dist, dist_incomplete) = Huffman::build(&lengths[nlen..nlen + ndist])?;
    if dist_incomplete && ndist != (dist.count[0] + dist.count[1]) as usize {
        return Err(GzipError::Corrupt("incomplete distance code"));
    }
    Ok((lit, dist))
}

fn inflate(data: &[u8], out: &mut Out) -> Result<usize, GzipError> {
    let mut bits = Bits {
        data,
        pos: 0,
        buf: 0,
        count: 0,
    };
    loop {
        let last = bits.take(1)?;
        match bits.take(2)? {
            0 => stored(&mut bits, out)?,
            1 => {
                let (lit, dist) = fixed()?;
                codes(&mut bits, out, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic(&mut bits)?;
                codes(&mut bits, out, &lit, &dist)?;
            }
            _ => return Err(GzipError::Corrupt("reserved block type")),
        }
        if last == 1 {
            return Ok(bits.pos);
        }
    }
}

/// Reads the member header, returning the offset of the DEFLATE data.
fn header(data: &[u8]) -> Result<usize, GzipError> {
    if data.len() < 10 {
        return Err(if is_gzip(data) {
            GzipError::Truncated
        } else {
            GzipError::NotGzip
        });
    }
    if !is_gzip(data) {
        return Err(GzipError::NotGzip);
    }
    if data[2] != 8 {
        return Err(GzipError::UnsupportedMethod);
    }
    let flags = data[3];
    if flags & 0xe0 != 0 {
        return Err(GzipError::ReservedFlags);
    }
    let mut p = 10;
    if flags & 4 != 0 {
        let xlen = u16::from_le_bytes([
            *data.get(p).ok_or(GzipError::Truncated)?,
            *data.get(p + 1).ok_or(GzipError::Truncated)?,
        ]) as usize;
        p += 2 + xlen;
        if p > data.len() {
            return Err(GzipError::Truncated);
        }
    }
    for flag in [8u8, 16] {
        if flags & flag != 0 {
            let end = data[p.min(data.len())..]
                .iter()
                .position(|&b| b == 0)
                .ok_or(GzipError::Truncated)?;
            p += end + 1;
        }
    }
    if flags & 2 != 0 {
        let stored = u16::from_le_bytes([
            *data.get(p).ok_or(GzipError::Truncated)?,
            *data.get(p + 1).ok_or(GzipError::Truncated)?,
        ]);
        if stored != (crc32(&data[..p]) & 0xffff) as u16 {
            return Err(GzipError::HeaderChecksum);
        }
        p += 2;
    }
    Ok(p)
}

/// Decompresses one or more concatenated gzip members, failing past `max_out` output bytes.
pub fn gunzip(data: &[u8], max_out: usize) -> Result<Vec<u8>, GzipError> {
    let mut out = Out {
        bytes: Vec::new(),
        start: 0,
        limit: max_out,
    };
    let mut rest = data;
    loop {
        let body = header(rest)?;
        out.start = out.bytes.len();
        let used = inflate(&rest[body..], &mut out)?;
        let trailer = body + used;
        let t = rest.get(trailer..trailer + 8).ok_or(GzipError::Truncated)?;
        let member = &out.bytes[out.start..];
        if u32::from_le_bytes([t[0], t[1], t[2], t[3]]) != crc32(member) {
            return Err(GzipError::Checksum);
        }
        if u32::from_le_bytes([t[4], t[5], t[6], t[7]]) != member.len() as u32 {
            return Err(GzipError::Length);
        }
        rest = &rest[trailer + 8..];
        if rest.is_empty() {
            return Ok(out.bytes);
        }
        if !is_gzip(rest) {
            // Tools pad archives with zeros; anything else after a member is not ours to ignore.
            return if rest.iter().all(|&b| b == 0) {
                Ok(out.bytes)
            } else {
                Err(GzipError::TrailingData)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // zlib.compressobj(9, DEFLATED, -15) over b"hello hello hello hello\n" framed as gzip
    // (mtime 0, XFL 2, OS 255), made with CPython 3.13 / zlib 1.3.
    const HELLO: [u8; 29] = [
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xcb, 0x48, 0xcd, 0xc9, 0xc9,
        0x57, 0xc8, 0x40, 0x27, 0xb9, 0x00, 0x00, 0x88, 0x59, 0x0b, 0x18, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_stored_block_round_trips_and_errors_are_named() {
        // A hand-built member holding "abc" in one stored block.
        let mut m = vec![
            0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255, 0x01, 3, 0, 0xfc, 0xff, b'a', b'b', b'c',
        ];
        m.extend_from_slice(&crc32(b"abc").to_le_bytes());
        m.extend_from_slice(&3u32.to_le_bytes());
        assert_eq!(gunzip(&m, 100).unwrap(), b"abc");
        assert_eq!(gunzip(&m, 2), Err(GzipError::TooLarge));
        assert_eq!(gunzip(&m[..m.len() - 3], 100), Err(GzipError::Truncated));
        let mut bad = m.clone();
        let n = bad.len();
        bad[n - 5] ^= 1;
        assert_eq!(gunzip(&bad, 100), Err(GzipError::Checksum));
        let mut bad = m.clone();
        let n = bad.len();
        bad[n - 1] = 9;
        assert_eq!(gunzip(&bad, 100), Err(GzipError::Length));
        assert_eq!(
            gunzip(b"plain text, not gzip", 100),
            Err(GzipError::NotGzip)
        );
        assert_eq!(
            gunzip(&[0x1f, 0x8b, 7, 0, 0, 0, 0, 0, 0, 0], 100),
            Err(GzipError::UnsupportedMethod)
        );
        assert_eq!(
            gunzip(&[0x1f, 0x8b, 8, 0x20, 0, 0, 0, 0, 0, 0], 100),
            Err(GzipError::ReservedFlags)
        );
        let mut tail = m.clone();
        tail.extend_from_slice(b"junk");
        assert_eq!(gunzip(&tail, 100), Err(GzipError::TrailingData));
        let mut padded = m.clone();
        padded.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(gunzip(&padded, 100).unwrap(), b"abc");
        // Concatenated members are one stream.
        let mut two = m.clone();
        two.extend_from_slice(&m);
        assert_eq!(gunzip(&two, 100).unwrap(), b"abcabc");
    }

    #[test]
    fn a_compressed_member_from_a_reference_implementation_decodes() {
        assert_eq!(gunzip(&HELLO, 1000).unwrap(), b"hello hello hello hello\n");
        assert_eq!(gunzip(&HELLO, 10), Err(GzipError::TooLarge));
    }
}
