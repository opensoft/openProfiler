use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, in UTC.
pub fn now_unix_seconds() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() as i64,
        Err(error) => -(error.duration().as_secs() as i64),
    }
}

/// Renders an instant as an RFC 3339 UTC timestamp, second resolution.
///
/// Hand-rolled rather than pulled from a date crate because the whole need is
/// one civil-date conversion, and this crate's dependency surface is part of
/// its security argument.
pub fn format_rfc3339_utc(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let second_of_day = unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's `civil_from_days`, for a proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_position + 2) / 5 + 1) as u32;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_epoch() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn renders_known_instants() {
        assert_eq!(format_rfc3339_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(format_rfc3339_utc(1_767_225_599), "2025-12-31T23:59:59Z");
        assert_eq!(format_rfc3339_utc(1_767_225_600), "2026-01-01T00:00:00Z");
    }

    #[test]
    fn renders_a_leap_day() {
        assert_eq!(format_rfc3339_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn renders_instants_before_the_epoch() {
        assert_eq!(format_rfc3339_utc(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn now_is_after_the_change_that_introduced_this_crate() {
        assert!(now_unix_seconds() > 1_767_225_600);
    }
}
