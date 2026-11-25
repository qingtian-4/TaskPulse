use chrono::{DateTime, NaiveDateTime};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Native events carry RFC 3339 times. Older status writers omit the zone and use UTC.
pub(super) fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    let utc = DateTime::parse_from_rfc3339(text)
        .map(|dt| dt.to_utc())
        .or_else(|_| text.parse::<NaiveDateTime>().map(|dt| dt.and_utc()))
        .ok()?;
    let fraction = Duration::from_nanos(u64::from(utc.timestamp_subsec_nanos()));
    let seconds = utc.timestamp();
    let epoch = UNIX_EPOCH.checked_add(fraction)?;
    if seconds < 0 {
        epoch.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
    } else {
        epoch.checked_add(Duration::from_secs(seconds as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_and_legacy_utc_preserve_subsecond_event_order() {
        let utc = parse_rfc3339("2026-01-01T00:00:00.125Z").unwrap();
        for equivalent in [
            "2026-01-01T08:00:00.125+08:00",
            "2025-12-31T18:30:00.125-05:30",
            "2026-01-01T00:00:00.125",
        ] {
            assert_eq!(parse_rfc3339(equivalent), Some(utc));
        }
        assert_eq!(
            parse_rfc3339("1969-12-31T23:59:59.750Z"),
            UNIX_EPOCH.checked_sub(Duration::from_millis(250))
        );
    }

    #[test]
    fn invalid_calendar_dates_offsets_and_unicode_do_not_become_event_times() {
        for invalid in [
            "2026-02-30T12:34:56Z",
            "2026-12-01T24:00:01Z",
            "2026-12-01T12:34:56+08:70",
            "2026-12-01T12:34:56.125unknown",
            "中年-01-01T12:34:56Z",
        ] {
            assert!(parse_rfc3339(invalid).is_none(), "{invalid}");
        }
    }
}
