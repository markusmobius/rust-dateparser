use chrono::{DateTime, Datelike, NaiveTime};
use regex::Regex;
use serde::Deserialize;
use std::{collections::HashMap, sync::OnceLock};

use crate::absolute::{Calendar, Part};
use crate::calendar::date_time;
use crate::{Configuration, Date, DateOrder, Error, Timezone};

struct Jalali;
struct Hijri;

#[derive(Deserialize)]
struct Translations {
    replacement: String,
    aliases: Vec<String>,
}

#[derive(Deserialize)]
struct CalendarData {
    jalali_months: Vec<Translations>,
    jalali_weekdays: Vec<Translations>,
    jalali_days: Vec<Translations>,
    jalali_patterns: HashMap<String, String>,
}

fn data() -> &'static CalendarData {
    static DATA: OnceLock<CalendarData> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../data/calendars.json")).unwrap())
}

#[derive(Deserialize)]
struct CalendarTable {
    first_year: i32,
    #[serde(default)]
    year_starts: Vec<i64>,
    #[serde(default)]
    month_starts: Vec<i64>,
    gregorian_min: i64,
    gregorian_max: i64,
}

#[derive(Deserialize)]
struct ConversionData {
    jalali: CalendarTable,
    hijri: CalendarTable,
}

fn conversions() -> &'static ConversionData {
    static DATA: OnceLock<ConversionData> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/calendar-conversions.json")).unwrap()
    })
}

const JALALI_RANGE: &str = "date is outside the supported Jalali range";
const HIJRI_RANGE: &str = "date is outside Umm al-Qura scope";

struct JalaliExpressions {
    times: [Regex; 3],
    ordinal: Regex,
    colon: Regex,
    non_digit: Regex,
}

fn translate_jalali(mut input: String) -> String {
    static EXPRESSIONS: OnceLock<JalaliExpressions> = OnceLock::new();
    let expressions = EXPRESSIONS.get_or_init(|| {
        let compile = |name: &str| Regex::new(&data().jalali_patterns[name]).unwrap();
        JalaliExpressions {
            times: ["rxHourPattern", "rxMinutePattern", "rxSecondPattern"].map(compile),
            ordinal: compile("rxOrdinalPattern"),
            colon: compile("rxColonPattern"),
            non_digit: compile("rxNonDigit"),
        }
    });
    for translation in data().jalali_months.iter().chain(&data().jalali_weekdays) {
        for alias in &translation.aliases {
            input = input.replace(alias, &format!(" {} ", translation.replacement));
        }
    }
    input = expressions.ordinal.replace_all(&input, "").into_owned();
    for translation in &data().jalali_days {
        for alias in &translation.aliases {
            input = input.replace(alias, &translation.replacement);
        }
    }
    for pattern in &expressions.times {
        input = pattern
            .replace_all(&input, |captures: &regex::Captures<'_>| {
                expressions
                    .non_digit
                    .replace_all(&captures[0], " ")
                    .into_owned()
            })
            .into_owned();
    }
    input = expressions
        .colon
        .replace_all(&input, ":")
        .replace("\u{633}\u{627}\u{639}\u{62a}", "");
    for translation in &data().jalali_weekdays {
        input = input.replace(&translation.replacement, " ");
    }
    input
}

pub fn parse_jalali(configuration: &Configuration, input: &str) -> Result<Date, Error> {
    let order = calendar_order(configuration);
    let configuration = configuration.initialized()?;
    let input = translate_jalali(crate::text::normalize_digits(
        &crate::text::normalize_unicode(input),
    ));
    parse_prepared(&configuration, &input, order, Jalali)
}

pub fn parse_hijri(configuration: &Configuration, input: &str) -> Result<Date, Error> {
    let order = calendar_order(configuration);
    let configuration = configuration.initialized()?;
    let input = crate::text::normalize_digits(&crate::text::normalize(input))
        .replace("\u{635}\u{628}\u{627}\u{62d}\u{627}", "am")
        .replace("\u{645}\u{633}\u{627}\u{621}", "pm");
    parse_prepared(&configuration, &input, order, Hijri)
}

fn calendar_order(configuration: &Configuration) -> DateOrder {
    configuration.date_order_for_locale.as_ref().map_or_else(
        || configuration.date_order.unwrap_or(DateOrder::Mdy),
        |resolver| resolver.resolve("").unwrap_or(DateOrder::Mdy),
    )
}

fn parse_prepared(
    configuration: &Configuration,
    input: &str,
    order: DateOrder,
    calendar: impl Calendar,
) -> Result<Date, Error> {
    let input = crate::text::strip_braces(&crate::text::sanitize_date(input));
    crate::absolute::parse_with_calendar(
        configuration,
        &input,
        None,
        order.as_str(),
        configuration.date_order.is_some() || configuration.date_order_for_locale.is_some(),
        calendar,
    )
    .map_err(Error::CalendarParsing)
}

fn calendar_number(part: Part, text: &str, cutoff: i32, max_day: i32) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = text.parse::<i32>().ok()?;
    match part {
        Part::Year if text.len() == 4 => Some(number),
        Part::Year if text.len() == 2 => Some(number + if number >= cutoff { 1300 } else { 1400 }),
        Part::Month if text.len() <= 2 && (1..=12).contains(&number) => Some(number),
        Part::Day if text.len() <= 2 && (1..=max_day).contains(&number) => Some(number),
        _ => None,
    }
}

impl Calendar for Jalali {
    fn current_date(&self, now: &DateTime<Timezone>) -> Result<[i32; 3], String> {
        jalali_from_gregorian(now)
    }

    fn number_value(&self, part: Part, text: &str, _width: usize) -> Option<i32> {
        calendar_number(part, text, 61, 31)
    }

    fn letter_value(&self, part: Part, text: &str) -> Option<i32> {
        if part == Part::Month {
            data()
                .jalali_months
                .iter()
                .position(|month| month.replacement == text)
                .map(|index| index as i32 + 1)
        } else {
            None
        }
    }

    fn create_date(
        &self,
        _configuration: &Configuration,
        [year, month, mut day]: [i32; 3],
        default_day: bool,
        clock: NaiveTime,
        zone: &Timezone,
    ) -> Result<DateTime<Timezone>, String> {
        let last_day = jalali_month_length(year, month)?;
        if default_day {
            day = day.min(last_day);
        }
        let [year, month, day] = jalali_to_gregorian(year, month, day)?;
        date_time(year, month, day, clock, zone)
            .ok_or_else(|| "date is outside the supported calendar range".into())
    }
}

impl Calendar for Hijri {
    fn current_date(&self, now: &DateTime<Timezone>) -> Result<[i32; 3], String> {
        hijri_from_gregorian(now)
    }

    fn number_value(&self, part: Part, text: &str, _width: usize) -> Option<i32> {
        calendar_number(part, text, 90, 30)
    }

    fn letter_value(&self, _part: Part, _text: &str) -> Option<i32> {
        None
    }

    fn create_date(
        &self,
        _configuration: &Configuration,
        [year, month, mut day]: [i32; 3],
        default_day: bool,
        clock: NaiveTime,
        zone: &Timezone,
    ) -> Result<DateTime<Timezone>, String> {
        let last_day = hijri_month_length(year, month)?;
        if default_day {
            day = day.min(last_day);
        }
        let [year, month, day] = hijri_to_gregorian(year, month, day)?;
        date_time(year, month, day, clock, zone)
            .ok_or_else(|| "date is outside the supported calendar range".into())
    }
}

fn gregorian_day(now: &DateTime<Timezone>) -> i64 {
    now.date_naive()
        .and_time(NaiveTime::MIN)
        .and_utc()
        .timestamp()
        / 86400
}

fn gregorian_parts(day: i64) -> Result<[i32; 3], String> {
    let value = DateTime::from_timestamp(day * 86400, 0).ok_or(JALALI_RANGE)?;
    if !(1..=9999).contains(&value.year()) {
        return Err(JALALI_RANGE.into());
    }
    Ok([value.year(), value.month() as i32, value.day() as i32])
}

fn jalali_from_gregorian(now: &DateTime<Timezone>) -> Result<[i32; 3], String> {
    let table = &conversions().jalali;
    let day = gregorian_day(now);
    if !(table.gregorian_min..=table.gregorian_max).contains(&day) {
        return Err(JALALI_RANGE.into());
    }
    let index = table.year_starts.partition_point(|start| *start <= day) - 1;
    let remaining = day - table.year_starts[index];
    let (month, month_day) = if remaining < 186 {
        (remaining / 31 + 1, remaining % 31 + 1)
    } else {
        ((remaining - 186) / 30 + 7, (remaining - 186) % 30 + 1)
    };
    Ok([
        table.first_year + index as i32,
        month as i32,
        month_day as i32,
    ])
}

fn jalali_index(year: i32, month: i32) -> Result<usize, String> {
    let table = &conversions().jalali;
    if year < table.first_year
        || year >= table.first_year + table.year_starts.len() as i32
        || !(1..=12).contains(&month)
    {
        return Err(JALALI_RANGE.into());
    }
    Ok((year - table.first_year) as usize)
}

fn jalali_month_length(year: i32, month: i32) -> Result<i32, String> {
    let index = jalali_index(year, month)?;
    match month {
        1..=6 => Ok(31),
        7..=11 => Ok(30),
        _ => {
            let starts = &conversions().jalali.year_starts;
            Ok((starts.get(index + 1).ok_or(JALALI_RANGE)? - starts[index] - 336) as i32)
        }
    }
}

fn jalali_to_gregorian(year: i32, month: i32, day: i32) -> Result<[i32; 3], String> {
    let index = jalali_index(year, month)?;
    if day < 1 {
        return Err(JALALI_RANGE.into());
    }
    let month_days = if month <= 7 {
        (month - 1) * 31
    } else {
        (month - 1) * 30 + 6
    };
    gregorian_parts(conversions().jalali.year_starts[index] + i64::from(month_days + day - 1))
}

fn hijri_from_gregorian(now: &DateTime<Timezone>) -> Result<[i32; 3], String> {
    let table = &conversions().hijri;
    let day = gregorian_day(now);
    if !(table.gregorian_min..=table.gregorian_max).contains(&day) {
        return Err(HIJRI_RANGE.into());
    }
    let index = table.month_starts.partition_point(|start| *start <= day) - 1;
    Ok([
        table.first_year + (index / 12) as i32,
        (index % 12) as i32 + 1,
        (day - table.month_starts[index] + 1) as i32,
    ])
}

fn hijri_index(year: i32, month: i32) -> Result<usize, String> {
    let table = &conversions().hijri;
    if year < table.first_year
        || year >= table.first_year + ((table.month_starts.len() - 1) / 12) as i32
        || !(1..=12).contains(&month)
    {
        return Err(HIJRI_RANGE.into());
    }
    Ok(((year - table.first_year) * 12 + month - 1) as usize)
}

fn hijri_month_length(year: i32, month: i32) -> Result<i32, String> {
    let index = hijri_index(year, month)?;
    let starts = &conversions().hijri.month_starts;
    Ok((starts[index + 1] - starts[index]) as i32)
}

fn hijri_to_gregorian(year: i32, month: i32, day: i32) -> Result<[i32; 3], String> {
    let index = hijri_index(year, month)?;
    if day < 1 {
        return Err(HIJRI_RANGE.into());
    }
    gregorian_parts(conversions().hijri.month_starts[index] + i64::from(day) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn hijri_uses_python_month_starts_rollover_and_safe_bounds() {
        let configuration = Configuration {
            date_order: Some(DateOrder::Dmy),
            current_time: Some(
                Timezone::Utc
                    .with_ymd_and_hms(2024, 3, 20, 12, 0, 0)
                    .unwrap(),
            ),
            ..Configuration::default()
        };
        for (input, expected) in [
            ("14-09-1432", "2011-08-14"),
            ("30-02-1433", "2012-01-24"),
            ("30-02-33", "2012-01-24"),
            ("10-03-90", "1970-05-16"),
            ("1/1/1343", "1924-08-01"),
            ("1/1/1355", "1936-03-24"),
            ("30/12/1356", "1938-03-02"),
        ] {
            assert_eq!(
                parse_hijri(&configuration, input)
                    .unwrap()
                    .time
                    .format("%F")
                    .to_string(),
                expected,
                "{input}"
            );
        }
        for (index, start) in conversions()
            .hijri
            .month_starts
            .iter()
            .enumerate()
            .take(1896)
        {
            let now = DateTime::from_timestamp(start * 86400, 0)
                .unwrap()
                .with_timezone(&Timezone::Utc);
            assert_eq!(
                hijri_from_gregorian(&now).unwrap(),
                [1343 + (index / 12) as i32, (index % 12) as i32 + 1, 1]
            );
        }
        assert_eq!(calendar_number(Part::Year, "89", 90, 30), Some(1489));
        assert_eq!(calendar_number(Part::Year, "90", 90, 30), Some(1390));
        for input in ["1/1/1342", "1/1/1501", "1/1/1444 GMT+05:30", "1/1/1444 UTC"] {
            assert!(parse_hijri(&configuration, input).is_err(), "{input}");
        }
    }

    #[test]
    fn jalali_numeric_dates_use_python_conversion_and_parser_rules() {
        let configuration = Configuration {
            date_order: Some(DateOrder::Dmy),
            current_time: Some(
                Timezone::Utc
                    .with_ymd_and_hms(2024, 3, 20, 12, 0, 0)
                    .unwrap(),
            ),
            ..Configuration::default()
        };
        for (input, expected) in [
            ("1/1/1403", "2024-03-20"),
            ("26/6/1394", "2015-09-17"),
            ("22/6/02", "2023-09-13"),
            ("30/12/1387", "2009-03-20"),
            ("31/1/1403", "2024-04-19"),
            ("30/12/1356", "1978-03-21"),
            ("01/02", "2024-04-20"),
            ("23:30", "2024-03-20"),
            ("1/1/2378", "2999-03-21"),
        ] {
            let parsed = parse_jalali(&configuration, input).unwrap();
            assert_eq!(parsed.time.format("%F").to_string(), expected, "{input}");
            assert_eq!(parsed.locale, "");
        }
        assert_eq!(
            Jalali
                .current_date(configuration.current_time.as_ref().unwrap())
                .unwrap(),
            [1403, 1, 1]
        );
        for input in [
            "1/1/2379",
            "1/12/2378",
            "1/1/3178",
            "1/1/1444 GMT+05:30",
            "1/1/1444 UTC",
        ] {
            assert!(parse_jalali(&configuration, input).is_err(), "{input}");
        }
        assert_eq!(calendar_number(Part::Year, "60", 61, 31), Some(1460));
        assert_eq!(calendar_number(Part::Year, "61", 61, 31), Some(1361));
        let default_order = Configuration {
            date_order: None,
            ..configuration
        };
        assert_eq!(
            parse_jalali(&default_order, "04-03-1433")
                .unwrap()
                .time
                .format("%F")
                .to_string(),
            "2054-06-24"
        );
        assert_eq!(
            parse_hijri(&default_order, "04-03-1433")
                .unwrap()
                .time
                .format("%F")
                .to_string(),
            "2012-02-25"
        );
    }
}
