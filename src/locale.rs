use std::{collections::HashMap, sync::OnceLock};

use crate::Error;
use aho_corasick::AhoCorasick;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Replacement {
    pub pattern: usize,
    pub replacement: String,
}

#[derive(Deserialize)]
pub(crate) struct Locale {
    pub name: String,
    pub date_order: String,
    pub no_word_spacing: bool,
    #[serde(default)]
    pub sentence_splitter_group: usize,
    #[serde(default)]
    pub charset: String,
    #[serde(default)]
    pub abbreviations: Option<Vec<String>>,
    pub simplifications: Vec<Replacement>,
    pub translations: HashMap<String, Vec<String>>,
    pub relative_type: HashMap<String, String>,
    pub relative_type_regexes: Vec<Replacement>,
    pub combined: i32,
    pub exact_combined: i32,
    pub known_words: Vec<String>,
    #[serde(skip)]
    known_word_matcher: OnceLock<AhoCorasick>,
    #[serde(skip)]
    applicability_dictionary: OnceLock<HashMap<String, bool>>,
}

#[derive(Deserialize)]
pub(crate) struct Locales {
    pub language_order: HashMap<String, usize>,
    pub locale_order: HashMap<String, usize>,
    pub locales: Vec<Locale>,
    pub patterns: Vec<String>,
    pub digit_mappings: Vec<[u32; 2]>,
    #[serde(skip)]
    compiled: Vec<OnceLock<Regex>>,
    #[serde(skip)]
    indexes: HashMap<String, usize>,
}

pub(crate) fn data() -> &'static Locales {
    static DATA: OnceLock<Locales> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut data: Locales = serde_json::from_slice(include_bytes!("../data/locales.json"))
            .expect("pinned locale data must decode");
        data.compiled = data.patterns.iter().map(|_| OnceLock::new()).collect();
        data.indexes = data
            .locales
            .iter()
            .enumerate()
            .map(|(index, locale)| (locale.name.clone(), index))
            .collect();
        data
    })
}

impl Locales {
    pub fn get(&self, name: &str) -> Option<&Locale> {
        self.indexes.get(name).map(|index| &self.locales[*index])
    }

    pub fn expression(&self, index: usize) -> &Regex {
        self.compiled[index].get_or_init(|| {
            RegexBuilder::new(&self.patterns[index])
                .size_limit(64 * 1024 * 1024)
                .build()
                .expect("pinned locale expression must compile")
        })
    }
}

impl Locale {
    pub fn applicability_dictionary(&self) -> &HashMap<String, bool> {
        self.applicability_dictionary.get_or_init(|| {
            let mut dictionary = HashMap::new();
            let mut add_words = |phrase: &str, meaningful: bool| {
                let words = if phrase.contains(' ') {
                    phrase.split_whitespace().collect()
                } else {
                    vec![phrase]
                };
                for word in words {
                    *dictionary.entry(word.to_string()).or_insert(false) |= meaningful;
                }
            };
            for (phrase, translations) in &self.translations {
                add_words(
                    phrase,
                    translations
                        .iter()
                        .any(|translation| !translation.is_empty()),
                );
            }
            for (phrase, translation) in &self.relative_type {
                add_words(phrase, !translation.is_empty());
            }
            dictionary
        })
    }

    pub fn known_word_matcher(&self) -> &AhoCorasick {
        self.known_word_matcher.get_or_init(|| {
            AhoCorasick::new(
                self.known_words
                    .iter()
                    .filter(|word| !word.is_empty())
                    .map(String::as_str),
            )
            .expect("pinned known words must compile")
        })
    }

    pub fn exact_match(&self, input: &str) -> bool {
        self.exact_combined >= 0
            && input.bytes().any(|byte| byte.is_ascii_digit())
            && data()
                .expression(self.exact_combined as usize)
                .is_match(input)
    }

    pub fn contains(&self, input: &str) -> bool {
        self.relative_type.contains_key(input) || self.translations.contains_key(input)
    }
}

pub(crate) fn load_languages(
    locales: &[String],
    languages: &[String],
    given_order: bool,
) -> Result<Vec<String>, Error> {
    let data = data();
    if locales.is_empty() && languages.is_empty() {
        static DEFAULTS: OnceLock<Vec<String>> = OnceLock::new();
        return Ok(DEFAULTS
            .get_or_init(|| {
                let mut languages: Vec<_> = data.language_order.keys().cloned().collect();
                languages.sort_by_key(|name| data.language_order[name]);
                languages
            })
            .clone());
    }
    let mut result = Vec::new();
    let mut unknown = Vec::new();
    if !locales.is_empty() {
        for name in locales {
            if data.get(name).is_none() {
                unknown.push(name.clone());
                continue;
            }
            let language = name
                .rsplit_once('-')
                .filter(|(_, suffix)| !suffix.is_empty() && suffix.to_uppercase() == *suffix)
                .map_or(name.as_str(), |(language, _)| language);
            if !result.iter().any(|name| name == language) {
                result.push(language.into());
            }
        }
        if !unknown.is_empty() {
            return Err(Error::UnknownLocales(unknown));
        }
    } else {
        result = languages.to_vec();
        for name in languages {
            if !data.language_order.contains_key(name) {
                unknown.push(name.clone());
            }
        }
        if !unknown.is_empty() {
            return Err(Error::UnknownLanguages(unknown));
        }
    }
    if !given_order {
        result.sort_by(|left, right| {
            data.language_order[left]
                .cmp(&data.language_order[right])
                .then(left.cmp(right))
        });
    }
    if result.is_empty() {
        result.extend(data.language_order.keys().cloned());
        result.sort_by_key(|name| data.language_order[name]);
    }
    Ok(result)
}

pub(crate) fn load(
    locales: &[String],
    languages: &[String],
    region: &str,
    given_order: bool,
) -> Result<Vec<&'static Locale>, Error> {
    if let [name] = locales {
        return data()
            .get(name)
            .map(|locale| vec![locale])
            .ok_or_else(|| Error::UnknownLocales(vec![name.clone()]));
    }
    if locales.is_empty() {
        if let [language] = languages {
            let data = data();
            if !data.language_order.contains_key(language) {
                return Err(Error::UnknownLanguages(vec![language.clone()]));
            }
            let region = region.trim();
            let locale = if region.is_empty() {
                data.get(language)
            } else {
                data.get(&format!("{language}-{}", region.to_uppercase()))
            };
            return Ok(locale.into_iter().collect());
        }
    }
    if locales.is_empty() && languages.is_empty() && region.trim().is_empty() {
        static DEFAULTS: [OnceLock<Vec<&'static Locale>>; 2] = [const { OnceLock::new() }; 2];
        return Ok(DEFAULTS[usize::from(given_order)]
            .get_or_init(|| {
                load_uncached(&[], &[], "", given_order).expect("default locales must load")
            })
            .clone());
    }
    load_uncached(locales, languages, region, given_order)
}

fn load_uncached(
    locales: &[String],
    languages: &[String],
    region: &str,
    given_order: bool,
) -> Result<Vec<&'static Locale>, Error> {
    let data = data();
    let mut names = Vec::new();
    let mut seen_locales = std::collections::HashSet::new();
    let mut seen_languages = std::collections::HashSet::new();
    if !locales.is_empty() {
        let mut unknown = Vec::new();
        for name in locales {
            if seen_locales.contains(name) {
                continue;
            }
            if data.get(name).is_none() {
                unknown.push(name.clone());
                continue;
            }
            let language = name
                .rsplit_once('-')
                .filter(|(_, suffix)| !suffix.is_empty() && suffix.to_uppercase() == *suffix)
                .map_or(name.as_str(), |(language, _)| language);
            if !seen_languages.insert(language.to_string()) {
                return Err(Error::ConflictingLocales);
            }
            seen_locales.insert(name.clone());
            names.push(name.clone());
        }
        if !unknown.is_empty() {
            return Err(Error::UnknownLocales(unknown));
        }
    } else {
        let languages = if languages.is_empty() {
            let mut languages: Vec<_> = data.language_order.keys().cloned().collect();
            languages.sort_by_key(|name| data.language_order[name]);
            languages
        } else {
            languages.to_vec()
        };
        let region = region.trim().to_uppercase();
        let mut unknown = Vec::new();
        for language in languages {
            if !seen_languages.insert(language.clone()) {
                continue;
            }
            if !data.language_order.contains_key(&language) {
                unknown.push(language);
                continue;
            }
            let name = if region.is_empty() {
                language
            } else {
                format!("{language}-{region}")
            };
            if seen_locales.contains(&name) || data.get(&name).is_none() {
                continue;
            }
            seen_locales.insert(name.clone());
            names.push(name);
        }
        if !unknown.is_empty() {
            return Err(Error::UnknownLanguages(unknown));
        }
    }
    if !given_order {
        names.sort_by(|left, right| {
            data.locale_order[left]
                .cmp(&data.locale_order[right])
                .then(left.cmp(right))
        });
    }
    Ok(names
        .into_iter()
        .map(|name| data.get(&name).unwrap())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_locale_loading_matches_general_validation() {
        for given_order in [false, true] {
            for name in data()
                .locale_order
                .keys()
                .chain(["unknown".to_string()].iter())
            {
                for region in ["", "  us ", "ZZ"] {
                    let names = [name.clone()];
                    let actual = load(&names, &[], region, given_order).map(|locales| {
                        locales
                            .into_iter()
                            .map(|locale| &locale.name)
                            .collect::<Vec<_>>()
                    });
                    let expected = load_uncached(&names, &[], region, given_order).map(|locales| {
                        locales
                            .into_iter()
                            .map(|locale| &locale.name)
                            .collect::<Vec<_>>()
                    });
                    assert_eq!(actual, expected, "{name} {region}");
                }
            }
            for name in data()
                .language_order
                .keys()
                .chain(["unknown".to_string()].iter())
            {
                for region in ["", "  us ", "ZZ"] {
                    let names = [name.clone()];
                    let actual = load(&[], &names, region, given_order).map(|locales| {
                        locales
                            .into_iter()
                            .map(|locale| &locale.name)
                            .collect::<Vec<_>>()
                    });
                    let expected = load_uncached(&[], &names, region, given_order).map(|locales| {
                        locales
                            .into_iter()
                            .map(|locale| &locale.name)
                            .collect::<Vec<_>>()
                    });
                    assert_eq!(actual, expected, "{name} {region}");
                }
            }
            let expected = load_uncached(&[], &[], "", true).unwrap();
            let actual = load_languages(&[], &[], given_order).unwrap();
            assert_eq!(
                actual,
                expected
                    .iter()
                    .map(|locale| locale.name.clone())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn cached_default_locales_preserve_order() {
        for given_order in [false, true] {
            let expected = load_uncached(&[], &[], "", given_order).unwrap();
            for region in ["", "   "] {
                let actual = load(&[], &[], region, given_order).unwrap();
                assert_eq!(actual.len(), expected.len());
                assert!(actual
                    .iter()
                    .zip(&expected)
                    .all(|(actual, expected)| std::ptr::eq(*actual, *expected)));
            }
        }
    }

    #[test]
    fn locale_data_contains_all_pinned_rules() {
        let data = data();
        assert_eq!(data.language_order.len(), 205);
        assert_eq!(data.locales.len(), 512);
        assert_eq!(data.locale_order.len(), 512);
        assert_eq!(data.get("en").unwrap().date_order, "MDY");
        assert_eq!(data.get("en-GB").unwrap().date_order, "DMY");
        for index in 0..data.patterns.len() {
            data.expression(index);
        }
        for locale in &data.locales {
            assert!(data.locale_order.contains_key(&locale.name));
            for rule in locale
                .simplifications
                .iter()
                .chain(&locale.relative_type_regexes)
            {
                data.expression(rule.pattern);
            }
        }
    }
}
