//! `date`, `time`, `datetime`, `julianday`, `unixepoch` and `strftime` with SQLite's time values:
//! the ISO-8601 text forms, Julian day and Unix numbers, `'now'`, and the modifiers that need no
//! time zone (`±N unit`, `start of ...`, `weekday N`, `unixepoch`, `auto`, `ceiling`, `floor`).
//! `localtime`, `utc` and `subsec` are refused: the store has no zone database, so a result that
//! silently ignored the zone would be wrong. Arithmetic follows SQLite's own: a Julian day count
//! in milliseconds, civil dates derived with the same integer and floating steps.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    error::{Error, Result},
    value::Value,
};

const MS_DAY: i64 = 86_400_000;
const UNIX_MS: i64 = 210_866_760_000_000;
const MAX_JD: i64 = 464_269_060_799_999;

#[derive(Clone, Copy, Default)]
struct Dt {
    jd: i64,
    y: i32,
    m: i32,
    d: i32,
    h: i32,
    min: i32,
    s: f64,
    valid_jd: bool,
    valid_ymd: bool,
    valid_hms: bool,
    valid_tz: bool,
    raw_s: bool,
    raw_bad: bool,
    floor: i32,
    raw: f64,
    tz: i32,
    error: bool,
}

impl Dt {
    fn compute_jd(&mut self) {
        if self.valid_jd {
            return;
        }
        let (mut y, mut m, d) = if self.valid_ymd {
            (self.y, self.m, self.d)
        } else {
            (2000, 1, 1)
        };
        if !(-4713..=9999).contains(&y) || self.raw_bad {
            self.error = true;
            return;
        }
        if m <= 2 {
            y -= 1;
            m += 12;
        }
        let a = y / 100;
        let b = 2 - a + a / 4;
        let x1 = 36525 * (y + 4716) / 100;
        let x2 = 306001 * (m + 1) / 10000;
        self.jd = ((f64::from(x1) + f64::from(x2) + f64::from(d) + f64::from(b) - 1524.5)
            * MS_DAY as f64) as i64;
        self.valid_jd = true;
        if self.valid_hms {
            self.jd += i64::from(self.h) * 3_600_000
                + i64::from(self.min) * 60_000
                + (self.s * 1000.0 + 0.5) as i64;
            if self.valid_tz {
                self.jd -= i64::from(self.tz) * 60_000;
                self.valid_ymd = false;
                self.valid_hms = false;
                self.valid_tz = false;
            }
        }
    }

    fn compute_ymd(&mut self) {
        if self.valid_ymd {
            return;
        }
        if !self.valid_jd {
            self.y = 2000;
            self.m = 1;
            self.d = 1;
        } else if !(0..=MAX_JD).contains(&self.jd) {
            self.error = true;
            return;
        } else {
            let z = ((self.jd + 43_200_000) / MS_DAY) as i32;
            let alpha = ((f64::from(z) + 32044.75) / 36524.25) as i32 - 52;
            let a = z + 1 + alpha - (alpha + 100) / 4 + 25;
            let b = a + 1524;
            let c = ((f64::from(b) - 122.1) / 365.25) as i32;
            let d = 36525 * (c & 32767) / 100;
            let e = (f64::from(b - d) / 30.6001) as i32;
            let x1 = (30.6001 * f64::from(e)) as i32;
            self.d = b - d - x1;
            self.m = if e < 14 { e - 1 } else { e - 13 };
            self.y = if self.m > 2 { c - 4716 } else { c - 4715 };
        }
        self.valid_ymd = true;
    }

    fn compute_hms(&mut self) {
        if self.valid_hms {
            return;
        }
        self.compute_jd();
        let day_ms = ((self.jd + 43_200_000) % MS_DAY) as i32;
        self.s = f64::from(day_ms % 60_000) / 1000.0;
        let day_min = day_ms / 60_000;
        self.min = day_min % 60;
        self.h = day_min / 60;
        self.valid_hms = true;
    }

    fn compute_ymd_hms(&mut self) {
        self.compute_ymd();
        self.compute_hms();
    }

    fn compute_floor(&mut self) {
        self.floor = if self.d <= 28 || (1 << self.m) & 0x15aa != 0 {
            0
        } else if self.m != 2 {
            i32::from(self.d == 31)
        } else if self.y % 4 != 0 || (self.y % 100 == 0 && self.y % 400 != 0) {
            self.d - 28
        } else {
            self.d - 29
        };
    }

    fn clear(&mut self) {
        self.valid_ymd = false;
        self.valid_hms = false;
        self.valid_tz = false;
    }
}

fn digits(s: &[u8], n: usize, lo: i32, hi: i32) -> Option<(i32, &[u8])> {
    if s.len() < n || !s[..n].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let v = s[..n].iter().fold(0, |a, b| a * 10 + i32::from(b - b'0'));
    (lo..=hi).contains(&v).then_some((v, &s[n..]))
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn spaces(mut s: &[u8]) -> &[u8] {
    while let [c, rest @ ..] = s {
        if is_space(*c) {
            s = rest;
        } else {
            break;
        }
    }
    s
}

/// A trailing zone: nothing, `Z`, or `±HH:MM`. The rest must be empty.
fn timezone(s: &[u8]) -> Option<Option<i32>> {
    let s = spaces(s);
    let (sign, rest) = match s.first() {
        Some(b'-') => (-1, &s[1..]),
        Some(b'+') => (1, &s[1..]),
        Some(b'Z' | b'z') => return spaces(&s[1..]).is_empty().then_some(Some(0)),
        None => return Some(None),
        _ => return None,
    };
    let (h, rest) = digits(rest, 2, 0, 14)?;
    let (m, rest) = digits(rest.strip_prefix(b":")?, 2, 0, 59)?;
    spaces(rest).is_empty().then_some(Some(sign * (h * 60 + m)))
}

/// `HH:MM[:SS[.FFF]]`, then an optional zone, then the end.
fn time_of_day(s: &[u8]) -> Option<(i32, i32, f64, Option<i32>)> {
    let (h, s) = digits(s, 2, 0, 24)?;
    let (m, mut s) = digits(s.strip_prefix(b":")?, 2, 0, 59)?;
    let mut sec = 0.0;
    if let Some(r) = s.strip_prefix(b":") {
        let (v, r) = digits(r, 2, 0, 59)?;
        sec = f64::from(v);
        s = r;
        if s.first() == Some(&b'.') && s.get(1).is_some_and(u8::is_ascii_digit) {
            let (mut ms, mut scale) = (0.0, 1.0);
            s = &s[1..];
            while let Some(c) = s.first().filter(|c| c.is_ascii_digit()) {
                ms = ms * 10.0 + f64::from(c - b'0');
                scale *= 10.0;
                s = &s[1..];
            }
            ms /= scale;
            if ms > 0.999 {
                ms = 0.999;
            }
            sec += ms;
        }
    }
    Some((h, m, sec, timezone(s)?))
}

fn now_ms() -> i64 {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    d.as_millis() as i64 + UNIX_MS
}

/// A decimal or exponent number that fills the whole string (trailing blanks allowed).
fn whole_real(s: &str) -> Option<f64> {
    let t = s.trim_end_matches(|c: char| c.is_ascii_whitespace());
    let t = t.trim_start_matches(|c: char| c.is_ascii_whitespace());
    if t.is_empty()
        || !t
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-' | b'e' | b'E'))
    {
        return None;
    }
    t.parse::<f64>().ok().filter(|f| f.is_finite())
}

fn set_raw(r: f64) -> Dt {
    let mut p = Dt {
        raw: r,
        raw_s: true,
        ..Dt::default()
    };
    if (0.0..5_373_484.5).contains(&r) {
        p.jd = (r * MS_DAY as f64 + 0.5) as i64;
        p.valid_jd = true;
    } else {
        p.raw_bad = true;
    }
    p
}

fn time_value(v: &Value) -> Option<Dt> {
    match v {
        Value::Null => None,
        Value::Integer(i) => Some(set_raw(*i as f64)),
        Value::Real(f) => Some(set_raw(*f)),
        Value::Blob(_) => None,
        Value::Text(t) => {
            if t.eq_ignore_ascii_case("now") {
                return Some(Dt {
                    jd: now_ms(),
                    valid_jd: true,
                    ..Dt::default()
                });
            }
            let b = t.as_bytes();
            let (neg, rest) = match b.first() {
                Some(b'-') => (true, &b[1..]),
                _ => (false, b),
            };
            if let Some((y, r)) = digits(rest, 4, 0, 9999)
                && let Some(r) = r.strip_prefix(b"-")
                && let Some((m, r)) = digits(r, 2, 1, 12)
                && let Some(r) = r.strip_prefix(b"-")
                && let Some((d, r)) = digits(r, 2, 1, 31)
            {
                let mut p = Dt {
                    y: if neg { -y } else { y },
                    m,
                    d,
                    valid_ymd: true,
                    ..Dt::default()
                };
                let mut r = r;
                while let [c, rest @ ..] = r {
                    if *c == b'T' || is_space(*c) {
                        r = rest;
                    } else {
                        break;
                    }
                }
                if !r.is_empty() {
                    let (h, mi, s, tz) = time_of_day(r)?;
                    p.valid_hms = true;
                    p.h = h;
                    p.min = mi;
                    p.s = s;
                    if let Some(tz) = tz {
                        p.tz = tz;
                        p.valid_tz = true;
                    }
                }
                p.compute_floor();
                if p.valid_tz {
                    p.compute_jd();
                }
                return Some(p);
            }
            if let Some((h, m, s, tz)) = time_of_day(b) {
                let mut p = Dt {
                    valid_hms: true,
                    h,
                    min: m,
                    s,
                    ..Dt::default()
                };
                if let Some(tz) = tz {
                    p.tz = tz;
                    p.valid_tz = true;
                }
                return Some(p);
            }
            whole_real(t).map(set_raw)
        }
    }
}

/// Applies one modifier (`index` counts from 0); `None` is SQLite's NULL result and `Err` is a
/// form this store does not implement.
fn modify(p: &mut Dt, text: &str, index: usize) -> Result<Option<()>> {
    if text.len() >= 30 {
        return Ok(None);
    }
    let m = text.to_ascii_lowercase();
    let unsupported = |what: &str| {
        Error::syntax(format!(
            "the date modifier '{what}' is not supported by this store"
        ))
    };
    Ok(match m.as_bytes().first().copied() {
        Some(b'a') if m == "auto" => {
            if index > 0 {
                return Ok(None);
            }
            if !p.raw_s || p.valid_jd {
                return Ok(Some(()));
            }
            let r = p.raw;
            if (-210_866_760_000.0..=253_402_300_799.0).contains(&r) {
                p.jd = (r * 1000.0 + UNIX_MS as f64 + 0.5) as i64;
                p.clear();
                p.valid_jd = true;
                p.raw_s = false;
                p.raw_bad = false;
                Some(())
            } else {
                None
            }
        }
        Some(b'u') if m == "unixepoch" => {
            if index > 0 || !p.raw_s {
                return Ok(None);
            }
            let r = p.raw * 1000.0 + UNIX_MS as f64;
            if (0.0..464_269_060_800_000.0).contains(&r) {
                p.clear();
                p.jd = (r + 0.5) as i64;
                p.valid_jd = true;
                p.raw_s = false;
                p.raw_bad = false;
                Some(())
            } else {
                None
            }
        }
        Some(b'c') if m == "ceiling" => {
            p.compute_jd();
            p.clear();
            p.floor = 0;
            Some(())
        }
        Some(b'f') if m == "floor" => {
            p.compute_jd();
            if p.floor != 0 {
                p.jd -= i64::from(p.floor) * MS_DAY;
            }
            p.clear();
            Some(())
        }
        Some(b'j') if m == "julianday" => {
            if index > 0 || !p.raw_s || !p.valid_jd {
                return Ok(None);
            }
            Some(())
        }
        Some(b'l') if m == "localtime" => return Err(unsupported("localtime")),
        Some(b'u') if m == "utc" => return Err(unsupported("utc")),
        Some(b's') if m == "subsec" || m == "subsecond" => return Err(unsupported("subsec")),
        Some(b's') if m.starts_with("start of ") => {
            if !p.valid_jd && !p.valid_ymd && !p.valid_hms {
                return Ok(None);
            }
            let what = &m["start of ".len()..];
            if !matches!(what, "month" | "year" | "day") {
                return Ok(None);
            }
            p.compute_ymd();
            p.valid_hms = true;
            p.h = 0;
            p.min = 0;
            p.s = 0.0;
            p.raw_s = false;
            p.valid_tz = false;
            p.valid_jd = false;
            match what {
                "month" => p.d = 1,
                "year" => {
                    p.m = 1;
                    p.d = 1;
                }
                _ => {}
            }
            Some(())
        }
        Some(b'w') if m.starts_with("weekday ") => {
            let Some(r) = whole_real(&m["weekday ".len()..]) else {
                return Ok(None);
            };
            if !(0.0..7.0).contains(&r) || r != r.trunc() {
                return Ok(None);
            }
            let n = r as i64;
            p.compute_ymd_hms();
            p.valid_tz = false;
            p.valid_jd = false;
            p.compute_jd();
            let mut z = ((p.jd + 129_600_000) / MS_DAY) % 7;
            if z > n {
                z -= 7;
            }
            p.jd += (n - z) * MS_DAY;
            p.clear();
            Some(())
        }
        Some(b'+' | b'-' | b'0'..=b'9') => {
            let b = m.as_bytes();
            let n = b
                .iter()
                .position(|c| is_space(*c) || *c == b':')
                .unwrap_or(b.len());
            if b.get(n) == Some(&b':') {
                // [+-]HH:MM[:SS[.FFF]] shifts by a time of day.
                let body = if b[0].is_ascii_digit() { b } else { &b[1..] };
                let Some((h, mi, s, tz)) = time_of_day(body) else {
                    return Ok(None);
                };
                if tz.is_some() {
                    return Ok(None);
                }
                let mut tx = Dt {
                    valid_hms: true,
                    h,
                    min: mi,
                    s,
                    ..Dt::default()
                };
                tx.compute_jd();
                tx.jd -= 43_200_000;
                let day = tx.jd / MS_DAY;
                tx.jd -= day * MS_DAY;
                if b[0] == b'-' {
                    tx.jd = -tx.jd;
                }
                p.compute_jd();
                p.clear();
                p.jd += tx.jd;
                return Ok(Some(()));
            }
            let tok = &m[..n];
            let tb = tok.as_bytes();
            if (1..tb.len()).any(|i| tb[i] == b'-' && !matches!(tb[i - 1], b'e' | b'E')) {
                return Err(unsupported("date offset"));
            }
            let Some(r) = whole_real(tok) else {
                return Ok(None);
            };
            let unit = m[n..].trim_start_matches(|c: char| c.is_ascii_whitespace());
            if unit.len() > 10 || unit.len() < 3 {
                return Ok(None);
            }
            let unit = unit.strip_suffix('s').unwrap_or(unit);
            let limit = match unit {
                "second" => 4.6427e+14,
                "minute" => 7.7379e+12,
                "hour" => 1.2897e+11,
                "day" => 5_373_485.0,
                "month" => 176_546.0,
                "year" => 14_713.0,
                _ => return Ok(None),
            };
            if !(r > -limit && r < limit) {
                return Ok(None);
            }
            let rounder = if r < 0.0 { -0.5 } else { 0.5 };
            if !matches!(unit, "month" | "year") {
                p.floor = 0;
            }
            p.compute_jd();
            match unit {
                "second" => p.jd += (r * 1000.0 + rounder) as i64,
                "minute" => p.jd += (r * 60_000.0 + rounder) as i64,
                "hour" => p.jd += (r * 3_600_000.0 + rounder) as i64,
                "day" => p.jd += (r * MS_DAY as f64 + rounder) as i64,
                "month" => {
                    p.compute_ymd_hms();
                    p.m += r as i32;
                    let x = if p.m > 0 {
                        (p.m - 1) / 12
                    } else {
                        (p.m - 12) / 12
                    };
                    p.y += x;
                    p.m -= x * 12;
                    p.compute_floor();
                    p.valid_jd = false;
                    p.compute_jd();
                    let whole = r as i32;
                    if f64::from(whole) != r {
                        p.jd += ((r - f64::from(whole)) * 30.0 * MS_DAY as f64) as i64;
                    }
                }
                _ => {
                    p.compute_ymd_hms();
                    p.y += r as i32;
                    p.compute_floor();
                    p.valid_jd = false;
                    p.compute_jd();
                    let whole = r as i32;
                    if f64::from(whole) != r {
                        p.jd += ((r - f64::from(whole)) * 365.0 * MS_DAY as f64) as i64;
                    }
                }
            }
            p.clear();
            Some(())
        }
        _ => None,
    })
}

fn evaluate(args: &[Value]) -> Result<Option<Dt>> {
    let mut p = match args.first() {
        None => Dt {
            jd: now_ms(),
            valid_jd: true,
            ..Dt::default()
        },
        Some(v) => match time_value(v) {
            Some(p) => p,
            None => return Ok(None),
        },
    };
    for (i, a) in args.iter().skip(1).enumerate() {
        let Some(t) = a.to_text() else {
            return Ok(None);
        };
        if modify(&mut p, &t, i)?.is_none() {
            return Ok(None);
        }
    }
    p.compute_jd();
    if args.len() == 1 {
        p.valid_ymd = false;
    }
    Ok((!p.error && (0..=MAX_JD).contains(&p.jd)).then_some(p))
}

/// date() and datetime() pad the digits to four places and put the sign in front.
fn year4(y: i32) -> String {
    if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{y:04}")
    }
}

fn text(s: String) -> Value {
    Value::Text(s)
}

pub fn date(args: &[Value]) -> Result<Value> {
    Ok(evaluate(args)?.map_or(Value::Null, |mut p| {
        p.compute_ymd();
        text(format!("{}-{:02}-{:02}", year4(p.y), p.m, p.d))
    }))
}

pub fn time(args: &[Value]) -> Result<Value> {
    Ok(evaluate(args)?.map_or(Value::Null, |mut p| {
        p.compute_hms();
        text(format!("{:02}:{:02}:{:02}", p.h, p.min, p.s as i32))
    }))
}

pub fn datetime(args: &[Value]) -> Result<Value> {
    Ok(evaluate(args)?.map_or(Value::Null, |mut p| {
        p.compute_ymd_hms();
        text(format!(
            "{}-{:02}-{:02} {:02}:{:02}:{:02}",
            year4(p.y),
            p.m,
            p.d,
            p.h,
            p.min,
            p.s as i32
        ))
    }))
}

pub fn julianday(args: &[Value]) -> Result<Value> {
    Ok(evaluate(args)?.map_or(Value::Null, |p| Value::Real(p.jd as f64 / MS_DAY as f64)))
}

pub fn unixepoch(args: &[Value]) -> Result<Value> {
    Ok(evaluate(args)?.map_or(Value::Null, |p| {
        Value::Integer(p.jd / 1000 - UNIX_MS / 1000)
    }))
}

pub fn strftime(args: &[Value]) -> Result<Value> {
    let Some(fmt) = args.first().and_then(|v| match v {
        Value::Text(t) => Some(t.clone()),
        Value::Null => None,
        other => other.to_text(),
    }) else {
        return Ok(Value::Null);
    };
    let Some(mut p) = evaluate(&args[1..])? else {
        return Ok(Value::Null);
    };
    p.compute_ymd_hms();
    let unix = p.jd / 1000 - UNIX_MS / 1000;
    let wday = ((p.jd + 129_600_000) / MS_DAY % 7) as i32;
    let mut jan1 = p;
    jan1.valid_jd = false;
    jan1.valid_tz = false;
    jan1.m = 1;
    jan1.d = 1;
    jan1.compute_jd();
    let yday0 = ((p.jd - jan1.jd + 43_200_000) / MS_DAY) as i32;
    // ISO 8601 week: the Thursday of this week decides the year, derived as SQLite does.
    let mut thu = p;
    thu.valid_ymd = false;
    thu.jd += (3 - (p.jd + 43_200_000) / MS_DAY % 7) * MS_DAY;
    thu.compute_ymd();
    let iso_year = thu.y;
    let mut thu_jan1 = thu;
    thu_jan1.valid_jd = false;
    thu_jan1.m = 1;
    thu_jan1.d = 1;
    thu_jan1.compute_jd();
    let iso_week = ((thu.jd - thu_jan1.jd + 43_200_000) / MS_DAY / 7 + 1) as i32;
    let hour12 = if p.h % 12 == 0 { 12 } else { p.h % 12 };
    let mut out = String::new();
    let mut it = fmt.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(f) = it.next() else {
            return Ok(Value::Null);
        };
        match f {
            'd' => out.push_str(&format!("{:02}", p.d)),
            'e' => out.push_str(&format!("{:2}", p.d)),
            'f' => {
                let s = p.s.min(59.999);
                out.push_str(&format!("{s:06.3}"));
            }
            'F' => out.push_str(&format!("{:04}-{:02}-{:02}", p.y, p.m, p.d)),
            'G' => out.push_str(&format!("{iso_year:04}")),
            'g' => out.push_str(&format!("{:02}", iso_year % 100)),
            'H' => out.push_str(&format!("{:02}", p.h)),
            'I' => out.push_str(&format!("{hour12:02}")),
            'j' => out.push_str(&format!("{:03}", yday0 + 1)),
            'J' => {
                return Err(Error::syntax(
                    "the strftime() format %J is not supported by this store",
                ));
            }
            'k' => out.push_str(&format!("{:2}", p.h)),
            'l' => out.push_str(&format!("{hour12:2}")),
            'm' => out.push_str(&format!("{:02}", p.m)),
            'M' => out.push_str(&format!("{:02}", p.min)),
            'p' => out.push_str(if p.h >= 12 { "PM" } else { "AM" }),
            'P' => out.push_str(if p.h >= 12 { "pm" } else { "am" }),
            'R' => out.push_str(&format!("{:02}:{:02}", p.h, p.min)),
            's' => out.push_str(&unix.to_string()),
            'S' => out.push_str(&format!("{:02}", p.s as i32)),
            'T' => out.push_str(&format!("{:02}:{:02}:{:02}", p.h, p.min, p.s as i32)),
            'u' => out.push_str(&(if wday == 0 { 7 } else { wday }).to_string()),
            'U' => out.push_str(&format!("{:02}", (yday0 + 7 - wday) / 7)),
            'V' => out.push_str(&format!("{iso_week:02}")),
            'w' => out.push_str(&wday.to_string()),
            'W' => out.push_str(&format!("{:02}", (yday0 + 7 - (wday + 6) % 7) / 7)),
            'Y' => out.push_str(&format!("{:04}", p.y)),
            '%' => out.push('%'),
            _ => return Ok(Value::Null),
        }
    }
    if p.error || thu.error || thu_jan1.error {
        return Ok(Value::Null);
    }
    Ok(Value::Text(out))
}
