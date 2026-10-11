//! Tokenizer and recursive-descent parser for the supported SQL.

use crate::{ast::*, error::Error, value::Value};

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    Int(i64),
    Real(f64),
    Str(String),
    Blob(Vec<u8>),
    Param(Option<usize>),
    Sym(&'static str),
    End,
}

/// Tokens and the byte span of each.
type Lexed = (Vec<Tok>, Vec<(usize, usize)>);

fn lex(sql: &str) -> Result<Lexed, Error> {
    let b: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    while i < b.len() {
        let c = b[i];
        let before = out.len();
        let start_at = i;
        match c {
            c if c.is_whitespace() => i += 1,
            '-' if b.get(i + 1) == Some(&'-') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            '\'' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match b.get(i) {
                        None => return Err(Error::syntax("unterminated string")),
                        Some('\'') if b.get(i + 1) == Some(&'\'') => {
                            s.push('\'');
                            i += 2;
                        }
                        Some('\'') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Tok::Str(s));
            }
            '"' | '`' | '[' => {
                let close = if c == '[' { ']' } else { c };
                let mut s = String::new();
                i += 1;
                loop {
                    match b.get(i) {
                        None => return Err(Error::syntax("unterminated identifier")),
                        Some(&ch)
                            if ch == close && close != ']' && b.get(i + 1) == Some(&close) =>
                        {
                            s.push(close);
                            i += 2;
                        }
                        Some(&ch) if ch == close => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Tok::Quoted(s));
            }
            'x' | 'X' if b.get(i + 1) == Some(&'\'') => {
                i += 2;
                let mut hex = String::new();
                while let Some(&ch) = b.get(i) {
                    i += 1;
                    if ch == '\'' {
                        break;
                    }
                    hex.push(ch);
                }
                if !hex.len().is_multiple_of(2) || !hex.chars().all(|h| h.is_ascii_hexdigit()) {
                    return Err(Error::syntax("malformed blob literal"));
                }
                out.push(Tok::Blob(
                    (0..hex.len() / 2)
                        .map(|k| u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).unwrap())
                        .collect(),
                ));
            }
            c if c.is_ascii_digit()
                || (c == '.' && b.get(i + 1).is_some_and(char::is_ascii_digit)) =>
            {
                let start = i;
                if c == '0' && matches!(b.get(i + 1), Some('x' | 'X')) {
                    i += 2;
                    while i < b.len() && b[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                    let text: String = b[start + 2..i].iter().collect();
                    let v = u64::from_str_radix(&text, 16)
                        .map_err(|_| Error::syntax("bad hex literal"))?;
                    out.push(Tok::Int(v as i64));
                    spans.push((start_at, i));
                    continue;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                let mut real = false;
                if i < b.len() && b[i] == '.' {
                    real = true;
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < b.len() && (b[i] == 'e' || b[i] == 'E') {
                    let mut j = i + 1;
                    if j < b.len() && (b[j] == '+' || b[j] == '-') {
                        j += 1;
                    }
                    if j < b.len() && b[j].is_ascii_digit() {
                        real = true;
                        while j < b.len() && b[j].is_ascii_digit() {
                            j += 1;
                        }
                        i = j;
                    }
                }
                let text: String = b[start..i].iter().collect();
                if real {
                    out.push(Tok::Real(
                        text.parse().map_err(|_| Error::syntax("bad number"))?,
                    ));
                } else {
                    match text.parse::<i64>() {
                        Ok(v) => out.push(Tok::Int(v)),
                        Err(_) => out.push(Tok::Real(
                            text.parse().map_err(|_| Error::syntax("bad number"))?,
                        )),
                    }
                }
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_' || b[i] == '$') {
                    i += 1;
                }
                out.push(Tok::Word(b[start..i].iter().collect()));
            }
            '?' => {
                i += 1;
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i > start {
                    let n: usize = b[start..i]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .map_err(|_| Error::syntax("bad parameter"))?;
                    out.push(Tok::Param(Some(n)));
                } else {
                    out.push(Tok::Param(None));
                }
            }
            _ => {
                let two: String = b[i..(i + 2).min(b.len())].iter().collect();
                let sym = match two.as_str() {
                    "<=" => Some("<="),
                    ">=" => Some(">="),
                    "<>" => Some("<>"),
                    "!=" => Some("!="),
                    "==" => Some("=="),
                    "||" => Some("||"),
                    "<<" => Some("<<"),
                    ">>" => Some(">>"),
                    _ => None,
                };
                if let Some(s) = sym {
                    out.push(Tok::Sym(s));
                    i += 2;
                    spans.push((start_at, i));
                    continue;
                }
                let s = match c {
                    '(' => "(",
                    ')' => ")",
                    ',' => ",",
                    ';' => ";",
                    '.' => ".",
                    '*' => "*",
                    '+' => "+",
                    '-' => "-",
                    '/' => "/",
                    '%' => "%",
                    '=' => "=",
                    '<' => "<",
                    '>' => ">",
                    '&' => "&",
                    '|' => "|",
                    '~' => "~",
                    _ => return Err(Error::syntax(format!("unexpected character {c:?}"))),
                };
                out.push(Tok::Sym(s));
                i += 1;
            }
        }
        if out.len() > before {
            spans.push((start_at, i));
        }
    }
    out.push(Tok::End);
    spans.push((b.len(), b.len()));
    Ok((out, spans))
}

pub struct Parser {
    toks: Vec<Tok>,
    spans: Vec<(usize, usize)>,
    chars: Vec<char>,
    pos: usize,
    next_param: usize,
    pub max_param: usize,
}

const RESERVED: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "ON",
    "JOIN",
    "LEFT",
    "INNER",
    "CROSS",
    "NATURAL",
    "AND",
    "OR",
    "NOT",
    "AS",
    "SET",
    "VALUES",
    "INTO",
    "DO",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "USING",
    "DEFAULT",
    "CHECK",
    "PRIMARY",
    "UNIQUE",
    "REFERENCES",
    "IS",
    "IN",
    "LIKE",
    "BETWEEN",
    "CASE",
    "NULL",
    "EXISTS",
    "DISTINCT",
    "ALL",
    "BY",
    "CONFLICT",
    "RETURNING",
];

/// Parses one or more statements separated by semicolons.
pub struct Parsed {
    pub stmt: Stmt,
    pub params: usize,
    pub text: String,
}

pub fn parse_script(sql: &str) -> Result<Vec<Parsed>, Error> {
    let (toks, spans) = lex(sql)?;
    let mut p = Parser {
        toks,
        spans,
        chars: sql.chars().collect(),
        pos: 0,
        next_param: 0,
        max_param: 0,
    };
    let mut out = Vec::new();
    loop {
        while p.eat_sym(";") {}
        if p.peek() == &Tok::End {
            break;
        }
        p.next_param = 0;
        p.max_param = 0;
        let first = p.pos;
        let stmt = p.statement()?;
        let text: String = p.chars[p.spans[first].0..p.spans[p.pos - 1].1]
            .iter()
            .collect();
        out.push(Parsed {
            stmt,
            params: p.max_param,
            text,
        });
        if !p.eat_sym(";") && p.peek() != &Tok::End {
            return Err(Error::syntax(format!(
                "unexpected {:?} after statement",
                p.peek()
            )));
        }
    }
    Ok(out)
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }
    fn peek_at(&self, n: usize) -> &Tok {
        self.toks.get(self.pos + n).unwrap_or(&Tok::End)
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Word(w) if w.eq_ignore_ascii_case(kw))
    }
    fn is_kw_at(&self, n: usize, kw: &str) -> bool {
        matches!(self.peek_at(n), Tok::Word(w) if w.eq_ignore_ascii_case(kw))
    }
    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.is_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_kw(&mut self, kw: &str) -> Result<(), Error> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(Error::syntax(format!(
                "expected {kw}, found {:?}",
                self.peek()
            )))
        }
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Tok::Sym(t) if *t == s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_sym(&mut self, s: &str) -> Result<(), Error> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            Err(Error::syntax(format!(
                "expected `{s}`, found {:?}",
                self.peek()
            )))
        }
    }
    fn ident(&mut self) -> Result<String, Error> {
        match self.bump() {
            Tok::Word(w) | Tok::Quoted(w) => Ok(w),
            Tok::Str(s) => Ok(s),
            other => Err(Error::syntax(format!("expected a name, found {other:?}"))),
        }
    }
    fn is_reserved(&self) -> bool {
        matches!(self.peek(), Tok::Word(w) if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(w)))
    }

    // ---- statements ----------------------------------------------------------------------------

    fn statement(&mut self) -> Result<Stmt, Error> {
        let word = match self.peek() {
            Tok::Word(w) => w.to_ascii_uppercase(),
            other => {
                return Err(Error::syntax(format!(
                    "expected a statement, found {other:?}"
                )));
            }
        };
        match word.as_str() {
            "SELECT" | "WITH" | "VALUES" => Ok(Stmt::Select(Box::new(self.select()?))),
            "INSERT" | "REPLACE" => self.insert(),
            "UPDATE" => self.update(),
            "DELETE" => self.delete(),
            "CREATE" => self.create(),
            "DROP" => {
                self.bump();
                self.expect_kw("TABLE")?;
                let if_exists = self.eat_kw("IF") && {
                    self.expect_kw("EXISTS")?;
                    true
                };
                Ok(Stmt::DropTable {
                    name: self.ident()?,
                    if_exists,
                })
            }
            "ALTER" => {
                self.bump();
                self.expect_kw("TABLE")?;
                let table = self.ident()?;
                self.expect_kw("ADD")?;
                self.eat_kw("COLUMN");
                Ok(Stmt::AlterAddColumn {
                    table,
                    column: self.column_def()?,
                })
            }
            "PRAGMA" => {
                self.bump();
                let name = self.ident()?;
                if name.eq_ignore_ascii_case("main") && self.eat_sym(".") {
                    let inner = self.ident()?;
                    return self.pragma_rest(inner);
                }
                self.pragma_rest(name)
            }
            "BEGIN" => {
                self.bump();
                let kind = if self.eat_kw("IMMEDIATE") {
                    BeginKind::Immediate
                } else if self.eat_kw("EXCLUSIVE") {
                    BeginKind::Exclusive
                } else {
                    self.eat_kw("DEFERRED");
                    BeginKind::Deferred
                };
                self.eat_kw("TRANSACTION");
                Ok(Stmt::Begin(kind))
            }
            "COMMIT" | "END" => {
                self.bump();
                self.eat_kw("TRANSACTION");
                Ok(Stmt::Commit)
            }
            "ROLLBACK" => {
                self.bump();
                self.eat_kw("TRANSACTION");
                Ok(Stmt::Rollback)
            }
            other => Err(Error::syntax(format!("unsupported statement {other}"))),
        }
    }

    fn pragma_rest(&mut self, name: String) -> Result<Stmt, Error> {
        let argument = if self.eat_sym("=") {
            Some(Pragma::Set(self.pragma_value()?))
        } else if self.eat_sym("(") {
            let v = self.pragma_value()?;
            self.expect_sym(")")?;
            Some(Pragma::Call(v))
        } else {
            None
        };
        Ok(Stmt::Pragma { name, argument })
    }

    fn pragma_value(&mut self) -> Result<Value, Error> {
        let negative = self.eat_sym("-");
        Ok(match self.bump() {
            Tok::Int(i) => Value::Integer(if negative { -i } else { i }),
            Tok::Real(f) => Value::Real(if negative { -f } else { f }),
            Tok::Str(s) | Tok::Word(s) | Tok::Quoted(s) => Value::Text(s),
            other => return Err(Error::syntax(format!("bad pragma value {other:?}"))),
        })
    }

    // ---- SELECT --------------------------------------------------------------------------------

    pub fn select(&mut self) -> Result<Select, Error> {
        if self.is_kw("WITH") {
            return Err(Error::syntax("WITH is not supported"));
        }
        let mut first = self.select_core()?;
        let mut compound = Vec::new();
        loop {
            let op = if self.eat_kw("UNION") {
                if self.eat_kw("ALL") {
                    Compound::UnionAll
                } else {
                    Compound::Union
                }
            } else if self.eat_kw("INTERSECT") {
                Compound::Intersect
            } else if self.eat_kw("EXCEPT") {
                Compound::Except
            } else {
                break;
            };
            compound.push((op, self.select_core()?));
        }
        first.compound = compound;
        if self.eat_kw("ORDER") {
            self.expect_kw("BY")?;
            first.order_by = self.order_terms()?;
        }
        if self.eat_kw("LIMIT") {
            let first_expr = self.expr()?;
            if self.eat_kw("OFFSET") {
                first.limit = Some(first_expr);
                first.offset = Some(self.expr()?);
            } else if self.eat_sym(",") {
                first.offset = Some(first_expr);
                first.limit = Some(self.expr()?);
            } else {
                first.limit = Some(first_expr);
            }
        }
        Ok(first)
    }

    fn order_terms(&mut self) -> Result<Vec<OrderTerm>, Error> {
        let mut terms = Vec::new();
        loop {
            let expr = self.expr()?;
            let descending = if self.eat_kw("DESC") {
                true
            } else {
                self.eat_kw("ASC");
                false
            };
            terms.push(OrderTerm { expr, descending });
            if !self.eat_sym(",") {
                return Ok(terms);
            }
        }
    }

    fn select_core(&mut self) -> Result<Select, Error> {
        if self.eat_sym("(") {
            let s = self.select()?;
            self.expect_sym(")")?;
            return Ok(s);
        }
        if self.is_kw("VALUES") {
            return Err(Error::syntax("VALUES as a SELECT is not supported"));
        }
        self.expect_kw("SELECT")?;
        let distinct = if self.eat_kw("DISTINCT") {
            true
        } else {
            self.eat_kw("ALL");
            false
        };
        let mut columns = Vec::new();
        loop {
            if self.eat_sym("*") {
                columns.push(ResultColumn::Star);
            } else if matches!(self.peek(), Tok::Word(_) | Tok::Quoted(_))
                && self.peek_at(1) == &Tok::Sym(".")
                && self.peek_at(2) == &Tok::Sym("*")
            {
                let t = self.ident()?;
                self.bump();
                self.bump();
                columns.push(ResultColumn::TableStar(t));
            } else {
                let expr = self.expr()?;
                let alias = if self.eat_kw("AS")
                    || (matches!(self.peek(), Tok::Word(_) | Tok::Quoted(_) | Tok::Str(_))
                        && !self.is_reserved())
                {
                    Some(self.ident()?)
                } else {
                    None
                };
                columns.push(ResultColumn::Expr { expr, alias });
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        let from = if self.eat_kw("FROM") {
            Some(self.from()?)
        } else {
            None
        };
        let where_ = if self.eat_kw("WHERE") {
            Some(self.expr()?)
        } else {
            None
        };
        let mut group_by = Vec::new();
        if self.eat_kw("GROUP") {
            self.expect_kw("BY")?;
            loop {
                group_by.push(self.expr()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        let having = if self.eat_kw("HAVING") {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(Select {
            distinct,
            columns,
            from,
            where_,
            group_by,
            having,
            compound: Vec::new(),
            order_by: Vec::new(),
            limit: None,
            offset: None,
        })
    }

    fn table_ref(&mut self) -> Result<TableRef, Error> {
        if self.eat_sym("(") {
            let select = Box::new(self.select()?);
            self.expect_sym(")")?;
            let alias = self.alias()?;
            return Ok(TableRef::Subquery { select, alias });
        }
        let name = self.ident()?;
        let alias = self.alias()?;
        Ok(TableRef::Table { name, alias })
    }

    fn alias(&mut self) -> Result<Option<String>, Error> {
        if self.eat_kw("AS") {
            return Ok(Some(self.ident()?));
        }
        if matches!(self.peek(), Tok::Word(_) | Tok::Quoted(_)) && !self.is_reserved() {
            return Ok(Some(self.ident()?));
        }
        Ok(None)
    }

    fn from(&mut self) -> Result<From, Error> {
        let first = self.table_ref()?;
        let mut joins = Vec::new();
        loop {
            let kind = if self.eat_sym(",") {
                Some(JoinKind::Cross)
            } else if self.eat_kw("JOIN") {
                Some(JoinKind::Inner)
            } else if self.is_kw("INNER") {
                self.bump();
                self.expect_kw("JOIN")?;
                Some(JoinKind::Inner)
            } else if self.is_kw("CROSS") {
                self.bump();
                self.expect_kw("JOIN")?;
                Some(JoinKind::Cross)
            } else if self.is_kw("LEFT") {
                self.bump();
                self.eat_kw("OUTER");
                self.expect_kw("JOIN")?;
                Some(JoinKind::Left)
            } else {
                None
            };
            let Some(kind) = kind else {
                return Ok(From { first, joins });
            };
            let table = self.table_ref()?;
            let on = if self.eat_kw("ON") {
                Some(self.expr()?)
            } else {
                None
            };
            joins.push(Join { kind, table, on });
        }
    }

    // ---- expressions ---------------------------------------------------------------------------

    pub fn expr(&mut self) -> Result<Expr, Error> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Expr, Error> {
        let mut left = self.and_expr()?;
        while self.eat_kw("OR") {
            let right = self.and_expr()?;
            left = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Expr, Error> {
        let mut left = self.not_expr()?;
        while self.eat_kw("AND") {
            let right = self.not_expr()?;
            left = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn not_expr(&mut self) -> Result<Expr, Error> {
        if self.is_kw("NOT") && !self.is_kw_at(1, "EXISTS") {
            self.bump();
            let operand = self.not_expr()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            });
        }
        self.equality()
    }

    fn equality(&mut self) -> Result<Expr, Error> {
        let mut left = self.comparison()?;
        loop {
            let op = match self.peek() {
                Tok::Sym("=") | Tok::Sym("==") => Some(BinaryOp::Eq),
                Tok::Sym("!=") | Tok::Sym("<>") => Some(BinaryOp::Ne),
                _ => None,
            };
            if let Some(op) = op {
                self.bump();
                let right = self.comparison()?;
                left = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
                continue;
            }
            if self.is_kw("IS") {
                self.bump();
                let negate = self.eat_kw("NOT");
                if self.eat_kw("NULL") {
                    left = Expr::IsNull {
                        expr: Box::new(left),
                        negate,
                    };
                } else {
                    let right = self.comparison()?;
                    left = Expr::Binary {
                        op: if negate {
                            BinaryOp::IsNot
                        } else {
                            BinaryOp::Is
                        },
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                continue;
            }
            let negate = self.is_kw("NOT")
                && (self.is_kw_at(1, "IN")
                    || self.is_kw_at(1, "LIKE")
                    || self.is_kw_at(1, "BETWEEN")
                    || self.is_kw_at(1, "NULL"));
            if negate {
                self.bump();
            }
            if self.is_kw("NOTNULL") {
                self.bump();
                left = Expr::IsNull {
                    expr: Box::new(left),
                    negate: true,
                };
                continue;
            }
            if self.eat_kw("ISNULL") {
                left = Expr::IsNull {
                    expr: Box::new(left),
                    negate: false,
                };
                continue;
            }
            if negate && self.eat_kw("NULL") {
                left = Expr::IsNull {
                    expr: Box::new(left),
                    negate: true,
                };
                continue;
            }
            if self.eat_kw("IN") {
                self.expect_sym("(")?;
                if self.is_kw("SELECT") {
                    let select = Box::new(self.select()?);
                    self.expect_sym(")")?;
                    left = Expr::InSelect {
                        expr: Box::new(left),
                        select,
                        negate,
                    };
                } else {
                    let mut list = Vec::new();
                    if !self.eat_sym(")") {
                        loop {
                            list.push(self.expr()?);
                            if !self.eat_sym(",") {
                                break;
                            }
                        }
                        self.expect_sym(")")?;
                    }
                    left = Expr::InList {
                        expr: Box::new(left),
                        list,
                        negate,
                    };
                }
                continue;
            }
            if self.eat_kw("LIKE") {
                let pattern = self.comparison()?;
                left = Expr::Like {
                    expr: Box::new(left),
                    pattern: Box::new(pattern),
                    negate,
                };
                continue;
            }
            if self.eat_kw("BETWEEN") {
                let low = self.comparison()?;
                self.expect_kw("AND")?;
                let high = self.comparison()?;
                left = Expr::Between {
                    expr: Box::new(left),
                    low: Box::new(low),
                    high: Box::new(high),
                    negate,
                };
                continue;
            }
            if negate {
                return Err(Error::syntax("expected IN, LIKE or BETWEEN after NOT"));
            }
            return Ok(left);
        }
    }

    fn comparison(&mut self) -> Result<Expr, Error> {
        let mut left = self.bitwise()?;
        loop {
            let op = match self.peek() {
                Tok::Sym("<") => BinaryOp::Lt,
                Tok::Sym("<=") => BinaryOp::Le,
                Tok::Sym(">") => BinaryOp::Gt,
                Tok::Sym(">=") => BinaryOp::Ge,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.bitwise()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn bitwise(&mut self) -> Result<Expr, Error> {
        let mut left = self.additive()?;
        loop {
            let op = match self.peek() {
                Tok::Sym("&") => BinaryOp::BitAnd,
                Tok::Sym("|") => BinaryOp::BitOr,
                Tok::Sym("<<") => BinaryOp::ShiftLeft,
                Tok::Sym(">>") => BinaryOp::ShiftRight,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.additive()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn additive(&mut self) -> Result<Expr, Error> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Sym("+") => BinaryOp::Add,
                Tok::Sym("-") => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.multiplicative()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, Error> {
        let mut left = self.concat()?;
        loop {
            let op = match self.peek() {
                Tok::Sym("*") => BinaryOp::Mul,
                Tok::Sym("/") => BinaryOp::Div,
                Tok::Sym("%") => BinaryOp::Rem,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.concat()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn concat(&mut self) -> Result<Expr, Error> {
        let mut left = self.unary()?;
        while self.eat_sym("||") {
            let right = self.unary()?;
            left = Expr::Binary {
                op: BinaryOp::Concat,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, Error> {
        if self.eat_sym("-") {
            let operand = self.unary()?;
            return Ok(match operand {
                Expr::Literal(Value::Integer(i)) => Expr::Literal(Value::Integer(i.wrapping_neg())),
                Expr::Literal(Value::Real(f)) => Expr::Literal(Value::Real(-f)),
                other => Expr::Unary {
                    op: UnaryOp::Negate,
                    operand: Box::new(other),
                },
            });
        }
        if self.eat_sym("+") {
            let operand = self.unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Plus,
                operand: Box::new(operand),
            });
        }
        if self.eat_sym("~") {
            let operand = self.unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::BitNot,
                operand: Box::new(operand),
            });
        }
        self.primary()
    }

    fn param(&mut self, n: Option<usize>) -> Expr {
        let index = match n {
            Some(n) => n,
            None => {
                self.next_param += 1;
                self.next_param
            }
        };
        self.max_param = self.max_param.max(index);
        Expr::Param(index)
    }

    fn primary(&mut self) -> Result<Expr, Error> {
        match self.bump() {
            Tok::Int(i) => Ok(Expr::Literal(Value::Integer(i))),
            Tok::Real(f) => Ok(Expr::Literal(Value::Real(f))),
            Tok::Str(s) => Ok(Expr::Literal(Value::Text(s))),
            Tok::Blob(b) => Ok(Expr::Literal(Value::Blob(b))),
            Tok::Param(n) => Ok(self.param(n)),
            Tok::Sym("(") => {
                if self.is_kw("SELECT") {
                    let s = self.select()?;
                    self.expect_sym(")")?;
                    return Ok(Expr::Subquery(Box::new(s)));
                }
                let e = self.expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Tok::Word(w) => {
                let upper = w.to_ascii_uppercase();
                match upper.as_str() {
                    "NULL" => return Ok(Expr::Literal(Value::Null)),
                    "TRUE" => return Ok(Expr::Literal(Value::Integer(1))),
                    "FALSE" => return Ok(Expr::Literal(Value::Integer(0))),
                    "CURRENT_TIMESTAMP" | "CURRENT_DATE" | "CURRENT_TIME" => {
                        return Err(Error::syntax("current time keywords are not supported"));
                    }
                    "EXISTS" => {
                        self.expect_sym("(")?;
                        let s = self.select()?;
                        self.expect_sym(")")?;
                        return Ok(Expr::Exists(Box::new(s)));
                    }
                    "NOT" if self.is_kw("EXISTS") => {
                        self.bump();
                        self.expect_sym("(")?;
                        let s = self.select()?;
                        self.expect_sym(")")?;
                        return Ok(Expr::Unary {
                            op: UnaryOp::Not,
                            operand: Box::new(Expr::Exists(Box::new(s))),
                        });
                    }
                    "CASE" => return self.case_expr(),
                    "CAST" => {
                        self.expect_sym("(")?;
                        let expr = self.expr()?;
                        self.expect_kw("AS")?;
                        let mut to = self.ident()?;
                        while matches!(self.peek(), Tok::Word(_)) {
                            to.push(' ');
                            to.push_str(&self.ident()?);
                        }
                        self.expect_sym(")")?;
                        return Ok(Expr::Cast {
                            expr: Box::new(expr),
                            to,
                        });
                    }
                    _ => {}
                }
                self.name_or_call(w)
            }
            Tok::Quoted(w) => self.name_or_call(w),
            other => Err(Error::syntax(format!("unexpected {other:?} in expression"))),
        }
    }

    fn name_or_call(&mut self, name: String) -> Result<Expr, Error> {
        if self.eat_sym("(") {
            let mut distinct = false;
            let mut star = false;
            let mut args = Vec::new();
            if self.eat_sym("*") {
                star = true;
            } else if !matches!(self.peek(), Tok::Sym(")")) {
                distinct = self.eat_kw("DISTINCT");
                loop {
                    args.push(self.expr()?);
                    if !self.eat_sym(",") {
                        break;
                    }
                }
            }
            self.expect_sym(")")?;
            return Ok(Expr::Function {
                name: name.to_ascii_lowercase(),
                args,
                star,
                distinct,
            });
        }
        if self.eat_sym(".") {
            let column = self.ident()?;
            return Ok(Expr::Column {
                table: Some(name),
                name: column,
            });
        }
        Ok(Expr::Column { table: None, name })
    }

    fn case_expr(&mut self) -> Result<Expr, Error> {
        let operand = if self.is_kw("WHEN") {
            None
        } else {
            Some(Box::new(self.expr()?))
        };
        let mut branches = Vec::new();
        while self.eat_kw("WHEN") {
            let when = self.expr()?;
            self.expect_kw("THEN")?;
            branches.push((when, self.expr()?));
        }
        let otherwise = if self.eat_kw("ELSE") {
            Some(Box::new(self.expr()?))
        } else {
            None
        };
        self.expect_kw("END")?;
        Ok(Expr::Case {
            operand,
            branches,
            otherwise,
        })
    }

    // ---- DML -----------------------------------------------------------------------------------

    fn insert(&mut self) -> Result<Stmt, Error> {
        let mut or = OnConflict::Abort;
        if self.eat_kw("REPLACE") {
            or = OnConflict::Replace;
        } else {
            self.expect_kw("INSERT")?;
            if self.eat_kw("OR") {
                or = if self.eat_kw("REPLACE") {
                    OnConflict::Replace
                } else if self.eat_kw("IGNORE") {
                    OnConflict::Ignore
                } else if self.eat_kw("ABORT") {
                    OnConflict::Abort
                } else {
                    return Err(Error::syntax("unsupported INSERT OR conflict action"));
                };
            }
        }
        self.expect_kw("INTO")?;
        let table = self.ident()?;
        let mut columns = None;
        if self.eat_sym("(") {
            let mut names = Vec::new();
            loop {
                names.push(self.ident()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
            self.expect_sym(")")?;
            columns = Some(names);
        }
        let source = if self.eat_kw("VALUES") {
            let mut rows = Vec::new();
            loop {
                self.expect_sym("(")?;
                let mut row = Vec::new();
                loop {
                    row.push(self.expr()?);
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym(")")?;
                rows.push(row);
                if !self.eat_sym(",") {
                    break;
                }
            }
            InsertSource::Values(rows)
        } else if self.eat_kw("DEFAULT") {
            self.expect_kw("VALUES")?;
            InsertSource::DefaultValues
        } else if self.is_kw("SELECT") {
            InsertSource::Select(Box::new(self.select()?))
        } else {
            return Err(Error::syntax("expected VALUES or SELECT"));
        };
        let upsert = if self.is_kw("ON") && self.is_kw_at(1, "CONFLICT") {
            self.bump();
            self.bump();
            let mut target = Vec::new();
            if self.eat_sym("(") {
                loop {
                    target.push(self.ident()?);
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym(")")?;
            }
            self.expect_kw("DO")?;
            let action = if self.eat_kw("NOTHING") {
                UpsertAction::Nothing
            } else {
                self.expect_kw("UPDATE")?;
                self.expect_kw("SET")?;
                let sets = self.assignments()?;
                let where_ = if self.eat_kw("WHERE") {
                    Some(self.expr()?)
                } else {
                    None
                };
                UpsertAction::Update { sets, where_ }
            };
            Some(Upsert { target, action })
        } else {
            None
        };
        Ok(Stmt::Insert(Insert {
            table,
            columns,
            source,
            or,
            upsert,
        }))
    }

    fn assignments(&mut self) -> Result<Vec<(String, Expr)>, Error> {
        let mut sets = Vec::new();
        loop {
            let column = self.ident()?;
            self.expect_sym("=")?;
            sets.push((column, self.expr()?));
            if !self.eat_sym(",") {
                return Ok(sets);
            }
        }
    }

    fn update(&mut self) -> Result<Stmt, Error> {
        self.expect_kw("UPDATE")?;
        let mut or = OnConflict::Abort;
        if self.eat_kw("OR") {
            or = if self.eat_kw("REPLACE") {
                OnConflict::Replace
            } else if self.eat_kw("IGNORE") {
                OnConflict::Ignore
            } else {
                self.expect_kw("ABORT")?;
                OnConflict::Abort
            };
        }
        let table = self.ident()?;
        self.expect_kw("SET")?;
        let sets = self.assignments()?;
        let where_ = if self.eat_kw("WHERE") {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(Stmt::Update(Update {
            table,
            sets,
            where_,
            or,
        }))
    }

    fn delete(&mut self) -> Result<Stmt, Error> {
        self.expect_kw("DELETE")?;
        self.expect_kw("FROM")?;
        let table = self.ident()?;
        let where_ = if self.eat_kw("WHERE") {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(Stmt::Delete(Delete { table, where_ }))
    }

    // ---- DDL -----------------------------------------------------------------------------------

    fn create(&mut self) -> Result<Stmt, Error> {
        self.expect_kw("CREATE")?;
        let unique = self.eat_kw("UNIQUE");
        if self.eat_kw("INDEX") {
            let if_not_exists = self.eat_kw("IF") && {
                self.expect_kw("NOT")?;
                self.expect_kw("EXISTS")?;
                true
            };
            let name = self.ident()?;
            self.expect_kw("ON")?;
            let table = self.ident()?;
            self.expect_sym("(")?;
            let mut columns = Vec::new();
            loop {
                columns.push(self.ident()?);
                self.eat_kw("ASC");
                self.eat_kw("DESC");
                if !self.eat_sym(",") {
                    break;
                }
            }
            self.expect_sym(")")?;
            return Ok(Stmt::CreateIndex(CreateIndex {
                name,
                table,
                columns,
                unique,
                if_not_exists,
            }));
        }
        if unique {
            return Err(Error::syntax("expected INDEX after UNIQUE"));
        }
        self.expect_kw("TABLE")?;
        let if_not_exists = self.eat_kw("IF") && {
            self.expect_kw("NOT")?;
            self.expect_kw("EXISTS")?;
            true
        };
        let name = self.ident()?;
        self.expect_sym("(")?;
        let mut columns = Vec::new();
        let mut constraints = Vec::new();
        loop {
            if self.is_kw("PRIMARY")
                || self.is_kw("UNIQUE")
                || self.is_kw("CHECK")
                || self.is_kw("FOREIGN")
                || self.is_kw("CONSTRAINT")
            {
                if self.eat_kw("CONSTRAINT") {
                    self.ident()?;
                }
                constraints.push(self.table_constraint()?);
            } else {
                columns.push(self.column_def()?);
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        Ok(Stmt::CreateTable(CreateTable {
            name,
            if_not_exists,
            columns,
            constraints,
        }))
    }

    fn name_list(&mut self) -> Result<Vec<String>, Error> {
        self.expect_sym("(")?;
        let mut names = Vec::new();
        loop {
            names.push(self.ident()?);
            self.eat_kw("ASC");
            self.eat_kw("DESC");
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        Ok(names)
    }

    fn table_constraint(&mut self) -> Result<TableConstraint, Error> {
        if self.eat_kw("PRIMARY") {
            self.expect_kw("KEY")?;
            return Ok(TableConstraint::PrimaryKey(self.name_list()?));
        }
        if self.eat_kw("UNIQUE") {
            return Ok(TableConstraint::Unique(self.name_list()?));
        }
        if self.eat_kw("CHECK") {
            self.expect_sym("(")?;
            let e = self.expr()?;
            self.expect_sym(")")?;
            return Ok(TableConstraint::Check(e));
        }
        self.expect_kw("FOREIGN")?;
        self.expect_kw("KEY")?;
        let columns = self.name_list()?;
        self.expect_kw("REFERENCES")?;
        Ok(TableConstraint::ForeignKey {
            columns,
            target: self.foreign_target()?,
        })
    }

    fn foreign_target(&mut self) -> Result<ForeignKey, Error> {
        let table = self.ident()?;
        let columns = if matches!(self.peek(), Tok::Sym("(")) {
            self.name_list()?
        } else {
            Vec::new()
        };
        // ON DELETE / ON UPDATE actions are accepted and ignored (NO ACTION semantics).
        while self.is_kw("ON") && (self.is_kw_at(1, "DELETE") || self.is_kw_at(1, "UPDATE")) {
            self.bump();
            self.bump();
            if self.eat_kw("SET") {
                self.bump();
            } else if self.eat_kw("NO") {
                self.expect_kw("ACTION")?;
            } else {
                self.bump();
            }
        }
        Ok(ForeignKey { table, columns })
    }

    fn column_def(&mut self) -> Result<ColumnDef, Error> {
        let name = self.ident()?;
        let mut declared_type = String::new();
        while matches!(self.peek(), Tok::Word(w) if !is_constraint_word(w)) {
            if !declared_type.is_empty() {
                declared_type.push(' ');
            }
            declared_type.push_str(&self.ident()?);
        }
        if self.eat_sym("(") {
            declared_type.push('(');
            while !self.eat_sym(")") {
                match self.bump() {
                    Tok::Int(i) => declared_type.push_str(&i.to_string()),
                    Tok::Sym(s) => declared_type.push_str(s),
                    Tok::End => return Err(Error::syntax("unterminated type")),
                    _ => {}
                }
            }
            declared_type.push(')');
        }
        let mut def = ColumnDef {
            name,
            declared_type,
            not_null: false,
            primary_key: false,
            autoincrement: false,
            unique: false,
            default: None,
            check: None,
            references: None,
        };
        loop {
            if self.eat_kw("CONSTRAINT") {
                self.ident()?;
                continue;
            }
            if self.eat_kw("PRIMARY") {
                self.expect_kw("KEY")?;
                def.primary_key = true;
                self.eat_kw("ASC");
                self.eat_kw("DESC");
                if self.eat_kw("AUTOINCREMENT") {
                    def.autoincrement = true;
                }
            } else if self.eat_kw("NOT") {
                self.expect_kw("NULL")?;
                def.not_null = true;
            } else if self.eat_kw("NULL") {
            } else if self.eat_kw("UNIQUE") {
                def.unique = true;
            } else if self.eat_kw("DEFAULT") {
                def.default = Some(if self.eat_sym("(") {
                    let e = self.expr()?;
                    self.expect_sym(")")?;
                    e
                } else {
                    self.unary()?
                });
            } else if self.eat_kw("CHECK") {
                self.expect_sym("(")?;
                def.check = Some(self.expr()?);
                self.expect_sym(")")?;
            } else if self.eat_kw("REFERENCES") {
                def.references = Some(self.foreign_target()?);
            } else {
                return Ok(def);
            }
        }
    }
}

fn is_constraint_word(w: &str) -> bool {
    [
        "PRIMARY",
        "NOT",
        "NULL",
        "UNIQUE",
        "DEFAULT",
        "CHECK",
        "REFERENCES",
        "CONSTRAINT",
        "COLLATE",
        "AUTOINCREMENT",
    ]
    .iter()
    .any(|k| k.eq_ignore_ascii_case(w))
}
