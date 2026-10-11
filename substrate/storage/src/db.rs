//! The in-memory database: catalog, tables, unique and secondary indexes, and the undo journal
//! that makes a statement or a transaction atomic.

use crate::{
    ast::*,
    error::{Error, ErrorKind, Result},
    value::{Affinity, Value},
};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

/// A value usable as an ordered key.
#[derive(Clone, Debug)]
pub struct Key(pub Value);

impl PartialEq for Key {
    fn eq(&self, other: &Key) -> bool {
        self.0.compare(&other.0) == Ordering::Equal
    }
}
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Key) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Key) -> Ordering {
        self.0.compare(&other.0)
    }
}

pub type KeyTuple = Vec<Key>;

#[derive(Clone, Debug)]
pub struct Column {
    pub name: String,
    pub declared_type: String,
    pub affinity: Affinity,
    pub not_null: bool,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct ForeignKeyDef {
    pub columns: Vec<usize>,
    pub parent: String,
    pub parent_columns: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TableDef {
    pub name: String,
    pub columns: Vec<Column>,
    pub primary_key: Vec<usize>,
    /// The column that aliases the rowid (`INTEGER PRIMARY KEY`).
    pub rowid_alias: Option<usize>,
    pub autoincrement: bool,
    pub unique_sets: Vec<Vec<usize>>,
    pub checks: Vec<Expr>,
    pub foreign_keys: Vec<ForeignKeyDef>,
}

impl TableDef {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Index {
    pub name: String,
    pub columns: Vec<usize>,
    pub unique: bool,
    pub entries: BTreeMap<KeyTuple, BTreeSet<i64>>,
}

#[derive(Clone, Debug)]
pub struct Table {
    pub def: TableDef,
    pub rows: BTreeMap<i64, Vec<Value>>,
    /// Constraint indexes first (primary key, uniques), then declared indexes.
    pub indexes: Vec<Index>,
    pub next_rowid: i64,
    pub sequence: i64,
    /// The DDL statements that built this table, in order, for the snapshot.
    pub ddl: Vec<String>,
}

impl Table {
    pub fn new(def: TableDef, declared: Vec<(String, Vec<usize>, bool)>) -> Table {
        let mut indexes = Vec::new();
        if !def.primary_key.is_empty() && def.rowid_alias.is_none() {
            indexes.push(Index {
                name: "pk".into(),
                columns: def.primary_key.clone(),
                unique: true,
                entries: BTreeMap::new(),
            });
        }
        for set in &def.unique_sets {
            indexes.push(Index {
                name: "unique".into(),
                columns: set.clone(),
                unique: true,
                entries: BTreeMap::new(),
            });
        }
        for (name, columns, unique) in declared {
            indexes.push(Index {
                name,
                columns,
                unique,
                entries: BTreeMap::new(),
            });
        }
        Table {
            def,
            rows: BTreeMap::new(),
            indexes,
            next_rowid: 1,
            sequence: 0,
            ddl: Vec::new(),
        }
    }

    fn key_of(row: &[Value], columns: &[usize]) -> Option<KeyTuple> {
        let mut key = Vec::with_capacity(columns.len());
        for &c in columns {
            if row[c].is_null() {
                return None;
            }
            key.push(Key(row[c].clone()));
        }
        Some(key)
    }

    fn index_add(&mut self, rowid: i64, row: &[Value]) {
        for ix in &mut self.indexes {
            let key = if ix.unique {
                Self::key_of(row, &ix.columns)
            } else {
                Some(ix.columns.iter().map(|&c| Key(row[c].clone())).collect())
            };
            if let Some(key) = key {
                ix.entries.entry(key).or_default().insert(rowid);
            }
        }
    }

    fn index_remove(&mut self, rowid: i64, row: &[Value]) {
        for ix in &mut self.indexes {
            let key = if ix.unique {
                Self::key_of(row, &ix.columns)
            } else {
                Some(ix.columns.iter().map(|&c| Key(row[c].clone())).collect())
            };
            if let Some(key) = key
                && let Some(set) = ix.entries.get_mut(&key)
            {
                set.remove(&rowid);
                if set.is_empty() {
                    ix.entries.remove(&key);
                }
            }
        }
    }

    /// The rowid of a different row that holds the same value in some unique index.
    pub fn unique_conflict(&self, row: &[Value], except: Option<i64>) -> Option<(usize, i64)> {
        for (n, ix) in self.indexes.iter().enumerate() {
            if !ix.unique {
                continue;
            }
            if let Some(key) = Self::key_of(row, &ix.columns)
                && let Some(set) = ix.entries.get(&key)
                && let Some(&other) = set.iter().find(|&&r| Some(r) != except)
            {
                return Some((n, other));
            }
        }
        if let Some(alias) = self.def.rowid_alias
            && let Value::Integer(id) = row[alias]
            && Some(id) != except
            && self.rows.contains_key(&id)
        {
            return Some((usize::MAX, id));
        }
        None
    }

    pub fn put(&mut self, rowid: i64, row: Vec<Value>) {
        if let Some(old) = self.rows.remove(&rowid) {
            self.index_remove(rowid, &old);
        }
        self.index_add(rowid, &row);
        if rowid >= self.next_rowid {
            self.next_rowid = rowid + 1;
        }
        if self.def.autoincrement && rowid > self.sequence {
            self.sequence = rowid;
        }
        self.rows.insert(rowid, row);
    }

    pub fn remove(&mut self, rowid: i64) -> Option<Vec<Value>> {
        let old = self.rows.remove(&rowid)?;
        self.index_remove(rowid, &old);
        Some(old)
    }

    /// Rowids whose indexed columns equal `values` (a prefix of the index columns), in index order.
    pub fn lookup(&self, columns: &[usize], values: &[Value]) -> Option<Vec<i64>> {
        if let (Some(alias), [c], [Value::Integer(id)]) = (self.def.rowid_alias, columns, values)
            && *c == alias
        {
            return Some(if self.rows.contains_key(id) {
                vec![*id]
            } else {
                Vec::new()
            });
        }
        for ix in &self.indexes {
            if ix.columns.len() < columns.len() || ix.columns[..columns.len()] != *columns {
                continue;
            }
            let prefix: KeyTuple = values.iter().map(|v| Key(v.clone())).collect();
            let mut out = Vec::new();
            if ix.columns.len() == columns.len() {
                if let Some(set) = ix.entries.get(&prefix) {
                    out.extend(set.iter().copied());
                }
            } else {
                for (k, set) in ix.entries.range(prefix.clone()..) {
                    if k.len() < prefix.len() || k[..prefix.len()] != prefix[..] {
                        break;
                    }
                    out.extend(set.iter().copied());
                }
            }
            out.sort_unstable();
            return Some(out);
        }
        None
    }
}

#[derive(Clone, Debug)]
pub enum Change {
    Insert {
        table: String,
        rowid: i64,
        /// The AUTOINCREMENT counter before the row was added; a rollback restores it.
        sequence: i64,
    },
    Delete {
        table: String,
        rowid: i64,
        row: Vec<Value>,
    },
    Update {
        table: String,
        rowid: i64,
        old: Vec<Value>,
    },
    CreateTable {
        name: String,
    },
    DropTable {
        table: Box<Table>,
    },
    AddColumn {
        table: String,
        old_def: Box<TableDef>,
    },
    CreateIndex {
        table: String,
        name: String,
    },
    UserVersion {
        old: i64,
    },
}

#[derive(Debug, Default)]
pub struct Database {
    pub tables: BTreeMap<String, Table>,
    pub user_version: i64,
    pub foreign_keys: bool,
    pub journal: Vec<Change>,
    /// Redo records for the persistence layer: everything a commit must append.
    pub redo: Vec<crate::persist::Op>,
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

impl Database {
    pub fn table(&self, name: &str) -> Result<&Table> {
        self.tables
            .get(&lower(name))
            .ok_or_else(|| Error::new(ErrorKind::NoSuchTable, format!("no such table: {name}")))
    }

    pub fn table_mut(&mut self, name: &str) -> Result<&mut Table> {
        self.tables
            .get_mut(&lower(name))
            .ok_or_else(|| Error::new(ErrorKind::NoSuchTable, format!("no such table: {name}")))
    }

    pub fn mark(&self) -> usize {
        self.journal.len()
    }

    /// Records a change together with the redo operation that reproduces it.
    pub fn record(&mut self, change: Change, op: crate::persist::Op) {
        self.journal.push(change);
        self.redo.push(op);
    }

    /// Undoes changes back to `mark`.
    pub fn rollback_to(&mut self, mark: usize) {
        self.redo.truncate(mark);
        while self.journal.len() > mark {
            match self.journal.pop().unwrap() {
                Change::Insert {
                    table,
                    rowid,
                    sequence,
                } => {
                    if let Some(t) = self.tables.get_mut(&lower(&table)) {
                        t.remove(rowid);
                        t.sequence = sequence;
                    }
                }
                Change::Delete { table, rowid, row } => {
                    if let Some(t) = self.tables.get_mut(&lower(&table)) {
                        t.put(rowid, row);
                    }
                }
                Change::Update { table, rowid, old } => {
                    if let Some(t) = self.tables.get_mut(&lower(&table)) {
                        t.put(rowid, old);
                    }
                }
                Change::CreateTable { name } => {
                    self.tables.remove(&lower(&name));
                }
                Change::DropTable { table } => {
                    self.tables.insert(lower(&table.def.name), *table);
                }
                Change::AddColumn { table, old_def } => {
                    if let Some(t) = self.tables.get_mut(&lower(&table)) {
                        let old_len = old_def.columns.len();
                        t.def = *old_def;
                        for row in t.rows.values_mut() {
                            row.truncate(old_len);
                        }
                    }
                }
                Change::CreateIndex { table, name } => {
                    if let Some(t) = self.tables.get_mut(&lower(&table)) {
                        t.indexes.retain(|ix| ix.name != name);
                    }
                }
                Change::UserVersion { old } => self.user_version = old,
            }
        }
    }
}
