//! Evidence-first social contracts. Capture mode and evidence state are orthogonal.
pub mod coverage;
pub mod displayed;
pub mod feeds;
pub mod growth;
pub mod pagination;
pub mod runtime;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceState {
    Observed,
    Derived,
    Estimated,
    Simulated,
    Unknown,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IdentityState {
    ExactIdentifier,
    SourceAsserted,
    DerivedStrongMatch,
    DerivedWeakMatch,
    Unresolved,
    Conflict,
}
/// Equality of identifiers is scoped; topic similarity never establishes product identity.
pub fn entity_link(a: &Value, b: &Value) -> IdentityState {
    if a["kind"] != b["kind"] {
        return IdentityState::Unresolved;
    }
    let (Some(av), Some(bv)) = (a["value"].as_str(), b["value"].as_str()) else {
        return IdentityState::Unresolved;
    };
    if av != bv {
        return if a["kind"] == "GTIN" {
            IdentityState::Conflict
        } else {
            IdentityState::DerivedWeakMatch
        };
    }
    if a["kind"] == "GTIN"
        && av.bytes().all(|c| c.is_ascii_digit())
        && [8, 12, 13, 14].contains(&av.len())
    {
        let sum = av
            .bytes()
            .rev()
            .enumerate()
            .map(|(i, c)| (c - b'0') as u32 * if i % 2 == 0 { 1 } else { 3 })
            .sum::<u32>();
        return if sum % 10 == 0 {
            IdentityState::ExactIdentifier
        } else {
            IdentityState::Conflict
        };
    }
    if a["kind"] == "ASIN"
        && av.len() == 10
        && av
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && a["market"].is_string()
        && a["market"] == b["market"]
    {
        return IdentityState::ExactIdentifier;
    }
    if matches!(a["kind"].as_str(), Some("URL" | "SKU" | "MPN")) {
        IdentityState::SourceAsserted
    } else {
        IdentityState::DerivedWeakMatch
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialEngagement {
    pub views: Option<u64>,
    pub likes: Option<u64>,
    pub comments: Option<u64>,
    pub reposts: Option<u64>,
    pub favorites: Option<u64>,
    pub followers: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialPost {
    pub platform: String,
    pub provider: String,
    pub source_url: String,
    pub native_id: String,
    pub thread_id: Option<String>,
    pub author_id: Option<String>,
    pub publisher: Option<String>,
    pub published_at: Option<u64>,
    pub captured_at: u64,
    pub text: String,
    pub language: Option<String>,
    pub media: Vec<String>,
    pub hashtags: Vec<String>,
    pub mentions: Vec<String>,
    pub entities: Vec<Value>,
    pub propagation: String,
    pub parent_id: Option<String>,
    pub engagement: SocialEngagement,
    pub raw_hash: String,
    pub raw_locator: String,
    pub extraction_method: String,
    pub state: EvidenceState,
    pub capture_mode: String,
    pub evidence_id: String,
    pub freshness_seconds: Option<u64>,
    pub origin_evidence_id: Option<String>,
    /// Depth below the thread root for posts read from a reply tree; absent elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
}
impl SocialPost {
    pub fn key(&self) -> String {
        format!("{}:{}", self.platform, self.native_id)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.native_id.is_empty()
            || self.native_id.len() > 2048
            || self.platform.is_empty()
            || self.text.len() > 65536
            || self.text.chars().any(|c| c == '\0')
            || self.raw_hash.len() != 64
            || !self.raw_hash.bytes().all(|c| c.is_ascii_hexdigit())
            || !matches!(self.capture_mode.as_str(), "LIVE" | "CACHED" | "FIXTURE")
            || self.state != EvidenceState::Observed
            || self.evidence_id.is_empty()
            || !matches!(
                self.propagation.as_str(),
                "ORIGINAL"
                    | "REPLY"
                    | "REPOST"
                    | "QUOTE"
                    | "SYNDICATION"
                    | "MIRROR"
                    | "AGGREGATOR"
                    | "UNKNOWN"
            )
        {
            return Err("INVALID_OBSERVED_SOCIAL_POST".into());
        }
        let url = url::Url::parse(&self.source_url).map_err(|_| "INVALID_SOURCE_URL")?;
        if !matches!(url.scheme(), "https" | "http")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("PUBLIC_SOURCE_URL_REQUIRED".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimulatedActor {
    pub id: String,
    pub persona: String,
    pub state: EvidenceState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimulatedPost {
    pub actor_id: String,
    pub text: String,
    pub state: EvidenceState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimulatedReaction {
    pub actor_id: String,
    pub post_index: usize,
    pub state: EvidenceState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForecastScenario {
    pub id: String,
    pub seed_snapshot_id: String,
    pub question: String,
    pub population: Vec<SimulatedActor>,
    pub posts: Vec<SimulatedPost>,
    pub reactions: Vec<SimulatedReaction>,
    pub state: EvidenceState,
}
impl ForecastScenario {
    pub fn validate(&self) -> Result<(), String> {
        if self.state != EvidenceState::Simulated
            || self
                .population
                .iter()
                .any(|a| a.state != EvidenceState::Simulated)
            || self
                .posts
                .iter()
                .any(|a| a.state != EvidenceState::Simulated)
            || self
                .reactions
                .iter()
                .any(|a| a.state != EvidenceState::Simulated)
        {
            return Err("SCENARIO_MUST_BE_SIMULATED".into());
        }
        Ok(())
    }
}

/// Han, kana and Hangul: scripts written without spaces between words.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3005 | 0x3007 | 0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF
        | 0x20000..=0x2FFFF)
}

/// Lowercased text with full-width ASCII folded to ASCII ("Ｍａｔｃｈａ" reads "matcha").
fn folded(text: &str) -> String {
    text.chars()
        .map(|c| match c as u32 {
            0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            0x3000 => ' ',
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Alphanumeric runs split where CJK meets other scripts, with whether each is CJK.
fn runs(text: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = vec![];
    for word in folded(text).split(|c: char| !c.is_alphanumeric()) {
        let mut current: Option<(String, bool)> = None;
        for c in word.chars() {
            let cjk = is_cjk(c);
            match &mut current {
                Some((run, k)) if *k == cjk => run.push(c),
                _ => out.extend(current.replace((c.to_string(), cjk))),
            }
        }
        out.extend(current);
    }
    out
}

/// Index terms: Latin and other spaced words of three or more characters minus a few stop
/// words; runs of CJK, which has no spaces, as overlapping character bigrams (a lone character
/// as itself), the usual way to search unsegmented text without a dictionary.
pub fn terms(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (run, cjk) in runs(text) {
        if out.len() >= 512 {
            break;
        }
        if cjk {
            let chars: Vec<char> = run.chars().collect();
            if chars.len() == 1 {
                out.insert(run);
            } else {
                out.extend(chars.windows(2).map(|w| w.iter().collect::<String>()));
            }
        } else if run.chars().count() > 2
            && !matches!(
                run.as_str(),
                "the"
                    | "and"
                    | "for"
                    | "with"
                    | "that"
                    | "this"
                    | "from"
                    | "https"
                    | "http"
                    | "com"
            )
        {
            out.insert(run);
        }
    }
    out
}

/// Whether a text mentions a query: every query term is a term of the text, and every CJK run
/// of the query appears in the text as written (bigrams alone could be scattered).
/// Two-letter Latin words a query may name ("AI", "5G", "TV"): too short to index from text,
/// so they are matched as whole words. Common function words are not query words.
fn short_words(query: &str) -> BTreeSet<String> {
    runs(query)
        .into_iter()
        .filter(|(w, cjk)| !cjk && w.chars().count() == 2)
        .map(|(w, _)| w)
        .filter(|w| {
            !matches!(
                w.as_str(),
                "an" | "as"
                    | "at"
                    | "be"
                    | "by"
                    | "do"
                    | "go"
                    | "he"
                    | "if"
                    | "in"
                    | "is"
                    | "it"
                    | "me"
                    | "my"
                    | "no"
                    | "of"
                    | "on"
                    | "or"
                    | "so"
                    | "to"
                    | "up"
                    | "us"
                    | "we"
            )
        })
        .collect()
}

/// Whether a query names anything ECDEV can look for in text.
pub fn searchable(query: &str) -> bool {
    !terms(query).is_empty() || !short_words(query).is_empty()
}

pub fn mentions(query: &str, text: &str) -> bool {
    let short = short_words(query);
    if !short.is_empty() {
        let words: BTreeSet<String> = runs(text)
            .into_iter()
            .filter(|(_, cjk)| !cjk)
            .map(|(w, _)| w)
            .collect();
        if !short.is_subset(&words) {
            return false;
        }
        if terms(query).is_empty() {
            return runs(query)
                .into_iter()
                .filter(|(_, cjk)| *cjk)
                .all(|(run, _)| folded(text).contains(&run));
        }
    }
    let q = terms(query);
    // A lone CJK character is checked as written below; text holds it only inside bigrams.
    let indexed: BTreeSet<String> = q
        .iter()
        .filter(|t| !(t.chars().count() == 1 && t.chars().all(is_cjk)))
        .cloned()
        .collect();
    if q.is_empty() || !indexed.is_subset(&terms(text)) {
        return false;
    }
    let body = folded(text);
    runs(query)
        .into_iter()
        .filter(|(_, cjk)| *cjk)
        .all(|(run, _)| body.contains(&run))
}

pub fn sentiment(post: &SocialPost) -> Value {
    let tokens = terms(&post.text);
    let english = post
        .language
        .as_deref()
        .is_some_and(|s| s == "en" || s.starts_with("en-"));
    let positive = tokens
        .iter()
        .filter(|s| {
            matches!(
                s.as_str(),
                "love" | "excellent" | "great" | "happy" | "good"
            )
        })
        .count();
    let negative = tokens
        .iter()
        .filter(|s| matches!(s.as_str(), "hate" | "awful" | "bad" | "broken" | "terrible"))
        .count();
    let label = if !english {
        "UNKNOWN"
    } else if positive > 0 && negative > 0 {
        "MIXED"
    } else if positive > 0 {
        "POSITIVE"
    } else if negative > 0 {
        "NEGATIVE"
    } else {
        "NEUTRAL"
    };
    json!({"label":label,"method":"ECDEV_SMALL_ENGLISH_TOKEN_RULE_V1","language":post.language,"confidence":if english {json!(0.3)} else {Value::Null},"state":if english {"DERIVED"}else{"UNKNOWN"},"evidence_ids":[post.evidence_id],"limitations":"No sarcasm/negation/model calibration; neutral means no matched tokens, never purchase intent"})
}

/// Wilson score interval (95 %) for k successes in n trials; None when n is 0.
pub fn wilson_interval(k: u64, n: u64) -> Option<(f64, f64)> {
    if n == 0 || k > n {
        return None;
    }
    let (z, n_f) = (1.959_963_984_540_054_f64, n as f64);
    let p = k as f64 / n_f;
    let denom = 1. + z * z / n_f;
    let centre = (p + z * z / (2. * n_f)) / denom;
    let half = z * (p * (1. - p) / n_f + z * z / (4. * n_f * n_f)).sqrt() / denom;
    Some(((centre - half).max(0.), (centre + half).min(1.)))
}

pub const SENTIMENT_LABELS: [&str; 4] = ["POSITIVE", "NEGATIVE", "NEUTRAL", "MIXED"];

/// Label shares over the posts that have a label, each with its count and a 95 % Wilson
/// interval; UNKNOWN posts are counted apart and never enter a denominator. Against a prior
/// summary, a share is only called changed when the two intervals do not overlap.
pub fn sentiment_summary(rows: &[Value], prior: Option<&Value>) -> Value {
    let label = |r: &Value| r["label"].as_str().map(str::to_owned);
    let labelled = rows
        .iter()
        .filter(|r| label(r).is_some_and(|l| SENTIMENT_LABELS.contains(&l.as_str())))
        .count() as u64;
    let mut shares = serde_json::Map::new();
    for l in SENTIMENT_LABELS {
        let k = rows
            .iter()
            .filter(|r| label(r).as_deref() == Some(l))
            .count() as u64;
        let interval = wilson_interval(k, labelled);
        let before = prior.map(|p| &p["shares"][l]);
        let separated = before.and_then(|b| {
            let (lo, hi) = interval?;
            let (blo, bhi) = (b["interval_95"][0].as_f64()?, b["interval_95"][1].as_f64()?);
            Some(if lo > bhi {
                "HIGHER"
            } else if hi < blo {
                "LOWER"
            } else {
                "NOT_SEPARATED"
            })
        });
        shares.insert(
            l.into(),
            json!({"count":k,"share":(labelled>0).then(|| k as f64 / labelled as f64),"interval_95":interval.map(|(a,b)|[a,b]),"versus_prior":separated}),
        );
    }
    json!({"labelled_count":labelled,"unknown_count":rows.len() as u64-labelled,"shares":shares,"interval":"WILSON_SCORE_95","comparison":"NON_OVERLAPPING_INTERVALS_ONLY","method":"ECDEV_SMALL_ENGLISH_TOKEN_RULE_V1","scope":"CAPTURED_SAMPLE_ONLY"})
}
pub fn threshold_events(input: &Value) -> Value {
    let m = &input["metrics"];
    let cur = m["current_count"].as_f64().unwrap_or(0.);
    let base = m["baseline_count"].as_f64().unwrap_or(0.);
    let avg = m["baseline_average"].as_f64().unwrap_or(0.);
    let min = input["minimum_mentions"].as_f64().unwrap_or(0.);
    let mult = input["volume_multiplier"].as_f64().unwrap_or(0.);
    let drop = input["sentiment_drop"].as_f64().unwrap_or(0.);
    let sentiment = m["current_net_sentiment"]
        .as_f64()
        .zip(m["baseline_net_sentiment"].as_f64())
        .is_some_and(|(c, b)| drop > 0. && cur >= min && base >= min && b - c >= drop);
    json!({"volume_spike":mult>0.&&cur>=min&&base>0.&&(avg==0.||cur/avg>=mult),"sentiment_drop":sentiment})
}
pub fn decay(age_seconds: u64, half_life_seconds: u64) -> Option<f64> {
    (half_life_seconds > 0).then(|| 2_f64.powf(-(age_seconds as f64) / half_life_seconds as f64))
}
pub fn derivative(a: f64, b: f64, ta: u64, tb: u64) -> Option<f64> {
    (tb > ta).then(|| (b - a) / ((tb - ta) as f64 / 3600.))
}

/// Stable platform/native IDs collapse repeated captures; text collisions remain conflicts.
pub fn deduplicate(posts: &[SocialPost]) -> (Vec<SocialPost>, Vec<Value>) {
    let mut unique: BTreeMap<String, SocialPost> = BTreeMap::new();
    let mut conflicts = vec![];
    for p in posts {
        if p.validate().is_err() {
            continue;
        }
        if let Some(old) = unique.get(&p.key()) {
            if old.text != p.text {
                conflicts.push(json!({"key":p.key(),"state":"CONFLICT","kind":"POST_TEXT_CHANGED","evidence_ids":[old.evidence_id,p.evidence_id]}));
            }
            if old.captured_at > p.captured_at {
                continue;
            }
        }
        unique.insert(p.key(), p.clone());
    }
    (unique.into_values().collect(), conflicts)
}

/// Deterministic lexical similarity is a topic link, never verified product identity.
pub const PHRASES_PER_CLUSTER: usize = 5;
/// Posts, authors and evidence ids a phrase was seen with.
type PhraseSupport = (BTreeSet<String>, BTreeSet<String>, Vec<String>);
const PHRASE_STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "has", "have", "i",
    "in", "is", "it", "its", "my", "of", "on", "or", "our", "so", "that", "the", "their", "this",
    "to", "was", "we", "were", "with", "you", "your", "just", "very", "really", "about", "what",
    "when", "how", "why", "not", "no", "do", "does", "did", "can", "will", "would", "should",
    "could", "http", "https", "www", "com",
];

/// Recurring 2- and 3-word phrases of a cluster as unverified product-name candidates. A phrase
/// is a contiguous run of words in one post's text; a stop word or punctuation breaks it, so no
/// phrase joins words the text kept apart. It must recur in at least two posts by at least two
/// distinct authors (or sources when the author is unknown). Ranked by posts, then length, then
/// text; a phrase inside a longer kept phrase with the same posts is dropped.
pub fn candidate_phrases(posts: &[&SocialPost]) -> Vec<Value> {
    let mut seen: BTreeMap<String, PhraseSupport> = BTreeMap::new();
    for p in posts {
        let mut in_post = BTreeSet::new();
        for run in p
            .text
            .to_lowercase()
            .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
            .collect::<Vec<_>>()
            .split(|w| w.is_empty() || PHRASE_STOP_WORDS.contains(w) || w.chars().count() < 2)
        {
            for n in 2..=3 {
                for w in run.windows(n) {
                    in_post.insert(w.join(" "));
                }
            }
        }
        let who = p.author_id.clone().unwrap_or_else(|| p.source_url.clone());
        for phrase in in_post {
            let e = seen.entry(phrase).or_default();
            e.0.insert(p.key());
            e.1.insert(who.clone());
            e.2.push(p.evidence_id.clone());
        }
    }
    let mut ranked: Vec<(String, usize, usize, Vec<String>)> = seen
        .into_iter()
        .filter(|(_, (posts, authors, _))| posts.len() >= 2 && authors.len() >= 2)
        .map(|(phrase, (posts, authors, ids))| (phrase, posts.len(), authors.len(), ids))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.0.split(' ').count().cmp(&a.0.split(' ').count()))
            .then(a.0.cmp(&b.0))
    });
    let mut kept: Vec<(String, usize, usize, Vec<String>)> = vec![];
    for r in ranked {
        let covered = kept
            .iter()
            .any(|k| k.1 == r.1 && format!(" {} ", k.0).contains(&format!(" {} ", r.0)));
        if !covered {
            kept.push(r);
        }
        if kept.len() == PHRASES_PER_CLUSTER {
            break;
        }
    }
    kept.into_iter()
        .map(|(phrase, posts, authors, ids)| json!({"phrase":phrase,"posts":posts,"distinct_authors":authors,"evidence_ids":ids,"state":"CANDIDATE_PRODUCT_PHRASE_UNVERIFIED","method":"CONTIGUOUS_2_3_GRAM_STOPWORD_BREAKS_MIN_2_POSTS_2_AUTHORS"}))
        .collect()
}

pub const REPRESENTATIVES_PER_CLUSTER: usize = 3;

fn post_terms(p: &SocialPost) -> BTreeSet<String> {
    let mut t = terms(&p.text);
    t.extend(p.hashtags.iter().map(|s| s.to_lowercase()));
    t
}

/// Explainable cluster summaries: posts ranked by lexical centrality (mean Jaccard similarity to
/// the cluster's other posts, two decimals), then by engagement rank among the cluster's posts of
/// the same platform (likes; counters of different platforms are never compared), unknown
/// engagement last and never zero, then by post key. Each pick carries both components.
pub fn representatives(posts: &[&SocialPost]) -> Vec<Value> {
    let sets: Vec<_> = posts.iter().map(|p| post_terms(p)).collect();
    let mut rows: Vec<(i64, Option<f64>, String, Value)> = posts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let others: Vec<f64> = (0..posts.len())
                .filter(|&j| j != i)
                .map(|j| {
                    let union = sets[i].union(&sets[j]).count();
                    if union == 0 {
                        0.
                    } else {
                        sets[i].intersection(&sets[j]).count() as f64 / union as f64
                    }
                })
                .collect();
            let centrality = if others.is_empty() {
                1.
            } else {
                others.iter().sum::<f64>() / others.len() as f64
            };
            let peers: Vec<u64> = posts
                .iter()
                .filter(|q| q.platform == p.platform)
                .filter_map(|q| q.engagement.likes)
                .collect();
            let rank = p.engagement.likes.map(|l| {
                if peers.len() < 2 {
                    1.
                } else {
                    peers.iter().filter(|&&x| x < l).count() as f64 / (peers.len() - 1) as f64
                }
            });
            let rounded = (centrality * 100.).round() as i64;
            let row = json!({"post_key":p.key(),"evidence_id":p.evidence_id,"platform":p.platform,"centrality":rounded as f64 / 100.,"engagement_rank_within_platform":rank,"engagement_counter":"likes"});
            (rounded, rank, p.key(), row)
        })
        .collect();
    rows.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| match (a.1, b.1) {
                (Some(x), Some(y)) => y.total_cmp(&x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.2.cmp(&b.2))
    });
    rows.into_iter()
        .take(REPRESENTATIVES_PER_CLUSTER)
        .map(|r| r.3)
        .collect()
}

pub fn clusters(posts: &[SocialPost]) -> Vec<Value> {
    let mut groups: Vec<(BTreeSet<String>, Vec<&SocialPost>)> = vec![];
    for p in posts {
        let mut t = terms(&p.text);
        t.extend(p.hashtags.iter().map(|s| s.to_lowercase()));
        for e in &p.entities {
            if let Some(s) = e["value"].as_str() {
                t.insert(s.to_lowercase());
            }
        }
        let index = groups.iter().position(|(g, ps)| {
            let intersection = t.intersection(g).count();
            let union = t.union(g).count();
            let temporal = p
                .published_at
                .zip(ps[0].published_at)
                .is_some_and(|(a, b)| a.abs_diff(b) <= 604800);
            temporal && intersection >= 2 && union > 0 && intersection as f64 / union as f64 >= 0.25
        });
        if let Some(i) = index {
            groups[i].0.extend(t);
            groups[i].1.push(p);
        } else {
            groups.push((t, vec![p]));
        }
    }
    groups.into_iter().enumerate().map(|(i,(t,ps))|json!({"id":format!("cluster-{i}"),"terms":t,"representatives":representatives(&ps),"candidate_phrases":candidate_phrases(&ps),"method":"LEXICAL_JACCARD_0.25_MIN_2_SHARED_TERMS_7_DAY_COOCCURRENCE","state":"DERIVED","identity_state":"DERIVED_WEAK_MATCH","observations":ps.iter().map(|p|p.key()).collect::<Vec<_>>(),"evidence_ids":ps.iter().map(|p|p.evidence_id.clone()).collect::<Vec<_>>(),"platforms":ps.iter().map(|p|p.platform.clone()).collect::<BTreeSet<_>>(),"semantic_embedding_similarity":"UNAVAILABLE","source_edges":ps.iter().map(|p|json!({"from":format!("cluster-{i}"),"relation":"SUPPORTED_BY","to":p.evidence_id})).collect::<Vec<_>>()})).collect()
}

pub fn snapshot(
    posts: &[SocialPost],
    history: &[Value],
    query: &str,
    now: u64,
    window: u64,
    mode: &str,
) -> Value {
    let selected: Vec<_> = posts
        .iter()
        .filter(|p| p.state == EvidenceState::Observed && p.capture_mode == mode)
        .cloned()
        .collect();
    let (dedup, conflicts) = deduplicate(&selected);
    let eligible: Vec<_> = dedup
        .iter()
        .filter(|p| {
            p.published_at
                .is_some_and(|t| t > now.saturating_sub(window) && t <= now)
                && mentions(query, &p.text)
        })
        .cloned()
        .collect();
    let platforms: BTreeSet<_> = eligible.iter().map(|p| p.platform.clone()).collect();
    let sources: BTreeSet<_> = eligible.iter().map(|p| p.source_url.clone()).collect();
    let publishers: BTreeSet<_> = eligible
        .iter()
        .filter_map(|p| p.publisher.clone())
        .collect();
    let originals = eligible
        .iter()
        .filter(|p| p.propagation == "ORIGINAL")
        .count();
    let ids: Vec<_> = eligible.iter().map(|p| p.evidence_id.clone()).collect();
    let n = eligible.len();
    let last = history.last();
    let velocity = last.and_then(|v| {
        derivative(
            v["mention_count"].as_f64()?,
            n as f64,
            v["captured_at"].as_u64()?,
            now,
        )
    });
    let acceleration = last.and_then(|v| {
        derivative(
            v["velocity"]["value"].as_f64()?,
            velocity?,
            v["captured_at"].as_u64()?,
            now,
        )
    });
    let persistence = if history.is_empty() {
        None
    } else {
        Some(
            history
                .iter()
                .filter(|s| s["mention_count"].as_u64().is_some_and(|v| v > 0))
                .count() as f64
                / history.len() as f64,
        )
    };
    let previous_ids: BTreeSet<_> = last
        .and_then(|v| v["post_keys"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let novelty = last.map(|_| {
        eligible
            .iter()
            .filter(|p| !previous_ids.contains(p.key().as_str()))
            .count()
    });
    let metric = |value: Option<f64>, normalization: &str, denom: Value| json!({"value":value,"state":if value.is_some(){"DERIVED"}else{"UNKNOWN"},"window_seconds":window,"denominator":denom,"source_count":sources.len(),"evidence_ids":ids,"normalization":normalization,"confidence":"UNCALIBRATED_SAMPLED_EVIDENCE"});
    let latest = eligible.iter().filter_map(|p| p.published_at).max();
    let mut components = vec![
        json!({"name":"velocity","weight":0.3,"metric":metric(velocity.map(|v|v.max(0.)/(1.+v.max(0.))),"POSITIVE_CAPTURED_WINDOW_COUNT_DELTA_SATURATION",json!(last.map(|s|now.saturating_sub(s["captured_at"].as_u64().unwrap_or(now)))))}),
        json!({"name":"acceleration","weight":0.15,"metric":metric(acceleration.map(|v|v.max(0.)/(1.+v.max(0.))),"POSITIVE_VELOCITY_DELTA_SATURATION",json!("hours between snapshots"))}),
        json!({"name":"persistence","weight":0.15,"metric":metric(persistence,"NONEMPTY_PRIOR_SNAPSHOTS_RATIO",json!(history.len()))}),
        json!({"name":"source_diversity","weight":0.1,"metric":metric(Some(sources.len() as f64/(sources.len()+1) as f64),"DISTINCT_URLS_NOT_INDEPENDENT_PUBLISHERS",json!(sources.len()+1))}),
        json!({"name":"platform_diversity","weight":0.1,"metric":metric(Some(platforms.len() as f64/(platforms.len()+1) as f64),"DISTINCT_PLATFORM_LABELS",json!(platforms.len()+1))}),
        json!({"name":"novelty","weight":0.1,"metric":metric(novelty.map(|v|v as f64/n.max(1) as f64),"NEW_POST_IDS_OVER_WINDOW_POSTS",json!(n))}),
        json!({"name":"cross_platform_recurrence","weight":0.1,"metric":metric(Some(if platforms.len()>1{1.}else{0.}),"SAME_QUERY_PRESENT_ON_MULTIPLE_PLATFORMS_NOT_ORIGINAL_SOURCE_INDEPENDENCE",json!(platforms.len()))}),
    ];
    components.push(json!({"name":"engagement_growth","weight":0.,"metric":metric(None,"NEEDS_COMPATIBLE_POST_LEVEL_COUNTER_HISTORY",Value::Null)}));
    let coverage = components
        .iter()
        .filter(|c| c["metric"]["value"].is_number())
        .map(|c| c["weight"].as_f64().unwrap())
        .sum::<f64>();
    let score = components
        .iter()
        .filter_map(|c| Some(c["weight"].as_f64()? * c["metric"]["value"].as_f64()?))
        .sum::<f64>();
    let state = if n < 2 {
        "INSUFFICIENT_EVIDENCE"
    } else if acceleration.is_some_and(|v| v > 0.) {
        "ACCELERATING"
    } else if velocity.is_some_and(|v| v < 0.) {
        "COOLING"
    } else if persistence.is_some_and(|v| v >= 0.75) {
        "PERSISTENT"
    } else if velocity.is_some_and(|v| v > 0.) {
        "EMERGING"
    } else {
        "DISCOVERED"
    };
    let mut result = json!({"query":query,"capture_mode":mode,"captured_at":now,"window_seconds":window,"window_start_exclusive":now.saturating_sub(window),"window_end_inclusive":now,"mention_count":n,"observation_count":n,"unique_sources":sources.len(),"platform_count":platforms.len(),"platforms":platforms,"publisher_count":publishers.len(),"publishers":publishers,"independent_original_publishers":{"state":"UNKNOWN","value":null},"original_post_count":originals,"post_keys":eligible.iter().map(SocialPost::key).collect::<Vec<_>>(),"evidence_ids":ids,"start_time":eligible.iter().filter_map(|p|p.published_at).min(),"last_observed_time":latest,"velocity":metric(velocity,"DELTA_CAPTURED_WINDOW_MENTIONS_PER_HOUR",json!(last.map(|s|now.saturating_sub(s["captured_at"].as_u64().unwrap_or(now))))),"acceleration":metric(acceleration,"DELTA_VELOCITY_PER_HOUR",json!("actual snapshot hours")),"persistence":metric(persistence,"NONEMPTY_PRIOR_SNAPSHOTS_RATIO",json!(history.len())),"novelty":metric(novelty.map(|v|v as f64),"NEW_POST_IDENTITIES",json!(n)),"time_decay":metric(latest.and_then(|t|decay(now-t,86400)),"HEURISTIC_ONE_DAY_HALF_LIFE",json!(86400)),"score":{"value":score,"state":"DERIVED","components":components,"known_weight_coverage":coverage,"policy":"EXPLICIT_HEURISTIC_V1_UNKNOWN_COMPONENTS_UNSCORED_NOT_ZERO","learned":false},"state":state,"clusters":clusters(&eligible),"engagement_observations":eligible.iter().map(|p|json!({"post_key":p.key(),"platform":p.platform,"metrics":p.engagement,"evidence_id":p.evidence_id,"missing":"UNKNOWN","comparability":"PLATFORM_SPECIFIC_COUNTERS_NOT_SUMMED"})).collect::<Vec<_>>(),"sentiment":eligible.iter().map(sentiment).collect::<Vec<_>>(),"entity_links":eligible.iter().flat_map(|p|p.entities.iter()).collect::<Vec<_>>(),"commerce_links":[],"conflicts":conflicts,"unknowns":["Population coverage","Independent publisher verification","Sales/search demand/conversion/revenue","Semantic embedding similarity","Engagement growth without compatible counter history"],"timestamp_unknown_excluded":dedup.iter().filter(|p|p.published_at.is_none()).count(),"sampling":"Bounded retrieved sample, not total platform mentions; velocity includes capture coverage changes","simulation_contribution":0});
    let growth = last.and_then(|old| runtime::compatible_engagement_delta(old, &result));
    result["engagement_growth"] = metric(
        growth,
        "MAX_COMPATIBLE_SAME_POST_SAME_PLATFORM_COUNTER_DELTA_NOT_AGGREGATE",
        json!("matching observed counters"),
    );
    let prior_end = now.saturating_sub(window);
    let prior_start = prior_end.saturating_sub(window);
    let captured_windows: Vec<(u64, u64)> = history
        .iter()
        .filter_map(|s| {
            let c = s["captured_at"].as_u64()?;
            Some((c.saturating_sub(s["window_seconds"].as_u64()?), c))
        })
        .collect();
    let covered = growth::covered_seconds(prior_start, prior_end, &captured_windows);
    let prior = dedup
        .iter()
        .filter(|p| {
            p.published_at
                .is_some_and(|t| t > prior_start && t <= prior_end)
                && mentions(query, &p.text)
        })
        .count() as u64;
    let comparison = (covered == window && window > 0)
        .then(|| growth::rate_comparison(prior, window, n as u64, window))
        .flatten();
    result["mention_growth"] = json!({
        "prior_window_start_exclusive": prior_start,
        "prior_window_end_inclusive": prior_end,
        "prior_window_covered_seconds": covered,
        "state": comparison.as_ref().map_or("PRIOR_WINDOW_NOT_FULLY_CAPTURED", |c| c.state),
        "comparison": comparison,
        "counts": "RETRIEVED_SAMPLE_NOT_PLATFORM_TOTAL",
        "score_weight": 0,
    });
    let summary = sentiment_summary(
        result["sentiment"]
            .as_array()
            .map_or(&[][..], Vec::as_slice),
        last.map(|old| &old["sentiment_summary"])
            .filter(|v| v.is_object()),
    );
    result["sentiment_summary"] = summary;
    result["cluster_growth"] = metric(
        last.map(|old| {
            result["clusters"].as_array().map_or(0, Vec::len) as f64
                - old["clusters"].as_array().map_or(0, Vec::len) as f64
        }),
        "DELTA_LEXICAL_CLUSTER_COUNT",
        json!("prior snapshot"),
    );
    let mut recurrence: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in &eligible {
        for e in &p.entities {
            if let Some(v) = e["value"].as_str() {
                recurrence
                    .entry(v.to_owned())
                    .or_default()
                    .insert(p.platform.clone());
            }
        }
    }
    result["entity_recurrence"]=json!(recurrence.into_iter().map(|(value,platforms)|json!({"value":value,"platform_count":platforms.len(),"platforms":platforms,"state":"DERIVED","identity_state":"SOURCE_ASSERTED","evidence_ids":ids})).collect::<Vec<_>>());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_queries_match_unspaced_text() {
        let t = terms("抹茶ラテが人気、matcha latte");
        for want in ["抹茶", "茶ラ", "ラテ", "人気", "matcha", "latte"] {
            assert!(t.contains(want), "{want} in {t:?}");
        }
        // A two-character query is valid and matches inside a sentence.
        assert_eq!(terms("抹茶"), BTreeSet::from(["抹茶".to_string()]));
        assert!(mentions("抹茶", "新作の抹茶ラテが人気"));
        assert!(mentions("抹茶 ラテ", "新作の抹茶ラテが人気"));
        // Scattered bigrams are not a mention of the run.
        assert!(!mentions("抹茶ラテ", "抹茶と茶ラッテとラテ"));
        assert!(!mentions("ほうじ茶", "新作の抹茶ラテ"));
        // Full-width Latin folds; a lone kanji is its own term; Latin keeps its rules.
        assert!(mentions("matcha", "ＭＡＴＣＨＡ　ラテ"));
        assert!(terms("茶").contains("茶"));
        assert!(mentions("茶", "新作の抹茶ラテ") && !mentions("茶", "コーヒー"));
        assert!(terms("a an the matcha").iter().eq(["matcha"].iter()));
        // Script boundaries split runs: "iPhone17ケース" is iphone17 + ケース bigrams.
        let mixed = terms("iPhone17ケース");
        assert!(mixed.contains("iphone17") && mixed.contains("ケー") && mixed.contains("ース"));
        // Korean and Chinese are unspaced runs too.
        assert!(mentions("말차", "말차라떼 인기") && mentions("抹茶", "抹茶拿铁很受欢迎"));
        // Two-letter Latin query words match whole words only; function words name nothing.
        assert!(searchable("AI") && mentions("AI", "New AI kettle") && !mentions("AI", "Thai tea"));
        assert!(mentions("5G router", "A 5G Router launch") && !mentions("5G router", "router"));
        assert!(mentions("AI 家電", "AI搭載の家電") && !mentions("AI 家電", "家電の新作"));
        assert!(!searchable("of an") && !searchable("a"));
    }
    #[test]
    fn independent_social_donor_oracles() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../research/commerce/social-oracle-fixtures.json"
        ))
        .unwrap();
        assert_eq!(fixture["families"].as_array().unwrap().len(), 2);
        assert_eq!(fixture["cases"].as_array().unwrap().len(), 560);
        // The frozen fixture keeps TrendRadar's 240 rank-exposure cases; that GPL-3.0 formula was
        // withdrawn from ECDEV (research/commerce/trendradar-capability-review.json).
        let mut executed = 0;
        for c in fixture["cases"].as_array().unwrap() {
            if c["family"] == "thresholds" {
                assert_eq!(threshold_events(&c["input"]), c["expected"]);
                executed += 1;
            }
        }
        assert_eq!(executed, 320);
    }
    #[test]
    fn actual_timestamps_and_decay() {
        assert_eq!(derivative(2., 6., 100, 7300), Some(2.));
        assert_eq!(derivative(2., 6., 100, 100), None);
        assert_eq!(derivative(2., 6., 100, 99), None);
        assert_eq!(derivative(1., 3., 100, 3700), Some(2.));
        assert_eq!(decay(86400, 86400), Some(0.5));
        assert_eq!(decay(1, 0), None);
    }
}
