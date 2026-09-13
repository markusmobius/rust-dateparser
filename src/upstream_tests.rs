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
    search_strategy: String,
    return_time_span: bool,
    default_start_of_week: String,
    default_days_in_month: i32,
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
            search_strategy: self.search_strategy.clone(),
            return_time_span: self.return_time_span,
            default_start_of_week: self.default_start_of_week.clone(),
            default_days_in_month: self.default_days_in_month,
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

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct FeatureMatch {
    text: String,
    date: Expected,
}

#[derive(Deserialize)]
struct CorrectionReference {
    module: String,
    version: String,
    commit: String,
    module_sum: String,
    source_sha256: String,
}

#[derive(Deserialize)]
struct CorrectionPython {
    version: String,
    dateparser: String,
    dateutil: String,
    evidence: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct Corrections {
    reference: CorrectionReference,
    core_sha256: String,
    features_sha256: String,
    core: std::collections::BTreeMap<String, Expected>,
    features: std::collections::BTreeMap<String, Vec<FeatureMatch>>,
    python: CorrectionPython,
    source_files: std::collections::BTreeMap<String, String>,
}

fn dateutil_corrections() -> Corrections {
    let corrections: Corrections =
        serde_json::from_slice(include_bytes!("../testdata/dateutil-corrections.json")).unwrap();
    assert_eq!(
        corrections.reference.module,
        "github.com/markusmobius/go-dateparser"
    );
    assert_eq!(corrections.reference.version, "worktree");
    assert_eq!(
        corrections.reference.commit,
        "7837629bf3773c8d94524ec603706bec9a0c665b"
    );
    assert!(corrections.reference.module_sum.is_empty());
    assert_eq!(
        corrections.reference.source_sha256,
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&corrections.source_files).unwrap())
        )
    );
    assert_eq!(
        corrections.core_sha256,
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../testdata/go-core.json"))
        )
    );
    assert_eq!(
        corrections.features_sha256,
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../testdata/go-features.json"))
        )
    );
    assert_eq!(corrections.python.version, "3.14.6");
    assert_eq!(corrections.python.dateparser, "1.4.3");
    assert_eq!(corrections.python.dateutil, "2.9.0.post0");
    assert_eq!(corrections.core.len(), 75);
    assert_eq!(corrections.features.len(), 16);
    let identities: std::collections::BTreeSet<_> = corrections
        .core
        .keys()
        .chain(corrections.features.keys())
        .collect();
    assert_eq!(identities, corrections.python.evidence.keys().collect());
    corrections
}

fn apply_core_corrections(cases: &mut [Case], corrections: &mut Corrections) {
    for case in cases {
        if let Some(expected) = corrections.core.remove(&case.id) {
            assert_ne!(case.expected, expected, "{} is not a correction", case.id);
            case.expected = expected;
        }
    }
    assert!(
        corrections.core.is_empty(),
        "correction references an unknown core input"
    );
}

#[derive(Debug, Deserialize)]
struct FeatureCase {
    id: String,
    stage: String,
    input: String,
    #[serde(default)]
    reference_input: String,
    #[serde(default)]
    language: String,
    configuration: Settings,
    #[serde(default)]
    unspecified_current_time: bool,
    #[serde(default)]
    parser_types: Vec<u8>,
    #[serde(default)]
    error: String,
    #[serde(default)]
    detected: String,
    detector_languages: Option<Vec<String>>,
    #[serde(default)]
    detection_inputs: Vec<String>,
    #[serde(default)]
    date_order_inputs: Vec<String>,
    #[serde(default)]
    upstream_panic: String,
    #[serde(default)]
    known_difference: String,
    matches: Vec<FeatureMatch>,
    #[serde(default)]
    translations: Vec<String>,
    #[serde(default)]
    originals: Vec<String>,
}

#[derive(Deserialize)]
struct PythonReference {
    module: String,
    commit: String,
    versions: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
struct PythonFixture {
    reference: PythonReference,
    calendar_data_sha256: String,
    cases: Vec<FeatureCase>,
}

fn run_feature(
    case: &FeatureCase,
    configuration: &Configuration,
    parser: &crate::Parser,
) -> Result<(String, Vec<crate::SearchResult>), crate::Error> {
    match case.stage.as_str() {
        "jalali" | "hijri" => {
            let parse = if case.stage == "jalali" {
                crate::parse_jalali
            } else {
                crate::parse_hijri
            };
            parse(configuration, &case.input).map(|date| {
                (
                    String::new(),
                    vec![crate::SearchResult {
                        text: case.input.clone(),
                        date,
                    }],
                )
            })
        }
        "search" => parser.search(configuration, &case.input),
        "search_with_language" => parser
            .search_with_language(configuration, &case.language, &case.input)
            .map(|found| (case.language.clone(), found)),
        other => panic!("unknown feature stage {other}"),
    }
}

fn verify_python_features(bytes: &[u8], calendars: bool) {
    let fixture: PythonFixture = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        fixture.reference.module,
        "https://github.com/scrapinghub/dateparser"
    );
    assert_eq!(
        fixture.reference.commit,
        "9ce60b1958f1b285886bcfbb743f6419feacfc92"
    );
    for (package, version) in [
        ("dateparser", "1.4.3"),
        ("convertdate", "2.4.1"),
        ("hijridate", "2.6.0"),
        ("pymeeus", "0.5.12"),
    ] {
        assert_eq!(fixture.reference.versions[package], version);
    }
    assert_eq!(
        fixture.calendar_data_sha256,
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../data/calendar-conversions.json"))
        )
    );
    let mut checked = 0;
    let mut safety = 0;
    let mut failures = Vec::new();
    for mut case in fixture.cases {
        if matches!(case.stage.as_str(), "jalali" | "hijri") != calendars {
            continue;
        }
        case.configuration.date_order_is_explicit = !case.configuration.date_order.is_empty();
        let configuration = case.configuration.configuration();
        let detection_inputs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut parser = crate::Parser::new();
        if let Some(languages) = &case.detector_languages {
            let languages = languages.clone();
            let inputs = detection_inputs.clone();
            parser.detect_languages_function = Some(std::sync::Arc::new(move |input| {
                inputs.lock().unwrap().push(input.to_string());
                languages.clone()
            }));
        }
        let actual = run_feature(&case, &configuration, &parser);
        let (detected, matches, error) = match actual {
            Ok((detected, matches)) => (detected, matches, String::new()),
            Err(error) => (String::new(), Vec::new(), error.to_string()),
        };
        if !case.known_difference.is_empty() {
            assert!(!calendars);
            match case.known_difference.as_str() {
                "python-search-exception" => (),
                "python-relative-range" => {
                    assert!(matches.is_empty(), "{} must reject overflow", case.id)
                }
                other => panic!("unreviewed Python difference {other}"),
            }
            safety += 1;
            continue;
        }
        let matches: Vec<_> = matches
            .into_iter()
            .map(|matched| {
                let mut date = result(Some(matched.date), false);
                if !calendars {
                    date.locale.clear();
                    date.period.clear();
                    date.timezone.clear();
                } else if case
                    .matches
                    .first()
                    .is_some_and(|expected| expected.date.period == "Time")
                {
                    assert!(matches!(date.period.as_str(), "Hour" | "Minute" | "Second"));
                    date.period = "Time".into();
                }
                FeatureMatch {
                    text: matched.text,
                    date,
                }
            })
            .collect();
        if matches != case.matches
            || (calendars && error.is_empty() != case.error.is_empty())
            || (!calendars && !matches.is_empty() && detected != case.detected)
            || *detection_inputs.lock().unwrap() != case.detection_inputs
        {
            failures.push(format!(
                "{} {:?}: {:?} / {:?}; language {:?}/{:?}; error {:?}/{:?}; callbacks {:?}/{:?}",
                case.id,
                case.input,
                matches,
                case.matches,
                detected,
                case.detected,
                error,
                case.error,
                detection_inputs.lock().unwrap(),
                case.detection_inputs
            ));
        }
        checked += 1;
    }
    assert_eq!(checked, if calendars { 7504 } else { 178 });
    assert_eq!(safety, if calendars { 0 } else { 10 });
    assert!(
        failures.is_empty(),
        "{} of {checked} Python feature cases differed:\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("Matched {checked} Python cases; {safety} exception inputs checked for safety");
}

#[test]
fn python_calendar_fixture() {
    verify_python_features(include_bytes!("../testdata/python-features.json"), true);
}

#[test]
fn python_search_fixture() {
    verify_python_features(include_bytes!("../testdata/python-features.json"), false);
}

fn verify_features(bytes: &[u8]) {
    #[derive(Deserialize)]
    struct FeatureFixture {
        reference: Reference,
        calendar_data_sha256: String,
        cases: Vec<FeatureCase>,
    }
    let mut fixture: FeatureFixture = serde_json::from_slice(bytes).unwrap();
    let mut corrections = dateutil_corrections();
    assert_eq!(
        corrections.features_sha256,
        format!("{:x}", Sha256::digest(bytes))
    );
    for case in &mut fixture.cases {
        if let Some(matches) = corrections.features.remove(&case.id) {
            assert_ne!(case.matches, matches, "{} is not a correction", case.id);
            assert_eq!(case.matches.len(), matches.len());
            for (original, corrected) in case.matches.iter().zip(&matches) {
                assert_eq!(
                    original.text, corrected.text,
                    "{} changed a search input",
                    case.id
                );
            }
            case.matches = matches;
        }
    }
    assert!(
        corrections.features.is_empty(),
        "correction references an unknown feature input"
    );
    assert_eq!(fixture.reference.version, "v1.4.5");
    assert_eq!(
        fixture.reference.commit,
        "e02a0cfd80decdd47412d773b4799a89af078409"
    );
    assert_eq!(
        fixture.reference.module_sum,
        "h1:Y34+feJSV/d7QGMbLFlQSfdPDVdhQS5qVL8/sHJ9eKk="
    );
    assert_eq!(
        fixture.reference.project_license_sha256,
        format!("{:x}", Sha256::digest(include_bytes!("../LICENSE")))
    );
    assert_eq!(
        fixture.reference.locale_data_sha256,
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../data/locales.json"))
        )
    );
    assert_eq!(
        fixture.calendar_data_sha256,
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../data/calendars.json"))
        )
    );
    let mut checked = 0;
    let mut upstream_panics = 0;
    let mut failures = Vec::new();
    for case in fixture.cases {
        assert!(
            case.known_difference.is_empty(),
            "{} must match Go exactly",
            case.id
        );
        assert!(
            case.reference_input.is_empty(),
            "{} must use its original input",
            case.id
        );
        let mut configuration = case.configuration.configuration();
        if case.unspecified_current_time {
            configuration.current_time = None;
        }
        if case.stage == "translation" {
            let (translations, originals) = crate::language::translate_search(
                &configuration.initialized().unwrap(),
                crate::locale::data().get(&case.language).unwrap(),
                &case.input,
            );
            if case.upstream_panic.is_empty()
                && (translations != case.translations || originals != case.originals)
            {
                failures.push(format!(
                    "{} {} {:?}: translation {:?} / {:?}; originals {:?} / {:?}",
                    case.id,
                    case.language,
                    case.input,
                    translations,
                    case.translations,
                    originals,
                    case.originals
                ));
            }
        } else {
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
                    _ => panic!("unexpected parser kind"),
                })
                .collect();
            let detection_inputs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let date_order_inputs =
                std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            if matches!(case.stage.as_str(), "jalali" | "hijri")
                && (configuration.date_order.is_some()
                    || configuration.date_order_for_locale.is_some())
            {
                let resolve = configuration.date_order_for_locale.clone();
                let order = configuration.date_order;
                let inputs = date_order_inputs.clone();
                configuration.date_order_for_locale =
                    Some(crate::DateOrderResolver::new(move |locale| {
                        inputs.lock().unwrap().push(locale.into());
                        resolve
                            .as_ref()
                            .map_or(order, |resolve| resolve.resolve(locale))
                    }));
            }
            if let Some(detected) = &case.detector_languages {
                let detected = detected.clone();
                let inputs = detection_inputs.clone();
                parser.detect_languages_function = Some(std::sync::Arc::new(move |input| {
                    inputs.lock().unwrap().push(input.to_string());
                    detected.clone()
                }));
            }
            let actual = if case.stage == "search" {
                parser.search(&configuration, &case.input)
            } else if case.stage == "jalali" || case.stage == "hijri" {
                let parse = if case.stage == "jalali" {
                    crate::parse_jalali
                } else {
                    crate::parse_hijri
                };
                parse(&configuration, &case.input).map(|date| {
                    let matches = if date.is_zero() {
                        Vec::new()
                    } else {
                        vec![crate::SearchResult {
                            text: case.input.clone(),
                            date,
                        }]
                    };
                    (String::new(), matches)
                })
            } else {
                parser
                    .search_with_language(&configuration, &case.language, &case.input)
                    .map(|found| (String::new(), found))
            };
            if case.upstream_panic.is_empty() {
                let (detected, matches, error) = match actual {
                    Ok((detected, matches)) => (detected, matches, String::new()),
                    Err(error) => (String::new(), Vec::new(), error.to_string()),
                };
                if error != case.error
                    || (error.is_empty() && detected != case.detected)
                    || *detection_inputs.lock().unwrap() != case.detection_inputs
                {
                    failures.push(format!(
                        "{} {} {:?}: error {:?}/{:?}, language {:?}/{:?}, detector {:?}/{:?}",
                        case.id,
                        case.language,
                        case.input,
                        error,
                        case.error,
                        detected,
                        case.detected,
                        detection_inputs.lock().unwrap(),
                        case.detection_inputs
                    ));
                }
                if *date_order_inputs.lock().unwrap() != case.date_order_inputs {
                    failures.push(format!(
                        "{}: date-order callback {:?}/{:?}",
                        case.id,
                        date_order_inputs.lock().unwrap(),
                        case.date_order_inputs
                    ));
                }
                let matches: Vec<_> = matches
                    .into_iter()
                    .map(|matched| FeatureMatch {
                        text: matched.text,
                        date: {
                            let local = matches!(matched.date.time.timezone(), Timezone::Local);
                            result(Some(matched.date), local)
                        },
                    })
                    .collect();
                if matches != case.matches {
                    failures.push(format!(
                        "{} {} {:?}: matches {:?} / {:?}",
                        case.id, case.language, case.input, matches, case.matches
                    ));
                }
            } else if case.stage == "hijri" {
                assert_eq!(
                    actual.unwrap_err().to_string(),
                    "date is outside Umm al-Qura scope",
                    "{}",
                    case.id
                );
            }
        }
        if !case.upstream_panic.is_empty() {
            upstream_panics += 1;
        } else {
            checked += 1;
        }
    }
    assert_eq!(checked, 9_886);
    assert!(
        failures.is_empty(),
        "{} feature differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(
        upstream_panics, 0,
        "the Go v1.4.5 fixture must not mask panics"
    );
    eprintln!("Matched {checked} supplementary Go cases with Python-qualified Dateutil corrections, without exceptions or panic inputs");
}

#[test]
fn go_feature_fixture() {
    verify_features(include_bytes!("../testdata/go-features.json"));
}

fn verify(bytes: &[u8]) {
    let mut fixture: Fixture = serde_json::from_slice(bytes).unwrap();
    let mut corrections = dateutil_corrections();
    assert_eq!(
        corrections.core_sha256,
        format!("{:x}", Sha256::digest(bytes))
    );
    apply_core_corrections(&mut fixture.cases, &mut corrections);
    assert_eq!(
        fixture.reference.module,
        "github.com/markusmobius/go-dateparser"
    );
    assert_eq!(fixture.reference.version, "v1.4.5");
    assert_eq!(
        fixture.reference.commit,
        "e02a0cfd80decdd47412d773b4799a89af078409"
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
        "h1:Y34+feJSV/d7QGMbLFlQSfdPDVdhQS5qVL8/sHJ9eKk="
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
    println!(
        "Exact Go parsing parity with Python-qualified Dateutil corrections: {} cases",
        fixture.cases.len()
    );
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
        let translation_input = crate::text::normalize_digits(
            &crate::text::normalize_unicode(&case.input)
                .chars()
                .flat_map(char::to_lowercase)
                .collect::<String>(),
        );
        let simplified = crate::language::simplify(locale, &translation_input);
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
#[ignore = "generate a fresh supplementary snapshot with tools/go-reference first"]
fn live_go_feature_parity() {
    let path = std::env::var_os("GO_DATEPARSER_FEATURE_REFERENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "target/go-features-reference.json".into());
    let bytes = std::fs::read(path).expect("fresh Go feature reference file is missing");
    assert!(
        bytes == include_bytes!("../testdata/go-features.json"),
        "supplementary fixture did not reproduce byte-for-byte"
    );
    verify_features(&bytes);
}

#[test]
#[ignore = "run tools/benchmark.py for isolated single-thread measurements"]
fn benchmark_public_parse() {
    use chrono::Datelike;
    use std::{hint::black_box, time::Instant};

    let path = std::env::var("DATEPARSER_BENCHMARK_SUITE")
        .unwrap_or_else(|_| "testdata/go-core.json".into());
    let bytes = std::fs::read(path).unwrap();
    let mut fixture: Fixture = serde_json::from_slice(&bytes).unwrap();
    let historical_parsed: std::collections::HashSet<_> = fixture
        .cases
        .iter()
        .filter(|case| case.expected.parsed)
        .map(|case| case.id.clone())
        .collect();
    let mut corrections = dateutil_corrections();
    assert_eq!(
        corrections.core_sha256,
        format!("{:x}", Sha256::digest(&bytes))
    );
    let corrected_ids: std::collections::HashSet<_> = corrections.core.keys().cloned().collect();
    apply_core_corrections(&mut fixture.cases, &mut corrections);
    assert_eq!(fixture.reference.version, "v1.4.5");
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
        "historical_parsed": prepared.iter().filter(|entry| historical_parsed.contains(&entry.0.id)).count(),
        "corrected_cases": prepared.iter().filter(|entry| corrected_ids.contains(&entry.0.id)).map(|entry| &entry.0.id).collect::<Vec<_>>(),
        "corrections_sha256": format!("{:x}", Sha256::digest(include_bytes!("../testdata/dateutil-corrections.json"))),
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

#[test]
#[ignore = "run tools/benchmark.py --features for isolated single-thread measurements"]
fn benchmark_features() {
    use std::{hint::black_box, time::Instant};

    let path = std::env::var("DATEPARSER_BENCHMARK_SUITE")
        .unwrap_or_else(|_| "testdata/python-features.json".into());
    let bytes = std::fs::read(path).unwrap();
    let fixture: PythonFixture = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fixture.reference.versions["dateparser"], "1.4.3");
    let cohort =
        std::env::var("DATEPARSER_BENCHMARK_COHORT").unwrap_or_else(|_| "search-auto".into());
    assert!([
        "search-auto",
        "search-split",
        "search-ngram",
        "time-span",
        "jalali",
        "hijri"
    ]
    .contains(&cohort.as_str()));
    let number = |name: &str, default: &str| -> usize {
        std::env::var(name)
            .unwrap_or_else(|_| default.into())
            .parse()
            .unwrap()
    };
    let passes = number("DATEPARSER_BENCHMARK_PASSES", "8");
    let iterations = number("DATEPARSER_BENCHMARK_ITERATIONS", "16");
    assert!(passes > 0 && iterations > 0);
    let prepared: Vec<_> = fixture
        .cases
        .into_iter()
        .filter_map(|mut case| {
            if !case.known_difference.is_empty() || case.detector_languages.is_some() {
                return None;
            }
            let selected = if matches!(case.stage.as_str(), "jalali" | "hijri") {
                case.stage.as_str()
            } else if case.configuration.return_time_span {
                "time-span"
            } else if case.configuration.search_strategy == "ngram" {
                "search-ngram"
            } else if case.stage == "search"
                && case.configuration.languages.is_empty()
                && case.configuration.locales.is_empty()
            {
                "search-auto"
            } else {
                "search-split"
            };
            if selected != cohort {
                return None;
            }
            case.configuration.date_order_is_explicit = !case.configuration.date_order.is_empty();
            let configuration = case.configuration.configuration();
            Some((case, configuration, crate::Parser::new()))
        })
        .collect();
    assert!(!prepared.is_empty());
    let expected_parsed = prepared
        .iter()
        .filter(|entry| !entry.0.matches.is_empty())
        .count();
    let expected_matches: usize = prepared.iter().map(|entry| entry.0.matches.len()).sum();
    let calendars = matches!(cohort.as_str(), "jalali" | "hijri");
    let first = Instant::now();
    for (case, configuration, parser) in &prepared {
        let actual = run_feature(case, configuration, parser);
        if calendars {
            assert_eq!(actual.is_err(), !case.error.is_empty(), "{}", case.id);
        }
        let (detected, matches) = actual.unwrap_or_default();
        if !calendars && !matches.is_empty() {
            assert_eq!(detected, case.detected, "{}", case.id);
        }
        let matches: Vec<_> = matches
            .into_iter()
            .map(|matched| {
                let mut date = result(Some(matched.date), false);
                if !calendars {
                    date.locale.clear();
                    date.period.clear();
                    date.timezone.clear();
                } else if case
                    .matches
                    .first()
                    .is_some_and(|expected| expected.date.period == "Time")
                {
                    assert!(matches!(date.period.as_str(), "Hour" | "Minute" | "Second"));
                    date.period = "Time".into();
                }
                FeatureMatch {
                    text: matched.text,
                    date,
                }
            })
            .collect();
        assert_eq!(matches, case.matches, "{} {:?}", case.id, case.input);
    }
    let metadata = serde_json::json!({
        "cohort": cohort, "cases": prepared.len(), "parsed": expected_parsed,
        "matched_dates": expected_matches, "iterations": iterations,
        "fixture_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "first_pass_ms": first.elapsed().as_secs_f64() * 1000.0,
    });
    println!("BENCHMARK_READY {metadata}");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    let mut pass_ms = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut parsed_count = 0;
        let mut match_count = 0;
        let start = Instant::now();
        for _ in 0..iterations {
            for (case, configuration, parser) in &prepared {
                let parsed = run_feature(black_box(case), configuration, parser);
                if let Ok((_, matches)) = &parsed {
                    parsed_count += usize::from(!matches.is_empty());
                    match_count += matches.len();
                }
                let _ = black_box(parsed);
            }
        }
        pass_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(parsed_count, expected_parsed * iterations);
        assert_eq!(match_count, expected_matches * iterations);
    }
    println!(
        "BENCHMARK_RESULT {}",
        serde_json::json!({ "metadata": metadata, "pass_ms": pass_ms })
    );
}
