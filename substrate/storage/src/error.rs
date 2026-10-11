use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
/// The class of a failure. `Syntax` stands for every plain SQL error (SQLite's generic error code):
/// a syntax error, an unknown table or column, a misused transaction statement, an overflow.
/// `Type` is a datatype mismatch.
pub enum ErrorKind {
    Syntax,
    Constraint,
    NoSuchTable,
    NoSuchColumn,
    Misuse,
    Busy,
    Io,
    Corrupt,
    Type,
    QueryReturnedNoRows,
    InvalidParameterCount,
    /// A column read as a Rust type its storage class cannot convert to.
    ColumnType,
    /// A column index past the last column.
    ColumnIndex,
    /// A column name the row does not have.
    ColumnName,
    /// A stored integer outside the range of the requested Rust integer type.
    OutOfRange,
    /// A Rust value that cannot be stored (an unsigned integer above i64::MAX).
    ToSql,
    /// `execute` ran a statement that produced rows.
    ExecuteReturnedResults,
    /// A single-statement call was given a script.
    MultipleStatement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error {
            kind,
            message: message.into(),
        }
    }
    pub fn syntax(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Syntax, message)
    }
    pub fn constraint(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Constraint, message)
    }
    pub fn misuse(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Misuse, message)
    }
    pub fn io(e: std::io::Error) -> Error {
        Error::new(ErrorKind::Io, e.to_string())
    }
    pub fn is_constraint(&self) -> bool {
        self.kind == ErrorKind::Constraint
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::io(e)
    }
}
