//! JSON text from a value: compact (`{"a":1}`) or indented by two spaces.

use super::value::Value;

pub fn compact(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(n.as_str()),
        Value::String(s) => string(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                compact(item, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            for (i, (k, item)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                string(k, out);
                out.push(':');
                compact(item, out);
            }
            out.push('}');
        }
    }
}

pub fn pretty(v: &Value, out: &mut String) {
    indented(v, 0, out);
}

fn indent(level: usize, out: &mut String) {
    out.push('\n');
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn indented(v: &Value, level: usize, out: &mut String) {
    match v {
        Value::Array(a) if !a.is_empty() => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                indent(level + 1, out);
                indented(item, level + 1, out);
            }
            indent(level, out);
            out.push(']');
        }
        Value::Object(m) if !m.is_empty() => {
            out.push('{');
            for (i, (k, item)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                indent(level + 1, out);
                string(k, out);
                out.push_str(": ");
                indented(item, level + 1, out);
            }
            indent(level, out);
            out.push('}');
        }
        other => compact(other, out),
    }
}

/// A JSON string literal: quotes, backslash and control characters are escaped, everything else
/// (including U+007F and all non-ASCII) is written as UTF-8.
pub fn string(s: &str, out: &mut String) {
    out.push('"');
    let mut start = 0;
    for (i, c) in s.char_indices() {
        let escape = match c {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\u{8}' => "\\b",
            '\u{c}' => "\\f",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            c if (c as u32) < 0x20 => {
                out.push_str(&s[start..i]);
                out.push_str(&format!("\\u{:04x}", c as u32));
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        out.push_str(&s[start..i]);
        out.push_str(escape);
        start = i + c.len_utf8();
    }
    out.push_str(&s[start..]);
    out.push('"');
}
