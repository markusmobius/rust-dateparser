use std::{
    fmt,
    str::FromStr,
    sync::{Arc, OnceLock},
};

use chrono::{
    FixedOffset, Local, MappedLocalTime, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc,
};
use chrono_tz::Tz;

use crate::Error;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Timezone {
    #[default]
    Utc,
    Local,
    Fixed {
        name: Arc<str>,
        offset: FixedOffset,
    },
    Iana(Tz),
}

impl Timezone {
    pub fn fixed(name: impl Into<Arc<str>>, seconds_east: i32) -> Result<Self, Error> {
        let name = name.into();
        let offset = FixedOffset::east_opt(seconds_east)
            .ok_or_else(|| Error::InvalidTimezone(name.to_string()))?;
        Ok(Self::Fixed { name, offset })
    }

    pub fn load(name: &str) -> Result<Self, Error> {
        let upper = name.to_uppercase();
        if let Ok(index) =
            crate::timezone_data::TIMEZONES.binary_search_by_key(&upper.as_str(), |entry| entry.0)
        {
            return Self::fixed(name, crate::timezone_data::TIMEZONES[index].2);
        }
        if let Ok(timezone) = name.parse() {
            return Ok(timezone);
        }
        if let Some(Self::Fixed { offset, .. }) = detected_zone(&format!(" {name}")) {
            return Self::fixed(name, offset.local_minus_utc());
        }
        Err(Error::InvalidTimezone(name.into()))
    }
}

fn matcher_index(input: &str) -> Option<usize> {
    static MATCHERS: OnceLock<regex::RegexSet> = OnceLock::new();
    let matchers = MATCHERS.get_or_init(|| {
        regex::RegexSetBuilder::new(crate::timezone_data::MATCHERS.iter().map(|entry| entry.2))
            .size_limit(64 * 1024 * 1024)
            .build()
            .expect("pinned Go timezone patterns must compile")
    });
    if !matchers.is_match(input) {
        return None;
    }
    matchers.matches(input).iter().next()
}

pub(crate) fn detected_zone(input: &str) -> Option<Timezone> {
    let index = matcher_index(input)?;
    let (name, offset, _) = crate::timezone_data::MATCHERS[index];
    Timezone::fixed(name, offset).ok()
}

pub(crate) fn is_token(input: &str) -> bool {
    static EXPRESSION: OnceLock<regex::Regex> = OnceLock::new();
    EXPRESSION
        .get_or_init(|| {
            regex::Regex::new(crate::timezone_data::TOKEN_PATTERN)
                .expect("pinned timezone token expression must compile")
        })
        .is_match(input.trim())
}

pub(crate) fn word_is_timezone(input: &str) -> bool {
    static EXPRESSION: OnceLock<regex::Regex> = OnceLock::new();
    EXPRESSION
        .get_or_init(|| {
            regex::Regex::new(crate::timezone_data::SEARCH_PATTERN)
                .expect("pinned timezone search expression must compile")
        })
        .is_match(input)
}

pub(crate) fn pop_offset(input: &str) -> (String, Option<Timezone>) {
    let Some(index) = matcher_index(input) else {
        return (input.into(), None);
    };
    static EXPRESSIONS: OnceLock<Vec<OnceLock<regex::Regex>>> = OnceLock::new();
    let expressions = EXPRESSIONS.get_or_init(|| {
        crate::timezone_data::MATCHERS
            .iter()
            .map(|_| OnceLock::new())
            .collect()
    });
    let (name, offset, pattern) = crate::timezone_data::MATCHERS[index];
    let expression = expressions[index].get_or_init(|| {
        regex::Regex::new(pattern).expect("pinned timezone expression must compile")
    });
    let matched = expression
        .find(input)
        .expect("timezone set and expression must agree");
    let mut bytes = input.as_bytes()[..matched.start() + 1].to_vec();
    bytes.extend_from_slice(&input.as_bytes()[matched.end()..]);
    (
        String::from_utf8_lossy(&bytes).into_owned(),
        Timezone::fixed(name, offset).ok(),
    )
}

impl FromStr for Timezone {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "" | "UTC" => Ok(Self::Utc),
            "Local" => Ok(Self::Local),
            _ => name
                .parse::<Tz>()
                .map(Self::Iana)
                .map_err(|_| Error::InvalidTimezone(name.into())),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimezoneOffset {
    timezone: Timezone,
    fixed: FixedOffset,
    name: Arc<str>,
}

impl fmt::Display for TimezoneOffset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.name)
    }
}

impl Offset for TimezoneOffset {
    fn fix(&self) -> FixedOffset {
        self.fixed
    }
}

impl Timezone {
    fn offset(&self, fixed: FixedOffset, name: impl Into<Arc<str>>) -> TimezoneOffset {
        TimezoneOffset {
            timezone: self.clone(),
            fixed,
            name: name.into(),
        }
    }
}

impl TimeZone for Timezone {
    type Offset = TimezoneOffset;

    fn from_offset(offset: &Self::Offset) -> Self {
        offset.timezone.clone()
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<Self::Offset> {
        self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> MappedLocalTime<Self::Offset> {
        match self {
            Self::Utc => Utc
                .offset_from_local_datetime(local)
                .map(|offset| self.offset(offset.fix(), "UTC")),
            Self::Local => Local
                .offset_from_local_datetime(local)
                .map(|offset| self.offset(offset, offset.to_string())),
            Self::Fixed { name, offset } => {
                MappedLocalTime::Single(self.offset(*offset, name.clone()))
            }
            Self::Iana(timezone) => timezone
                .offset_from_local_datetime(local)
                .map(|offset| self.offset(offset.fix(), offset.to_string())),
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> Self::Offset {
        self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> Self::Offset {
        match self {
            Self::Utc => self.offset(Utc.fix(), "UTC"),
            Self::Local => {
                let offset = Local.offset_from_utc_datetime(utc);
                self.offset(offset, offset.to_string())
            }
            Self::Fixed { name, offset } => self.offset(*offset, name.clone()),
            Self::Iana(timezone) => {
                let offset = timezone.offset_from_utc_datetime(utc);
                self.offset(offset.fix(), offset.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timezone_retains_identity_and_dst_offsets() {
        let timezone: Timezone = "America/New_York".parse().unwrap();
        let winter = timezone.with_ymd_and_hms(2024, 3, 9, 12, 0, 0).unwrap();
        let summer = timezone.with_ymd_and_hms(2024, 3, 10, 12, 0, 0).unwrap();
        assert_eq!(winter.offset().fix().local_minus_utc(), -18_000);
        assert_eq!(summer.offset().fix().local_minus_utc(), -14_400);
        assert_eq!(winter.timezone(), timezone);
        assert_eq!(summer.timezone(), timezone);
        assert_eq!(winter.format("%Z").to_string(), "EST");
        assert_eq!(summer.format("%Z").to_string(), "EDT");
        assert!(timezone
            .with_ymd_and_hms(2024, 3, 10, 2, 30, 0)
            .single()
            .is_none());
        assert!(matches!(
            timezone.with_ymd_and_hms(2024, 11, 3, 1, 30, 0),
            MappedLocalTime::Ambiguous(_, _)
        ));
    }

    #[test]
    fn timezone_preserves_fixed_names() {
        let timezone = Timezone::fixed("NPT", 20_700).unwrap();
        let time = timezone.with_ymd_and_hms(2024, 2, 29, 12, 34, 56).unwrap();
        assert_eq!(time.timezone(), timezone);
        assert_eq!(time.format("%Z %:z").to_string(), "NPT +05:45");
        assert!(Timezone::fixed("overflow", 86_400).is_err());
        assert!("Not/AZone".parse::<Timezone>().is_err());
    }
}
