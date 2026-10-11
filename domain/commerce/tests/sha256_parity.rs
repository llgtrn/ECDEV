//! The native SHA-256 reproduces sha2 0.10.9 (recorded before that crate left the workspace) on
//! 2242 inputs: every length 0 to 2200 across all block boundaries, 40 seeded inputs up to 70000
//! bytes and one million 'a'. Chunked updates must equal one-shot digests too.
use ecdev_core::sha256::Sha256;

fn inputs() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = vec![];
    for len in 0..=2200usize {
        v.push((0..len).map(|i| ((i * 131 + len) % 251) as u8).collect());
    }
    let mut x: u64 = 0x2545f4914f6cdd1d;
    for k in 0..40usize {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let len = (x % 70_000) as usize + k;
        let mut y = x;
        v.push(
            (0..len)
                .map(|_| {
                    y ^= y << 13;
                    y ^= y >> 7;
                    y ^= y << 17;
                    (y >> 24) as u8
                })
                .collect(),
        );
    }
    v.push(vec![b'a'; 1_000_000]);
    v
}

#[test]
fn native_digests_reproduce_the_frozen_sha2_digest_of_digests() {
    let frozen: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/sha256-differential.json")).unwrap();
    let mut all = Sha256::new();
    let mut n = 0;
    for input in inputs() {
        let d = Sha256::digest(&input);
        all.update(d);
        let mut h = Sha256::new();
        for chunk in input.chunks(37) {
            h.update(chunk);
        }
        assert_eq!(h.finalize(), d);
        n += 1;
    }
    assert_eq!(n, frozen["inputs"].as_u64().unwrap());
    assert_eq!(
        format!("{:x}", all.finalize()),
        frozen["digest_of_digests"].as_str().unwrap()
    );
}
