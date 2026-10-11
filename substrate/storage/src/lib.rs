//! ECDEV's embedded SQL store.
//!
//! A small relational engine over a durable log: the SQL the application uses (tables with
//! primary keys, unique and foreign-key constraints, indexes, joins, subqueries, aggregates,
//! upserts, transactions) runs against an in-memory image that is rebuilt from, and every commit
//! appended to, a checksummed write-ahead log with cross-process file locking.
//!
//! The Rust API follows the shape of the embedded database it replaces (connections, prepared
//! statements, parameters, rows, transactions) so callers read the same.

mod ast;
mod datetime;
mod db;
mod error;
mod eval;
mod exec;
mod jsonlite;
mod parser;
mod persist;
mod value;

pub use error::{Error, ErrorKind, Result};
pub use value::Value;

use ast::{BeginKind, Stmt};
use db::Database;
use exec::{Functions, Session};
use parser::Parsed;
use persist::{LockMode, LogFile, Op};
use std::{cell::RefCell, path::Path, rc::Rc, time::Duration};

// ---- parameters and values -----------------------------------------------------------------------

pub trait ToSql {
    fn to_sql(&self) -> Result<Value>;
}

macro_rules! to_sql_int {
    ($($t:ty),*) => {$(
        impl ToSql for $t {
            fn to_sql(&self) -> Result<Value> {
                i64::try_from(*self).map(Value::Integer).map_err(|_| Error::new(ErrorKind::ToSql, "integer out of range for storage"))
            }
        }
    )*};
}
to_sql_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl ToSql for f64 {
    fn to_sql(&self) -> Result<Value> {
        Ok(Value::real(*self))
    }
}
impl ToSql for f32 {
    fn to_sql(&self) -> Result<Value> {
        Ok(Value::real(f64::from(*self)))
    }
}
impl ToSql for bool {
    fn to_sql(&self) -> Result<Value> {
        Ok(Value::Integer(i64::from(*self)))
    }
}
impl ToSql for str {
    fn to_sql(&self) -> Result<Value> {
        Ok(Value::Text(self.to_string()))
    }
}
impl ToSql for String {
    fn to_sql(&self) -> Result<Value> {
        Ok(Value::Text(self.clone()))
    }
}
impl ToSql for [u8] {
    fn to_sql(&self) -> Result<Value> {
        Ok(Value::Blob(self.to_vec()))
    }
}
impl ToSql for Vec<u8> {
    fn to_sql(&self) -> Result<Value> {
        Ok(Value::Blob(self.clone()))
    }
}
impl ToSql for Value {
    fn to_sql(&self) -> Result<Value> {
        Ok(match self {
            Value::Real(f) => Value::real(*f),
            other => other.clone(),
        })
    }
}
impl<T: ToSql + ?Sized> ToSql for &T {
    fn to_sql(&self) -> Result<Value> {
        (**self).to_sql()
    }
}
impl<T: ToSql> ToSql for Option<T> {
    fn to_sql(&self) -> Result<Value> {
        match self {
            Some(v) => v.to_sql(),
            None => Ok(Value::Null),
        }
    }
}
impl<T: ToSql + ?Sized> ToSql for Box<T> {
    fn to_sql(&self) -> Result<Value> {
        (**self).to_sql()
    }
}

pub trait Params {
    fn bind(&self) -> Result<Vec<Value>>;
}

impl Params for () {
    fn bind(&self) -> Result<Vec<Value>> {
        Ok(Vec::new())
    }
}
// `[]` must stay unambiguous, so the empty array has one non-generic impl and the
// generic ones are written per length (as the donor does).
impl Params for [&dyn ToSql; 0] {
    fn bind(&self) -> Result<Vec<Value>> {
        Ok(Vec::new())
    }
}
macro_rules! array_params {
    ($($n:literal)+) => {$(
        impl<T: ToSql> Params for [T; $n] {
            fn bind(&self) -> Result<Vec<Value>> {
                self.iter().map(ToSql::to_sql).collect()
            }
        }
        impl<T: ToSql> Params for &[T; $n] {
            fn bind(&self) -> Result<Vec<Value>> {
                self.iter().map(ToSql::to_sql).collect()
            }
        }
    )+};
}
array_params!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32);
impl<T: ToSql> Params for &[T] {
    fn bind(&self) -> Result<Vec<Value>> {
        self.iter().map(ToSql::to_sql).collect()
    }
}
impl<T: ToSql> Params for Vec<T> {
    fn bind(&self) -> Result<Vec<Value>> {
        self.iter().map(ToSql::to_sql).collect()
    }
}

#[macro_export]
macro_rules! params {
    () => {
        &[] as &[&dyn $crate::ToSql]
    };
    ($($param:expr),+ $(,)?) => {
        &[$(&$param as &dyn $crate::ToSql),+] as &[&dyn $crate::ToSql]
    };
}

pub trait FromSql: Sized {
    fn from_sql(v: &Value) -> Result<Self>;
}

fn mismatch<T>(v: &Value, want: &str) -> Result<T> {
    Err(Error::new(
        ErrorKind::ColumnType,
        format!("invalid column type {}, expected {want}", v.type_name()),
    ))
}

macro_rules! from_sql_int {
    ($($t:ty),*) => {$(
        impl FromSql for $t {
            fn from_sql(v: &Value) -> Result<$t> {
                match v {
                    Value::Integer(i) => <$t>::try_from(*i).map_err(|_| Error::new(ErrorKind::OutOfRange, "integer out of range")),
                    other => mismatch(other, "integer"),
                }
            }
        }
    )*};
}
from_sql_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl FromSql for f64 {
    fn from_sql(v: &Value) -> Result<f64> {
        match v {
            Value::Real(f) => Ok(*f),
            Value::Integer(i) => Ok(*i as f64),
            other => mismatch(other, "real"),
        }
    }
}
impl FromSql for f32 {
    fn from_sql(v: &Value) -> Result<f32> {
        match v {
            Value::Real(f) => Ok(*f as f32),
            Value::Integer(i) => Ok(*i as f32),
            other => mismatch(other, "real"),
        }
    }
}
impl FromSql for bool {
    fn from_sql(v: &Value) -> Result<bool> {
        match v {
            Value::Integer(i) => Ok(*i != 0),
            other => mismatch(other, "integer"),
        }
    }
}
impl FromSql for String {
    fn from_sql(v: &Value) -> Result<String> {
        match v {
            Value::Text(s) => Ok(s.clone()),
            other => mismatch(other, "text"),
        }
    }
}
impl FromSql for Vec<u8> {
    fn from_sql(v: &Value) -> Result<Vec<u8>> {
        match v {
            Value::Blob(b) => Ok(b.clone()),
            other => mismatch(other, "blob"),
        }
    }
}
impl FromSql for Value {
    fn from_sql(v: &Value) -> Result<Value> {
        Ok(v.clone())
    }
}
impl<T: FromSql> FromSql for Option<T> {
    fn from_sql(v: &Value) -> Result<Option<T>> {
        if v.is_null() {
            Ok(None)
        } else {
            T::from_sql(v).map(Some)
        }
    }
}

pub trait RowIndex {
    fn index(&self, row: &Row) -> Result<usize>;
}
impl RowIndex for usize {
    fn index(&self, row: &Row) -> Result<usize> {
        if *self < row.values.len() {
            Ok(*self)
        } else {
            Err(Error::new(
                ErrorKind::ColumnIndex,
                format!("column index {self} out of range"),
            ))
        }
    }
}
impl RowIndex for &str {
    fn index(&self, row: &Row) -> Result<usize> {
        row.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(self))
            .ok_or_else(|| Error::new(ErrorKind::ColumnName, format!("no column named {self}")))
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    values: Vec<Value>,
    columns: Rc<Vec<String>>,
}

impl Row {
    pub fn get<I: RowIndex, T: FromSql>(&self, idx: I) -> Result<T> {
        let i = idx.index(self)?;
        T::from_sql(&self.values[i])
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

pub trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>>;
}
impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind == ErrorKind::QueryReturnedNoRows => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn returned_nothing(out: &exec::Outcome) -> Result<()> {
    if out.result.as_ref().is_some_and(|r| !r.rows.is_empty()) {
        return Err(Error::new(
            ErrorKind::ExecuteReturnedResults,
            "execute returned results",
        ));
    }
    Ok(())
}

// ---- connection ----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionBehavior {
    Deferred,
    Immediate,
    Exclusive,
}

struct Txn {
    lock_held: bool,
}

struct Inner {
    db: Database,
    log: Option<LogFile>,
    functions: Functions,
    txn: Option<Txn>,
    busy: Duration,
    last_insert: i64,
    last_changes: usize,
    compact_after: u64,
    /// The log length at which the next compaction check runs: measuring the live size costs a
    /// pass over every row, so it happens each time the log doubles, not on every commit.
    next_check: u64,
}

pub struct Connection {
    inner: RefCell<Inner>,
}

fn is_write(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Select(_) => false,
        Stmt::Pragma { name, argument } => {
            let n = name.to_ascii_lowercase();
            argument.is_some()
                && matches!(n.as_str(), "user_version")
                && matches!(argument, Some(ast::Pragma::Set(_)))
        }
        Stmt::Begin(_) | Stmt::Commit | Stmt::Rollback => false,
        _ => true,
    }
}

fn apply_op(db: &mut Database, functions: &Functions, op: Op) -> Result<()> {
    match op {
        Op::Put { table, rowid, row } => {
            db.table_mut(&table)?.put(rowid, row);
        }
        Op::Del { table, rowid } => {
            db.table_mut(&table)?.remove(rowid);
        }
        Op::Ddl { sql } => {
            for parsed in parser::parse_script(&sql)? {
                Session { db, functions }.execute(&parsed, &[])?;
            }
            db.journal.clear();
            db.redo.clear();
        }
        Op::UserVersion(v) => db.user_version = v,
        Op::Sequence { table, value } => {
            db.table_mut(&table)?.sequence = value;
        }
    }
    Ok(())
}

impl Inner {
    fn refresh(&mut self) -> Result<()> {
        if let Some(log) = &mut self.log {
            let fk = self.db.foreign_keys;
            self.db.foreign_keys = false;
            let functions = &self.functions;
            let result = log.refresh(&mut self.db, &mut |db, op| apply_op(db, functions, op));
            self.db.foreign_keys = fk;
            result?;
        }
        Ok(())
    }

    fn with_lock<T>(
        &mut self,
        mode: LockMode,
        f: impl FnOnce(&mut Inner) -> Result<T>,
    ) -> Result<T> {
        if self.log.is_none() {
            return f(self);
        }
        self.log.as_ref().unwrap().lock(mode, self.busy)?;
        let r = self.refresh().and_then(|()| f(self));
        if let Some(log) = &self.log {
            log.unlock();
        }
        r
    }

    fn commit_ops(&mut self) -> Result<()> {
        let ops = std::mem::take(&mut self.db.redo);
        self.db.journal.clear();
        if let Some(log) = &mut self.log {
            if let Err(e) = log.append(&ops) {
                // The image no longer matches the file: reload on the next use.
                let fk = self.db.foreign_keys;
                self.db = Database::default();
                self.db.foreign_keys = fk;
                return Err(e);
            }
            self.maybe_compact()?;
        }
        Ok(())
    }

    fn snapshot(&self) -> Vec<Op> {
        let mut ops = Vec::new();
        for table in self.db.tables.values() {
            for sql in &table.ddl {
                ops.push(Op::Ddl { sql: sql.clone() });
            }
            for (rowid, row) in &table.rows {
                ops.push(Op::Put {
                    table: table.def.name.to_ascii_lowercase(),
                    rowid: *rowid,
                    row: row.clone(),
                });
            }
            if table.def.autoincrement {
                ops.push(Op::Sequence {
                    table: table.def.name.to_ascii_lowercase(),
                    value: table.sequence,
                });
            }
        }
        ops.push(Op::UserVersion(self.db.user_version));
        ops
    }

    fn maybe_compact(&mut self) -> Result<()> {
        let Some(log) = &self.log else {
            return Ok(());
        };
        let bytes = log.log_bytes();
        if bytes < self.compact_after.max(self.next_check) {
            return Ok(());
        }
        let snapshot = self.snapshot();
        let live = persist::encode_ops(&snapshot).len() as u64;
        let log = self.log.as_mut().unwrap();
        if bytes > 4 * live.max(self.compact_after.min(1 << 20)) {
            log.compact(&snapshot)?;
        }
        self.next_check = log.log_bytes().saturating_mul(2);
        Ok(())
    }

    /// Parameters bound by the caller must match the statement's placeholders exactly.
    fn run_bound(&mut self, parsed: &Parsed, params: &[Value]) -> Result<exec::Outcome> {
        if params.len() != parsed.params {
            return Err(Error::new(
                ErrorKind::InvalidParameterCount,
                format!(
                    "statement needs {} parameters, {} supplied",
                    parsed.params,
                    params.len()
                ),
            ));
        }
        self.run(parsed, params)
    }

    /// Runs a statement. As in SQLite, the count a call reports is that of the latest INSERT,
    /// UPDATE or DELETE that completed on the connection: BEGIN, COMMIT, ROLLBACK, DDL and SELECT
    /// report the stale count of an earlier statement.
    fn run(&mut self, parsed: &Parsed, params: &[Value]) -> Result<exec::Outcome> {
        let dml = matches!(
            parsed.stmt,
            Stmt::Insert(_) | Stmt::Update(_) | Stmt::Delete(_)
        );
        let mut out = match self.run_statement(parsed, params) {
            Ok(out) => out,
            Err(e) => {
                // A write that fails while running is rolled back as a whole: it changed nothing.
                // Errors found while compiling (unknown names, syntax) leave the count alone.
                if dml && matches!(e.kind, ErrorKind::Constraint | ErrorKind::Type) {
                    self.last_changes = 0;
                }
                return Err(e);
            }
        };
        if dml {
            self.last_changes = out.changes;
        } else {
            out.changes = self.last_changes;
        }
        Ok(out)
    }

    fn run_statement(&mut self, parsed: &Parsed, params: &[Value]) -> Result<exec::Outcome> {
        if params.len() < parsed.params {
            return Err(Error::new(
                ErrorKind::InvalidParameterCount,
                format!(
                    "statement needs {} parameters, {} supplied",
                    parsed.params,
                    params.len()
                ),
            ));
        }
        match &parsed.stmt {
            Stmt::Begin(kind) => {
                if self.txn.is_some() {
                    return Err(Error::misuse(
                        "cannot start a transaction within a transaction",
                    ));
                }
                let held = match kind {
                    BeginKind::Deferred => {
                        if self.log.is_some() {
                            self.with_lock(LockMode::Shared, |_| Ok(()))?;
                        }
                        false
                    }
                    _ => {
                        if let Some(log) = &self.log {
                            log.lock(LockMode::Exclusive, self.busy)?;
                            if let Err(e) = self.refresh() {
                                self.log.as_ref().unwrap().unlock();
                                return Err(e);
                            }
                        }
                        self.log.is_some()
                    }
                };
                self.txn = Some(Txn { lock_held: held });
                Ok(exec::Outcome::default())
            }
            Stmt::Commit => {
                let Some(txn) = self.txn.take() else {
                    return Err(Error::misuse("cannot commit - no transaction is active"));
                };
                let r = if self.db.redo.is_empty() {
                    Ok(())
                } else {
                    self.commit_ops()
                };
                if txn.lock_held
                    && let Some(log) = &self.log
                {
                    log.unlock();
                }
                r.map(|()| exec::Outcome::default())
            }
            Stmt::Rollback => {
                let Some(txn) = self.txn.take() else {
                    return Err(Error::misuse("cannot rollback - no transaction is active"));
                };
                self.db.rollback_to(0);
                if txn.lock_held
                    && let Some(log) = &self.log
                {
                    log.unlock();
                }
                Ok(exec::Outcome::default())
            }
            stmt => {
                let write = is_write(stmt);
                if self.txn.is_some() {
                    if write && self.log.is_some() && !self.txn.as_ref().unwrap().lock_held {
                        self.log
                            .as_ref()
                            .unwrap()
                            .lock(LockMode::Exclusive, self.busy)?;
                        if let Err(e) = self.refresh() {
                            self.log.as_ref().unwrap().unlock();
                            return Err(e);
                        }
                        self.txn.as_mut().unwrap().lock_held = true;
                    }
                    let functions = &self.functions;
                    let out = Session {
                        db: &mut self.db,
                        functions,
                    }
                    .execute(parsed, params)?;
                    if let Some(r) = out.last_insert_rowid {
                        self.last_insert = r;
                    }
                    return Ok(out);
                }
                // Autocommit.
                let mode = if write {
                    LockMode::Exclusive
                } else {
                    LockMode::Shared
                };
                let out = self.with_lock(mode, |me| {
                    let functions = &me.functions;
                    let out = Session {
                        db: &mut me.db,
                        functions,
                    }
                    .execute(parsed, params)?;
                    if write {
                        me.commit_ops()?;
                    }
                    Ok(out)
                })?;
                if let Some(r) = out.last_insert_rowid {
                    self.last_insert = r;
                }
                Ok(out)
            }
        }
    }
}

impl Connection {
    pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
        let log = LogFile::open(path.as_ref())?;
        let c = Connection::with(Some(log));
        {
            let mut inner = c.inner.borrow_mut();
            inner.with_lock(LockMode::Shared, |_| Ok(()))?;
        }
        Ok(c)
    }

    pub fn open_in_memory() -> Result<Connection> {
        Ok(Connection::with(None))
    }

    fn with(log: Option<LogFile>) -> Connection {
        let mut functions = Functions::new();
        functions.insert("json_extract".into(), Box::new(jsonlite::json_extract));
        functions.insert("date".into(), Box::new(datetime::date));
        functions.insert("time".into(), Box::new(datetime::time));
        functions.insert("datetime".into(), Box::new(datetime::datetime));
        functions.insert("julianday".into(), Box::new(datetime::julianday));
        functions.insert("unixepoch".into(), Box::new(datetime::unixepoch));
        functions.insert("strftime".into(), Box::new(datetime::strftime));
        Connection {
            inner: RefCell::new(Inner {
                db: Database::default(),
                log,
                functions,
                txn: None,
                busy: Duration::from_secs(5),
                last_insert: 0,
                last_changes: 0,
                compact_after: 8 << 20,
                next_check: 0,
            }),
        }
    }

    pub fn busy_timeout(&self, d: Duration) -> Result<()> {
        self.inner.borrow_mut().busy = d;
        Ok(())
    }

    /// Registers a scalar SQL function.
    pub fn create_scalar_function<F>(&self, name: &str, f: F)
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        self.inner
            .borrow_mut()
            .functions
            .insert(name.to_ascii_lowercase(), Box::new(f));
    }

    /// Rewrites the log as a snapshot once it has grown past this many bytes and holds four times
    /// the live data (default 8 MiB).
    pub fn set_compaction_threshold(&self, bytes: u64) {
        let mut inner = self.inner.borrow_mut();
        inner.compact_after = bytes;
        inner.next_check = 0;
    }

    pub fn last_insert_rowid(&self) -> i64 {
        self.inner.borrow().last_insert
    }

    pub fn is_autocommit(&self) -> bool {
        self.inner.borrow().txn.is_none()
    }

    fn run_one(&self, sql: &str, params: &[Value]) -> Result<exec::Outcome> {
        let parsed = parser::parse_script(sql)?;
        match parsed.as_slice() {
            [one] => self.inner.borrow_mut().run_bound(one, params),
            [] => Ok(exec::Outcome::default()),
            _ => Err(Error::new(
                ErrorKind::MultipleStatement,
                "multiple statements provided",
            )),
        }
    }

    pub fn execute<P: Params>(&self, sql: &str, params: P) -> Result<usize> {
        let out = self.run_one(sql, &params.bind()?)?;
        returned_nothing(&out)?;
        Ok(out.changes)
    }

    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        for p in parser::parse_script(sql)? {
            self.inner.borrow_mut().run(&p, &[])?;
        }
        Ok(())
    }

    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>> {
        let mut parsed = parser::parse_script(sql)?;
        if parsed.len() != 1 {
            return Err(Error::new(
                ErrorKind::MultipleStatement,
                "multiple statements provided",
            ));
        }
        Ok(Statement {
            conn: self,
            parsed: parsed.remove(0),
        })
    }

    pub fn query_row<T, P: Params, F: FnOnce(&Row) -> Result<T>>(
        &self,
        sql: &str,
        params: P,
        f: F,
    ) -> Result<T> {
        self.prepare(sql)?.query_row(params, f)
    }

    pub fn transaction(&mut self) -> Result<Transaction<'_>> {
        self.transaction_with_behavior(TransactionBehavior::Deferred)
    }

    pub fn transaction_with_behavior(
        &mut self,
        behavior: TransactionBehavior,
    ) -> Result<Transaction<'_>> {
        let kind = match behavior {
            TransactionBehavior::Deferred => BeginKind::Deferred,
            TransactionBehavior::Immediate => BeginKind::Immediate,
            TransactionBehavior::Exclusive => BeginKind::Exclusive,
        };
        let parsed = Parsed {
            stmt: Stmt::Begin(kind),
            params: 0,
            text: "BEGIN".into(),
        };
        self.inner.borrow_mut().run(&parsed, &[])?;
        Ok(Transaction {
            conn: self,
            done: false,
        })
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        if let Some(txn) = inner.txn.take() {
            inner.db.rollback_to(0);
            if txn.lock_held
                && let Some(log) = &inner.log
            {
                log.unlock();
            }
        }
    }
}

pub struct Transaction<'c> {
    conn: &'c Connection,
    done: bool,
}

impl std::ops::Deref for Transaction<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl Transaction<'_> {
    pub fn commit(mut self) -> Result<()> {
        self.done = true;
        let parsed = Parsed {
            stmt: Stmt::Commit,
            params: 0,
            text: "COMMIT".into(),
        };
        self.conn.inner.borrow_mut().run(&parsed, &[]).map(|_| ())
    }

    pub fn rollback(mut self) -> Result<()> {
        self.done = true;
        let parsed = Parsed {
            stmt: Stmt::Rollback,
            params: 0,
            text: "ROLLBACK".into(),
        };
        self.conn.inner.borrow_mut().run(&parsed, &[]).map(|_| ())
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.done {
            let parsed = Parsed {
                stmt: Stmt::Rollback,
                params: 0,
                text: "ROLLBACK".into(),
            };
            let _ = self.conn.inner.borrow_mut().run(&parsed, &[]);
        }
    }
}

pub struct Statement<'c> {
    conn: &'c Connection,
    parsed: Parsed,
}

pub struct Rows {
    rows: std::vec::IntoIter<Row>,
    current: Option<Row>,
}

impl Rows {
    // The donor's `Rows::next` is fallible and lends the row, so it cannot be an `Iterator`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<&Row>> {
        self.current = self.rows.next();
        Ok(self.current.as_ref())
    }
}

pub struct MappedRows<F> {
    rows: std::vec::IntoIter<Row>,
    f: F,
}

impl<T, F: FnMut(&Row) -> Result<T>> Iterator for MappedRows<F> {
    type Item = Result<T>;
    fn next(&mut self) -> Option<Result<T>> {
        let row = self.rows.next()?;
        Some((self.f)(&row))
    }
}

impl Statement<'_> {
    fn fetch<P: Params>(&self, params: P) -> Result<Vec<Row>> {
        let out = self
            .conn
            .inner
            .borrow_mut()
            .run_bound(&self.parsed, &params.bind()?)?;
        let result = out.result.unwrap_or_default();
        let columns = Rc::new(result.columns);
        Ok(result
            .rows
            .into_iter()
            .map(|values| Row {
                values,
                columns: columns.clone(),
            })
            .collect())
    }

    pub fn query<P: Params>(&mut self, params: P) -> Result<Rows> {
        Ok(Rows {
            rows: self.fetch(params)?.into_iter(),
            current: None,
        })
    }

    pub fn query_map<T, P: Params, F: FnMut(&Row) -> Result<T>>(
        &mut self,
        params: P,
        f: F,
    ) -> Result<MappedRows<F>> {
        Ok(MappedRows {
            rows: self.fetch(params)?.into_iter(),
            f,
        })
    }

    pub fn query_row<T, P: Params, F: FnOnce(&Row) -> Result<T>>(
        &mut self,
        params: P,
        f: F,
    ) -> Result<T> {
        let rows = self.fetch(params)?;
        match rows.first() {
            Some(r) => f(r),
            None => Err(Error::new(
                ErrorKind::QueryReturnedNoRows,
                "query returned no rows",
            )),
        }
    }

    pub fn exists<P: Params>(&mut self, params: P) -> Result<bool> {
        Ok(!self.fetch(params)?.is_empty())
    }

    pub fn execute<P: Params>(&mut self, params: P) -> Result<usize> {
        let out = self
            .conn
            .inner
            .borrow_mut()
            .run_bound(&self.parsed, &params.bind()?)?;
        returned_nothing(&out)?;
        Ok(out.changes)
    }
}
