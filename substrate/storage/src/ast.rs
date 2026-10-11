//! The syntax tree of the supported SQL.

use crate::value::Value;

#[derive(Clone, Debug)]
pub enum Stmt {
    Select(Box<Select>),
    Insert(Insert),
    Update(Update),
    Delete(Delete),
    CreateTable(CreateTable),
    CreateIndex(CreateIndex),
    DropTable {
        name: String,
        if_exists: bool,
    },
    AlterAddColumn {
        table: String,
        column: ColumnDef,
    },
    Pragma {
        name: String,
        argument: Option<Pragma>,
    },
    Begin(BeginKind),
    Commit,
    Rollback,
}

#[derive(Clone, Debug)]
pub enum Pragma {
    Set(Value),
    Call(Value),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeginKind {
    Deferred,
    Immediate,
    Exclusive,
}

#[derive(Clone, Debug)]
pub struct Select {
    pub distinct: bool,
    pub columns: Vec<ResultColumn>,
    pub from: Option<From>,
    pub where_: Option<Expr>,
    pub group_by: Vec<Expr>,
    pub having: Option<Expr>,
    pub compound: Vec<(Compound, Select)>,
    pub order_by: Vec<OrderTerm>,
    pub limit: Option<Expr>,
    pub offset: Option<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compound {
    Union,
    UnionAll,
    Intersect,
    Except,
}

#[derive(Clone, Debug)]
pub enum ResultColumn {
    Star,
    TableStar(String),
    Expr { expr: Expr, alias: Option<String> },
}

#[derive(Clone, Debug)]
pub struct From {
    pub first: TableRef,
    pub joins: Vec<Join>,
}

#[derive(Clone, Debug)]
pub enum TableRef {
    Table {
        name: String,
        alias: Option<String>,
    },
    Subquery {
        select: Box<Select>,
        alias: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
    Cross,
}

#[derive(Clone, Debug)]
pub struct Join {
    pub kind: JoinKind,
    pub table: TableRef,
    pub on: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct OrderTerm {
    pub expr: Expr,
    pub descending: bool,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Literal(Value),
    Param(usize),
    Column {
        table: Option<String>,
        name: String,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negate: bool,
    },
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negate: bool,
    },
    InSelect {
        expr: Box<Expr>,
        select: Box<Select>,
        negate: bool,
    },
    Like {
        expr: Box<Expr>,
        pattern: Box<Expr>,
        negate: bool,
    },
    IsNull {
        expr: Box<Expr>,
        negate: bool,
    },
    Case {
        operand: Option<Box<Expr>>,
        branches: Vec<(Expr, Expr)>,
        otherwise: Option<Box<Expr>>,
    },
    Function {
        name: String,
        args: Vec<Expr>,
        star: bool,
        distinct: bool,
    },
    Exists(Box<Select>),
    Subquery(Box<Select>),
    Cast {
        expr: Box<Expr>,
        to: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
    Plus,
    Not,
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Is,
    IsNot,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Concat,
    BitAnd,
    BitOr,
    ShiftLeft,
    ShiftRight,
}

#[derive(Clone, Debug)]
pub struct ColumnDef {
    pub name: String,
    pub declared_type: String,
    pub not_null: bool,
    pub primary_key: bool,
    pub autoincrement: bool,
    pub unique: bool,
    pub default: Option<Expr>,
    pub check: Option<Expr>,
    pub references: Option<ForeignKey>,
}

#[derive(Clone, Debug)]
pub struct ForeignKey {
    pub table: String,
    pub columns: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum TableConstraint {
    PrimaryKey(Vec<String>),
    Unique(Vec<String>),
    Check(Expr),
    ForeignKey {
        columns: Vec<String>,
        target: ForeignKey,
    },
}

#[derive(Clone, Debug)]
pub struct CreateTable {
    pub name: String,
    pub if_not_exists: bool,
    pub columns: Vec<ColumnDef>,
    pub constraints: Vec<TableConstraint>,
}

#[derive(Clone, Debug)]
pub struct CreateIndex {
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub if_not_exists: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnConflict {
    Abort,
    Replace,
    Ignore,
}

#[derive(Clone, Debug)]
pub enum InsertSource {
    Values(Vec<Vec<Expr>>),
    Select(Box<Select>),
    DefaultValues,
}

#[derive(Clone, Debug)]
pub struct Upsert {
    pub target: Vec<String>,
    pub action: UpsertAction,
}

#[derive(Clone, Debug)]
pub enum UpsertAction {
    Nothing,
    Update {
        sets: Vec<(String, Expr)>,
        where_: Option<Expr>,
    },
}

#[derive(Clone, Debug)]
pub struct Insert {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub source: InsertSource,
    pub or: OnConflict,
    pub upsert: Option<Upsert>,
}

#[derive(Clone, Debug)]
pub struct Update {
    pub table: String,
    pub sets: Vec<(String, Expr)>,
    pub where_: Option<Expr>,
    pub or: OnConflict,
}

#[derive(Clone, Debug)]
pub struct Delete {
    pub table: String,
    pub where_: Option<Expr>,
}
