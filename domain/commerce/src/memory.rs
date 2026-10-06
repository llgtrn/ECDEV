//! Deterministic evidence-memory semantics learned from Graphiti (Apache-2.0, getzep/graphiti
//! at 5d47d4d0182aa6350edeb435b734837a88ed9738): name similarity for entity candidates
//! (exact normalization, entropy gate, 3-gram MinHash/LSH, Jaccard), temporal invalidation of
//! superseded facts. Similarity only proposes candidates: ECDEV never
//! treats a similar name as an identity. Where Graphiti breaks equal-score ties by hash-seeded
//! set order, ECDEV picks the earliest existing candidate.

use std::collections::{BTreeMap, BTreeSet};

pub const NAME_ENTROPY_THRESHOLD: f64 = 1.5;
pub const MIN_NAME_LENGTH: usize = 6;
pub const MIN_TOKEN_COUNT: usize = 2;
pub const FUZZY_JACCARD_THRESHOLD: f64 = 0.9;
pub const MINHASH_PERMUTATIONS: u64 = 32;
pub const MINHASH_BAND_SIZE: usize = 4;

fn space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
fn collapse(s: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for c in s.chars() {
        if space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}
fn strip(s: &str) -> &str {
    s.trim_matches(space)
}

/// Lowercase and collapse whitespace so equal names map to one key.
pub fn normalize_exact(name: &str) -> String {
    strip(&collapse(&name.to_lowercase())).to_string()
}

/// Keep ASCII alphanumerics, apostrophes and spaces for shingling.
pub fn normalize_fuzzy(name: &str) -> String {
    let kept: String = normalize_exact(name)
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '\'' || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect();
    collapse(strip(&kept))
}

/// Shannon entropy over the characters of a normalized name, spaces removed.
pub fn name_entropy(normalized: &str) -> f64 {
    let mut counts: Vec<(char, u64)> = Vec::new();
    for c in normalized.chars().filter(|c| *c != ' ') {
        match counts.iter_mut().find(|(k, _)| *k == c) {
            Some((_, n)) => *n += 1,
            None => counts.push((c, 1)),
        }
    }
    let total: u64 = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return 0.0;
    }
    let mut entropy = 0.0;
    for (_, n) in counts {
        let p = n as f64 / total as f64;
        entropy -= p * p.log2();
    }
    entropy
}

/// Whether a normalized name is specific enough for fuzzy matching.
pub fn has_high_entropy(normalized: &str) -> bool {
    let tokens = normalized.split(' ').filter(|t| !t.is_empty()).count();
    if normalized.chars().count() < MIN_NAME_LENGTH && tokens < MIN_TOKEN_COUNT {
        return false;
    }
    name_entropy(normalized) >= NAME_ENTROPY_THRESHOLD
}

/// Character 3-grams of a normalized name with spaces removed.
pub fn shingles(normalized: &str) -> BTreeSet<String> {
    let cleaned: Vec<char> = normalized.chars().filter(|c| *c != ' ').collect();
    if cleaned.len() < 2 {
        return cleaned
            .first()
            .map(|c| BTreeSet::from([c.to_string()]))
            .unwrap_or_default();
    }
    (0..cleaned.len().saturating_sub(2))
        .map(|i| cleaned[i..i + 3].iter().collect())
        .collect()
}

/// BLAKE2b (RFC 7693), unkeyed, 8-byte digest, read big-endian.
pub fn blake2b64(input: &[u8]) -> u64 {
    const IV: [u64; 8] = [
        0x6a09e667f3bcc908,
        0xbb67ae8584caa73b,
        0x3c6ef372fe94f82b,
        0xa54ff53a5f1d36f1,
        0x510e527fade682d1,
        0x9b05688c2b3e6c1f,
        0x1f83d9abfb41bd6b,
        0x5be0cd19137e2179,
    ];
    const SIGMA: [[usize; 16]; 12] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
        [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
        [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
        [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
        [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
        [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
        [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
        [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
        [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    ];
    const OUT: usize = 8;
    let mut h = IV;
    h[0] ^= 0x0101_0000 ^ OUT as u64;
    let compress = |h: &mut [u64; 8], block: &[u8; 128], t: u128, last: bool| {
        let mut m = [0u64; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
        }
        let mut v = [0u64; 16];
        v[..8].copy_from_slice(h);
        v[8..].copy_from_slice(&IV);
        v[12] ^= t as u64;
        v[13] ^= (t >> 64) as u64;
        if last {
            v[14] = !v[14];
        }
        let g = |v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64| {
            v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
            v[d] = (v[d] ^ v[a]).rotate_right(32);
            v[c] = v[c].wrapping_add(v[d]);
            v[b] = (v[b] ^ v[c]).rotate_right(24);
            v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
            v[d] = (v[d] ^ v[a]).rotate_right(16);
            v[c] = v[c].wrapping_add(v[d]);
            v[b] = (v[b] ^ v[c]).rotate_right(63);
        };
        for s in &SIGMA {
            g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
            g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
            g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
            g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
            g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
            g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
            g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
            g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
        }
        for i in 0..8 {
            h[i] ^= v[i] ^ v[i + 8];
        }
    };
    let mut t: u128 = 0;
    let mut chunks = input.chunks(128).peekable();
    if input.is_empty() {
        compress(&mut h, &[0u8; 128], 0, true);
    }
    while let Some(chunk) = chunks.next() {
        let mut block = [0u8; 128];
        block[..chunk.len()].copy_from_slice(chunk);
        t += chunk.len() as u128;
        compress(&mut h, &block, t, chunks.peek().is_none());
    }
    u64::from_be_bytes(h[0].to_le_bytes())
}

/// MinHash signature over [`MINHASH_PERMUTATIONS`] seeded hashes; empty for no shingles.
pub fn minhash_signature(shingles: &BTreeSet<String>) -> Vec<u64> {
    if shingles.is_empty() {
        return Vec::new();
    }
    (0..MINHASH_PERMUTATIONS)
        .map(|seed| {
            shingles
                .iter()
                .map(|s| blake2b64(format!("{seed}:{s}").as_bytes()))
                .min()
                .unwrap()
        })
        .collect()
}

/// Complete bands of [`MINHASH_BAND_SIZE`] signature values.
pub fn lsh_bands(signature: &[u64]) -> Vec<Vec<u64>> {
    signature
        .chunks(MINHASH_BAND_SIZE)
        .filter(|b| b.len() == MINHASH_BAND_SIZE)
        .map(<[u64]>::to_vec)
        .collect()
}

pub fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    let union = a.union(b).count();
    inter as f64 / union as f64
}

/// A named entity candidate with type labels (`Entity` is the generic label).
#[derive(Clone, Debug, PartialEq)]
pub struct NamedEntity {
    pub id: String,
    pub name: String,
    pub labels: Vec<String>,
}

/// How one incoming name relates to the existing entities.
#[derive(Clone, Debug, PartialEq)]
pub enum NameMatch {
    /// Exactly one existing entity has the same normalized name.
    Exact(usize),
    /// The best LSH candidate reaches the Jaccard threshold (earliest existing on ties).
    Similar(usize, f64),
    /// Several exact matches, a low-entropy name, or nothing similar enough: undecided.
    Unresolved,
}

fn promote(existing: &mut NamedEntity, incoming: &NamedEntity) {
    if existing.labels.iter().any(|l| l != "Entity") {
        return;
    }
    let specific: Vec<&String> = incoming.labels.iter().filter(|l| *l != "Entity").collect();
    if specific.is_empty() {
        return;
    }
    let mut labels: Vec<String> = Vec::new();
    for l in std::iter::once(&"Entity".to_string())
        .chain(existing.labels.iter())
        .chain(specific)
    {
        if !labels.contains(l) {
            labels.push(l.clone());
        }
    }
    existing.labels = labels;
}

/// Matches incoming names against existing entities in order; a matched existing entity with
/// only the generic label takes the incoming entity's specific labels.
pub fn match_names(existing: &mut [NamedEntity], incoming: &[NamedEntity]) -> Vec<NameMatch> {
    let mut exact: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut shingle_sets = Vec::new();
    let mut buckets: BTreeMap<(usize, Vec<u64>), Vec<usize>> = BTreeMap::new();
    for (i, e) in existing.iter().enumerate() {
        exact.entry(normalize_exact(&e.name)).or_default().push(i);
        let sh = shingles(&normalize_fuzzy(&e.name));
        for (b, band) in lsh_bands(&minhash_signature(&sh)).into_iter().enumerate() {
            buckets.entry((b, band)).or_default().push(i);
        }
        shingle_sets.push(sh);
    }
    let mut out = Vec::new();
    for n in incoming {
        match exact.get(&normalize_exact(&n.name)).map(Vec::as_slice) {
            Some([i]) => {
                promote(&mut existing[*i], n);
                out.push(NameMatch::Exact(*i));
                continue;
            }
            Some([_, _, ..]) => {
                out.push(NameMatch::Unresolved);
                continue;
            }
            _ => {}
        }
        let fuzzy = normalize_fuzzy(&n.name);
        if !has_high_entropy(&fuzzy) {
            out.push(NameMatch::Unresolved);
            continue;
        }
        let sh = shingles(&fuzzy);
        let mut candidates = BTreeSet::new();
        for (b, band) in lsh_bands(&minhash_signature(&sh)).into_iter().enumerate() {
            if let Some(ids) = buckets.get(&(b, band)) {
                candidates.extend(ids.iter().copied());
            }
        }
        let mut best: Option<(usize, f64)> = None;
        for i in candidates {
            let score = jaccard(&sh, &shingle_sets[i]);
            if score > best.map_or(0.0, |(_, s)| s) {
                best = Some((i, score));
            }
        }
        match best {
            Some((i, score)) if score >= FUZZY_JACCARD_THRESHOLD => {
                promote(&mut existing[i], n);
                out.push(NameMatch::Similar(i, score));
            }
            _ => out.push(NameMatch::Unresolved),
        }
    }
    out
}

/// A fact's validity window; `None` is unknown.
#[derive(Clone, Debug, PartialEq)]
pub struct Validity<T> {
    pub valid_at: Option<T>,
    pub invalid_at: Option<T>,
}

/// Indices of existing facts a newer fact supersedes, with the time they stop being valid.
/// Facts whose window ends before the new one starts (or starts after it ends) are untouched;
/// an existing fact that started strictly earlier ends when the new fact starts.
pub fn superseded<T: Ord + Clone>(new: &Validity<T>, existing: &[Validity<T>]) -> Vec<(usize, T)> {
    let mut out = Vec::new();
    for (i, e) in existing.iter().enumerate() {
        let disjoint = matches!((&e.invalid_at, &new.valid_at), (Some(ei), Some(nv)) if ei <= nv)
            || matches!((&e.valid_at, &new.invalid_at), (Some(ev), Some(ni)) if ni <= ev);
        if disjoint {
            continue;
        }
        if let (Some(ev), Some(nv)) = (&e.valid_at, &new.valid_at)
            && ev < nv
        {
            out.push((i, nv.clone()));
        }
    }
    out
}
