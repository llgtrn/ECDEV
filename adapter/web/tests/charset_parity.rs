//! Parity of the native WHATWG decoders with encoding_rs 0.8.35, frozen before that crate left the
//! workspace: for every label, the digest of the native output over empty input, every one- and
//! two-byte input and 300000 seeded multi-byte inputs equals the digest recorded from encoding_rs
//! (adapter/web/tests/fixtures/charset-differential-digest.json). The differential run itself
//! compared text and error flag input by input and found no difference.
use ecdev_core::sha256::Sha256;
use ecdev_web::text_decoding::Encoding;

fn labels() -> Vec<&'static str> {
    // One label per encoding: the canonical lower-case names plus the aliases that differ in kind.
    let mut v = vec![];
    for e in [
        "utf-8",
        "utf-16le",
        "utf-16be",
        "ibm866",
        "iso-8859-2",
        "iso-8859-3",
        "iso-8859-4",
        "iso-8859-5",
        "iso-8859-6",
        "iso-8859-7",
        "iso-8859-8",
        "iso-8859-8-i",
        "iso-8859-10",
        "iso-8859-13",
        "iso-8859-14",
        "iso-8859-15",
        "iso-8859-16",
        "koi8-r",
        "koi8-u",
        "macintosh",
        "windows-874",
        "windows-1250",
        "windows-1251",
        "windows-1252",
        "windows-1253",
        "windows-1254",
        "windows-1255",
        "windows-1256",
        "windows-1257",
        "windows-1258",
        "x-mac-cyrillic",
        "gbk",
        "gb18030",
        "big5",
        "euc-jp",
        "iso-2022-jp",
        "shift_jis",
        "euc-kr",
        "replacement",
        "x-user-defined",
        "latin1",
        "ascii",
        "sjis",
        "hz-gb-2312",
    ] {
        v.push(e);
    }
    v
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn absorb(hash: &mut Sha256, text: &str, flag: bool) {
    hash.update((text.len() as u64).to_be_bytes());
    hash.update(text.as_bytes());
    hash.update([flag as u8]);
}

#[test]
fn native_decoders_reproduce_the_frozen_encoding_rs_digests() {
    let frozen: ecdev_core::json::Value =
        ecdev_core::json::from_str(include_str!("fixtures/charset-differential-digest.json"))
            .unwrap();
    for label in labels() {
        let mine = Encoding::for_label(label.as_bytes()).unwrap_or_else(|| panic!("label {label}"));
        let mut hash = Sha256::new();
        let mut take = |input: &[u8]| {
            let (text, flag) = mine.decode_without_bom_handling(input);
            absorb(&mut hash, &text, flag);
        };
        take(&[]);
        for a in 0..=255u8 {
            take(&[a]);
            for b in 0..=255u8 {
                take(&[a, b]);
            }
        }
        let mut rng = Rng(0x9e3779b97f4a7c15 ^ label.len() as u64);
        for _ in 0..300_000 {
            let len = 3 + (rng.next() % 8) as usize;
            let input: Vec<u8> = (0..len)
                .map(|_| match rng.next() % 4 {
                    0 => (rng.next() % 128) as u8,
                    1 => 0x81 + (rng.next() % 126) as u8,
                    2 => 0x30 + (rng.next() % 10) as u8,
                    _ => (rng.next() % 256) as u8,
                })
                .collect();
            take(&input);
        }
        assert_eq!(
            format!("{:x}", hash.finalize()),
            frozen["digest"][label].as_str().unwrap(),
            "{label}"
        );
    }
}

#[test]
fn labels_and_byte_order_marks_follow_the_standard() {
    for (label, name) in [
        ("UTF8", Some("UTF-8")),
        (" Shift_JIS ", Some("Shift_JIS")),
        ("unicode-1-1-utf-8", Some("UTF-8")),
        ("csshiftjis", Some("Shift_JIS")),
        ("x-sjis", Some("Shift_JIS")),
        ("nonsense", None),
        ("", None),
        ("  ", None),
        ("ISO-8859-1", Some("windows-1252")),
        ("ISO_8859-1:1987", Some("windows-1252")),
        ("ascii", Some("windows-1252")),
        ("us-ascii", Some("windows-1252")),
        ("UTF-16", Some("UTF-16LE")),
    ] {
        assert_eq!(
            Encoding::for_label(label.as_bytes()).map(|e| e.name()),
            name,
            "{label:?}"
        );
    }
    for (bom, expected) in [
        (&[0xef, 0xbb, 0xbf, b'a'][..], Some(("UTF-8", 3))),
        (&[0xfe, 0xff, 0][..], Some(("UTF-16BE", 2))),
        (&[0xff, 0xfe, 0][..], Some(("UTF-16LE", 2))),
        (&[0xff, 0xfe, 0, 0][..], Some(("UTF-16LE", 2))),
        (&[0xef, 0xbb][..], None),
        (&[][..], None),
    ] {
        assert_eq!(
            Encoding::for_bom(bom).map(|(e, n)| (e.name(), n)),
            expected,
            "{bom:02x?}"
        );
    }
}
