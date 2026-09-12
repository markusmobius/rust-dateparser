use chrono::{
    format::{parse_and_remainder, Parsed, StrftimeItems},
    Datelike, NaiveDate, NaiveTime, Offset,
};

use crate::calendar::{apply_day, apply_month, correct_leap_year, date_time, leap_year};
use crate::timezone::detected_zone;
use crate::{Configuration, Date, Period, PreferredDateSource, Timezone};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Field {
    Year,
    ShortYear,
    Month,
    LongMonth,
    ShortMonth,
    Day,
    Ordinal,
    Hour,
    Hour12,
    Minute,
    Second,
    Meridian,
    Weekday,
    LongWeekday,
    Zone,
    Offset,
    Fraction(bool),
}

#[derive(Clone, Copy)]
struct Piece<'layout> {
    text: &'layout str,
    field: Option<Field>,
    example: &'static str,
}

fn pieces(mut layout: &str) -> Vec<Piece<'_>> {
    const FIELDS: &[(&str, Field, &str)] = &[
        ("Z07:00:00", Field::Offset, "Z"),
        ("-07:00:00", Field::Offset, "+00:00:00"),
        ("Z070000", Field::Offset, "Z"),
        ("-070000", Field::Offset, "+000000"),
        ("Z07:00", Field::Offset, "Z"),
        ("-07:00", Field::Offset, "+00:00"),
        ("Z0700", Field::Offset, "Z"),
        ("-0700", Field::Offset, "+0000"),
        ("January", Field::LongMonth, "March"),
        ("Monday", Field::LongWeekday, "Sunday"),
        ("2006", Field::Year, "0012"),
        ("__2", Field::Ordinal, " 64"),
        ("002", Field::Ordinal, "064"),
        ("Jan", Field::ShortMonth, "Mar"),
        ("Mon", Field::Weekday, "Sun"),
        ("MST", Field::Zone, "UTC"),
        ("Z07", Field::Offset, "Z"),
        ("-07", Field::Offset, "+00"),
        ("01", Field::Month, "03"),
        ("02", Field::Day, "04"),
        ("_2", Field::Day, " 4"),
        ("03", Field::Hour12, "05"),
        ("04", Field::Minute, "06"),
        ("05", Field::Second, "07"),
        ("06", Field::ShortYear, "12"),
        ("15", Field::Hour, "05"),
        ("PM", Field::Meridian, "AM"),
        ("pm", Field::Meridian, "am"),
        ("1", Field::Month, "3"),
        ("2", Field::Day, "4"),
        ("3", Field::Hour12, "5"),
        ("4", Field::Minute, "6"),
        ("5", Field::Second, "7"),
    ];
    let mut result = Vec::new();
    while !layout.is_empty() {
        if layout.starts_with(['.', ','])
            && layout
                .as_bytes()
                .get(1)
                .is_some_and(|byte| matches!(byte, b'0' | b'9'))
        {
            let digit = layout.as_bytes()[1];
            let width = layout.as_bytes()[1..]
                .iter()
                .take_while(|byte| **byte == digit)
                .count();
            if layout
                .as_bytes()
                .get(width + 1)
                .is_none_or(|byte| !byte.is_ascii_digit())
            {
                result.push(Piece {
                    text: &layout[..width + 1],
                    field: Some(Field::Fraction(digit == b'0')),
                    example: "",
                });
                layout = &layout[width + 1..];
                continue;
            }
        }
        let field = FIELDS.iter().find(|(token, field, _)| {
            layout.starts_with(token)
                && (!matches!(field, Field::ShortMonth | Field::Weekday)
                    || layout
                        .as_bytes()
                        .get(token.len())
                        .is_none_or(|byte| !byte.is_ascii_lowercase()))
        });
        if let Some(&(text, field, example)) = field {
            result.push(Piece {
                text: &layout[..text.len()],
                field: Some(field),
                example,
            });
            layout = &layout[text.len()..];
        } else {
            let width = layout.chars().next().unwrap().len_utf8();
            result.push(Piece {
                text: &layout[..width],
                field: None,
                example: "",
            });
            layout = &layout[width..];
        }
    }
    result
}

fn number(input: &mut &str, width: usize, fixed: bool, spaces: usize) -> Option<u32> {
    for _ in 0..spaces {
        if let Some(rest) = input.strip_prefix(' ') {
            *input = rest;
        }
    }
    let count = input
        .bytes()
        .take(width)
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if count == 0 || (fixed && count != width) {
        return None;
    }
    let value = input[..count].parse().ok()?;
    *input = &input[count..];
    Some(value)
}

fn fraction(input: &mut &str, fixed: Option<usize>) -> Option<u32> {
    if !input.starts_with(['.', ',']) {
        return if fixed.is_none() { Some(0) } else { None };
    }
    let digits = &input[1..];
    let available = digits
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    let width = fixed.unwrap_or(available);
    if width == 0 || available < width {
        return if fixed.is_none() { Some(0) } else { None };
    }
    let parsed_width = width.min(9);
    let nanos = digits[..parsed_width].parse::<u32>().ok()? * 10u32.pow(9 - parsed_width as u32);
    *input = &digits[width..];
    Some(nanos)
}

fn numeric_offset(input: &mut &str, layout: &str) -> Option<(i32, bool)> {
    if layout.starts_with('Z') && input.starts_with('Z') {
        *input = &input[1..];
        return Some((0, true));
    }
    let negative = match input.as_bytes().first()? {
        b'-' => true,
        b'+' => false,
        _ => return None,
    };
    *input = &input[1..];
    let hour = number(input, 2, true, 0)?;
    let mut minute = 0;
    let mut second = 0;
    let colons = layout.contains(':');
    let fields = layout.bytes().filter(|byte| byte.is_ascii_digit()).count() / 2;
    if fields >= 2 {
        if colons {
            *input = input.strip_prefix(':')?;
        }
        minute = number(input, 2, true, 0)?;
    }
    if fields == 3 {
        if colons {
            *input = input.strip_prefix(':')?;
        }
        second = number(input, 2, true, 0)?;
    }
    if hour > 24 || minute > 60 || second > 60 {
        return None;
    }
    let value = (hour * 3600 + minute * 60 + second) as i32;
    Some((if negative { -value } else { value }, false))
}

fn zone_name<'input>(input: &mut &'input str) -> Option<&'input str> {
    let width = if input.starts_with("UTC") {
        3
    } else if let Some(suffix) = input.strip_prefix("GMT") {
        if suffix.starts_with(['+', '-']) {
            let digits = suffix[1..]
                .bytes()
                .take(2)
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if digits == 0 {
                return None;
            }
            4 + digits
        } else {
            3
        }
    } else if input.starts_with("ChST") || input.starts_with("MeST") {
        4
    } else {
        let length = input
            .bytes()
            .take_while(|byte| byte.is_ascii_uppercase())
            .count();
        match length {
            3 => 3,
            4 if input.as_bytes()[3] == b'T' || input.starts_with("WITA") => 4,
            5 if input.as_bytes()[4] == b'T' => 5,
            _ => return None,
        }
    };
    let name = &input[..width];
    *input = &input[width..];
    Some(name)
}

pub(crate) fn parse_layout(
    configuration: &Configuration,
    original: &str,
    layout: &str,
    base_zone: &Timezone,
) -> Option<Date> {
    let tokens = pieces(layout);
    let mut input = original;
    let mut year = 0;
    let mut month = 1;
    let mut day = 1;
    let mut month_set = false;
    let mut day_set = false;
    let mut ordinal = None;
    let mut hour = 0;
    let mut minute = 0;
    let mut second = 0;
    let mut nanos = 0;
    let mut meridian = None;
    let mut parsed_offset = None;
    let mut explicit_utc = false;
    let mut parsed_zone_name = None;
    let mut checker = String::new();
    for (index, token) in tokens.iter().enumerate() {
        checker.push_str(if token.field.is_some() {
            token.example
        } else {
            token.text
        });
        let Some(field) = token.field else {
            if token.text == " " {
                input = input.trim_start_matches(' ');
            } else {
                input = input.strip_prefix(token.text)?;
            }
            continue;
        };
        match field {
            Field::Year => year = number(&mut input, 4, true, 0)? as i32,
            Field::ShortYear => {
                let short = number(&mut input, 2, true, 0)? as i32;
                year = if short >= 69 {
                    1900 + short
                } else {
                    2000 + short
                };
            }
            Field::Month => {
                month = number(&mut input, 2, token.text.starts_with('0'), 0)?;
                month_set = true;
            }
            Field::Day => {
                day = number(
                    &mut input,
                    2,
                    token.text.starts_with('0'),
                    usize::from(token.text.starts_with('_')),
                )?;
                day_set = true;
            }
            Field::Ordinal => {
                ordinal = Some(number(
                    &mut input,
                    3,
                    token.text.starts_with('0'),
                    if token.text.starts_with('_') { 2 } else { 0 },
                )?)
            }
            Field::Hour | Field::Hour12 => {
                hour = number(&mut input, 2, token.text.starts_with('0'), 0)?;
                if hour > if field == Field::Hour12 { 12 } else { 23 } {
                    return None;
                }
            }
            Field::Minute => minute = number(&mut input, 2, token.text.starts_with('0'), 0)?,
            Field::Second => {
                second = number(&mut input, 2, token.text.starts_with('0'), 0)?;
                if !tokens
                    .get(index + 1)
                    .is_some_and(|token| matches!(token.field, Some(Field::Fraction(_))))
                {
                    nanos = fraction(&mut input, None)?;
                }
            }
            Field::Fraction(required) => {
                nanos = fraction(&mut input, required.then_some(token.text.len() - 1))?
            }
            Field::Meridian => {
                let upper = token.text == "PM";
                if let Some(rest) = input.strip_prefix(if upper { "PM" } else { "pm" }) {
                    input = rest;
                    meridian = Some(true);
                } else {
                    input = input.strip_prefix(if upper { "AM" } else { "am" })?;
                    meridian = Some(false);
                }
            }
            Field::LongMonth | Field::ShortMonth | Field::Weekday | Field::LongWeekday => {
                let mut parsed = Parsed::new();
                let format = match field {
                    Field::LongMonth => "%B",
                    Field::ShortMonth => "%b",
                    Field::LongWeekday => "%A",
                    _ => "%a",
                };
                let short = matches!(field, Field::ShortMonth | Field::Weekday);
                let text = if short { input.get(..3)? } else { input };
                let rest =
                    parse_and_remainder(&mut parsed, text, StrftimeItems::new(format)).ok()?;
                let consumed = text.len() - rest.len();
                if matches!(field, Field::LongMonth | Field::ShortMonth) {
                    month = parsed.month()?;
                    month_set = true;
                    if !short
                        && consumed != [7, 8, 5, 5, 3, 4, 4, 6, 9, 7, 8, 8][month as usize - 1]
                    {
                        return None;
                    }
                } else if !short
                    && consumed
                        != [6, 7, 9, 8, 6, 8, 6][parsed.weekday()?.num_days_from_monday() as usize]
                {
                    return None;
                }
                input = &input[consumed..];
            }
            Field::Zone => {
                let name = zone_name(&mut input)?;
                explicit_utc |= name == "UTC";
                parsed_zone_name = Some(name);
            }
            Field::Offset => {
                let (offset, utc) = numeric_offset(&mut input, token.text)?;
                parsed_offset = Some(offset);
                explicit_utc |= utc;
            }
        }
    }
    if !input.is_empty() || minute >= 60 || second >= 60 {
        return None;
    }
    if let Some(pm) = meridian {
        if pm && hour < 12 {
            hour += 12;
        }
        if !pm && hour == 12 {
            hour = 0;
        }
    }
    let calendar_date = if let Some(ordinal) = ordinal {
        let date = NaiveDate::from_yo_opt(year, ordinal)?;
        if (month_set && date.month() != month) || (day_set && date.day() != day) {
            return None;
        }
        date
    } else {
        NaiveDate::from_ymd_opt(year, month, day)?
    };
    let clock = NaiveTime::from_hms_nano_opt(hour, minute, second, nanos)?;
    let mut zone = if explicit_utc {
        Timezone::Utc
    } else {
        base_zone.clone()
    };
    let initial = date_time(
        calendar_date.year(),
        calendar_date.month() as i32,
        calendar_date.day() as i32,
        clock,
        &zone,
    )?;
    if !explicit_utc {
        if let Some(offset) = parsed_offset {
            if initial.offset().fix().local_minus_utc() != offset {
                zone = Timezone::fixed("", offset).ok()?;
            }
        } else if let Some(name) = parsed_zone_name {
            if initial.offset().to_string() != name {
                let offset = name
                    .strip_prefix("GMT")
                    .filter(|suffix| !suffix.is_empty())
                    .and_then(|suffix| suffix.parse::<i32>().ok())
                    .unwrap_or(0)
                    * 3600;
                zone = Timezone::fixed(name, offset).ok()?;
            }
        }
    }
    let mut value = date_time(
        calendar_date.year(),
        calendar_date.month() as i32,
        calendar_date.day() as i32,
        clock,
        &zone,
    )?;
    let has_ordinal = layout.contains("002") || layout.contains("__2");
    let has_day = has_ordinal || checker.contains('4');
    let has_month = has_ordinal || checker.contains('3') || checker.contains("Mar");
    let mut period = Period::Day;
    if !has_month && !has_day {
        period = Period::Year;
        value = apply_month(configuration, &value)?;
        value = apply_day(configuration, &value)?;
    } else if !has_month {
        period = Period::Year;
        value = apply_month(configuration, &value)?;
    } else if !has_day {
        period = Period::Month;
        value = apply_day(configuration, &value)?;
    }
    let now = configuration.current_time.as_ref()?;
    if value.year() == 0 {
        value = date_time(
            now.year(),
            value.month() as i32,
            value.day() as i32,
            value.time(),
            &value.timezone(),
        )?;
    } else if layout.contains("06") && !layout.contains("2006") {
        let comparable_now = date_time(
            now.year(),
            now.month() as i32,
            now.day() as i32,
            now.time(),
            &value.timezone(),
        )?;
        let mut year = value.year();
        if configuration.preferred_date_source == PreferredDateSource::Past
            && comparable_now < value
        {
            year -= 100;
        } else if configuration.preferred_date_source == PreferredDateSource::Future
            && comparable_now >= value
        {
            year += 100;
        }
        if value.month() == 2 && value.day() == 29 && !leap_year(year) {
            year = correct_leap_year(
                year,
                if configuration.preferred_date_source == PreferredDateSource::Future {
                    PreferredDateSource::Future
                } else {
                    PreferredDateSource::Past
                },
            );
        }
        value = date_time(
            year,
            value.month() as i32,
            value.day() as i32,
            value.time(),
            &value.timezone(),
        )?;
    }
    Some(Date {
        time: value,
        period,
        locale: String::new(),
    })
}

pub(crate) fn parse(configuration: &Configuration, input: &str, formats: &[&str]) -> Option<Date> {
    let zone = detected_zone(input)
        .or_else(|| configuration.default_timezone.clone())
        .unwrap_or(Timezone::Utc);
    formats
        .iter()
        .find_map(|format| parse_layout(configuration, input, format, &zone))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn formatted_go_layouts_preserve_ordinal_and_century_rules() {
        let mut configuration = Configuration {
            current_time: Some(Timezone::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            ..Configuration::default()
        };
        for (input, format, expected) in [
            ("2023-100", "2006-002", "2023-04-10"),
            ("2024  60", "2006 __2", "2024-02-29"),
            ("1999050", "2006002", "1999-02-19"),
            ("25-03-14", "02-01-06", "2014-03-25"),
        ] {
            let date = parse(&configuration, input, &[format]).unwrap();
            assert_eq!(date.time.format("%F").to_string(), expected);
            assert_eq!(date.period, Period::Day);
        }
        assert!(parse(&configuration, "1999366", &["2006002"]).is_none());
        configuration.preferred_date_source = PreferredDateSource::Past;
        assert_eq!(
            parse(&configuration, "1/15/64", &["1/2/06"])
                .unwrap()
                .time
                .year(),
            1964
        );
        configuration.preferred_date_source = PreferredDateSource::Future;
        assert_eq!(
            parse(&configuration, "2/29/00", &["1/2/06"])
                .unwrap()
                .time
                .year(),
            2104
        );
    }
}
