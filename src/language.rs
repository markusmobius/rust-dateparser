use std::{collections::HashSet, sync::OnceLock};

use regex::Regex;

use crate::locale::{data, Locale};
use crate::{text, Configuration};

const SEPARATOR: &str = "|#=#=#|";

fn always_kept(token: &str) -> bool {
    matches!(token, "+" | ":" | "." | " " | "-" | "/")
}
fn is_space(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|byte| byte == b' ')
}

fn skipped_tokens<'configuration>(
    configuration: &'configuration Configuration,
    locale: &Locale,
) -> HashSet<&'configuration str> {
    configuration
        .skip_tokens
        .iter()
        .map(String::as_str)
        .filter(|token| locale.name != "fi" || *token != "t")
        .collect()
}

pub(crate) fn simplify(locale: &Locale, input: &str) -> String {
    let mut simplified = input.to_string();
    for rule in &locale.simplifications {
        simplified = data()
            .expression(rule.pattern)
            .replace_all(&simplified, rule.replacement.as_str())
            .into_owned();
    }
    if locale.name == "ru" {
        static NUMBER_PAIR: OnceLock<Regex> = OnceLock::new();
        let expression = NUMBER_PAIR.get_or_init(|| {
            Regex::new(r"(?-u:\b)([0-9]+)[\t\n\r\x0c \p{Z}]+([0-9]+)(?-u:\b)").unwrap()
        });
        simplified = expression
            .replace_all(&simplified, |captures: &regex::Captures<'_>| {
                let first = captures[1].parse::<i64>().unwrap_or(0);
                let second = captures[2].parse::<i64>().unwrap_or(0);
                if matches!(first, 20 | 30) && (1..=9).contains(&second) && first + second <= 31 {
                    (first + second).to_string()
                } else {
                    captures[0].to_string()
                }
            })
            .into_owned();
    }
    simplified
}

fn captured(token: &str, formatting: bool) -> bool {
    formatting
        || is_space(token)
        || always_kept(token)
        || (!token.contains('\n')
            && token
                .chars()
                .any(|character| character.is_ascii_alphanumeric() || text::is_letter(character)))
}

fn split_numerals(input: &str, formatting: bool) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut previous_digit = false;
    for (index, character) in input.char_indices() {
        let digit = text::is_digit(character);
        if index > 0 && digit != previous_digit {
            if captured(&input[start..index], formatting) {
                tokens.push(input[start..index].into());
            }
            start = index;
        }
        previous_digit = digit;
    }
    if captured(&input[start..], formatting) {
        tokens.push(input[start..].into());
    }
    tokens
}

fn known_word(locale: &Locale, input: &str) -> Option<(usize, usize)> {
    if input.trim().is_empty() {
        return None;
    }
    let mut earliest = None;
    for word in &locale.known_words {
        if word.is_empty() {
            continue;
        }
        let Some(start) = input.find(word) else {
            continue;
        };
        let end = start + word.len();
        let allowed = |neighbor: Option<char>| {
            neighbor.is_none_or(|character| !text::is_letter_or_mark(character))
        };
        if locale.no_word_spacing
            || (allowed(input[..start].chars().next_back()) && allowed(input[end..].chars().next()))
        {
            if earliest.is_none_or(|(previous, _)| start < previous) {
                earliest = Some((start, end));
            }
            if start == 0 {
                break;
            }
        }
    }
    earliest
}

fn split_known(locale: &Locale, mut input: &str, formatting: bool) -> Vec<String> {
    let mut tokens = Vec::new();
    loop {
        let Some((start, end)) = known_word(locale, input) else {
            if captured(input, formatting) {
                tokens.extend(split_numerals(input, formatting));
            }
            break;
        };
        if start > 0 && captured(&input[..start], formatting) {
            tokens.extend(split_numerals(&input[..start], formatting));
        }
        if captured(&input[start..end], formatting) {
            tokens.push(input[start..end].into());
        }
        input = &input[end..];
        if input.is_empty() {
            break;
        }
    }
    tokens
}

pub(crate) fn split(
    locale: &Locale,
    input: &str,
    formatting: bool,
    skipped: &HashSet<&str>,
) -> Vec<String> {
    let mut separated = String::new();
    let mut remaining = input;
    if locale.combined >= 0 {
        let expression = data().expression(locale.combined as usize);
        while let Some(captures) = expression.captures(remaining) {
            let matched = captures.get(0).unwrap();
            if matched.end() == 0 {
                break;
            }
            separated.push_str(&remaining[..matched.start()]);
            let group = |index| captures.get(index).map_or("", |capture| capture.as_str());
            if captures.len() == 4 {
                separated.push_str(group(1));
                separated.push_str(SEPARATOR);
                separated.push_str(group(2));
                separated.push_str(SEPARATOR);
                separated.push_str(group(3));
            } else {
                separated.push_str(SEPARATOR);
                separated.push_str(if captures.len() == 2 {
                    group(1)
                } else {
                    matched.as_str()
                });
                separated.push_str(SEPARATOR);
            }
            remaining = &remaining[matched.end()..];
        }
    }
    separated.push_str(remaining);
    let mut tokens: Vec<String> = Vec::new();
    for segment in separated.split(SEPARATOR) {
        let pieces = if locale.exact_match(segment) {
            vec![segment.into()]
        } else {
            split_known(locale, segment, formatting)
        };
        for token in pieces {
            if token.is_empty() {
                continue;
            }
            if tokens
                .last()
                .is_some_and(|previous| text::is_number_only(previous))
                && matches!(token.as_str(), "st" | "nd" | "rd" | "th")
            {
                continue;
            }
            if !skipped.contains(token.trim()) {
                tokens.push(token);
            }
        }
    }
    tokens
}

fn trim_unknown(locale: &Locale, tokens: Vec<String>) -> Vec<String> {
    let extra = |token: &str| {
        token.trim().is_empty()
            || (!text::is_number_only(token)
                && !locale.contains(token)
                && !locale.exact_match(token))
    };
    let start = tokens
        .iter()
        .position(|token| !extra(token))
        .unwrap_or(tokens.len());
    let mut end = tokens.len();
    while end > start && extra(&tokens[end - 1]) && !crate::timezone::is_token(&tokens[end - 1]) {
        end -= 1;
    }
    tokens[start..end].to_vec()
}

pub(crate) fn applicable(
    configuration: &Configuration,
    locale: &Locale,
    input: &str,
    ignore_surrounding: bool,
) -> bool {
    let input = simplify(locale, &text::normalize_digits(&text::normalize(input)));
    let skipped = skipped_tokens(configuration, locale);
    let tokens = split(locale, &input, false, &skipped);
    let tokens = if ignore_surrounding {
        trim_unknown(locale, tokens)
    } else {
        tokens
    };
    if tokens.iter().all(|token| always_kept(token)) {
        return false;
    }
    tokens.iter().all(|token| {
        text::is_number_only(token)
            || locale.contains(token)
            || skipped.contains(token.as_str())
            || locale.exact_match(token)
    })
}

fn remove_empty(tokens: &[String]) -> Vec<String> {
    let mut filtered: Vec<String> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if !tokens[index].is_empty() {
            filtered.push(tokens[index].clone());
            index += 1;
            continue;
        }
        let mut start = filtered.len();
        let mut previous_spaces = 0;
        while start > 0 && is_space(&filtered[start - 1]) {
            start -= 1;
            previous_spaces += filtered[start].len();
        }
        let mut next = index + 1;
        let mut next_spaces = 0;
        while next < tokens.len() && is_space(&tokens[next]) {
            next_spaces += tokens[next].len();
            next += 1;
        }
        if previous_spaces > 0 && next_spaces > 0 {
            filtered.truncate(start);
            filtered.push(" ".repeat(previous_spaces.max(next_spaces)));
            index = next;
        } else {
            index += 1;
        }
    }
    filtered
}

fn join(tokens: &[String], formatting: bool) -> String {
    let mut joined = String::new();
    for (index, token) in tokens.iter().enumerate() {
        if index > 0 && !formatting {
            let left = &tokens[index - 1];
            if !always_kept(left) && !is_space(left) && !always_kept(token) && !is_space(token) {
                joined.push(' ');
            }
        }
        joined.push_str(token);
    }
    joined.trim().into()
}

pub(crate) fn translate(
    configuration: &Configuration,
    locale: &Locale,
    input: &str,
    formatting: bool,
    ignore_surrounding: bool,
) -> Vec<String> {
    let skipped = skipped_tokens(configuration, locale);
    let input = simplify(
        locale,
        &text::normalize_digits(
            &text::normalize_unicode(input)
                .chars()
                .flat_map(char::to_lowercase)
                .collect::<String>(),
        ),
    );
    let tokens = split(locale, &input, formatting, &skipped);
    let tokens = if ignore_surrounding {
        trim_unknown(locale, tokens)
    } else {
        tokens
    };
    let mut permutations = vec![Vec::new()];
    for token in tokens {
        let translations = if skipped.contains(token.as_str()) {
            vec![String::new()]
        } else if let Some(rule) = locale
            .relative_type_regexes
            .iter()
            .find(|rule| data().expression(rule.pattern).is_match(&token))
        {
            vec![data()
                .expression(rule.pattern)
                .replace_all(&token, rule.replacement.as_str())
                .into_owned()]
        } else if let Some(translation) = locale.relative_type.get(&token) {
            vec![translation.clone()]
        } else if let Some(translations) = locale.translations.get(&token) {
            translations
                .iter()
                .map(|translation| {
                    if translation.is_empty()
                        && formatting
                        && token.chars().any(|character| !text::is_letter(character))
                    {
                        token.clone()
                    } else {
                        translation.clone()
                    }
                })
                .collect()
        } else {
            vec![token]
        };
        let mut next = Vec::new();
        for permutation in permutations {
            for translation in &translations {
                let mut candidate = permutation.clone();
                candidate.push(translation.clone());
                next.push(candidate);
            }
        }
        permutations = next;
    }
    permutations
        .into_iter()
        .map(|mut tokens| {
            if !tokens.iter().any(|token| {
                matches!(
                    token.as_str(),
                    "day" | "week" | "month" | "year" | "hour" | "minute" | "second"
                )
            }) {
                if let Some(index) = tokens.iter().position(|token| token == "in") {
                    tokens[index].clear();
                }
            }
            join(&remove_empty(&tokens), formatting)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translations_preserve_go_whitespace_and_localized_rules() {
        let configuration = Configuration::default();
        for (locale, input, expected) in [
            ("en", "Sep 03 2014", "september 03 2014"),
            ("fr", "20 F\u{e9}vrier 2012", "20 february 2012"),
            ("fi", "28  maalis  klo  9:37", "28  march  9:37"),
            (
                "cs",
                "22.  prosinec  2014  v  2:38",
                "22.  december  2014  2:38",
            ),
            ("de", "vor einer Woche", "1 week ago"),
            ("ja", "2013\u{5e74}04\u{6708}08\u{65e5}", "2013-04-08"),
        ] {
            let locale = data().get(locale).unwrap();
            for formatting in [false, true] {
                assert_eq!(
                    translate(&configuration, locale, input, formatting, false)[0],
                    expected,
                    "{} {input:?} {formatting}",
                    locale.name
                );
                assert!(applicable(&configuration, locale, input, false));
            }
        }
    }
}
