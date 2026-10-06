//! `ecdev-gov donor summary`: what ECDEV knows of the donor universe and how far along the
//! lifecycle it is. A legacy registry that is not reconciled is reported as such, never as 0.

use super::SourceState;
use crate::formats::json::Json;
use crate::metrics::Counts;
use crate::schema::UniverseKind;

/// One repository's row.
pub struct Row {
    pub repository: String,
    pub counts: Counts,
    pub sources: Vec<(String, SourceState)>,
}

impl Row {
    pub fn of(a: &crate::Assessment) -> Row {
        Row {
            repository: a.declaration.repository.id.clone(),
            counts: a.counts.clone(),
            sources: a.universe.sources.clone(),
        }
    }
    pub fn unreconciled(&self) -> Vec<&str> {
        self.sources
            .iter()
            .filter(|(_, s)| *s != SourceState::Reconciled)
            .map(|(p, _)| p.as_str())
            .collect()
    }
    /// The known-universe cell: a count, or "not reconciled" where a legacy registry is unread.
    pub fn known(&self) -> String {
        known_cell(
            self.counts.known_universe_total,
            self.unreconciled().len() as u64,
        )
    }
}

fn known_cell(n: u64, unreconciled: u64) -> String {
    match (n, unreconciled) {
        (n, 0) => n.to_string(),
        (0, _) => "not reconciled".into(),
        (n, _) => format!("{n} + not reconciled"),
    }
}

/// Known relations by primary kind.
pub fn by_kind(c: &Counts) -> Vec<(UniverseKind, u64)> {
    use UniverseKind as K;
    vec![
        (K::OssRepository, c.known_oss_repositories),
        (K::InformationSource, c.known_information_sources),
        (K::OfficialAuthority, c.known_official_authorities),
        (K::ProtocolSpec, c.known_protocol_specs),
        (K::AcademicReference, c.known_academic_references),
        (K::Benchmark, c.known_benchmarks),
        (K::Dataset, c.known_datasets),
        (K::Simulator, c.known_simulators),
        (K::Standard, c.known_standards),
        (K::ReferenceArchitecture, c.known_reference_architectures),
        (K::Paper, c.known_papers),
    ]
}

const COLUMNS: &[&str] = &[
    "Repository",
    "Known universe",
    "Registered",
    "Selected",
    "Censused",
    "Technology mapped",
    "Native shadow",
    "Parity proven",
    "Cutover",
    "Extinct",
];

fn cells(name: &str, known: String, c: &Counts) -> Vec<String> {
    vec![
        name.to_string(),
        known,
        c.registered_universe_total.to_string(),
        c.selected_for_census.to_string(),
        c.censused.to_string(),
        c.technology_mapped.to_string(),
        c.native_shadow.to_string(),
        c.parity_proven.to_string(),
        c.cutover.to_string(),
        c.extinct.to_string(),
    ]
}

fn table(rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = COLUMNS.iter().map(|c| c.len()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let line = |r: &[String]| -> String {
        let mut s = String::from(" ");
        for (i, c) in r.iter().enumerate() {
            let pad = widths[i] - c.chars().count();
            if i == 0 {
                s.push_str(&format!(" {c}{}", " ".repeat(pad)));
            } else {
                s.push_str(&format!("  {}{c}", " ".repeat(pad)));
            }
        }
        s.push('\n');
        s
    };
    let mut s = line(&COLUMNS.iter().map(|c| c.to_string()).collect::<Vec<_>>());
    for r in rows {
        s.push_str(&line(r));
    }
    s
}

fn identity_lines(c: &Counts) -> String {
    format!(
        "  identity: unknown identity {}; ambiguous look-alike groups {}\n  verification: unverified origin {}; unverified licence {}; unresolved relevance {}\n  lifecycle beyond: native targeted {}\n",
        c.unknown_identity,
        c.ambiguous_donor_identities,
        c.unverified_origin,
        c.unverified_license,
        c.unresolved_relevance,
        c.native_targeted
    )
}

fn kinds_line(c: &Counts) -> String {
    let v: Vec<String> = by_kind(c)
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| format!("{} {n}", k.wire()))
        .collect();
    if v.is_empty() {
        "-".into()
    } else {
        v.join(", ")
    }
}

/// The text summary.
pub fn render(rows: &[Row]) -> String {
    let mut s = String::new();
    for r in rows {
        let c = &r.counts;
        s.push_str(&format!("DONOR UNIVERSE of {}\n", r.repository));
        s.push_str(&table(&[cells(&r.repository, r.known(), c)]));
        s.push_str(&format!(
            "  known universe by kind: {}\n  global donors: {} unique; {} repository->donor references\n",
            kinds_line(c),
            c.unique_global_donors,
            c.repository_donor_references
        ));
        s.push_str(&identity_lines(c));
        s.push_str("  legacy registries:\n");
        if r.sources.is_empty() {
            s.push_str("    (none)\n");
        }
        for (p, st) in &r.sources {
            s.push_str(&format!("    {:<15} {p}\n", st.wire()));
        }
    }
    s
}

fn row_json(r: &Row) -> Json {
    let c = &r.counts;
    let mut kinds = Json::obj();
    for (k, n) in by_kind(c) {
        kinds = kinds.with(k.wire(), n);
    }
    Json::obj()
        .with("repository", &r.repository)
        .with("known_universe", r.known())
        .with("known_universe_total", c.known_universe_total)
        .with("by_kind", kinds)
        .with("registered", c.registered_universe_total)
        .with("selected_for_census", c.selected_for_census)
        .with("censused", c.censused)
        .with("technology_mapped", c.technology_mapped)
        .with("native_targeted", c.native_targeted)
        .with("native_shadow", c.native_shadow)
        .with("parity_proven", c.parity_proven)
        .with("cutover", c.cutover)
        .with("extinct", c.extinct)
        .with("unique_global_donors", c.unique_global_donors)
        .with("repository_donor_references", c.repository_donor_references)
        .with("unknown_identity", c.unknown_identity)
        .with("unverified_origin", c.unverified_origin)
        .with("unverified_license", c.unverified_license)
        .with("unresolved_relevance", c.unresolved_relevance)
        .with("ambiguous_donor_identities", c.ambiguous_donor_identities)
        .with(
            "legacy_registries",
            Json::Array(
                r.sources
                    .iter()
                    .map(|(p, st)| Json::obj().with("path", p).with("state", st.wire()))
                    .collect(),
            ),
        )
}

pub fn to_json(rows: &[Row]) -> Json {
    Json::obj()
        .with("schema", crate::protocol::schema_identity())
        .with(
            "repositories",
            Json::Array(rows.iter().map(row_json).collect()),
        )
}
