//! Local capture integrity for stored commerce projections. Never performs network IO.
use crate::sha256::Sha256;
use serde_json::Value;
use std::{fs::File, io::Read, path::Path};

pub(crate) const UNAVAILABLE: &str = "RAW_CAPTURE_HASH_UNAVAILABLE_OR_MISMATCH";
const MAX_CAPTURE: u64 = 8 * 1024 * 1024;

pub(crate) struct Verifier<'a> {
    root: &'a Path,
    checked: std::collections::BTreeMap<(String, String), bool>,
}
impl<'a> Verifier<'a> {
    pub(crate) fn new(root: &'a Path) -> Self {
        Self {
            root,
            checked: Default::default(),
        }
    }
    pub(crate) fn verify(&mut self, observation: &Value) -> Result<(), String> {
        let key = (
            observation["source_type"].to_string(),
            observation["raw_hash"].to_string(),
        );
        let valid = *self
            .checked
            .entry(key)
            .or_insert_with(|| verify(self.root, observation).is_ok());
        if valid {
            Ok(())
        } else {
            Err(UNAVAILABLE.into())
        }
    }
}

pub(crate) fn verify(root: &Path, observation: &Value) -> Result<(), String> {
    read_verified(root, observation).map(|_| ())
}

/// The stored capture bytes of an observation, only when they match its recorded hash.
pub(crate) fn read_verified(root: &Path, observation: &Value) -> Result<Vec<u8>, String> {
    let hash = observation["raw_hash"]
        .as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
        .ok_or(UNAVAILABLE)?;
    let path = match observation["source_type"].as_str() {
        Some("PUBLIC_SOCIAL_JSON") => root
            .join(".ecdev-data/runtime/social-captures")
            .join(format!("{hash}.raw")),
        // Research runs write every public web capture, sitemaps included, as `<hash>.html`.
        Some("PUBLIC_HTML" | "PUBLIC_SITEMAP_XML") => {
            root.join(".ecdev-data/raw").join(format!("{hash}.html"))
        }
        Some("API") => root.join(".ecdev-data/raw").join(format!("{hash}.json")),
        _ => return Err(UNAVAILABLE.into()),
    };
    let file = File::open(path).map_err(|_| UNAVAILABLE)?;
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= MAX_CAPTURE)
    {
        return Err(UNAVAILABLE.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_CAPTURE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNAVAILABLE)?;
    if bytes.len() as u64 > MAX_CAPTURE || format!("{:x}", Sha256::digest(&bytes)) != hash {
        return Err(UNAVAILABLE.into());
    }
    Ok(bytes)
}

pub(crate) fn verify_payload(root: &Path, payload: &Value) -> Result<(), String> {
    if let Some(observations) = payload["observations"].as_array() {
        // Several products/posts share one response. Read each capture once per projection.
        let mut verifier = Verifier::new(root);
        for observation in observations {
            verifier.verify(observation)?;
        }
    } else if !payload["observations"].is_null() {
        return Err(UNAVAILABLE.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn captures_require_matching_bytes_in_the_declared_source_boundary() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-capture-{}",
            crate::identifier::Uuid::new_v4()
        ));
        let bytes = b"captured response";
        let hash = format!("{:x}", Sha256::digest(bytes));
        for (source, directory, extension) in [
            ("PUBLIC_HTML", ".ecdev-data/raw", "html"),
            ("PUBLIC_SITEMAP_XML", ".ecdev-data/raw", "html"),
            ("API", ".ecdev-data/raw", "json"),
            (
                "PUBLIC_SOCIAL_JSON",
                ".ecdev-data/runtime/social-captures",
                "raw",
            ),
        ] {
            let observation = json!({"source_type":source,"raw_hash":hash});
            let directory = root.join(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join(format!("{hash}.{extension}"));
            assert!(verify(&root, &observation).is_err());
            std::fs::write(&path, bytes).unwrap();
            verify(&root, &observation).unwrap();
            std::fs::write(&path, b"corrupt response").unwrap();
            assert_eq!(verify(&root, &observation).unwrap_err(), UNAVAILABLE);
            std::fs::remove_file(path).unwrap();
        }
        for hash in ["../../secret".to_string(), "A".repeat(64), "a".repeat(63)] {
            assert!(verify(&root, &json!({"source_type":"API","raw_hash":hash})).is_err());
        }
        assert!(verify(&root, &json!({"source_type":"UNKNOWN","raw_hash":hash})).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
