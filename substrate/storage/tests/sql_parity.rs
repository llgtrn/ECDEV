//! The application's SQL replays on the native store with seeded random parameters; each seed's
//! outcome digest must equal the one SQLite produced when the donor was still a dependency.
#[macro_use]
mod common;
use common::*;
use ecdev_store::Value as V;

struct Native(ecdev_store::Connection);
impl Engine for Native {
    fn open() -> Self {
        Native(ecdev_store::Connection::open_in_memory().unwrap())
    }
    fn batch(&mut self, sql: &str) {
        if sql
            .trim_start()
            .to_ascii_uppercase()
            .starts_with("PRAGMA JOURNAL_MODE")
        {
            let _ = self.0.execute_batch(sql);
        } else {
            self.0
                .execute_batch(sql)
                .unwrap_or_else(|e| panic!("schema: {e}: {sql}"));
        }
    }
    fn run(&mut self, sql: &str, params: &[V]) -> Out {
        if is_query(sql) {
            let Ok(mut st) = self.0.prepare(sql) else {
                return Out::Err;
            };
            let rows = st.query_map(params, |r| {
                Ok((0..r.len())
                    .map(|i| r.get::<_, V>(i).unwrap())
                    .collect::<Vec<V>>())
            });
            match rows {
                Ok(it) => it
                    .collect::<Result<Vec<_>, _>>()
                    .map_or(Out::Err, Out::Rows),
                Err(_) => Out::Err,
            }
        } else {
            self.0
                .prepare(sql)
                .and_then(|mut s| s.execute(params))
                .map_or(Out::Err, Out::Changes)
        }
    }
}

fn frozen_datetime() -> String {
    let text = include_str!("fixtures/sql-differential-digests.json");
    let line = text.lines().find(|l| l.contains("\"datetime\"")).unwrap();
    line.split("\"digest\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string()
}

fn frozen() -> (Vec<(u64, String)>, String) {
    let text = include_str!("fixtures/sql-differential-digests.json");
    let mut seeds = Vec::new();
    let mut json = String::new();
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("{\"seed\":") {
            let seed: u64 = rest.split(',').next().unwrap().parse().unwrap();
            let digest = rest
                .split("\"digest\":\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .to_string();
            seeds.push((seed, digest));
        } else if let Some(rest) = line.trim().strip_prefix("\"json_extract\":\"") {
            json = rest.trim_end_matches(['"', ',']).to_string();
        }
    }
    (seeds, json)
}

#[test]
fn application_sql_matches_the_frozen_sqlite_digests() {
    assert!(templates().len() > 80);
    let (seeds, _) = frozen();
    assert_eq!(seeds.len() as u64, SEEDS);
    for (seed, want) in seeds {
        let got = replay::<Native>(seed, STEPS);
        assert!(
            got.rows > 2000 && got.changes > 2000 && got.errors > 500,
            "seed {seed} exercised too little"
        );
        assert_eq!(got.digest, want, "seed {seed} diverges from SQLite");
    }
}

#[test]
fn json_extract_matches_the_frozen_sqlite_digest() {
    assert_eq!(json_digest::<Native>(), frozen().1);
}

#[test]
fn date_and_time_functions_match_the_frozen_sqlite_digest() {
    assert_eq!(datetime_digest::<Native>(), frozen_datetime());
}

#[test]
fn a_script_is_an_error_for_execute_and_prepare_but_not_execute_batch() {
    // The donor silently runs only the first statement; the native store refuses, so a second
    // statement can never be dropped unnoticed.
    let db = ecdev_store::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES(1); INSERT INTO t VALUES(2);")
        .unwrap();
    let e = db
        .execute("DELETE FROM t WHERE x=1; DELETE FROM t WHERE x=2", [])
        .unwrap_err();
    assert_eq!(e.kind, ecdev_store::ErrorKind::MultipleStatement);
    assert_eq!(
        db.prepare("SELECT 1; SELECT 2").err().map(|e| e.kind),
        Some(ecdev_store::ErrorKind::MultipleStatement)
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn typed_access_bindings_and_error_classes_match_the_frozen_sqlite_digests() {
    let text = include_str!("fixtures/sql-differential-digests.json");
    let want = |section: &str| -> String {
        let line = text
            .lines()
            .find(|l| l.contains(&format!("\"{section}\":{{")))
            .unwrap();
        line.split("\"digest\":\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        lines_digest(&common::native::conversions()),
        want("conversions")
    );
    assert_eq!(lines_digest(&common::native::bindings()), want("bindings"));
    assert_eq!(lines_digest(&common::native::errors()), want("errors"));
}
