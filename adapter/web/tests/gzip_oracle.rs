//! ECDEV's gunzip against streams made by CPython's zlib (the reference DEFLATE implementation),
//! frozen by tools/commerce/gzip_oracle.py: every block type, level and strategy, header variants,
//! concatenated members, and malformed streams. Scrapy's gunzip is recorded on the malformed ones.
use ecdev_web::gzip::{GzipError, gunzip};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/gzip-oracle.json")).unwrap()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

const LIMIT: usize = 80 * 1024 * 1024;

#[test]
fn every_valid_stream_decodes_to_the_reference_bytes() {
    let o = oracle();
    let (mut valid, mut bytes_checked) = (0, 0usize);
    for c in o["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["valid"] == true)
    {
        let out = gunzip(&unhex(c["gz"].as_str().unwrap()), LIMIT)
            .unwrap_or_else(|e| panic!("{}: {e:?}", c["name"]));
        assert_eq!(
            out.len() as u64,
            c["length"].as_u64().unwrap(),
            "{}",
            c["name"]
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(&out)),
            c["sha256"].as_str().unwrap(),
            "{}",
            c["name"]
        );
        valid += 1;
        bytes_checked += out.len();
    }
    assert!(valid >= 300, "{valid} valid cases");
    assert!(bytes_checked > 64 * 1024 * 1024);
}

#[test]
fn every_malformed_stream_is_an_error_never_partial_data() {
    let o = oracle();
    let mut refused = 0;
    let mut scrapy_returned_data = 0;
    for c in o["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["valid"] == false)
    {
        let r = gunzip(&unhex(c["gz"].as_str().unwrap()), LIMIT);
        assert!(
            r.is_err(),
            "{}: decoded to {} bytes",
            c["name"],
            r.map(|v| v.len()).unwrap_or(0)
        );
        refused += 1;
        scrapy_returned_data += c["scrapy"]["returned_bytes"].is_u64() as usize;
    }
    assert_eq!(refused, 17);
    // Scrapy's gunzip (scrapy/utils/_compression.py) hands back the bytes it decoded before an
    // error, and does not stop on a bad CRC or length: of these 17, it returned data for 10,
    // including streams cut short, with a wrong checksum and with a flipped bit.
    assert_eq!(scrapy_returned_data, 10);
}

#[test]
fn the_output_cap_bounds_a_decompression_bomb() {
    let o = oracle();
    let bomb = o["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "bomb-64MiB-of-zeros")
        .unwrap();
    let blob = unhex(bomb["gz"].as_str().unwrap());
    assert!(blob.len() < 70_000, "{} compressed bytes", blob.len());
    // 64 MiB of output from about 64 KB of input; a 1 MiB cap stops it at once.
    assert_eq!(gunzip(&blob, 1024 * 1024), Err(GzipError::TooLarge));
    assert_eq!(
        gunzip(&blob, 64 * 1024 * 1024 - 1),
        Err(GzipError::TooLarge)
    );
}

#[test]
fn a_gzip_sitemap_parses_to_the_same_entries_as_its_plain_text() {
    let o = oracle();
    let case = o["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "sitemap-70000-L9-default")
        .unwrap();
    let blob = unhex(case["gz"].as_str().unwrap());
    let plain = gunzip(&blob, LIMIT).unwrap();
    let (from_gz, from_plain) = (
        ecdev_web::sitemap::parse(&blob).unwrap(),
        ecdev_web::sitemap::parse(&plain).unwrap(),
    );
    assert!(from_plain.entries.len() > 500);
    assert_eq!(from_gz.entries, from_plain.entries);
    // Corrupting the compressed bytes refuses the sitemap whole; it never yields a prefix.
    let mut bad = blob.clone();
    let n = bad.len();
    bad[n / 2] ^= 0x40;
    assert!(ecdev_web::sitemap::parse(&bad).is_err());
}
