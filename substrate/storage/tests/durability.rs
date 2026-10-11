use ecdev_store::{Connection, OptionalExtension, TransactionBehavior, Value, params};
use std::{path::PathBuf, process::Command, time::Duration};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ecdev-store-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("db.ecdb")
}

fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn data_survives_reopen_and_ddl_replays() {
    let path = temp("reopen");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, n REAL DEFAULT 1.5); CREATE INDEX t_n ON t(n); PRAGMA user_version=7;").unwrap();
        db.execute("INSERT INTO t(name) VALUES(?1)", ["a"]).unwrap();
        db.execute("INSERT INTO t(name,n) VALUES(?1,?2)", params!["b", 2.25])
            .unwrap();
        db.execute(
            "ALTER TABLE t ADD COLUMN extra INTEGER NOT NULL DEFAULT 4",
            [],
        )
        .unwrap();
        db.execute("DELETE FROM t WHERE name='b'", []).unwrap();
    }
    let db = Connection::open(&path).unwrap();
    assert_eq!(count(&db, "t"), 1);
    let (id, n, extra): (i64, f64, i64) = db
        .query_row("SELECT id,n,extra FROM t WHERE name='a'", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!((id, n, extra), (1, 1.5, 4));
    let v: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 7);
    // AUTOINCREMENT never reuses an id, even after the newest row was deleted.
    db.execute("INSERT INTO t(name) VALUES('c')", []).unwrap();
    let id: i64 = db
        .query_row("SELECT id FROM t WHERE name='c'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, 3);
    assert!(
        db.execute("INSERT INTO t(name) VALUES('a')", [])
            .unwrap_err()
            .is_constraint()
    );
}

#[test]
fn a_torn_tail_is_ignored_and_overwritten() {
    let path = temp("torn");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE t(n INTEGER)").unwrap();
        for i in 0..5 {
            db.execute("INSERT INTO t VALUES(?1)", [i]).unwrap();
        }
    }
    let full = std::fs::read(&path).unwrap();
    // Every prefix is a valid log of some number of whole commits plus a torn tail.
    let mut expected_prev = None;
    for cut in (16..=full.len()).step_by(1) {
        std::fs::write(&path, &full[..cut]).unwrap();
        let db = Connection::open(&path).unwrap();
        let c = db
            .query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
            .optional()
            .unwrap_or(None);
        let c = c.unwrap_or(-1);
        if let Some(p) = expected_prev {
            assert!(c >= p, "commits only ever appear");
        }
        expected_prev = Some(c);
        if cut == full.len() {
            assert_eq!(c, 5);
        }
        // A writer cuts the torn tail off and carries on.
        if c >= 0 {
            db.execute("INSERT INTO t VALUES(99)", []).unwrap();
            let again = Connection::open(&path).unwrap();
            assert_eq!(count(&again, "t"), c + 1);
        }
    }
    // Flipped bits inside the last record are caught by its checksum.
    let mut bad = full.clone();
    let at = bad.len() - 6;
    bad[at] ^= 0x40;
    std::fs::write(&path, &bad).unwrap();
    let db = Connection::open(&path).unwrap();
    assert_eq!(count(&db, "t"), 4);
}

#[test]
fn threads_with_their_own_connections_serialise_on_the_file() {
    let path = temp("threads");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE counter(id INTEGER PRIMARY KEY, n INTEGER NOT NULL); INSERT INTO counter VALUES(1, 0); CREATE TABLE log(id INTEGER PRIMARY KEY AUTOINCREMENT, who INTEGER)").unwrap();
    }
    let handles: Vec<_> = (0..4)
        .map(|who| {
            let path = path.clone();
            std::thread::spawn(move || {
                let mut db = Connection::open(&path).unwrap();
                db.busy_timeout(Duration::from_secs(30)).unwrap();
                for _ in 0..60 {
                    let tx = db
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .unwrap();
                    let n: i64 = tx
                        .query_row("SELECT n FROM counter WHERE id=1", [], |r| r.get(0))
                        .unwrap();
                    tx.execute("UPDATE counter SET n=?1 WHERE id=1", [n + 1])
                        .unwrap();
                    tx.execute("INSERT INTO log(who) VALUES(?1)", [who])
                        .unwrap();
                    tx.commit().unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT n FROM counter", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        240
    );
    assert_eq!(count(&db, "log"), 240);
}

#[test]
fn rollback_and_failed_statements_leave_nothing_behind() {
    let path = temp("rollback");
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT NOT NULL)")
        .unwrap();
    {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute("INSERT INTO t VALUES(1,'a')", []).unwrap();
        assert!(tx.execute("INSERT INTO t VALUES(2,NULL)", []).is_err());
        assert!(tx.execute("INSERT INTO t VALUES(1,'dup')", []).is_err());
        tx.execute("INSERT INTO t VALUES(3,'c')", []).unwrap();
        // dropped without commit
    }
    assert_eq!(count(&db, "t"), 0);
    // A multi-row statement that fails halfway applies none of its rows.
    assert!(
        db.execute("INSERT INTO t VALUES(1,'a'),(2,'b'),(1,'c')", [])
            .is_err()
    );
    assert_eq!(count(&db, "t"), 0);
    drop(db);
    assert_eq!(count(&Connection::open(&path).unwrap(), "t"), 0);
}

#[test]
fn compaction_rewrites_the_log_and_other_handles_follow() {
    let path = temp("compact");
    let a = Connection::open(&path).unwrap();
    let b = Connection::open(&path).unwrap();
    a.set_compaction_threshold(20_000);
    a.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)")
        .unwrap();
    for i in 0..400 {
        a.execute(
            "INSERT OR REPLACE INTO t VALUES(1, ?1)",
            [format!("{i}-{}", "x".repeat(200))],
        )
        .unwrap();
    }
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size < 100_000, "log was compacted: {size}");
    let v: String = b
        .query_row("SELECT v FROM t WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert!(v.starts_with("399-"));
    b.execute("INSERT INTO t VALUES(2,'from b')", []).unwrap();
    assert_eq!(count(&a, "t"), 2);
    assert_eq!(count(&Connection::open(&path).unwrap(), "t"), 2);
}

/// Child mode of the kill test: commit pairs of rows forever.
#[test]
fn zz_child_writer() {
    let Ok(path) = std::env::var("ECDEV_STORE_CHILD") else {
        return;
    };
    let mut db = Connection::open(&path).unwrap();
    db.busy_timeout(Duration::from_secs(30)).unwrap();
    loop {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let n: i64 = tx
            .query_row("SELECT COALESCE(MAX(n),0)+1 FROM a", [], |r| r.get(0))
            .unwrap();
        tx.execute("INSERT INTO a VALUES(?1)", [n]).unwrap();
        tx.execute("INSERT INTO b VALUES(?1)", [n]).unwrap();
        tx.commit().unwrap();
    }
}

#[test]
fn killing_the_writer_never_leaves_half_a_transaction() {
    let path = temp("kill");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE a(n INTEGER PRIMARY KEY); CREATE TABLE b(n INTEGER PRIMARY KEY)",
        )
        .unwrap();
    }
    let exe = std::env::current_exe().unwrap();
    let mut last = 0;
    for round in 0..12u64 {
        let mut child = Command::new(&exe)
            .args(["--exact", "zz_child_writer", "--nocapture"])
            .env("ECDEV_STORE_CHILD", &path)
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(40 + round * 17));
        child.kill().unwrap();
        child.wait().unwrap();
        let db = Connection::open(&path).unwrap();
        let (a, b): (i64, i64) = (count(&db, "a"), count(&db, "b"));
        assert_eq!(a, b, "both tables of a transaction or neither");
        let max: i64 = db
            .query_row("SELECT COALESCE(MAX(n),0) FROM a", [], |r| r.get(0))
            .unwrap();
        assert_eq!(max, a, "no gaps");
        assert!(a >= last, "committed work is never lost");
        last = a;
    }
    assert!(last > 0, "the writer made progress");
}

#[test]
fn values_round_trip() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE v(a, b INTEGER, c REAL, d TEXT, e BLOB)")
        .unwrap();
    db.execute(
        "INSERT INTO v VALUES(?1,?2,?3,?4,?5)",
        params![
            Value::Null,
            i64::MIN,
            -0.5f64,
            "é\u{1F600}",
            vec![0u8, 255, 7]
        ],
    )
    .unwrap();
    let (b, c, d, e): (i64, f64, String, Vec<u8>) = db
        .query_row("SELECT b,c,d,e FROM v", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(
        (b, c, d.as_str(), e),
        (i64::MIN, -0.5, "é\u{1F600}", vec![0, 255, 7])
    );
    let a: Option<i64> = db.query_row("SELECT a FROM v", [], |r| r.get(0)).unwrap();
    assert_eq!(a, None);
}

#[test]
fn a_sqlite_file_is_refused_not_overwritten() {
    let path = temp("foreign");
    let mut bytes = b"SQLite format 3\0".to_vec();
    bytes.resize(4096, 0);
    std::fs::write(&path, &bytes).unwrap();
    let err = Connection::open(&path)
        .err()
        .expect("a SQLite file must not open");
    assert!(err.to_string().contains("not an ECDEV database"), "{err}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "the foreign file is left untouched"
    );
}

#[test]
fn an_open_write_transaction_makes_other_connections_wait_then_see_the_commit() {
    // Unlike SQLite in WAL mode, readers on other connections wait for an open write transaction
    // instead of reading the previous commit; the application's write transactions are short.
    let path = temp("waiting");
    let mut a = Connection::open(&path).unwrap();
    a.execute_batch("CREATE TABLE t(x)").unwrap();
    let b = Connection::open(&path).unwrap();
    b.busy_timeout(Duration::from_millis(100)).unwrap();
    let tx = a
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    tx.execute("INSERT INTO t VALUES(1)", []).unwrap();
    let blocked = b.execute("INSERT INTO t VALUES(2)", []).unwrap_err();
    assert_eq!(blocked.kind, ecdev_store::ErrorKind::Busy);
    let waiting = b
        .query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
        .unwrap_err();
    assert_eq!(waiting.kind, ecdev_store::ErrorKind::Busy);
    tx.commit().unwrap();
    b.busy_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(count(&b, "t"), 1);
    b.execute("INSERT INTO t VALUES(2)", []).unwrap();
    assert_eq!(count(&a, "t"), 2);
}
