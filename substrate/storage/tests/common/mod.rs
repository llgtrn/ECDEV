//! The application's own SQL, replayed with seeded random parameters against any engine that
//! implements [`Engine`]; each seed's outcomes fold into one digest. The native store checks its
//! digests against the frozen ones that SQLite (the donor) produced for the same replay.
#![allow(dead_code)]

use ecdev_store::Value as V;

pub const APP_SQL: &str = include_str!("../fixtures/app-sql.txt");

pub enum Out {
    Rows(Vec<Vec<V>>),
    Changes(usize),
    Err,
}

pub trait Engine {
    fn open() -> Self;
    fn run(&mut self, sql: &str, params: &[V]) -> Out;
    fn batch(&mut self, sql: &str);
}

pub struct Rng(pub u64);
impl Rng {
    pub fn seeded(seed: u64) -> Rng {
        Rng(0x9E37_79B9_7F4A_7C15 ^ (seed + 1).wrapping_mul(0x1234_5678_9ABC_DEF1))
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn param(rng: &mut Rng) -> V {
    let vocab = [
        "a",
        "b",
        "c",
        "PENDING",
        "LEASED",
        "RETRYABLE",
        "HANDLED",
        "FAILED",
        "LIVE",
        "e1",
        "x",
        "{\"a\":1}",
        "{\"research_run\":1}",
        "OBSERVED",
        "SIMULATED",
        "DISCOVERED",
        "CANCELLED",
    ];
    match rng.below(10) {
        0 => V::Null,
        1..=4 => V::Integer(rng.below(5) as i64),
        5 => V::Integer(100 + rng.below(900) as i64),
        6 => V::Real(rng.below(100) as f64 / 4.0),
        _ => V::Text(vocab[rng.below(vocab.len() as u64) as usize].to_string()),
    }
}

fn max_param(sql: &str) -> usize {
    let b = sql.as_bytes();
    let (mut m, mut i) = (0, 0);
    while i < b.len() {
        if b[i] == b'?' {
            let mut j = i + 1;
            let mut n = 0usize;
            while j < b.len() && b[j].is_ascii_digit() {
                n = n * 10 + (b[j] - b'0') as usize;
                j += 1;
            }
            m = m.max(n);
            i = j;
        } else {
            i += 1;
        }
    }
    m
}

pub fn is_query(sql: &str) -> bool {
    sql.trim_start().to_ascii_uppercase().starts_with("SELECT")
}

const TABLES: &[&str] = &[
    "runs",
    "events",
    "evidence",
    "entities",
    "candidates",
    "edges",
    "provider_accounting",
    "provider_calls",
    "fetch_cache",
    "paid_reservations",
    "simulation_runs",
    "crawl_runs",
    "crawl_urls",
    "crawl_events",
    "crawl_origins",
    "watches",
    "watch_snapshots",
    "watch_changes",
    "watch_facts",
    "price_observation_runs",
    "listing_searches",
    "social_posts",
    "trend_snapshots",
    "social_cache",
    "trend_watches",
    "social_scenarios",
    "trend_feeds",
];

fn schema() -> Vec<String> {
    let mut ddl: Vec<String> = Vec::new();
    let mut alters: Vec<String> = Vec::new();
    for line in APP_SQL.lines() {
        let Some((_, sql)) = line.split_once('\t') else {
            continue;
        };
        let up = sql.trim_start().to_ascii_uppercase();
        if up.starts_with("ALTER") {
            alters.push(sql.to_string());
        } else if up.starts_with("CREATE TABLE")
            || up.starts_with("CREATE INDEX")
            || up.starts_with("CREATE UNIQUE")
            || up.starts_with("PRAGMA JOURNAL_MODE")
        {
            ddl.push(sql.to_string());
        }
    }
    // The service tables come first: other tables reference them.
    ddl.sort_by_key(|s| !s.contains("CREATE TABLE IF NOT EXISTS runs("));
    ddl.extend(alters);
    ddl
}

pub fn templates() -> Vec<String> {
    let mut out = Vec::new();
    for line in APP_SQL.lines() {
        let Some((_, sql)) = line.split_once('\t') else {
            continue;
        };
        let up = sql.trim_start().to_ascii_uppercase();
        if !(up.starts_with("SELECT")
            || up.starts_with("INSERT")
            || up.starts_with("UPDATE")
            || up.starts_with("DELETE"))
        {
            continue;
        }
        if up.contains("; ") && !up.starts_with("SELECT") {
            continue;
        }
        let sql = sql.replace("{table}", "watch_changes");
        if sql.contains('{') {
            continue;
        }
        out.push(sql);
    }
    // Transactions: the replay interleaves them with the writes, so nesting errors, rollbacks of
    // earlier statements and commits of failed ones are all exercised.
    for sql in [
        "BEGIN",
        "BEGIN IMMEDIATE",
        "BEGIN EXCLUSIVE",
        "BEGIN DEFERRED",
        "COMMIT",
        "COMMIT",
        "ROLLBACK",
        "ROLLBACK",
    ] {
        out.push(sql.to_string());
    }
    out
}

pub fn setup<E: Engine>() -> E {
    let mut e = E::open();
    for stmt in schema() {
        e.batch(&stmt);
    }
    e
}

/// 64-bit FNV-1a over the outcome text: a regression freeze, not a security digest.
pub struct Digest(u64);
impl Digest {
    pub fn new() -> Digest {
        Digest(0xcbf2_9ce4_8422_2325)
    }
    pub fn put(&mut self, text: &str) {
        for b in text.bytes().chain([0xff]) {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    pub fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

pub fn value_text(v: &V) -> String {
    match v {
        V::Null => "N".into(),
        V::Integer(i) => format!("i{i}"),
        V::Real(f) => format!(
            "r{:016x}",
            if f.is_nan() {
                f64::NAN.to_bits()
            } else {
                f.to_bits()
            }
        ),
        V::Text(t) => format!("t{}:{t}", t.len()),
        V::Blob(b) => format!(
            "b{}",
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        ),
    }
}

fn row_text(r: &[V]) -> String {
    r.iter().map(value_text).collect::<Vec<_>>().join("|")
}

/// Rows of an unordered query are compared as a set; an error is just an error.
fn outcome_text(sql: &str, out: &Out) -> String {
    match out {
        Out::Err => "ERR".into(),
        Out::Changes(n) => format!("changes {n}"),
        Out::Rows(rows) => {
            let up = sql.to_ascii_uppercase();
            let mut lines: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
            if !up.contains("ORDER BY") && !up.contains(" LIMIT ") {
                lines.sort();
            }
            format!("rows {}\n{}", rows.len(), lines.join("\n"))
        }
    }
}

fn dump_text<E: Engine>(e: &mut E) -> String {
    let mut all = String::new();
    for t in TABLES {
        let out = e.run(&format!("SELECT *, rowid FROM {t} ORDER BY rowid"), &[]);
        let Out::Rows(rows) = out else { continue };
        let mut lines: Vec<String> = if *t == "crawl_events" {
            // Rows inserted by INSERT ... SELECT without ORDER BY come in the planner's scan order;
            // the event log is compared without its ids, as a set.
            let mut v: Vec<String> = rows.iter().map(|r| row_text(&r[1..r.len() - 1])).collect();
            v.sort();
            v
        } else {
            rows.iter().map(|r| row_text(r)).collect()
        };
        all.push_str(&format!("table {t} {}\n", lines.len()));
        for l in lines.drain(..) {
            all.push_str(&l);
            all.push('\n');
        }
    }
    all
}

pub struct Replay {
    pub digest: String,
    pub rows: usize,
    pub changes: usize,
    pub errors: usize,
}

pub fn replay<E: Engine>(seed: u64, steps: usize) -> Replay {
    let templates = templates();
    let mut e: E = setup();
    let mut rng = Rng::seeded(seed);
    let mut d = Digest::new();
    let (mut rows, mut changes, mut errors) = (0, 0, 0);
    for _ in 0..steps {
        let sql = &templates[rng.below(templates.len() as u64) as usize];
        let n = max_param(sql);
        let params: Vec<V> = (0..n).map(|_| param(&mut rng)).collect();
        let out = e.run(sql, &params);
        match &out {
            Out::Rows(_) => rows += 1,
            Out::Changes(_) => changes += 1,
            Out::Err => errors += 1,
        }
        d.put(sql);
        d.put(&outcome_text(sql, &out));
        if !is_query(sql) {
            d.put(&dump_text(&mut e));
        }
    }
    Replay {
        digest: d.hex(),
        rows,
        changes,
        errors,
    }
}

pub const JSON_DOCS: &[&str] = &[
    r#"{"a":1,"b":{"c":[1,2,{"d":"x"}],"e":null,"f":true,"g":false},"h":"sé\n","i":1.5,"j":10000000000000000000,"k":[],"l":{}}"#,
    r#"[1,"two",[3],{"four":4}]"#,
    r#"  {"research_run":1,"provider_cost":{"headers":{"etag":"abc"}}}  "#,
    r#"{"research_run":0}"#,
    r#"{"a":{"b":{"c":1}}}"#,
    "5",
    "\"str\"",
    "null",
    "true",
    "{bad",
    "",
    "{\"a\":01}",
    "{\"a\":1e5,\"b\":-0,\"c\":-0.0}",
];
pub const JSON_PATHS: &[&str] = &[
    "$",
    "$.a",
    "$.b.c",
    "$.b.c[2].d",
    "$.b.c[#-1]",
    "$.b.c[#-2]",
    "$.b.e",
    "$.b.f",
    "$.b.g",
    "$.h",
    "$.i",
    "$.j",
    "$.k",
    "$.l",
    "$[1]",
    "$[2][0]",
    "$[3].four",
    "$.research_run",
    "$.provider_cost.headers.etag",
    "$.nope",
    "$.a.b.c",
    "$.\"a\"",
    "$[9]",
    "$.c",
    "$.b",
    "$.a[0]",
];

pub fn json_digest<E: Engine>() -> String {
    let mut e: E = setup();
    let mut d = Digest::new();
    for doc in JSON_DOCS {
        for p in JSON_PATHS {
            let out = e.run(
                "SELECT json_extract(?1, ?2)",
                &[V::Text((*doc).into()), V::Text((*p).into())],
            );
            d.put(&outcome_text("SELECT json_extract", &out));
        }
    }
    d.hex()
}

pub const SEEDS: u64 = 12;
pub const STEPS: usize = 6000;

pub const BASES: &[&str] = &[
    "2024-02-29",
    "2024-02-29 13:45",
    "2024-02-29 13:45:59",
    "2024-02-29 13:45:59.123",
    "2024-02-29T13:45:59",
    "2023-12-31 23:59:59",
    "2000-01-01",
    "1999-12-31 24:00",
    "0000-01-01",
    "9999-12-31 23:59:59",
    "2024-13-01",
    "2024-00-10",
    "2024-02-30",
    "2024-1-1",
    "13:45",
    "13:45:59.9",
    "25:00",
    "2024-02-29 13:45:59+02:00",
    "2024-02-29 13:45:59-05:30",
    "2024-02-29 13:45:59Z",
    "2024-02-29 13:45:59 +02:00",
    "  2024-02-29",
    "2024-02-29 ",
    "2460000.5",
    "0",
    "1",
    "5373484.5",
    "5373484",
    "-1",
    "1700000000",
    "1700000000.5",
    "253402300799",
    "253402300800",
    "1e3",
    "abc",
    "",
    "now2",
    "2024-02-29 13:45:59.1234567",
    "2024-02-29 1:45",
    "2024-02-29 13:45:60",
    "0000-00-00",
    "2024-06-15 12:00:00.000",
    "1970-01-01",
    "1970-01-01 00:00:00",
    "-2024-01-01",
    "2024-02-29T",
    "2024-02-29 13:45:59.",
    "2024-02-29 13:45:59.x",
];
pub const MODS: &[&str] = &[
    "start of month",
    "start of year",
    "start of day",
    "unixepoch",
    "auto",
    "+1 day",
    "-1 day",
    "+7 days",
    "+0.5 days",
    "-1.5 hours",
    "+90 minutes",
    "+3600 seconds",
    "+1 second",
    "+1 month",
    "-1 month",
    "+13 months",
    "-13 months",
    "+0.5 months",
    "+1 year",
    "-1 year",
    "+0.5 years",
    "+1.25 years",
    "weekday 0",
    "weekday 3",
    "weekday 6",
    "weekday 7",
    "weekday 1.5",
    "ceiling",
    "floor",
    "bogus",
    "+1 fortnight",
    "1 day",
    "+ 1 day",
    "+1day",
    "-0 days",
    "+100000 years",
    "-5000 years",
    "+1 Day",
    "+01:30",
    "-12:00:00.250",
    "+00:00:59",
    "START OF MONTH",
    "+1 months",
    "+1.5 minutes",
    "-0.0004 seconds",
    "+2 hours",
    "-999999 days",
];
pub const FORMATS: &[&str] = &[
    "%s",
    "%Y-%m-%d",
    "%H:%M:%S",
    "%d",
    "%e",
    "%f",
    "%F",
    "%H",
    "%I",
    "%j",
    "%k",
    "%l",
    "%m",
    "%M",
    "%p",
    "%P",
    "%R",
    "%S",
    "%T",
    "%u",
    "%w",
    "%W",
    "%Y",
    "%%",
    "%Q",
    "%Y%m%d%H%M%S",
    "x%sx",
    "%",
    "%G",
    "%V",
    "%U",
    "%a",
    "%A",
    "%b",
    "%B",
    "%c",
    "%C",
    "%D",
    "%g",
    "%h",
    "%n",
    "%t",
    "%y",
    "%Z",
    "%z",
];
pub const FUNCS: &[&str] = &["date", "time", "datetime", "julianday", "unixepoch"];

pub const DATETIME_CASES: usize = 200_000;

/// One query per case: a function or format, a time value and up to three modifiers.
pub fn datetime_case(rng: &mut Rng) -> (String, Vec<String>) {
    let base = BASES[rng.below(BASES.len() as u64) as usize];
    let nm = rng.below(4) as usize;
    let mut params: Vec<String> = vec![base.to_string()];
    params.extend((0..nm).map(|_| MODS[rng.below(MODS.len() as u64) as usize].to_string()));
    if rng.below(2) == 0 {
        let f = FUNCS[rng.below(FUNCS.len() as u64) as usize];
        let marks: Vec<String> = (1..=params.len()).map(|i| format!("?{i}")).collect();
        (format!("SELECT {f}({})", marks.join(",")), params)
    } else {
        let mut p = vec![FORMATS[rng.below(FORMATS.len() as u64) as usize].to_string()];
        p.extend(params);
        let marks: Vec<String> = (1..=p.len()).map(|i| format!("?{i}")).collect();
        (format!("SELECT strftime({})", marks.join(",")), p)
    }
}

pub fn datetime_digest<E: Engine>() -> String {
    let mut e: E = setup();
    let mut rng = Rng::seeded(7);
    let mut d = Digest::new();
    for _ in 0..DATETIME_CASES {
        let (sql, params) = datetime_case(&mut rng);
        let ps: Vec<V> = params.into_iter().map(V::Text).collect();
        d.put(&outcome_text(&sql, &e.run(&sql, &ps)));
    }
    d.hex()
}

// ---- typed access, bindings and error classes --------------------------------------------------

pub fn render<T: std::fmt::Debug, E>(r: Result<T, E>, classify: &dyn Fn(&E) -> String) -> String {
    match r {
        Ok(v) => format!("ok:{v:?}"),
        Err(e) => format!("err:{}", classify(&e)),
    }
}

pub const CONV_TYPES: &[&str] = &[
    "i8",
    "i16",
    "i32",
    "i64",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "usize",
    "f32",
    "f64",
    "bool",
    "String",
    "Vec<u8>",
    "Option<i64>",
    "Option<u64>",
    "Option<String>",
    "Option<bool>",
    "Option<Vec<u8>>",
];

pub fn conv_values() -> Vec<V> {
    let mut v = vec![V::Null];
    for i in [
        0i64,
        1,
        -1,
        2,
        127,
        128,
        -128,
        -129,
        255,
        256,
        32767,
        32768,
        65535,
        65536,
        2147483647,
        2147483648,
        4294967295,
        4294967296,
        i64::MAX,
        i64::MIN,
    ] {
        v.push(V::Integer(i));
    }
    for f in [
        0.0f64,
        -0.0,
        0.5,
        1.0,
        1.5,
        3.0e38,
        1e300,
        -1e300,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ] {
        v.push(V::Real(f));
    }
    for t in ["", "abc", "12", "1.5", "\u{e9}t\u{e9}", "true"] {
        v.push(V::Text(t.to_string()));
    }
    for b in [&[][..], &[0u8][..], &[1, 2, 255][..]] {
        v.push(V::Blob(b.to_vec()));
    }
    v
}

/// Reads column 0 of `$row` as the Rust type named `$ty`, rendering the value or the error class.
#[macro_export]
macro_rules! read_as {
    ($row:expr, $ty:expr, $classify:expr) => {
        match $ty {
            "i8" => render($row.get::<_, i8>(0), $classify),
            "i16" => render($row.get::<_, i16>(0), $classify),
            "i32" => render($row.get::<_, i32>(0), $classify),
            "i64" => render($row.get::<_, i64>(0), $classify),
            "isize" => render($row.get::<_, isize>(0), $classify),
            "u8" => render($row.get::<_, u8>(0), $classify),
            "u16" => render($row.get::<_, u16>(0), $classify),
            "u32" => render($row.get::<_, u32>(0), $classify),
            "u64" => render($row.get::<_, u64>(0), $classify),
            "usize" => render($row.get::<_, usize>(0), $classify),
            "f32" => render($row.get::<_, f32>(0), $classify),
            "f64" => render($row.get::<_, f64>(0), $classify),
            "bool" => render($row.get::<_, bool>(0), $classify),
            "String" => render($row.get::<_, String>(0), $classify),
            "Vec<u8>" => render($row.get::<_, Vec<u8>>(0), $classify),
            "Option<i64>" => render($row.get::<_, Option<i64>>(0), $classify),
            "Option<u64>" => render($row.get::<_, Option<u64>>(0), $classify),
            "Option<String>" => render($row.get::<_, Option<String>>(0), $classify),
            "Option<bool>" => render($row.get::<_, Option<bool>>(0), $classify),
            "Option<Vec<u8>>" => render($row.get::<_, Option<Vec<u8>>>(0), $classify),
            other => panic!("unknown type {other}"),
        }
    };
}

/// Stores each value, then reads it back as every Rust type: one line per (value, type).
#[macro_export]
macro_rules! conversions {
    ($conn:expr, $insert:expr, $classify:expr) => {{
        let mut lines = Vec::new();
        $conn
            .execute_batch("CREATE TABLE IF NOT EXISTS conv(v)")
            .unwrap();
        for (i, v) in conv_values().iter().enumerate() {
            $conn.execute_batch("DELETE FROM conv").unwrap();
            $insert(v);
            for ty in CONV_TYPES {
                let got = $conn
                    .query_row("SELECT v FROM conv", [], |row| {
                        Ok(read_as!(row, *ty, $classify))
                    })
                    .unwrap();
                lines.push(format!("{i} {ty} {got}"));
            }
        }
        lines
    }};
}

/// Binds typed Rust values and reads back what was stored and its storage class.
#[macro_export]
macro_rules! bindings {
    ($conn:expr, $read:expr, $classify:expr) => {{
        let mut lines: Vec<String> = Vec::new();
        macro_rules! one {
            ($label:literal, $val:expr) => {{
                let r = $conn.query_row("SELECT ?1, typeof(?1)", [&$val], |row| {
                    Ok(($read(row), row.get::<_, String>(1).unwrap()))
                });
                lines.push(match r {
                    Ok((v, t)) => format!("{} {} {}", $label, value_text(&v), t),
                    Err(e) => format!("{} err:{}", $label, $classify(&e)),
                });
            }};
        }
        one!("i8", -1i8);
        one!("i16", 300i16);
        one!("i32", i32::MIN);
        one!("i64", i64::MAX);
        one!("isize", -5isize);
        one!("u8", 255u8);
        one!("u16", 65535u16);
        one!("u32", u32::MAX);
        one!("u64 small", 7u64);
        one!("u64 max i64", i64::MAX as u64);
        one!("u64 over", i64::MAX as u64 + 1);
        one!("u64 max", u64::MAX);
        one!("usize", 12usize);
        one!("usize max", usize::MAX);
        one!("f32", 1.1f32);
        one!("f32 half", 0.5f32);
        one!("f64", 0.1f64);
        one!("f64 neg zero", -0.0f64);
        one!("f64 inf", f64::INFINITY);
        one!("f64 nan", f64::NAN);
        one!("true", true);
        one!("false", false);
        one!("str", "text");
        one!("str empty", "");
        one!("String", String::from("\u{e9}"));
        one!("blob", vec![1u8, 2, 3]);
        one!("blob empty", Vec::<u8>::new());
        one!("slice", &[9u8, 8][..]);
        one!("none i64", None::<i64>);
        one!("some i64", Some(5i64));
        one!("none str", None::<String>);
        one!("some str", Some("x".to_string()));
        one!("some u64 over", Some(u64::MAX));
        lines
    }};
}

/// Statements that fail in different ways, and the class of each failure.
#[macro_export]
macro_rules! errors {
    ($conn:expr, $classify:expr) => {{
        let mut lines: Vec<String> = Vec::new();
        $conn
            .execute_batch(
                "CREATE TABLE e(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, n INTEGER CHECK(n >= 0), r INTEGER REFERENCES e(id)); INSERT INTO e VALUES(1,'a',1,NULL); INSERT INTO e VALUES(2,'b',2,1); PRAGMA foreign_keys=ON;",
            )
            .unwrap();
        macro_rules! exec {
            ($label:literal, $sql:expr, $params:expr) => {{
                lines.push(match $conn.execute($sql, $params) {
                    Ok(n) => format!("{} ok:{n}", $label),
                    Err(e) => format!("{} err:{}", $label, $classify(&e)),
                });
            }};
        }
        macro_rules! query {
            ($label:literal, $sql:expr, $params:expr) => {{
                lines.push(match $conn.query_row($sql, $params, |row| Ok(read_as!(row, "i64", &$classify))) {
                    Ok(v) => format!("{} {v}", $label),
                    Err(e) => format!("{} err:{}", $label, $classify(&e)),
                });
            }};
        }
        exec!("pk duplicate", "INSERT INTO e VALUES(1,'z',1,NULL)", []);
        exec!("unique", "INSERT INTO e VALUES(9,'a',1,NULL)", []);
        exec!("not null", "INSERT INTO e(id,name) VALUES(9,NULL)", []);
        exec!("check", "INSERT INTO e VALUES(9,'c',-1,NULL)", []);
        exec!("foreign key insert", "INSERT INTO e VALUES(9,'c',1,77)", []);
        exec!("foreign key delete", "DELETE FROM e WHERE id=1", []);
        exec!("datatype mismatch", "INSERT INTO e(id,name) VALUES('abc','z')", []);
        exec!("syntax", "SELEC 1", []);
        exec!("syntax tail", "INSERT INTO e VALUES(", []);
        exec!("no table", "INSERT INTO nope VALUES(1)", []);
        exec!("no column", "UPDATE e SET zzz=1", []);
        exec!("no function", "UPDATE e SET n=nofunc(1)", []);
        exec!("exists", "CREATE TABLE e(x)", []);
        exec!("drop missing", "DROP TABLE nope", []);
        exec!("alter missing", "ALTER TABLE nope ADD COLUMN x", []);
        exec!("commit idle", "COMMIT", []);
        exec!("rollback idle", "ROLLBACK", []);
        exec!("begin", "BEGIN", []);
        exec!("begin nested", "BEGIN", []);
        exec!("rollback", "ROLLBACK", []);
        exec!("params missing", "INSERT INTO e(id,name) VALUES(?1,?2)", [1i64]);
        exec!("params extra", "INSERT INTO e(id,name) VALUES(?1,?2)", [1i64, 2, 3]);
        exec!("params none", "INSERT INTO e(id,name) VALUES(?1,?2)", []);
        exec!("returns rows", "SELECT id FROM e", []);
        exec!("returns no rows", "SELECT id FROM e WHERE id=-1", []);
        exec!("to_sql", "INSERT INTO e(id,name) VALUES(?1,'q')", [u64::MAX]);
        exec!("ok insert", "INSERT INTO e(id,name) VALUES(?1,?2)", ["5", "five"]);
        query!("rows", "SELECT id FROM e WHERE id=1", []);
        query!("no rows", "SELECT id FROM e WHERE id=-1", []);
        query!("overflow", "SELECT sum(x) FROM (SELECT 9223372036854775807 AS x UNION ALL SELECT 1)", []);
        query!("text as int", "SELECT name FROM e WHERE id=1", []);
        lines.push(match $conn.query_row("SELECT id FROM e WHERE id=1", [], |row| row.get::<_, i64>(5)) {
            Ok(v) => format!("index ok:{v}"),
            Err(e) => format!("index err:{}", $classify(&e)),
        });
        lines.push(match $conn.query_row("SELECT id FROM e WHERE id=1", [], |row| row.get::<_, i64>("nope")) {
            Ok(v) => format!("name ok:{v}"),
            Err(e) => format!("name err:{}", $classify(&e)),
        });
        lines.push(match $conn.query_row("SELECT id AS ID FROM e WHERE id=1", [], |row| row.get::<_, i64>("id")) {
            Ok(v) => format!("name case ok:{v}"),
            Err(e) => format!("name case err:{}", $classify(&e)),
        });
        lines
    }};
}

/// The native store's side of the typed checks; the donor's side lives in the scratch freezer.
pub mod native {
    use super::*;
    use ecdev_store::{Connection, ErrorKind};

    pub fn class(e: &ecdev_store::Error) -> String {
        match e.kind {
            ErrorKind::Constraint => "constraint",
            ErrorKind::Busy => "busy",
            ErrorKind::Type => "type",
            ErrorKind::Syntax
            | ErrorKind::NoSuchTable
            | ErrorKind::NoSuchColumn
            | ErrorKind::Misuse => "sql",
            ErrorKind::QueryReturnedNoRows => "no_rows",
            ErrorKind::ColumnType => "column_type",
            ErrorKind::OutOfRange => "out_of_range",
            ErrorKind::ColumnIndex => "column_index",
            ErrorKind::ColumnName => "column_name",
            ErrorKind::InvalidParameterCount => "param_count",
            ErrorKind::ToSql => "to_sql",
            ErrorKind::ExecuteReturnedResults => "returned_results",
            ErrorKind::MultipleStatement => "multiple",
            ErrorKind::Io | ErrorKind::Corrupt => "io",
        }
        .to_string()
    }

    pub fn conversions() -> Vec<String> {
        let conn = Connection::open_in_memory().unwrap();
        conversions!(
            conn,
            |v: &V| {
                conn.execute("INSERT INTO conv VALUES(?1)", [v]).unwrap();
            },
            &class
        )
    }

    pub fn bindings() -> Vec<String> {
        let conn = Connection::open_in_memory().unwrap();
        bindings!(
            conn,
            |row: &ecdev_store::Row| row.get::<_, V>(0).unwrap(),
            class
        )
    }

    pub fn errors() -> Vec<String> {
        let conn = Connection::open_in_memory().unwrap();
        errors!(conn, class)
    }
}

pub fn lines_digest(lines: &[String]) -> String {
    let mut d = Digest::new();
    for l in lines {
        d.put(l);
    }
    d.hex()
}

/// Runs two engines in lockstep and describes the first statement whose outcome or resulting
/// table contents differ. Used to debug a digest mismatch.
pub fn lockstep<A: Engine, B: Engine>(seed: u64, steps: usize) -> Option<String> {
    let templates = templates();
    let mut a: A = setup();
    let mut b: B = setup();
    let mut rng = Rng::seeded(seed);
    for step in 0..steps {
        let sql = &templates[rng.below(templates.len() as u64) as usize];
        let n = max_param(sql);
        let params: Vec<V> = (0..n).map(|_| param(&mut rng)).collect();
        let (x, y) = (a.run(sql, &params), b.run(sql, &params));
        let (tx, ty) = (outcome_text(sql, &x), outcome_text(sql, &y));
        if tx != ty {
            return Some(format!(
                "step {step}: {sql}\n  params {params:?}\n  A: {}\n  B: {}",
                tx.lines().take(6).collect::<Vec<_>>().join(" / "),
                ty.lines().take(6).collect::<Vec<_>>().join(" / ")
            ));
        }
        if !is_query(sql) {
            let (da, db) = (dump_text(&mut a), dump_text(&mut b));
            if da != db {
                let first = da
                    .lines()
                    .zip(db.lines())
                    .find(|(p, q)| p != q)
                    .map(|(p, q)| format!("A: {p}\n  B: {q}"))
                    .unwrap_or_else(|| format!("lengths {} vs {}", da.len(), db.len()));
                return Some(format!(
                    "step {step}: {sql}\n  params {params:?}\n  state differs: {first}"
                ));
            }
        }
    }
    None
}
