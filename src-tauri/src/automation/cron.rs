//! Minimal standard 5-field cron parser/matcher.
//!
//! We deliberately don't pull in an external cron crate: the grammar we need is small and
//! fully specified, and keeping it in-tree avoids a dependency bump in the packaged binary.
//! Supported syntax per field: `*`, `?` (day fields), numbers, names (JAN/DEC, SUN/SAT),
//! ranges (`1-5`), lists (`1,3,5`) and steps (`*/5`, `1-30/2`, `5/10`).
//!
//! Schedules are evaluated in the server's **local** time zone, which matches how a NAS
//! user thinks about "every day at 9am".

use chrono::{DateTime, Datelike, Duration, Local, Timelike};

const MONTH_NAMES: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const DOW_NAMES: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

#[derive(Debug, Clone)]
pub struct CronSchedule {
    minutes: [bool; 60],
    hours: [bool; 24],
    days_of_month: [bool; 32], // 1..=31
    months: [bool; 13],        // 1..=12
    days_of_week: [bool; 7],   // 0 = Sunday
    dom_restricted: bool,
    dow_restricted: bool,
}

impl CronSchedule {
    pub fn parse(expression: &str) -> Result<Self, String> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err("cron expression is empty".to_string());
        }

        // Friendly macros — accepted by Vixie cron and most schedulers.
        let macro_fields: Option<[&str; 5]> = match expression.to_ascii_lowercase().as_str() {
            "@yearly" | "@annually" => Some(["0", "0", "1", "1", "*"]),
            "@monthly" => Some(["0", "0", "1", "*", "*"]),
            "@weekly" => Some(["0", "0", "*", "*", "0"]),
            "@daily" | "@midnight" => Some(["0", "0", "*", "*", "*"]),
            "@hourly" => Some(["0", "*", "*", "*", "*"]),
            _ => None,
        };
        let fields: Vec<&str> = match macro_fields {
            Some(fields) => fields.to_vec(),
            None => expression.split_whitespace().collect(),
        };
        if fields.len() != 5 {
            return Err(format!(
                "cron expression must have 5 fields (minute hour day month weekday), got {}",
                fields.len()
            ));
        }

        let (minutes, _) = parse_field(fields[0], 0, 59, None)?;
        let (hours, _) = parse_field(fields[1], 0, 23, None)?;
        let (days_of_month, dom_restricted) = parse_field(fields[2], 1, 31, None)?;
        let (months, _) = parse_field(fields[3], 1, 12, Some(MONTH_NAMES.as_slice()))?;

        let (dow_values, dow_restricted) =
            parse_field(fields[4], 0, 7, Some(DOW_NAMES.as_slice()))?;
        let mut days_of_week = [false; 7];
        for value in dow_values {
            // Both 0 and 7 mean Sunday in cron.
            days_of_week[(value % 7) as usize] = true;
        }

        Ok(Self {
            minutes: to_fixed::<60>(&minutes),
            hours: to_fixed::<24>(&hours),
            days_of_month: to_fixed::<32>(&days_of_month),
            months: to_fixed::<13>(&months),
            days_of_week,
            dom_restricted,
            dow_restricted,
        })
    }

    pub fn matches(&self, at: &DateTime<Local>) -> bool {
        if !self.minutes[at.minute() as usize] || !self.hours[at.hour() as usize] {
            return false;
        }
        if !self.months[at.month() as usize] {
            return false;
        }

        let dom_ok = self.days_of_month[at.day() as usize];
        let dow_ok = self.days_of_week[at.weekday().num_days_from_sunday() as usize];
        // Standard cron quirk: when BOTH day-of-month and day-of-week are restricted,
        // a match on either is enough. When only one is restricted, it must match.
        if self.dom_restricted && self.dow_restricted {
            dom_ok || dow_ok
        } else {
            dom_ok && dow_ok
        }
    }

    /// Next `count` matching minutes strictly after `from`, scanning at most five years.
    pub fn next_runs(&self, from: DateTime<Local>, count: usize) -> Vec<DateTime<Local>> {
        let mut runs = Vec::with_capacity(count);
        if count == 0 {
            return runs;
        }
        let start = from
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(from)
            + Duration::minutes(1);
        let mut cursor = start;
        // 5 years of minutes comfortably covers leap-day schedules; impossible schedules
        // simply return fewer results instead of hanging.
        for _ in 0..(366 * 24 * 60 * 5) {
            if self.matches(&cursor) {
                runs.push(cursor);
                if runs.len() == count {
                    break;
                }
            }
            cursor += Duration::minutes(1);
        }
        runs
    }
}

/// Parse one cron field into a list of matched values.
/// Returns `(values, restricted)` where `restricted` is false for `*` / `?`.
fn parse_field(
    field: &str,
    min: u32,
    max: u32,
    names: Option<&[&str]>,
) -> Result<(Vec<u32>, bool), String> {
    let mut values = Vec::new();
    let mut restricted = false;

    for item in field.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Err(format!("empty item in cron field '{}'", field));
        }
        if item != "*" && item != "?" {
            restricted = true;
        }
        values.extend(expand_item(item, min, max, names)?);
    }

    values.sort_unstable();
    values.dedup();
    // Guard against nonsense like `*/0` producing an empty field.
    if values.is_empty() {
        return Err(format!("cron field '{}' matches no values", field));
    }
    Ok((values, restricted))
}

fn expand_item(item: &str, min: u32, max: u32, names: Option<&[&str]>) -> Result<Vec<u32>, String> {
    let (base, step) = match item.split_once('/') {
        Some((base, step)) => {
            let step: u32 = step
                .parse()
                .map_err(|_| format!("invalid step '{}' in '{}'", step, item))?;
            if step == 0 {
                return Err(format!("step must be greater than 0 in '{}'", item));
            }
            (base, step)
        }
        None => (item, 1),
    };

    let (start, end) = if base == "*" || base == "?" {
        (min, max)
    } else if let Some((a, b)) = base.split_once('-') {
        (
            parse_value(a, min, max, names)?,
            parse_value(b, min, max, names)?,
        )
    } else {
        let value = parse_value(base, min, max, names)?;
        // `5/10` means "from 5 to the end, every 10".
        if step > 1 {
            (value, max)
        } else {
            (value, value)
        }
    };

    if start > end {
        return Err(format!(
            "invalid range {}-{} in '{}': start is greater than end",
            start, end, item
        ));
    }

    let mut values = Vec::new();
    let mut value = start;
    while value <= end {
        values.push(value);
        value += step;
    }
    Ok(values)
}

fn parse_value(token: &str, min: u32, max: u32, names: Option<&[&str]>) -> Result<u32, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("empty value in cron field".to_string());
    }

    let (value, from_name) = if let Ok(number) = token.parse::<u32>() {
        (number, false)
    } else {
        let upper = token.to_ascii_uppercase();
        let named = names
            .and_then(|names| names.iter().position(|name| *name == upper))
            .map(|index| index as u32);
        match named {
            Some(value) => (value, true),
            None => {
                return Err(format!(
                    "invalid value '{}' (expected {}-{})",
                    token, min, max
                ))
            }
        }
    };

    // Month names start at 1 while their slice index starts at 0.
    let value = if from_name && min == 1 {
        value + 1
    } else {
        value
    };

    if value < min || value > max {
        return Err(format!(
            "value {} out of range {}-{} in cron field",
            value, min, max
        ));
    }
    Ok(value)
}

fn to_fixed<const N: usize>(values: &[u32]) -> [bool; N] {
    let mut set = [false; N];
    for value in values {
        if let Some(slot) = set.get_mut(*value as usize) {
            *slot = true;
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 3, 10, hour, minute, 0)
            .single()
            .expect("valid local time")
    }

    #[test]
    fn parses_basic_schedule() {
        let schedule = CronSchedule::parse("*/15 9-17 * * 1-5").unwrap();
        assert!(schedule.matches(&at(9, 0)));
        assert!(schedule.matches(&at(17, 45)));
        assert!(!schedule.matches(&at(8, 0)));
        assert!(!schedule.matches(&at(10, 7)));
    }

    #[test]
    fn names_and_macros() {
        let schedule = CronSchedule::parse("@daily").unwrap();
        assert!(schedule.matches(&at(0, 0)));
        assert!(!schedule.matches(&at(0, 1)));

        let weekdays = CronSchedule::parse("0 9 * * MON-FRI").unwrap();
        assert!(weekdays.matches(&at(9, 0))); // 2026-03-10 is a Tuesday
    }

    #[test]
    fn sunday_accepts_zero_and_seven() {
        let zero = CronSchedule::parse("0 0 * * 0").unwrap();
        let seven = CronSchedule::parse("0 0 * * 7").unwrap();
        let sunday = Local
            .with_ymd_and_hms(2026, 3, 15, 0, 0, 0)
            .single()
            .unwrap();
        assert!(zero.matches(&sunday));
        assert!(seven.matches(&sunday));
    }

    #[test]
    fn dom_and_dow_use_or_when_both_restricted() {
        // 1st of month OR Monday.
        let schedule = CronSchedule::parse("0 0 1 * 1").unwrap();
        let first = Local
            .with_ymd_and_hms(2026, 4, 1, 0, 0, 0)
            .single()
            .unwrap(); // Wednesday
        let monday = Local
            .with_ymd_and_hms(2026, 4, 6, 0, 0, 0)
            .single()
            .unwrap();
        let other = Local
            .with_ymd_and_hms(2026, 4, 7, 0, 0, 0)
            .single()
            .unwrap();
        assert!(schedule.matches(&first));
        assert!(schedule.matches(&monday));
        assert!(!schedule.matches(&other));
    }

    #[test]
    fn next_runs_skips_to_matching_minute() {
        let schedule = CronSchedule::parse("30 8 * * *").unwrap();
        let runs = schedule.next_runs(at(8, 10), 2);
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].hour(), runs[0].minute()), (8, 30));
    }

    #[test]
    fn rejects_invalid_expressions() {
        assert!(CronSchedule::parse("").is_err());
        assert!(CronSchedule::parse("* * *").is_err());
        assert!(CronSchedule::parse("60 * * * *").is_err());
        assert!(CronSchedule::parse("* * * * 8").is_err());
        assert!(CronSchedule::parse("*/0 * * * *").is_err());
        assert!(CronSchedule::parse("5-1 * * * *").is_err());
    }
}
