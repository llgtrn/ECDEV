//! Platform-displayed counters and relative times ("1.2万", "10w+", "3分钟前"). A displayed
//! value is an abbreviation, not a measurement: it keeps its unit, its resolution and the interval
//! it can stand for. Platforms either truncate or round when abbreviating and do not say which, so
//! the interval covers both. Months and years have no fixed length and are refused, as are words
//! without a stated quantity ("刚刚", "昨天").

use serde::Serialize;

pub const DISPLAY_BASIS: &str = "TRUNCATED_OR_ROUNDED_DISPLAY";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DisplayedCount {
    pub display: String,
    pub unit: Option<String>,
    pub multiplier: u64,
    pub nominal: u64,
    /// The smallest step the display can express.
    pub resolution: u64,
    /// "+" suffix: the display is a floor.
    pub at_least: bool,
    pub lower: u64,
    /// None when the display is a floor.
    pub upper: Option<u64>,
    pub basis: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DisplayedTime {
    pub display: String,
    pub unit: String,
    pub unit_seconds: u64,
    pub quantity: u64,
    pub earliest: u64,
    pub latest: u64,
    pub captured_at: u64,
    pub basis: &'static str,
}

const COUNT_UNITS: &[(&str, u64)] = &[
    ("万", 10_000),
    ("萬", 10_000),
    ("w", 10_000),
    ("W", 10_000),
    ("亿", 100_000_000),
    ("億", 100_000_000),
    ("千", 1_000),
    ("k", 1_000),
    ("K", 1_000),
    ("M", 1_000_000),
    ("B", 1_000_000_000),
];

const AGE_UNITS: &[(&str, u64)] = &[
    ("秒钟", 1),
    ("秒", 1),
    ("分钟", 60),
    ("分鐘", 60),
    ("分", 60),
    ("个小时", 3_600),
    ("個小時", 3_600),
    ("小时", 3_600),
    ("小時", 3_600),
    ("天", 86_400),
    ("日", 86_400),
    ("周", 604_800),
    ("週", 604_800),
    ("星期", 604_800),
];

fn quantity(s: &str, allow_grouping: bool) -> Option<(u64, u32, u64)> {
    let s = if allow_grouping && s.contains(',') {
        let groups: Vec<&str> = s.split(',').collect();
        let ok = !groups[0].is_empty()
            && groups[0].len() <= 3
            && groups[1..].iter().all(|g| g.len() == 3);
        if !ok {
            return None;
        }
        groups.concat()
    } else {
        s.to_string()
    };
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    if int.is_empty()
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
        || (s.contains('.') && frac.is_empty())
    {
        return None;
    }
    let digits = frac.len() as u32;
    let frac_value = if frac.is_empty() {
        0
    } else {
        frac.parse().ok()?
    };
    Some((int.parse().ok()?, digits, frac_value))
}

/// Parses a platform-displayed counter. None when the text is not a counter this function can
/// bound (unknown unit, more decimals than the unit can carry, grouping with a unit, overflow).
pub fn displayed_count(text: &str) -> Option<DisplayedCount> {
    let display = text.trim();
    let (body, at_least) = match display.strip_suffix('+') {
        Some(b) => (b.trim_end(), true),
        None => (display, false),
    };
    let (number, unit, multiplier) = COUNT_UNITS
        .iter()
        .find_map(|(u, m)| body.strip_suffix(u).map(|n| (n.trim_end(), Some(*u), *m)))
        .unwrap_or((body, None, 1));
    let (int, digits, frac) = quantity(number, unit.is_none())?;
    let scale = 10u64.checked_pow(digits)?;
    if multiplier % scale != 0 {
        return None;
    }
    let resolution = multiplier / scale;
    let nominal = int
        .checked_mul(multiplier)?
        .checked_add(frac * resolution)?;
    let (lower, upper) = if at_least {
        (nominal, None)
    } else {
        (
            nominal.saturating_sub(resolution / 2),
            Some(nominal.checked_add(resolution - 1)?),
        )
    };
    Some(DisplayedCount {
        display: display.to_string(),
        unit: unit.map(str::to_string),
        multiplier,
        nominal,
        resolution,
        at_least,
        lower,
        upper,
        basis: DISPLAY_BASIS,
    })
}

/// Parses "<N><unit>前" against the capture time into the publication interval it allows.
pub fn displayed_age(text: &str, captured_at: u64) -> Option<DisplayedTime> {
    let display = text.trim();
    let body = display.strip_suffix('前')?.trim_end();
    let (number, unit, seconds) = AGE_UNITS
        .iter()
        .find_map(|(u, s)| body.strip_suffix(u).map(|n| (n.trim_end(), *u, *s)))?;
    let (n, digits, _) = quantity(number, false)?;
    if digits != 0 {
        return None;
    }
    let shortest = n.checked_mul(seconds)?.saturating_sub(seconds / 2);
    let longest = n.checked_add(1)?.checked_mul(seconds)?;
    Some(DisplayedTime {
        display: display.to_string(),
        unit: unit.to_string(),
        unit_seconds: seconds,
        quantity: n,
        earliest: captured_at.saturating_sub(longest),
        latest: captured_at.saturating_sub(shortest),
        captured_at,
        basis: DISPLAY_BASIS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviated_counts_keep_unit_and_resolution() {
        let c = displayed_count("1.2万").unwrap();
        assert_eq!(
            (c.nominal, c.resolution, c.lower, c.upper),
            (12_000, 1_000, 11_500, Some(12_999))
        );
        assert_eq!(c.unit.as_deref(), Some("万"));
        let w = displayed_count("10w+").unwrap();
        assert_eq!(
            (w.nominal, w.lower, w.upper, w.at_least),
            (100_000, 100_000, None, true)
        );
        let y = displayed_count("3.05亿").unwrap();
        assert_eq!((y.nominal, y.resolution), (305_000_000, 1_000_000));
        let k = displayed_count("1.5k").unwrap();
        assert_eq!((k.nominal, k.lower, k.upper), (1_500, 1_450, Some(1_599)));
    }

    #[test]
    fn exact_counts_are_exact() {
        let c = displayed_count("1,234").unwrap();
        assert_eq!(
            (c.nominal, c.lower, c.upper, c.unit),
            (1_234, 1_234, Some(1_234), None)
        );
        assert_eq!(displayed_count("0").unwrap().upper, Some(0));
    }

    #[test]
    fn unbounded_or_malformed_counts_are_refused() {
        for t in [
            "",
            "万",
            "1.万",
            "1.23456k",
            "12,34",
            "1,234万",
            "-3",
            "1.2",
            "约1万",
            "1e3",
            "99999999999999999999",
        ] {
            assert!(displayed_count(t).is_none(), "{t}");
        }
    }

    #[test]
    fn relative_ages_bound_the_publication_time() {
        let t = displayed_age("3分钟前", 10_000).unwrap();
        assert_eq!((t.earliest, t.latest), (10_000 - 240, 10_000 - 150));
        let h = displayed_age("2 小时前", 100_000).unwrap();
        assert_eq!(
            (h.earliest, h.latest, h.unit_seconds),
            (100_000 - 10_800, 100_000 - 5_400, 3_600)
        );
        for t in [
            "刚刚",
            "昨天",
            "3个月前",
            "1年前",
            "1.5小时前",
            "3分钟",
            "分钟前",
        ] {
            assert!(displayed_age(t, 10_000).is_none(), "{t}");
        }
    }
}
