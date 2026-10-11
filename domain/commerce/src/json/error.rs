use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// The text is not JSON.
    Syntax,
    /// The text is JSON, the value is not what the type needs.
    Data,
    /// The text ended inside a value.
    Eof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    category: Category,
    message: String,
    position: Option<(usize, usize)>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn syntax(message: impl Into<String>, line: usize, column: usize) -> Error {
        Error {
            category: Category::Syntax,
            message: message.into(),
            position: Some((line, column)),
        }
    }

    pub(crate) fn eof(message: impl Into<String>, line: usize, column: usize) -> Error {
        Error {
            category: Category::Eof,
            message: message.into(),
            position: Some((line, column)),
        }
    }

    /// An error about the shape of a value, as typed decoding reports it.
    pub fn custom(message: impl fmt::Display) -> Error {
        Error {
            category: Category::Data,
            message: message.to_string(),
            position: None,
        }
    }

    pub fn classify(&self) -> Category {
        self.category
    }
    pub fn is_syntax(&self) -> bool {
        self.category == Category::Syntax
    }
    pub fn is_data(&self) -> bool {
        self.category == Category::Data
    }
    pub fn is_eof(&self) -> bool {
        self.category == Category::Eof
    }
    /// 1-based line of a syntax error, 0 when the error has no position.
    pub fn line(&self) -> usize {
        self.position.map_or(0, |p| p.0)
    }
    /// 1-based column of a syntax error, 0 when the error has no position.
    pub fn column(&self) -> usize {
        self.position.map_or(0, |p| p.1)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some((line, column)) if line > 0 => {
                write!(f, "{} at line {line} column {column}", self.message)
            }
            _ => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for Error {}
