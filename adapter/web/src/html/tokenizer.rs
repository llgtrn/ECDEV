//! The HTML tokenizer of the WHATWG HTML Standard (13.2.5), as a pull-style state machine.
//!
//! The tree builder owns the tokenizer: it pulls one token at a time, may switch the content model
//! (RCDATA, RAWTEXT, script data, PLAINTEXT) after a start tag, and tells the tokenizer whether a
//! CDATA section is allowed (the adjusted current node is foreign content).

use std::{collections::HashMap, collections::VecDeque, sync::OnceLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Doctype {
        name: Option<String>,
        public_id: Option<String>,
        system_id: Option<String>,
        force_quirks: bool,
    },
    StartTag {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    EndTag {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    Comment(String),
    ProcessingInstruction {
        target: String,
        data: String,
    },
    Character(char),
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    TagOpen,
    EndTagOpen,
    TagName,
    RcdataLessThan,
    RcdataEndTagOpen,
    RcdataEndTagName,
    RawtextLessThan,
    RawtextEndTagOpen,
    RawtextEndTagName,
    ScriptDataLessThan,
    ScriptDataEndTagOpen,
    ScriptDataEndTagName,
    ScriptDataEscapeStart,
    ScriptDataEscapeStartDash,
    ScriptDataEscaped,
    ScriptDataEscapedDash,
    ScriptDataEscapedDashDash,
    ScriptDataEscapedLessThan,
    ScriptDataEscapedEndTagOpen,
    ScriptDataEscapedEndTagName,
    ScriptDataDoubleEscapeStart,
    ScriptDataDoubleEscaped,
    ScriptDataDoubleEscapedDash,
    ScriptDataDoubleEscapedDashDash,
    ScriptDataDoubleEscapedLessThan,
    ScriptDataDoubleEscapeEnd,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThan,
    CommentLessThanBang,
    CommentLessThanBangDash,
    CommentLessThanBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicIdentifier,
    DoctypePublicIdentifierDoubleQuoted,
    DoctypePublicIdentifierSingleQuoted,
    AfterDoctypePublicIdentifier,
    BetweenDoctypePublicAndSystemIdentifiers,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemIdentifier,
    DoctypeSystemIdentifierDoubleQuoted,
    DoctypeSystemIdentifierSingleQuoted,
    AfterDoctypeSystemIdentifier,
    BogusDoctype,
    CdataSection,
    CdataSectionBracket,
    CdataSectionEnd,
    ProcessingInstructionOpen,
    ProcessingInstructionTarget,
    AfterProcessingInstructionTarget,
    ProcessingInstructionData,
    ProcessingInstructionQuestionable,
    CharacterReference,
    NamedCharacterReference,
    AmbiguousAmpersand,
    NumericCharacterReference,
    HexadecimalCharacterReferenceStart,
    HexadecimalCharacterReference,
    DecimalCharacterReference,
    NumericCharacterReferenceEnd,
}

/// The named character references of the standard (2231 names, with and without the semicolon).
fn entities() -> &'static HashMap<&'static str, String> {
    static TABLE: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("entities.tsv")
            .lines()
            .filter_map(|line| {
                let (name, chars) = line.split_once('\t')?;
                let mut out = String::new();
                let mut it = chars.chars();
                while let Some(c) = it.next() {
                    if c == '\\' {
                        match it.next() {
                            Some('n') => out.push('\n'),
                            Some('t') => out.push('\t'),
                            Some(other) => out.push(other),
                            None => {}
                        }
                    } else {
                        out.push(c);
                    }
                }
                Some((name, out))
            })
            .collect()
    })
}

const WINDOWS_1252: &[(u32, u32)] = &[
    (0x80, 0x20AC),
    (0x82, 0x201A),
    (0x83, 0x0192),
    (0x84, 0x201E),
    (0x85, 0x2026),
    (0x86, 0x2020),
    (0x87, 0x2021),
    (0x88, 0x02C6),
    (0x89, 0x2030),
    (0x8A, 0x0160),
    (0x8B, 0x2039),
    (0x8C, 0x0152),
    (0x8E, 0x017D),
    (0x91, 0x2018),
    (0x92, 0x2019),
    (0x93, 0x201C),
    (0x94, 0x201D),
    (0x95, 0x2022),
    (0x96, 0x2013),
    (0x97, 0x2014),
    (0x98, 0x02DC),
    (0x99, 0x2122),
    (0x9A, 0x0161),
    (0x9B, 0x203A),
    (0x9C, 0x0153),
    (0x9E, 0x017E),
    (0x9F, 0x0178),
];

#[derive(Default)]
struct TagBuild {
    name: String,
    attrs: Vec<(String, String)>,
    self_closing: bool,
    end: bool,
    /// The attribute being built was a duplicate and is dropped (its value is discarded).
    dropped_attr: bool,
}

#[derive(Default)]
struct DoctypeBuild {
    name: Option<String>,
    public_id: Option<String>,
    system_id: Option<String>,
    force_quirks: bool,
}

pub struct Tokenizer {
    input: Vec<char>,
    pos: usize,
    pub state: State,
    return_state: State,
    temp: String,
    tag: TagBuild,
    comment: String,
    doctype: DoctypeBuild,
    pi_target: String,
    pi_data: String,
    last_start_tag: String,
    char_ref_code: u32,
    pending: VecDeque<Token>,
    finished: bool,
    /// Set by the tree builder: a CDATA section is allowed (the adjusted current node is foreign).
    pub allow_cdata: bool,
}

fn preprocess(input: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | ' ')
}

impl Tokenizer {
    pub fn new(input: &str) -> Tokenizer {
        Tokenizer {
            input: preprocess(input),
            pos: 0,
            state: State::Data,
            return_state: State::Data,
            temp: String::new(),
            tag: TagBuild::default(),
            comment: String::new(),
            doctype: DoctypeBuild::default(),
            pi_target: String::new(),
            pi_data: String::new(),
            last_start_tag: String::new(),
            char_ref_code: 0,
            pending: VecDeque::new(),
            finished: false,
            allow_cdata: false,
        }
    }

    /// Lets the tokenizer know the name of the last start tag it emitted (fragment parsing and tests).
    pub fn set_last_start_tag(&mut self, name: &str) {
        self.last_start_tag = name.to_string();
    }

    pub fn next_token(&mut self) -> Token {
        loop {
            if let Some(t) = self.pending.pop_front() {
                return t;
            }
            if self.finished {
                return Token::Eof;
            }
            self.step();
        }
    }

    fn consume(&mut self) -> Option<char> {
        let c = self.input.get(self.pos).copied();
        self.pos += 1;
        c
    }

    fn reconsume(&mut self) {
        self.pos -= 1;
    }

    fn peek_matches(&self, text: &str, ignore_case: bool) -> bool {
        let mut at = self.pos;
        for expected in text.chars() {
            match self.input.get(at) {
                Some(&c) if c == expected || (ignore_case && c.eq_ignore_ascii_case(&expected)) => {
                    at += 1
                }
                _ => return false,
            }
        }
        true
    }

    fn emit(&mut self, t: Token) {
        self.pending.push_back(t);
    }

    fn emit_char(&mut self, c: char) {
        self.pending.push_back(Token::Character(c));
    }

    fn emit_eof(&mut self) {
        self.pending.push_back(Token::Eof);
        self.finished = true;
    }

    fn start_tag(&mut self, end: bool) {
        self.tag = TagBuild {
            end,
            ..TagBuild::default()
        };
    }

    fn emit_tag(&mut self) {
        let tag = std::mem::take(&mut self.tag);
        if tag.end {
            self.pending.push_back(Token::EndTag {
                name: tag.name,
                attrs: tag.attrs,
                self_closing: tag.self_closing,
            });
        } else {
            self.last_start_tag = tag.name.clone();
            self.pending.push_back(Token::StartTag {
                name: tag.name,
                attrs: tag.attrs,
                self_closing: tag.self_closing,
            });
        }
    }

    fn emit_comment(&mut self) {
        let c = std::mem::take(&mut self.comment);
        self.pending.push_back(Token::Comment(c));
    }

    fn emit_doctype(&mut self) {
        let d = std::mem::take(&mut self.doctype);
        self.pending.push_back(Token::Doctype {
            name: d.name,
            public_id: d.public_id,
            system_id: d.system_id,
            force_quirks: d.force_quirks,
        });
    }

    fn appropriate_end_tag(&self) -> bool {
        self.tag.end && !self.last_start_tag.is_empty() && self.tag.name == self.last_start_tag
    }

    fn new_attribute(&mut self) {
        self.finish_attribute_name();
        self.tag.attrs.push((String::new(), String::new()));
        self.tag.dropped_attr = false;
    }

    /// Called when the tokenizer leaves the attribute name state: a repeated name is dropped.
    fn finish_attribute_name(&mut self) {
        if self.tag.dropped_attr {
            return;
        }
        if let Some((last, rest)) = self.tag.attrs.split_last()
            && !last.0.is_empty()
            && rest.iter().any(|(n, _)| n == &last.0)
        {
            self.tag.attrs.pop();
            self.tag.dropped_attr = true;
        }
    }

    fn attr_name_push(&mut self, c: char) {
        if let Some(last) = self.tag.attrs.last_mut() {
            last.0.push(c);
        }
    }

    fn attr_value_push(&mut self, c: char) {
        if self.tag.dropped_attr {
            return;
        }
        if let Some(last) = self.tag.attrs.last_mut() {
            last.1.push(c);
        }
    }

    fn consumed_in_attribute(&self) -> bool {
        matches!(
            self.return_state,
            State::AttributeValueDoubleQuoted
                | State::AttributeValueSingleQuoted
                | State::AttributeValueUnquoted
        )
    }

    fn flush_char_ref(&mut self) {
        let buf = std::mem::take(&mut self.temp);
        if self.consumed_in_attribute() {
            for c in buf.chars() {
                self.attr_value_push(c);
            }
        } else {
            for c in buf.chars() {
                self.emit_char(c);
            }
        }
    }

    fn emit_str(&mut self, s: &str) {
        for c in s.chars() {
            self.emit_char(c);
        }
    }

    fn step(&mut self) {
        use State::*;
        let c = self.consume();
        match self.state {
            Data => match c {
                Some('&') => {
                    self.return_state = Data;
                    self.state = CharacterReference;
                }
                Some('<') => self.state = TagOpen,
                Some(ch) => self.emit_char(ch),
                None => self.emit_eof(),
            },
            Rcdata => match c {
                Some('&') => {
                    self.return_state = Rcdata;
                    self.state = CharacterReference;
                }
                Some('<') => self.state = RcdataLessThan,
                Some('\0') => self.emit_char('\u{fffd}'),
                Some(ch) => self.emit_char(ch),
                None => self.emit_eof(),
            },
            Rawtext => match c {
                Some('<') => self.state = RawtextLessThan,
                Some('\0') => self.emit_char('\u{fffd}'),
                Some(ch) => self.emit_char(ch),
                None => self.emit_eof(),
            },
            ScriptData => match c {
                Some('<') => self.state = ScriptDataLessThan,
                Some('\0') => self.emit_char('\u{fffd}'),
                Some(ch) => self.emit_char(ch),
                None => self.emit_eof(),
            },
            Plaintext => match c {
                Some('\0') => self.emit_char('\u{fffd}'),
                Some(ch) => self.emit_char(ch),
                None => self.emit_eof(),
            },
            TagOpen => match c {
                Some('!') => self.state = MarkupDeclarationOpen,
                Some('/') => self.state = EndTagOpen,
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.start_tag(false);
                    self.reconsume();
                    self.state = TagName;
                }
                Some('?') => {
                    self.temp.clear();
                    self.state = ProcessingInstructionOpen;
                }
                None => {
                    self.emit_char('<');
                    self.emit_eof();
                }
                Some(_) => {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = Data;
                }
            },
            EndTagOpen => match c {
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.start_tag(true);
                    self.reconsume();
                    self.state = TagName;
                }
                Some('>') => self.state = Data,
                None => {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_eof();
                }
                Some(_) => {
                    self.comment.clear();
                    self.reconsume();
                    self.state = BogusComment;
                }
            },
            TagName => match c {
                Some(ch) if is_space(ch) => self.state = BeforeAttributeName,
                Some('/') => self.state = SelfClosingStartTag,
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                Some(ch) if ch.is_ascii_uppercase() => self.tag.name.push(ch.to_ascii_lowercase()),
                Some('\0') => self.tag.name.push('\u{fffd}'),
                Some(ch) => self.tag.name.push(ch),
                None => self.emit_eof(),
            },
            RcdataLessThan => match c {
                Some('/') => {
                    self.temp.clear();
                    self.state = RcdataEndTagOpen;
                }
                _ => {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = Rcdata;
                }
            },
            RcdataEndTagOpen => match c {
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.start_tag(true);
                    self.reconsume();
                    self.state = RcdataEndTagName;
                }
                _ => {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.reconsume();
                    self.state = Rcdata;
                }
            },
            RcdataEndTagName => self.raw_end_tag_name(c, Rcdata, RcdataEndTagName),
            RawtextLessThan => match c {
                Some('/') => {
                    self.temp.clear();
                    self.state = RawtextEndTagOpen;
                }
                _ => {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = Rawtext;
                }
            },
            RawtextEndTagOpen => match c {
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.start_tag(true);
                    self.reconsume();
                    self.state = RawtextEndTagName;
                }
                _ => {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.reconsume();
                    self.state = Rawtext;
                }
            },
            RawtextEndTagName => self.raw_end_tag_name(c, Rawtext, RawtextEndTagName),
            ScriptDataLessThan => match c {
                Some('/') => {
                    self.temp.clear();
                    self.state = ScriptDataEndTagOpen;
                }
                Some('!') => {
                    self.state = ScriptDataEscapeStart;
                    self.emit_char('<');
                    self.emit_char('!');
                }
                _ => {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = ScriptData;
                }
            },
            ScriptDataEndTagOpen => match c {
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.start_tag(true);
                    self.reconsume();
                    self.state = ScriptDataEndTagName;
                }
                _ => {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.reconsume();
                    self.state = ScriptData;
                }
            },
            ScriptDataEndTagName => self.raw_end_tag_name(c, ScriptData, ScriptDataEndTagName),
            ScriptDataEscapeStart => match c {
                Some('-') => {
                    self.state = ScriptDataEscapeStartDash;
                    self.emit_char('-');
                }
                _ => {
                    self.reconsume();
                    self.state = ScriptData;
                }
            },
            ScriptDataEscapeStartDash => match c {
                Some('-') => {
                    self.state = ScriptDataEscapedDashDash;
                    self.emit_char('-');
                }
                _ => {
                    self.reconsume();
                    self.state = ScriptData;
                }
            },
            ScriptDataEscaped => match c {
                Some('-') => {
                    self.state = ScriptDataEscapedDash;
                    self.emit_char('-');
                }
                Some('<') => self.state = ScriptDataEscapedLessThan,
                Some('\0') => self.emit_char('\u{fffd}'),
                Some(ch) => self.emit_char(ch),
                None => self.emit_eof(),
            },
            ScriptDataEscapedDash => match c {
                Some('-') => {
                    self.state = ScriptDataEscapedDashDash;
                    self.emit_char('-');
                }
                Some('<') => self.state = ScriptDataEscapedLessThan,
                Some('\0') => {
                    self.state = ScriptDataEscaped;
                    self.emit_char('\u{fffd}');
                }
                Some(ch) => {
                    self.state = ScriptDataEscaped;
                    self.emit_char(ch);
                }
                None => self.emit_eof(),
            },
            ScriptDataEscapedDashDash => match c {
                Some('-') => self.emit_char('-'),
                Some('<') => self.state = ScriptDataEscapedLessThan,
                Some('>') => {
                    self.state = ScriptData;
                    self.emit_char('>');
                }
                Some('\0') => {
                    self.state = ScriptDataEscaped;
                    self.emit_char('\u{fffd}');
                }
                Some(ch) => {
                    self.state = ScriptDataEscaped;
                    self.emit_char(ch);
                }
                None => self.emit_eof(),
            },
            ScriptDataEscapedLessThan => match c {
                Some('/') => {
                    self.temp.clear();
                    self.state = ScriptDataEscapedEndTagOpen;
                }
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.temp.clear();
                    self.emit_char('<');
                    self.reconsume();
                    self.state = ScriptDataDoubleEscapeStart;
                }
                _ => {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = ScriptDataEscaped;
                }
            },
            ScriptDataEscapedEndTagOpen => match c {
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.start_tag(true);
                    self.reconsume();
                    self.state = ScriptDataEscapedEndTagName;
                }
                _ => {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.reconsume();
                    self.state = ScriptDataEscaped;
                }
            },
            ScriptDataEscapedEndTagName => {
                self.raw_end_tag_name(c, ScriptDataEscaped, ScriptDataEscapedEndTagName)
            }
            ScriptDataDoubleEscapeStart => match c {
                Some(ch) if is_space(ch) || ch == '/' || ch == '>' => {
                    self.state = if self.temp == "script" {
                        ScriptDataDoubleEscaped
                    } else {
                        ScriptDataEscaped
                    };
                    self.emit_char(ch);
                }
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.temp.push(ch.to_ascii_lowercase());
                    self.emit_char(ch);
                }
                _ => {
                    self.reconsume();
                    self.state = ScriptDataEscaped;
                }
            },
            ScriptDataDoubleEscaped => match c {
                Some('-') => {
                    self.state = ScriptDataDoubleEscapedDash;
                    self.emit_char('-');
                }
                Some('<') => {
                    self.state = ScriptDataDoubleEscapedLessThan;
                    self.emit_char('<');
                }
                Some('\0') => self.emit_char('\u{fffd}'),
                Some(ch) => self.emit_char(ch),
                None => self.emit_eof(),
            },
            ScriptDataDoubleEscapedDash => match c {
                Some('-') => {
                    self.state = ScriptDataDoubleEscapedDashDash;
                    self.emit_char('-');
                }
                Some('<') => {
                    self.state = ScriptDataDoubleEscapedLessThan;
                    self.emit_char('<');
                }
                Some('\0') => {
                    self.state = ScriptDataDoubleEscaped;
                    self.emit_char('\u{fffd}');
                }
                Some(ch) => {
                    self.state = ScriptDataDoubleEscaped;
                    self.emit_char(ch);
                }
                None => self.emit_eof(),
            },
            ScriptDataDoubleEscapedDashDash => match c {
                Some('-') => self.emit_char('-'),
                Some('<') => {
                    self.state = ScriptDataDoubleEscapedLessThan;
                    self.emit_char('<');
                }
                Some('>') => {
                    self.state = ScriptData;
                    self.emit_char('>');
                }
                Some('\0') => {
                    self.state = ScriptDataDoubleEscaped;
                    self.emit_char('\u{fffd}');
                }
                Some(ch) => {
                    self.state = ScriptDataDoubleEscaped;
                    self.emit_char(ch);
                }
                None => self.emit_eof(),
            },
            ScriptDataDoubleEscapedLessThan => match c {
                Some('/') => {
                    self.temp.clear();
                    self.state = ScriptDataDoubleEscapeEnd;
                    self.emit_char('/');
                }
                _ => {
                    self.reconsume();
                    self.state = ScriptDataDoubleEscaped;
                }
            },
            ScriptDataDoubleEscapeEnd => match c {
                Some(ch) if is_space(ch) || ch == '/' || ch == '>' => {
                    self.state = if self.temp == "script" {
                        ScriptDataEscaped
                    } else {
                        ScriptDataDoubleEscaped
                    };
                    self.emit_char(ch);
                }
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.temp.push(ch.to_ascii_lowercase());
                    self.emit_char(ch);
                }
                _ => {
                    self.reconsume();
                    self.state = ScriptDataDoubleEscaped;
                }
            },
            BeforeAttributeName => match c {
                Some(ch) if is_space(ch) => {}
                Some('/') | Some('>') | None => {
                    self.reconsume();
                    self.state = AfterAttributeName;
                }
                Some('=') => {
                    self.new_attribute();
                    self.attr_name_push('=');
                    self.state = AttributeName;
                }
                Some(_) => {
                    self.new_attribute();
                    self.reconsume();
                    self.state = AttributeName;
                }
            },
            AttributeName => match c {
                Some(ch) if is_space(ch) || ch == '/' || ch == '>' => {
                    self.finish_attribute_name();
                    self.reconsume();
                    self.state = AfterAttributeName;
                }
                None => {
                    self.finish_attribute_name();
                    self.reconsume();
                    self.state = AfterAttributeName;
                }
                Some('=') => {
                    self.finish_attribute_name();
                    self.state = BeforeAttributeValue;
                }
                Some(ch) if ch.is_ascii_uppercase() => self.attr_name_push(ch.to_ascii_lowercase()),
                Some('\0') => self.attr_name_push('\u{fffd}'),
                Some(ch) => self.attr_name_push(ch),
            },
            AfterAttributeName => match c {
                Some(ch) if is_space(ch) => {}
                Some('/') => self.state = SelfClosingStartTag,
                Some('=') => self.state = BeforeAttributeValue,
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                None => self.emit_eof(),
                Some(_) => {
                    self.new_attribute();
                    self.reconsume();
                    self.state = AttributeName;
                }
            },
            BeforeAttributeValue => match c {
                Some(ch) if is_space(ch) => {}
                Some('"') => self.state = AttributeValueDoubleQuoted,
                Some('\'') => self.state = AttributeValueSingleQuoted,
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                _ => {
                    self.reconsume();
                    self.state = AttributeValueUnquoted;
                }
            },
            AttributeValueDoubleQuoted => match c {
                Some('"') => self.state = AfterAttributeValueQuoted,
                Some('&') => {
                    self.return_state = AttributeValueDoubleQuoted;
                    self.state = CharacterReference;
                }
                Some('\0') => self.attr_value_push('\u{fffd}'),
                Some(ch) => self.attr_value_push(ch),
                None => self.emit_eof(),
            },
            AttributeValueSingleQuoted => match c {
                Some('\'') => self.state = AfterAttributeValueQuoted,
                Some('&') => {
                    self.return_state = AttributeValueSingleQuoted;
                    self.state = CharacterReference;
                }
                Some('\0') => self.attr_value_push('\u{fffd}'),
                Some(ch) => self.attr_value_push(ch),
                None => self.emit_eof(),
            },
            AttributeValueUnquoted => match c {
                Some(ch) if is_space(ch) => self.state = BeforeAttributeName,
                Some('&') => {
                    self.return_state = AttributeValueUnquoted;
                    self.state = CharacterReference;
                }
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                Some('\0') => self.attr_value_push('\u{fffd}'),
                Some(ch) => self.attr_value_push(ch),
                None => self.emit_eof(),
            },
            AfterAttributeValueQuoted => match c {
                Some(ch) if is_space(ch) => self.state = BeforeAttributeName,
                Some('/') => self.state = SelfClosingStartTag,
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                None => self.emit_eof(),
                Some(_) => {
                    self.reconsume();
                    self.state = BeforeAttributeName;
                }
            },
            SelfClosingStartTag => match c {
                Some('>') => {
                    self.tag.self_closing = true;
                    self.state = Data;
                    self.emit_tag();
                }
                None => self.emit_eof(),
                Some(_) => {
                    self.reconsume();
                    self.state = BeforeAttributeName;
                }
            },
            BogusComment => match c {
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                None => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Some('\0') => self.comment.push('\u{fffd}'),
                Some(ch) => self.comment.push(ch),
            },
            MarkupDeclarationOpen => {
                self.reconsume();
                if self.peek_matches("--", false) {
                    self.pos += 2;
                    self.comment.clear();
                    self.state = CommentStart;
                } else if self.peek_matches("DOCTYPE", true) {
                    self.pos += 7;
                    self.state = Doctype;
                } else if self.peek_matches("[CDATA[", false) {
                    self.pos += 7;
                    if self.allow_cdata {
                        self.state = CdataSection;
                    } else {
                        self.comment = "[CDATA[".to_string();
                        self.state = BogusComment;
                    }
                } else {
                    self.comment.clear();
                    self.state = BogusComment;
                }
            }
            CommentStart => match c {
                Some('-') => self.state = CommentStartDash,
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                _ => {
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentStartDash => match c {
                Some('-') => self.state = CommentEnd,
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                None => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Some(_) => {
                    self.comment.push('-');
                    self.reconsume();
                    self.state = Comment;
                }
            },
            Comment => match c {
                Some('<') => {
                    self.comment.push('<');
                    self.state = CommentLessThan;
                }
                Some('-') => self.state = CommentEndDash,
                Some('\0') => self.comment.push('\u{fffd}'),
                None => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Some(ch) => self.comment.push(ch),
            },
            CommentLessThan => match c {
                Some('!') => {
                    self.comment.push('!');
                    self.state = CommentLessThanBang;
                }
                Some('<') => self.comment.push('<'),
                _ => {
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentLessThanBang => match c {
                Some('-') => self.state = CommentLessThanBangDash,
                _ => {
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentLessThanBangDash => match c {
                Some('-') => self.state = CommentLessThanBangDashDash,
                _ => {
                    self.reconsume();
                    self.state = CommentEndDash;
                }
            },
            CommentLessThanBangDashDash => {
                self.reconsume();
                self.state = CommentEnd;
            }
            CommentEndDash => match c {
                Some('-') => self.state = CommentEnd,
                None => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Some(_) => {
                    self.comment.push('-');
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentEnd => match c {
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                Some('!') => self.state = CommentEndBang,
                Some('-') => self.comment.push('-'),
                None => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Some(_) => {
                    self.comment.push_str("--");
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentEndBang => match c {
                Some('-') => {
                    self.comment.push_str("--!");
                    self.state = CommentEndDash;
                }
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                None => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Some(_) => {
                    self.comment.push_str("--!");
                    self.reconsume();
                    self.state = Comment;
                }
            },
            Doctype => match c {
                Some(ch) if is_space(ch) => self.state = BeforeDoctypeName,
                Some('>') => {
                    self.reconsume();
                    self.state = BeforeDoctypeName;
                }
                None => {
                    self.doctype = DoctypeBuild {
                        force_quirks: true,
                        ..DoctypeBuild::default()
                    };
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.reconsume();
                    self.state = BeforeDoctypeName;
                }
            },
            BeforeDoctypeName => match c {
                Some(ch) if is_space(ch) => {}
                Some(ch) if ch.is_ascii_uppercase() => {
                    self.doctype = DoctypeBuild {
                        name: Some(ch.to_ascii_lowercase().to_string()),
                        ..DoctypeBuild::default()
                    };
                    self.state = DoctypeName;
                }
                Some('\0') => {
                    self.doctype = DoctypeBuild {
                        name: Some("\u{fffd}".into()),
                        ..DoctypeBuild::default()
                    };
                    self.state = DoctypeName;
                }
                Some('>') => {
                    self.doctype = DoctypeBuild {
                        force_quirks: true,
                        ..DoctypeBuild::default()
                    };
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype = DoctypeBuild {
                        force_quirks: true,
                        ..DoctypeBuild::default()
                    };
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(ch) => {
                    self.doctype = DoctypeBuild {
                        name: Some(ch.to_string()),
                        ..DoctypeBuild::default()
                    };
                    self.state = DoctypeName;
                }
            },
            DoctypeName => match c {
                Some(ch) if is_space(ch) => self.state = AfterDoctypeName,
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                Some(ch) if ch.is_ascii_uppercase() => self
                    .doctype
                    .name
                    .get_or_insert_with(String::new)
                    .push(ch.to_ascii_lowercase()),
                Some('\0') => self
                    .doctype
                    .name
                    .get_or_insert_with(String::new)
                    .push('\u{fffd}'),
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(ch) => self.doctype.name.get_or_insert_with(String::new).push(ch),
            },
            AfterDoctypeName => match c {
                Some(ch) if is_space(ch) => {}
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.reconsume();
                    if self.peek_matches("PUBLIC", true) {
                        self.pos += 6;
                        self.state = AfterDoctypePublicKeyword;
                    } else if self.peek_matches("SYSTEM", true) {
                        self.pos += 6;
                        self.state = AfterDoctypeSystemKeyword;
                    } else {
                        self.doctype.force_quirks = true;
                        self.state = BogusDoctype;
                    }
                }
            },
            AfterDoctypePublicKeyword => match c {
                Some(ch) if is_space(ch) => self.state = BeforeDoctypePublicIdentifier,
                Some('"') => {
                    self.doctype.public_id = Some(String::new());
                    self.state = DoctypePublicIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.doctype.public_id = Some(String::new());
                    self.state = DoctypePublicIdentifierSingleQuoted;
                }
                Some('>') => {
                    self.doctype.force_quirks = true;
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            BeforeDoctypePublicIdentifier => match c {
                Some(ch) if is_space(ch) => {}
                Some('"') => {
                    self.doctype.public_id = Some(String::new());
                    self.state = DoctypePublicIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.doctype.public_id = Some(String::new());
                    self.state = DoctypePublicIdentifierSingleQuoted;
                }
                Some('>') => {
                    self.doctype.force_quirks = true;
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            DoctypePublicIdentifierDoubleQuoted => self.doctype_identifier(c, '"', true),
            DoctypePublicIdentifierSingleQuoted => self.doctype_identifier(c, '\'', true),
            AfterDoctypePublicIdentifier => match c {
                Some(ch) if is_space(ch) => self.state = BetweenDoctypePublicAndSystemIdentifiers,
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                Some('"') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierSingleQuoted;
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            BetweenDoctypePublicAndSystemIdentifiers => match c {
                Some(ch) if is_space(ch) => {}
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                Some('"') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierSingleQuoted;
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            AfterDoctypeSystemKeyword => match c {
                Some(ch) if is_space(ch) => self.state = BeforeDoctypeSystemIdentifier,
                Some('"') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierSingleQuoted;
                }
                Some('>') => {
                    self.doctype.force_quirks = true;
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            BeforeDoctypeSystemIdentifier => match c {
                Some(ch) if is_space(ch) => {}
                Some('"') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierSingleQuoted;
                }
                Some('>') => {
                    self.doctype.force_quirks = true;
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            DoctypeSystemIdentifierDoubleQuoted => self.doctype_identifier(c, '"', false),
            DoctypeSystemIdentifierSingleQuoted => self.doctype_identifier(c, '\'', false),
            AfterDoctypeSystemIdentifier => match c {
                Some(ch) if is_space(ch) => {}
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            BogusDoctype => match c {
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.emit_doctype();
                    self.emit_eof();
                }
                Some(_) => {}
            },
            CdataSection => match c {
                Some(']') => self.state = CdataSectionBracket,
                None => self.emit_eof(),
                Some(ch) => self.emit_char(ch),
            },
            CdataSectionBracket => match c {
                Some(']') => self.state = CdataSectionEnd,
                _ => {
                    self.emit_char(']');
                    self.reconsume();
                    self.state = CdataSection;
                }
            },
            CdataSectionEnd => match c {
                Some(']') => self.emit_char(']'),
                Some('>') => self.state = Data,
                _ => {
                    self.emit_char(']');
                    self.emit_char(']');
                    self.reconsume();
                    self.state = CdataSection;
                }
            },
            ProcessingInstructionOpen => match c {
                Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {
                    self.reconsume();
                    self.pi_target.clear();
                    self.pi_data.clear();
                    self.state = ProcessingInstructionTarget;
                }
                None => self.emit_eof(),
                Some(_) => {
                    self.comment = format!("?{}", self.temp);
                    self.reconsume();
                    self.state = BogusComment;
                }
            },
            ProcessingInstructionTarget => match c {
                Some(ch) if is_space(ch) || ch == '?' || ch == '>' => {
                    let target = self.temp.clone();
                    if target.eq_ignore_ascii_case("xml")
                        || target.eq_ignore_ascii_case("xml-stylesheet")
                    {
                        self.comment = format!("?{}", self.temp);
                        self.reconsume();
                        self.state = BogusComment;
                    } else {
                        self.pi_target = target;
                        self.pi_data.clear();
                        self.reconsume();
                        self.state = AfterProcessingInstructionTarget;
                    }
                }
                Some(ch) if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' => {
                    self.temp.push(ch)
                }
                None => self.emit_eof(),
                Some(_) => {
                    self.comment = format!("?{}", self.temp);
                    self.reconsume();
                    self.state = BogusComment;
                }
            },
            AfterProcessingInstructionTarget => match c {
                Some(ch) if is_space(ch) => {}
                _ => {
                    self.reconsume();
                    self.state = ProcessingInstructionData;
                }
            },
            ProcessingInstructionData => match c {
                Some('?') => self.state = ProcessingInstructionQuestionable,
                Some('>') => {
                    self.state = Data;
                    let (target, data) = (
                        std::mem::take(&mut self.pi_target),
                        std::mem::take(&mut self.pi_data),
                    );
                    self.emit(Token::ProcessingInstruction { target, data });
                }
                None => self.emit_eof(),
                Some(ch) => self.pi_data.push(ch),
            },
            ProcessingInstructionQuestionable => match c {
                Some('>') => {
                    self.state = Data;
                    let (target, data) = (
                        std::mem::take(&mut self.pi_target),
                        std::mem::take(&mut self.pi_data),
                    );
                    self.emit(Token::ProcessingInstruction { target, data });
                }
                None => self.emit_eof(),
                Some(_) => {
                    self.pi_data.push('?');
                    self.reconsume();
                    self.state = ProcessingInstructionData;
                }
            },
            CharacterReference => {
                self.temp.clear();
                self.temp.push('&');
                match c {
                    Some(ch) if ch.is_ascii_alphanumeric() => {
                        self.reconsume();
                        self.state = NamedCharacterReference;
                    }
                    Some('#') => {
                        self.temp.push('#');
                        self.state = NumericCharacterReference;
                    }
                    _ => {
                        self.flush_char_ref();
                        self.reconsume();
                        self.state = self.return_state;
                    }
                }
            }
            NamedCharacterReference => {
                self.reconsume();
                let rest: String = self.input[self.pos..].iter().take(33).collect();
                let table = entities();
                let mut matched: Option<(usize, &String)> = None;
                for len in (1..=rest.chars().count()).rev() {
                    let end = rest.char_indices().nth(len).map_or(rest.len(), |(i, _)| i);
                    if let Some(v) = table.get(&rest[..end]) {
                        matched = Some((len, v));
                        break;
                    }
                }
                match matched {
                    Some((len, value)) => {
                        let last = self.input[self.pos + len - 1];
                        let next = self.input.get(self.pos + len).copied();
                        let value = value.clone();
                        if self.consumed_in_attribute()
                            && last != ';'
                            && matches!(next, Some(n) if n == '=' || n.is_ascii_alphanumeric())
                        {
                            let consumed: String =
                                self.input[self.pos..self.pos + len].iter().collect();
                            self.temp.push_str(&consumed);
                            self.pos += len;
                            self.flush_char_ref();
                            self.state = self.return_state;
                        } else {
                            self.pos += len;
                            self.temp = value;
                            self.flush_char_ref();
                            self.state = self.return_state;
                        }
                    }
                    None => {
                        // Consume the alphanumerics (and a closing semicolon) as part of the unknown reference.
                        let mut len = 0;
                        while self
                            .input
                            .get(self.pos + len)
                            .is_some_and(|c| c.is_ascii_alphanumeric())
                        {
                            len += 1;
                        }
                        let consumed: String =
                            self.input[self.pos..self.pos + len].iter().collect();
                        self.temp.push_str(&consumed);
                        self.pos += len;
                        self.flush_char_ref();
                        self.state = AmbiguousAmpersand;
                    }
                }
            }
            AmbiguousAmpersand => match c {
                Some(ch) if ch.is_ascii_alphanumeric() => {
                    if self.consumed_in_attribute() {
                        self.attr_value_push(ch);
                    } else {
                        self.emit_char(ch);
                    }
                }
                _ => {
                    self.reconsume();
                    self.state = self.return_state;
                }
            },
            NumericCharacterReference => {
                self.char_ref_code = 0;
                match c {
                    Some(ch @ ('x' | 'X')) => {
                        self.temp.push(ch);
                        self.state = HexadecimalCharacterReferenceStart;
                    }
                    Some(ch) if ch.is_ascii_digit() => {
                        self.reconsume();
                        self.state = DecimalCharacterReference;
                    }
                    _ => {
                        self.flush_char_ref();
                        self.reconsume();
                        self.state = self.return_state;
                    }
                }
            }
            HexadecimalCharacterReferenceStart => match c {
                Some(ch) if ch.is_ascii_hexdigit() => {
                    self.reconsume();
                    self.state = HexadecimalCharacterReference;
                }
                _ => {
                    self.flush_char_ref();
                    self.reconsume();
                    self.state = self.return_state;
                }
            },
            HexadecimalCharacterReference => match c {
                Some(ch) if ch.is_ascii_hexdigit() => {
                    self.char_ref_code = self
                        .char_ref_code
                        .saturating_mul(16)
                        .saturating_add(ch.to_digit(16).unwrap());
                }
                Some(';') => self.state = NumericCharacterReferenceEnd,
                _ => {
                    self.reconsume();
                    self.state = NumericCharacterReferenceEnd;
                }
            },
            DecimalCharacterReference => match c {
                Some(ch) if ch.is_ascii_digit() => {
                    self.char_ref_code = self
                        .char_ref_code
                        .saturating_mul(10)
                        .saturating_add(ch.to_digit(10).unwrap());
                }
                Some(';') => self.state = NumericCharacterReferenceEnd,
                _ => {
                    self.reconsume();
                    self.state = NumericCharacterReferenceEnd;
                }
            },
            NumericCharacterReferenceEnd => {
                self.reconsume();
                let mut code = self.char_ref_code;
                if code == 0 || code > 0x10FFFF || (0xD800..=0xDFFF).contains(&code) {
                    code = 0xFFFD;
                } else if let Some((_, mapped)) =
                    WINDOWS_1252.iter().find(|(from, _)| *from == code)
                {
                    code = *mapped;
                }
                self.temp.clear();
                self.temp.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                self.flush_char_ref();
                self.state = self.return_state;
            }
        }
    }

    fn raw_end_tag_name(&mut self, c: Option<char>, fallback: State, own: State) {
        use State::*;
        let _ = own;
        match c {
            Some(ch) if is_space(ch) && self.appropriate_end_tag() => {
                self.state = BeforeAttributeName
            }
            Some('/') if self.appropriate_end_tag() => self.state = SelfClosingStartTag,
            Some('>') if self.appropriate_end_tag() => {
                self.state = Data;
                self.emit_tag();
            }
            Some(ch) if ch.is_ascii_uppercase() => {
                self.tag.name.push(ch.to_ascii_lowercase());
                self.temp.push(ch);
            }
            Some(ch) if ch.is_ascii_lowercase() => {
                self.tag.name.push(ch);
                self.temp.push(ch);
            }
            _ => {
                self.emit_char('<');
                self.emit_char('/');
                let buffered = std::mem::take(&mut self.temp);
                self.emit_str(&buffered);
                self.reconsume();
                self.state = fallback;
            }
        }
    }

    fn doctype_identifier(&mut self, c: Option<char>, quote: char, public: bool) {
        use State::*;
        match c {
            Some(ch) if ch == quote => {
                self.state = if public {
                    AfterDoctypePublicIdentifier
                } else {
                    AfterDoctypeSystemIdentifier
                }
            }
            Some('\0') => doctype_slot(&mut self.doctype, public).push('\u{fffd}'),
            Some('>') => {
                self.doctype.force_quirks = true;
                self.state = Data;
                self.emit_doctype();
            }
            None => {
                self.doctype.force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(ch) => doctype_slot(&mut self.doctype, public).push(ch),
        }
    }
}

fn doctype_slot(d: &mut DoctypeBuild, public: bool) -> &mut String {
    if public {
        d.public_id.get_or_insert_with(String::new)
    } else {
        d.system_id.get_or_insert_with(String::new)
    }
}
