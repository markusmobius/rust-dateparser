use chrono::Datelike;

use crate::calendar::last_day;
use crate::formatted::parse_layout;
use crate::tokenizer::{tokenize, Kind};
use crate::{Configuration, Date, DateOrder, Timezone};

const DATE_FORMATS: &[&str] = &[
    "Ymd", "Ydm", "mYd", "mdY", "dYm", "dmY", "ymd", "ydm", "myd", "mdy", "dym", "dmy",
];
const TIME_FORMATS: &[&str] = &["HMS", "HM", "H"];

fn formats(order: DateOrder) -> Vec<String> {
    let mut formats: Vec<_> = DATE_FORMATS
        .iter()
        .map(|format| format.to_string())
        .collect();
    for date in DATE_FORMATS {
        for time in TIME_FORMATS {
            formats.push(format!("{date}{time}"));
        }
    }
    formats.extend(TIME_FORMATS.iter().map(|format| format.to_string()));
    let prefix = order.as_str().to_ascii_lowercase();
    formats.sort_by_key(|format| !format.to_ascii_lowercase().starts_with(&prefix));
    if order == DateOrder::Mdy {
        formats.splice(..0, ["YmdHM".into(), "YmdHMS".into()]);
    }
    formats
}

fn part_size(part: u8) -> usize {
    if part == b'Y' {
        4
    } else {
        2
    }
}

fn valid_value(part: u8, value: u32) -> bool {
    match part {
        b'Y' => value >= 1000,
        b'y' => value <= 99,
        b'm' => (1..=12).contains(&value),
        b'd' => (1..=31).contains(&value),
        b'H' => value < 24,
        b'M' | b'S' => value < 60,
        _ => false,
    }
}

fn candidates<'input>(
    input: &'input str,
    format: &[u8],
    wall_year: i32,
    parts: &mut Vec<&'input str>,
    results: &mut Vec<String>,
) {
    let index = parts.len();
    if index == format.len() {
        if !input.is_empty() {
            return;
        }
        let lookup = |part: u8| {
            format
                .iter()
                .position(|candidate| *candidate == part)
                .and_then(|index| parts[index].parse::<i32>().ok())
        };
        let year = if let Some(year) = lookup(b'Y') {
            year
        } else if let Some(short) = lookup(b'y') {
            let millennium = wall_year / 1000 * 1000;
            short
                + if short < wall_year - millennium {
                    millennium
                } else {
                    millennium - 100
                }
        } else {
            return;
        };
        let Some(month) = lookup(b'm') else {
            return;
        };
        let Some(day) = lookup(b'd') else {
            return;
        };
        if day >= 1 && day as u32 <= last_day(year, month as u32) {
            results.push(parts.join("-"));
        }
        return;
    }
    let remaining = &format[index + 1..];
    for width in (1..=part_size(format[index])).rev() {
        if input.len() < width + remaining.len()
            || input.len() > width + remaining.iter().map(|part| part_size(*part)).sum::<usize>()
        {
            continue;
        }
        let Some(text) = input.get(..width) else {
            continue;
        };
        let Some(value) = text.parse::<u32>().ok() else {
            continue;
        };
        if !valid_value(format[index], value) {
            continue;
        }
        parts.push(text);
        candidates(&input[width..], format, wall_year, parts, results);
        parts.pop();
    }
}

pub(crate) fn parse(configuration: &Configuration, input: &str, wall_year: i32) -> Option<Date> {
    if let Some((start, _)) = input
        .char_indices()
        .find(|(_, character)| !character.is_ascii_digit())
    {
        let suffix = &input[start..];
        let end = suffix
            .char_indices()
            .find(|(_, character)| character.is_ascii_digit())
            .map_or(suffix.len(), |(index, _)| index);
        if &suffix[..end] != ":" {
            return None;
        }
    }
    let input = crate::text::sanitize_spaces(input).replace(':', "");
    if input.is_empty() {
        return None;
    }
    let raw_configuration = Configuration {
        current_time: configuration.current_time.clone(),
        ..Configuration::default()
    };
    if configuration.date_order.is_none()
        && input.len() == 8
        && input.bytes().all(|byte| byte.is_ascii_digit())
    {
        for layout in [
            "01022006", "02012006", "20060102", "20060201", "01200602", "02200601",
        ] {
            if let Some(date) = parse_layout(&raw_configuration, &input, layout, &Timezone::Utc)
                .filter(|date| !date.is_zero() && date.time.year() >= 1000)
            {
                return Some(date);
            }
        }
    }
    let mut ambiguous = None;
    for token in tokenize(&input)
        .iter()
        .filter(|token| token.kind == Kind::Digit)
    {
        for format in formats(configuration.date_order.unwrap_or(DateOrder::Mdy)) {
            let mut texts = Vec::new();
            candidates(
                &token.text,
                format.as_bytes(),
                wall_year,
                &mut Vec::new(),
                &mut texts,
            );
            let layout = format
                .bytes()
                .map(|part| match part {
                    b'Y' => "2006",
                    b'y' => "06",
                    b'm' => "1",
                    b'd' => "2",
                    b'H' => "15",
                    b'M' => "4",
                    _ => "5",
                })
                .collect::<Vec<_>>()
                .join("-");
            for text in texts {
                let Some(date) = parse_layout(&raw_configuration, &text, &layout, &Timezone::Utc)
                    .filter(|date| !date.is_zero())
                else {
                    continue;
                };
                if date.time.year() < 1000 {
                    ambiguous = Some(date);
                    continue;
                }
                let missing: Vec<_> = [('d', "day"), ('m', "month"), ('y', "year"), ('Y', "year")]
                    .into_iter()
                    .filter(|(part, _)| !format.contains(*part))
                    .map(|(_, name)| name)
                    .collect();
                if configuration.strict_parsing && !missing.is_empty() {
                    continue;
                }
                if configuration
                    .required_parts
                    .iter()
                    .any(|part| missing.contains(&part.as_str()))
                {
                    continue;
                }
                return Some(date);
            }
        }
    }
    ambiguous
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn compact_dates_preserve_go_order_and_strict_shortcut() {
        let mut configuration = Configuration {
            current_time: Some(Timezone::Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap()),
            ..Configuration::default()
        };
        for (input, expected) in [
            ("20211011", "2021-10-11"),
            ("01022006", "2006-01-02"),
            ("000229", "2000-02-29"),
        ] {
            assert_eq!(
                parse(&configuration, input, 2026)
                    .unwrap()
                    .time
                    .format("%F")
                    .to_string(),
                expected
            );
        }
        configuration.strict_parsing = true;
        assert!(parse(&configuration, "20211011", 2026).is_some());
        configuration.date_order = Some(DateOrder::Ymd);
        assert!(parse(&configuration, "20211011", 2026).is_none());
    }
}
