//! Byte sizes and durations: human formatting and parsing.
//!
//! Sizes display in decimal (SI) units by default to match Finder; IEC
//! (binary) units are available for users who prefer them.

use std::fmt;
use std::time::Duration;

/// Unit system used when formatting byte sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnitSystem {
    /// Powers of 1000 (kB, MB, GB…). Matches Finder.
    #[default]
    Decimal,
    /// Powers of 1024 (KiB, MiB, GiB…).
    Binary,
}

/// Wrapper that formats a byte count for humans, e.g. `1.23 GB`.
#[derive(Debug, Clone, Copy)]
pub struct HumanBytes(pub u64, pub UnitSystem);

impl fmt::Display for HumanBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (base, units): (f64, [&str; 7]) = match self.1 {
            UnitSystem::Decimal => (1000.0, ["B", "kB", "MB", "GB", "TB", "PB", "EB"]),
            UnitSystem::Binary => (1024.0, ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"]),
        };
        if self.0 < base as u64 {
            return write!(f, "{} B", self.0);
        }
        let mut value = self.0 as f64;
        let mut unit = 0;
        while value >= base && unit < units.len() - 1 {
            value /= base;
            unit += 1;
        }
        // Avoid printing "1000 MB" when rounding pushes the value to the next unit.
        if value >= 999.5 && unit < units.len() - 1 {
            value /= base;
            unit += 1;
        }
        let decimals = if value < 10.0 {
            2
        } else if value < 100.0 {
            1
        } else {
            0
        };
        write!(f, "{value:.decimals$} {}", units[unit])
    }
}

/// Formats bytes in decimal units (`HumanBytes(n, Decimal)`).
#[must_use]
pub fn human(bytes: u64) -> HumanBytes {
    HumanBytes(bytes, UnitSystem::Decimal)
}

/// Error returned when parsing a size or duration fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what} `{input}`: {reason}")]
pub struct ParseError {
    what: &'static str,
    input: String,
    reason: &'static str,
}

impl ParseError {
    fn new(what: &'static str, input: &str, reason: &'static str) -> Self {
        Self {
            what,
            input: input.to_owned(),
            reason,
        }
    }
}

/// Parses a size such as `512`, `5MB`, `1.5 GB`, `10KiB`, `2g`.
///
/// Suffixes without `i` are decimal (`k`/`kB` = 1000); suffixes with `i` are
/// binary (`KiB` = 1024). A bare number is bytes. Case-insensitive.
///
/// # Errors
/// Returns [`ParseError`] for empty input, unknown suffixes, negative or
/// overflowing values.
pub fn parse_size(input: &str) -> Result<u64, ParseError> {
    let err = |reason| ParseError::new("size", input, reason);
    let s = input.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, suffix) = s.split_at(split);
    if num.is_empty() {
        return Err(err("missing number"));
    }
    let suffix = suffix.trim().to_ascii_lowercase();
    let multiplier: u64 = match suffix.as_str() {
        "" | "b" => 1,
        "k" | "kb" => 1_000,
        "m" | "mb" => 1_000_000,
        "g" | "gb" => 1_000_000_000,
        "t" | "tb" => 1_000_000_000_000,
        "ki" | "kib" => 1 << 10,
        "mi" | "mib" => 1 << 20,
        "gi" | "gib" => 1 << 30,
        "ti" | "tib" => 1 << 40,
        _ => return Err(err("unknown unit")),
    };
    if let Ok(n) = num.parse::<u64>() {
        return n.checked_mul(multiplier).ok_or_else(|| err("too large"));
    }
    let f: f64 = num.parse().map_err(|_| err("not a number"))?;
    let bytes = f * multiplier as f64;
    if !bytes.is_finite() || bytes < 0.0 || bytes > u64::MAX as f64 {
        return Err(err("out of range"));
    }
    Ok(bytes.round() as u64)
}

/// Parses a duration such as `90s`, `30m`, `12h`, `7d`, `2w`, or a
/// combination like `1d12h`. A bare `0` is accepted.
///
/// # Errors
/// Returns [`ParseError`] for empty input, missing units or unknown units.
pub fn parse_duration(input: &str) -> Result<Duration, ParseError> {
    let err = |reason| ParseError::new("duration", input, reason);
    let s = input.trim();
    if s.is_empty() {
        return Err(err("empty"));
    }
    if s == "0" {
        return Ok(Duration::ZERO);
    }
    let mut total: u64 = 0;
    let mut rest = s;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(|| err("missing unit"))?;
        if digits == 0 {
            return Err(err("expected a number"));
        }
        let n: u64 = rest[..digits]
            .parse()
            .map_err(|_| err("number too large"))?;
        let unit_len = rest[digits..]
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(rest.len() - digits);
        let secs_per: u64 = match &rest[digits..digits + unit_len] {
            "s" => 1,
            "m" | "min" => 60,
            "h" => 3_600,
            "d" => 86_400,
            "w" => 604_800,
            _ => return Err(err("unknown unit (use s, m, h, d, w)")),
        };
        total = n
            .checked_mul(secs_per)
            .and_then(|v| total.checked_add(v))
            .ok_or_else(|| err("too large"))?;
        rest = &rest[digits + unit_len..];
    }
    Ok(Duration::from_secs(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_decimal() {
        assert_eq!(human(0).to_string(), "0 B");
        assert_eq!(human(999).to_string(), "999 B");
        assert_eq!(human(1_000).to_string(), "1.00 kB");
        assert_eq!(human(1_234_567).to_string(), "1.23 MB");
        assert_eq!(human(45_600_000_000).to_string(), "45.6 GB");
        assert_eq!(human(405_278_142_464).to_string(), "405 GB");
        assert_eq!(human(999_999_999).to_string(), "1.00 GB");
    }

    #[test]
    fn formats_binary() {
        let b = |n| HumanBytes(n, UnitSystem::Binary).to_string();
        assert_eq!(b(1023), "1023 B");
        assert_eq!(b(1024), "1.00 KiB");
        assert_eq!(b(3 << 30), "3.00 GiB");
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("512"), Ok(512));
        assert_eq!(parse_size("5MB"), Ok(5_000_000));
        assert_eq!(parse_size("1.5 GB"), Ok(1_500_000_000));
        assert_eq!(parse_size("10KiB"), Ok(10_240));
        assert_eq!(parse_size("2g"), Ok(2_000_000_000));
        assert!(parse_size("").is_err());
        assert!(parse_size("MB").is_err());
        assert!(parse_size("5 parsecs").is_err());
        assert!(parse_size("-1").is_err());
        assert!(parse_size("99999999999TB").is_err());
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("0"), Ok(Duration::ZERO));
        assert_eq!(parse_duration("0d"), Ok(Duration::ZERO));
        assert_eq!(parse_duration("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(parse_duration("7d"), Ok(Duration::from_secs(7 * 86_400)));
        assert_eq!(parse_duration("1d12h"), Ok(Duration::from_secs(36 * 3_600)));
        assert_eq!(parse_duration("2w"), Ok(Duration::from_secs(14 * 86_400)));
        assert!(parse_duration("").is_err());
        assert!(parse_duration("7").is_err());
        assert!(parse_duration("d").is_err());
        assert!(parse_duration("3y").is_err());
    }
}
