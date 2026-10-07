//! Keyed author pseudonyms. Trend evidence needs to tell authors apart (distinct authors behind
//! a phrase, one author repeating himself is not recurrence), never who they are. Derived
//! records therefore carry `anon1:` + 16 hex of HMAC-SHA256(installation key, platform|handle)
//! for the author and for each @mention. The key is random per installation, kept in
//! .ecdev-data/runtime (owner read/write only, never logged or exported), so a pseudonym
//! cannot be recomputed from a public handle as an unsalted hash could. Raw captures keep the
//! source bytes unchanged: they are hashed evidence.

use super::SocialPost;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub const PREFIX: &str = "anon1:";
const KEY_FILE: &str = ".ecdev-data/runtime/author-pseudonym.key";

fn hmac_sha256(key: &[u8; 32], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    block[..32].copy_from_slice(key);
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// The installation's pseudonym key, created on first use.
pub fn installation_key(root: &Path) -> Result<[u8; 32], String> {
    let path = root.join(KEY_FILE);
    if let Ok(bytes) = fs::read(&path) {
        return <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| "AUTHOR_PSEUDONYM_KEY_CORRUPT".to_string());
    }
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    use std::io::Write;
    match options.open(&path) {
        Ok(mut f) => f.write_all(&key).map_err(|e| e.to_string())?,
        // Another writer won the race: use its key.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return installation_key(root),
        Err(e) => return Err(e.to_string()),
    }
    Ok(key)
}

pub fn pseudonym(key: &[u8; 32], platform: &str, handle: &str) -> String {
    if handle.starts_with(PREFIX) {
        return handle.to_string();
    }
    let mac = hmac_sha256(
        key,
        format!("{platform}|{}", handle.to_lowercase()).as_bytes(),
    );
    let hex: String = mac[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{PREFIX}{hex}")
}

/// Replaces the author and @mentions of a post with keyed pseudonyms; idempotent.
pub fn pseudonymise(post: &mut SocialPost, key: &[u8; 32]) {
    let platform = post.platform.clone();
    if let Some(a) = &post.author_id {
        post.author_id = Some(pseudonym(key, &platform, a));
    }
    for m in &mut post.mentions {
        *m = pseudonym(key, &platform, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231_shape_and_pseudonyms_are_keyed() {
        // RFC 4231 test case 2 uses a 4-byte key; with a 32-byte zero-padded key the HMAC of
        // "what do ya want for nothing?" under key "Jefe" is the same value.
        let mut key = [0u8; 32];
        key[..4].copy_from_slice(b"Jefe");
        let mac = hmac_sha256(&key, b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let a = pseudonym(&key, "BLUESKY", "Alice.bsky.social");
        assert!(a.starts_with(PREFIX) && a.len() == PREFIX.len() + 16);
        assert_eq!(
            a,
            pseudonym(&key, "BLUESKY", "alice.bsky.social"),
            "handles are case-insensitive"
        );
        assert_ne!(
            a,
            pseudonym(&key, "HACKER_NEWS", "alice.bsky.social"),
            "platforms are separate"
        );
        assert_ne!(
            a,
            pseudonym(&[7u8; 32], "BLUESKY", "alice.bsky.social"),
            "another installation, another pseudonym"
        );
        assert_eq!(pseudonym(&key, "BLUESKY", &a), a, "idempotent");
    }

    #[test]
    fn the_installation_key_is_created_once_and_private() {
        let root = std::env::temp_dir().join(format!("ecdev-pseudonym-{}", uuid::Uuid::new_v4()));
        let k1 = installation_key(&root).unwrap();
        assert_eq!(k1, installation_key(&root).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
        fs::write(root.join(KEY_FILE), b"short").unwrap();
        assert_eq!(
            installation_key(&root).unwrap_err(),
            "AUTHOR_PSEUDONYM_KEY_CORRUPT"
        );
        let _ = fs::remove_dir_all(root);
    }
}
