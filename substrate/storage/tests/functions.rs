//! Worked examples of the date, time and JSON functions, each a fact of SQLite's documented
//! behaviour (the large-scale comparison with the donor is in sql_parity.rs).
use ecdev_store::Connection;

fn text(db: &Connection, sql: &str) -> Option<String> {
    db.query_row(sql, [], |r| r.get::<_, Option<String>>(0))
        .unwrap()
}

#[test]
fn date_and_time_functions_follow_the_documented_examples() {
    let db = Connection::open_in_memory().unwrap();
    for (sql, want) in [
        ("SELECT date('2004-01-01','+1 month')", Some("2004-02-01")),
        ("SELECT date('2024-01-31','+1 month')", Some("2024-03-02")),
        (
            "SELECT date('2024-01-31','+1 month','floor')",
            Some("2024-02-29"),
        ),
        ("SELECT date('2024-02-29','weekday 0')", Some("2024-03-03")),
        (
            "SELECT date('2024-02-29','start of month','+1 month','-1 day')",
            Some("2024-02-29"),
        ),
        (
            "SELECT datetime(0,'unixepoch')",
            Some("1970-01-01 00:00:00"),
        ),
        (
            "SELECT datetime('2000-01-01 12:00','+36 hours')",
            Some("2000-01-03 00:00:00"),
        ),
        ("SELECT time('13:45:59.999')", Some("13:45:59")),
        ("SELECT strftime('%s','2000-01-01')", Some("946684800")),
        (
            "SELECT strftime('%Y-%m-%dT%H:%M:%S','2004-01-01 02:34:56')",
            Some("2004-01-01T02:34:56"),
        ),
        ("SELECT strftime('%j %w %W','2024-12-31')", Some("366 2 53")),
        ("SELECT strftime('%G-W%V','2021-01-03')", Some("2020-W53")),
        ("SELECT date('2024-13-01')", None),
        ("SELECT date('not a date')", None),
        ("SELECT date(NULL)", None),
    ] {
        assert_eq!(text(&db, sql).as_deref(), want, "{sql}");
    }
    let jd: f64 = db
        .query_row("SELECT julianday('2000-01-01 12:00')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(jd, 2451545.0);
    let unix: i64 = db
        .query_row("SELECT unixepoch('2024-02-29 12:00:00')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(unix, 1709208000);
    // The calls the application makes: the start of the current month, as Unix seconds.
    let start: i64 = db
        .query_row(
            "SELECT CAST(strftime('%s','now','start of month') AS INTEGER)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let now: i64 = db
        .query_row("SELECT CAST(strftime('%s','now') AS INTEGER)", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(start <= now && now - start < 32 * 86_400 && start % 86_400 == 0);
    // Modifiers that need a time zone database are refused instead of ignored.
    for sql in [
        "SELECT date('now','localtime')",
        "SELECT datetime('now','utc')",
        "SELECT strftime('%J','now')",
    ] {
        assert!(
            db.query_row(sql, [], |r| r.get::<_, Option<String>>(0))
                .is_err(),
            "{sql}"
        );
    }
}

#[test]
fn json_extract_follows_the_documented_path_forms() {
    let db = Connection::open_in_memory().unwrap();
    let doc = r#"{"a":[1,2,{"b":"x"}],"c":{"d":true,"e":null},"f":1.5,"g":"sé"}"#;
    for (path, want) in [
        ("$.a[0]", Some("1")),
        ("$.a[2].b", Some("x")),
        ("$.a[#-1].b", Some("x")),
        ("$.c.d", Some("1")),
        ("$.c.e", None),
        ("$.f", Some("1.5")),
        ("$.g", Some("s\u{e9}")),
        ("$.a", Some("[1,2,{\"b\":\"x\"}]")),
        ("$.nope", None),
        ("$.a[9]", None),
    ] {
        let got = db.query_row("SELECT json_extract(?1, ?2)", [doc, path], |r| {
            r.get::<_, Option<String>>(0)
        });
        // Numbers come back as numbers, so read them through a text cast.
        let got = got.unwrap_or_else(|_| {
            let n: f64 = db
                .query_row("SELECT json_extract(?1, ?2)", [doc, path], |r| r.get(0))
                .unwrap();
            Some(if n.fract() == 0.0 {
                format!("{}", n as i64)
            } else {
                n.to_string()
            })
        });
        assert_eq!(got.as_deref(), want, "{path}");
    }
    assert!(
        db.query_row("SELECT json_extract('{bad', '$.a')", [], |r| r
            .get::<_, Option<String>>(0))
            .is_err()
    );
    assert!(
        db.query_row("SELECT json_extract('{}', 'a')", [], |r| r
            .get::<_, Option<String>>(0))
            .is_err()
    );
}
