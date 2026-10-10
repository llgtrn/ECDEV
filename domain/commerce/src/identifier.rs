//! Random version-4 identifiers (RFC 4122) for runs, evidence and temporary names. The 16 random bytes
//! come from the operating system (`/dev/urandom` on Unix); where that is unavailable they are derived
//! by hashing std's per-process random hasher seed with the clock, process id and a counter. Run and
//! evidence ids are not secrets; the author pseudonym key reads the OS source directly and fails
//! closed if it cannot.

use crate::sha256::Sha256;
use std::{
    collections::hash_map::RandomState,
    fmt,
    hash::{BuildHasher, Hasher},
    io::Read,
    sync::atomic::{AtomicU64, Ordering},
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Uuid([u8; 16]);

/// 16 bytes from the operating system's random source, if it can be read.
pub fn os_random_16() -> Option<[u8; 16]> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut bytes)
        .ok()?;
    Some(bytes)
}

fn fallback_16() -> [u8; 16] {
    let seed = RandomState::new().build_hasher().finish();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let digest = Sha256::new()
        .chain_update(seed.to_be_bytes())
        .chain_update(nanos.to_be_bytes())
        .chain_update(std::process::id().to_be_bytes())
        .chain_update(COUNTER.fetch_add(1, Ordering::Relaxed).to_be_bytes())
        .finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

impl Uuid {
    pub fn new_v4() -> Uuid {
        let mut bytes = os_random_16().unwrap_or_else(fallback_16);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_four_variant_one_and_hyphenated_lowercase() {
        let text = Uuid::new_v4().to_string();
        assert_eq!(text.len(), 36);
        let parts: Vec<&str> = text.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
        assert!(
            text.bytes()
                .all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }

    #[test]
    fn identifiers_do_not_repeat_and_the_fallback_varies() {
        let ids: std::collections::BTreeSet<_> =
            (0..1000).map(|_| Uuid::new_v4().to_string()).collect();
        assert_eq!(ids.len(), 1000);
        assert_ne!(fallback_16(), fallback_16());
    }
}
