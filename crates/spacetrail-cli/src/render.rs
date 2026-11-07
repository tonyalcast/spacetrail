//! Human output helpers: sizes, counts, durations, bars and aligned tables.

use std::fmt::Write as _;
use std::time::Duration;

use spacetrail_core::{HumanBytes, UnitSystem};

/// Output settings shared by every command.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Style {
    pub(crate) units: UnitSystem,
}

impl Style {
    pub(crate) fn bytes(self, n: u64) -> String {
        HumanBytes(n, self.units).to_string()
    }
}

/// `1234567` → `1,234,567`.
pub(crate) fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `72.4s` → `1m 12s`.
pub(crate) fn duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

/// Percentage of `part` in `whole`, as an integer string with `%`.
pub(crate) fn pct(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "–".to_owned();
    }
    let p = (u128::from(part) * 100 + u128::from(whole) / 2) / u128::from(whole);
    format!("{p}%")
}

/// A fixed-width bar made of eighth blocks.
pub(crate) fn bar(part: u64, whole: u64, width: usize) -> String {
    const EIGHTHS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    if whole == 0 {
        return " ".repeat(width);
    }
    let units = u128::from(part.min(whole)) * (width as u128) * 8 / u128::from(whole);
    let units = usize::try_from(units).unwrap_or(width * 8);
    let mut s = "█".repeat(units / 8);
    if units % 8 > 0 {
        s.push(EIGHTHS[units % 8 - 1]);
    }
    let len = s.chars().count();
    s.push_str(&" ".repeat(width.saturating_sub(len)));
    s
}

/// Column alignment for [`table`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Right,
}

/// Renders rows as space-separated, aligned columns (no trailing spaces).
pub(crate) fn table(rows: &[Vec<String>], align: &[Align], indent: usize) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        let mut line = " ".repeat(indent);
        for (c, cell) in row.iter().enumerate() {
            let w = widths[c];
            let pad = w - cell.chars().count();
            let a = align.get(c).copied().unwrap_or(Align::Left);
            if c > 0 {
                line.push_str("  ");
            }
            if a == Align::Right {
                line.push_str(&" ".repeat(pad));
                line.push_str(cell);
            } else {
                line.push_str(cell);
                line.push_str(&" ".repeat(pad));
            }
        }
        let _ = writeln!(out, "{}", line.trim_end());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(count(0), "0");
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(count(100), "100");
        assert_eq!(duration(Duration::from_millis(1500)), "1.5s");
        assert_eq!(duration(Duration::from_secs(72)), "1m 12s");
        assert_eq!(pct(1, 3), "33%");
        assert_eq!(pct(1, 0), "–");
        assert_eq!(bar(1, 2, 4), "██  ");
        assert_eq!(bar(1, 16, 2), "▏ ");
        assert_eq!(bar(5, 4, 3), "███");
    }

    #[test]
    fn aligned_table() {
        let rows = vec![
            vec!["a".to_owned(), "1 GB".to_owned()],
            vec!["long name".to_owned(), "10 GB".to_owned()],
        ];
        let t = table(&rows, &[Align::Left, Align::Right], 2);
        assert_eq!(t, "  a           1 GB\n  long name  10 GB\n");
    }
}
