use std::sync::OnceLock;

use chrono::{DateTime, Datelike, Utc};
use regex::Regex;

use crate::{calendar, Configuration, Date, Period, SearchResult, Timezone};

fn shift(base: &DateTime<Timezone>, months: i64, days: i64) -> Option<DateTime<Timezone>> {
    let month = i64::from(base.month0()).checked_add(months)?;
    let year = i32::try_from(i64::from(base.year()).checked_add(month.div_euclid(12))?).ok()?;
    let day = i32::try_from(i64::from(base.day()).checked_add(days)?).ok()?;
    calendar::date_time(
        year,
        month.rem_euclid(12) as i32 + 1,
        day,
        base.time(),
        &base.timezone(),
    )
}

fn span(configuration: &Configuration, code: &str, input: &str) -> Option<Vec<SearchResult>> {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str, bool)>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        let mut patterns = Vec::new();
        for (direction, future) in [("past|last|previous", false), ("next|coming|following", true)] {
            for (suffix, unit) in [("month", "month"), ("week", "week"), (r"([0-9]+)[\t\n\r\x0c ]+days?", "days"),
                (r"([0-9]+)[\t\n\r\x0c ]+weeks?", "weeks"), (r"([0-9]+)[\t\n\r\x0c ]+months?", "months")] {
                let pattern = format!(r"(?i)(?-u:\b)(?:for[\t\n\r\x0c ]+the[\t\n\r\x0c ]+|during[\t\n\r\x0c ]+the[\t\n\r\x0c ]+|in[\t\n\r\x0c ]+the[\t\n\r\x0c ]+)?(?:{direction})[\t\n\r\x0c ]+{suffix}(?-u:\b)");
                patterns.push((Regex::new(&pattern).unwrap(), unit, future));
            }
        }
        patterns
    });
    for (pattern, unit, future) in patterns {
        let Some(captures) = pattern.captures(input) else {
            continue;
        };
        let count = match captures.get(1) {
            Some(number) => number.as_str().parse::<i64>().ok()?,
            None => 1,
        };
        let base = configuration
            .current_time
            .clone()
            .unwrap_or_else(|| Utc::now().with_timezone(&Timezone::Utc));
        let mut start = base.clone();
        let mut end = base.clone();
        let direction = if *future { 1 } else { -1 };
        let boundary = match *unit {
            "month" => shift(
                &base,
                0,
                direction * i64::from(configuration.default_days_in_month),
            )?,
            "week" => {
                let back = if configuration.default_start_of_week == "sunday" {
                    base.weekday().num_days_from_sunday()
                } else {
                    base.weekday().num_days_from_monday()
                };
                let week = shift(&base, 0, -i64::from(back))?;
                start = shift(&week, 0, direction * 7)?;
                end = shift(&start, 0, 6)?;
                base.clone()
            }
            "days" => shift(&base, 0, direction * count)?,
            "weeks" => shift(&base, 0, direction * count.checked_mul(7)?)?,
            "months" => {
                let first = shift(&base, direction * count, 1 - i64::from(base.day()))?;
                let day = base
                    .day()
                    .min(calendar::last_day(first.year(), first.month()));
                shift(&first, 0, i64::from(day) - 1)?
            }
            _ => unreachable!(),
        };
        if *unit != "week" {
            if *future {
                end = boundary;
            } else {
                start = boundary;
            }
        }
        return Some(
            [(start, "start"), (end, "end")]
                .into_iter()
                .map(|(time, label)| SearchResult {
                    date: Date {
                        time,
                        period: Period::Day,
                        locale: code.into(),
                    },
                    text: format!("{} ({label})", captures.get(0).unwrap().as_str()),
                })
                .collect(),
        );
    }
    None
}

pub(crate) fn search(configuration: &Configuration, code: &str, input: &str) -> Vec<SearchResult> {
    span(configuration, code, input).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn spans_preserve_week_starts_month_clamping_and_timezones() {
        let zone: Timezone = "America/New_York".parse().unwrap();
        let configuration = Configuration {
            current_time: Some(zone.with_ymd_and_hms(2025, 1, 31, 12, 30, 45).unwrap()),
            ..Configuration::default()
        }
        .initialized()
        .unwrap();
        let found = search(&configuration, "en", "next 1 months");
        assert_eq!(
            found[1].date.time.format("%F %T").to_string(),
            "2025-02-28 12:30:45"
        );
        assert_eq!(found[1].date.time.timezone(), zone);
        let configuration = Configuration {
            current_time: Some(
                Timezone::Utc
                    .with_ymd_and_hms(2025, 2, 18, 12, 0, 0)
                    .unwrap(),
            ),
            default_start_of_week: "sunday".into(),
            ..configuration
        };
        let found = search(&configuration, "en", "last week");
        assert_eq!(found[0].date.time.format("%F").to_string(), "2025-02-09");
        assert_eq!(found[1].date.time.format("%F").to_string(), "2025-02-15");
        assert!(search(&configuration, "en", "past 99999999999999999999 weeks").is_empty());
    }
}
