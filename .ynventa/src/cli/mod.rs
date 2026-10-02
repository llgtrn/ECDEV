//! `ynventa <command>` — identical commands with identical semantics in every repository:
//!
//! ```text
//! cargo run --manifest-path .ynventa/Cargo.toml -- <command> [--root <repo>] [options]
//! ```

use crate::compact::facts::{Fact, Knowledge};
use crate::compact::history::{Batch, History, Row};
use crate::formats::json::Json;
use crate::protocol::{own_subsystem_dir, Snapshot, COMMANDS, SNAPSHOT_FILE};
use crate::schema::{DonorState, Ecosystem, FactKind};
use crate::{
    assess, audit, compact, conformance, declare, evidence, migration, Assessment, Severity,
};
use std::path::{Path, PathBuf};

struct Args {
    root: PathBuf,
    json: bool,
    flags: Vec<String>,
    values: Vec<(String, String)>,
    positional: Vec<String>,
    multi: Vec<(String, Vec<String>)>,
}

impl Args {
    fn parse(raw: &[String]) -> Result<Args, String> {
        let mut a = Args {
            root: crate::default_root(),
            json: false,
            flags: vec![],
            values: vec![],
            positional: vec![],
            multi: vec![],
        };
        let valued = [
            "--root",
            "--shard",
            "--origin",
            "--binary",
            "--reference",
            "--out",
            "--system",
            "--from",
            "--node",
            "--into",
            "--cargo-home",
            "--provenance",
            "--kind",
            "--subject",
            "--document",
            "--against",
            "--table",
        ];
        let lists = ["--merge", "--aggregate", "--ecosystem"];
        let mut i = 0;
        while i < raw.len() {
            let x = &raw[i];
            if valued.contains(&x.as_str()) {
                let v = raw.get(i + 1).ok_or(format!("{x} needs a value"))?.clone();
                if x == "--root" {
                    a.root = PathBuf::from(&v);
                } else {
                    a.values.push((x.clone(), v));
                }
                i += 2;
            } else if lists.contains(&x.as_str()) {
                let mut vs = Vec::new();
                i += 1;
                while i < raw.len() && !raw[i].starts_with("--") {
                    vs.push(raw[i].clone());
                    i += 1;
                }
                a.multi.push((x.clone(), vs));
            } else if x == "--json" {
                a.json = true;
                i += 1;
            } else if x.starts_with("--") {
                a.flags.push(x.clone());
                i += 1;
            } else {
                a.positional.push(x.clone());
                i += 1;
            }
        }
        Ok(a)
    }
    fn flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }
    fn value(&self, k: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }
    /// Every value of a repeatable valued option, in order.
    fn values_of(&self, k: &str) -> Vec<String> {
        self.values
            .iter()
            .filter(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .collect()
    }
    fn list(&self, k: &str) -> Vec<PathBuf> {
        self.multi
            .iter()
            .filter(|(n, _)| n == k)
            .flat_map(|(_, v)| v.iter().map(PathBuf::from))
            .collect()
    }
}

fn usage() -> String {
    let mut s = String::from("ynventa (protocol v1)\nusage: ynventa <command> [--root <repo>] [--json] [options]\n\ncommands:\n");
    for (c, m) in COMMANDS {
        s.push_str(&format!("  {c:<12} {m}\n"));
    }
    s
}

/// Runs a command; returns (exit status, output).
pub fn run(raw: &[String]) -> (i32, String) {
    let Some(cmd) = raw.first() else {
        return (2, usage());
    };
    let args = match Args::parse(&raw[1..]) {
        Ok(a) => a,
        Err(e) => return (2, format!("{e}\n{}", usage())),
    };
    let result = match cmd.as_str() {
        "status" => with(&args, status),
        "verify" => with(&args, verify),
        "audit" => with(&args, audit_cmd),
        "census" => with(&args, census),
        "extinction" => with(&args, extinction),
        "graph" => with(&args, graph),
        "metrics" => with(&args, metrics),
        "compact" => compact_cmd(&args),
        "conformance" => with(&args, conformance_cmd),
        "migrate" => migrate(&args),
        "protocol" => protocol(&args),
        "prove" => prove(&args),
        "capsule" => capsule(&args),
        "link" => link_cmd(&args),
        "show" => show(&args, false),
        "backlinks" => show(&args, true),
        "context" => context(&args),
        "technology" => technology(&args),
        "organism" => organism_cmd(&args),
        "fact" => fact_cmd(&args),
        "knowledge" => knowledge_cmd(&args),
        "donor" => donor_cmd(&args),
        "help" | "--help" | "-h" => Ok((0, usage())),
        other => Err(format!("unknown command `{other}`\n{}", usage())),
    };
    match result {
        Ok(r) => r,
        Err(e) => (2, format!("error: {e}\n")),
    }
}

type Out = Result<(i32, String), String>;

fn with(args: &Args, f: fn(&Args, &Assessment) -> Out) -> Out {
    let a = assess(&args.root)?;
    f(args, &a)
}

fn findings_text(a: &Assessment, limit: usize) -> String {
    let mut s = String::new();
    let (e, w, i) = a
        .findings
        .iter()
        .fold((0, 0, 0), |(e, w, i), f| match f.severity {
            Severity::Error => (e + 1, w, i),
            Severity::Warning => (e, w + 1, i),
            Severity::Info => (e, w, i + 1),
        });
    s.push_str(&format!("findings: {e} errors, {w} warnings, {i} info\n"));
    for f in a.findings.iter().take(limit) {
        s.push_str(&format!(
            "  {:<7} {:<26} {}: {}\n",
            f.severity.wire(),
            f.code,
            f.subject,
            f.detail
        ));
    }
    if a.findings.len() > limit {
        s.push_str(&format!(
            "  … {} more (ynventa audit)\n",
            a.findings.len() - limit
        ));
    }
    s
}

fn metrics_text(a: &Assessment) -> String {
    let mut s = String::new();
    for (k, v) in a.counts.values() {
        s.push_str(&format!("  {k:<30} {v}\n"));
    }
    s
}

fn gate_text(a: &Assessment) -> (bool, String) {
    let mut s = String::from("V1 gate:\n");
    let mut all = true;
    for (k, want, got, pass) in a.counts.v1_gate() {
        all &= pass;
        s.push_str(&format!(
            "  [{}] {k} = {got} (required {want})\n",
            if pass { "x" } else { " " }
        ));
    }
    (all, s)
}

/// Subjects per kind shown in the knowledge section of `status`.
const STATUS_KNOWLEDGE_LIMIT: usize = 12;

fn status(args: &Args, a: &Assessment) -> Out {
    if args.json {
        return Ok((
            0,
            conformance::render_json(a, &conformance::Suite { checks: vec![] }).render(),
        ));
    }
    let d = &a.declaration;
    let (pass, gate) = gate_text(a);
    Ok((
        0,
        format!(
            "{} ({}) system={} shard={} protocol=v{}\nnodes={} donors={} waves={} shims={}\n\nmetrics:\n{}\n{}V1: {}\n\n{}\n{}",
            d.repository.name,
            d.repository.origin,
            d.repository.system,
            d.repository.shard,
            crate::YNVENTA_PROTOCOL_VERSION,
            d.repository.nodes.len(),
            d.donors.len(),
            d.migration.waves.len(),
            d.migration.shims.len(),
            metrics_text(a),
            gate,
            if pass { "PASS" } else { "NOT YET" },
            compact::view::summary(&a.knowledge, STATUS_KNOWLEDGE_LIMIT),
            findings_text(a, 25)
        ),
    ))
}

fn verify(_: &Args, a: &Assessment) -> Out {
    let errors = a.errors().count();
    let checks = conformance::protocol_checks(a, None);
    let failed: Vec<&conformance::Check> = checks.iter().filter(|c| !c.pass).collect();
    let mut s = findings_text(a, usize::MAX);
    s.push_str(&format!(
        "protocol checks: {}/{} pass\n",
        checks.len() - failed.len(),
        checks.len()
    ));
    for c in &failed {
        s.push_str(&format!("  FAIL {}: {}\n", c.id, c.detail));
    }
    let ok = errors == 0 && failed.is_empty();
    s.push_str(if ok {
        "verify: PASS\n"
    } else {
        "verify: FAIL\n"
    });
    Ok((if ok { 0 } else { 1 }, s))
}

fn audit_cmd(args: &Args, a: &Assessment) -> Out {
    if args.json {
        let docs: Vec<Json> = a
            .docs
            .docs
            .iter()
            .map(|d| {
                Json::obj()
                    .with("path", &d.path)
                    .with("allowed", d.allowed)
                    .with("extracted", d.extracted)
                    .with(
                        "issues",
                        d.issues.iter().map(|i| i.wire()).collect::<Vec<_>>(),
                    )
            })
            .collect();
        return Ok((
            0,
            Json::obj()
                .with(
                    "findings",
                    Json::Array(a.findings.iter().map(|f| f.to_json()).collect()),
                )
                .with("documents", Json::Array(docs))
                .render(),
        ));
    }
    let mut s = findings_text(a, usize::MAX);
    s.push_str(&format!(
        "\ndocument budget: {} documents, {} over budget, {} extracted\n",
        a.docs.docs.len(),
        a.docs.over_budget().count(),
        a.docs.docs.iter().filter(|d| d.extracted).count()
    ));
    Ok((0, s))
}

fn census(args: &Args, a: &Assessment) -> Out {
    let c = &a.census;
    let mut s = format!(
        "members: {}\ninternal edges: {}\nforeign references: {}\nlocked external closure: {}\n",
        c.members.len(),
        c.internal.len(),
        c.observations.len(),
        c.closure.len()
    );
    for o in &c.observations {
        s.push_str(&format!(
            "  {:<16} {:<8} {:<7} {} ({})\n",
            o.via.wire(),
            o.ecosystem,
            o.scope,
            o.name,
            o.file
        ));
    }
    if args.flag("--record") {
        let mut registered = a.history.registered.clone();
        let mut active = a.history.active.clone();
        let mut states = Vec::new();
        for d in &a.analysis.donors {
            if d.effective >= DonorState::Registered {
                registered.insert(d.key.clone());
            }
            if d.facts.active() {
                active.insert(d.key.clone());
            }
            states.push((d.key.clone(), d.effective));
        }
        let row = Row {
            seq: a.history.next_seq(),
            commit: crate::repository::files::head_commit(&a.root).unwrap_or_default(),
            states,
            metrics: a.counts.values(),
        };
        let path = History::record(
            &a.root,
            &Batch {
                registered,
                active,
                rows: vec![row],
            },
        )
        .map_err(|e| e.to_string())?;
        s.push_str(&format!("recorded {path}\n"));
    }
    Ok((0, s))
}

fn extinction(args: &Args, a: &Assessment) -> Out {
    let only = args.positional.first();
    if args.json {
        let donors: Vec<Json> = a
            .analysis
            .donors
            .iter()
            .filter(|d| only.is_none_or(|k| *k == d.key))
            .map(|d| {
                Json::obj()
                    .with("donor", &d.key)
                    .with("claimed", d.claimed.wire())
                    .with("effective", d.effective.wire())
                    .with(
                        "exception",
                        d.exception
                            .as_ref()
                            .map(|(k, r)| format!("{k}: {r}"))
                            .unwrap_or_default(),
                    )
                    .with("stopped_by", &d.stopped_by)
                    .with(
                        "gates",
                        Json::Array(
                            d.gates
                                .iter()
                                .map(|g| {
                                    Json::obj()
                                        .with("gate", g.gate.wire())
                                        .with("pass", g.pass)
                                        .with("detail", &g.detail)
                                })
                                .collect(),
                        ),
                    )
            })
            .collect();
        return Ok((
            0,
            Json::obj()
                .with("schema", crate::protocol::schema_identity())
                .with("donors", Json::Array(donors))
                .render(),
        ));
    }
    let mut s = String::new();
    for d in a
        .analysis
        .donors
        .iter()
        .filter(|d| only.is_none_or(|k| *k == d.key))
    {
        s.push_str(&format!(
            "{}  claimed {}  effective {}{}\n",
            d.key,
            d.claimed,
            d.effective,
            d.exception
                .as_ref()
                .map(|(k, r)| format!("  [{k}: {r}]"))
                .unwrap_or_default()
        ));
        if !d.stopped_by.is_empty() {
            s.push_str(&format!("  next: {}\n", d.stopped_by));
        }
        if only.is_some() {
            for g in &d.gates {
                s.push_str(&format!(
                    "  [{}] {:<28} {}\n",
                    if g.pass { "x" } else { " " },
                    g.gate.wire(),
                    g.detail
                ));
            }
            for c in &d.capabilities {
                s.push_str(&format!("  capability {} required={} specified={} native={} ({}) parity={:?} regression={:?}\n", c.key, c.required, c.specified, c.native, c.native_detail, c.parity.iter().map(|(_, v)| v.wire()).collect::<Vec<_>>(), c.regression.iter().map(|(_, v)| v.wire()).collect::<Vec<_>>()));
            }
        }
    }
    if s.is_empty() {
        s.push_str("no donors\n");
    }
    Ok((0, s))
}

fn graph(args: &Args, a: &Assessment) -> Out {
    let mut g = a.graph.clone();
    let mut issues = Vec::new();
    for other in args.list("--merge") {
        let b = assess(&other)?;
        issues.extend(g.merge(&b.graph));
    }
    if let Some(out) = args.value("--binary") {
        std::fs::write(out, g.encode()).map_err(|e| e.to_string())?;
    }
    let mut s = if args.json {
        g.to_json().render()
    } else {
        g.render_text()
    };
    for i in issues {
        s.push_str(&format!(
            "merge issue {} {}: {}\n",
            i.code, i.subject, i.detail
        ));
    }
    Ok((0, s))
}

fn metrics(args: &Args, a: &Assessment) -> Out {
    let mut total = a.counts.clone();
    let others = args.list("--aggregate");
    let mut names = vec![a.declaration.repository.shard.clone()];
    if !others.is_empty() {
        // The ecosystem is the linked system: counts are summed, and what only the linked graph
        // decides (the organism, orphans, sharing, adoption) is judged on it.
        let mut capsules = vec![crate::capsule::Capsule::compile(a)];
        for o in &others {
            let b = assess(o)?;
            names.push(b.declaration.repository.shard.clone());
            capsules.push(crate::capsule::Capsule::compile(&b));
        }
        total = crate::linker::link(&capsules).counts();
    }
    if args.json {
        return Ok((0, total.to_json().with("repositories", names).render()));
    }
    let mut s = format!("repositories: {}\n", names.join(" "));
    for (k, v) in total.values() {
        s.push_str(&format!("{k} {v}\n"));
    }
    Ok((0, s))
}

fn conformance_cmd(args: &Args, a: &Assessment) -> Out {
    let reference = match args.value("--reference") {
        Some(p) => {
            Some(conformance::load_reference(Path::new(p)).ok_or(format!("no snapshot at {p}"))?)
        }
        None => None,
    };
    let suite = conformance::run(a, reference.as_ref());
    let pass = suite.checks.iter().all(|c| c.pass);
    if args.json {
        return Ok((
            if pass { 0 } else { 1 },
            conformance::render_json(a, &suite).render(),
        ));
    }
    let mut s = String::new();
    for c in &suite.checks {
        s.push_str(&format!(
            "[{}] {:<36} {}\n",
            if c.pass { "x" } else { " " },
            c.id,
            c.detail
        ));
    }
    let (gate_pass, gate) = gate_text(a);
    s.push_str(&format!("\n{gate}"));
    s.push_str(&format!(
        "\nconformance: {}/{} checks pass; V1 gate {}\n",
        suite.checks.iter().filter(|c| c.pass).count(),
        suite.checks.len(),
        if gate_pass { "PASS" } else { "NOT YET" }
    ));
    Ok((if pass { 0 } else { 1 }, s))
}

fn compact_cmd(args: &Args) -> Out {
    let root = &args.root;
    let mut s = String::new();
    let a = assess(root)?;
    if args.flag("--extract-docs") {
        let mut seq = a.knowledge.next_seq();
        let mut facts = Vec::new();
        let mut n = 0;
        let commit = crate::repository::files::head_commit(root);
        let mut originals = Vec::new();
        for d in a.docs.over_budget().filter(|d| !d.extracted) {
            if let Some(text) = a.files.read(&d.path) {
                facts.extend(audit::extract_at(&d.path, &text, seq, commit.as_deref()));
                originals.push((d.path.clone(), text.into_bytes()));
                seq += 1;
                n += 1;
            }
        }
        prove_round_trip(&facts, &originals)?;
        if !facts.is_empty() {
            let p = Knowledge::add(root, &facts).map_err(|e| e.to_string())?;
            s.push_str(&format!("extracted {n} documents into {p}\n"));
        }
    }
    if args.flag("--extract-legacy") {
        let mut seq = a.knowledge.next_seq();
        let mut facts = Vec::new();
        let (mut n, mut binary) = (0, 0);
        let docs: std::collections::BTreeSet<&str> =
            a.docs.docs.iter().map(|d| d.path.as_str()).collect();
        let sources = crate::census::excluded_roots(&a.declaration);
        let mut originals = Vec::new();
        for sh in a
            .declaration
            .migration
            .shims
            .iter()
            .filter(|s| s.kind == crate::schema::ShimKind::LegacyInput)
        {
            for f in a.files.under(&sh.path) {
                let in_source = sources
                    .iter()
                    .any(|p| p != &sh.path && (f.starts_with(&format!("{p}/")) || f == p));
                if docs.contains(f.as_str()) || in_source {
                    continue;
                }
                let Some(bytes) = a.files.read_bytes(f) else {
                    continue;
                };
                // Already imported when knowledge rebuilds exactly these bytes; a file imported
                // line by line before content was stored is imported again, losslessly.
                if a.knowledge.stores(f, &bytes) {
                    continue;
                }
                match migration::atlas::extract_file(f, &bytes, seq) {
                    Some(x) => {
                        facts.extend(x);
                        originals.push((f.clone(), bytes));
                        n += 1;
                        seq += 1;
                    }
                    None => binary += 1,
                }
            }
        }
        prove_round_trip(&facts, &originals)?;
        if !facts.is_empty() {
            let p = Knowledge::add(root, &facts).map_err(|e| e.to_string())?;
            s.push_str(&format!("extracted {n} legacy files into {p}\n"));
        }
        if binary > 0 {
            s.push_str(&format!(
                "{binary} binary legacy files cannot be consumed; handle them explicitly\n"
            ));
        }
    }
    let (kr, kw) = Knowledge::compact(root).map_err(|e| e.to_string())?;
    let (hr, hw) = History::compact(root).map_err(|e| e.to_string())?;
    s.push_str(&format!(
        "knowledge: folded {kr} batches -> {}\nhistory: folded {hr} batches -> {}\n",
        kw.unwrap_or_else(|| "(empty)".into()),
        hw.unwrap_or_else(|| "(empty)".into())
    ));
    // Evidence: a stale record is dropped once a fresh record of the same evidence key
    // (locator, subject) exists.
    let pruned = evidence::prune_stale(root, &a.files, &a.declaration, &a.evidence);
    s.push_str(&format!("evidence: pruned {pruned} stale records\n"));
    // A record whose (locator, subject) no longer names a declared proof is history, not evidence.
    let commit = crate::repository::files::head_commit(root).unwrap_or_default();
    let (retired, file) =
        evidence::retire(root, &a.declaration, &commit).map_err(|e| e.to_string())?;
    s.push_str(&format!(
        "evidence: retired {retired} records of undeclared proofs into history -> {}\n",
        file.unwrap_or_else(|| "(none)".into())
    ));
    if args.flag("--prune-docs") {
        // Judge extraction on the knowledge as it is now, including what was just extracted.
        let a = assess(root)?;
        let mut n = 0;
        for d in a.docs.over_budget().filter(|d| d.extracted) {
            if std::fs::remove_file(root.join(&d.path)).is_ok() {
                n += 1;
            }
        }
        s.push_str(&format!("documents: deleted {n} extracted documents\n"));
    }
    let a = assess(root)?;
    let facts = compact::facts::encode(&a.knowledge.facts);
    let capsule = compact::encode_capsule(&a.graph.encode(), &facts, &a.counts.values());
    std::fs::create_dir_all(root.join(compact::GENERATED_DIR)).map_err(|e| e.to_string())?;
    std::fs::write(root.join(compact::CAPSULE_FILE), &capsule).map_err(|e| e.to_string())?;
    std::fs::write(root.join(compact::VIEW_FILE), view(&a)).map_err(|e| e.to_string())?;
    s.push_str(&format!(
        "capsule: {} ({} bytes)\nview: {}\n",
        compact::CAPSULE_FILE,
        capsule.len(),
        compact::VIEW_FILE
    ));
    Ok((0, s))
}

/// Refuses to write extracted facts unless they rebuild every original byte for byte.
fn prove_round_trip(facts: &[Fact], originals: &[(String, Vec<u8>)]) -> Result<(), String> {
    let stored = compact::facts::stored_texts(&compact::facts::fold(facts.iter().cloned()));
    for (path, bytes) in originals {
        match stored.get(path) {
            Some(Ok(t)) if t.as_bytes() == bytes.as_slice() => {}
            Some(Err(e)) => return Err(format!("refusing to write: {e}")),
            _ => {
                return Err(format!(
                    "refusing to write: the facts extracted from {path} do not rebuild it"
                ))
            }
        }
    }
    Ok(())
}

fn parse_kind(s: &str) -> Result<FactKind, String> {
    FactKind::from_wire(&s.to_ascii_uppercase())
        .or_else(|| FactKind::from_variant(s))
        .ok_or_else(|| {
            format!(
                "unknown fact kind `{s}`; one of {}",
                FactKind::ALL
                    .iter()
                    .map(|k| k.wire())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn kind_filter(args: &Args) -> Result<Option<FactKind>, String> {
    args.value("--kind").map(parse_kind).transpose()
}

fn fact_json(f: &Fact) -> Json {
    Json::obj()
        .with("id", f.id())
        .with("kind", f.kind.wire())
        .with("subject", &f.subject)
        .with("key", &f.key)
        .with("value", &f.value)
        .with(
            "provenance",
            f.provenance.iter().cloned().collect::<Vec<_>>(),
        )
        .with("seq", f.seq)
        .with("superseded", f.superseded.clone())
}

/// `fact add | supersede | list`: single typed facts, written one batch at a time.
fn fact_cmd(args: &Args) -> Out {
    const USAGE: &str = "fact add <kind> <subject> <key> <value> --provenance <p> [--provenance <p>...]\n     | fact supersede <kind> <subject> <key> <value> --provenance <p>...\n     | fact list [--kind <kind>] [--subject <prefix>] [--json]";
    let sub = args.positional.first().map(String::as_str).unwrap_or("");
    match sub {
        "add" | "supersede" => {
            let [kind, subject, key, value] = match &args.positional[1..] {
                [k, s, ke, v] => [k, s, ke, v],
                _ => {
                    return Err(format!(
                        "fact {sub} takes exactly <kind> <subject> <key> <value> (quote values with spaces)\nusage: {USAGE}"
                    ))
                }
            };
            let kind = parse_kind(kind)?;
            let mode = if sub == "add" {
                compact::facts::Assert::Add
            } else {
                compact::facts::Assert::Supersede
            };
            let (batch, f) = compact::facts::assert_fact(
                &args.root,
                mode,
                kind,
                subject,
                key,
                value,
                &args.values_of("--provenance"),
            )?;
            if args.json {
                return Ok((
                    0,
                    Json::obj()
                        .with("batch", batch)
                        .with("fact", fact_json(&f))
                        .render(),
                ));
            }
            Ok((
                0,
                format!(
                    "{}wrote {batch} (`ynventa compact` folds batches)\n",
                    compact::view::line(&f)
                ),
            ))
        }
        "list" => {
            let k = Knowledge::load(&args.root);
            let kind = kind_filter(args)?;
            let facts = k.current(kind, args.value("--subject"));
            if args.json {
                return Ok((
                    0,
                    Json::obj()
                        .with(
                            "facts",
                            Json::Array(facts.iter().map(|f| fact_json(f)).collect()),
                        )
                        .with("unreadable", k.unreadable.clone())
                        .render(),
                ));
            }
            let mut s: String = facts.iter().map(|f| compact::view::line(f)).collect();
            if facts.is_empty() {
                s.push_str("no facts\n");
            }
            for u in &k.unreadable {
                s.push_str(&format!("unreadable: {u}\n"));
            }
            Ok((0, s))
        }
        _ => Err(format!("usage: {USAGE}")),
    }
}

/// Whether `out` lies under `<root>/.ynventa`, judged on the absolute paths of the existing
/// ancestors (so `..` and symlinks cannot smuggle a view into canonical state).
fn under_subsystem(root: &Path, out: &Path) -> bool {
    let abs = |p: &Path| -> PathBuf {
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        };
        // Canonicalize the deepest existing ancestor; keep the rest lexically.
        let mut base = p.clone();
        let mut rest = Vec::new();
        while !base.exists() {
            match (base.parent(), base.file_name()) {
                (Some(parent), Some(name)) => {
                    rest.push(name.to_os_string());
                    base = parent.to_path_buf();
                }
                _ => break,
            }
        }
        let mut full = base.canonicalize().unwrap_or(base);
        for r in rest.into_iter().rev() {
            full.push(r);
        }
        full
    };
    abs(out).starts_with(abs(&root.join(".ynventa")))
}

/// `knowledge view | extract`: read-only views of current facts, and targeted extraction.
fn knowledge_cmd(args: &Args) -> Out {
    const USAGE: &str = "knowledge view [--kind <kind>] [--text] [--out <file>]\n     | knowledge view --document <path> [--outline] [--out <file>]\n     | knowledge extract <document or licence text>...";
    let sub = args.positional.first().map(String::as_str).unwrap_or("");
    match sub {
        "view" => {
            let k = Knowledge::load(&args.root);
            let kind = kind_filter(args)?;
            let body = match args.value("--document") {
                Some(p) => {
                    let p = p.trim_start_matches("./");
                    if args.flag("--outline") {
                        compact::view::outline(&k, p)?
                    } else {
                        compact::view::document(&k, p)?
                    }
                }
                None if args.flag("--text") => compact::view::text(&k, kind),
                None => compact::view::markdown(&k, kind),
            };
            match args.value("--out") {
                None => Ok((0, body)),
                Some(out) => {
                    let out = PathBuf::from(out);
                    if under_subsystem(&args.root, &out) {
                        return Err(format!(
                            "{} is under .ynventa/: a view is never written into canonical state",
                            out.display()
                        ));
                    }
                    if let Some(p) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&out, &body).map_err(|e| e.to_string())?;
                    Ok((
                        0,
                        format!("view: {} ({} bytes)\n", out.display(), body.len()),
                    ))
                }
            }
        }
        "extract" => {
            let paths: Vec<String> = args.positional[1..]
                .iter()
                .map(|p| p.trim_start_matches("./").trim_end_matches('/').to_string())
                .collect();
            if paths.is_empty() {
                return Err(format!("name the documents to extract\nusage: {USAGE}"));
            }
            let k = Knowledge::load(&args.root);
            if !k.unreadable.is_empty() {
                return Err(format!(
                    "unreadable knowledge; refusing to write: {}",
                    k.unreadable.join("; ")
                ));
            }
            // Validate every path before writing anything.
            let mut texts = Vec::new();
            for p in &paths {
                if p.is_empty() || p.starts_with("../") || Path::new(p).is_absolute() {
                    return Err(format!("`{p}` is not a repository-relative path"));
                }
                if p == ".ynventa" || p.starts_with(".ynventa/") {
                    return Err(format!("`{p}` is canonical state, not a document"));
                }
                if !audit::is_document(p) && !audit::is_licence_text(p) {
                    return Err(format!(
                        "`{p}` is not a document (.md, .markdown, .mdx) or a licence text"
                    ));
                }
                let text =
                    std::fs::read_to_string(args.root.join(p)).map_err(|e| format!("{p}: {e}"))?;
                texts.push((p.clone(), text));
            }
            texts.sort();
            texts.dedup();
            let commit = crate::repository::files::head_commit(&args.root);
            let mut seq = k.next_seq();
            let mut facts = Vec::new();
            let mut s = String::new();
            let mut originals = Vec::new();
            for (p, text) in &texts {
                if k.stores(p, text.as_bytes()) {
                    s.push_str(&format!("{p}: unchanged since its extraction\n"));
                    continue;
                }
                let x = if audit::is_document(p) {
                    audit::extract_at(p, text, seq, commit.as_deref())
                } else {
                    audit::extract_verbatim(p, text, seq, commit.as_deref())
                };
                let blocks = x.iter().filter(|f| f.kind == FactKind::Block).count();
                s.push_str(&format!(
                    "{p}: {} facts, {blocks} blocks (rebuilds byte for byte: `ynventa knowledge view --document {p}`)\n",
                    x.len() - 1 - blocks
                ));
                facts.extend(x);
                originals.push((p.clone(), text.clone().into_bytes()));
                seq += 1;
            }
            prove_round_trip(&facts, &originals)?;
            if !facts.is_empty() {
                let batch = Knowledge::add(&args.root, &facts).map_err(|e| e.to_string())?;
                s.push_str(&format!("wrote {batch}\n"));
            }
            Ok((0, s))
        }
        _ => Err(format!("usage: {USAGE}")),
    }
}

/// The generated human view. Never authoritative; regenerate at will.
pub fn view(a: &Assessment) -> String {
    let d = &a.declaration;
    let mut s = format!("<!-- GENERATED by `ynventa compact`: a view, not a source of truth. Do not edit or commit. -->\n# {} — Ynventa view\n\nschema `{}`\n\n## Metrics\n\n| metric | value |\n|---|---|\n", d.repository.name, crate::protocol::schema_identity());
    for (k, v) in a.counts.values() {
        s.push_str(&format!("| {k} | {v} |\n"));
    }
    s.push_str("\n## Nodes (legacy path → node → canonical path)\n\n| node | id | kind | path | canonical | native |\n|---|---|---|---|---|---|\n");
    for m in migration::pathmap::path_map(d) {
        let n = d.node(&m.node).unwrap();
        s.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} |\n",
            m.node,
            m.node_id,
            n.kind,
            m.legacy_path,
            m.canonical_path,
            a.analysis
                .node_status
                .get(&m.node)
                .map(|x| x.wire())
                .unwrap_or("-")
        ));
    }
    s.push_str("\n## Donors\n\n| donor | claimed | effective | next |\n|---|---|---|---|\n");
    for x in &a.analysis.donors {
        s.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            x.key,
            x.claimed,
            x.effective,
            x.stopped_by.replace('|', "/")
        ));
    }
    s.push_str("\n## Knowledge\n\n");
    for f in a
        .knowledge
        .facts
        .iter()
        .filter(|f| !matches!(f.kind, FactKind::Document | FactKind::Block))
    {
        s.push_str(&format!(
            "- {} {} {}: {}{}{}\n",
            f.kind,
            f.subject,
            f.key,
            f.value.replace('\n', " "),
            if f.superseded.is_empty() {
                String::new()
            } else {
                format!(" (supersedes {})", f.superseded.len())
            },
            compact::view::provenance_brief(f)
        ));
    }
    s
}

fn migrate(args: &Args) -> Out {
    let root = &args.root;
    let sub = args.positional.first().map(String::as_str).unwrap_or("");
    match sub {
        "scaffold" => {
            let r = migration::scaffold::scaffold(
                root,
                args.value("--shard"),
                args.value("--origin"),
                args.flag("--force"),
            )?;
            Ok((
                0,
                format!(
                    "subsystem files copied: {} (stale removed: {})\ndeclarations written: {}\ndeclarations migrated to the current grammar: {}\ndialects: {}\nnodes: {} donors: {} waves: {} legacy facts: {}\n",
                    r.copied,
                    r.removed,
                    r.declarations_written,
                    if r.migrated.is_empty() { "none".into() } else { r.migrated.join(", ") },
                    if r.dialects.is_empty() { "none".into() } else { r.dialects.join(", ") },
                    r.nodes,
                    r.donors,
                    r.waves,
                    r.facts
                ),
            ))
        }
        "schema" => {
            // The declarations follow the subsystem's grammar: new fields are written with their
            // undecided values (`maps_to: None`, `norl: NorlRelevance::Unresolved`) and a missing
            // organism.rs is created empty. No claim is changed; what the new ladder no longer
            // supports is reported here and by `verify` as CLAIM_EXCEEDS_EVIDENCE.
            let written = declare::migrate_schema(root)?;
            let mut s = String::new();
            for f in &written {
                s.push_str(&format!("rewrote {f}\n"));
            }
            if written.is_empty() {
                s.push_str("declarations already use the current grammar\n");
            }
            let a = crate::assess(root)?;
            let over: Vec<&crate::donors::DonorAssessment> = a
                .analysis
                .donors
                .iter()
                .filter(|x| x.claimed > x.effective && !x.resolved())
                .collect();
            for x in &over {
                s.push_str(&format!(
                    "  claim exceeds evidence: {:<32} claims {} effective {}; stopped at {}\n",
                    x.key, x.claimed, x.effective, x.stopped_by
                ));
            }
            s.push_str(&format!(
                "{} donor claims exceed their evidence under the current ladder (claims are never lowered here: map each required capability (maps_to) and resolve its Norl relevance (norl), or lower the claim deliberately)\n",
                over.len()
            ));
            Ok((0, s))
        }
        "register" => {
            // Discovered donors that participate are registered from their own package metadata.
            use crate::migration::register::{cargo_home, donor_licence, locked_versions, Licence};
            let a = crate::assess(root)?;
            let mut d = a.declaration.clone();
            // New externals enter the graph as DISCOVERED donors first.
            let mut discovered = Vec::new();
            for ((eco, name), obs) in &a.analysis.unregistered {
                let dn = crate::migration::register::discovered(
                    *eco,
                    name,
                    obs.iter().map(|o| o.file.clone()),
                );
                if d.donor(&dn.key).is_none() {
                    discovered.push(dn.key.clone());
                    d.donors.push(dn);
                }
            }
            d.donors.sort_by(|a, b| a.key.cmp(&b.key));
            // Programs collapse into the project that provides them.
            let moves = crate::migration::register::consolidate_programs(&mut d.donors);
            let active: Vec<&str> = a
                .findings
                .iter()
                .filter(|f| f.code == "DISCOVERED_BUT_ACTIVE")
                .map(|f| f.subject.as_str())
                .chain(discovered.iter().map(String::as_str))
                .collect();
            let locked = locked_versions(&a.files);
            let home = args
                .value("--cargo-home")
                .map(PathBuf::from)
                .or_else(cargo_home);
            let (mut registered, mut left) = (Vec::new(), Vec::new());
            for dn in d.donors.iter_mut().filter(|dn| {
                active.contains(&dn.key.as_str())
                    && (dn.claimed < DonorState::Registered || dn.license.is_empty())
            }) {
                if dn.origin.trim().is_empty() {
                    let native = !dn.packages.is_empty()
                        && dn.packages.iter().all(|p| p.ecosystem == Ecosystem::Native);
                    left.push((
                        dn.key.clone(),
                        if native {
                            "a program with no single providing project on every platform: record its donor by hand".to_string()
                        } else if dn.packages.is_empty() {
                            "a source donor with no origin recorded: record its origin and licence by hand".to_string()
                        } else {
                            "no origin recorded".to_string()
                        },
                    ));
                    continue;
                }
                match donor_licence(root, &a.files, home.as_deref(), &locked, dn) {
                    Licence::Found { licence, sources } => {
                        dn.license = licence.clone();
                        dn.claimed = dn.claimed.max(DonorState::Registered);
                        dn.provenance.extend(sources);
                        dn.provenance.sort();
                        dn.provenance.dedup();
                        registered.push((dn.key.clone(), licence));
                    }
                    Licence::Unavailable(why) => left.push((dn.key.clone(), why)),
                }
            }
            let dry = args.flag("--dry-run");
            let changed = !registered.is_empty() || !discovered.is_empty() || !moves.is_empty();
            if !dry && changed {
                if moves.iter().any(|(old, new)| old != new) {
                    let seq = Knowledge::load(root).next_seq();
                    let facts: Vec<Fact> = moves
                        .iter()
                        .filter(|(old, new)| old != new)
                        .map(|(old, new)| {
                            Fact::new(
                                FactKind::LegacyRecord,
                                old,
                                "merged_into",
                                new,
                                "ynventa migrate register: a program donor is the project that provides it",
                                seq,
                            )
                        })
                        .collect();
                    Knowledge::add(root, &facts).map_err(|e| e.to_string())?;
                }
                declare::store(root, &d).map_err(|e| e.to_string())?;
            }
            let mut s = String::new();
            for (old, new) in &moves {
                if old == new {
                    s.push_str(&format!("  REGISTERED {new:<32} (program: its project's origin and licence)\n"));
                } else {
                    s.push_str(&format!("  MERGED     {old:<32} -> {new} (REGISTERED)\n"));
                }
            }
            for k in &discovered {
                s.push_str(&format!("  DISCOVERED {k}\n"));
            }
            for (k, l) in &registered {
                s.push_str(&format!("  REGISTERED {k:<32} {l}\n"));
            }
            for (k, why) in &left {
                s.push_str(&format!("  left       {k:<32} {why}\n"));
            }
            s.push_str(&format!(
                "{} new donors discovered, {} program donors merged into their projects, {} donors registered from package metadata, {} left{}\n",
                discovered.len(),
                moves.len(),
                registered.len(),
                left.len(),
                if dry { " (dry run)" } else { "" }
            ));
            Ok((0, s))
        }
        "reclaim" => {
            // A donor claim above its evidence (typically inherited from `.atlas`) is lowered to
            // the effective state; the retired claim stays as a LEGACY_CLAIM fact (provenance
            // kept), so nothing is lost and the declaration states only what is proven.
            let a = crate::assess(root)?;
            let over: Vec<(String, DonorState, DonorState)> = a
                .analysis
                .donors
                .iter()
                .filter(|x| {
                    a.findings
                        .iter()
                        .any(|f| f.code == "CLAIM_EXCEEDS_EVIDENCE" && f.subject == x.key)
                })
                .map(|x| (x.key.clone(), x.claimed, x.effective))
                .collect();
            // An exception (REJECTED, BLOCKED, SUPERSEDED) the evidence does not allow is the same
            // kind of false claim: it is withdrawn and kept as a fact too.
            let mut d = a.declaration.clone();
            let illegal: Vec<(String, String)> = d
                .donors
                .iter()
                .filter(|dn| {
                    a.findings
                        .iter()
                        .any(|f| f.code == "ILLEGAL_EXCEPTION" && f.subject == dn.key)
                })
                .filter_map(|dn| {
                    let (kind, why) = dn.exception.as_ref()?;
                    Some((dn.key.clone(), format!("{}: {why}", kind.wire())))
                })
                .collect();
            let mut s = String::new();
            if over.is_empty() && illegal.is_empty() {
                s.push_str("every donor claim is within its evidence\n");
                return Ok((0, s));
            }
            let seq = Knowledge::load(root).next_seq();
            let prov = "ynventa migrate reclaim: claim above evidence";
            let facts: Vec<Fact> =
                over.iter()
                    .map(|(k, claimed, _)| {
                        Fact::new(
                            FactKind::LegacyClaim,
                            k,
                            "claimed",
                            claimed.wire(),
                            prov,
                            seq,
                        )
                    })
                    .chain(illegal.iter().map(|(k, e)| {
                        Fact::new(FactKind::LegacyClaim, k, "exception", e, prov, seq)
                    }))
                    .collect();
            for (k, _, effective) in &over {
                if let Some(dn) = d.donors.iter_mut().find(|dn| &dn.key == k) {
                    dn.claimed = *effective;
                }
            }
            for (k, _) in &illegal {
                if let Some(dn) = d.donors.iter_mut().find(|dn| &dn.key == k) {
                    dn.exception = None;
                }
            }
            if !args.flag("--dry-run") {
                Knowledge::add(root, &facts).map_err(|e| e.to_string())?;
                declare::store(root, &d).map_err(|e| e.to_string())?;
            }
            for (k, claimed, effective) in &over {
                s.push_str(&format!(
                    "  {k:<32} {} -> {}\n",
                    claimed.wire(),
                    effective.wire()
                ));
            }
            for (k, e) in &illegal {
                s.push_str(&format!("  {k:<32} exception withdrawn ({e})\n"));
            }
            s.push_str(&format!(
                "{} donor claims lowered, {} exceptions withdrawn{}\n",
                over.len(),
                illegal.len(),
                if args.flag("--dry-run") {
                    " (dry run)"
                } else {
                    "; retired claims kept as LEGACY_CLAIM facts"
                }
            ));
            Ok((0, s))
        }
        "universe" => {
            // A lossless import of one legacy registry into the repository's universe.
            let from = args
                .value("--from")
                .ok_or("migrate universe --from <legacy registry> [--kind <KIND>] [--table <name>] [--dry-run]")?;
            let kind = match args.value("--kind") {
                None => None,
                Some(k) => Some(crate::universe::import::kind_of_word(k).ok_or(format!(
                    "`{k}` is not a universe kind ({})",
                    crate::schema::UniverseKind::ALL
                        .iter()
                        .map(|k| k.wire())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))?),
            };
            let opts = crate::universe::import::Options {
                kind,
                table: args.value("--table").map(str::to_string),
            };
            let declaration = declare::load(root).ok();
            let dry = args.flag("--dry-run");
            let o = crate::universe::import::import(root, from, &opts, declaration.as_ref(), dry)?;
            Ok((0, render_import(&o, dry)))
        }
        "import" => {
            let files = crate::repository::files::Files::scan(root).map_err(|e| e.to_string())?;
            let imp = migration::atlas::import(
                root,
                &files,
                args.value("--shard"),
                args.value("--origin"),
            );
            let exists = declare::declared_dir(root)
                .join(declare::REPOSITORY_FILE)
                .exists();
            let write = !args.flag("--dry-run") && (!exists || args.flag("--force"));
            if write {
                declare::store(root, &imp.declaration).map_err(|e| e.to_string())?;
                if !imp.facts.is_empty() {
                    Knowledge::add(root, &imp.facts).map_err(|e| e.to_string())?;
                }
            }
            let mut s = format!(
                "dialects: {}\nnodes: {}\ndonors: {}\nwaves: {}\nlegacy facts: {}\n",
                imp.dialects.join(", "),
                imp.declaration.repository.nodes.len(),
                imp.declaration.donors.len(),
                imp.declaration.migration.waves.len(),
                imp.facts.len()
            );
            s.push_str(if write {
                "declarations written\n"
            } else {
                "dry run (declarations exist; --force overwrites)\n"
            });
            if !write && args.flag("--report") {
                // Judge the imported declarations in memory; nothing is written.
                let a = crate::assess_with(root, files, imp.declaration);
                let (_, gate) = gate_text(&a);
                s.push_str(&format!(
                    "\nmetrics (in memory):\n{}\n{gate}",
                    metrics_text(&a)
                ));
                let mut codes: std::collections::BTreeMap<(&str, &str), usize> =
                    std::collections::BTreeMap::new();
                for f in &a.findings {
                    *codes
                        .entry((f.severity.wire(), f.code.as_str()))
                        .or_default() += 1;
                }
                s.push_str("findings by code:\n");
                for ((sev, code), n) in codes {
                    s.push_str(&format!("  {sev:<7} {code:<26} {n}\n"));
                    for f in a.findings.iter().filter(|f| f.code == code).take(3) {
                        s.push_str(&format!("      {}: {}\n", f.subject, f.detail));
                    }
                }
            }
            Ok((0, s))
        }
        "map" | "plan" | "apply" => {
            let mut d = declare::load(root).map_err(|e| e.to_string())?;
            match sub {
                "map" => {
                    let mut s = String::new();
                    for m in migration::pathmap::path_map(&d) {
                        s.push_str(&format!(
                            "{} -> {} {} -> {}\n",
                            m.legacy_path, m.node, m.node_id, m.canonical_path
                        ));
                    }
                    Ok((0, s))
                }
                "plan" => {
                    let waves = migration::pathmap::plan(&d);
                    let mut s = String::new();
                    for w in &waves {
                        s.push_str(&format!("{} {}: {}\n", w.key, w.status, w.nodes.join(", ")));
                    }
                    if waves.is_empty() {
                        // Nothing new to plan is not the same as nothing left to move.
                        let pending = migration::pathmap::pending(&d);
                        let n: usize = pending.iter().map(|(_, m)| m.len()).sum();
                        if n == 0 {
                            s.push_str("nothing to plan: every node is at its canonical path\n");
                        } else {
                            s.push_str(&format!(
                                "nothing new to plan: {}, each already in a wave\n",
                                if n == 1 {
                                    "1 node is not at its canonical path".to_string()
                                } else {
                                    format!("{n} nodes are not at their canonical paths")
                                }
                            ));
                            for (w, ms) in &pending {
                                let nodes: Vec<String> = ms
                                    .iter()
                                    .map(|m| {
                                        format!(
                                            "{} ({} -> {})",
                                            m.node, m.legacy_path, m.canonical_path
                                        )
                                    })
                                    .collect();
                                let state = if w.status == crate::schema::WaveStatus::Applied {
                                    "marked APPLIED but unfinished"
                                } else {
                                    "pending"
                                };
                                s.push_str(&format!(
                                    "{state} {} {}: {}\n",
                                    w.key,
                                    w.status,
                                    nodes.join(", ")
                                ));
                            }
                            s.push_str("apply each with `ynventa migrate apply <wave>`\n");
                        }
                    }
                    if args.flag("--write") && !waves.is_empty() {
                        d.migration.waves.extend(waves);
                        declare::store(root, &d).map_err(|e| e.to_string())?;
                        s.push_str("waves written to .ynventa/declared/migration.rs\n");
                    }
                    Ok((0, s))
                }
                _ => {
                    let wave = args.positional.get(1).ok_or("migrate apply <wave>")?;
                    let files =
                        crate::repository::files::Files::scan(root).map_err(|e| e.to_string())?;
                    let r = migration::pathmap::apply(root, &files, &mut d, wave)?;
                    let mut s = String::new();
                    for (f, t) in &r.moves {
                        s.push_str(&format!("moved {f} -> {t}\n"));
                    }
                    for m in &r.manifests {
                        s.push_str(&format!("rewrote {m}\n"));
                    }
                    for t in &r.locks {
                        s.push_str(&format!("relocated materialization lock of {t}\n"));
                    }
                    for w in &r.declared {
                        s.push_str(&format!(
                            "declared {} `{}` {}: {} -> {}\n",
                            w.owner, w.key, w.field, w.before, w.after
                        ));
                    }
                    let mut reprove: Vec<&str> = r
                        .declared
                        .iter()
                        .filter(|w| w.field == "proof" || w.field == "source")
                        .map(|w| w.key.as_str())
                        .collect();
                    reprove.sort();
                    reprove.dedup();
                    if !reprove.is_empty() {
                        s.push_str(&format!(
                            "evidence recorded under the old locators stays history and proves nothing for the new ones; re-prove: {}\n",
                            reprove
                                .iter()
                                .map(|k| format!("`ynventa prove {k}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    for (f, p) in &r.stale_references {
                        s.push_str(&format!(
                            "stale reference to {p} in {f} (fix within this wave)\n"
                        ));
                    }
                    s.push_str(&format!("wave {wave} APPLIED; node ids unchanged\n"));
                    Ok((0, s))
                }
            }
        }
        _ => Err(
            "migrate import | map | plan [--write] | apply <wave> | scaffold | schema | reclaim [--dry-run] | register [--dry-run] [--cargo-home <dir>]"
                .into(),
        ),
    }
}

fn render_import(o: &crate::universe::import::Outcome, dry: bool) -> String {
    use crate::schema::{UniverseKind, UniverseState};
    let src = &o.source;
    let mut s = format!(
        "source {} ({} {}{}; {} bytes, {})\nrecords {} (rebuilds the registry byte for byte)\n",
        src.source,
        src.format,
        if src.table.is_empty() {
            "lines"
        } else {
            "table "
        },
        src.table,
        src.bytes,
        src.digest,
        src.records.len()
    );
    s.push_str("by kind:\n");
    for k in UniverseKind::ALL {
        let n = src.records.iter().filter(|r| r.kind == *k).count();
        if n > 0 {
            s.push_str(&format!("  {:<24} {n}\n", k.wire()));
        }
    }
    let ids: std::collections::BTreeSet<&str> = src
        .records
        .iter()
        .filter_map(|r| r.global.as_deref())
        .collect();
    let count =
        |f: &dyn Fn(&crate::universe::Record) -> bool| src.records.iter().filter(|r| f(r)).count();
    s.push_str(&format!(
        "identities: {} distinct global donors; unknown identity {}\nverified: origin {} licence {}; relevance resolved {}; selected for census {}\n",
        ids.len(),
        count(&|r| r.global.is_none()),
        count(&|r| r.origin_verified),
        count(&|r| !r.license.is_empty()),
        count(&|r| r.relevance.resolved()),
        count(&|r| r.effective() == UniverseState::SelectedForCensus),
    ));
    s.push_str("effective states:");
    for st in UniverseState::ALL {
        s.push_str(&format!(
            " {} {}",
            st.wire(),
            count(&|r| r.effective() == *st)
        ));
    }
    s.push('\n');
    s.push_str(&format!(
        "{} {} (records added {}, removed {}, changed {})\n",
        if dry {
            "dry run: would write"
        } else if o.changed {
            "wrote"
        } else {
            "unchanged"
        },
        src.file,
        o.added,
        o.removed,
        o.modified
    ));
    if dry && !o.changed {
        s.push_str("(the universe file already holds exactly this import)\n");
    }
    s
}

/// The capsules of shard roots or capsule files.
fn capsules_of(paths: &[PathBuf]) -> Result<Vec<crate::capsule::Capsule>, String> {
    let mut capsules = Vec::new();
    for path in paths {
        let c = if path.is_file() {
            let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            crate::capsule::Capsule::decode(&b).map_err(|e| format!("{}: {e}", path.display()))?
        } else {
            crate::capsule::Capsule::compile(&assess_or_import(path)?)
        };
        capsules.push(c);
    }
    Ok(capsules)
}

/// `donor summary [--root <repo>] [--ecosystem <roots or capsules>...] [--json]`.
fn donor_cmd(args: &Args) -> Out {
    let sub = args
        .positional
        .first()
        .map(String::as_str)
        .unwrap_or("summary");
    if sub != "summary" {
        return Err("donor summary [--ecosystem <roots or capsules>...] [--json]".into());
    }
    let eco = args.list("--ecosystem");
    let capsules = if eco.is_empty() {
        vec![crate::capsule::Capsule::compile(&assess_or_import(
            &args.root,
        )?)]
    } else {
        capsules_of(&eco)?
    };
    let image = (!eco.is_empty()).then(|| crate::linker::link(&capsules));
    let rows: Vec<crate::universe::summary::Row> = capsules
        .iter()
        .map(crate::universe::summary::Row::of)
        .collect();
    if args.json {
        return Ok((
            0,
            crate::universe::summary::to_json(&rows, image.as_ref()).render(),
        ));
    }
    Ok((0, crate::universe::summary::render(&rows, image.as_ref())))
}

fn protocol(args: &Args) -> Out {
    let own = Snapshot::current(own_subsystem_dir());
    if args.flag("--write") {
        std::fs::write(own_subsystem_dir().join("protocol.snapshot"), own.render())
            .map_err(|e| e.to_string())?;
        return Ok((
            0,
            format!(
                "wrote {}\n{}",
                own_subsystem_dir().join("protocol.snapshot").display(),
                own.render()
            ),
        ));
    }
    if args.flag("--schema") {
        return Ok((0, crate::protocol::schema_text()));
    }
    if args.flag("--check") {
        let at = args.root.join(SNAPSHOT_FILE);
        let recorded = Snapshot::load(&args.root);
        return Ok(match recorded {
            Some(r) if r == own => (0, format!("{} is current\n", at.display())),
            Some(r) => (
                1,
                format!(
                    "{} is stale\nrecorded:\n{}current:\n{}",
                    at.display(),
                    r.render(),
                    own.render()
                ),
            ),
            None => (1, format!("{} missing\n", at.display())),
        });
    }
    Ok((0, own.render()))
}

fn prove(args: &Args) -> Out {
    let root = &args.root;
    let d = declare::load(root).map_err(|e| e.to_string())?;
    let files = crate::repository::files::Files::scan(root).map_err(|e| e.to_string())?;
    let results = evidence::prove(
        root,
        &files,
        &d,
        args.positional.first().map(String::as_str),
        &mut evidence::CargoRunner,
    );
    let mut s = String::new();
    let mut ok = true;
    for (loc, pass, rec) in &results {
        ok &= *pass;
        s.push_str(&format!(
            "[{}] {loc} -> {rec}\n",
            if *pass { "x" } else { " " }
        ));
    }
    if results.is_empty() {
        s.push_str("no declared proofs\n");
    }
    Ok((if ok { 0 } else { 1 }, s))
}

/// Assesses a shard from its declarations, or — for a shard without `.ynventa` yet — from an
/// in-memory legacy import that writes nothing.
pub fn assess_or_import(root: &Path) -> Result<Assessment, String> {
    if declare::declared_dir(root)
        .join(declare::REPOSITORY_FILE)
        .exists()
    {
        return assess(root);
    }
    let files = crate::repository::files::Files::scan(root).map_err(|e| e.to_string())?;
    let imp = migration::atlas::import(root, &files, None, None);
    Ok(crate::assess_with(root, files, imp.declaration))
}

fn load_system(args: &Args) -> Result<Option<crate::linker::SystemImage>, String> {
    let Some(p) = args.value("--system") else {
        return Ok(None);
    };
    let b = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
    crate::linker::SystemImage::decode(&b)
        .map(Some)
        .map_err(|e| format!("{p}: {e}"))
}

fn capsule(args: &Args) -> Out {
    let a = assess_or_import(&args.root)?;
    let c = crate::capsule::Capsule::compile(&a);
    let bytes = c.encode();
    let out = args
        .value("--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| args.root.join(compact::GENERATED_DIR).join(c.file_name()));
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    std::fs::write(&out, &bytes).map_err(|e| e.to_string())?;
    Ok((
        0,
        format!(
            "capsule {} ({} bytes, {})
shard {} head {}
nodes {} edges {} symbols {} technologies {} donors {}
provides {}
requires {}
",
            out.display(),
            bytes.len(),
            crate::digest::content_digest(&bytes),
            c.shard,
            if c.head.is_empty() { "-" } else { &c.head },
            c.graph.nodes.len(),
            c.graph.edges.len(),
            c.symbols.len(),
            c.technologies.len(),
            c.donors.len(),
            c.provides().join(", "),
            c.requires().join(", ")
        ),
    ))
}

fn link_cmd(args: &Args) -> Out {
    if args.positional.is_empty() {
        return Err("link <capsule.ynv | shard root>...".into());
    }
    let mut capsules = Vec::new();
    for p in &args.positional {
        let path = Path::new(p);
        let c = if path.is_file() {
            let b = std::fs::read(path).map_err(|e| format!("{p}: {e}"))?;
            crate::capsule::Capsule::decode(&b).map_err(|e| format!("{p}: {e}"))?
        } else {
            crate::capsule::Capsule::compile(&assess_or_import(path)?)
        };
        capsules.push(c);
    }
    let image = crate::linker::link(&capsules);
    let bytes = image.encode();
    let out = args.value("--out").map(PathBuf::from).unwrap_or_else(|| {
        args.root
            .join(compact::GENERATED_DIR)
            .join(crate::linker::SYSTEM_FILE)
    });
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    std::fs::write(&out, &bytes).map_err(|e| e.to_string())?;
    let code = if image.pass() { 0 } else { 1 };
    if args.json {
        return Ok((code, image.to_json().render()));
    }
    Ok((
        code,
        format!(
            "{}image {} ({} bytes, {})\n",
            image.render_text(),
            out.display(),
            bytes.len(),
            crate::digest::content_digest(&bytes)
        ),
    ))
}

/// Resolves a user key to a node: a node key, `capability/<k>`, `technology/<k>` or `ynv://` URI.
fn resolve_key<'a>(g: &'a crate::graph::Graph, key: &str) -> Option<&'a crate::graph::GNode> {
    let k = key
        .strip_prefix("ynv://chronica/")
        .map(|r| r.replace('/', "."))
        .unwrap_or_else(|| key.to_string());
    let k = k.strip_prefix("chronica.").unwrap_or(&k).to_string();
    [
        k.clone(),
        format!("capability/{k}"),
        format!("technology/{k}"),
    ]
    .iter()
    .find_map(|c| g.node_by_key(crate::graph::SYSTEM, c))
    .or_else(|| g.nodes.values().find(|n| n.semantic_key == key))
}

fn show(args: &Args, backlinks_only: bool) -> Out {
    let key = args.positional.first().ok_or("<key>")?;
    let system = load_system(args)?;
    let local;
    let g = match &system {
        Some(s) => &s.graph,
        None => {
            local = assess_or_import(&args.root)?;
            &local.graph
        }
    };
    let n = resolve_key(g, key).ok_or(format!("no node `{key}`"))?;
    let label = |id: &crate::graph::NodeId| {
        g.nodes
            .get(id)
            .map(|m| {
                format!(
                    "{} [{}@{}]",
                    m.semantic_key,
                    m.kind,
                    if m.repository.is_empty() {
                        "-"
                    } else {
                        &m.repository
                    }
                )
            })
            .unwrap_or_else(|| format!("{id} (unresolved)"))
    };
    let mut s = format!(
        "{} {}
  id {}
  kind {} concept {}
  physical owner {}
  path {} canonical {}
  lifecycle {} native {}
",
        crate::graph::uri(&n.namespace, &n.semantic_key),
        n.name,
        n.id,
        n.kind,
        n.concept,
        if n.repository.is_empty() {
            "-"
        } else {
            &n.repository
        },
        if n.path.is_empty() { "-" } else { &n.path },
        if n.canonical_path.is_empty() {
            "-"
        } else {
            &n.canonical_path
        },
        n.lifecycle,
        n.native_status
    );
    if !backlinks_only {
        for e in g.outbound(n.id) {
            s.push_str(&format!("  -> {} {} {}\n", e.kind, e.scope, label(&e.to)));
        }
    }
    for e in g.backlinks(n.id) {
        s.push_str(&format!("  <- {} {} {}\n", e.kind, e.scope, label(&e.from)));
    }
    Ok((0, s))
}

fn context(args: &Args) -> Out {
    let a = assess_or_import(&args.root)?;
    let system = load_system(args)?;
    Ok((0, crate::context::render(&a, system.as_ref())))
}

/// `organism`: what Norl is, its organs, growth, feed chains, material, shared technologies and
/// donors — of this shard alone, or of a linked system (`--system`); `--against <older image>`
/// says what changed.
fn organism_cmd(args: &Args) -> Out {
    use crate::organism::{DonorView, TechnologyView, View};
    let system = load_system(args)?;
    let local;
    let (report, technologies, sharing, donors, tech_shard) = match &system {
        Some(sys) => {
            let runtime = |id: &str| {
                sys.graph.edges.iter().any(|e| {
                    e.to.to_string() == id
                        && matches!(
                            e.kind,
                            crate::schema::EdgeKind::DependsOn | crate::schema::EdgeKind::Calls
                        )
                        && matches!(
                            e.scope,
                            crate::schema::Scope::Runtime | crate::schema::Scope::Linked
                        )
                })
            };
            (
                sys.organism_report(),
                sys.technologies
                    .iter()
                    .map(|t| TechnologyView {
                        key: t.key.clone(),
                        shard: t.birthplace.clone(),
                        lineage: t.lineage.clone(),
                        feeds: t.feeds.clone(),
                    })
                    .collect::<Vec<_>>(),
                sys.sharing.clone(),
                sys.donors
                    .iter()
                    .map(|d| DonorView {
                        shard: d.shard.clone(),
                        key: d.key.clone(),
                        effective: d.effective,
                        runtime: runtime(&d.id),
                    })
                    .collect::<Vec<_>>(),
                sys.technologies
                    .iter()
                    .map(|t| (t.key.clone(), t.birthplace.clone()))
                    .collect(),
            )
        }
        None => {
            local = assess_or_import(&args.root)?;
            let a = &local;
            let shard = a.declaration.repository.shard.clone();
            let technologies = a
                .declaration
                .technologies
                .iter()
                .map(|t| {
                    let tid = crate::graph::NodeId::of(
                        crate::graph::SYSTEM,
                        &crate::graph::technology_key(&t.key),
                    );
                    let mut feeds: Vec<String> = a
                        .graph
                        .edges
                        .iter()
                        .filter(|e| e.from == tid && e.kind == crate::schema::EdgeKind::Feeds)
                        .map(|e| {
                            a.graph
                                .nodes
                                .get(&e.to)
                                .map(|n| n.semantic_key.clone())
                                .unwrap_or_else(|| e.to.to_string())
                        })
                        .collect();
                    feeds.sort();
                    TechnologyView {
                        key: t.key.clone(),
                        shard: shard.clone(),
                        lineage: t
                            .lineage
                            .iter()
                            .map(|k| match a.donor(k) {
                                Some(x) => format!("{k} ({})", x.effective),
                                None => k.clone(),
                            })
                            .collect(),
                        feeds,
                    }
                })
                .collect::<Vec<_>>();
            (
                a.organism.clone(),
                technologies,
                crate::technology_sharing(a),
                a.analysis
                    .donors
                    .iter()
                    .filter(|d| !d.resolved())
                    .map(|d| DonorView {
                        shard: shard.clone(),
                        key: d.key.clone(),
                        effective: d.effective,
                        runtime: !(d.facts.runtime.is_empty() && d.facts.linked.is_empty()),
                    })
                    .collect::<Vec<_>>(),
                a.declaration
                    .technologies
                    .iter()
                    .map(|t| (t.key.clone(), shard.clone()))
                    .collect(),
            )
        }
    };
    let view = View {
        report: &report,
        technologies,
        sharing: &sharing,
        donors,
        tech_shard,
    };
    let diff = match args.value("--against") {
        None => None,
        Some(p) => {
            let b = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
            let old = crate::linker::SystemImage::decode(&b).map_err(|e| format!("{p}: {e}"))?;
            Some(crate::organism::diff(&old.organism_report(), &report))
        }
    };
    if args.json {
        let mut j = view.to_json();
        if let Some(d) = diff {
            j = j.with("against", d);
        }
        return Ok((0, j.render()));
    }
    Ok((
        0,
        match diff {
            Some(d) => d,
            None => view.render_text(),
        },
    ))
}

fn technology(args: &Args) -> Out {
    let sub = args
        .positional
        .first()
        .map(String::as_str)
        .unwrap_or("list");
    let system = load_system(args)?;
    match sub {
        "materialize" => {
            let key = args.positional.get(1).ok_or(
                "technology materialize <key> --from <birthplace root> --node <node> --into <dir>",
            )?;
            let from = PathBuf::from(args.value("--from").ok_or("--from <birthplace root>")?);
            let node = args.value("--node").ok_or("--node <consuming node key>")?;
            let into = args.value("--into").ok_or("--into <directory>")?;
            let fd = declare::load(&from).map_err(|e| e.to_string())?;
            let mut d = declare::load(&args.root).map_err(|e| e.to_string())?;
            let snapshot = |d: &declare::Declaration| -> Result<_, String> {
                let files =
                    crate::repository::files::Files::scan(&args.root).map_err(|e| e.to_string())?;
                Ok(evidence::verdicts(
                    &files,
                    d,
                    &evidence::Store::load(&args.root),
                ))
            };
            let before = snapshot(&d)?;
            if d.node(node).is_none() {
                return Err(format!("no node `{node}` here"));
            }
            let m = crate::technology::materialize(&from, &fd, key, &args.root, &d, node, into)?;
            let n = d
                .repository
                .nodes
                .iter_mut()
                .find(|n| n.key == node)
                .ok_or(format!("no node `{node}` here"))?;
            if !n.reuses.contains(key) {
                n.reuses.push(key.clone());
                n.reuses.sort();
            }
            declare::store(&args.root, &d).map_err(|e| e.to_string())?;
            let after = snapshot(&d)?;
            let stale: Vec<&(String, String)> = before
                .iter()
                .filter(|(k, v)| {
                    **v == evidence::Verdict::Pass
                        && after.get(*k) != Some(&evidence::Verdict::Pass)
                })
                .map(|(k, _)| k)
                .collect();
            let mut s = format!(
                "materialized {} from {} ({})\n",
                m.technology, m.birthplace, m.source_digest
            );
            for (dest, src, _) in &m.files {
                s.push_str(&format!("  {src} -> {dest}\n"));
            }
            s.push_str(&format!("node `{node}` REUSES {key}; edits to these files are forks until re-materialized\n"));
            if !stale.is_empty() {
                let mut owners: Vec<&str> = stale
                    .iter()
                    .map(|(o, _)| o.strip_prefix("technology/").unwrap_or(o))
                    .collect();
                owners.dedup();
                s.push_str(&format!(
                    "evidence made stale by this refresh ({}); states built on it fall until re-proven:\n",
                    stale.len()
                ));
                for (owner, loc) in &stale {
                    s.push_str(&format!("  {owner}: {loc}\n"));
                }
                s.push_str(&format!(
                    "  re-prove: {}\n",
                    owners
                        .iter()
                        .map(|o| format!("`ynventa prove {o}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok((0, s))
        }
        "search" | "list" | "consumers" => {
            let needle = args
                .positional
                .get(1)
                .map(|x| x.to_ascii_lowercase())
                .unwrap_or_default();
            let mut s = String::new();
            let mut json = Vec::new();
            match &system {
                Some(sys) => {
                    for t in &sys.technologies {
                        let hay = format!("{} {} {}", t.key, t.name, t.implements.join(" "))
                            .to_ascii_lowercase();
                        let hit = match sub {
                            "consumers" => t.key == needle,
                            _ => needle.split_whitespace().all(|w| hay.contains(w)),
                        };
                        if hit {
                            s.push_str(&sys.technology_text(t));
                            json.push(sys.technology_json(t));
                        }
                    }
                    if sub == "search" {
                        for c in sys.capabilities.iter().filter(|c| {
                            c.key.to_ascii_lowercase().contains(&needle) && !needle.is_empty()
                        }) {
                            s.push_str(&format!(
                                "capability {:<30} providers {}\n",
                                c.key,
                                c.providers
                                    .iter()
                                    .map(|(sh, n)| format!("{n}@{sh}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ));
                        }
                    }
                }
                None => {
                    let a = assess_or_import(&args.root)?;
                    let shard = &a.declaration.repository.shard;
                    let sharing = crate::technology_sharing(&a);
                    for t in &a.technologies {
                        if !t.key.contains(&needle) {
                            continue;
                        }
                        let d = a.declaration.technology(&t.key).expect("declared");
                        let lineage: Vec<String> = d
                            .lineage
                            .iter()
                            .map(|k| match a.donor(k) {
                                Some(x) => format!("{k} ({})", x.effective),
                                None => k.clone(),
                            })
                            .collect();
                        let related: Vec<String> = sharing
                            .iter()
                            .filter(|x| x.a == t.key || x.b == t.key)
                            .map(|x| {
                                format!(
                                    "{} [{}]",
                                    if x.a == t.key { &x.b } else { &x.a },
                                    x.class.wire()
                                )
                            })
                            .collect();
                        let list = |v: &[String]| {
                            if v.is_empty() {
                                "-".to_string()
                            } else {
                                v.join(", ")
                            }
                        };
                        s.push_str(&format!(
                            "{} — {} ({}) claimed {} effective {}\n  born in {shard} (its owner) node {}\n  implements {}\n  implementations here {}\n  lineage {}\n  proofs {}\n  norl {}; donor-dependent: {}\n{}",
                            t.key,
                            d.name,
                            d.kind.wire(),
                            t.claimed.wire(),
                            t.effective.wire(),
                            d.node,
                            list(&d.implements),
                            list(&related),
                            list(&lineage),
                            list(&d.proofs.iter().map(|p| p.locator.clone()).collect::<Vec<_>>()),
                            d.norl.wire(),
                            if t.effective < crate::schema::TechnologyLifecycle::Native { "yes" } else { "no" },
                            if t.stopped_by.is_empty() { String::new() } else { format!("  next: {}\n", t.stopped_by) }
                        ));
                        json.push(
                            Json::obj()
                                .with("key", &t.key)
                                .with("born_in", shard.as_str())
                                .with("claimed", t.claimed.wire())
                                .with("effective", t.effective.wire())
                                .with("implements", d.implements.clone())
                                .with("implementations", related)
                                .with("lineage", lineage)
                                .with("norl", d.norl.wire()),
                        );
                    }
                }
            }
            if args.json {
                return Ok((0, Json::Array(json).render()));
            }
            if s.is_empty() {
                s.push_str("nothing found: no canonical technology exists for this need; start the research -> native technology pipeline\n");
            }
            Ok((0, s))
        }
        _ => Err("technology list | search <text> | consumers <key> | materialize <key>".into()),
    }
}
