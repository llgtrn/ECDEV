//! Statement execution over the in-memory database.

use crate::{
    ast::*,
    db::*,
    error::{Error, ErrorKind, Result},
    eval::*,
    persist::Op,
    value::{Affinity, Value, coerce_for_comparison},
};
use std::{cmp::Ordering, collections::BTreeMap};

pub type ScalarFn = Box<dyn Fn(&[Value]) -> Result<Value> + Send + Sync>;
pub type Functions = std::collections::HashMap<String, ScalarFn>;

#[derive(Clone, Debug, Default)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

fn lc(s: &str) -> String {
    s.to_ascii_lowercase()
}

pub struct Executor<'a> {
    pub env: Env<'a>,
}

/// A table definition with the rowid and values of each matching row.
type Matched = (TableDef, Vec<(i64, Vec<Value>)>);

impl SubqueryRunner for Executor<'_> {
    fn run_select(&self, select: &Select, outer: Option<&Scope>) -> Result<QueryResult> {
        self.select(select, outer)
    }
}

struct Candidate {
    source: Source,
    rows: Vec<Vec<Value>>,
}

fn table_source(t: &Table, alias: Option<&str>) -> Source {
    let mut columns: Vec<(String, Option<Affinity>)> = t
        .def
        .columns
        .iter()
        .map(|c| (lc(&c.name), Some(c.affinity)))
        .collect();
    if !columns.iter().any(|(n, _)| n == "rowid") {
        columns.push(("rowid".into(), Some(Affinity::Integer)));
    }
    Source {
        alias: lc(alias.unwrap_or(&t.def.name)),
        columns,
    }
}

/// Rows of a table as joined-row values: the columns plus the hidden rowid.
fn table_rows(t: &Table, rowids: Option<&[i64]>) -> Vec<Vec<Value>> {
    let has_rowid_col = t
        .def
        .columns
        .iter()
        .any(|c| c.name.eq_ignore_ascii_case("rowid"));
    let make = |id: i64, r: &Vec<Value>| {
        let mut row = r.clone();
        if !has_rowid_col {
            row.push(Value::Integer(id));
        }
        row
    };
    match rowids {
        Some(ids) => ids
            .iter()
            .filter_map(|id| t.rows.get(id).map(|r| make(*id, r)))
            .collect(),
        None => t.rows.iter().map(|(id, r)| make(*id, r)).collect(),
    }
}

fn visible_columns(src: &Source, t: Option<&Table>) -> usize {
    match t {
        Some(t) => t.def.columns.len(),
        None => src.columns.len(),
    }
}

/// AND-ed terms of a condition.
fn conjuncts<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
    if let Expr::Binary {
        op: BinaryOp::And,
        left,
        right,
    } = e
    {
        conjuncts(left, out);
        conjuncts(right, out);
    } else {
        out.push(e);
    }
}

/// Whether `e` can be evaluated without any row of the query's own sources.
fn row_independent(e: &Expr) -> bool {
    match e {
        Expr::Literal(_) | Expr::Param(_) => true,
        Expr::Unary { operand, .. } => row_independent(operand),
        Expr::Binary { left, right, .. } => row_independent(left) && row_independent(right),
        _ => false,
    }
}

impl Executor<'_> {
    // ---- SELECT --------------------------------------------------------------------------------

    pub fn select(&self, sel: &Select, outer: Option<&Scope>) -> Result<QueryResult> {
        let mut result = self.core(sel, outer)?;
        for (op, other) in &sel.compound {
            let right = self.core(other, outer)?;
            if right.columns.len() != result.columns.len() {
                return Err(Error::syntax(
                    "SELECTs to the left and right of a compound operator do not have the same number of result columns",
                ));
            }
            result.rows = combine(*op, std::mem::take(&mut result.rows), right.rows);
        }
        if !sel.compound.is_empty() {
            self.finish_compound(sel, &mut result)?;
        }
        Ok(result)
    }

    fn finish_compound(&self, sel: &Select, result: &mut QueryResult) -> Result<()> {
        if !sel.order_by.is_empty() {
            let mut keyed: Vec<(Vec<Value>, Vec<Value>)> = Vec::new();
            for row in std::mem::take(&mut result.rows) {
                let mut keys = Vec::new();
                for t in &sel.order_by {
                    keys.push(self.compound_key(&t.expr, &result.columns, &row)?);
                }
                keyed.push((keys, row));
            }
            keyed.sort_by(|a, b| compare_keys(&a.0, &b.0, &sel.order_by));
            result.rows = keyed.into_iter().map(|(_, r)| r).collect();
        }
        self.apply_limit(sel, result)
    }

    fn compound_key(&self, e: &Expr, columns: &[String], row: &[Value]) -> Result<Value> {
        match e {
            Expr::Literal(Value::Integer(n)) if *n >= 1 && (*n as usize) <= row.len() => {
                Ok(row[*n as usize - 1].clone())
            }
            Expr::Column { table: None, name } => columns
                .iter()
                .position(|c| c.eq_ignore_ascii_case(name))
                .map(|i| row[i].clone())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::NoSuchColumn,
                        format!(
                            "ORDER BY term does not match any column in the result set: {name}"
                        ),
                    )
                }),
            _ => Err(Error::syntax(
                "unsupported ORDER BY term in a compound SELECT",
            )),
        }
    }

    fn apply_limit(&self, sel: &Select, result: &mut QueryResult) -> Result<()> {
        let eval_int = |e: &Expr| -> Result<i64> {
            let v = self.env.eval(e, None, None, self)?;
            match v.to_numeric() {
                Some(crate::value::Numeric::Int(i)) => Ok(i),
                Some(crate::value::Numeric::Real(f)) => Ok(f as i64),
                None => Err(Error::new(ErrorKind::Type, "datatype mismatch")),
            }
        };
        let offset = match &sel.offset {
            Some(e) => eval_int(e)?.max(0) as usize,
            None => 0,
        };
        let limit = match &sel.limit {
            Some(e) => {
                let n = eval_int(e)?;
                if n < 0 { usize::MAX } else { n as usize }
            }
            None => usize::MAX,
        };
        if offset > 0 {
            result.rows.drain(..offset.min(result.rows.len()));
        }
        result.rows.truncate(limit);
        Ok(())
    }

    fn build_source(
        &self,
        tr: &TableRef,
        outer: Option<&Scope>,
    ) -> Result<(Candidate, Option<String>)> {
        match tr {
            TableRef::Table { name, alias } => {
                let t = self.env.db.table(name)?;
                Ok((
                    Candidate {
                        source: table_source(t, alias.as_deref()),
                        rows: Vec::new(),
                    },
                    Some(name.clone()),
                ))
            }
            TableRef::Subquery { select, alias } => {
                let r = self.select(select, outer)?;
                let source = Source {
                    alias: lc(alias.as_deref().unwrap_or("")),
                    columns: r.columns.iter().map(|c| (lc(c), None)).collect(),
                };
                Ok((
                    Candidate {
                        source,
                        rows: r.rows,
                    },
                    None,
                ))
            }
        }
    }

    /// Equality constraints on columns of `source` from the AND-ed terms of `conds`, each with
    /// the expression it must equal.
    fn equalities<'e>(
        &self,
        conds: &[&'e Expr],
        source: &Source,
        single: bool,
    ) -> Vec<(usize, &'e Expr)> {
        let mut out = Vec::new();
        for c in conds {
            if let Expr::Binary {
                op: BinaryOp::Eq,
                left,
                right,
            } = c
            {
                for (col, other) in [(left, right), (right, left)] {
                    if let Expr::Column { table, name } = &**col {
                        let ok = match table {
                            Some(t) => t.eq_ignore_ascii_case(&source.alias),
                            None => single,
                        };
                        if !ok {
                            continue;
                        }
                        let n = lc(name);
                        if let Some(ci) = source.columns.iter().position(|(cn, _)| *cn == n) {
                            // the other side must not mention this source
                            if !mentions(other, &source.alias, single) {
                                out.push((ci, &**other));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    fn rowids_by_index(&self, t: &Table, eqs: &[(usize, Value)]) -> Option<Vec<i64>> {
        if eqs.is_empty() {
            return None;
        }
        let rowid_col = t.def.columns.len();
        if let Some((_, Value::Integer(id))) = eqs
            .iter()
            .find(|(c, _)| *c == rowid_col || Some(*c) == t.def.rowid_alias)
        {
            return Some(if t.rows.contains_key(id) {
                vec![*id]
            } else {
                Vec::new()
            });
        }
        let map: BTreeMap<usize, Value> = eqs
            .iter()
            .filter(|(c, _)| *c < t.def.columns.len())
            .cloned()
            .collect();
        let mut best: Option<(usize, Vec<usize>)> = None;
        for ix in &t.indexes {
            let mut prefix = 0;
            for c in &ix.columns {
                if map.contains_key(c) {
                    prefix += 1;
                } else {
                    break;
                }
            }
            if prefix > 0 && best.as_ref().is_none_or(|(p, _)| prefix > *p) {
                best = Some((prefix, ix.columns[..prefix].to_vec()));
            }
        }
        let (_, cols) = best?;
        let vals: Vec<Value> = cols
            .iter()
            .map(|c| map[c].clone().apply_affinity(t.def.columns[*c].affinity))
            .collect();
        t.lookup(&cols, &vals)
    }

    fn core(&self, sel: &Select, outer: Option<&Scope>) -> Result<QueryResult> {
        // 1. sources and rows
        let mut sources: Vec<Source> = Vec::new();
        let mut visible: Vec<usize> = Vec::new();
        let mut rows: Vec<JoinedRow>;
        let where_terms: Vec<&Expr> = {
            let mut v = Vec::new();
            if let Some(w) = &sel.where_ {
                conjuncts(w, &mut v);
            }
            v
        };
        match &sel.from {
            None => rows = vec![Vec::new()],
            Some(from) => {
                let single = from.joins.is_empty();
                let (mut first, table_name) = self.build_source(&from.first, outer)?;
                if let Some(name) = &table_name {
                    let t = self.env.db.table(name)?;
                    let eqs =
                        self.constant_equalities(&where_terms, &first.source, single, outer)?;
                    let ids = self.rowids_by_index(t, &eqs);
                    first.rows = table_rows(t, ids.as_deref());
                    visible.push(visible_columns(&first.source, Some(t)));
                } else {
                    visible.push(visible_columns(&first.source, None));
                }
                sources.push(first.source.clone());
                rows = first.rows.into_iter().map(|r| vec![Some(r)]).collect();
                for join in &from.joins {
                    let (mut cand, table_name) = self.build_source(&join.table, outer)?;
                    let mut on_terms = Vec::new();
                    if let Some(on) = &join.on {
                        conjuncts(on, &mut on_terms);
                    }
                    let right_table = table_name
                        .as_ref()
                        .map(|n| self.env.db.table(n))
                        .transpose()?;
                    if let Some(t) = right_table {
                        if join.kind == JoinKind::Cross {
                            cand.rows = table_rows(t, None);
                        }
                        visible.push(visible_columns(&cand.source, Some(t)));
                    } else {
                        visible.push(visible_columns(&cand.source, None));
                    }
                    let eq_exprs = if right_table.is_some() {
                        self.equalities(&on_terms, &cand.source, false)
                    } else {
                        Vec::new()
                    };
                    let mut all_sources = sources.clone();
                    all_sources.push(cand.source.clone());
                    let mut joined: Vec<JoinedRow> = Vec::new();
                    for left in &rows {
                        let candidates: Vec<Vec<Value>> = match right_table {
                            Some(t) => {
                                let mut eqs = Vec::new();
                                if !eq_exprs.is_empty() {
                                    let lscope = Scope {
                                        sources: &sources,
                                        row: left,
                                        parent: outer,
                                    };
                                    for (ci, e) in &eq_exprs {
                                        if mentions_unknown(e, &sources, &cand.source) {
                                            continue;
                                        }
                                        let v = self.env.eval(e, Some(&lscope), None, self)?;
                                        if !v.is_null() {
                                            eqs.push((*ci, v));
                                        }
                                    }
                                }
                                let ids = self.rowids_by_index(t, &eqs);
                                table_rows(t, ids.as_deref())
                            }
                            None => cand.rows.clone(),
                        };
                        let mut matched = false;
                        for r in candidates {
                            let mut combined = left.clone();
                            combined.push(Some(r));
                            let keep = match &join.on {
                                Some(on) => {
                                    let sc = Scope {
                                        sources: &all_sources,
                                        row: &combined,
                                        parent: outer,
                                    };
                                    self.env.eval(on, Some(&sc), None, self)?.truth() == Some(true)
                                }
                                None => true,
                            };
                            if keep {
                                matched = true;
                                joined.push(combined);
                            }
                        }
                        if !matched && join.kind == JoinKind::Left {
                            let mut combined = left.clone();
                            combined.push(None);
                            joined.push(combined);
                        }
                    }
                    sources = all_sources;
                    rows = joined;
                }
            }
        }
        // 2. WHERE
        if let Some(w) = &sel.where_ {
            let mut kept = Vec::with_capacity(rows.len());
            for row in rows {
                let sc = Scope {
                    sources: &sources,
                    row: &row,
                    parent: outer,
                };
                if self.env.eval(w, Some(&sc), None, self)?.truth() == Some(true) {
                    kept.push(row);
                }
            }
            rows = kept;
        }
        // 3. result columns
        let mut out_cols: Vec<String> = Vec::new();
        let mut out_exprs: Vec<Option<&Expr>> = Vec::new();
        let mut star_slots: Vec<(usize, usize)> = Vec::new();
        let mut aliases: Vec<Option<String>> = Vec::new();
        // plan: Vec of either Expr index or (source, col)
        enum Item<'e> {
            Expr(&'e Expr),
            Slot(usize, usize),
        }
        let mut items: Vec<Item> = Vec::new();
        for rc in &sel.columns {
            match rc {
                ResultColumn::Star => {
                    for (si, s) in sources.iter().enumerate() {
                        for ci in 0..visible[si] {
                            out_cols.push(s.columns[ci].0.clone());
                            items.push(Item::Slot(si, ci));
                            aliases.push(None);
                        }
                    }
                }
                ResultColumn::TableStar(t) => {
                    let si = sources
                        .iter()
                        .position(|s| s.alias.eq_ignore_ascii_case(t))
                        .ok_or_else(|| {
                            Error::new(ErrorKind::NoSuchTable, format!("no such table: {t}"))
                        })?;
                    for ci in 0..visible[si] {
                        out_cols.push(sources[si].columns[ci].0.clone());
                        items.push(Item::Slot(si, ci));
                        aliases.push(None);
                    }
                }
                ResultColumn::Expr { expr, alias } => {
                    out_cols.push(alias.clone().unwrap_or_else(|| match expr {
                        Expr::Column { name, .. } => name.clone(),
                        _ => "expr".to_string(),
                    }));
                    aliases.push(alias.clone());
                    items.push(Item::Expr(expr));
                }
            }
        }
        let _ = (&mut out_exprs, &mut star_slots);
        let grouped = !sel.group_by.is_empty()
            || sel.having.is_some()
            || items
                .iter()
                .any(|i| matches!(i, Item::Expr(e) if contains_aggregate(e)))
            || sel.order_by.iter().any(|t| contains_aggregate(&t.expr));
        // 4. rows -> (output row, sort keys)
        let mut produced: Vec<(Vec<Value>, Vec<Value>)> = Vec::new();
        let order_terms = &sel.order_by;
        let project = |row: &JoinedRow,
                       group: Option<&Group>|
         -> Result<Option<(Vec<Value>, Vec<Value>)>> {
            let sc = Scope {
                sources: &sources,
                row,
                parent: outer,
            };
            if let Some(h) = &sel.having
                && self.env.eval(h, Some(&sc), group, self)?.truth() != Some(true)
            {
                return Ok(None);
            }
            let mut out = Vec::with_capacity(items.len());
            for it in &items {
                out.push(match it {
                    Item::Slot(si, ci) => match &row[*si] {
                        Some(r) => r[*ci].clone(),
                        None => Value::Null,
                    },
                    Item::Expr(e) => self.env.eval(e, Some(&sc), group, self)?,
                });
            }
            let mut keys = Vec::with_capacity(order_terms.len());
            for t in order_terms {
                let by_alias = match &t.expr {
                    Expr::Literal(Value::Integer(n)) if *n >= 1 && (*n as usize) <= out.len() => {
                        Some(*n as usize - 1)
                    }
                    Expr::Column { table: None, name } => aliases
                        .iter()
                        .position(|a| a.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(name))),
                    _ => None,
                };
                keys.push(match by_alias {
                    Some(i) => out[i].clone(),
                    None => self.env.eval(&t.expr, Some(&sc), group, self)?,
                });
            }
            Ok(Some((out, keys)))
        };
        if grouped {
            let mut groups: BTreeMap<KeyTuple, Vec<JoinedRow>> = BTreeMap::new();
            if sel.group_by.is_empty() {
                groups.insert(Vec::new(), rows);
            } else {
                for row in rows {
                    let sc = Scope {
                        sources: &sources,
                        row: &row,
                        parent: outer,
                    };
                    let mut key = Vec::new();
                    for g in &sel.group_by {
                        key.push(Key(self.env.eval(g, Some(&sc), None, self)?));
                    }
                    groups.entry(key).or_default().push(row);
                }
            }
            for (_, grows) in groups {
                let first: JoinedRow = grows
                    .first()
                    .cloned()
                    .unwrap_or_else(|| sources.iter().map(|_| None).collect());
                let g = Group { rows: &grows };
                if let Some(p) = project(&first, Some(&g))? {
                    produced.push(p);
                }
            }
        } else {
            for row in &rows {
                if let Some(p) = project(row, None)? {
                    produced.push(p);
                }
            }
        }
        // 5. DISTINCT, ORDER BY, LIMIT
        if sel.distinct {
            let mut seen: Vec<Vec<Value>> = Vec::new();
            produced.retain(|(row, _)| {
                if seen.iter().any(|s| {
                    s.len() == row.len()
                        && s.iter()
                            .zip(row)
                            .all(|(a, b)| a.compare(b) == Ordering::Equal)
                }) {
                    false
                } else {
                    seen.push(row.clone());
                    true
                }
            });
        }
        if !sel.order_by.is_empty() && sel.compound.is_empty() {
            produced.sort_by(|a, b| compare_keys(&a.1, &b.1, &sel.order_by));
        }
        let mut result = QueryResult {
            columns: out_cols,
            rows: produced.into_iter().map(|(r, _)| r).collect(),
        };
        if sel.compound.is_empty() {
            self.apply_limit(sel, &mut result)?;
        }
        Ok(result)
    }

    /// Equalities of the table's columns to expressions that need no row: literals and
    /// parameters, evaluated now.
    fn constant_equalities(
        &self,
        conds: &[&Expr],
        source: &Source,
        single: bool,
        outer: Option<&Scope>,
    ) -> Result<Vec<(usize, Value)>> {
        let mut out = Vec::new();
        for (ci, e) in self.equalities(conds, source, single) {
            if row_independent(e) {
                let v = self.env.eval(e, outer, None, self)?;
                if !v.is_null() {
                    out.push((ci, v));
                }
            }
        }
        Ok(out)
    }
}

fn mentions(e: &Expr, alias: &str, single: bool) -> bool {
    match e {
        Expr::Column { table, .. } => match table {
            Some(t) => t.eq_ignore_ascii_case(alias),
            None => single,
        },
        Expr::Unary { operand, .. } => mentions(operand, alias, single),
        Expr::Binary { left, right, .. } => {
            mentions(left, alias, single) || mentions(right, alias, single)
        }
        Expr::Function { args, .. } => args.iter().any(|a| mentions(a, alias, single)),
        Expr::Subquery(_) | Expr::Exists(_) | Expr::InSelect { .. } => true,
        _ => false,
    }
}

/// Whether `e` refers to a column that is not among `known` (it may live in the join's own source).
fn mentions_unknown(e: &Expr, known: &[Source], own: &Source) -> bool {
    match e {
        Expr::Column { table, name } => match table {
            Some(t) => {
                !known.iter().any(|s| s.alias.eq_ignore_ascii_case(t))
                    || t.eq_ignore_ascii_case(&own.alias)
            }
            None => {
                !known
                    .iter()
                    .any(|s| s.columns.iter().any(|(c, _)| *c == lc(name)))
                    && own.columns.iter().any(|(c, _)| *c == lc(name))
            }
        },
        Expr::Unary { operand, .. } => mentions_unknown(operand, known, own),
        Expr::Binary { left, right, .. } => {
            mentions_unknown(left, known, own) || mentions_unknown(right, known, own)
        }
        Expr::Function { args, .. } => args.iter().any(|a| mentions_unknown(a, known, own)),
        Expr::Subquery(_) | Expr::Exists(_) | Expr::InSelect { .. } => true,
        _ => false,
    }
}

fn compare_keys(a: &[Value], b: &[Value], terms: &[OrderTerm]) -> Ordering {
    for (i, t) in terms.iter().enumerate() {
        let (x, y) = (&a[i.min(a.len() - 1)], &b[i.min(b.len() - 1)]);
        let o = x.compare(y);
        let o = if t.descending { o.reverse() } else { o };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

fn combine(op: Compound, left: Vec<Vec<Value>>, right: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    let eq = |a: &Vec<Value>, b: &Vec<Value>| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| x.compare(y) == Ordering::Equal)
    };
    let dedup = |rows: Vec<Vec<Value>>| {
        let mut out: Vec<Vec<Value>> = Vec::new();
        for r in rows {
            if !out.iter().any(|o| eq(o, &r)) {
                out.push(r);
            }
        }
        out
    };
    match op {
        Compound::UnionAll => left.into_iter().chain(right).collect(),
        Compound::Union => dedup(left.into_iter().chain(right).collect()),
        Compound::Intersect => dedup(left)
            .into_iter()
            .filter(|l| right.iter().any(|r| eq(l, r)))
            .collect(),
        Compound::Except => dedup(left)
            .into_iter()
            .filter(|l| !right.iter().any(|r| eq(l, r)))
            .collect(),
    }
}

// ---- statement execution (mutating) ---------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Outcome {
    pub result: Option<QueryResult>,
    pub changes: usize,
    pub last_insert_rowid: Option<i64>,
}

pub struct Session<'a> {
    pub db: &'a mut Database,
    pub functions: &'a Functions,
}

impl Session<'_> {
    fn executor<'x>(
        db: &'x Database,
        functions: &'x Functions,
        params: &'x [Value],
    ) -> Executor<'x> {
        Executor {
            env: Env {
                db,
                params,
                functions,
                excluded: None,
            },
        }
    }

    pub fn execute(&mut self, parsed: &crate::parser::Parsed, params: &[Value]) -> Result<Outcome> {
        let mark = self.db.mark();
        let r = self.execute_inner(parsed, params);
        if r.is_err() {
            self.db.rollback_to(mark);
        }
        r
    }

    fn execute_inner(
        &mut self,
        parsed: &crate::parser::Parsed,
        params: &[Value],
    ) -> Result<Outcome> {
        match &parsed.stmt {
            Stmt::Select(sel) => {
                let ex = Self::executor(self.db, self.functions, params);
                Ok(Outcome {
                    result: Some(ex.select(sel, None)?),
                    changes: 0,
                    last_insert_rowid: None,
                })
            }
            Stmt::Insert(ins) => self.insert(ins, params),
            Stmt::Update(upd) => self.update(upd, params),
            Stmt::Delete(del) => self.delete(del, params),
            Stmt::CreateTable(ct) => self.create_table(ct, &parsed.text),
            Stmt::CreateIndex(ci) => self.create_index(ci, &parsed.text),
            Stmt::DropTable { name, if_exists } => {
                let key = lc(name);
                match self.db.tables.remove(&key) {
                    Some(table) => {
                        self.db.record(
                            Change::DropTable {
                                table: Box::new(table),
                            },
                            Op::Ddl {
                                sql: parsed.text.clone(),
                            },
                        );
                    }
                    None if *if_exists => {}
                    None => {
                        return Err(Error::new(
                            ErrorKind::NoSuchTable,
                            format!("no such table: {name}"),
                        ));
                    }
                }
                Ok(Outcome::default())
            }
            Stmt::AlterAddColumn { table, column } => self.add_column(table, column, &parsed.text),
            Stmt::Pragma { name, argument } => self.pragma(name, argument.as_ref()),
            Stmt::Begin(_) | Stmt::Commit | Stmt::Rollback => Err(Error::misuse(
                "transaction control is handled by the connection",
            )),
        }
    }

    // ---- DDL -----------------------------------------------------------------------------------

    fn create_table(&mut self, ct: &CreateTable, text: &str) -> Result<Outcome> {
        let key = lc(&ct.name);
        if self.db.tables.contains_key(&key) {
            if ct.if_not_exists {
                return Ok(Outcome::default());
            }
            return Err(Error::syntax(format!("table {} already exists", ct.name)));
        }
        let mut columns = Vec::new();
        for c in &ct.columns {
            columns.push(Column {
                name: c.name.clone(),
                declared_type: c.declared_type.clone(),
                affinity: Affinity::of_declared(&c.declared_type),
                not_null: c.not_null,
                default: c.default.clone(),
            });
        }
        let idx = |name: &str, columns: &[Column]| {
            columns
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    Error::new(ErrorKind::NoSuchColumn, format!("no such column: {name}"))
                })
        };
        let mut primary_key: Vec<usize> = ct
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.primary_key)
            .map(|(i, _)| i)
            .collect();
        let mut unique_sets: Vec<Vec<usize>> = ct
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.unique)
            .map(|(i, _)| vec![i])
            .collect();
        let mut checks: Vec<Expr> = ct.columns.iter().filter_map(|c| c.check.clone()).collect();
        let mut foreign_keys = Vec::new();
        for (i, c) in ct.columns.iter().enumerate() {
            if let Some(r) = &c.references {
                foreign_keys.push(ForeignKeyDef {
                    columns: vec![i],
                    parent: r.table.clone(),
                    parent_columns: r.columns.clone(),
                });
            }
        }
        for tc in &ct.constraints {
            match tc {
                TableConstraint::PrimaryKey(names) => {
                    primary_key = names
                        .iter()
                        .map(|n| idx(n, &columns))
                        .collect::<Result<_>>()?;
                }
                TableConstraint::Unique(names) => unique_sets.push(
                    names
                        .iter()
                        .map(|n| idx(n, &columns))
                        .collect::<Result<_>>()?,
                ),
                TableConstraint::Check(e) => checks.push(e.clone()),
                TableConstraint::ForeignKey {
                    columns: cols,
                    target,
                } => {
                    foreign_keys.push(ForeignKeyDef {
                        columns: cols
                            .iter()
                            .map(|n| idx(n, &columns))
                            .collect::<Result<_>>()?,
                        parent: target.table.clone(),
                        parent_columns: target.columns.clone(),
                    });
                }
            }
        }
        let rowid_alias = if primary_key.len() == 1
            && columns[primary_key[0]]
                .declared_type
                .eq_ignore_ascii_case("integer")
        {
            Some(primary_key[0])
        } else {
            None
        };
        let autoincrement = ct.columns.iter().any(|c| c.autoincrement);
        let def = TableDef {
            name: ct.name.clone(),
            columns,
            primary_key,
            rowid_alias,
            autoincrement,
            unique_sets,
            checks,
            foreign_keys,
        };
        let mut table = Table::new(def, Vec::new());
        table.ddl.push(text.to_string());
        self.db.tables.insert(key, table);
        self.db.record(
            Change::CreateTable {
                name: ct.name.clone(),
            },
            Op::Ddl {
                sql: text.to_string(),
            },
        );
        Ok(Outcome::default())
    }

    fn create_index(&mut self, ci: &CreateIndex, text: &str) -> Result<Outcome> {
        let t = self.db.table_mut(&ci.table)?;
        if t.indexes
            .iter()
            .any(|ix| ix.name.eq_ignore_ascii_case(&ci.name))
        {
            if ci.if_not_exists {
                return Ok(Outcome::default());
            }
            return Err(Error::syntax(format!("index {} already exists", ci.name)));
        }
        let columns: Vec<usize> = ci
            .columns
            .iter()
            .map(|n| {
                t.def.column_index(n).ok_or_else(|| {
                    Error::new(ErrorKind::NoSuchColumn, format!("no such column: {n}"))
                })
            })
            .collect::<Result<_>>()?;
        let mut ix = Index {
            name: ci.name.clone(),
            columns,
            unique: ci.unique,
            entries: BTreeMap::new(),
        };
        let rows: Vec<(i64, Vec<Value>)> = t.rows.iter().map(|(k, v)| (*k, v.clone())).collect();
        for (rowid, row) in &rows {
            let key: Option<KeyTuple> = if ix.unique {
                ix.columns
                    .iter()
                    .map(|&c| {
                        if row[c].is_null() {
                            None
                        } else {
                            Some(Key(row[c].clone()))
                        }
                    })
                    .collect()
            } else {
                Some(ix.columns.iter().map(|&c| Key(row[c].clone())).collect())
            };
            if let Some(key) = key {
                let set = ix.entries.entry(key).or_default();
                if ix.unique && !set.is_empty() {
                    return Err(Error::constraint(format!(
                        "UNIQUE constraint failed: {}.{}",
                        t.def.name,
                        ci.columns.join(", ")
                    )));
                }
                set.insert(*rowid);
            }
        }
        t.indexes.push(ix);
        t.ddl.push(text.to_string());
        self.db.record(
            Change::CreateIndex {
                table: ci.table.clone(),
                name: ci.name.clone(),
            },
            Op::Ddl {
                sql: text.to_string(),
            },
        );
        Ok(Outcome::default())
    }

    fn add_column(&mut self, table: &str, column: &ColumnDef, text: &str) -> Result<Outcome> {
        let functions = self.functions;
        let default_value = match &column.default {
            Some(e) => {
                let ex = Self::executor(self.db, functions, &[]);
                ex.env.eval(e, None, None, &ex)?
            }
            None => Value::Null,
        };
        if column.not_null && default_value.is_null() {
            return Err(Error::syntax(
                "Cannot add a NOT NULL column with default value NULL",
            ));
        }
        if column.primary_key || column.unique {
            return Err(Error::syntax("Cannot add a PRIMARY KEY or UNIQUE column"));
        }
        let t = self.db.table_mut(table)?;
        let old_def = Box::new(t.def.clone());
        let affinity = Affinity::of_declared(&column.declared_type);
        t.def.columns.push(Column {
            name: column.name.clone(),
            declared_type: column.declared_type.clone(),
            affinity,
            not_null: column.not_null,
            default: column.default.clone(),
        });
        let v = default_value.apply_affinity(affinity);
        for row in t.rows.values_mut() {
            row.push(v.clone());
        }
        t.ddl.push(text.to_string());
        self.db.record(
            Change::AddColumn {
                table: table.to_string(),
                old_def,
            },
            Op::Ddl {
                sql: text.to_string(),
            },
        );
        Ok(Outcome::default())
    }

    fn pragma(&mut self, name: &str, argument: Option<&Pragma>) -> Result<Outcome> {
        let lname = lc(name);
        let text_result = |col: &str, v: Value| Outcome {
            result: Some(QueryResult {
                columns: vec![col.into()],
                rows: vec![vec![v]],
            }),
            changes: 0,
            last_insert_rowid: None,
        };
        match lname.as_str() {
            "journal_mode" => Ok(text_result("journal_mode", Value::Text("wal".into()))),
            "synchronous" | "busy_timeout" | "cache_size" | "temp_store" | "wal_autocheckpoint"
            | "locking_mode" => Ok(Outcome::default()),
            "foreign_keys" => match argument {
                Some(Pragma::Set(v)) => {
                    self.db.foreign_keys = matches!(v, Value::Integer(1))
                        || matches!(v, Value::Text(s) if matches!(lc(s).as_str(), "on" | "true" | "yes" | "1"));
                    Ok(Outcome::default())
                }
                _ => Ok(text_result(
                    "foreign_keys",
                    Value::Integer(i64::from(self.db.foreign_keys)),
                )),
            },
            "user_version" => match argument {
                Some(Pragma::Set(Value::Integer(v))) => {
                    let old = self.db.user_version;
                    self.db.user_version = *v;
                    self.db
                        .record(Change::UserVersion { old }, Op::UserVersion(*v));
                    Ok(Outcome::default())
                }
                _ => Ok(text_result(
                    "user_version",
                    Value::Integer(self.db.user_version),
                )),
            },
            "table_info" => {
                let Some(Pragma::Call(Value::Text(tn))) = argument else {
                    return Err(Error::syntax("table_info needs a table name"));
                };
                let t = self.db.table(tn)?;
                let rows = t
                    .def
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        vec![
                            Value::Integer(i as i64),
                            Value::Text(c.name.clone()),
                            Value::Text(c.declared_type.clone()),
                            Value::Integer(i64::from(c.not_null)),
                            Value::Null,
                            Value::Integer(
                                t.def
                                    .primary_key
                                    .iter()
                                    .position(|&p| p == i)
                                    .map_or(0, |p| p as i64 + 1),
                            ),
                        ]
                    })
                    .collect();
                Ok(Outcome {
                    result: Some(QueryResult {
                        columns: ["cid", "name", "type", "notnull", "dflt_value", "pk"]
                            .map(String::from)
                            .to_vec(),
                        rows,
                    }),
                    changes: 0,
                    last_insert_rowid: None,
                })
            }
            other => Err(Error::syntax(format!("unsupported PRAGMA {other}"))),
        }
    }

    // ---- INSERT --------------------------------------------------------------------------------

    fn insert(&mut self, ins: &Insert, params: &[Value]) -> Result<Outcome> {
        let functions = self.functions;
        // Phase 1: the rows to insert.
        let (target_cols, source_rows): (Vec<usize>, Vec<Vec<Value>>) = {
            let ex = Self::executor(self.db, functions, params);
            let t = self.db.table(&ins.table)?;
            let cols: Vec<usize> = match &ins.columns {
                Some(names) => names
                    .iter()
                    .map(|n| {
                        t.def.column_index(n).ok_or_else(|| {
                            Error::new(
                                ErrorKind::NoSuchColumn,
                                format!("table {} has no column named {n}", ins.table),
                            )
                        })
                    })
                    .collect::<Result<_>>()?,
                None => (0..t.def.columns.len()).collect(),
            };
            let rows = match &ins.source {
                InsertSource::Values(rows) => {
                    let mut out = Vec::new();
                    for r in rows {
                        let mut vals = Vec::new();
                        for e in r {
                            vals.push(ex.env.eval(e, None, None, &ex)?);
                        }
                        out.push(vals);
                    }
                    out
                }
                InsertSource::Select(s) => ex.select(s, None)?.rows,
                InsertSource::DefaultValues => vec![Vec::new()],
            };
            let cols = if matches!(ins.source, InsertSource::DefaultValues) {
                Vec::new()
            } else {
                cols
            };
            for r in &rows {
                if r.len() != cols.len() {
                    return Err(Error::syntax(format!(
                        "table {} has {} columns but {} values were supplied",
                        ins.table,
                        cols.len(),
                        r.len()
                    )));
                }
            }
            (cols, rows)
        };
        let mut changes = 0;
        let mut last = None;
        for src in source_rows {
            if let Some(rowid) = self.insert_row(ins, &target_cols, src, params)? {
                changes += 1;
                last = Some(rowid);
            }
        }
        Ok(Outcome {
            result: None,
            changes,
            last_insert_rowid: last,
        })
    }

    /// Inserts one row; `None` when the row was skipped by OR IGNORE or DO NOTHING.
    fn insert_row(
        &mut self,
        ins: &Insert,
        target_cols: &[usize],
        values: Vec<Value>,
        params: &[Value],
    ) -> Result<Option<i64>> {
        let functions = self.functions;
        let def = self.db.table(&ins.table)?.def.clone();
        // Build the full row.
        let mut row: Vec<Option<Value>> = vec![None; def.columns.len()];
        for (slot, v) in target_cols.iter().zip(values) {
            row[*slot] = Some(v);
        }
        let mut full = Vec::with_capacity(def.columns.len());
        for (i, c) in def.columns.iter().enumerate() {
            let v = match row[i].take() {
                Some(v) => v,
                None => match &c.default {
                    Some(e) => {
                        let ex = Self::executor(self.db, functions, params);
                        ex.env.eval(e, None, None, &ex)?
                    }
                    None => Value::Null,
                },
            };
            full.push(v.apply_affinity(c.affinity));
        }
        let t_next = {
            let t = self.db.table(&ins.table)?;
            let base = if def.autoincrement {
                t.sequence
                    .max(t.rows.keys().next_back().copied().unwrap_or(0))
            } else {
                t.rows.keys().next_back().copied().unwrap_or(0)
            };
            base + 1
        };
        let rowid = match def.rowid_alias {
            Some(a) => match &full[a] {
                Value::Null => {
                    full[a] = Value::Integer(t_next);
                    t_next
                }
                Value::Integer(i) => *i,
                _ => return Err(Error::new(ErrorKind::Type, "datatype mismatch")),
            },
            None => t_next,
        };
        if let Err(e) = self.check_row(&def, &full, params) {
            if ins.or == OnConflict::Ignore && e.is_constraint() {
                return Ok(None);
            }
            return Err(e);
        }
        // Conflicts.
        loop {
            let conflict = self.db.table(&ins.table)?.unique_conflict(&full, None);
            let Some((index, other)) = conflict else {
                break;
            };
            if let Some(up) = &ins.upsert {
                let t = self.db.table(&ins.table)?;
                let target_matches = up.target.is_empty() || {
                    let want: Vec<usize> = up
                        .target
                        .iter()
                        .filter_map(|n| def.column_index(n))
                        .collect();
                    let have: Vec<usize> = if index == usize::MAX {
                        def.rowid_alias.into_iter().collect()
                    } else {
                        t.indexes[index].columns.clone()
                    };
                    let mut a = want.clone();
                    let mut b = have;
                    a.sort_unstable();
                    b.sort_unstable();
                    a == b
                };
                if target_matches {
                    return match &up.action {
                        UpsertAction::Nothing => Ok(None),
                        UpsertAction::Update { sets, where_ } => {
                            let old = t.rows.get(&other).cloned().ok_or_else(|| {
                                Error::new(ErrorKind::Corrupt, "conflicting row vanished")
                            })?;
                            let new_row = {
                                let mut has_rowid = def
                                    .columns
                                    .iter()
                                    .any(|c| c.name.eq_ignore_ascii_case("rowid"));
                                let mut src_row = old.clone();
                                if !has_rowid {
                                    src_row.push(Value::Integer(other));
                                    has_rowid = true;
                                }
                                let _ = has_rowid;
                                let source = table_source(t, None);
                                let joined: JoinedRow = vec![Some(src_row)];
                                let sources = [source];
                                let scope = Scope {
                                    sources: &sources,
                                    row: &joined,
                                    parent: None,
                                };
                                let ex = Executor {
                                    env: Env {
                                        db: self.db,
                                        params,
                                        functions,
                                        excluded: Some(Excluded {
                                            def: &def,
                                            row: &full,
                                        }),
                                    },
                                };
                                if let Some(w) = where_
                                    && ex.env.eval(w, Some(&scope), None, &ex)?.truth()
                                        != Some(true)
                                {
                                    return Ok(None);
                                }
                                let mut nr = old.clone();
                                for (name, e) in sets {
                                    let ci = def.column_index(name).ok_or_else(|| {
                                        Error::new(
                                            ErrorKind::NoSuchColumn,
                                            format!("no such column: {name}"),
                                        )
                                    })?;
                                    nr[ci] = ex
                                        .env
                                        .eval(e, Some(&scope), None, &ex)?
                                        .apply_affinity(def.columns[ci].affinity);
                                }
                                nr
                            };
                            self.check_row(&def, &new_row, params)?;
                            self.write_update(&ins.table, other, new_row)?;
                            Ok(Some(other))
                        }
                    };
                }
            }
            match ins.or {
                OnConflict::Ignore => return Ok(None),
                OnConflict::Replace => {
                    self.delete_row(&ins.table, other)?;
                }
                OnConflict::Abort => {
                    let t = self.db.table(&ins.table)?;
                    let cols: Vec<String> = if index == usize::MAX {
                        def.rowid_alias
                            .map(|a| def.columns[a].name.clone())
                            .into_iter()
                            .collect()
                    } else {
                        t.indexes[index]
                            .columns
                            .iter()
                            .map(|&c| def.columns[c].name.clone())
                            .collect()
                    };
                    return Err(Error::constraint(format!(
                        "UNIQUE constraint failed: {}",
                        cols.iter()
                            .map(|c| format!("{}.{c}", def.name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        }
        self.check_foreign_keys(&def, &full)?;
        let t = self.db.table_mut(&ins.table)?;
        let sequence = t.sequence;
        t.put(rowid, full.clone());
        self.db.record(
            Change::Insert {
                table: ins.table.clone(),
                rowid,
                sequence,
            },
            Op::Put {
                table: lc(&ins.table),
                rowid,
                row: full,
            },
        );
        Ok(Some(rowid))
    }

    fn check_row(&self, def: &TableDef, row: &[Value], params: &[Value]) -> Result<()> {
        for (i, c) in def.columns.iter().enumerate() {
            if c.not_null && row[i].is_null() && Some(i) != def.rowid_alias {
                return Err(Error::constraint(format!(
                    "NOT NULL constraint failed: {}.{}",
                    def.name, c.name
                )));
            }
        }
        if !def.checks.is_empty() {
            let mut columns: Vec<(String, Option<Affinity>)> = def
                .columns
                .iter()
                .map(|c| (lc(&c.name), Some(c.affinity)))
                .collect();
            columns.push(("rowid".into(), Some(Affinity::Integer)));
            let sources = [Source {
                alias: lc(&def.name),
                columns,
            }];
            let mut r = row.to_vec();
            r.push(Value::Null);
            let joined: JoinedRow = vec![Some(r)];
            let scope = Scope {
                sources: &sources,
                row: &joined,
                parent: None,
            };
            let ex = Self::executor(self.db, self.functions, params);
            for chk in &def.checks {
                if ex.env.eval(chk, Some(&scope), None, &ex)?.truth() == Some(false) {
                    return Err(Error::constraint(format!(
                        "CHECK constraint failed: {}",
                        def.name
                    )));
                }
            }
        }
        Ok(())
    }

    fn check_foreign_keys(&self, def: &TableDef, row: &[Value]) -> Result<()> {
        if !self.db.foreign_keys {
            return Ok(());
        }
        for fk in &def.foreign_keys {
            let vals: Vec<Value> = fk.columns.iter().map(|&c| row[c].clone()).collect();
            if vals.iter().any(Value::is_null) {
                continue;
            }
            let parent = self
                .db
                .table(&fk.parent)
                .map_err(|_| Error::new(ErrorKind::Constraint, "FOREIGN KEY constraint failed"))?;
            let pcols: Vec<usize> = if fk.parent_columns.is_empty() {
                parent.def.primary_key.clone()
            } else {
                fk.parent_columns
                    .iter()
                    .filter_map(|n| parent.def.column_index(n))
                    .collect()
            };
            let found = match parent.lookup(&pcols, &vals) {
                Some(ids) => !ids.is_empty(),
                None => parent.rows.values().any(|r| {
                    pcols
                        .iter()
                        .zip(&vals)
                        .all(|(&c, v)| r[c].compare(v) == Ordering::Equal)
                }),
            };
            if !found {
                return Err(Error::constraint("FOREIGN KEY constraint failed"));
            }
        }
        Ok(())
    }

    fn check_not_referenced(&self, def: &TableDef, old: &[Value]) -> Result<()> {
        if !self.db.foreign_keys {
            return Ok(());
        }
        for child in self.db.tables.values() {
            for fk in &child.def.foreign_keys {
                if !fk.parent.eq_ignore_ascii_case(&def.name) {
                    continue;
                }
                let pcols: Vec<usize> = if fk.parent_columns.is_empty() {
                    def.primary_key.clone()
                } else {
                    fk.parent_columns
                        .iter()
                        .filter_map(|n| def.column_index(n))
                        .collect()
                };
                let vals: Vec<Value> = pcols.iter().map(|&c| old[c].clone()).collect();
                let referenced = match child.lookup(&fk.columns, &vals) {
                    Some(ids) => !ids.is_empty(),
                    None => child.rows.values().any(|r| {
                        fk.columns
                            .iter()
                            .zip(&vals)
                            .all(|(&c, v)| r[c].compare(v) == Ordering::Equal)
                    }),
                };
                if referenced {
                    return Err(Error::constraint("FOREIGN KEY constraint failed"));
                }
            }
        }
        Ok(())
    }

    fn write_update(&mut self, table: &str, rowid: i64, new_row: Vec<Value>) -> Result<()> {
        let t = self.db.table_mut(table)?;
        let old = t
            .rows
            .get(&rowid)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Corrupt, "row vanished"))?;
        t.put(rowid, new_row.clone());
        self.db.record(
            Change::Update {
                table: table.to_string(),
                rowid,
                old,
            },
            Op::Put {
                table: lc(table),
                rowid,
                row: new_row,
            },
        );
        Ok(())
    }

    fn delete_row(&mut self, table: &str, rowid: i64) -> Result<()> {
        let t = self.db.table_mut(table)?;
        if let Some(old) = t.remove(rowid) {
            self.db.record(
                Change::Delete {
                    table: table.to_string(),
                    rowid,
                    row: old,
                },
                Op::Del {
                    table: lc(table),
                    rowid,
                },
            );
        }
        Ok(())
    }

    // ---- UPDATE and DELETE ---------------------------------------------------------------------

    /// Rowids and current rows matching a WHERE clause, found through indexes when possible.
    fn matching_rows(
        &self,
        table: &str,
        where_: Option<&Expr>,
        params: &[Value],
    ) -> Result<Matched> {
        let ex = Self::executor(self.db, self.functions, params);
        let t = self.db.table(table)?;
        let source = table_source(t, None);
        let mut terms = Vec::new();
        if let Some(w) = where_ {
            conjuncts(w, &mut terms);
        }
        let eqs = ex.constant_equalities(&terms, &source, true, None)?;
        let ids = ex.rowids_by_index(t, &eqs);
        let candidates = table_rows(t, ids.as_deref());
        let has_rowid_col = t
            .def
            .columns
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case("rowid"));
        let mut out = Vec::new();
        let sources = [source];
        for mut row in candidates {
            let rowid = if has_rowid_col {
                None
            } else {
                row.last().cloned()
            };
            let joined: JoinedRow = vec![Some(row.clone())];
            let keep = match where_ {
                Some(w) => {
                    let scope = Scope {
                        sources: &sources,
                        row: &joined,
                        parent: None,
                    };
                    ex.env.eval(w, Some(&scope), None, &ex)?.truth() == Some(true)
                }
                None => true,
            };
            if keep {
                let id = match rowid {
                    Some(Value::Integer(i)) => i,
                    _ => match t.def.rowid_alias {
                        Some(a) => match &row[a] {
                            Value::Integer(i) => *i,
                            _ => 0,
                        },
                        None => 0,
                    },
                };
                if !has_rowid_col {
                    row.pop();
                }
                out.push((id, row));
            }
        }
        Ok((t.def.clone(), out))
    }

    fn update(&mut self, upd: &Update, params: &[Value]) -> Result<Outcome> {
        let functions = self.functions;
        let (def, matches) = self.matching_rows(&upd.table, upd.where_.as_ref(), params)?;
        // Compute new rows against the pre-update state.
        let mut planned: Vec<(i64, Vec<Value>, Vec<Value>)> = Vec::new();
        {
            let ex = Self::executor(self.db, functions, params);
            let t = self.db.table(&upd.table)?;
            let source = table_source(t, None);
            let sources = [source];
            let set_idx: Vec<(usize, &Expr)> = upd
                .sets
                .iter()
                .map(|(n, e)| {
                    def.column_index(n).map(|i| (i, e)).ok_or_else(|| {
                        Error::new(ErrorKind::NoSuchColumn, format!("no such column: {n}"))
                    })
                })
                .collect::<Result<_>>()?;
            let has_rowid_col = def
                .columns
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case("rowid"));
            for (rowid, old) in matches {
                let mut src_row = old.clone();
                if !has_rowid_col {
                    src_row.push(Value::Integer(rowid));
                }
                let joined: JoinedRow = vec![Some(src_row)];
                let scope = Scope {
                    sources: &sources,
                    row: &joined,
                    parent: None,
                };
                let mut new_row = old.clone();
                for (ci, e) in &set_idx {
                    new_row[*ci] = ex
                        .env
                        .eval(e, Some(&scope), None, &ex)?
                        .apply_affinity(def.columns[*ci].affinity);
                }
                planned.push((rowid, old, new_row));
            }
        }
        let mut changes = 0;
        for (rowid, old, new_row) in planned {
            self.check_row(&def, &new_row, params)?;
            let new_rowid = match def.rowid_alias {
                Some(a) => match &new_row[a] {
                    Value::Integer(i) => *i,
                    Value::Null => rowid,
                    _ => return Err(Error::new(ErrorKind::Type, "datatype mismatch")),
                },
                None => rowid,
            };
            // Unique conflicts with other rows.
            if let Some((index, other)) = self
                .db
                .table(&upd.table)?
                .unique_conflict(&new_row, Some(rowid))
            {
                match upd.or {
                    OnConflict::Ignore => continue,
                    OnConflict::Replace => self.delete_row(&upd.table, other)?,
                    OnConflict::Abort => {
                        let t = self.db.table(&upd.table)?;
                        let cols: Vec<String> = if index == usize::MAX {
                            def.rowid_alias
                                .map(|a| def.columns[a].name.clone())
                                .into_iter()
                                .collect()
                        } else {
                            t.indexes[index]
                                .columns
                                .iter()
                                .map(|&c| def.columns[c].name.clone())
                                .collect()
                        };
                        return Err(Error::constraint(format!(
                            "UNIQUE constraint failed: {}",
                            cols.iter()
                                .map(|c| format!("{}.{c}", def.name))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }
                }
            }
            self.check_foreign_keys(&def, &new_row)?;
            // A changed key that other tables reference is refused.
            let key_changed = def
                .primary_key
                .iter()
                .any(|&c| old[c].compare(&new_row[c]) != Ordering::Equal)
                || new_rowid != rowid;
            if key_changed {
                self.check_not_referenced(&def, &old)?;
            }
            if new_rowid != rowid {
                self.delete_row(&upd.table, rowid)?;
                let t = self.db.table_mut(&upd.table)?;
                let sequence = t.sequence;
                t.put(new_rowid, new_row.clone());
                self.db.record(
                    Change::Insert {
                        table: upd.table.clone(),
                        rowid: new_rowid,
                        sequence,
                    },
                    Op::Put {
                        table: lc(&upd.table),
                        rowid: new_rowid,
                        row: new_row,
                    },
                );
            } else {
                self.write_update(&upd.table, rowid, new_row)?;
            }
            changes += 1;
        }
        Ok(Outcome {
            result: None,
            changes,
            last_insert_rowid: None,
        })
    }

    fn delete(&mut self, del: &Delete, params: &[Value]) -> Result<Outcome> {
        let (def, matches) = self.matching_rows(&del.table, del.where_.as_ref(), params)?;
        let mut changes = 0;
        for (rowid, old) in matches {
            self.check_not_referenced(&def, &old)?;
            self.delete_row(&del.table, rowid)?;
            changes += 1;
        }
        Ok(Outcome {
            result: None,
            changes,
            last_insert_rowid: None,
        })
    }
}

#[allow(dead_code)]
fn _coerce(a: Value, b: Value) -> (Value, Value) {
    coerce_for_comparison(a, None, b, None)
}
