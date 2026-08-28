use chrono::Duration;

/// Format a duration as "Xh Ym"
pub fn format(dur: Duration) -> String {
    let hours = dur.num_hours();
    let minutes = dur.num_minutes() % 60;
    format!("{}h {}m", hours, minutes)
}

/// Leading run of ASCII digits, plus what follows it.
fn take_number(s: &str) -> Option<(i64, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let value = s[..end].parse().ok()?;
    Some((value, &s[end..]))
}

/// Parse a duration string: `90` (bare minutes), `45m`, `2h`, or `1h30m`. The
/// space [`format`] writes is allowed, so `1h 30m` round-trips.
///
/// Anything else — a sign, stray characters, an empty string, a number too big
/// for a `Duration` — is `None`, never a silently truncated value.
pub fn parse(duration_str: &str) -> Option<Duration> {
    let input = duration_str.trim();
    let (first, rest) = take_number(input)?;

    match rest.as_bytes().first() {
        None => Duration::try_minutes(first),
        Some(b'h' | b'H') => {
            let rest = rest[1..].trim_start();
            if rest.is_empty() {
                return Duration::try_hours(first);
            }
            let (mins, rest) = take_number(rest)?;
            if !matches!(rest, "m" | "M") {
                return None;
            }
            Duration::try_hours(first)?.checked_add(&Duration::try_minutes(mins)?)
        }
        Some(b'm' | b'M') if rest.len() == 1 => Duration::try_minutes(first),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_forms_all_parse() {
        assert_eq!(parse("2h"), Some(Duration::hours(2)));
        assert_eq!(parse("45m"), Some(Duration::minutes(45)));
        assert_eq!(parse("1h30m"), Some(Duration::minutes(90)));
        assert_eq!(parse("90"), Some(Duration::minutes(90)), "bare = minutes");
        assert_eq!(parse("0m"), Some(Duration::zero()));
    }

    #[test]
    fn the_unit_letters_are_case_insensitive() {
        assert_eq!(parse("2H"), Some(Duration::hours(2)));
        assert_eq!(parse("45M"), Some(Duration::minutes(45)));
        assert_eq!(parse("1H30M"), Some(Duration::minutes(90)));
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        assert_eq!(parse("  1h30m \n"), Some(Duration::minutes(90)));
    }

    /// The TUI seeds and rewrites its Duration field with `format` output, so
    /// its spacing has to parse back.
    #[test]
    fn format_output_parses_back() {
        for minutes in [0, 45, 60, 95, 1440] {
            let dur = Duration::minutes(minutes);
            assert_eq!(parse(&format(dur)), Some(dur), "{minutes}m round trip");
        }
    }

    #[test]
    fn minutes_beyond_an_hour_are_kept_as_written() {
        assert_eq!(parse("1h90m"), Some(Duration::minutes(150)));
    }

    /// The point of #41: unparseable input is an error, not a zero or a
    /// partially honoured value.
    #[test]
    fn unparseable_input_is_rejected_rather_than_truncated() {
        for bad in [
            "",
            "   ",
            "hello",
            "1x30",
            "-45m",
            "+45m",
            "1.5h",
            "1h30",
            "30mm",
            "m",
            "h",
            "1m30h",
            "1h30s",
            "90 minutes",
        ] {
            assert_eq!(parse(bad), None, "`{bad}` parsed instead of failing");
        }
    }

    #[test]
    fn a_number_too_large_for_a_duration_is_rejected() {
        assert_eq!(parse("999999999999999999999"), None, "overflows i64");
        assert_eq!(parse("9223372036854775807h"), None, "overflows Duration");
    }
}
