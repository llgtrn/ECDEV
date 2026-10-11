//! Domain-to-ASCII for the URL parser: UTS #46 Unicode IDNA Compatibility Processing with the
//! parameters the URL Standard uses outside strict mode (CheckHyphens off, CheckBidi and CheckJoiners
//! on, STD3 rules off, nontransitional, no DNS length verification), then Punycode (RFC 3492).
//!
//! Every table is committed in domain/commerce/idna-tables, generated from the Unicode Character
//! Database and IdnaMappingTable of Unicode 18.0.0 (research/commerce/native-url-parsing.json).

use std::cmp::Ordering;

static IDNA_RANGES: &[u8] = include_bytes!("../idna-tables/idna-ranges.bin");
static IDNA_MAP: &[u8] = include_bytes!("../idna-tables/idna-map.bin");
static MARKS: &[u8] = include_bytes!("../idna-tables/marks.bin");
static CCC: &[u8] = include_bytes!("../idna-tables/ccc.bin");
static NFD: &[u8] = include_bytes!("../idna-tables/nfd.bin");
static NFD_POOL: &[u8] = include_bytes!("../idna-tables/nfd-pool.bin");
static COMPOSE: &[u8] = include_bytes!("../idna-tables/compose.bin");
static BIDI: &[u8] = include_bytes!("../idna-tables/bidi.bin");
static JOINING: &[u8] = include_bytes!("../idna-tables/joining.bin");

fn word(table: &[u8], index: usize) -> u32 {
    let at = index * 4;
    u32::from_le_bytes([table[at], table[at + 1], table[at + 2], table[at + 3]])
}

/// The record of `width` words whose [start, end] range holds `cp` (records are sorted by start).
fn range_record(table: &[u8], width: usize, cp: u32) -> Option<usize> {
    let (mut lo, mut hi) = (0, table.len() / 4 / width);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let (start, end) = (word(table, mid * width), word(table, mid * width + 1));
        if cp < start {
            hi = mid;
        } else if cp > end {
            lo = mid + 1;
        } else {
            return Some(mid);
        }
    }
    None
}

fn is_mark(c: char) -> bool {
    range_record(MARKS, 2, c as u32).is_some()
}

fn ccc(c: char) -> u8 {
    range_record(CCC, 3, c as u32).map_or(0, |r| word(CCC, r * 3 + 2) as u8)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bidi {
    L,
    R,
    Al,
    En,
    Es,
    Et,
    An,
    Cs,
    Nsm,
    Bn,
    Other,
}

fn bidi_class(c: char) -> Bidi {
    // Order of the generator's class list: L R AL EN ES ET AN CS NSM BN B S WS ON LRE ...
    match range_record(BIDI, 3, c as u32).map_or(0, |r| word(BIDI, r * 3 + 2)) {
        0 => Bidi::L,
        1 => Bidi::R,
        2 => Bidi::Al,
        3 => Bidi::En,
        4 => Bidi::Es,
        5 => Bidi::Et,
        6 => Bidi::An,
        7 => Bidi::Cs,
        8 => Bidi::Nsm,
        9 => Bidi::Bn,
        _ => Bidi::Other,
    }
}

/// On is "Other Neutral" (class 13); the bidi rules name it explicitly.
fn is_other_neutral(c: char) -> bool {
    range_record(BIDI, 3, c as u32).is_some_and(|r| word(BIDI, r * 3 + 2) == 13)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Joining {
    C,
    D,
    L,
    R,
    T,
    U,
}

fn joining_type(c: char) -> Joining {
    match range_record(JOINING, 3, c as u32).map_or(5, |r| word(JOINING, r * 3 + 2)) {
        0 => Joining::C,
        1 => Joining::D,
        2 => Joining::L,
        3 => Joining::R,
        4 => Joining::T,
        _ => Joining::U,
    }
}

// ---- normalisation (NFC) ----

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

fn decompose_into(c: char, out: &mut Vec<char>) {
    let cp = c as u32;
    if (S_BASE..S_BASE + S_COUNT).contains(&cp) {
        let s = cp - S_BASE;
        out.push(char::from_u32(L_BASE + s / N_COUNT).unwrap());
        out.push(char::from_u32(V_BASE + (s % N_COUNT) / T_COUNT).unwrap());
        if !s.is_multiple_of(T_COUNT) {
            out.push(char::from_u32(T_BASE + s % T_COUNT).unwrap());
        }
        return;
    }
    let (mut lo, mut hi) = (0, NFD.len() / 12);
    while lo < hi {
        let mid = (lo + hi) / 2;
        match word(NFD, mid * 3).cmp(&cp) {
            Ordering::Less => lo = mid + 1,
            Ordering::Greater => hi = mid,
            Ordering::Equal => {
                let (offset, len) = (
                    word(NFD, mid * 3 + 1) as usize,
                    word(NFD, mid * 3 + 2) as usize,
                );
                out.extend(
                    (offset..offset + len).map(|i| char::from_u32(word(NFD_POOL, i)).unwrap()),
                );
                return;
            }
        }
    }
    out.push(c);
}

fn compose_pair(a: char, b: char) -> Option<char> {
    let (a32, b32) = (a as u32, b as u32);
    if (L_BASE..L_BASE + L_COUNT).contains(&a32) && (V_BASE..V_BASE + V_COUNT).contains(&b32) {
        return char::from_u32(S_BASE + ((a32 - L_BASE) * V_COUNT + (b32 - V_BASE)) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&a32)
        && (a32 - S_BASE).is_multiple_of(T_COUNT)
        && (T_BASE + 1..T_BASE + T_COUNT).contains(&b32)
    {
        return char::from_u32(a32 + (b32 - T_BASE));
    }
    let (mut lo, mut hi) = (0, COMPOSE.len() / 12);
    while lo < hi {
        let mid = (lo + hi) / 2;
        match (word(COMPOSE, mid * 3), word(COMPOSE, mid * 3 + 1)).cmp(&(a32, b32)) {
            Ordering::Less => lo = mid + 1,
            Ordering::Greater => hi = mid,
            Ordering::Equal => return char::from_u32(word(COMPOSE, mid * 3 + 2)),
        }
    }
    None
}

fn nfc(input: &str) -> String {
    if input.is_ascii() {
        return input.to_string();
    }
    let mut d = Vec::with_capacity(input.len());
    for c in input.chars() {
        decompose_into(c, &mut d);
    }
    // Canonical ordering: stable sort of each run of non-starters by combining class.
    let mut i = 0;
    while i < d.len() {
        if ccc(d[i]) == 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < d.len() && ccc(d[i]) != 0 {
            i += 1;
        }
        d[start..i].sort_by_key(|c| ccc(*c));
    }
    // Canonical composition: a character composes with the last starter unless a character with an
    // equal or higher combining class (or another starter) stands between them.
    let mut out: Vec<char> = Vec::with_capacity(d.len());
    let mut starter: Option<usize> = None;
    let mut last_class: Option<u8> = None;
    for c in d {
        let class = ccc(c);
        if let Some(s) = starter
            && last_class.is_none_or(|last| last < class)
            && let Some(composite) = compose_pair(out[s], c)
        {
            out[s] = composite;
            continue;
        }
        if class == 0 {
            starter = Some(out.len());
            last_class = None;
        } else {
            last_class = Some(class);
        }
        out.push(c);
    }
    out.into_iter().collect()
}

// ---- Punycode (RFC 3492) ----

const BASE: u32 = 36;

pub fn punycode_decode(input: &str) -> Option<String> {
    // A delimiter at the very start leaves no basic part and is itself an invalid digit.
    let (basic, extended) = match input.rfind('-') {
        Some(i) if i > 0 => (&input[..i], &input[i + 1..]),
        _ => ("", input),
    };
    if !basic.is_ascii() {
        return None;
    }
    let mut output: Vec<char> = basic.chars().collect();
    let (mut n, mut i, mut bias) = (128u32, 0u32, 72u32);
    let mut chars = extended.chars().peekable();
    while chars.peek().is_some() {
        let old_i = i;
        let (mut w, mut k) = (1u32, BASE);
        loop {
            let c = chars.next()?;
            let digit = match c {
                'a'..='z' => c as u32 - 'a' as u32,
                'A'..='Z' => c as u32 - 'A' as u32,
                '0'..='9' => c as u32 - '0' as u32 + 26,
                _ => return None,
            };
            i = i.checked_add(digit.checked_mul(w)?)?;
            let t = if k <= bias {
                1
            } else if k >= bias + 26 {
                26
            } else {
                k - bias
            };
            if digit < t {
                break;
            }
            w = w.checked_mul(BASE - t)?;
            k += BASE;
        }
        let len = output.len() as u32 + 1;
        bias = adapt(i - old_i, len, old_i == 0);
        n = n.checked_add(i / len)?;
        i %= len;
        output.insert(i as usize, char::from_u32(n)?);
        i += 1;
    }
    Some(output.into_iter().collect())
}

fn adapt(delta: u32, points: u32, first: bool) -> u32 {
    let mut delta = if first { delta / 700 } else { delta / 2 };
    delta += delta / points;
    let mut k = 0;
    while delta > 455 {
        delta /= 35;
        k += BASE;
    }
    k + 36 * delta / (delta + 38)
}

fn punycode_encode(input: &str) -> Option<String> {
    let chars: Vec<u32> = input.chars().map(|c| c as u32).collect();
    let mut output: String = input.chars().filter(char::is_ascii).collect();
    let basic = output.len() as u32;
    let mut handled = basic;
    if basic > 0 {
        output.push('-');
    }
    let (mut n, mut delta, mut bias) = (128u32, 0u32, 72u32);
    let digit = |d: u32| {
        char::from_u32(if d < 26 {
            b'a' as u32 + d
        } else {
            b'0' as u32 + d - 26
        })
        .unwrap()
    };
    while (handled as usize) < chars.len() {
        let m = chars.iter().copied().filter(|&c| c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(handled + 1)?)?;
        n = m;
        for &c in &chars {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        1
                    } else if k >= bias + 26 {
                        26
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    output.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                output.push(digit(q));
                bias = adapt(delta, handled + 1, handled == basic);
                delta = 0;
                handled += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    Some(output)
}

// ---- UTS #46 ----

const VALID: u32 = 0;
const IGNORED: u32 = 1;
const MAPPED: u32 = 2;
const DEVIATION: u32 = 3;
const DISALLOWED_STD3_VALID: u32 = 5;
const DISALLOWED_STD3_MAPPED: u32 = 6;

fn status(c: char) -> Option<(u32, usize, usize)> {
    let r = range_record(IDNA_RANGES, 4, c as u32)?;
    let flags = word(IDNA_RANGES, r * 4 + 2);
    Some((
        flags >> 24,
        (flags & 0x00ff_ffff) as usize,
        word(IDNA_RANGES, r * 4 + 3) as usize,
    ))
}

/// Step 1 of the processing: map, ignore or refuse each code point.
fn map(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        let (st, len, offset) = status(c)?;
        match st {
            VALID | DEVIATION | DISALLOWED_STD3_VALID => out.push(c),
            IGNORED => {}
            MAPPED | DISALLOWED_STD3_MAPPED => {
                for i in offset..offset + len {
                    out.push(char::from_u32(word(IDNA_MAP, i))?);
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

fn valid_in_label(c: char) -> bool {
    matches!(
        status(c),
        Some((VALID | DEVIATION | DISALLOWED_STD3_VALID, _, _))
    )
}

fn context_j_ok(label: &[char]) -> bool {
    for (i, &c) in label.iter().enumerate() {
        if c != '\u{200c}' && c != '\u{200d}' {
            continue;
        }
        if i > 0 && ccc(label[i - 1]) == 9 {
            continue;
        }
        if c == '\u{200d}' {
            return false;
        }
        // ZWNJ: (L|D) T* ZWNJ T* (R|D)
        let mut before = false;
        for &p in label[..i].iter().rev() {
            match joining_type(p) {
                Joining::T => continue,
                Joining::L | Joining::D => {
                    before = true;
                    break;
                }
                _ => break,
            }
        }
        let mut after = false;
        for &n in &label[i + 1..] {
            match joining_type(n) {
                Joining::T => continue,
                Joining::R | Joining::D => {
                    after = true;
                    break;
                }
                _ => break,
            }
        }
        if !(before && after) {
            return false;
        }
    }
    true
}

fn is_rtl_label(label: &[char]) -> bool {
    label
        .iter()
        .any(|&c| matches!(bidi_class(c), Bidi::R | Bidi::Al | Bidi::An))
}

fn bidi_label_ok(label: &[char]) -> bool {
    let Some(&first) = label.first() else {
        return true;
    };
    let rtl = match bidi_class(first) {
        Bidi::R | Bidi::Al => true,
        Bidi::L => false,
        _ => return false,
    };
    let classes: Vec<Bidi> = label.iter().map(|&c| bidi_class(c)).collect();
    let neutral = |i: usize| is_other_neutral(label[i]);
    if rtl {
        for (i, c) in classes.iter().enumerate() {
            if !(matches!(
                c,
                Bidi::R
                    | Bidi::Al
                    | Bidi::An
                    | Bidi::En
                    | Bidi::Es
                    | Bidi::Cs
                    | Bidi::Et
                    | Bidi::Bn
                    | Bidi::Nsm
            ) || neutral(i))
            {
                return false;
            }
        }
        let mut end = classes.len();
        while end > 0 && classes[end - 1] == Bidi::Nsm {
            end -= 1;
        }
        if end == 0 || !matches!(classes[end - 1], Bidi::R | Bidi::Al | Bidi::En | Bidi::An) {
            return false;
        }
        let (en, an) = (classes.contains(&Bidi::En), classes.contains(&Bidi::An));
        !(en && an)
    } else {
        for (i, c) in classes.iter().enumerate() {
            if !(matches!(
                c,
                Bidi::L | Bidi::En | Bidi::Es | Bidi::Cs | Bidi::Et | Bidi::Bn | Bidi::Nsm
            ) || neutral(i))
            {
                return false;
            }
        }
        let mut end = classes.len();
        while end > 0 && classes[end - 1] == Bidi::Nsm {
            end -= 1;
        }
        end > 0 && matches!(classes[end - 1], Bidi::L | Bidi::En)
    }
}

/// The URL Standard's "domain to ASCII" outside strict mode. `None` is a failure.
pub fn domain_to_ascii(domain: &str) -> Option<String> {
    let lowered_ascii = domain.is_ascii()
        && !domain
            .split('.')
            .any(|l| l.len() >= 4 && l[..4].eq_ignore_ascii_case("xn--"));
    if lowered_ascii {
        return Some(domain.to_ascii_lowercase());
    }
    let mapped = nfc(&map(domain)?);
    let mut labels: Vec<Vec<char>> = vec![];
    for label in mapped.split('.') {
        if let Some(rest) = label.strip_prefix("xn--") {
            let decoded = punycode_decode(rest)?;
            if !rest.is_empty() && decoded.is_ascii() {
                return None;
            }
            let chars: Vec<char> = decoded.chars().collect();
            if nfc(&decoded) != decoded
                || chars.contains(&'.')
                || !chars.iter().all(|&c| valid_in_label(c))
            {
                return None;
            }
            labels.push(chars);
        } else {
            labels.push(label.chars().collect());
        }
    }
    for label in &labels {
        if label.first().is_some_and(|&c| is_mark(c)) || !context_j_ok(label) {
            return None;
        }
    }
    if labels.iter().any(|l| is_rtl_label(l)) && !labels.iter().all(|l| bidi_label_ok(l)) {
        return None;
    }
    let mut out = Vec::with_capacity(labels.len());
    for label in labels {
        let text: String = label.iter().collect();
        out.push(if text.is_ascii() {
            text
        } else {
            format!("xn--{}", punycode_encode(&text)?)
        });
    }
    Some(out.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punycode_matches_the_rfc_3492_samples() {
        for (unicode, encoded) in [
            ("bücher", "bcher-kva"),
            ("münchen", "mnchen-3ya"),
            ("他们为什么不说中文", "ihqwcrb4cv8a8dqg056pqjye"),
            ("ليهمابتكلموشعربي؟", "egbpdaj6bu4bxfgehfvwxn"),
            ("Pročprostěnemluvíčesky", "Proprostnemluvesky-uyb24dma41a"),
        ] {
            assert_eq!(
                punycode_encode(unicode).as_deref(),
                Some(encoded),
                "{unicode}"
            );
            assert_eq!(
                punycode_decode(encoded).as_deref(),
                Some(unicode),
                "{encoded}"
            );
        }
        assert_eq!(
            punycode_decode("-abc"),
            None,
            "a leading delimiter is not a basic part"
        );
    }

    #[test]
    fn normalisation_composes_reorders_and_handles_hangul() {
        assert_eq!(nfc("e\u{0301}"), "\u{e9}");
        assert_eq!(nfc("a\u{0323}\u{0307}"), "\u{1ea1}\u{0307}");
        assert_eq!(
            nfc("a\u{0307}\u{0323}"),
            "\u{1ea1}\u{0307}",
            "combining marks are reordered before composing"
        );
        assert_eq!(nfc("\u{1100}\u{1161}\u{11a8}"), "\u{ac01}");
        assert_eq!(nfc("\u{ac01}"), "\u{ac01}");
        assert_eq!(
            nfc("\u{212b}"),
            "\u{c5}",
            "the angstrom sign normalises to A with ring"
        );
    }

    #[test]
    fn mapping_follows_the_idna_table() {
        assert_eq!(
            domain_to_ascii("Ｅｘａｍｐｌｅ．ＣＯＭ").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            domain_to_ascii("a\u{00ad}b.example").as_deref(),
            Some("ab.example"),
            "soft hyphen is ignored"
        );
        assert_eq!(
            domain_to_ascii("ǆ.example").as_deref(),
            Some("xn--d-toa.example")
        );
        assert_eq!(
            domain_to_ascii("\u{2028}x"),
            None,
            "disallowed code points fail"
        );
    }

    #[test]
    fn bidi_labels_follow_rfc_5893() {
        assert!(
            domain_to_ascii("\u{5d0}\u{5d1}\u{5d2}.example").is_some(),
            "a Hebrew label with LTR neighbours is a valid bidi domain"
        );
        assert!(
            domain_to_ascii("\u{5d0}1\u{5d2}\u{661}.example").is_none(),
            "EN and AN may not mix in an RTL label"
        );
        assert!(
            domain_to_ascii("a\u{5d0}.example").is_none(),
            "an LTR label with an R character"
        );
    }
}
