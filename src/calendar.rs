use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Offset, TimeZone};

use crate::{
    Configuration, Period, PreferredDateSource, PreferredDayOfMonth, PreferredMonthOfYear, Timezone,
};

pub(crate) fn leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub(crate) fn last_day(year: i32, month: u32) -> u32 {
    match month {
        2 if leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

pub(crate) fn correct_leap_year(year: i32, source: PreferredDateSource) -> i32 {
    let mut previous = year;
    while !leap_year(previous) {
        previous -= 1;
    }
    let mut next = year;
    while !leap_year(next) {
        next += 1;
    }
    match source {
        PreferredDateSource::Past => previous,
        PreferredDateSource::Future => next,
        PreferredDateSource::CurrentPeriod if next - year < year - previous => next,
        PreferredDateSource::CurrentPeriod => previous,
    }
}

pub(crate) fn date_time(
    year: i32,
    month: i32,
    day: i32,
    time: NaiveTime,
    zone: &Timezone,
) -> Option<DateTime<Timezone>> {
    let year = year.checked_add((month - 1).div_euclid(12))?;
    let month = (month - 1).rem_euclid(12) as u32 + 1;
    let local = NaiveDate::from_ymd_opt(year, month, 1)?
        .checked_add_signed(Duration::days(i64::from(day) - 1))?
        .and_time(time);
    let initial_offset = zone
        .offset_from_utc_datetime(&local)
        .fix()
        .local_minus_utc();
    let tentative_utc = local.checked_sub_signed(Duration::seconds(initial_offset.into()))?;
    let final_offset = zone
        .offset_from_utc_datetime(&tentative_utc)
        .fix()
        .local_minus_utc();
    let utc = local.checked_sub_signed(Duration::seconds(final_offset.into()))?;
    Some(zone.from_utc_datetime(&utc))
}

pub(crate) fn add_date(
    value: &DateTime<Timezone>,
    years: i32,
    months: i32,
    days: i32,
) -> Option<DateTime<Timezone>> {
    date_time(
        value.year().checked_add(years)?,
        value.month() as i32 + months,
        value.day() as i32 + days,
        value.time(),
        &value.timezone(),
    )
}

pub(crate) fn apply_month(
    configuration: &Configuration,
    value: &DateTime<Timezone>,
) -> Option<DateTime<Timezone>> {
    let month = match configuration.preferred_month_of_year {
        PreferredMonthOfYear::CurrentMonth => configuration.current_time.as_ref()?.month(),
        PreferredMonthOfYear::FirstMonth => 1,
        PreferredMonthOfYear::LastMonth => 12,
    };
    date_time(
        value.year(),
        month as i32,
        value.day() as i32,
        value.time(),
        &value.timezone(),
    )
}

pub(crate) fn apply_day(
    configuration: &Configuration,
    value: &DateTime<Timezone>,
) -> Option<DateTime<Timezone>> {
    let last = last_day(value.year(), value.month());
    let day = match configuration.preferred_day_of_month {
        PreferredDayOfMonth::Current => configuration.current_time.as_ref()?.day().min(last),
        PreferredDayOfMonth::First => 1,
        PreferredDayOfMonth::Last => last,
    };
    date_time(
        value.year(),
        value.month() as i32,
        day as i32,
        value.time(),
        &value.timezone(),
    )
}

pub(crate) fn parse_time(input: &str) -> Option<(NaiveTime, Period)> {
    let text = input.trim().to_ascii_uppercase();
    let (clock, meridian) = if let Some(clock) = text.strip_suffix(" AM") {
        (clock.trim_end_matches(' '), Some(false))
    } else if let Some(clock) = text.strip_suffix(" PM") {
        (clock.trim_end_matches(' '), Some(true))
    } else {
        (text.as_str(), None)
    };
    let parts: Vec<_> = clock.split(':').collect();
    if parts.len() > 3 || (parts.len() == 1 && meridian.is_none()) {
        return None;
    }
    let number = |value: &str| -> Option<u32> {
        if value.is_empty() || value.len() > 2 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        value.parse().ok()
    };
    let mut hour = number(parts[0])?;
    if hour > 23 || (meridian.is_some() && parts.len() != 2 && hour > 12) {
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
    let minute = if parts.len() >= 2 {
        number(parts[1])?
    } else {
        0
    };
    let mut second = 0;
    let mut nanos = 0;
    if parts.len() == 3 {
        if let Some((whole, fraction)) = parts[2].split_once(['.', ',']) {
            second = number(whole)?;
            if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let width = fraction.len().min(9);
            nanos = fraction[..width].parse::<u32>().ok()? * 10u32.pow(9 - width as u32);
        } else {
            second = number(parts[2])?;
        }
    }
    if second >= 60 {
        return None;
    }
    let time = NaiveTime::from_hms_nano_opt(hour, minute, second, nanos)?;
    let period = match parts.len() {
        1 => Period::Hour,
        2 => Period::Minute,
        _ => Period::Second,
    };
    Some((time, period))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn calendar_preserves_go_rollover_and_dst_resolution() {
        let midnight = NaiveTime::MIN;
        let value = date_time(2024, 1, 31, midnight, &Timezone::Utc).unwrap();
        assert_eq!(
            add_date(&value, 0, 1, 0).unwrap().format("%F").to_string(),
            "2024-03-02"
        );
        let zone: Timezone = "America/New_York".parse().unwrap();
        let gap = date_time(
            2024,
            3,
            10,
            NaiveTime::from_hms_opt(2, 30, 0).unwrap(),
            &zone,
        )
        .unwrap();
        assert_eq!(
            gap.format("%F %T %:z").to_string(),
            "2024-03-10 01:30:00 -05:00"
        );
        let overlap = date_time(
            2024,
            11,
            3,
            NaiveTime::from_hms_opt(1, 30, 0).unwrap(),
            &zone,
        )
        .unwrap();
        assert_eq!(overlap.offset().fix().local_minus_utc(), -14_400);
    }

    #[test]
    fn common_time_retains_go_precision_and_meridian_rules() {
        for (input, hour, minute, second, nanos, period) in [
            ("13:2:3", 13, 2, 3, 0, Period::Second),
            ("0 AM", 0, 0, 0, 0, Period::Hour),
            ("12 AM", 0, 0, 0, 0, Period::Hour),
            ("16:50 pm", 16, 50, 0, 0, Period::Minute),
            (
                "4:2:3.12345678999 PM",
                16,
                2,
                3,
                123_456_789,
                Period::Second,
            ),
        ] {
            let (time, actual_period) = parse_time(input).unwrap();
            assert_eq!(
                (time.hour(), time.minute(), time.second(), time.nanosecond()),
                (hour, minute, second, nanos)
            );
            assert_eq!(actual_period, period);
        }
        for input in ["24:00", "13:30:00 PM", "10:60", "10:30:60", "12:00."] {
            assert!(parse_time(input).is_none(), "{input}");
        }
    }
}
