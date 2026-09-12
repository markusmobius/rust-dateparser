use chrono::{DateTime, Offset};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    Configuration, Date, DateOrder, PreferredDateSource, PreferredDayOfMonth, PreferredMonthOfYear,
    Timezone,
};

#[derive(Debug, Deserialize)]
struct Reference {
    module: String,
    version: String,
    commit: String,
    module_sum: String,
    go_version: String,
    text_version: String,
    timezone_source_sha256: String,
    locale_data_sha256: String,
    license_sha256: String,
    python_license_sha256: String,
    project_license_sha256: String,
    wall_year: i32,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    reference: Reference,
    cases: Vec<Case>,
    language_cases: Vec<LanguageCase>,
}

#[derive(Debug, Deserialize)]
struct LanguageCase {
    locale: String,
    input: String,
    keep_formatting: bool,
    ignore_surrounding: bool,
    skip_tokens: Vec<String>,
    normalized: String,
    simplified: String,
    tokens: Vec<String>,
    applicable: bool,
    translations: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    stage: String,
    input: String,
    configuration: Settings,
    #[serde(default)]
    negative: bool,
    #[serde(default)]
    formats: Vec<String>,
    #[serde(default)]
    parser_types: Vec<u8>,
    detector_languages: Option<Vec<String>>,
    #[serde(default)]
    detection_inputs: Vec<String>,
    #[serde(default)]
    session: String,
    expected: Expected,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Settings {
    locales: Vec<String>,
    languages: Vec<String>,
    region: String,
    use_given_order: bool,
    try_previous_locales: bool,
    default_languages: Vec<String>,
    skip_tokens: Vec<String>,
    ignore_surrounding_text: bool,
    date_order_for_locales: std::collections::HashMap<String, String>,
    current_time: String,
    current_timezone: String,
    default_timezone: String,
    date_order: String,
    date_order_is_explicit: bool,
    preferred_day_of_month: u8,
    preferred_month_of_year: u8,
    preferred_date_source: u8,
    strict_parsing: bool,
    required_parts: Vec<String>,
    return_time_as_period: bool,
    preserve_end_of_month: bool,
}

impl Settings {
    fn configuration(&self) -> Configuration {
        let current = DateTime::parse_from_rfc3339(&self.current_time).unwrap();
        let zone = if self.current_timezone.is_empty() {
            if current.offset().local_minus_utc() == 0 {
                Timezone::Utc
            } else {
                Timezone::fixed("", current.offset().local_minus_utc()).unwrap()
            }
        } else {
            self.current_timezone.parse().unwrap()
        };
        let date_order = if !self.date_order_is_explicit {
            None
        } else {
            Some(match self.date_order.as_str() {
                "YMD" => DateOrder::Ymd,
                "YDM" => DateOrder::Ydm,
                "MDY" => DateOrder::Mdy,
                "MYD" => DateOrder::Myd,
                "DMY" => DateOrder::Dmy,
                "DYM" => DateOrder::Dym,
                order => panic!("unexpected reference date order {order}"),
            })
        };
        Configuration {
            locales: self.locales.clone(),
            languages: self.languages.clone(),
            region: self.region.clone(),
            use_given_order: self.use_given_order,
            try_previous_locales: self.try_previous_locales,
            default_languages: self.default_languages.clone(),
            skip_tokens: self.skip_tokens.clone(),
            ignore_surrounding_text: self.ignore_surrounding_text,
            date_order_for_locale: if self.date_order_for_locales.is_empty() {
                None
            } else {
                let orders = self.date_order_for_locales.clone();
                Some(crate::DateOrderResolver::new(move |locale| {
                    orders
                        .get(locale)
                        .and_then(|order| DateOrder::from_code(order))
                }))
            },
            current_time: Some(current.with_timezone(&zone)),
            default_timezone: if self.default_timezone.is_empty() {
                None
            } else {
                Some(self.default_timezone.parse().unwrap())
            },
            date_order,
            preferred_day_of_month: match self.preferred_day_of_month {
                0 => PreferredDayOfMonth::Current,
                1 => PreferredDayOfMonth::First,
                2 => PreferredDayOfMonth::Last,
                value => panic!("invalid reference day preference {value}"),
            },
            preferred_month_of_year: match self.preferred_month_of_year {
                0 => PreferredMonthOfYear::CurrentMonth,
                1 => PreferredMonthOfYear::FirstMonth,
                2 => PreferredMonthOfYear::LastMonth,
                value => panic!("invalid reference month preference {value}"),
            },
            preferred_date_source: match self.preferred_date_source {
                0 => PreferredDateSource::CurrentPeriod,
                1 => PreferredDateSource::Past,
                2 => PreferredDateSource::Future,
                value => panic!("invalid reference source preference {value}"),
            },
            strict_parsing: self.strict_parsing,
            required_parts: self.required_parts.clone(),
            return_time_as_period: self.return_time_as_period,
            preserve_end_of_month: self.preserve_end_of_month,
            ..Configuration::default()
        }
    }
}

#[derive(Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
struct Expected {
    locale: String,
    error: String,
    parsed: bool,
    unix_seconds: i64,
    nanosecond: u32,
    offset: i32,
    period: String,
    timezone: String,
}

fn result(date: Option<Date>, timestamp: bool) -> Expected {
    let Some(date) = date else {
        return Expected::default();
    };
    let time = if timestamp {
        date.time.with_timezone(&Timezone::Utc)
    } else {
        date.time
    };
    let timezone = match time.timezone() {
        Timezone::Utc => "UTC".into(),
        Timezone::Local => "Local".into(),
        Timezone::Fixed { name, .. } => name.to_string(),
        Timezone::Iana(timezone) => timezone.to_string(),
    };
    Expected {
        locale: date.locale,
        error: String::new(),
        parsed: true,
        unix_seconds: time.timestamp(),
        nanosecond: time.timestamp_subsec_nanos(),
        offset: time.offset().fix().local_minus_utc(),
        period: format!("{:?}", date.period),
        timezone,
    }
}

fn verify(bytes: &[u8]) {
    let fixture: Fixture = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        fixture.reference.module,
        "github.com/markusmobius/go-dateparser"
    );
    assert_eq!(fixture.reference.version, "v1.4.3");
    assert_eq!(
        fixture.reference.commit,
        "ce55302a57663c33e2d7bb668a29686b4c036fde"
    );
    assert_eq!(fixture.reference.go_version, "go1.27.1");
    assert_eq!(fixture.reference.text_version, "v0.42.0");
    assert_eq!(
        fixture.reference.timezone_source_sha256,
        crate::timezone_data::SOURCE_SHA256
    );
    assert_eq!(
        fixture.reference.locale_data_sha256,
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../data/locales.json"))
        )
    );
    assert_eq!(
        fixture.reference.license_sha256,
        "6c6a5f897960bf47bdc044cecf8d1b3fb484348020fa8a0320f3bf73d6d3cf3b"
    );
    assert_eq!(
        fixture.reference.python_license_sha256,
        "8639063668407f5e478ae2d3c3c17966088558b4b5574b548c436eea302bbe7e"
    );
    assert_eq!(
        fixture.reference.project_license_sha256,
        format!("{:x}", Sha256::digest(include_bytes!("../LICENSE")))
    );
    assert_eq!(
        fixture.reference.module_sum,
        "h1:FWb52fQDRTdHcRfU8R2hkTW+U4HA4m39ldonanH5x0E="
    );
    assert_eq!(
        fixture
            .cases
            .iter()
            .filter(|case| case.stage != "public")
            .count(),
        1335
    );
    assert!(
        fixture
            .cases
            .iter()
            .filter(|case| case.stage == "public")
            .count()
            > 2000
    );
    let mut differences = Vec::new();
    let mut sessions: std::collections::HashMap<String, crate::Parser> =
        std::collections::HashMap::new();
    for case in &fixture.cases {
        let configuration = case.configuration.configuration();
        let actual = match case.stage.as_str() {
            "public" => {
                let mut fresh = crate::Parser::new();
                let parser = if case.session.is_empty() {
                    &mut fresh
                } else {
                    sessions.entry(case.session.clone()).or_default()
                };
                parser.parser_types = case
                    .parser_types
                    .iter()
                    .map(|kind| match kind {
                        0 => crate::ParserType::Timestamp,
                        1 => crate::ParserType::NegativeTimestamp,
                        2 => crate::ParserType::RelativeTime,
                        3 => crate::ParserType::CustomFormat,
                        4 => crate::ParserType::AbsoluteTime,
                        5 => crate::ParserType::NoSpacesTime,
                        value => panic!("unknown reference parser type {value}"),
                    })
                    .collect();
                let inputs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                parser.detect_languages_function =
                    case.detector_languages.as_ref().map(|languages| {
                        let languages = languages.clone();
                        let inputs = inputs.clone();
                        std::sync::Arc::new(move |input: &str| {
                            inputs.lock().unwrap().push(input.to_string());
                            languages.clone()
                        }) as crate::DetectLanguagesFunction
                    });
                let parsed = parser.parse_with_wall_year(
                    &configuration,
                    &case.input,
                    &case.formats.iter().map(String::as_str).collect::<Vec<_>>(),
                    fixture.reference.wall_year,
                );
                assert_eq!(
                    *inputs.lock().unwrap(),
                    case.detection_inputs,
                    "{} detector inputs",
                    case.id
                );
                match parsed {
                    Ok(date) => {
                        let local = matches!(date.time.timezone(), Timezone::Local);
                        result(Some(date), local)
                    }
                    Err(error) => Expected {
                        error: error.to_string(),
                        ..Expected::default()
                    },
                }
            }
            "timestamp" => result(
                crate::parse_timestamp(&configuration, &case.input, case.negative),
                true,
            ),
            "absolute" => result(
                crate::parse_absolute(&configuration, &case.input).ok(),
                false,
            ),
            "formatted" => result(
                crate::parse_formatted(
                    &configuration,
                    &case.input,
                    &case.formats.iter().map(String::as_str).collect::<Vec<_>>(),
                )
                .ok(),
                false,
            ),
            "nospace" => result(
                crate::nospace::parse(&configuration, &case.input, fixture.reference.wall_year),
                false,
            ),
            "relative" => result(
                crate::parse_relative(&configuration, &case.input).ok(),
                false,
            ),
            stage => panic!("unknown reference stage {stage}"),
        };
        if actual != case.expected {
            differences.push(format!(
                "{} {:?} {:?}\nexpected: {:?}\nactual:   {:?}",
                case.id, case.input, case.configuration, case.expected, actual
            ));
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} Go cases differ:\n{}",
        differences.len(),
        fixture.cases.len(),
        differences
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    println!("Exact Go parsing parity: {} cases", fixture.cases.len());
}

#[test]
fn go_language_fixture() {
    verify_languages(include_bytes!("../testdata/go-core.json"));
}

fn verify_languages(bytes: &[u8]) {
    let fixture: Fixture = serde_json::from_slice(bytes).unwrap();
    assert!(fixture.language_cases.len() > 2000);
    let locales: std::collections::HashSet<_> = fixture
        .language_cases
        .iter()
        .map(|case| case.locale.as_str())
        .collect();
    assert_eq!(locales.len(), 512);
    let mut differences = Vec::new();
    for case in &fixture.language_cases {
        let locale = crate::locale::data().get(&case.locale).unwrap();
        let configuration = Configuration {
            skip_tokens: case.skip_tokens.clone(),
            ..Configuration::default()
        };
        let skipped = configuration
            .skip_tokens
            .iter()
            .map(String::as_str)
            .filter(|token| case.locale != "fi" || *token != "t")
            .collect();
        let normalized = crate::text::normalize(&case.input);
        let simplified = crate::language::simplify(
            locale,
            &crate::text::normalize_digits(
                &crate::text::normalize_unicode(&case.input)
                    .chars()
                    .flat_map(char::to_lowercase)
                    .collect::<String>(),
            ),
        );
        let tokens = crate::language::split(locale, &simplified, case.keep_formatting, &skipped);
        let applicable = crate::language::applicable(
            &configuration,
            locale,
            &case.input,
            case.ignore_surrounding,
        );
        let translations = crate::language::translate(
            &configuration,
            locale,
            &case.input,
            case.keep_formatting,
            case.ignore_surrounding,
        );
        if normalized != case.normalized
            || simplified != case.simplified
            || tokens != case.tokens
            || applicable != case.applicable
            || translations != case.translations
        {
            differences.push(format!("{case:?}\nnormalized: {normalized:?}\nsimplified: {simplified:?}\ntokens: {tokens:?}\napplicable: {applicable}\ntranslations: {translations:?}"));
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} language cases differ:\n{}",
        differences.len(),
        fixture.language_cases.len(),
        differences
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    println!(
        "Exact Go language parity: {} cases across {} locales",
        fixture.language_cases.len(),
        locales.len()
    );
}

#[test]
fn go_core_fixture() {
    verify(include_bytes!("../testdata/go-core.json"));
}

#[test]
#[ignore = "generate a fresh snapshot with tools/go-reference first"]
fn live_go_parity() {
    let path = std::env::var_os("GO_DATEPARSER_REFERENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "target/go-reference.json".into());
    let bytes = std::fs::read(path).expect("fresh Go reference file is missing");
    verify(&bytes);
    verify_languages(&bytes);
}

#[test]
#[ignore = "run tools/benchmark.py for isolated single-thread measurements"]
fn benchmark_public_parse() {
    use chrono::Datelike;
    use std::{hint::black_box, time::Instant};

    let path = std::env::var("DATEPARSER_BENCHMARK_SUITE")
        .unwrap_or_else(|_| "testdata/go-core.json".into());
    let bytes = std::fs::read(path).unwrap();
    let fixture: Fixture = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fixture.reference.version, "v1.4.3");
    assert_eq!(
        fixture.reference.wall_year,
        chrono::Local::now().year(),
        "regenerate the Go fixture for the current wall-clock year"
    );
    let cohort = std::env::var("DATEPARSER_BENCHMARK_COHORT").unwrap_or_else(|_| "auto".into());
    assert!(["auto", "explicit", "htmldate"].contains(&cohort.as_str()));
    let passes: usize = std::env::var("DATEPARSER_BENCHMARK_PASSES")
        .unwrap_or_else(|_| "8".into())
        .parse()
        .unwrap();
    assert!(passes > 0);
    let prepared: Vec<_> = fixture
        .cases
        .iter()
        .filter(|case| {
            if case.stage != "public"
                || !case.session.is_empty()
                || case.detector_languages.is_some()
            {
                return false;
            }
            let html_date = case.parser_types == [3, 4];
            let explicit =
                !case.configuration.locales.is_empty() || !case.configuration.languages.is_empty();
            match cohort.as_str() {
                "htmldate" => html_date,
                "explicit" => !html_date && explicit,
                _ => !html_date && !explicit,
            }
        })
        .map(|case| {
            let mut parser = crate::Parser::new();
            parser.parser_types = case
                .parser_types
                .iter()
                .map(|kind| match kind {
                    0 => crate::ParserType::Timestamp,
                    1 => crate::ParserType::NegativeTimestamp,
                    2 => crate::ParserType::RelativeTime,
                    3 => crate::ParserType::CustomFormat,
                    4 => crate::ParserType::AbsoluteTime,
                    5 => crate::ParserType::NoSpacesTime,
                    value => panic!("unknown reference parser type {value}"),
                })
                .collect();
            (
                case,
                case.configuration.configuration(),
                parser,
                case.formats.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        })
        .collect();
    assert!(!prepared.is_empty());
    let expected_parsed = prepared
        .iter()
        .filter(|entry| entry.0.expected.parsed)
        .count();
    let first = Instant::now();
    for (case, configuration, parser, formats) in &prepared {
        let actual = match parser.parse(configuration, &case.input, formats) {
            Ok(date) => {
                let local = matches!(date.time.timezone(), Timezone::Local);
                result(Some(date), local)
            }
            Err(error) => Expected {
                error: error.to_string(),
                ..Expected::default()
            },
        };
        assert_eq!(actual, case.expected, "{}: {:?}", case.id, case.input);
    }
    let first_pass_ms = first.elapsed().as_secs_f64() * 1000.0;
    let metadata = serde_json::json!({
        "cohort": cohort,
        "cases": prepared.len(),
        "parsed": expected_parsed,
        "fixture_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "first_pass_ms": first_pass_ms,
    });
    println!("BENCHMARK_READY {metadata}");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    let mut pass_ms = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut parsed_count = 0;
        let start = Instant::now();
        for (case, configuration, parser, formats) in &prepared {
            let parsed = parser.parse(configuration, black_box(&case.input), formats);
            if let Ok(date) = &parsed {
                parsed_count += usize::from(!date.is_zero());
            }
            let _ = black_box(parsed);
        }
        pass_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(parsed_count, expected_parsed);
    }
    println!(
        "BENCHMARK_RESULT {}",
        serde_json::json!({ "metadata": metadata, "pass_ms": pass_ms })
    );
}
