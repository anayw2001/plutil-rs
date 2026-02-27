//! ISO 8601 ↔ Apple-epoch (2001-01-01T00:00:00Z) helpers.
//!
//! The Apple epoch is 978307200 seconds after the Unix epoch.

/// Unix timestamp for 2001-01-01T00:00:00Z.
const APPLE_EPOCH_UNIX: i64 = 978_307_200;

// ── Days-in-month helpers ───────────────────────────────────────────────────

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

// ── Unix timestamp ↔ calendar fields ──────────────────────────────────────

/// Convert a Unix timestamp (seconds) to (year, month, day, hour, min, sec).
/// Handles dates from roughly year 1 to 9999 without overflow.
fn unix_to_calendar(unix_secs: i64) -> (i64, u8, u8, u8, u8, u8) {
    let secs_per_day: i64 = 86_400;
    let mut days = unix_secs.div_euclid(secs_per_day);
    let time = unix_secs.rem_euclid(secs_per_day);

    let hour = (time / 3600) as u8;
    let min = ((time % 3600) / 60) as u8;
    let sec = (time % 60) as u8;

    // Days since 1970-01-01; convert to year/month/day using a simple loop.
    // Start from Unix epoch year.
    let mut year: i64 = 1970;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let mut month: u8 = 1;
    loop {
        let dim = days_in_month(year, month) as i64;
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }

    let day = (days + 1) as u8;
    (year, month, day, hour, min, sec)
}

/// Convert (year, month, day, hour, min, sec) to a Unix timestamp.
fn calendar_to_unix(year: i64, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> i64 {
    // Days from 1970-01-01 to the start of `year`.
    let mut days: i64 = 0;
    if year >= 1970 {
        for y in 1970..year {
            days += if is_leap(y) { 366 } else { 365 };
        }
    } else {
        for y in year..1970 {
            days -= if is_leap(y) { 366 } else { 365 };
        }
    }

    // Add days within the year.
    for m in 1..month {
        days += days_in_month(year, m) as i64;
    }
    days += (day as i64) - 1;

    days * 86_400 + (hour as i64) * 3600 + (min as i64) * 60 + sec as i64
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Convert Apple-epoch seconds (f64) to an ISO 8601 string (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn apple_epoch_to_iso8601(seconds: f64) -> String {
    let unix = seconds as i64 + APPLE_EPOCH_UNIX;
    let (y, mo, d, h, mi, s) = unix_to_calendar(unix);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Parse an ISO 8601 string (`YYYY-MM-DDTHH:MM:SSZ` or `YYYY-MM-DDTHH:MM:SS`)
/// into Apple-epoch seconds (f64).  Returns `Err(())` on parse failure.
pub fn iso8601_to_apple_epoch(s: &str) -> Result<f64, ()> {
    // Accept at minimum "YYYY-MM-DDTHH:MM:SS" (19 chars) with optional trailing 'Z'.
    let s = s.trim();
    let s = s.strip_suffix('Z').unwrap_or(s);
    if s.len() < 19 {
        return Err(());
    }

    let year: i64 = s[0..4].parse().map_err(|_| ())?;
    if s.as_bytes()[4] != b'-' {
        return Err(());
    }
    let month: u8 = s[5..7].parse().map_err(|_| ())?;
    if s.as_bytes()[7] != b'-' {
        return Err(());
    }
    let day: u8 = s[8..10].parse().map_err(|_| ())?;
    let sep = s.as_bytes()[10];
    if sep != b'T' && sep != b't' && sep != b' ' {
        return Err(());
    }
    let hour: u8 = s[11..13].parse().map_err(|_| ())?;
    if s.as_bytes()[13] != b':' {
        return Err(());
    }
    let min: u8 = s[14..16].parse().map_err(|_| ())?;
    if s.as_bytes()[16] != b':' {
        return Err(());
    }
    let sec: u8 = s[17..19].parse().map_err(|_| ())?;

    if month == 0 || month > 12 || day == 0 || day > 31 || hour > 23 || min > 59 || sec > 59 {
        return Err(());
    }

    let unix = calendar_to_unix(year, month, day, hour, min, sec);
    Ok((unix - APPLE_EPOCH_UNIX) as f64)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_epoch_roundtrip() {
        // 0.0 Apple epoch = 2001-01-01T00:00:00Z
        assert_eq!(apple_epoch_to_iso8601(0.0), "2001-01-01T00:00:00Z");
        assert_eq!(iso8601_to_apple_epoch("2001-01-01T00:00:00Z").unwrap(), 0.0);
    }

    #[test]
    fn known_dates() {
        // 2024-07-04T12:00:00Z  →  Apple epoch = ?
        let apple = iso8601_to_apple_epoch("2024-07-04T12:00:00Z").unwrap();
        let back = apple_epoch_to_iso8601(apple);
        assert_eq!(back, "2024-07-04T12:00:00Z");
    }

    #[test]
    fn negative_apple_epoch() {
        // Dates before 2001 have negative Apple-epoch values.
        let apple = iso8601_to_apple_epoch("2000-01-01T00:00:00Z").unwrap();
        assert!(apple < 0.0);
        let back = apple_epoch_to_iso8601(apple);
        assert_eq!(back, "2000-01-01T00:00:00Z");
    }

    #[test]
    fn invalid_date_string() {
        assert!(iso8601_to_apple_epoch("not-a-date").is_err());
        assert!(iso8601_to_apple_epoch("").is_err());
    }
}
