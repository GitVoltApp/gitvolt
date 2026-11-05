//! Times and sizes as text (`ui/src/format/relative.ts`, `ui/src/diff/format.ts`, `ui/src/diff/hex.tsx`).

const UNITS: [(&str, &str, f64); 6] = [("year", "y", 31_536_000.), ("month", "mo", 2_592_000.), ("week", "w", 604_800.), ("day", "d", 86_400.), ("hour", "h", 3600.), ("minute", "m", 60.)];

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

/// JavaScript's `Math.round`: halves go up, also below zero.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `Intl.RelativeTimeFormat('en', { numeric: 'auto' })` for a whole number of a unit.
fn relative(n: i64, unit: &str) -> String {
    match (n, unit) {
        (0, "second") => return "now".into(),
        (0, u) => return format!("this {u}"),
        (-1, "day") => return "yesterday".into(),
        (1, "day") => return "tomorrow".into(),
        (-1, "week" | "month" | "year") => return format!("last {unit}"),
        (1, "week" | "month" | "year") => return format!("next {unit}"),
        _ => {}
    }
    let abs = n.abs();
    let plural = if abs == 1 { "" } else { "s" };
    if n < 0 { format!("{abs} {unit}{plural} ago") } else { format!("in {abs} {unit}{plural}") }
}

/// "3 days ago" from unix seconds.
pub fn relative_time(unix_seconds: i64, now: f64) -> String {
    let diff = unix_seconds as f64 - now;
    for (unit, _, secs) in UNITS {
        if diff.abs() >= secs {
            return relative(js_round(diff / secs) as i64, unit);
        }
    }
    "just now".into()
}

/// The compact form, for narrow columns: "now", "5m", "3h", "2d", "3w", "9mo", "2y" (a time ahead of
/// `now`, clock skew, reads as its distance).
pub fn compact_relative_time(unix_seconds: i64, now: f64) -> String {
    let diff = (now - unix_seconds as f64).abs();
    for (_, short, secs) in UNITS {
        if diff >= secs {
            return format!("{}{short}", js_round(diff / secs) as i64);
        }
    }
    "now".into()
}

/// "512 B", "106.7 KB", "4.2 MB"; "—" with no size.
pub fn format_bytes(n: Option<u64>) -> String {
    match n {
        None => "—".into(),
        Some(n) if n < 1024 => format!("{n} B"),
        Some(n) if n < 1024 * 1024 => format!("{:.1} KB", n as f64 / 1024.),
        Some(n) => format!("{:.1} MB", n as f64 / (1024. * 1024.)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn relative_time_reads_as_the_web_uis() {
        let t = 1_000_000.;
        let ago = |s: i64| relative_time(t as i64 - s, t);
        assert_eq!(ago(3 * 86_400), "3 days ago");
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(7200), "2 hours ago");
        assert_eq!(ago(45 * 60), "45 minutes ago");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(3600), "1 hour ago");
        assert_eq!(ago(86_400), "yesterday");
        assert_eq!(ago(604_800), "last week");
        assert_eq!(ago(2_592_000), "last month");
        assert_eq!(ago(31_536_000), "last year");
        assert_eq!(relative_time(t as i64 + 7200, t), "in 2 hours");
        assert_eq!(relative_time(t as i64 + 86_400, t), "tomorrow");
    }

    #[test]
    fn rounding_is_javascripts() {
        // -1.5 rounds up, to -1: "yesterday", not "2 days ago".
        assert_eq!(relative_time(1_000_000 - 129_600, 1_000_000.), "yesterday");
    }

    #[test]
    fn compact_is_now_under_a_minute_then_each_unit_from_its_first_whole_one() {
        let now = 100_000_000.;
        let ago = |s: i64| compact_relative_time(now as i64 - s, now);
        for (secs, text) in [(0, "now"), (59, "now"), (60, "1m"), (5 * 60, "5m"), (3599, "60m"), (3600, "1h"), (86_399, "24h"), (86_400, "1d"), (604_799, "7d"), (604_800, "1w"), (2_591_999, "4w"), (2_592_000, "1mo"), (31_535_999, "12mo"), (31_536_000, "1y")] {
            assert_eq!(ago(secs), text, "{secs}");
        }
        assert_eq!(compact_relative_time(now as i64 + 300, now), "5m", "ahead: its distance");
    }
}
