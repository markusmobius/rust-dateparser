use chrono::{DateTime, TimeZone, Utc};
use std::{fmt, sync::Arc};

use crate::{Error, Timezone};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateOrder {
    Ymd,
    Ydm,
    Myd,
    Mdy,
    Dym,
    Dmy,
}

impl DateOrder {
    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_ascii_uppercase().as_str() {
            "YMD" => Some(Self::Ymd),
            "YDM" => Some(Self::Ydm),
            "MYD" => Some(Self::Myd),
            "MDY" => Some(Self::Mdy),
            "DYM" => Some(Self::Dym),
            "DMY" => Some(Self::Dmy),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ymd => "YMD",
            Self::Ydm => "YDM",
            Self::Myd => "MYD",
            Self::Mdy => "MDY",
            Self::Dym => "DYM",
            Self::Dmy => "DMY",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PreferredDateSource {
    #[default]
    CurrentPeriod,
    Past,
    Future,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PreferredDayOfMonth {
    #[default]
    Current,
    First,
    Last,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PreferredMonthOfYear {
    #[default]
    CurrentMonth,
    FirstMonth,
    LastMonth,
}

type OrderCallback = dyn Fn(&str) -> Option<DateOrder> + Send + Sync;

#[derive(Clone)]
pub struct DateOrderResolver(Arc<OrderCallback>);

impl DateOrderResolver {
    pub fn new(callback: impl Fn(&str) -> Option<DateOrder> + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }
    pub fn resolve(&self, locale: &str) -> Option<DateOrder> {
        self.0(locale)
    }
}

impl fmt::Debug for DateOrderResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DateOrderResolver(..)")
    }
}

#[derive(Clone, Debug, Default)]
pub struct Configuration {
    pub locales: Vec<String>,
    pub languages: Vec<String>,
    pub region: String,
    pub try_previous_locales: bool,
    pub use_given_order: bool,
    pub default_languages: Vec<String>,
    pub date_order: Option<DateOrder>,
    pub date_order_for_locale: Option<DateOrderResolver>,
    pub current_time: Option<DateTime<Timezone>>,
    pub default_timezone: Option<Timezone>,
    pub preferred_day_of_month: PreferredDayOfMonth,
    pub preferred_month_of_year: PreferredMonthOfYear,
    pub preferred_date_source: PreferredDateSource,
    pub strict_parsing: bool,
    pub ignore_surrounding_text: bool,
    pub required_parts: Vec<String>,
    pub skip_tokens: Vec<String>,
    pub return_time_as_period: bool,
    pub search_strategy: String,
    pub return_time_span: bool,
    pub default_start_of_week: String,
    pub default_days_in_month: i32,
    pub preserve_end_of_month: bool,
}

impl Configuration {
    pub fn validate(&self) -> Result<(), Error> {
        if !matches!(self.search_strategy.as_str(), "" | "split" | "ngram") {
            return Err(Error::InvalidConfiguration(format!(
                "invalid search strategy: {}",
                self.search_strategy
            )));
        }
        if !matches!(
            self.default_start_of_week.as_str(),
            "" | "monday" | "sunday"
        ) {
            return Err(Error::InvalidConfiguration(format!(
                "invalid default start of week: {}",
                self.default_start_of_week
            )));
        }
        for part in &self.required_parts {
            if !matches!(part.to_lowercase().as_str(), "day" | "month" | "year") {
                return Err(Error::InvalidConfiguration(format!(
                    "invalid component in required parts: {part}"
                )));
            }
        }
        Ok(())
    }

    pub fn initialized(&self) -> Result<Self, Error> {
        let mut configuration = self.clone();
        let zero = Utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0).unwrap();
        if configuration
            .current_time
            .as_ref()
            .is_none_or(|time| *time == zero)
        {
            configuration.current_time = Some(Utc::now().with_timezone(&Timezone::Utc));
        }
        if configuration.skip_tokens.is_empty() {
            configuration.skip_tokens.push("t".into());
        }
        if configuration.default_start_of_week.is_empty() {
            configuration.default_start_of_week = "monday".into();
        }
        if configuration.default_days_in_month == 0 {
            configuration.default_days_in_month = 30;
        }
        configuration.validate()?;
        Ok(configuration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_match_go_without_mutating_input() {
        let original = Configuration::default();
        let before = Utc::now();
        let configuration = original.initialized().unwrap();
        let after = Utc::now();
        let current_time = configuration.current_time.unwrap();
        assert!(current_time >= before && current_time <= after);
        assert_eq!(current_time.timezone(), Timezone::Utc);
        assert_eq!(configuration.skip_tokens, ["t"]);
        assert_eq!(configuration.default_start_of_week, "monday");
        assert_eq!(configuration.default_days_in_month, 30);
        assert!(original.current_time.is_none());
        assert!(original.skip_tokens.is_empty());
    }

    #[test]
    fn settings_validation_matches_go_strings() {
        for (configuration, message) in [
            (
                Configuration {
                    search_strategy: "Split".into(),
                    ..Configuration::default()
                },
                "invalid search strategy: Split",
            ),
            (
                Configuration {
                    default_start_of_week: "Monday".into(),
                    ..Configuration::default()
                },
                "invalid default start of week: Monday",
            ),
            (
                Configuration {
                    required_parts: vec!["hour".into()],
                    ..Configuration::default()
                },
                "invalid component in required parts: hour",
            ),
        ] {
            assert_eq!(
                configuration.validate(),
                Err(Error::InvalidConfiguration(message.into()))
            );
        }
        Configuration {
            required_parts: vec!["DAY".into(), "Month".into(), "year".into()],
            default_days_in_month: -1,
            ..Configuration::default()
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn settings_preserve_reference_zone_and_owned_lists() {
        let timezone: Timezone = "America/New_York".parse().unwrap();
        let current_time = timezone.with_ymd_and_hms(2024, 3, 9, 12, 30, 45).unwrap();
        let original = Configuration {
            languages: vec!["en".into()],
            current_time: Some(current_time.clone()),
            skip_tokens: vec!["at".into()],
            ..Configuration::default()
        };
        let mut initialized = original.initialized().unwrap();
        assert_eq!(initialized.current_time, Some(current_time));
        assert_eq!(initialized.current_time.unwrap().timezone(), timezone);
        initialized.languages.push("fr".into());
        assert_eq!(original.languages, ["en"]);
        assert_eq!(initialized.skip_tokens, ["at"]);
    }
}
