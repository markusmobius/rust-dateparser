use std::collections::HashSet;

use chrono::{DateTime, Datelike, Duration, NaiveTime, Offset};

use crate::calendar::{
    add_date, apply_day, apply_month, correct_leap_year, date_time, last_day, leap_year, parse_time,
};
use crate::tokenizer::{tokenize, Kind, Token};
use crate::{Configuration, Date, DateOrder, Period, PreferredDateSource, Timezone};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Part {
    Day,
    Month,
    Year,
    Weekday,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Month => "month",
            Self::Year => "year",
            Self::Weekday => "weekday",
        }
    }
}

pub(crate) trait Calendar {
    fn current_date(&self, now: &DateTime<Timezone>) -> Result<[i32; 3], String>;
    fn number_value(&self, part: Part, text: &str, width: usize) -> Option<i32>;
    fn letter_value(&self, part: Part, text: &str) -> Option<i32>;
    fn create_date(
        &self,
        configuration: &Configuration,
        parts: [i32; 3],
        default_day: bool,
        clock: NaiveTime,
        zone: &Timezone,
    ) -> Result<DateTime<Timezone>, String>;
}

struct Gregorian;

impl Calendar for Gregorian {
    fn current_date(&self, now: &DateTime<Timezone>) -> Result<[i32; 3], String> {
        Ok([now.year(), now.month() as i32, now.day() as i32])
    }

    fn number_value(&self, part: Part, text: &str, width: usize) -> Option<i32> {
        number_value(part, text, width)
    }

    fn letter_value(&self, part: Part, text: &str) -> Option<i32> {
        letter_value(part, text)
    }

    fn create_date(
        &self,
        configuration: &Configuration,
        [mut year, month, mut day]: [i32; 3],
        _default_day: bool,
        clock: NaiveTime,
        zone: &Timezone,
    ) -> Result<DateTime<Timezone>, String> {
        if day == 29 && month == 2 && !leap_year(year) {
            year = correct_leap_year(year, configuration.preferred_date_source);
        }
        day = day.min(last_day(year, month as u32) as i32);
        date_time(year, month, day, clock, zone).ok_or_else(calendar_range_error)
    }
}

fn calendar_range_error() -> String {
    "date is outside the supported calendar range".into()
}

struct Parser<'configuration, CalendarType> {
    calendar: CalendarType,
    configuration: &'configuration Configuration,
    now: DateTime<Timezone>,
    tokens: Vec<Token>,
    filtered: Vec<(usize, Token)>,
    order: Vec<Part>,
    values: [Option<i32>; 4],
    component_tokens: [Option<Token>; 4],
    time_token: Option<String>,
    auto_order: Vec<Part>,
    unset_tokens: Vec<Token>,
    skipped: HashSet<usize>,
    skip_year: bool,
    explicit_order: bool,
}

pub(crate) fn parse(
    configuration: &Configuration,
    input: &str,
    timezone_offset: Option<i32>,
) -> Option<Date> {
    parse_with_order(
        configuration,
        input,
        timezone_offset,
        configuration.date_order.unwrap_or(DateOrder::Mdy).as_str(),
        configuration.date_order.is_some(),
    )
}

pub(crate) fn parse_with_order(
    configuration: &Configuration,
    input: &str,
    timezone_offset: Option<i32>,
    date_order: &str,
    explicit_order: bool,
) -> Option<Date> {
    parse_with_calendar(
        configuration,
        input,
        timezone_offset,
        date_order,
        explicit_order,
        Gregorian,
    )
    .ok()
}

pub(crate) fn parse_with_calendar(
    configuration: &Configuration,
    input: &str,
    timezone_offset: Option<i32>,
    date_order: &str,
    explicit_order: bool,
    calendar: impl Calendar,
) -> Result<Date, String> {
    let input = crate::text::sanitize_spaces(input);
    if input.is_empty() {
        return Err("string is empty".into());
    }
    let mut tokens = tokenize(&input);
    for token in &mut tokens {
        token.text = token.text.trim().into();
    }
    let filtered = tokens
        .iter()
        .cloned()
        .enumerate()
        .filter(|(_, token)| token.kind != Kind::Other)
        .collect();
    let order = date_order
        .bytes()
        .filter_map(|part| match part.to_ascii_uppercase() {
            b'D' => Some(Part::Day),
            b'M' => Some(Part::Month),
            b'Y' => Some(Part::Year),
            _ => None,
        })
        .collect();
    let mut parser = Parser {
        calendar,
        configuration,
        now: configuration
            .current_time
            .clone()
            .ok_or_else(calendar_range_error)?,
        tokens,
        filtered,
        order,
        values: [None; 4],
        component_tokens: std::array::from_fn(|_| None),
        time_token: None,
        auto_order: Vec::new(),
        unset_tokens: Vec::new(),
        skipped: HashSet::new(),
        skip_year: false,
        explicit_order,
    };
    parser.initialize()?;
    parser.finish(timezone_offset)
}

impl<CalendarType: Calendar> Parser<'_, CalendarType> {
    fn initialize(&mut self) -> Result<(), String> {
        for index in 0..self.filtered.len() {
            let (original_index, mut token) = self.filtered[index].clone();
            if self.skipped.contains(&index)
                || matches!(token.text.as_str(), "t" | "year" | "hour" | "minute")
            {
                continue;
            }
            let mut meridian_index = index + 1;
            let before_period = self
                .tokens
                .get(original_index + 1)
                .is_some_and(|token| token.text == ".");
            let after_period = original_index
                .checked_sub(1)
                .and_then(|previous| self.tokens.get(previous))
                .is_some_and(|token| token.text == ".");
            if let Some((next_original, next)) = self.filtered.get(index + 1) {
                let last = index + 1 == self.filtered.len() - 1;
                let next_not_period = self
                    .tokens
                    .get(next_original + 1)
                    .is_none_or(|token| token.text != ".");
                if before_period && !after_period && (last || next_not_period) {
                    let combined = format!("{}:{}", token.text, next.text);
                    if next.text.len() == 2
                        && parse_time(&combined).is_some_and(|(_, period)| period == Period::Minute)
                    {
                        token.text = combined;
                        self.skipped.insert(index + 1);
                        meridian_index += 1;
                    }
                }
            }
            let mut micros = String::new();
            if let Some((_, next)) = self.filtered.get(index + 1) {
                let next_original = self
                    .tokens
                    .iter()
                    .position(|original| {
                        original.text == token.text && original.kind == Kind::Digit
                    })
                    .map(|position| position + 1);
                if token.text.contains(':')
                    && next_original
                        .and_then(|position| self.tokens.get(position))
                        .is_some_and(|original| original.text.contains('.'))
                {
                    let mut digits = next
                        .text
                        .chars()
                        .skip_while(|character| !character.is_ascii_digit());
                    micros = digits
                        .by_ref()
                        .take_while(|character| character.is_ascii_digit())
                        .take(6)
                        .collect();
                }
            }
            if !micros.is_empty() {
                meridian_index += 1;
            }
            let meridian = self.filtered.get(meridian_index).and_then(|(_, meridian)| {
                let lower = meridian.text.to_ascii_lowercase();
                [
                    lower.find("am").map(|position| (position, "am")),
                    lower.find("pm").map(|position| (position, "pm")),
                ]
                .into_iter()
                .flatten()
                .min_by_key(|(position, _)| *position)
                .map(|(_, meridian)| meridian)
            });
            if token.text.contains(':') || meridian.is_some() || !micros.is_empty() {
                let mut text = token.text;
                if !micros.is_empty() {
                    text.push('.');
                    text.push_str(&micros);
                    self.skipped.insert(index + 1);
                }
                if let Some(meridian) = meridian {
                    text.push(' ');
                    text.push_str(meridian);
                    self.skipped.insert(meridian_index);
                }
                self.time_token = Some(text);
                continue;
            }
            let results = match token.kind {
                Kind::Digit => self.parse_digit(&token),
                Kind::Letter => self.parse_letter(&token),
                Kind::Other => continue,
            }
            .ok_or_else(|| format!("unable to parse {}", token.text))?;
            for (part, value) in results {
                if token.text.len() == 4 && part == Part::Year {
                    self.skip_year = true;
                }
                self.values[part as usize] = Some(value);
            }
        }
        for part in [Part::Day, Part::Month, Part::Year] {
            if self.values[part as usize].is_none() {
                for token in &self.unset_tokens {
                    if token.kind == Kind::Digit {
                        self.component_tokens[part as usize] = Some(token.clone());
                        self.values[part as usize] = Some(
                            token
                                .text
                                .parse()
                                .map_err(|_| format!("unable to parse {}", token.text))?,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn save(
        &mut self,
        part: Part,
        token: &Token,
        value: i32,
        skip_order: bool,
    ) -> Vec<(Part, i32)> {
        if !skip_order {
            self.auto_order.push(part);
        }
        self.component_tokens[part as usize] = Some(token.clone());
        vec![(part, value)]
    }

    fn try_number_directives(
        &mut self,
        token: &Token,
        skip_short_year: bool,
    ) -> Option<Vec<(Part, i32)>> {
        let order = if self.skip_year
            && self.values[Part::Day as usize].unwrap_or(0) == 0
            && self.values[Part::Month as usize].unwrap_or(0) == 0
            && !self.explicit_order
        {
            vec![Part::Month, Part::Day, Part::Year]
        } else {
            self.order.clone()
        };
        for part in order {
            if self.skip_year && part == Part::Year {
                continue;
            }
            let directives: &[usize] = if part == Part::Year { &[4, 2] } else { &[0] };
            for &width in directives {
                if skip_short_year && width == 2 {
                    continue;
                }
                let Some(value) = self.calendar.number_value(part, &token.text, width) else {
                    continue;
                };
                if self.values[part as usize].unwrap_or(0) == 0 {
                    return Some(self.save(part, token, value, false));
                }
                if let Some(previous) = &self.component_tokens[part as usize] {
                    if previous.kind == Kind::Digit
                        && self
                            .calendar
                            .number_value(part, &previous.text, width)
                            .is_none()
                    {
                        self.unset_tokens.push(previous.clone());
                        return Some(self.save(part, token, value, false));
                    }
                }
            }
        }
        None
    }

    fn parse_digit(&mut self, token: &Token) -> Option<Vec<(Part, i32)>> {
        let mut after_year = false;
        for part in self.order.clone() {
            if after_year && self.values[part as usize].unwrap_or(0) != 0 {
                if let Some(result) = self.try_number_directives(token, true) {
                    return Some(result);
                }
                break;
            }
            if part == Part::Year {
                after_year = true;
            }
        }
        self.try_number_directives(token, false)
    }

    fn parse_letter(&mut self, token: &Token) -> Option<Vec<(Part, i32)>> {
        for part in [Part::Weekday, Part::Month] {
            let Some(value) = self.calendar.letter_value(part, &token.text) else {
                continue;
            };
            let previous = self.values[part as usize].unwrap_or(0);
            if previous == 0 {
                return Some(self.save(part, token, value, true));
            }
            if part == Part::Month {
                let Some(index) = self.auto_order.iter().position(|part| *part == Part::Month)
                else {
                    continue;
                };
                self.auto_order[index] = Part::Day;
                self.component_tokens[Part::Day as usize] =
                    self.component_tokens[Part::Month as usize].clone();
                self.component_tokens[Part::Month as usize] = Some(token.clone());
                return Some(vec![(Part::Month, value), (Part::Day, previous)]);
            }
        }
        None
    }

    fn finish(&self, timezone_offset: Option<i32>) -> Result<Date, String> {
        let missing: Vec<_> = [Part::Day, Part::Month, Part::Year]
            .into_iter()
            .filter(|part| self.values[*part as usize].is_none())
            .collect();
        if self.configuration.strict_parsing && !missing.is_empty() {
            return Err(format!(
                "fields missing from date string: {}",
                missing
                    .iter()
                    .map(|part| part.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(required) = self
            .configuration
            .required_parts
            .iter()
            .find(|required| missing.iter().any(|part| part.name() == *required))
        {
            return Err(format!("required part \"{required}\" is missing"));
        }
        let (clock, clock_period) = if let Some(token) = &self.time_token {
            parse_time(token).ok_or_else(|| format!("{token} doesn't seem to be a valid time"))?
        } else {
            (NaiveTime::MIN, Period::Day)
        };
        let [current_year, current_month, current_day] = self.calendar.current_date(&self.now)?;
        let year = self.values[Part::Year as usize]
            .filter(|year| *year != 0)
            .unwrap_or(current_year);
        let month = self.values[Part::Month as usize]
            .filter(|month| *month != 0)
            .unwrap_or(current_month);
        let day = self.values[Part::Day as usize]
            .filter(|day| *day != 0)
            .unwrap_or(current_day);
        let has = |part: Part| self.component_tokens[part as usize].is_some();
        let mut value = self.calendar.create_date(
            self.configuration,
            [year, month, day],
            !has(Part::Day) && !has(Part::Weekday),
            clock,
            &self.now.timezone(),
        )?;
        value = self
            .correct_time_frame(value, timezone_offset)
            .ok_or_else(calendar_range_error)?;
        let weekday_only =
            has(Part::Weekday) && !has(Part::Year) && !has(Part::Month) && !has(Part::Day);
        if !has(Part::Month) && !weekday_only {
            value = apply_month(self.configuration, &value).ok_or_else(calendar_range_error)?;
        }
        if !has(Part::Day) && self.time_token.is_none() && !has(Part::Weekday) {
            value = apply_day(self.configuration, &value).ok_or_else(calendar_range_error)?;
        }
        let mut period = if self.values[Part::Day as usize].is_some() {
            Period::Day
        } else if self.values[Part::Month as usize].is_some() {
            Period::Month
        } else if self.values[Part::Year as usize].is_some() {
            Period::Year
        } else {
            Period::Day
        };
        if self.time_token.is_some() && self.configuration.return_time_as_period {
            period = period.min(clock_period);
        }
        Ok(Date {
            time: value,
            period,
            locale: String::new(),
        })
    }

    fn correct_time_frame(
        &self,
        mut value: DateTime<Timezone>,
        timezone_offset: Option<i32>,
    ) -> Option<DateTime<Timezone>> {
        let has = |part: Part| self.component_tokens[part as usize].is_some();
        let preference = self.configuration.preferred_date_source;
        if has(Part::Weekday) && !has(Part::Year) && !has(Part::Month) && !has(Part::Day) {
            let weekday = self.component_tokens[Part::Weekday as usize]
                .as_ref()?
                .text
                .to_ascii_lowercase();
            let target = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
                .iter()
                .position(|name| *name == &weekday[..3])? as i32;
            let current = value.weekday().num_days_from_sunday() as i32;
            let steps = if preference == PreferredDateSource::Future {
                let distance = (target - current).rem_euclid(7);
                if distance == 0 {
                    7
                } else {
                    distance
                }
            } else {
                let distance = (current - target).rem_euclid(7);
                if distance == 0 && preference == PreferredDateSource::Past {
                    -7
                } else {
                    -distance
                }
            };
            value = add_date(&value, 0, 0, steps)?;
        }
        if self.values[Part::Month as usize].is_some() && self.values[Part::Year as usize].is_none()
        {
            let mut year = value.year();
            if self.now < value && preference == PreferredDateSource::Past {
                year -= 1;
            }
            if self.now >= value && preference == PreferredDateSource::Future {
                year += 1;
            }
            if value.month() == 2 && value.day() == 29 && !leap_year(year) {
                year = correct_leap_year(year, preference);
            }
            value = date_time(
                year,
                value.month() as i32,
                value.day() as i32,
                value.time(),
                &value.timezone(),
            )?;
        }
        if self.component_tokens[Part::Year as usize]
            .as_ref()
            .is_some_and(|token| token.text.len() == 2)
        {
            if self.now < value && preference == PreferredDateSource::Past {
                value = add_date(&value, -100, 0, 0)?;
            } else if self.now >= value && preference == PreferredDateSource::Future {
                value = add_date(&value, 100, 0, 0)?;
            }
        }
        if self.time_token.is_some()
            && !has(Part::Year)
            && !has(Part::Month)
            && !has(Part::Day)
            && !has(Part::Weekday)
        {
            let offset = timezone_offset.unwrap_or_else(|| {
                self.configuration
                    .default_timezone
                    .as_ref()
                    .map(|zone| value.with_timezone(zone).offset().fix().local_minus_utc())
                    .unwrap_or(0)
            });
            let comparable = value
                .clone()
                .checked_sub_signed(Duration::seconds(offset.into()))?;
            if preference == PreferredDateSource::Past && self.now < comparable {
                value = add_date(&value, 0, 0, -1)?;
            }
            if preference == PreferredDateSource::Future && self.now > comparable {
                value = add_date(&value, 0, 0, 1)?;
            }
        }
        Some(value)
    }
}

fn number_value(part: Part, text: &str, width: usize) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value: i32 = text.parse().ok()?;
    match part {
        Part::Day if text.len() <= 2 && (1..=31).contains(&value) => Some(value),
        Part::Month if text.len() <= 2 && (1..=12).contains(&value) => Some(value),
        Part::Year if width == 4 && text.len() == 4 && value != 1 => Some(value),
        Part::Year if width == 2 && text.len() == 2 => Some(if value >= 69 {
            1900 + value
        } else {
            2000 + value
        }),
        _ => None,
    }
}

fn letter_value(part: Part, text: &str) -> Option<i32> {
    let names: &[&str] = match part {
        Part::Month => &[
            "january",
            "february",
            "march",
            "april",
            "may",
            "june",
            "july",
            "august",
            "september",
            "october",
            "november",
            "december",
        ],
        Part::Weekday => &[
            "sunday",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
        ],
        _ => return None,
    };
    let text = text.to_ascii_lowercase();
    names
        .iter()
        .position(|name| text == *name || text == name[..3])
        .map(|index| {
            if part == Part::Month {
                index as i32 + 1
            } else {
                chrono::NaiveDate::from_ymd_opt(0, 1, 1)
                    .unwrap()
                    .weekday()
                    .num_days_from_sunday() as i32
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn configuration() -> Configuration {
        Configuration {
            current_time: Some(
                Timezone::Utc
                    .with_ymd_and_hms(2026, 9, 12, 12, 30, 45)
                    .unwrap(),
            ),
            ..Configuration::default()
        }
    }

    #[test]
    fn absolute_dates_keep_component_order_and_precision() {
        for (input, expected, period) in [
            ("12 August 2021", "2021-08-12 00:00:00", Period::Day),
            ("2021-10-11", "2021-10-11 00:00:00", Period::Day),
            ("2021-10", "2021-10-12 00:00:00", Period::Month),
            ("2021", "2021-09-12 00:00:00", Period::Year),
            ("February 2000", "2000-02-12 00:00:00", Period::Month),
            ("23 January, 15:10:01", "2026-01-23 15:10:01", Period::Day),
            ("13.20", "2026-09-12 13:20:00", Period::Day),
        ] {
            let date = parse(&configuration(), input, None).unwrap();
            assert_eq!(date.time.format("%F %T").to_string(), expected, "{input}");
            assert_eq!(date.period, period, "{input}");
        }
        let mut configuration = configuration();
        configuration.date_order = Some(DateOrder::Dmy);
        assert_eq!(
            parse(&configuration, "12/04/2018", None)
                .unwrap()
                .time
                .format("%F")
                .to_string(),
            "2018-04-12"
        );
        configuration.return_time_as_period = true;
        assert_eq!(
            parse(&configuration, "11:30", None).unwrap().period,
            Period::Minute
        );
        let fraction = parse(&configuration, "2018-04-12 17:20:03.12345678999", None).unwrap();
        assert_eq!(fraction.time.timestamp_subsec_nanos(), 123_456_000);
        assert_eq!(fraction.time.format("%F").to_string(), "2018-12-04");
    }

    #[test]
    fn absolute_preferences_and_required_parts_use_frozen_time() {
        let mut configuration = configuration();
        configuration.strict_parsing = true;
        assert!(parse(&configuration, "March", None).is_none());
        assert!(parse(&configuration, "11 March", None).is_none());
        assert!(parse(&configuration, "11 March 2024", None).is_some());
        configuration.strict_parsing = false;
        configuration.preferred_date_source = PreferredDateSource::Past;
        assert_eq!(
            parse(&configuration, "January 15, 64", None)
                .unwrap()
                .time
                .year(),
            1964
        );
        configuration.preferred_date_source = PreferredDateSource::Future;
        assert_eq!(
            parse(&configuration, "January 15, 24", None)
                .unwrap()
                .time
                .year(),
            2124
        );
        assert_eq!(
            parse(&configuration, "saturday", None).unwrap().time.day(),
            19
        );
        configuration.preferred_date_source = PreferredDateSource::Past;
        assert_eq!(
            parse(&configuration, "saturday", None).unwrap().time.day(),
            5
        );
        configuration.preferred_date_source = PreferredDateSource::CurrentPeriod;
        configuration.preferred_day_of_month = crate::PreferredDayOfMonth::Last;
        assert_eq!(
            parse(&configuration, "February 2024", None)
                .unwrap()
                .time
                .day(),
            29
        );
        configuration.required_parts = vec!["year".into()];
        assert!(parse(&configuration, "11 March", None).is_none());
        assert!(parse(&configuration, "March 2024", None).is_some());
    }
}
