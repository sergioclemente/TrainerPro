//! Calendar-date helpers shared by schedule projection and provider sync.

use chrono::{DateTime, Local, Utc};
use chrono_tz::Tz;

/// Resolve the calendar date for an instant in the planning authority's IANA
/// time zone, falling back to the machine's local zone when there is no active
/// authority or older stored metadata contains an unknown zone.
pub fn today_at(now_utc: DateTime<Utc>, time_zone: Option<&str>) -> String {
    if let Some(time_zone) = time_zone.and_then(|value| value.parse::<Tz>().ok()) {
        return now_utc.with_timezone(&time_zone).date_naive().to_string();
    }
    now_utc.with_timezone(&Local).date_naive().to_string()
}

pub fn shift(value: &str, days: i64) -> Option<String> {
    let (year, month, day) = parse(value)?;
    Some(civil_from_days(
        days_from_civil(year, month, day).checked_add(days)?,
    ))
}

fn parse(value: &str) -> Option<(i32, u32, u32)> {
    if value.len() != 10 || &value[4..5] != "-" || &value[7..8] != "-" {
        return None;
    }
    let year = value[0..4].parse::<i32>().ok()?;
    let month = value[5..7].parse::<u32>().ok()?;
    let day = value[8..10].parse::<u32>().ok()?;
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return None,
    };
    (year >= 1 && day >= 1 && day <= max_day).then_some((year, month, day))
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn today_uses_the_planning_zone_with_machine_local_fallback() {
        let now_utc = Utc.with_ymd_and_hms(2026, 9, 20, 22, 30, 0).unwrap();
        assert_eq!(today_at(now_utc, Some("Europe/Zurich")), "2026-09-21");
        assert_eq!(today_at(now_utc, Some("America/New_York")), "2026-09-20");
        assert_eq!(
            today_at(now_utc, Some("not/a-zone")),
            today_at(now_utc, None)
        );
    }

    #[test]
    fn shift_crosses_month_year_and_leap_day_boundaries() {
        assert_eq!(shift("2028-02-28", 1).as_deref(), Some("2028-02-29"));
        assert_eq!(shift("2028-03-01", -1).as_deref(), Some("2028-02-29"));
        assert_eq!(shift("2026-01-03", -7).as_deref(), Some("2025-12-27"));
    }

    #[test]
    fn shift_rejects_nonexistent_dates() {
        assert_eq!(shift("2026-02-29", 1), None);
        assert_eq!(shift("2026-13-01", 1), None);
        assert_eq!(shift("20260917", 1), None);
    }
}
