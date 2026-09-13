use chrono::TimeZone;
use regex::Regex;
use std::sync::{Arc, OnceLock};

use crate::{language, locale, text, Configuration, Date, Error, Parser};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub date: Date,
    pub text: String,
}

pub fn search(
    configuration: &Configuration,
    input: &str,
) -> Result<(String, Vec<SearchResult>), Error> {
    crate::parser::default_parser().search(configuration, input)
}

pub fn search_with_language(
    configuration: &Configuration,
    language: &str,
    input: &str,
) -> Result<Vec<SearchResult>, Error> {
    crate::parser::default_parser().search_with_language(configuration, language, input)
}

fn search_configuration(configuration: &Configuration) -> Result<Configuration, Error> {
    let mut initialized = configuration.initialized().map_err(|error| match error {
        Error::InvalidConfiguration(message) => Error::InvalidSearchConfiguration(message),
        other => other,
    })?;
    if configuration
        .current_time
        .as_ref()
        .is_none_or(|time| *time == chrono::Utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0).unwrap())
    {
        initialized.current_time = None;
    }
    Ok(initialized)
}

fn internal_configuration(configuration: &Configuration) -> Configuration {
    Configuration {
        current_time: configuration.current_time.clone(),
        default_timezone: configuration.default_timezone.clone(),
        preferred_day_of_month: configuration.preferred_day_of_month,
        preferred_month_of_year: configuration.preferred_month_of_year,
        preferred_date_source: configuration.preferred_date_source,
        strict_parsing: configuration.strict_parsing,
        ignore_surrounding_text: configuration.ignore_surrounding_text,
        required_parts: configuration.required_parts.clone(),
        skip_tokens: configuration.skip_tokens.clone(),
        default_languages: configuration.default_languages.clone(),
        return_time_as_period: configuration.return_time_as_period,
        preserve_end_of_month: configuration.preserve_end_of_month,
        ..Configuration::default()
    }
}

struct ParsedSearch {
    date: Option<Date>,
    is_relative: bool,
    text: String,
}

impl Parser {
    pub fn search(
        &self,
        configuration: &Configuration,
        input: &str,
    ) -> Result<(String, Vec<SearchResult>), Error> {
        let configuration = search_configuration(configuration)?;
        let input = if configuration
            .languages
            .iter()
            .any(|language| language == "ru")
        {
            static FROM: OnceLock<Regex> = OnceLock::new();
            FROM.get_or_init(|| {
                Regex::new(r"(^|[^\p{L}\p{N}_])\x{441}[\t\n\r\x0c \p{Z}]+(\p{Nd})").unwrap()
            })
            .replace_all(input, "${1}[FROM] ${2}")
        } else {
            std::borrow::Cow::Borrowed(input)
        };
        let detected = if let Some(detector) = self
            .detect_languages_function
            .as_ref()
            .filter(|_| configuration.languages.is_empty() && configuration.locales.is_empty())
        {
            let configured = detector(&input);
            let configured = if configured.is_empty() {
                &configuration.default_languages
            } else {
                &configured
            };
            if configured.is_empty() {
                Err(Error::LanguageDetection)
            } else {
                let languages = locale::load_languages(&[], configured, true)?;
                Ok(languages[0].clone())
            }
        } else {
            let languages = locale::load_languages(
                &configuration.locales,
                &configuration.languages,
                configuration.use_given_order,
            )?;
            let unique = {
                let mut cache = self
                    .search_charsets
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if cache.0 != languages {
                    *cache = (
                        languages.clone(),
                        Arc::new(language::unique_charsets(&languages)),
                    );
                }
                cache.1.clone()
            };
            language::detect_full_text(&configuration, &input, &languages, &unique)
        };
        let mut candidates = detected
            .as_ref()
            .ok()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        if configuration.languages.len() > 1 || configuration.locales.len() > 1 {
            for candidate in
                locale::load_languages(&configuration.locales, &configuration.languages, true)?
            {
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
        if configuration.search_strategy == "ngram" && !candidates.is_empty() {
            let code = detected.unwrap_or_default();
            return Ok((
                code.clone(),
                self.search_ngrams(&configuration, &candidates, &code, &input),
            ));
        }
        for candidate in candidates {
            let found = self.search_with_language(&configuration, &candidate, &input)?;
            if !found.is_empty() {
                return Ok((candidate, found));
            }
        }
        detected.map(|language| (language, Vec::new()))
    }

    pub fn search_with_language(
        &self,
        configuration: &Configuration,
        code: &str,
        input: &str,
    ) -> Result<Vec<SearchResult>, Error> {
        let configuration = search_configuration(configuration)?;
        let locale = locale::data()
            .get(code)
            .ok_or_else(|| Error::UnknownLanguage(code.into()))?;
        if configuration.search_strategy == "ngram" {
            return Ok(self.search_ngrams(&configuration, &[code.into()], code, input));
        }
        let (translations, originals) = language::translate_search(&configuration, locale, input);
        let mut internal = internal_configuration(&configuration);
        internal.languages = vec![if matches!(code, "vi" | "hu") {
            code
        } else {
            "en"
        }
        .into()];
        let need_relative_base = internal.current_time.is_none();
        let mut found = Vec::new();
        for (translation, original) in translations.iter().zip(&originals) {
            let entry = if matches!(code, "vi" | "hu") {
                original
            } else {
                translation
            };
            if entry.chars().count() <= 2 {
                continue;
            }
            let mut parsed = self.parse_search_entry(
                &mut internal,
                entry,
                translation,
                &found,
                need_relative_base,
            );
            if parsed.date.is_some() {
                parsed.text = original
                    .trim_matches(|character| " .,:()[]-'".contains(character))
                    .into();
                found.push(parsed);
                continue;
            }
            let mut alternatives = Vec::new();
            for split in split_if_not_parsed(entry, original) {
                let mut candidate = Vec::new();
                for (entry, original) in split.entries.iter().zip(&split.originals) {
                    if entry.chars().count() <= 2 {
                        continue;
                    }
                    let mut parsed = self.parse_search_entry(
                        &mut internal,
                        entry,
                        entry,
                        &candidate,
                        need_relative_base,
                    );
                    parsed.text = original
                        .trim_matches(|character| " .,:()[]-".contains(character))
                        .into();
                    candidate.push(parsed);
                }
                alternatives.push(candidate);
            }
            if let Some(best) = alternatives.into_iter().min_by(|left, right| {
                let rating = |candidate: &[ParsedSearch]| {
                    let size = candidate.len();
                    let unparsed = candidate
                        .iter()
                        .filter(|parsed| parsed.date.is_none())
                        .count();
                    let without_digits = candidate
                        .iter()
                        .filter(|parsed| !parsed.text.chars().any(text::is_digit))
                        .count();
                    (
                        if size == 0 {
                            0.0
                        } else {
                            unparsed as f64 / size as f64
                        },
                        size,
                        if size == 0 {
                            0.0
                        } else {
                            without_digits as f64 / size as f64
                        },
                    )
                };
                let left = rating(left);
                let right = rating(right);
                left.0
                    .total_cmp(&right.0)
                    .then(left.1.cmp(&right.1))
                    .then(left.2.total_cmp(&right.2))
            }) {
                found.extend(best.into_iter().filter(|parsed| parsed.date.is_some()));
            }
        }
        let mut results: Vec<_> = found
            .into_iter()
            .map(|parsed| {
                let mut date = parsed.date.unwrap();
                date.locale = code.into();
                SearchResult {
                    date,
                    text: parsed.text,
                }
            })
            .collect();
        if configuration.return_time_span {
            results.extend(crate::time_span::search(&configuration, code, input));
        }
        Ok(results)
    }

    fn search_ngrams(
        &self,
        configuration: &Configuration,
        locales: &[String],
        code: &str,
        input: &str,
    ) -> Vec<SearchResult> {
        static EXPRESSIONS: OnceLock<[Regex; 2]> = OnceLock::new();
        let [token, bad_candidate] = EXPRESSIONS.get_or_init(|| {
            [
                r"[^\t\n\r\x0c \p{Z}\x{0085},|()@]+",
                r"^(\p{Nd}{1,3}|#\p{Nd}+|[-/.]+|[\p{L}\p{N}_]\.?|an)$",
            ]
            .map(|pattern| Regex::new(pattern).unwrap())
        });
        let mut configuration_for_parse = configuration.clone();
        configuration_for_parse.locales = locales.to_vec();
        configuration_for_parse.languages.clear();
        configuration_for_parse.use_given_order = true;
        configuration_for_parse.try_previous_locales = false;
        let tokens: Vec<_> = token
            .find_iter(input)
            .filter(|token| !matches!(token.as_str(), "on" | "at" | "of" | "a"))
            .collect();
        let mut results = Vec::new();
        let mut index = 0;
        let mut candidate = String::new();
        while index < tokens.len() {
            let limit = 7.min(tokens.len() - index);
            let mut ends = [0; 7];
            candidate.clear();
            for (offset, token) in tokens[index..index + limit].iter().enumerate() {
                if offset > 0 {
                    candidate.push(' ');
                }
                candidate.push_str(token.as_str());
                ends[offset] = candidate.len();
            }
            let mut consumed = 1;
            for size in (1..=limit).rev() {
                let positions = &tokens[index..index + size];
                candidate.truncate(ends[size - 1]);
                if bad_candidate.is_match(&candidate) {
                    continue;
                }
                if let Ok(mut date) = self.parse(&configuration_for_parse, &candidate, &[]) {
                    date.locale = code.into();
                    results.push(SearchResult {
                        date,
                        text: input[positions[0].start()..positions[size - 1].end()]
                            .trim_matches(|character| " .,:()[]-'".contains(character))
                            .into(),
                    });
                    consumed = size;
                    break;
                }
            }
            index += consumed;
        }
        if configuration.return_time_span {
            results.extend(crate::time_span::search(configuration, code, input));
        }
        results
    }

    fn parse_search_entry(
        &self,
        configuration: &mut Configuration,
        entry: &str,
        translation: &str,
        previous: &[ParsedSearch],
        need_relative_base: bool,
    ) -> ParsedSearch {
        let entry = entry.replace("ng\u{e0}y", "").replace("am", "");
        let mut date = self.parse(configuration, &entry, &[]).ok();
        if need_relative_base {
            if let Some(base) = previous
                .iter()
                .rev()
                .find(|parsed| !parsed.is_relative)
                .and_then(|parsed| parsed.date.as_ref())
            {
                configuration.current_time = Some(base.time.clone());
                date = self.parse(configuration, &entry, &[]).ok();
            }
        }
        ParsedSearch {
            date,
            is_relative: ["ago", "in", "from now", "tomorrow", "today", "yesterday"]
                .iter()
                .any(|word| translation.contains(word)),
            text: String::new(),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct SplitResult {
    entries: Vec<String>,
    originals: Vec<String>,
}

fn split_by(entry: &str, original: &str, splitter: &str) -> Vec<SplitResult> {
    let entries: Vec<_> = entry.split(splitter).collect();
    let originals: Vec<_> = original.split(splitter).collect();
    let mut splits = vec![SplitResult {
        entries: entries.iter().map(|part| (*part).into()).collect(),
        originals: originals.iter().map(|part| (*part).into()).collect(),
    }];
    if entry.matches(splitter).count() > 2 {
        for width in 2..4 {
            splits.push(SplitResult {
                entries: entries
                    .chunks(width)
                    .map(|parts| parts.join(splitter))
                    .collect(),
                originals: originals
                    .chunks(width)
                    .map(|parts| parts.join(splitter))
                    .collect(),
            });
        }
    }
    splits
}

fn split_if_not_parsed(entry: &str, original: &str) -> Vec<SplitResult> {
    [
        ",",
        "\u{60c}",
        "\u{2014}\u{2014}",
        "\u{2014}",
        "\u{2013}",
        ".",
        " ",
    ]
    .into_iter()
    .filter(|splitter| {
        entry.contains(splitter)
            && entry.matches(splitter).count() == original.matches(splitter).count()
    })
    .flat_map(|splitter| split_by(entry, original, splitter))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_parser_searches_keep_caller_state_and_callbacks_independent() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let mut parser = Parser::new();
        parser.detect_languages_function = Some(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
            vec!["en".into(), "fr".into()]
        }));
        std::thread::scope(|scope| {
            for strategy in ["split", "ngram"] {
                for (language, input) in [("en", "4 October 1957"), ("fr", "4 octobre 1957")] {
                    let parser = &parser;
                    scope.spawn(move || {
                        let configuration = Configuration {
                            languages: vec![language.into()],
                            search_strategy: strategy.into(),
                            try_previous_locales: true,
                            current_time: Some(
                                crate::Timezone::Utc
                                    .with_ymd_and_hms(2024, 3, 20, 0, 0, 0)
                                    .unwrap(),
                            ),
                            ..Configuration::default()
                        };
                        for _ in 0..4 {
                            let date = parser.parse(&configuration, input, &[]).unwrap();
                            assert_eq!(date.time.format("%F").to_string(), "1957-10-04");
                            let (detected, found) = parser
                                .search(&configuration, &format!("Recorded {input}"))
                                .unwrap();
                            assert_eq!(detected, language);
                            assert_eq!(found.len(), 1);
                            assert_eq!(found[0].text, input);
                            assert_eq!(found[0].date.locale, language);
                            assert_eq!(found[0].date.time.format("%F").to_string(), "1957-10-04");
                            assert_eq!(
                                crate::parse_jalali(&configuration, "1/1/1403")
                                    .unwrap()
                                    .time
                                    .format("%F")
                                    .to_string(),
                                "2024-03-20"
                            );
                            assert_eq!(
                                crate::parse_hijri(&configuration, "14/9/1432")
                                    .unwrap()
                                    .time
                                    .format("%F")
                                    .to_string(),
                                "2011-08-14"
                            );
                        }
                        assert_eq!(configuration.languages, [language]);
                        assert!(configuration.skip_tokens.is_empty());
                        assert_eq!(configuration.search_strategy, strategy);
                    });
                }
            }
        });
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        let (detected, found) = parser
            .search(&Configuration::default(), "Recorded 4 October 1957")
            .unwrap();
        assert_eq!(detected, "en");
        assert_eq!(found.len(), 1);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn ngram_search_preserves_longest_matches_and_span_labels() {
        let configuration = Configuration {
            languages: vec!["en".into()],
            search_strategy: "ngram".into(),
            current_time: Some(
                crate::Timezone::Utc
                    .with_ymd_and_hms(2025, 2, 15, 12, 0, 0)
                    .unwrap(),
            ),
            ..Configuration::default()
        };
        let (code, found) = search(
            &configuration,
            "The first artificial Earth satellite was launched on 4 October 1957.",
        )
        .unwrap();
        assert_eq!(code, "en");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "4 October 1957");
        assert!(search(&configuration, "Chapter 12, page 3")
            .unwrap()
            .1
            .is_empty());
        let configuration = Configuration {
            return_time_span: true,
            ..configuration
        };
        let found =
            search_with_language(&configuration, "en", "messages received for the past week")
                .unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text, "for the past week (start)");
        assert_eq!(
            found[0].date.time.format("%F %T").to_string(),
            "2025-02-03 12:00:00"
        );
        assert_eq!(
            found[1].date.time.format("%F %T").to_string(),
            "2025-02-09 12:00:00"
        );
    }

    #[test]
    fn search_detects_languages_and_preserves_explicit_fallbacks() {
        let mut configuration = Configuration {
            current_time: Some(
                crate::Timezone::Utc
                    .with_ymd_and_hms(2025, 9, 15, 0, 0, 0)
                    .unwrap(),
            ),
            ..Configuration::default()
        };
        let (code, found) = search(
            &configuration,
            "The satellite was launched on 4 October 1957",
        )
        .unwrap();
        assert_eq!(code, "en");
        assert_eq!(found[0].text, "on 4 October 1957");
        configuration.languages = ["en", "fr", "es", "pt", "de", "it", "ar"]
            .map(String::from)
            .to_vec();
        configuration.strict_parsing = true;
        let (code, found) = search(
            &configuration,
            "Date de facture 23 juillet 2020 Condition Redevable livraison FR",
        )
        .unwrap();
        assert_eq!(code, "fr");
        assert_eq!(found[0].text, "23 juillet 2020");
        configuration.locales = vec!["en-US".into(), "en-GB".into()];
        assert!(search(&configuration, "4 October 1957").is_ok());
    }

    #[test]
    fn split_search_parses_text_and_uses_the_original_relative_base_rules() {
        let configuration = Configuration {
            current_time: Some(
                crate::Timezone::Utc
                    .with_ymd_and_hms(2000, 1, 1, 0, 0, 0)
                    .unwrap(),
            ),
            ..Configuration::default()
        };
        let input = "25th march 2015 , i need this report today.";
        let found = search_with_language(&configuration, "en", input).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text, "25th march 2015");
        assert_eq!(found[0].date.time.format("%F").to_string(), "2015-03-25");
        assert_eq!(found[1].text, "today");
        assert_eq!(found[1].date.time.format("%F").to_string(), "2000-01-01");
        let found = search_with_language(&Configuration::default(), "en", input).unwrap();
        assert_eq!(found[1].date.time.format("%F").to_string(), "2015-03-25");
        assert!(matches!(
            search_with_language(&configuration, "invalid", input),
            Err(Error::UnknownLanguage(_))
        ));
    }

    #[test]
    fn split_search_preserves_original_alignment_and_grouping() {
        let splits = split_if_not_parsed("10 mar 2015 12 feb 2016", "10 mars 2015 12 fevrier 2016");
        assert_eq!(splits.len(), 3);
        assert_eq!(
            splits[0].entries,
            ["10", "mar", "2015", "12", "feb", "2016"]
        );
        assert_eq!(splits[1].entries, ["10 mar", "2015 12", "feb 2016"]);
        assert_eq!(splits[2].entries, ["10 mar 2015", "12 feb 2016"]);
        assert_eq!(splits[2].originals, ["10 mars 2015", "12 fevrier 2016"]);
        assert!(split_if_not_parsed("one,two", "un deux").is_empty());
        assert_eq!(split_by("one,two,three", "un,deux,trois", ",").len(), 1);
    }
}
