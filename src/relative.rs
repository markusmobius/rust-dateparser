use std::{collections::HashMap, sync::OnceLock};

use chrono::Datelike;
use regex::Regex;
use rust_dateutil::relativedelta::Delta;

use crate::calendar::{apply_relative, date_time, parse_time};
use crate::{Configuration, Date, Period, PreferredDateSource};

struct Expressions {
    durations: Regex,
    in_word: Regex,
    ago_word: Regex,
    in_or_ago: Regex,
    non_word: Regex,
    skip_word: Regex,
}

fn expressions() -> &'static Expressions {
    static EXPRESSIONS: OnceLock<Expressions> = OnceLock::new();
    EXPRESSIONS.get_or_init(|| Expressions {
        durations: Regex::new(r"(?i)([+-]?[\t\n\r\x0c ]*[0-9]+[.,]?[0-9]*)[\t\n\r\x0c ]*(decade|year|month|week|day|hour|minute|second)(?-u:\b)").unwrap(),
        in_word: Regex::new(r"(?i)(?-u:\b)in(?-u:\b)").unwrap(),
        ago_word: Regex::new(r"(?i)(?-u:\b)ago(?-u:\b)").unwrap(),
        in_or_ago: Regex::new(r"(?i)(?-u:\b)(?:ago|in)(?-u:\b)").unwrap(),
        non_word: Regex::new(r"[^0-9A-Za-z_]").unwrap(),
        skip_word: Regex::new(r"(?i)^(?:decade|year|month|week|day|hour|minute|second|ago|in|[0-9]+|:|[ap]m)").unwrap(),
    })
}

fn durations(input: &str, forward: bool) -> Option<HashMap<String, f64>> {
    let mut values = HashMap::new();
    for captures in expressions().durations.captures_iter(input) {
        let unit = captures[2].to_string();
        let number = captures[1]
            .split_whitespace()
            .collect::<String>()
            .replace(',', ".");
        let mut value = number.parse::<f64>().ok()?;
        if !value.is_finite() {
            return None;
        }
        if !forward && !number.starts_with(['+', '-']) {
            value = -value;
        }
        values.insert(unit, value);
    }
    if let Some(decades) = values.remove("decade") {
        *values.entry("year".into()).or_default() += decades * 10.0;
    }
    if let Some(weeks) = values.remove("week") {
        *values.entry("day".into()).or_default() += weeks * 7.0;
    }
    Some(values)
}

pub(crate) fn parse(configuration: &Configuration, input: &str) -> Option<Date> {
    let input = crate::text::strip_braces(input);
    let (input, timezone) = crate::timezone::pop_offset(&input);
    let expressions = expressions();
    let sanitized = crate::text::sanitize_spaces(&input);
    if expressions
        .non_word
        .split(&sanitized)
        .filter(|word| !word.is_empty())
        .any(|word| !expressions.skip_word.is_match(word))
    {
        return None;
    }
    let forward = expressions.in_word.is_match(&input)
        || (configuration.preferred_date_source == PreferredDateSource::Future
            && !expressions.ago_word.is_match(&input));
    let values = durations(&input, forward)?;
    if values.is_empty() {
        return None;
    }
    let mut period = [
        ("second", Period::Second),
        ("minute", Period::Minute),
        ("hour", Period::Hour),
        ("day", Period::Day),
        ("month", Period::Month),
        ("year", Period::Year),
    ]
    .into_iter()
    .find(|(unit, _)| values.contains_key(*unit))
    .map_or(Period::Day, |(_, period)| period);
    let mut now = configuration.current_time.clone()?;
    if let Some(timezone) = timezone.or_else(|| configuration.default_timezone.clone()) {
        now = now.with_timezone(&timezone);
    }
    let amount = |unit: &str| values.get(unit).copied().unwrap_or_default();
    for (unit, maximum) in [
        ("year", 10000.0),
        ("month", 120000.0),
        ("day", 3660000.0),
        ("hour", 87840000.0),
        ("minute", 5270400000.0),
        ("second", 316224000000.0),
    ] {
        if !amount(unit).is_finite() || amount(unit).abs() > maximum {
            return None;
        }
    }
    if configuration.return_time_as_period {
        let fractional_hours = amount("day").fract() * 24.0;
        if fractional_hours.trunc() != 0.0 {
            period = period.min(Period::Hour);
        }
        let fractional_minutes = (amount("hour") + fractional_hours).fract() * 60.0;
        if fractional_minutes.trunc() != 0.0 {
            period = period.min(Period::Minute);
        }
        let fractional_seconds = (amount("minute") + fractional_minutes).fract() * 60.0;
        if fractional_seconds.trunc() != 0.0 {
            period = period.min(Period::Second);
        }
    }
    let mut value = apply_relative(
        &now,
        Delta {
            years: amount("year"),
            months: amount("month"),
            days: amount("day"),
            hours: amount("hour"),
            minutes: amount("minute"),
            seconds: amount("second"),
            ..Delta::default()
        },
    )?;
    let without_durations = expressions.durations.replace_all(&input, "");
    let clock_text = expressions.in_or_ago.replace_all(&without_durations, "");
    if let Some((clock, clock_period)) = parse_time(&clock_text) {
        value = date_time(
            value.year(),
            value.month() as i32,
            value.day() as i32,
            clock,
            &value.timezone(),
        )?;
        period = period.min(clock_period);
    }
    if !configuration.return_time_as_period && period.is_time() {
        period = Period::Day;
    }
    Some(Date {
        time: value,
        period,
        locale: String::new(),
    })
    .filter(|date| !date.is_zero())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Timezone;
    use chrono::TimeZone;

    #[test]
    fn python_relative_arithmetic() {
        for preserve_end_of_month in [false, true] {
            let configuration = Configuration {
                current_time: Some(
                    Timezone::Utc
                        .with_ymd_and_hms(2023, 1, 31, 12, 0, 0)
                        .unwrap(),
                ),
                preserve_end_of_month,
                return_time_as_period: true,
                ..Configuration::default()
            };
            for (input, expected, period) in [
                ("in 1 month", "2023-02-28T12:00:00+00:00", Period::Month),
                ("1 month ago", "2022-12-31T12:00:00+00:00", Period::Month),
                ("in 1.5 day", "2023-02-02T00:00:00+00:00", Period::Hour),
                (
                    "0.5 second ago",
                    "2023-01-31T11:59:59.500+00:00",
                    Period::Second,
                ),
            ] {
                let value = parse(&configuration, input).unwrap();
                assert_eq!(value.time.to_rfc3339(), expected, "{input}");
                assert_eq!(value.period, period, "{input}");
            }
            for input in ["in 1.5 year", "in 1.5 month"] {
                assert!(parse(&configuration, input).is_none(), "{input}");
            }
        }
    }

    #[test]
    fn relative_dates_preserve_fraction_order_and_month_end() {
        let mut configuration = Configuration {
            current_time: Some(
                Timezone::Utc
                    .with_ymd_and_hms(2024, 3, 31, 12, 30, 45)
                    .unwrap(),
            ),
            return_time_as_period: true,
            ..Configuration::default()
        };
        for (input, expected, period) in [
            ("1 month ago", "2024-02-29 12:30:45", Period::Month),
            ("1.5 day ago", "2024-03-30 00:30:45", Period::Hour),
            ("0.5 second ago", "2024-03-31 12:30:44.500", Period::Second),
            ("1 day 2 day ago", "2024-03-29 12:30:45", Period::Day),
            ("1 DAY ago", "2024-03-31 12:30:45", Period::Day),
            ("2 day ago 4 PM", "2024-03-29 16:00:00", Period::Hour),
        ] {
            let date = parse(&configuration, input).unwrap();
            assert_eq!(
                date.time.format("%F %T%.f").to_string(),
                expected,
                "{input}"
            );
            assert_eq!(date.period, period, "{input}");
        }
        configuration.preserve_end_of_month = true;
        assert_eq!(
            parse(&configuration, "1.1.1 day ago").unwrap().period,
            Period::Minute
        );
        assert_eq!(
            parse(&configuration, "1 month ago")
                .unwrap()
                .time
                .format("%F")
                .to_string(),
            "2024-02-29"
        );
    }
}
