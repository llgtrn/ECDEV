//! WHATWG Encoding Standard decoders for declared document charsets, written for ECDEV.
//!
//! Wire bytes stay the capture; decoding is a projection. Every decoder follows the standard's
//! algorithm and the committed index tables (adapter/web/charset-tables, from the 2024-09 indexes),
//! reports whether any byte sequence was invalid (the caller refuses such documents), and never
//! guesses. Encoders are intentionally absent: nothing encodes.

use super::text_decoding_labels::{ENCODINGS, LABELS, SINGLE_BYTE_DATA};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Utf8,
    Utf16Le,
    Utf16Be,
    SingleByte(usize),
    Gbk,
    Gb18030,
    Big5,
    EucJp,
    Iso2022Jp,
    ShiftJis,
    EucKr,
    Replacement,
    XUserDefined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encoding {
    name: &'static str,
    kind: Kind,
}

pub const UTF_8: Encoding = Encoding {
    name: "UTF-8",
    kind: Kind::Utf8,
};

static JIS0208: &[u8] = include_bytes!("../charset-tables/jis0208.bin");
static JIS0212: &[u8] = include_bytes!("../charset-tables/jis0212.bin");
static BIG5: &[u8] = include_bytes!("../charset-tables/big5.bin");
static EUC_KR: &[u8] = include_bytes!("../charset-tables/euc-kr.bin");
static GB18030: &[u8] = include_bytes!("../charset-tables/gb18030.bin");
static GB18030_RANGES: &[u8] = include_bytes!("../charset-tables/gb18030-ranges.bin");

const NONE: u32 = u32::MAX;

fn entry(table: &[u8], pointer: usize) -> Option<u32> {
    let bytes = table.get(pointer * 4..pointer * 4 + 4)?;
    let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    (value != NONE).then_some(value)
}

fn ranges() -> &'static [(u32, u32)] {
    static CELL: OnceLock<Vec<(u32, u32)>> = OnceLock::new();
    CELL.get_or_init(|| {
        GB18030_RANGES
            .chunks_exact(8)
            .map(|c| {
                (
                    u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                    u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                )
            })
            .collect()
    })
}

impl Encoding {
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The encoding a label names, per the standard's label table: ASCII-whitespace trimmed and
    /// ASCII case-insensitive. Unknown labels are `None`, never a guess.
    pub fn for_label(label: &[u8]) -> Option<Encoding> {
        let is_space = |b: &u8| matches!(b, b'\t' | b'\n' | 0x0c | b'\r' | b' ');
        let start = label.iter().position(|b| !is_space(b))?;
        let end = label.iter().rposition(|b| !is_space(b))? + 1;
        let wanted = std::str::from_utf8(&label[start..end])
            .ok()?
            .to_ascii_lowercase();
        let (_, at) = LABELS.iter().find(|(l, _)| *l == wanted)?;
        let (name, kind) = ENCODINGS[*at];
        Some(Encoding { name, kind })
    }

    /// The encoding a leading byte order mark selects, and the mark's length.
    pub fn for_bom(bytes: &[u8]) -> Option<(Encoding, usize)> {
        let find = |name: &str| {
            ENCODINGS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(n, k)| Encoding { name: n, kind: *k })
        };
        if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            Some((UTF_8, 3))
        } else if bytes.starts_with(&[0xfe, 0xff]) {
            Some((find("UTF-16BE")?, 2))
        } else if bytes.starts_with(&[0xff, 0xfe]) {
            Some((find("UTF-16LE")?, 2))
        } else {
            None
        }
    }

    /// Decodes `bytes` as given (a byte order mark is ordinary data here). The flag is true when any
    /// sequence was invalid; each is replaced by U+FFFD exactly where the standard replaces it.
    pub fn decode_without_bom_handling(&self, bytes: &[u8]) -> (String, bool) {
        match self.kind {
            Kind::Utf8 => {
                let invalid = std::str::from_utf8(bytes).is_err();
                (String::from_utf8_lossy(bytes).into_owned(), invalid)
            }
            Kind::Utf16Le => utf16(bytes, false),
            Kind::Utf16Be => utf16(bytes, true),
            Kind::SingleByte(i) => single_byte(bytes, SINGLE_BYTE_DATA[i]),
            Kind::XUserDefined => (
                bytes
                    .iter()
                    .map(|&b| {
                        if b < 0x80 {
                            b as char
                        } else {
                            char::from_u32(0xf780 + b as u32 - 0x80).unwrap()
                        }
                    })
                    .collect(),
                false,
            ),
            Kind::Replacement => {
                if bytes.is_empty() {
                    (String::new(), false)
                } else {
                    ("\u{fffd}".into(), true)
                }
            }
            Kind::Gbk | Kind::Gb18030 => gb18030(bytes),
            Kind::Big5 => big5(bytes),
            Kind::EucJp => euc_jp(bytes),
            Kind::ShiftJis => shift_jis(bytes),
            Kind::EucKr => euc_kr(bytes),
            Kind::Iso2022Jp => iso_2022_jp(bytes),
        }
    }
}

/// Accumulates code points and the error flag; `fffd` records one invalid sequence.
struct Out {
    text: String,
    errors: bool,
}

impl Out {
    fn new() -> Out {
        Out {
            text: String::new(),
            errors: false,
        }
    }
    fn cp(&mut self, cp: u32) {
        self.text.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
    }
    fn fffd(&mut self) {
        self.text.push('\u{fffd}');
        self.errors = true;
    }
    fn done(self) -> (String, bool) {
        (self.text, self.errors)
    }
}

fn utf16(bytes: &[u8], big: bool) -> (String, bool) {
    let mut out = Out::new();
    let unit = |i: usize| -> u16 {
        if big {
            u16::from_be_bytes([bytes[i], bytes[i + 1]])
        } else {
            u16::from_le_bytes([bytes[i], bytes[i + 1]])
        }
    };
    let mut i = 0;
    while i + 1 < bytes.len() {
        let u = unit(i);
        i += 2;
        if (0xd800..0xdc00).contains(&u) {
            if i + 1 < bytes.len() {
                let next = unit(i);
                if (0xdc00..0xe000).contains(&next) {
                    i += 2;
                    out.cp(0x10000 + ((u as u32 - 0xd800) << 10) + (next as u32 - 0xdc00));
                    continue;
                }
            }
            out.fffd();
            if i + 1 >= bytes.len() {
                // End of input with a pending lead surrogate (and perhaps an odd byte): one error.
                i = bytes.len();
            }
        } else if (0xdc00..0xe000).contains(&u) {
            out.fffd();
        } else {
            out.cp(u as u32);
        }
    }
    if i < bytes.len() {
        out.fffd();
    }
    out.done()
}

fn single_byte(bytes: &[u8], table: &[u8]) -> (String, bool) {
    let mut out = Out::new();
    for &b in bytes {
        if b < 0x80 {
            out.cp(b as u32);
        } else {
            match entry(table, (b - 0x80) as usize) {
                Some(cp) => out.cp(cp),
                None => out.fffd(),
            }
        }
    }
    out.done()
}

/// A byte stream where a decoder may push bytes back (the standard's "prepend to stream").
struct Stream<'a> {
    bytes: &'a [u8],
    at: usize,
    pushed: Vec<u8>,
}

impl<'a> Stream<'a> {
    fn new(bytes: &'a [u8]) -> Stream<'a> {
        Stream {
            bytes,
            at: 0,
            pushed: Vec::new(),
        }
    }
    fn next(&mut self) -> Option<u8> {
        if let Some(b) = self.pushed.pop() {
            return Some(b);
        }
        let b = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(b)
    }
    /// Puts `bytes` back so that the first of them is read next.
    fn prepend(&mut self, bytes: &[u8]) {
        self.pushed.extend(bytes.iter().rev());
    }
}

fn gb18030(bytes: &[u8]) -> (String, bool) {
    let mut out = Out::new();
    let mut s = Stream::new(bytes);
    while let Some(first) = s.next() {
        if first < 0x80 {
            out.cp(first as u32);
        } else if first == 0x80 {
            out.cp(0x20ac);
        } else if (0x81..=0xfe).contains(&first) {
            let Some(second) = s.next() else {
                out.fffd();
                break;
            };
            if (0x30..=0x39).contains(&second) {
                let Some(third) = s.next() else {
                    out.fffd();
                    break;
                };
                if !(0x81..=0xfe).contains(&third) {
                    s.prepend(&[second, third]);
                    out.fffd();
                    continue;
                }
                let Some(fourth) = s.next() else {
                    out.fffd();
                    break;
                };
                if !(0x30..=0x39).contains(&fourth) {
                    s.prepend(&[second, third, fourth]);
                    out.fffd();
                    continue;
                }
                let pointer = (first as u32 - 0x81) * 10 * 126 * 10
                    + (second as u32 - 0x30) * 10 * 126
                    + (third as u32 - 0x81) * 10
                    + (fourth as u32 - 0x30);
                match range_code_point(pointer) {
                    Some(cp) => out.cp(cp),
                    None => out.fffd(),
                }
            } else {
                let offset = if second < 0x7f { 0x40 } else { 0x41 };
                let pointer = ((0x40..=0x7e).contains(&second) || (0x80..=0xfe).contains(&second))
                    .then(|| (first as usize - 0x81) * 190 + (second as usize - offset));
                match pointer.and_then(|p| entry(GB18030, p)) {
                    Some(cp) => out.cp(cp),
                    None => {
                        if second < 0x80 {
                            s.prepend(&[second]);
                        }
                        out.fffd();
                    }
                }
            }
        } else {
            out.fffd();
        }
    }
    out.done()
}

fn range_code_point(pointer: u32) -> Option<u32> {
    if (pointer > 39419 && pointer < 189000) || pointer > 1237575 {
        return None;
    }
    if pointer == 7457 {
        return Some(0xe7c7);
    }
    let (offset, cp) = ranges().iter().rev().find(|(p, _)| *p <= pointer)?;
    Some(cp + pointer - offset)
}

fn big5(bytes: &[u8]) -> (String, bool) {
    let mut out = Out::new();
    let mut s = Stream::new(bytes);
    while let Some(b) = s.next() {
        if b < 0x80 {
            out.cp(b as u32);
        } else if (0x81..=0xfe).contains(&b) {
            let Some(next) = s.next() else {
                out.fffd();
                break;
            };
            let offset = if next < 0x7f { 0x40 } else { 0x62 };
            let pointer = ((0x40..=0x7e).contains(&next) || (0xa1..=0xfe).contains(&next))
                .then(|| (b as usize - 0x81) * 157 + (next as usize - offset));
            let pair = match pointer {
                Some(1133) => Some((0x00ca, 0x0304)),
                Some(1135) => Some((0x00ca, 0x030c)),
                Some(1164) => Some((0x00ea, 0x0304)),
                Some(1166) => Some((0x00ea, 0x030c)),
                _ => None,
            };
            if let Some((a, c)) = pair {
                out.cp(a);
                out.cp(c);
            } else if let Some(cp) = pointer.and_then(|p| entry(BIG5, p)) {
                out.cp(cp);
            } else {
                if next < 0x80 {
                    s.prepend(&[next]);
                }
                out.fffd();
            }
        } else {
            out.fffd();
        }
    }
    out.done()
}

fn euc_kr(bytes: &[u8]) -> (String, bool) {
    let mut out = Out::new();
    let mut s = Stream::new(bytes);
    while let Some(b) = s.next() {
        if b < 0x80 {
            out.cp(b as u32);
        } else if (0x81..=0xfe).contains(&b) {
            let Some(next) = s.next() else {
                out.fffd();
                break;
            };
            let cp = (0x41..=0xfe)
                .contains(&next)
                .then(|| (b as usize - 0x81) * 190 + (next as usize - 0x41))
                .and_then(|p| entry(EUC_KR, p));
            match cp {
                Some(cp) => out.cp(cp),
                None => {
                    if next < 0x80 {
                        s.prepend(&[next]);
                    }
                    out.fffd();
                }
            }
        } else {
            out.fffd();
        }
    }
    out.done()
}

fn euc_jp(bytes: &[u8]) -> (String, bool) {
    let mut out = Out::new();
    let mut s = Stream::new(bytes);
    while let Some(b) = s.next() {
        if b < 0x80 {
            out.cp(b as u32);
        } else if b == 0x8e || b == 0x8f || (0xa1..=0xfe).contains(&b) {
            let Some(next) = s.next() else {
                out.fffd();
                break;
            };
            if b == 0x8e && (0xa1..=0xdf).contains(&next) {
                out.cp(0xff61 - 0xa1 + next as u32);
            } else if b == 0x8f && (0xa1..=0xfe).contains(&next) {
                // Three-byte form: JIS X 0212 lead, then the trail.
                let Some(trail) = s.next() else {
                    out.fffd();
                    break;
                };
                let cp = (0xa1..=0xfe)
                    .contains(&trail)
                    .then(|| (next as usize - 0xa1) * 94 + (trail as usize - 0xa1))
                    .and_then(|p| entry(JIS0212, p));
                match cp {
                    Some(cp) => out.cp(cp),
                    None => {
                        if trail < 0x80 {
                            s.prepend(&[trail]);
                        }
                        out.fffd();
                    }
                }
            } else {
                let cp = (b != 0x8e && b != 0x8f && (0xa1..=0xfe).contains(&next))
                    .then(|| (b as usize - 0xa1) * 94 + (next as usize - 0xa1))
                    .and_then(|p| entry(JIS0208, p));
                match cp {
                    Some(cp) => out.cp(cp),
                    None => {
                        if next < 0x80 {
                            s.prepend(&[next]);
                        }
                        out.fffd();
                    }
                }
            }
        } else {
            out.fffd();
        }
    }
    out.done()
}

fn shift_jis(bytes: &[u8]) -> (String, bool) {
    let mut out = Out::new();
    let mut s = Stream::new(bytes);
    while let Some(b) = s.next() {
        if b <= 0x80 {
            out.cp(b as u32);
        } else if (0xa1..=0xdf).contains(&b) {
            out.cp(0xff61 - 0xa1 + b as u32);
        } else if (0x81..=0x9f).contains(&b) || (0xe0..=0xfc).contains(&b) {
            let Some(next) = s.next() else {
                out.fffd();
                break;
            };
            let offset = if next < 0x7f { 0x40 } else { 0x41 };
            let lead_offset = if b < 0xa0 { 0x81 } else { 0xc1 };
            let pointer = ((0x40..=0x7e).contains(&next) || (0x80..=0xfc).contains(&next))
                .then(|| (b as usize - lead_offset) * 188 + next as usize - offset);
            match pointer {
                Some(p) if (8836..=10715).contains(&p) => out.cp(0xe000 - 8836 + p as u32),
                _ => match pointer.and_then(|p| entry(JIS0208, p)) {
                    Some(cp) => out.cp(cp),
                    None => {
                        if next < 0x80 {
                            s.prepend(&[next]);
                        }
                        out.fffd();
                    }
                },
            }
        } else {
            out.fffd();
        }
    }
    out.done()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Iso2022State {
    Ascii,
    Roman,
    Katakana,
    LeadByte,
    TrailByte,
    EscapeStart,
    Escape,
}

fn iso_2022_jp(bytes: &[u8]) -> (String, bool) {
    use Iso2022State::*;
    let mut out = Out::new();
    let mut s = Stream::new(bytes);
    let (mut state, mut output_state, mut output_flag, mut lead) = (Ascii, Ascii, false, 0u8);
    loop {
        let byte = s.next();
        match state {
            Ascii => match byte {
                Some(0x1b) => state = EscapeStart,
                Some(b) if b <= 0x7f && b != 0x0e && b != 0x0f => {
                    output_flag = false;
                    out.cp(b as u32);
                }
                None => break,
                Some(_) => {
                    output_flag = false;
                    out.fffd();
                }
            },
            Roman => match byte {
                Some(0x1b) => state = EscapeStart,
                Some(0x5c) => {
                    output_flag = false;
                    out.cp(0xa5);
                }
                Some(0x7e) => {
                    output_flag = false;
                    out.cp(0x203e);
                }
                Some(b) if b <= 0x7f && b != 0x0e && b != 0x0f => {
                    output_flag = false;
                    out.cp(b as u32);
                }
                None => break,
                Some(_) => {
                    output_flag = false;
                    out.fffd();
                }
            },
            Katakana => match byte {
                Some(0x1b) => state = EscapeStart,
                Some(b) if (0x21..=0x5f).contains(&b) => {
                    output_flag = false;
                    out.cp(0xff61 - 0x21 + b as u32);
                }
                None => break,
                Some(_) => {
                    output_flag = false;
                    out.fffd();
                }
            },
            LeadByte => match byte {
                Some(0x1b) => state = EscapeStart,
                Some(b) if (0x21..=0x7e).contains(&b) => {
                    output_flag = false;
                    lead = b;
                    state = TrailByte;
                }
                None => break,
                Some(_) => {
                    output_flag = false;
                    out.fffd();
                }
            },
            TrailByte => match byte {
                Some(0x1b) => {
                    state = EscapeStart;
                    out.fffd();
                }
                Some(b) if (0x21..=0x7e).contains(&b) => {
                    state = LeadByte;
                    let pointer = (lead as usize - 0x21) * 94 + b as usize - 0x21;
                    match entry(JIS0208, pointer) {
                        Some(cp) => out.cp(cp),
                        None => out.fffd(),
                    }
                }
                _ => {
                    state = LeadByte;
                    out.fffd();
                }
            },
            EscapeStart => match byte {
                Some(b @ (0x24 | 0x28)) => {
                    lead = b;
                    state = Escape;
                }
                other => {
                    if let Some(b) = other {
                        s.prepend(&[b]);
                    }
                    output_flag = false;
                    state = output_state;
                    out.fffd();
                }
            },
            Escape => {
                let lead_ = lead;
                lead = 0;
                let next = match (lead_, byte) {
                    (0x28, Some(0x42)) => Some(Ascii),
                    (0x28, Some(0x4a)) => Some(Roman),
                    (0x28, Some(0x49)) => Some(Katakana),
                    (0x24, Some(0x40 | 0x42)) => Some(LeadByte),
                    _ => None,
                };
                match next {
                    Some(new_state) => {
                        state = new_state;
                        output_state = new_state;
                        let previous = output_flag;
                        output_flag = true;
                        if previous {
                            out.fffd();
                        }
                    }
                    None => {
                        match byte {
                            Some(b) => s.prepend(&[lead_, b]),
                            None => s.prepend(&[lead_]),
                        }
                        output_flag = false;
                        state = output_state;
                        out.fffd();
                    }
                }
            }
        }
    }
    out.done()
}
