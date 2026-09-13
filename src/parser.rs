use std::{
    collections::HashSet,
    sync::{Arc, Mutex, OnceLock},
};

use chrono::{Datelike, Local, Offset};

use crate::{absolute, calendar, formatted, language, locale, nospace, relative, text, timezone};
use crate::{Configuration, Date, DateOrder, Error, Timezone};

pub type DetectLanguagesFunction = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParserType {
    Timestamp,
    NegativeTimestamp,
    RelativeTime,
    CustomFormat,
    AbsoluteTime,
    NoSpacesTime,
}

const DEFAULT_PARSERS: &[ParserType] = &[
    ParserType::Timestamp,
    ParserType::NegativeTimestamp,
    ParserType::RelativeTime,
    ParserType::CustomFormat,
    ParserType::AbsoluteTime,
    ParserType::NoSpacesTime,
];

#[derive(Default)]
pub struct Parser {
    pub parser_types: Vec<ParserType>,
    pub detect_languages_function: Option<DetectLanguagesFunction>,
    used_locales: Mutex<Vec<&'static locale::Locale>>,
}

fn default_parser() -> &'static Parser {
    static PARSER: OnceLock<Parser> = OnceLock::new();
    PARSER.get_or_init(Parser::default)
}

pub fn parse(configuration: &Configuration, input: &str) -> Result<Date, Error> {
    default_parser().parse(configuration, input, &[])
}

pub fn parse_with_formats(
    configuration: &Configuration,
    input: &str,
    formats: &[&str],
) -> Result<Date, Error> {
    default_parser().parse(configuration, input, formats)
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse(
        &self,
        configuration: &Configuration,
        input: &str,
        formats: &[&str],
    ) -> Result<Date, Error> {
        self.parse_with_wall_year(configuration, input, formats, Local::now().year())
    }

    pub(crate) fn parse_with_wall_year(
        &self,
        configuration: &Configuration,
        input: &str,
        formats: &[&str],
        wall_year: i32,
    ) -> Result<Date, Error> {
        let mut previous = self
            .used_locales
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let configuration = configuration.initialized()?;
        if let Some(date) =
            formatted::parse(&configuration, input, formats).filter(|date| !date.is_zero())
        {
            return Ok(date);
        }
        let sanitized = text::sanitize_date(input);
        if let Some(date) = self.parse_using_locales(
            &configuration,
            &sanitized,
            formats,
            false,
            wall_year,
            &mut previous,
        )? {
            return Ok(date);
        }
        if configuration.ignore_surrounding_text {
            if let Some(date) = self.parse_using_locales(
                &configuration,
                &sanitized,
                formats,
                true,
                wall_year,
                &mut previous,
            )? {
                return Ok(date);
            }
        }
        Err(Error::UnknownFormat(input.into()))
    }

    fn applicable_locales<'configuration>(
        &self,
        configuration: &'configuration Configuration,
        input: &str,
        ignore_surrounding: bool,
        previous: &[&'static locale::Locale],
    ) -> Result<impl Iterator<Item = &'static locale::Locale> + 'configuration, Error> {
        let input = text::normalize(input);
        let popped = timezone::pop_offset(&input).0;
        let mut inputs = vec![text::normalize_digits(&text::normalize(&input))];
        if popped != input {
            inputs.push(text::normalize_digits(&text::normalize(&popped)));
        }
        let applicable = move |locale, inputs: &[String]| {
            inputs.iter().any(|input| {
                language::applicable_prepared(configuration, locale, input, ignore_surrounding)
            })
        };
        let previous = if configuration.try_previous_locales {
            previous
                .iter()
                .copied()
                .find(|locale| applicable(locale, &inputs))
        } else {
            None
        };
        let mut languages = configuration.languages.clone();
        if configuration.locales.is_empty() && languages.is_empty() {
            if let Some(detector) = &self.detect_languages_function {
                languages.extend(detector(&input));
            }
        }
        let locales = locale::load(
            &configuration.locales,
            &languages,
            &configuration.region,
            configuration.use_given_order,
        )?;
        let defaults = if configuration.default_languages.is_empty() {
            Vec::new()
        } else {
            locale::load(
                &[],
                &configuration.default_languages,
                &configuration.region,
                configuration.use_given_order,
            )
            .unwrap_or_default()
        };
        let candidates = previous
            .into_iter()
            .map(|locale| (locale, false))
            .chain(locales.into_iter().map(|locale| (locale, true)))
            .chain(defaults.into_iter().map(|locale| (locale, false)));
        let mut seen = HashSet::new();
        Ok(candidates.filter_map(move |(locale, requires_check)| {
            if seen.contains(locale.name.as_str())
                || (requires_check && !applicable(locale, &inputs))
            {
                return None;
            }
            seen.insert(locale.name.as_str());
            Some(locale)
        }))
    }

    fn parse_using_locales(
        &self,
        configuration: &Configuration,
        input: &str,
        formats: &[&str],
        ignore_surrounding: bool,
        wall_year: i32,
        previous: &mut Vec<&'static locale::Locale>,
    ) -> Result<Option<Date>, Error> {
        let parsers = if self.parser_types.is_empty() {
            DEFAULT_PARSERS
        } else {
            &self.parser_types
        };
        for locale in self.applicable_locales(configuration, input, ignore_surrounding, previous)? {
            let explicit =
                configuration.date_order_for_locale.is_some() || configuration.date_order.is_some();
            let override_order = if let Some(resolver) = &configuration.date_order_for_locale {
                resolver.resolve(&locale.name)
            } else {
                configuration.date_order
            };
            let order = override_order
                .map(DateOrder::as_str)
                .unwrap_or(&locale.date_order);
            let mut locale_configuration = configuration.clone();
            locale_configuration.date_order = DateOrder::from_code(order);
            let mut orders = vec![order];
            if !explicit
                && configuration
                    .required_parts
                    .iter()
                    .any(|part| part == "year")
                && !configuration
                    .required_parts
                    .iter()
                    .any(|part| part == "day")
            {
                for fallback in ["MYD", "YMD"] {
                    if !orders.contains(&fallback) {
                        orders.push(fallback);
                    }
                }
            }
            let translations = language::translate(
                &locale_configuration,
                locale,
                input,
                false,
                ignore_surrounding,
            );
            let formatted_translations = language::translate(
                &locale_configuration,
                locale,
                input,
                true,
                ignore_surrounding,
            );
            for &parser in parsers {
                let date = match parser {
                    ParserType::Timestamp => {
                        crate::parse_timestamp(&locale_configuration, input, false)
                    }
                    ParserType::NegativeTimestamp => {
                        crate::parse_timestamp(&locale_configuration, input, true)
                    }
                    ParserType::RelativeTime => translations.iter().find_map(|translation| {
                        relative::parse(&locale_configuration, translation)
                    }),
                    ParserType::CustomFormat => {
                        formatted_translations.iter().find_map(|translation| {
                            formatted::parse(&locale_configuration, translation, formats)
                        })
                    }
                    ParserType::AbsoluteTime | ParserType::NoSpacesTime => {
                        let mut parsed = None;
                        for order in &orders {
                            locale_configuration.date_order = DateOrder::from_code(order);
                            parsed = translations.iter().find_map(|translation| {
                                let (input, timezone) =
                                    timezone::pop_offset(&text::strip_braces(translation));
                                if input.is_empty() {
                                    return None;
                                }
                                let date = if parser == ParserType::AbsoluteTime {
                                    let offset = timezone.as_ref().map(|zone| {
                                        configuration
                                            .current_time
                                            .as_ref()
                                            .unwrap()
                                            .with_timezone(zone)
                                            .offset()
                                            .fix()
                                            .local_minus_utc()
                                    });
                                    absolute::parse_with_order(
                                        &locale_configuration,
                                        &input,
                                        offset,
                                        order,
                                        explicit,
                                    )
                                } else if !order.is_empty()
                                    && locale_configuration.date_order.is_none()
                                {
                                    None
                                } else {
                                    nospace::parse(&locale_configuration, &input, wall_year)
                                }?;
                                apply_timezone(&locale_configuration, date, timezone)
                            });
                            if parsed.is_some() {
                                break;
                            }
                        }
                        locale_configuration.date_order = DateOrder::from_code(order);
                        parsed
                    }
                };
                if let Some(mut date) = date.filter(|date| !date.is_zero()) {
                    date.locale.clone_from(&locale.name);
                    if configuration.try_previous_locales
                        && !previous.iter().any(|used| used.name == locale.name)
                    {
                        previous.push(locale);
                    }
                    return Ok(Some(date));
                }
            }
        }
        Ok(None)
    }
}

fn apply_timezone(
    configuration: &Configuration,
    mut date: Date,
    timezone: Option<Timezone>,
) -> Option<Date> {
    if date.is_zero() {
        return None;
    }
    let timezone = if let Some(timezone) = timezone {
        timezone
    } else if let Some(default) = &configuration.default_timezone {
        let localized = date.time.with_timezone(default);
        Timezone::fixed(
            localized.offset().to_string(),
            localized.offset().fix().local_minus_utc(),
        )
        .ok()?
    } else {
        return Some(date);
    };
    date.time = calendar::date_time(
        date.time.year(),
        date.time.month() as i32,
        date.time.day() as i32,
        date.time.time(),
        &timezone,
    )?;
    Some(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DateOrderResolver, PreferredDateSource};
    use chrono::TimeZone;

    #[test]
    fn locale_preparation_keeps_detector_input() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let callback_inputs = observed.clone();
        let mut parser = Parser::new();
        parser.detect_languages_function = Some(Arc::new(move |input| {
            callback_inputs.lock().unwrap().push(input.to_string());
            vec!["en".into()]
        }));
        let input = "\u{662}\u{660}\u{661}\u{662}-\u{661}\u{662}-\u{661}\u{664}";
        let date = parser.parse(&Configuration::default(), input, &[]).unwrap();
        assert_eq!(date.time.format("%F").to_string(), "2012-12-14");
        assert_eq!(*observed.lock().unwrap(), [input]);
    }

    #[test]
    fn previous_locale_does_not_bypass_invalid_configuration() {
        let parser = Parser::new();
        let mut configuration = Configuration {
            try_previous_locales: true,
            ..Configuration::default()
        };
        assert_eq!(
            parser
                .parse(&configuration, "12 August 2021", &[])
                .unwrap()
                .locale,
            "en"
        );
        configuration.locales = vec!["unknown".into()];
        assert!(matches!(
            parser.parse(&configuration, "12 August 2021", &[]),
            Err(Error::UnknownLocales(_))
        ));
        configuration.locales.clear();
        configuration.languages = vec!["unknown".into()];
        assert!(matches!(
            parser.parse(&configuration, "12 August 2021", &[]),
            Err(Error::UnknownLanguages(_))
        ));
        configuration.locales = vec!["en".into(), "en-GB".into()];
        assert!(matches!(
            parser.parse(&configuration, "12 August 2021", &[]),
            Err(Error::ConflictingLocales)
        ));
    }

    #[test]
    fn shared_parser_keeps_caller_configurations_independent() {
        let parser = Parser::new();
        std::thread::scope(|scope| {
            for (locale, expected) in [
                ("en", "2014-02-03"),
                ("fr", "2014-03-02"),
                ("en-GB", "2014-03-02"),
            ] {
                let parser = &parser;
                scope.spawn(move || {
                    let configuration = Configuration {
                        locales: vec![locale.into()],
                        ..Configuration::default()
                    };
                    for _ in 0..8 {
                        let date = parser.parse(&configuration, "02/03/2014", &[]).unwrap();
                        assert_eq!(date.time.format("%F").to_string(), expected);
                        assert_eq!(date.locale, locale);
                    }
                    assert_eq!(configuration.locales, [locale]);
                    assert!(configuration.current_time.is_none());
                    assert!(configuration.skip_tokens.is_empty());
                });
            }
        });
    }

    #[test]
    fn public_parser_uses_locales_and_preserves_explicit_order() {
        let mut configuration = Configuration {
            current_time: Some(
                Timezone::Utc
                    .with_ymd_and_hms(2026, 9, 12, 12, 0, 0)
                    .unwrap(),
            ),
            ..Configuration::default()
        };
        for (input, expected) in [
            ("12 August 2021", "2021-08-12"),
            ("20 F\u{e9}vrier 2012", "2012-02-20"),
            ("2 days ago", "2026-09-10"),
            ("2024\u{5e74}2\u{6708}29\u{65e5}", "2024-02-29"),
        ] {
            assert_eq!(
                parse(&configuration, input)
                    .unwrap()
                    .time
                    .format("%F")
                    .to_string(),
                expected,
                "{input}"
            );
        }
        configuration.languages = vec!["fr".into()];
        assert_eq!(
            parse(&configuration, "2018-04-12")
                .unwrap()
                .time
                .format("%F")
                .to_string(),
            "2018-04-12"
        );
        configuration.date_order = Some(DateOrder::Dmy);
        assert_eq!(
            parse(&configuration, "2018-04-12")
                .unwrap()
                .time
                .format("%F")
                .to_string(),
            "2018-12-04"
        );
        configuration.date_order_for_locale =
            Some(DateOrderResolver::new(|_| Some(DateOrder::Ymd)));
        assert_eq!(
            parse(&configuration, "2018-04-12")
                .unwrap()
                .time
                .format("%F")
                .to_string(),
            "2018-04-12"
        );
    }

    #[test]
    fn public_parser_supports_html_date_configuration_and_callbacks() {
        let configuration = Configuration {
            current_time: Some(
                Timezone::Utc
                    .with_ymd_and_hms(2026, 9, 12, 12, 0, 0)
                    .unwrap(),
            ),
            preferred_date_source: PreferredDateSource::Past,
            strict_parsing: true,
            ..Configuration::default()
        };
        let mut parser = Parser::new();
        parser.parser_types = vec![ParserType::CustomFormat, ParserType::AbsoluteTime];
        assert!(parser.parse(&configuration, "12 August 2021", &[]).is_ok());
        assert!(parser.parse(&configuration, "August 2021", &[]).is_err());
        assert!(parser.parse(&configuration, "2 days ago", &[]).is_err());
        parser.detect_languages_function = Some(Arc::new(|_| vec!["fr".into()]));
        assert_eq!(
            parser
                .parse(&configuration, "20 fevrier 2012", &[])
                .unwrap()
                .locale,
            "fr"
        );
        let invalid = Configuration {
            languages: vec!["unknown".into()],
            ..configuration
        };
        assert_eq!(
            parser
                .parse(&invalid, "2024-02-29", &["2006-01-02"])
                .unwrap()
                .locale,
            ""
        );
        assert!(matches!(
            parser.parse(&invalid, "2024-02-29", &[]),
            Err(Error::UnknownLanguages(_))
        ));
        fn assert_send_sync<Value: Send + Sync>() {}
        assert_send_sync::<Parser>();
        assert_send_sync::<Configuration>();
    }
}
