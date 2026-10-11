//! Expression evaluation with SQLite's semantics: three-valued logic, affinity-aware
//! comparison, integer overflow to real, and the scalar and aggregate functions in use.

use crate::{
    ast::*,
    db::{Database, TableDef},
    error::{Error, ErrorKind, Result},
    exec::Functions,
    value::{Affinity, Numeric, Value, coerce_for_comparison, format_real, parse_numeric_prefix},
};
use std::cmp::Ordering;

/// A table or subquery taking part in a query: its name, alias and columns.
#[derive(Clone, Debug)]
pub struct Source {
    pub alias: String,
    pub columns: Vec<(String, Option<Affinity>)>,
}

pub type JoinedRow = Vec<Option<Vec<Value>>>;

pub struct Scope<'a> {
    pub sources: &'a [Source],
    pub row: &'a JoinedRow,
    pub parent: Option<&'a Scope<'a>>,
}

/// The rows of one group, for aggregate evaluation.
pub struct Group<'a> {
    pub rows: &'a [JoinedRow],
}

pub struct Excluded<'a> {
    pub def: &'a TableDef,
    pub row: &'a [Value],
}

pub struct Env<'a> {
    pub db: &'a Database,
    pub params: &'a [Value],
    pub functions: &'a Functions,
    pub excluded: Option<Excluded<'a>>,
}

fn lc(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn bool_value(b: Option<bool>) -> Value {
    b.map_or(Value::Null, Value::from_bool)
}

pub fn is_aggregate_name(name: &str, args: usize) -> bool {
    match name {
        "count" | "sum" | "total" | "avg" | "group_concat" => true,
        "min" | "max" => args == 1,
        _ => false,
    }
}

pub fn contains_aggregate(e: &Expr) -> bool {
    match e {
        Expr::Function {
            name, args, star, ..
        } => (*star || is_aggregate_name(name, args.len())) || args.iter().any(contains_aggregate),
        Expr::Unary { operand, .. } => contains_aggregate(operand),
        Expr::Binary { left, right, .. } => contains_aggregate(left) || contains_aggregate(right),
        Expr::Between {
            expr, low, high, ..
        } => contains_aggregate(expr) || contains_aggregate(low) || contains_aggregate(high),
        Expr::InList { expr, list, .. } => {
            contains_aggregate(expr) || list.iter().any(contains_aggregate)
        }
        Expr::InSelect { expr, .. } => contains_aggregate(expr),
        Expr::Like { expr, pattern, .. } => contains_aggregate(expr) || contains_aggregate(pattern),
        Expr::IsNull { expr, .. } => contains_aggregate(expr),
        Expr::Case {
            operand,
            branches,
            otherwise,
        } => {
            operand.as_deref().is_some_and(contains_aggregate)
                || branches
                    .iter()
                    .any(|(w, t)| contains_aggregate(w) || contains_aggregate(t))
                || otherwise.as_deref().is_some_and(contains_aggregate)
        }
        Expr::Cast { expr, .. } => contains_aggregate(expr),
        _ => false,
    }
}

impl Env<'_> {
    pub fn affinity_of(&self, e: &Expr, scope: Option<&Scope>) -> Option<Affinity> {
        match e {
            Expr::Column { table, name } => {
                if table
                    .as_deref()
                    .is_some_and(|t| t.eq_ignore_ascii_case("excluded"))
                    && let Some(x) = &self.excluded
                {
                    return x.def.column_index(name).map(|i| x.def.columns[i].affinity);
                }
                let mut s = scope;
                while let Some(sc) = s {
                    if let Some((si, ci)) = find_column(sc, table.as_deref(), name) {
                        return sc.sources[si].columns[ci].1;
                    }
                    s = sc.parent;
                }
                None
            }
            Expr::Unary {
                op: UnaryOp::Plus,
                operand,
            } => self.affinity_of(operand, scope),
            Expr::Cast { to, .. } => {
                let a = Affinity::of_declared(to);
                if a == Affinity::Blob { None } else { Some(a) }
            }
            _ => None,
        }
    }

    pub fn eval(
        &self,
        e: &Expr,
        scope: Option<&Scope>,
        group: Option<&Group>,
        exec: &dyn SubqueryRunner,
    ) -> Result<Value> {
        match e {
            Expr::Literal(v) => Ok(v.clone()),
            Expr::Param(n) => self.params.get(n - 1).cloned().ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidParameterCount,
                    format!("parameter ?{n} was not bound"),
                )
            }),
            Expr::Column { table, name } => self.column(table.as_deref(), name, scope),
            Expr::Unary { op, operand } => {
                let v = self.eval(operand, scope, group, exec)?;
                Ok(match op {
                    UnaryOp::Not => bool_value(v.truth().map(|b| !b)),
                    UnaryOp::Plus => v,
                    UnaryOp::Negate => match v.to_numeric() {
                        None => Value::Null,
                        Some(Numeric::Int(i)) => i
                            .checked_neg()
                            .map_or(Value::Real(-(i as f64)), Value::Integer),
                        Some(Numeric::Real(f)) => Value::Real(-f),
                    },
                    UnaryOp::BitNot => match v.to_numeric() {
                        None => Value::Null,
                        Some(n) => Value::Integer(!to_int(n)),
                    },
                })
            }
            Expr::Binary { op, left, right } => self.binary(*op, left, right, scope, group, exec),
            Expr::Between {
                expr,
                low,
                high,
                negate,
            } => {
                let ge = Expr::Binary {
                    op: BinaryOp::Ge,
                    left: expr.clone(),
                    right: low.clone(),
                };
                let le = Expr::Binary {
                    op: BinaryOp::Le,
                    left: expr.clone(),
                    right: high.clone(),
                };
                let both = Expr::Binary {
                    op: BinaryOp::And,
                    left: Box::new(ge),
                    right: Box::new(le),
                };
                let v = self.eval(&both, scope, group, exec)?;
                Ok(if *negate {
                    bool_value(v.truth().map(|b| !b))
                } else {
                    v
                })
            }
            Expr::InList { expr, list, negate } => {
                let needle = self.eval(expr, scope, group, exec)?;
                if needle.is_null() {
                    return Ok(Value::Null);
                }
                let n_aff = self.affinity_of(expr, scope);
                let mut saw_null = false;
                for item in list {
                    let candidate = self.eval(item, scope, group, exec)?;
                    if candidate.is_null() {
                        saw_null = true;
                        continue;
                    }
                    let (a, b) = coerce_for_comparison(
                        needle.clone(),
                        n_aff,
                        candidate,
                        self.affinity_of(item, scope),
                    );
                    if a.compare(&b) == Ordering::Equal {
                        return Ok(Value::from_bool(!negate));
                    }
                }
                Ok(if saw_null {
                    Value::Null
                } else {
                    Value::from_bool(*negate)
                })
            }
            Expr::InSelect {
                expr,
                select,
                negate,
            } => {
                let needle = self.eval(expr, scope, group, exec)?;
                if needle.is_null() {
                    return Ok(Value::Null);
                }
                let rows = exec.run_select(select, scope)?;
                let mut saw_null = false;
                for r in &rows.rows {
                    let candidate = r.first().cloned().unwrap_or(Value::Null);
                    if candidate.is_null() {
                        saw_null = true;
                        continue;
                    }
                    if needle.compare(&candidate) == Ordering::Equal {
                        return Ok(Value::from_bool(!negate));
                    }
                }
                Ok(if saw_null {
                    Value::Null
                } else {
                    Value::from_bool(*negate)
                })
            }
            Expr::Like {
                expr,
                pattern,
                negate,
            } => {
                let v = self.eval(expr, scope, group, exec)?;
                let p = self.eval(pattern, scope, group, exec)?;
                match (v.to_text(), p.to_text()) {
                    (Some(v), Some(p)) => {
                        let m = like(&p, &v);
                        Ok(Value::from_bool(m != *negate))
                    }
                    _ => Ok(Value::Null),
                }
            }
            Expr::IsNull { expr, negate } => {
                let v = self.eval(expr, scope, group, exec)?;
                Ok(Value::from_bool(v.is_null() != *negate))
            }
            Expr::Case {
                operand,
                branches,
                otherwise,
            } => {
                let subject = match operand {
                    Some(o) => Some(self.eval(o, scope, group, exec)?),
                    None => None,
                };
                for (when, then) in branches {
                    let hit = match &subject {
                        Some(s) => {
                            let w = self.eval(when, scope, group, exec)?;
                            !s.is_null() && !w.is_null() && {
                                let (a, b) = coerce_for_comparison(
                                    s.clone(),
                                    self.affinity_of(operand.as_deref().unwrap(), scope),
                                    w,
                                    self.affinity_of(when, scope),
                                );
                                a.compare(&b) == Ordering::Equal
                            }
                        }
                        None => self.eval(when, scope, group, exec)?.truth() == Some(true),
                    };
                    if hit {
                        return self.eval(then, scope, group, exec);
                    }
                }
                match otherwise {
                    Some(e) => self.eval(e, scope, group, exec),
                    None => Ok(Value::Null),
                }
            }
            Expr::Function {
                name,
                args,
                star,
                distinct,
            } => self.function(name, args, *star, *distinct, scope, group, exec),
            Expr::Exists(select) => {
                let rows = exec.run_select(select, scope)?;
                Ok(Value::from_bool(!rows.rows.is_empty()))
            }
            Expr::Subquery(select) => {
                let rows = exec.run_select(select, scope)?;
                Ok(rows
                    .rows
                    .first()
                    .and_then(|r| r.first().cloned())
                    .unwrap_or(Value::Null))
            }
            Expr::Cast { expr, to } => {
                let v = self.eval(expr, scope, group, exec)?;
                Ok(cast(v, to))
            }
        }
    }

    fn column(&self, table: Option<&str>, name: &str, scope: Option<&Scope>) -> Result<Value> {
        if table.is_some_and(|t| t.eq_ignore_ascii_case("excluded"))
            && let Some(x) = &self.excluded
        {
            let i = x.def.column_index(name).ok_or_else(|| {
                Error::new(
                    ErrorKind::NoSuchColumn,
                    format!("no such column: excluded.{name}"),
                )
            })?;
            return Ok(x.row[i].clone());
        }
        let mut s = scope;
        while let Some(sc) = s {
            if let Some((si, ci)) = find_column_checked(sc, table, name)? {
                return Ok(match &sc.row[si] {
                    Some(r) => r[ci].clone(),
                    None => Value::Null,
                });
            }
            s = sc.parent;
        }
        if table.is_none() && name.eq_ignore_ascii_case("rowid") {
            return Err(Error::new(
                ErrorKind::NoSuchColumn,
                "rowid is not available here",
            ));
        }
        Err(Error::new(
            ErrorKind::NoSuchColumn,
            format!(
                "no such column: {}{}",
                table.map(|t| format!("{t}.")).unwrap_or_default(),
                name
            ),
        ))
    }

    fn binary(
        &self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        scope: Option<&Scope>,
        group: Option<&Group>,
        exec: &dyn SubqueryRunner,
    ) -> Result<Value> {
        match op {
            BinaryOp::And => {
                let l = self.eval(left, scope, group, exec)?.truth();
                if l == Some(false) {
                    return Ok(Value::Integer(0));
                }
                let r = self.eval(right, scope, group, exec)?.truth();
                return Ok(match (l, r) {
                    (_, Some(false)) => Value::Integer(0),
                    (Some(true), Some(true)) => Value::Integer(1),
                    _ => Value::Null,
                });
            }
            BinaryOp::Or => {
                let l = self.eval(left, scope, group, exec)?.truth();
                if l == Some(true) {
                    return Ok(Value::Integer(1));
                }
                let r = self.eval(right, scope, group, exec)?.truth();
                return Ok(match (l, r) {
                    (_, Some(true)) => Value::Integer(1),
                    (Some(false), Some(false)) => Value::Integer(0),
                    _ => Value::Null,
                });
            }
            _ => {}
        }
        let l = self.eval(left, scope, group, exec)?;
        let r = self.eval(right, scope, group, exec)?;
        match op {
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => {
                if l.is_null() || r.is_null() {
                    return Ok(Value::Null);
                }
                let (a, b) = coerce_for_comparison(
                    l,
                    self.affinity_of(left, scope),
                    r,
                    self.affinity_of(right, scope),
                );
                let o = a.compare(&b);
                Ok(Value::from_bool(match op {
                    BinaryOp::Eq => o == Ordering::Equal,
                    BinaryOp::Ne => o != Ordering::Equal,
                    BinaryOp::Lt => o == Ordering::Less,
                    BinaryOp::Le => o != Ordering::Greater,
                    BinaryOp::Gt => o == Ordering::Greater,
                    _ => o != Ordering::Less,
                }))
            }
            BinaryOp::Is | BinaryOp::IsNot => {
                let eq = if l.is_null() || r.is_null() {
                    l.is_null() && r.is_null()
                } else {
                    let (a, b) = coerce_for_comparison(
                        l,
                        self.affinity_of(left, scope),
                        r,
                        self.affinity_of(right, scope),
                    );
                    a.compare(&b) == Ordering::Equal
                };
                Ok(Value::from_bool(eq == (op == BinaryOp::Is)))
            }
            BinaryOp::Concat => match (l.to_text(), r.to_text()) {
                (Some(a), Some(b)) => Ok(Value::Text(a + &b)),
                _ => Ok(Value::Null),
            },
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                let (Some(a), Some(b)) = (l.to_numeric(), r.to_numeric()) else {
                    return Ok(Value::Null);
                };
                Ok(match (a, b) {
                    (Numeric::Int(x), Numeric::Int(y)) => {
                        let r = match op {
                            BinaryOp::Add => x.checked_add(y),
                            BinaryOp::Sub => x.checked_sub(y),
                            _ => x.checked_mul(y),
                        };
                        r.map_or_else(
                            || Value::real(real_op(op, x as f64, y as f64)),
                            Value::Integer,
                        )
                    }
                    (a, b) => Value::real(real_op(op, a.as_f64(), b.as_f64())),
                })
            }
            BinaryOp::Div => {
                let (Some(a), Some(b)) = (l.to_numeric(), r.to_numeric()) else {
                    return Ok(Value::Null);
                };
                Ok(match (a, b) {
                    (_, Numeric::Int(0)) => Value::Null,
                    (Numeric::Int(x), Numeric::Int(y)) => x
                        .checked_div(y)
                        .map_or_else(|| Value::Real(x as f64 / y as f64), Value::Integer),
                    (a, b) => {
                        let d = b.as_f64();
                        if d == 0.0 {
                            Value::Null
                        } else {
                            Value::real(a.as_f64() / d)
                        }
                    }
                })
            }
            BinaryOp::Rem => {
                let (Some(a), Some(b)) = (l.to_numeric(), r.to_numeric()) else {
                    return Ok(Value::Null);
                };
                let (x, y) = (to_int(a), to_int(b));
                if y == 0 {
                    return Ok(Value::Null);
                }
                let m = x.checked_rem(y).unwrap_or(0);
                Ok(
                    if matches!(a, Numeric::Real(_)) || matches!(b, Numeric::Real(_)) {
                        Value::Real(m as f64)
                    } else {
                        Value::Integer(m)
                    },
                )
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::ShiftLeft | BinaryOp::ShiftRight => {
                let (Some(a), Some(b)) = (l.to_numeric(), r.to_numeric()) else {
                    return Ok(Value::Null);
                };
                let (x, y) = (to_int(a), to_int(b));
                Ok(Value::Integer(match op {
                    BinaryOp::BitAnd => x & y,
                    BinaryOp::BitOr => x | y,
                    BinaryOp::ShiftLeft => shift(x, y),
                    _ => shift(x, y.saturating_neg()),
                }))
            }
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn function(
        &self,
        name: &str,
        args: &[Expr],
        star: bool,
        distinct: bool,
        scope: Option<&Scope>,
        group: Option<&Group>,
        exec: &dyn SubqueryRunner,
    ) -> Result<Value> {
        if star || is_aggregate_name(name, args.len()) {
            let Some(g) = group else {
                return Err(Error::misuse(format!(
                    "misuse of aggregate function {name}()"
                )));
            };
            return self.aggregate(name, args, star, distinct, scope, g, exec);
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval(a, scope, group, exec)?);
        }
        if let Some(f) = self.functions.get(name) {
            return f(&vals);
        }
        let need = |n: usize| -> Result<()> {
            if vals.len() == n {
                Ok(())
            } else {
                Err(Error::syntax(format!(
                    "wrong number of arguments to function {name}()"
                )))
            }
        };
        match name {
            "coalesce" => {
                if vals.len() < 2 {
                    return Err(Error::syntax(
                        "wrong number of arguments to function coalesce()",
                    ));
                }
                Ok(vals
                    .into_iter()
                    .find(|v| !v.is_null())
                    .unwrap_or(Value::Null))
            }
            "ifnull" => {
                need(2)?;
                Ok(if vals[0].is_null() {
                    vals[1].clone()
                } else {
                    vals[0].clone()
                })
            }
            "nullif" => {
                need(2)?;
                Ok(
                    if !vals[0].is_null() && vals[0].compare(&vals[1]) == Ordering::Equal {
                        Value::Null
                    } else {
                        vals[0].clone()
                    },
                )
            }
            "iif" => {
                need(3)?;
                Ok(if vals[0].truth() == Some(true) {
                    vals[1].clone()
                } else {
                    vals[2].clone()
                })
            }
            "max" | "min" => {
                if vals.len() < 2 {
                    return Err(Error::syntax(format!(
                        "wrong number of arguments to function {name}()"
                    )));
                }
                if vals.iter().any(Value::is_null) {
                    return Ok(Value::Null);
                }
                let pick = vals.iter().cloned().reduce(|a, b| {
                    let o = a.compare(&b);
                    if (name == "max" && o == Ordering::Less)
                        || (name == "min" && o == Ordering::Greater)
                    {
                        b
                    } else {
                        a
                    }
                });
                Ok(pick.unwrap())
            }
            "abs" => {
                need(1)?;
                Ok(match vals[0].to_numeric() {
                    None => Value::Null,
                    Some(Numeric::Int(i)) => i
                        .checked_abs()
                        .map_or(Value::Real((i as f64).abs()), Value::Integer),
                    Some(Numeric::Real(f)) => Value::Real(f.abs()),
                })
            }
            "length" => {
                need(1)?;
                Ok(match &vals[0] {
                    Value::Null => Value::Null,
                    Value::Blob(b) => Value::Integer(b.len() as i64),
                    v => Value::Integer(v.to_text().unwrap().chars().count() as i64),
                })
            }
            "lower" | "upper" => {
                need(1)?;
                Ok(match vals[0].to_text() {
                    None => Value::Null,
                    Some(t) => Value::Text(if name == "lower" {
                        t.to_ascii_lowercase()
                    } else {
                        t.to_ascii_uppercase()
                    }),
                })
            }
            "typeof" => {
                need(1)?;
                Ok(Value::Text(vals[0].type_name().into()))
            }
            "trim" | "ltrim" | "rtrim" => {
                if vals.is_empty() || vals.len() > 2 {
                    return Err(Error::syntax(format!(
                        "wrong number of arguments to function {name}()"
                    )));
                }
                let Some(t) = vals[0].to_text() else {
                    return Ok(Value::Null);
                };
                let set: Vec<char> = vals
                    .get(1)
                    .and_then(Value::to_text)
                    .unwrap_or_else(|| " ".into())
                    .chars()
                    .collect();
                let out = match name {
                    "trim" => t.trim_matches(|c| set.contains(&c)),
                    "ltrim" => t.trim_start_matches(|c| set.contains(&c)),
                    _ => t.trim_end_matches(|c| set.contains(&c)),
                };
                Ok(Value::Text(out.to_string()))
            }
            "substr" | "substring" => {
                if vals.len() < 2 || vals.len() > 3 {
                    return Err(Error::syntax(
                        "wrong number of arguments to function substr()",
                    ));
                }
                let Some(t) = vals[0].to_text() else {
                    return Ok(Value::Null);
                };
                let chars: Vec<char> = t.chars().collect();
                let n = chars.len() as i64;
                let Some(Numeric::Int(mut start)) =
                    vals[1].to_numeric().map(|x| Numeric::Int(to_int(x)))
                else {
                    return Ok(Value::Null);
                };
                let mut len = match vals.get(2) {
                    None => i64::MAX / 2,
                    Some(v) => match v.to_numeric() {
                        None => return Ok(Value::Null),
                        Some(x) => to_int(x),
                    },
                };
                if start < 0 {
                    start += n + 1;
                    if start < 1 {
                        len += start - 1;
                        start = 1;
                    }
                } else if start == 0 {
                    len -= 1;
                    start = 1;
                }
                if len < 0 {
                    start = (start + len).max(1);
                    len = -len;
                }
                let from = (start - 1).clamp(0, n) as usize;
                let to = (start - 1 + len).clamp(0, n) as usize;
                Ok(Value::Text(chars[from..to.max(from)].iter().collect()))
            }
            "instr" => {
                need(2)?;
                match (vals[0].to_text(), vals[1].to_text()) {
                    (Some(h), Some(n)) => Ok(Value::Integer(
                        h.find(&n).map_or(0, |i| h[..i].chars().count() as i64 + 1),
                    )),
                    _ => Ok(Value::Null),
                }
            }
            "replace" => {
                need(3)?;
                match (vals[0].to_text(), vals[1].to_text(), vals[2].to_text()) {
                    (Some(s), Some(a), Some(b)) => Ok(Value::Text(if a.is_empty() {
                        s
                    } else {
                        s.replace(&a, &b)
                    })),
                    _ => Ok(Value::Null),
                }
            }
            "round" => {
                if vals.is_empty() || vals.len() > 2 {
                    return Err(Error::syntax(
                        "wrong number of arguments to function round()",
                    ));
                }
                let Some(x) = vals[0].to_numeric() else {
                    return Ok(Value::Null);
                };
                let digits = vals
                    .get(1)
                    .and_then(Value::to_numeric)
                    .map_or(0, to_int)
                    .clamp(0, 30);
                let f = x.as_f64();
                let scale = 10f64.powi(digits as i32);
                let r = (f.abs() * scale + 0.5).floor() / scale;
                Ok(Value::Real(if f < 0.0 { -r } else { r }))
            }
            "like" => {
                need(2)?;
                match (vals[0].to_text(), vals[1].to_text()) {
                    (Some(p), Some(s)) => Ok(Value::from_bool(like(&p, &s))),
                    _ => Ok(Value::Null),
                }
            }
            "hex" => {
                need(1)?;
                let bytes = match &vals[0] {
                    Value::Blob(b) => b.clone(),
                    v => v.to_text().unwrap_or_default().into_bytes(),
                };
                Ok(Value::Text(
                    bytes.iter().map(|b| format!("{b:02X}")).collect(),
                ))
            }
            other => Err(Error::syntax(format!("no such function: {other}"))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn aggregate(
        &self,
        name: &str,
        args: &[Expr],
        star: bool,
        distinct: bool,
        scope: Option<&Scope>,
        group: &Group,
        exec: &dyn SubqueryRunner,
    ) -> Result<Value> {
        let sources: &[Source] = scope.map_or(&[], |s| s.sources);
        let parent = scope.and_then(|s| s.parent);
        if star {
            if name != "count" {
                return Err(Error::syntax(format!("{name}(*) is not valid")));
            }
            return Ok(Value::Integer(group.rows.len() as i64));
        }
        let arg = args.first().ok_or_else(|| {
            Error::syntax(format!("wrong number of arguments to aggregate {name}()"))
        })?;
        let mut values = Vec::with_capacity(group.rows.len());
        for row in group.rows {
            let sc = Scope {
                sources,
                row,
                parent,
            };
            let v = self.eval(arg, Some(&sc), None, exec)?;
            if !v.is_null() {
                values.push(v);
            }
        }
        if distinct {
            let mut seen: Vec<Value> = Vec::new();
            for v in values {
                if !seen.iter().any(|s| s.compare(&v) == Ordering::Equal) {
                    seen.push(v);
                }
            }
            values = seen;
        }
        Ok(match name {
            "count" => Value::Integer(values.len() as i64),
            "sum" | "total" => {
                if values.is_empty() {
                    if name == "sum" {
                        Value::Null
                    } else {
                        Value::Real(0.0)
                    }
                } else {
                    let mut all_int = name == "sum";
                    let mut isum: i64 = 0;
                    let mut fsum = 0.0f64;
                    for v in &values {
                        let n = match v {
                            Value::Text(_) | Value::Blob(_) => v.to_numeric().unwrap(),
                            _ => v.to_numeric().unwrap(),
                        };
                        match n {
                            Numeric::Int(i) if all_int => match isum.checked_add(i) {
                                Some(s) => {
                                    isum = s;
                                    fsum += i as f64;
                                }
                                None => {
                                    return Err(Error::syntax("integer overflow"));
                                }
                            },
                            n => {
                                all_int = false;
                                fsum += n.as_f64();
                            }
                        }
                    }
                    if all_int {
                        Value::Integer(isum)
                    } else {
                        Value::real(fsum)
                    }
                }
            }
            "avg" => {
                if values.is_empty() {
                    Value::Null
                } else {
                    Value::real(
                        values
                            .iter()
                            .map(|v| v.to_numeric().unwrap().as_f64())
                            .sum::<f64>()
                            / values.len() as f64,
                    )
                }
            }
            "min" => values
                .into_iter()
                .reduce(|a, b| {
                    if b.compare(&a) == Ordering::Less {
                        b
                    } else {
                        a
                    }
                })
                .unwrap_or(Value::Null),
            "max" => values
                .into_iter()
                .reduce(|a, b| {
                    if b.compare(&a) == Ordering::Greater {
                        b
                    } else {
                        a
                    }
                })
                .unwrap_or(Value::Null),
            "group_concat" => {
                if values.is_empty() {
                    Value::Null
                } else {
                    let sep = match args.get(1) {
                        Some(e) => self
                            .eval(e, scope, None, exec)?
                            .to_text()
                            .unwrap_or_default(),
                        None => ",".into(),
                    };
                    Value::Text(
                        values
                            .iter()
                            .map(|v| v.to_text().unwrap_or_default())
                            .collect::<Vec<_>>()
                            .join(&sep),
                    )
                }
            }
            other => return Err(Error::syntax(format!("no such aggregate: {other}"))),
        })
    }
}

pub trait SubqueryRunner {
    fn run_select(
        &self,
        select: &Select,
        outer: Option<&Scope>,
    ) -> Result<crate::exec::QueryResult>;
}

fn find_column(sc: &Scope, table: Option<&str>, name: &str) -> Option<(usize, usize)> {
    find_column_checked(sc, table, name).ok().flatten()
}

fn find_column_checked(
    sc: &Scope,
    table: Option<&str>,
    name: &str,
) -> Result<Option<(usize, usize)>> {
    let name_l = lc(name);
    let mut found = None;
    for (si, src) in sc.sources.iter().enumerate() {
        if let Some(t) = table
            && !src.alias.eq_ignore_ascii_case(t)
        {
            continue;
        }
        for (ci, (cn, _)) in src.columns.iter().enumerate() {
            if *cn == name_l {
                if found.is_some() {
                    return Err(Error::new(
                        ErrorKind::NoSuchColumn,
                        format!("ambiguous column name: {name}"),
                    ));
                }
                found = Some((si, ci));
            }
        }
    }
    Ok(found)
}

fn to_int(n: Numeric) -> i64 {
    match n {
        Numeric::Int(i) => i,
        Numeric::Real(f) => {
            if f.is_nan() {
                0
            } else if f >= 9.223_372_036_854_776e18 {
                i64::MAX
            } else if f <= -9.223_372_036_854_776e18 {
                i64::MIN
            } else {
                f as i64
            }
        }
    }
}

fn shift(x: i64, by: i64) -> i64 {
    if by >= 64 || by <= -64 {
        return if by < 0 && x < 0 { -1 } else { 0 };
    }
    if by >= 0 {
        x.wrapping_shl(by as u32)
    } else {
        x >> (-by)
    }
}

fn real_op(op: BinaryOp, a: f64, b: f64) -> f64 {
    match op {
        BinaryOp::Add => a + b,
        BinaryOp::Sub => a - b,
        _ => a * b,
    }
}

pub fn cast(v: Value, to: &str) -> Value {
    if v.is_null() {
        return Value::Null;
    }
    let t = to.to_ascii_uppercase();
    if t.contains("INT") {
        return Value::Integer(match v {
            Value::Integer(i) => i,
            other => to_int(other.to_numeric().unwrap_or(Numeric::Int(0))),
        });
    }
    if t.contains("CHAR") || t.contains("TEXT") || t.contains("CLOB") {
        return Value::Text(v.to_text().unwrap_or_default());
    }
    if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
        return Value::Real(v.to_numeric().map_or(0.0, Numeric::as_f64));
    }
    if t.contains("BLOB") || t.is_empty() {
        return match v {
            Value::Blob(b) => Value::Blob(b),
            other => Value::Blob(other.to_text().unwrap_or_default().into_bytes()),
        };
    }
    match v.to_numeric().unwrap_or(Numeric::Int(0)) {
        Numeric::Real(f) if f.fract() == 0.0 && f.abs() < 9.2e18 => Value::Integer(f as i64),
        n => n.to_value(),
    }
}

/// SQL LIKE: `%` any run, `_` any one character, ASCII letters compare without case.
pub fn like(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('%') => (0..=t.len()).any(|k| go(&p[1..], &t[k..])),
            Some('_') => !t.is_empty() && go(&p[1..], &t[1..]),
            Some(&c) => {
                t.first().is_some_and(|&x| x.eq_ignore_ascii_case(&c)) && go(&p[1..], &t[1..])
            }
        }
    }
    go(&p, &t)
}

#[allow(dead_code)]
fn unused(_: &str) -> String {
    format_real(0.0) + &format!("{:?}", parse_numeric_prefix(""))
}
