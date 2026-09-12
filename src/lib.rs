#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod absolute;
mod calendar;
mod config;
mod formatted;
mod language;
mod locale;
mod nospace;
mod parser;
mod relative;
mod text;
mod timezone;
mod timezone_data;
mod tokenizer;

#[cfg(test)]
mod upstream_tests;

use chrono::{DateTime, Local, TimeZone};
use std::fmt;

pub use config::{
    Configuration, DateOrder, DateOrderResolver, PreferredDateSource, PreferredDayOfMonth,
    PreferredMonthOfYear,
};
pub use parser::{parse, parse_with_formats, DetectLanguagesFunction, Parser, ParserType};
pub use timezone::{Timezone, TimezoneOffset};

pub fn is_known_locale(code: &str) -> bool {
    locale::data().get(code).is_some()
}

pub fn pop_tz_offset(input: &str) -> (String, String, i32) {
    let (cleaned, timezone) = timezone::pop_offset(input);
    match timezone {
        Some(Timezone::Fixed { name, offset }) => {
            (cleaned, name.to_string(), offset.local_minus_utc())
        }
        _ => (cleaned, String::new(), 0),
    }
}

pub fn parse_absolute(configuration: &Configuration, input: &str) -> Result<Date, Error> {
    let configuration = configuration.initialized()?;
    absolute::parse(&configuration, input, None).ok_or_else(|| Error::UnknownFormat(input.into()))
}

pub fn parse_formatted(
    configuration: &Configuration,
    input: &str,
    formats: &[&str],
) -> Result<Date, Error> {
    let configuration = configuration.initialized()?;
    formatted::parse(&configuration, input, formats)
        .filter(|date| !date.is_zero())
        .ok_or_else(|| Error::UnknownFormat(input.into()))
}

pub fn parse_no_spaces(configuration: &Configuration, input: &str) -> Result<Date, Error> {
    let configuration = configuration.initialized()?;
    let year = chrono::Datelike::year(&Local::now());
    nospace::parse(&configuration, input, year).ok_or_else(|| Error::UnknownFormat(input.into()))
}

pub fn parse_relative(configuration: &Configuration, input: &str) -> Result<Date, Error> {
    let configuration = configuration.initialized()?;
    relative::parse(&configuration, input).ok_or_else(|| Error::UnknownFormat(input.into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidConfiguration(String),
    InvalidTimezone(String),
    UnknownLocales(Vec<String>),
    UnknownLanguages(Vec<String>),
    ConflictingLocales,
    UnknownFormat(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => write!(formatter, "config error: {message}"),
            Self::InvalidTimezone(name) => write!(formatter, "unknown time zone {name}"),
            Self::UnknownLocales(locales) => {
                write!(formatter, "unknown locale(s): {}", locales.join(", "))
            }
            Self::UnknownLanguages(languages) => {
                write!(formatter, "unknown language(s): {}", languages.join(", "))
            }
            Self::ConflictingLocales => {
                formatter.write_str("locales should not have same language and different region")
            }
            Self::UnknownFormat(input) => {
                write!(formatter, "failed to parse \"{input}\": unknown format")
            }
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum Period {
    #[default]
    None,
    Second,
    Minute,
    Hour,
    Day,
    Month,
    Year,
}

impl Period {
    pub fn is_time(self) -> bool {
        matches!(self, Self::Second | Self::Minute | Self::Hour)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Date {
    pub locale: String,
    pub period: Period,
    pub time: DateTime<Timezone>,
}

impl Date {
    pub fn is_zero(&self) -> bool {
        self.period == Period::None
            || self.time == chrono::Utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0).unwrap()
    }
}

pub fn parse_timestamp(configuration: &Configuration, input: &str, negative: bool) -> Option<Date> {
    let digits = if negative {
        input.strip_prefix('-')?
    } else {
        input
    };
    let bytes = digits.as_bytes();
    let width = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !matches!(width, 10 | 13 | 16) {
        return None;
    }
    if let Some(delimiter) = bytes.get(width) {
        if !matches!(delimiter, b'.' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') {
            return None;
        }
    }

    let seconds = digits[..10].parse::<i64>().ok()?;
    let seconds = if negative { -seconds } else { seconds };
    let millis = if width >= 13 {
        digits[10..13].parse::<u32>().ok()?
    } else {
        0
    };
    let micros = if width == 16 {
        digits[13..16].parse::<u32>().ok()?
    } else {
        0
    };
    let time = Local
        .timestamp_opt(seconds, millis * 1_000_000 + micros * 1_000)
        .single()?
        .with_timezone(&Timezone::Local);
    Some(Date {
        locale: String::new(),
        period: if configuration.return_time_as_period {
            Period::Second
        } else {
            Period::Day
        },
        time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_helpers_preserve_known_locales_and_timezone_names() {
        for locale in ["en", "en-US", "fr-PF", "zh-Hant"] {
            assert!(is_known_locale(locale));
        }
        assert!(!is_known_locale("EN"));
        assert!(!is_known_locale("not-a-locale"));
        assert_eq!(
            pop_tz_offset("2024-02-29 UTC+05:45"),
            ("2024-02-29 ".into(), "UTC+05:45".into(), 20_700)
        );
        assert_eq!(
            pop_tz_offset("2024-02-29 unknown"),
            ("2024-02-29 unknown".into(), String::new(), 0)
        );
    }

    #[test]
    fn timestamp_matches_go_digit_widths() {
        let configuration = Configuration::default();
        for (input, negative, seconds, nanos) in [
            ("1570308760", false, 1_570_308_760, 0),
            ("1570308760263", false, 1_570_308_760, 263_000_000),
            ("1570308760263111", false, 1_570_308_760, 263_111_000),
            ("-1570308760", true, -1_570_308_760, 0),
            ("-1570308760263", true, -1_570_308_760, 263_000_000),
            ("-1570308760263111", true, -1_570_308_760, 263_111_000),
            ("0000000000", false, 0, 0),
            ("9999999999999999", false, 9_999_999_999, 999_999_000),
        ] {
            let date = parse_timestamp(&configuration, input, negative).unwrap();
            assert_eq!(date.time.timestamp(), seconds, "{input}");
            assert_eq!(date.time.timestamp_subsec_nanos(), nanos, "{input}");
            assert_eq!(date.period, Period::Day);
            assert!(date.locale.is_empty());
        }
    }

    #[test]
    fn timestamp_rejects_go_invalid_widths_and_signs() {
        for input in [
            "15703087602631",
            "157030876026xx",
            "1570308760263x",
            "157030876026311",
            "15703087602631x",
            "15703087602631xx",
            "15703087602631111",
            "1570308760263111x",
            "1570308760263111xx",
            "1570308760263111222",
            "+1570308760",
            " 1570308760",
            "1570308760\u{b}",
            "1570308760\u{a0}",
            "\u{0661}570308760",
        ] {
            assert!(
                parse_timestamp(&Configuration::default(), input, false).is_none(),
                "{input:?}"
            );
        }
        assert!(parse_timestamp(&Configuration::default(), "-1570308760", false).is_none());
        assert!(parse_timestamp(&Configuration::default(), "1570308760", true).is_none());
    }

    #[test]
    fn timestamp_accepts_go_prefix_delimiters_and_precision() {
        let configuration = Configuration {
            return_time_as_period: true,
            ..Configuration::default()
        };
        for suffix in [
            "",
            ".123",
            " text",
            "\ttext",
            "\ntext",
            "\rtext",
            "\u{c}text",
        ] {
            let date =
                parse_timestamp(&configuration, &format!("1570308760{suffix}"), false).unwrap();
            assert_eq!(date.time.timestamp(), 1_570_308_760);
            assert_eq!(date.time.timestamp_subsec_nanos(), 0);
            assert_eq!(date.period, Period::Second);
            assert_eq!(
                date.time,
                Local
                    .timestamp_opt(1_570_308_760, 0)
                    .unwrap()
                    .fixed_offset()
            );
        }
    }
}
