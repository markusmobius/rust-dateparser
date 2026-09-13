use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use regex::Regex;

use crate::locale::{data, Locale};
use crate::{text, Configuration, Error};

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

pub(crate) fn simplify<'input>(
    locale: &Locale,
    input: &'input str,
) -> std::borrow::Cow<'input, str> {
    let mut simplified = std::borrow::Cow::Borrowed(input);
    for rule in &locale.simplifications {
        let expression = data().expression(rule.pattern);
        if !expression.is_match(&simplified) {
            continue;
        }
        if let std::borrow::Cow::Owned(replaced) =
            expression.replace_all(&simplified, rule.replacement.as_str())
        {
            simplified = std::borrow::Cow::Owned(replaced);
        }
    }
    if locale.name == "ru" {
        static NUMBER_PAIR: OnceLock<Regex> = OnceLock::new();
        let expression = NUMBER_PAIR.get_or_init(|| {
            Regex::new(r"(?-u:\b)([0-9]+)[\t\n\r\x0c \p{Z}]+([0-9]+)(?-u:\b)").unwrap()
        });
        if let std::borrow::Cow::Owned(replaced) =
            expression.replace_all(&simplified, |captures: &regex::Captures<'_>| {
                let first = captures[1].parse::<i64>().unwrap_or(0);
                let second = captures[2].parse::<i64>().unwrap_or(0);
                if matches!(first, 20 | 30) && (1..=9).contains(&second) && first + second <= 31 {
                    (first + second).to_string()
                } else {
                    captures[0].to_string()
                }
            })
        {
            simplified = std::borrow::Cow::Owned(replaced);
        }
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

fn split_numerals<'input>(input: &'input str, formatting: bool, tokens: &mut Vec<&'input str>) {
    let mut start = 0;
    let mut previous_digit = false;
    for (index, character) in input.char_indices() {
        let digit = text::is_digit(character);
        if index > 0 && digit != previous_digit {
            if captured(&input[start..index], formatting) {
                tokens.push(&input[start..index]);
            }
            start = index;
        }
        previous_digit = digit;
    }
    if captured(&input[start..], formatting) {
        tokens.push(&input[start..]);
    }
}

fn known_word(locale: &Locale, input: &str) -> Option<(usize, usize)> {
    if input.trim().is_empty() {
        return None;
    }
    let mut earliest = None;
    let matcher = locale.known_word_matcher();
    let mut inline_seen = [0_u64; 4];
    let mut extended_seen;
    let seen = if matcher.patterns_len() <= inline_seen.len() * 64 {
        &mut inline_seen[..]
    } else {
        extended_seen = vec![0_u64; matcher.patterns_len().div_ceil(64)];
        &mut extended_seen[..]
    };
    for matched in matcher.find_overlapping_iter(input) {
        let priority = matched.pattern().as_usize();
        let mask = 1_u64 << (priority % 64);
        let entry = &mut seen[priority / 64];
        if *entry & mask != 0 {
            continue;
        }
        *entry |= mask;
        let start = matched.start();
        let end = matched.end();
        let allowed = |neighbor: Option<char>| {
            neighbor.is_none_or(|character| !text::is_letter_or_mark(character))
        };
        let boundary_matches = locale.no_word_spacing
            || (allowed(input[..start].chars().next_back())
                && allowed(input[end..].chars().next()));
        if boundary_matches
            && earliest.is_none_or(|(previous, previous_priority, _)| {
                (start, priority) < (previous, previous_priority)
            })
        {
            earliest = Some((start, priority, end));
        }
    }
    earliest.map(|(start, _, end)| (start, end))
}

fn split_known<'input>(
    locale: &Locale,
    mut input: &'input str,
    formatting: bool,
    tokens: &mut Vec<&'input str>,
) {
    loop {
        if input.len() <= 1 {
            if captured(input, formatting) {
                tokens.push(input);
            }
            break;
        }
        let Some((start, end)) = known_word(locale, input) else {
            if captured(input, formatting) {
                split_numerals(input, formatting, tokens);
            }
            break;
        };
        if start > 0 && captured(&input[..start], formatting) {
            split_numerals(&input[..start], formatting, tokens);
        }
        if captured(&input[start..end], formatting) {
            tokens.push(&input[start..end]);
        }
        input = &input[end..];
        if input.is_empty() {
            break;
        }
    }
}

pub(crate) fn split(
    locale: &Locale,
    input: &str,
    formatting: bool,
    skipped: &HashSet<&str>,
) -> Vec<String> {
    if !input.is_empty() && input.bytes().all(|byte| byte.is_ascii_digit()) {
        return if skipped.contains(input) {
            Vec::new()
        } else {
            vec![input.to_owned()]
        };
    }
    split_general(locale, input, formatting, skipped)
}

fn split_general(
    locale: &Locale,
    input: &str,
    formatting: bool,
    skipped: &HashSet<&str>,
) -> Vec<String> {
    with_split_tokens(locale, input, formatting, skipped, |tokens| {
        tokens.into_iter().map(String::from).collect()
    })
}

fn with_split_tokens<Output>(
    locale: &Locale,
    input: &str,
    formatting: bool,
    skipped: &HashSet<&str>,
    consume: impl FnOnce(Vec<&str>) -> Output,
) -> Output {
    let mut separated = String::new();
    let mut remaining = input;
    if locale.combined >= 0 && input.bytes().any(|byte| byte.is_ascii_digit()) {
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
    let separated = if remaining.len() == input.len() {
        std::borrow::Cow::Borrowed(input)
    } else {
        separated.push_str(remaining);
        std::borrow::Cow::Owned(separated)
    };
    let mut tokens = Vec::new();
    for segment in separated.split(SEPARATOR) {
        if locale.exact_match(segment) {
            tokens.push(segment);
        } else {
            split_known(locale, segment, formatting, &mut tokens);
        }
    }
    filter_tokens(&mut tokens, 0, skipped);
    consume(tokens)
}

fn filter_tokens(tokens: &mut Vec<&str>, start: usize, skipped: &HashSet<&str>) {
    let mut retained = start;
    for index in start..tokens.len() {
        let token = tokens[index];
        if token.is_empty() {
            continue;
        }
        if retained > start
            && text::is_number_only(tokens[retained - 1])
            && matches!(token, "st" | "nd" | "rd" | "th")
        {
            continue;
        }
        if !skipped.contains(token.trim()) {
            tokens[retained] = token;
            retained += 1;
        }
    }
    tokens.truncate(retained);
}

fn trim_unknown<'tokens, Token: AsRef<str>>(
    locale: &Locale,
    tokens: &'tokens [Token],
) -> &'tokens [Token] {
    let extra = |token: &str| {
        token.trim().is_empty()
            || (!text::is_number_only(token)
                && !locale.contains(token)
                && !locale.exact_match(token))
    };
    let start = tokens
        .iter()
        .position(|token| !extra(token.as_ref()))
        .unwrap_or(tokens.len());
    let mut end = tokens.len();
    while end > start
        && extra(tokens[end - 1].as_ref())
        && !crate::timezone::is_token(tokens[end - 1].as_ref())
    {
        end -= 1;
    }
    &tokens[start..end]
}

#[cfg(test)]
pub(crate) fn applicable(
    configuration: &Configuration,
    locale: &Locale,
    input: &str,
    ignore_surrounding: bool,
) -> bool {
    applicable_prepared(
        configuration,
        locale,
        &text::normalize_digits(&text::normalize(input)),
        ignore_surrounding,
    )
}

pub(crate) fn applicable_prepared(
    configuration: &Configuration,
    locale: &Locale,
    input: &str,
    ignore_surrounding: bool,
) -> bool {
    let input = simplify(locale, input);
    let skipped = skipped_tokens(configuration, locale);
    if !input.is_empty() && input.bytes().all(|byte| byte.is_ascii_digit()) {
        return !skipped.contains(input.as_ref());
    }
    with_split_tokens(locale, &input, false, &skipped, |tokens| {
        let tokens = if ignore_surrounding {
            trim_unknown(locale, &tokens)
        } else {
            &tokens
        };
        !tokens.iter().all(|token| always_kept(token))
            && tokens.iter().all(|token| {
                text::is_number_only(token)
                    || locale.contains(token)
                    || skipped.contains(*token)
                    || locale.exact_match(token)
            })
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

fn clear_future_words(tokens: &mut [String]) {
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
}

pub(crate) fn translate(
    configuration: &Configuration,
    locale: &Locale,
    input: &str,
    formatting: bool,
    ignore_surrounding: bool,
) -> Vec<String> {
    let skipped = skipped_tokens(configuration, locale);
    let input = text::normalize_digits(
        &text::normalize_unicode(input)
            .chars()
            .flat_map(char::to_lowercase)
            .collect::<String>(),
    );
    let input = simplify(locale, &input);
    let tokens = split(locale, &input, formatting, &skipped);
    let tokens = if ignore_surrounding {
        trim_unknown(locale, &tokens).to_vec()
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
            clear_future_words(&mut tokens);
            join(&remove_empty(&tokens), formatting)
        })
        .collect()
}

pub(crate) fn split_sentence<'input>(locale: &Locale, input: &'input str) -> Vec<&'input str> {
    let has_delimiter = match locale.sentence_splitter_group {
        2 => input.contains([
            '.', '!', '?', ';', '\u{2026}', '\r', '\n', '\u{a1}', '\u{bf}',
        ]),
        3 => input.contains(['|', '!', '?', ';', '\r', '\n']),
        4 => input.contains([
            '\u{3002}', '\u{2026}', '\u{2025}', '.', '!', '?', '\u{ff1f}', '\u{ff01}', ';', '\r',
            '\n',
        ]),
        5 => input.contains(['\r', '\n']),
        6 => input.contains(['\r', '\n', '\u{61f}', '!', '.', '\u{2026}']),
        _ => input.contains(['.', '!', '?', ';', '\u{2026}', '\r', '\n']),
    };
    if !has_delimiter {
        let input = input.trim();
        return if input.is_empty() {
            Vec::new()
        } else {
            vec![input]
        };
    }
    split_sentence_general(locale, input)
}

fn split_sentence_general<'input>(locale: &Locale, input: &'input str) -> Vec<&'input str> {
    static SPLITTERS: OnceLock<[Regex; 6]> = OnceLock::new();
    let expressions = SPLITTERS.get_or_init(|| [
        r"([^\t\n\r\x0c .]*)[.!?;\x{2026}\r\n]+(?:[\t\n\r\x0c ]|$)*",
        r"([^\t\n\r\x0c .]*)[.!?;\x{2026}\r\n]+([\t\n\r\x0c ]*[\x{a1}\x{bf}]*|$)|[\x{a1}\x{bf}]+",
        r"([^\t\n\r\x0c .]*)[|!?;\r\n]+(?:[\t\n\r\x0c ]|$)+",
        r"([^\t\n\r\x0c .]*)[\x{3002}\x{2026}\x{2025}.!?\x{ff1f}\x{ff01};\r\n]+(?:[\t\n\r\x0c ]|$)+",
        r"([^\t\n\r\x0c .]*)[\r\n]+",
        r"([^\t\n\r\x0c .]*)[\r\n\x{61f}!.\x{2026}]+(?:[\t\n\r\x0c ]|$)+",
    ].map(|pattern| Regex::new(pattern).unwrap()));
    let group = match locale.sentence_splitter_group {
        1..=6 => locale.sentence_splitter_group - 1,
        _ => 0,
    };
    let mut sentences = Vec::new();
    let mut last = 0;
    for captures in expressions[group].captures_iter(input) {
        let Some(ending) = captures.get(1) else {
            continue;
        };
        if locale
            .abbreviations
            .as_ref()
            .is_some_and(|words| words.iter().any(|word| word == ending.as_str()))
            || (matches!(locale.name.as_str(), "fi" | "cs" | "hu" | "de" | "da")
                && text::is_number_only(ending.as_str()))
        {
            continue;
        }
        let sentence = input[last..ending.end()].trim();
        if !sentence.is_empty() {
            sentences.push(sentence);
        }
        last = captures.get(0).unwrap().end();
    }
    let sentence = input[last..].trim();
    if !sentence.is_empty() {
        sentences.push(sentence);
    }
    sentences
}

fn simple_split<'input>(
    locale: &Locale,
    input: &'input str,
    formatting: bool,
    skipped: &HashSet<&str>,
) -> Vec<&'input str> {
    let split_run = |input: &'input str, digit, result: &mut Vec<&'input str>| {
        if digit {
            if !skipped.contains(input) {
                result.push(input);
            }
        } else {
            let start = result.len();
            for segment in input.split(SEPARATOR) {
                split_known(locale, segment, formatting, result);
            }
            filter_tokens(result, start, skipped);
        }
    };
    let mut result = Vec::new();
    let mut start = 0;
    let mut digit = input.starts_with(|character: char| character.is_ascii_digit());
    for (index, character) in input.char_indices() {
        if character.is_ascii_digit() != digit {
            split_run(&input[start..index], digit, &mut result);
            start = index;
            digit = character.is_ascii_digit();
        }
    }
    split_run(&input[start..], digit, &mut result);
    result
}

fn search_word_split(locale: &Locale, input: &str, skipped: &HashSet<&str>) -> Vec<String> {
    if locale.no_word_spacing {
        simple_split(locale, input, true, skipped)
            .into_iter()
            .map(String::from)
            .collect()
    } else {
        input.split_whitespace().map(String::from).collect()
    }
}

fn simplify_split_align(
    locale: &Locale,
    input: &str,
    skipped: &HashSet<&str>,
) -> (Vec<String>, Vec<String>) {
    let normalize = |input: &str| text::normalize_digits(&text::normalize(input));
    let mut originals = search_word_split(locale, input, skipped);
    let mut simplified = search_word_split(locale, &simplify(locale, &normalize(input)), skipped);
    let mut add_empty = false;
    if originals.len() < simplified.len() {
        for (index, token) in simplified.iter().enumerate() {
            if index >= originals.len() {
                originals.push(String::new());
            } else if *token == normalize(&originals[index]) {
                add_empty = false;
            } else if !add_empty {
                add_empty = true;
            } else {
                originals.insert(index, String::new());
            }
        }
    } else if originals.len() > simplified.len() {
        for (index, token) in originals.iter().enumerate() {
            if index >= simplified.len() {
                simplified.push(String::new());
            } else if normalize(token) == simplified[index] {
                add_empty = false;
            } else if !add_empty {
                add_empty = true;
            } else {
                simplified.insert(index, String::new());
            }
        }
    }
    while originals.len() != simplified.len() {
        let longer = if originals.len() > simplified.len() {
            &mut originals
        } else {
            &mut simplified
        };
        let Some(index) = longer.iter().position(String::is_empty) else {
            break;
        };
        longer.remove(index);
    }
    (originals, simplified)
}

fn join_chunk(locale: &Locale, tokens: &[String]) -> String {
    if locale.no_word_spacing {
        join(tokens, true)
    } else {
        static SPACES: OnceLock<Regex> = OnceLock::new();
        SPACES
            .get_or_init(|| Regex::new(r"[\t\n\r\x0c ]{2,}").unwrap())
            .replace_all(&tokens.join(" "), " ")
            .into_owned()
    }
}

fn translate_word(locale: &Locale, word: &str) -> Option<Vec<String>> {
    locale
        .relative_type
        .get(word)
        .map(|value| vec![value.clone()])
        .or_else(|| locale.translations.get(word).cloned())
}

pub(crate) fn translate_search(
    configuration: &Configuration,
    locale: &Locale,
    input: &str,
) -> (Vec<String>, Vec<String>) {
    let skipped = skipped_tokens(configuration, locale);
    let base_language = locale.name.split('-').next().unwrap_or(&locale.name);
    let mut translated = Vec::new();
    let mut original = Vec::new();
    let mut flush = |chunks: &mut Vec<Vec<String>>, originals: &mut Vec<String>| {
        if chunks.is_empty() {
            return;
        }
        let mut permutations = vec![Vec::new()];
        for translations in chunks.drain(..) {
            permutations = permutations
                .into_iter()
                .flat_map(|tokens| {
                    translations.iter().map(move |translation| {
                        let mut tokens = tokens.clone();
                        tokens.push(translation.clone());
                        tokens
                    })
                })
                .collect();
        }
        let original_text = join_chunk(
            locale,
            &originals
                .iter()
                .filter(|token| !token.is_empty())
                .cloned()
                .collect::<Vec<_>>(),
        );
        for mut tokens in permutations {
            clear_future_words(&mut tokens);
            tokens.retain(|token| !token.is_empty());
            translated.push(join_chunk(locale, &tokens));
            original.push(original_text.clone());
        }
        originals.clear();
    };
    for sentence in split_sentence(locale, input) {
        let (original_tokens, tokens) = simplify_split_align(locale, sentence, &skipped);
        let mut chunks = Vec::new();
        let mut originals = Vec::new();
        let mut index = 0;
        while index < tokens.len().min(original_tokens.len()) {
            let word = &tokens[index];
            let joined = join_chunk(
                locale,
                &[
                    word.clone(),
                    tokens.get(index + 1).cloned().unwrap_or_default(),
                ],
            );
            let cleaned = word.trim_matches(|character| "()\"'{}[],.\u{60c}".contains(character));
            let dash = matches!(
                word.as_str(),
                "-" | "\u{2014}\u{2014}" | "\u{2014}" | "\u{ff5e}"
            );
            let mut original_word = original_tokens[index].clone();
            let translations = if word.is_empty() || word == " " {
                Some(vec![word.clone()])
            } else if index + 1 < original_tokens.len()
                && index + 1 < tokens.len()
                && locale.contains(&joined)
                && !dash
                && !matches!(base_language, "zh" | "ja")
            {
                original_word =
                    join_chunk(locale, &[original_word, original_tokens[index + 1].clone()]);
                index += 1;
                translate_word(locale, &joined)
            } else if locale.contains(word) && !dash {
                translate_word(locale, word)
            } else if locale.contains(cleaned) && !dash {
                let punctuation = word.get(cleaned.len()..).unwrap_or("");
                translate_word(locale, cleaned).map(|values| {
                    values
                        .into_iter()
                        .map(|value| value + punctuation)
                        .collect()
                })
            } else if word.chars().any(|character| {
                character.is_ascii_digit() || (locale.no_word_spacing && ".:-/".contains(character))
            }) || (!chunks.is_empty()
                && crate::timezone::word_is_timezone(&original_word))
            {
                Some(vec![word.clone()])
            } else {
                None
            };
            if let Some(translations) = translations {
                chunks.push(translations);
                originals.push(original_word);
            } else {
                flush(&mut chunks, &mut originals);
            }
            index += 1;
        }
        flush(&mut chunks, &mut originals);
    }
    (translated, original)
}

pub(crate) type UniqueCharsets = HashMap<String, HashSet<char>>;

pub(crate) fn unique_charsets(languages: &[String]) -> UniqueCharsets {
    let charsets: UniqueCharsets = languages
        .iter()
        .filter_map(|name| {
            data()
                .get(name)
                .map(|locale| (name.clone(), locale.charset.chars().collect()))
        })
        .collect();
    charsets
        .iter()
        .map(|(name, characters)| {
            let unique = characters
                .iter()
                .copied()
                .filter(|character| {
                    charsets
                        .iter()
                        .all(|(other, characters)| other == name || !characters.contains(character))
                })
                .collect();
            (name.clone(), unique)
        })
        .collect()
}

fn count_applicability(skipped: &HashSet<&str>, locale: &Locale, input: &str) -> (usize, usize) {
    let input = simplify(locale, input);
    let mut tokens: Vec<_> = split_sentence(locale, &input)
        .into_iter()
        .flat_map(|sentence| simple_split(locale, sentence, false, skipped))
        .collect();
    if tokens.len() <= 32 {
        tokens.sort_unstable();
        tokens.dedup();
    } else {
        let mut seen = HashSet::with_capacity(tokens.len());
        tokens.retain(|token| seen.insert(*token));
    }
    let mut words = 0;
    let mut skips = 0;
    let dictionary = locale.applicability_dictionary();
    for token in tokens {
        if let Some(meaningful) = (token.len() >= 2 && token.chars().nth(1).is_some())
            .then(|| dictionary.get(token))
            .flatten()
        {
            if *meaningful {
                words += 1;
            } else {
                skips += 1;
            }
        } else if text::is_number_only(token) {
            skips += 1;
        }
    }
    (words, skips)
}

pub(crate) fn detect_full_text(
    configuration: &Configuration,
    input: &str,
    languages: &[String],
    unique: &UniqueCharsets,
) -> Result<String, Error> {
    let characters: HashSet<_> = text::normalize_charset(input).chars().collect();
    if characters
        .iter()
        .all(|character| "0123456789()\\-/.,:' ".contains(*character))
    {
        return languages.first().cloned().ok_or(Error::LanguageDetection);
    }
    for language in languages {
        if unique.get(language).is_some_and(|charset| {
            charset
                .iter()
                .any(|character| characters.contains(character))
        }) {
            return Ok(language.clone());
        }
    }
    let candidates: Vec<_> = languages
        .iter()
        .filter(|name| {
            data().get(name).is_some_and(|locale| {
                locale
                    .charset
                    .chars()
                    .any(|character| characters.contains(&character))
            })
        })
        .collect();
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    let input = text::normalize(input);
    let skipped = configuration
        .skip_tokens
        .iter()
        .map(String::as_str)
        .collect();
    let mut best = None;
    let mut without_timezone = None;
    for candidate in candidates {
        let locale = data().get(candidate).unwrap();
        let mut score = count_applicability(&skipped, locale, &input);
        if score == (0, 0) {
            let cleaned =
                without_timezone.get_or_insert_with(|| crate::timezone::pop_offset(&input).0);
            if *cleaned != input {
                score = count_applicability(&skipped, locale, cleaned);
            }
        }
        if score != (0, 0) && best.as_ref().is_none_or(|(_, previous)| score > *previous) {
            best = Some((candidate.clone(), score));
        }
    }
    best.map(|(name, _)| name)
        .or_else(|| configuration.default_languages.first().cloned())
        .ok_or(Error::LanguageDetection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_text_scoring_does_not_count_custom_skips_as_language_evidence() {
        let configuration = Configuration {
            skip_tokens: vec!["xyzz".into()],
            ..Configuration::default()
        };
        let skipped = configuration
            .skip_tokens
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(
            count_applicability(&skipped, data().get("en").unwrap(), "xyzz"),
            (0, 0)
        );
        assert_eq!(
            count_applicability(&skipped, data().get("en").unwrap(), "March 2014"),
            (0, 1)
        );
        assert_eq!(
            count_applicability(&skipped, data().get("en").unwrap(), "march 2014"),
            (1, 1)
        );
        for (code, input, expected) in [
            ("en", "meeting on 2024-02-29", (0, 4)),
            ("nn", "meeting on 2024-02-29", (1, 3)),
            ("fr", "la reunion a eu lieu le 12 mars 2024.", (2, 2)),
            ("nn", "la reunion a eu lieu le 12 mars 2024.", (2, 2)),
        ] {
            assert_eq!(
                count_applicability(&skipped, data().get(code).unwrap(), input),
                expected,
                "{code} {input:?}"
            );
        }
    }

    #[test]
    fn sentence_delimiter_guard_matches_all_locale_rules() {
        let inputs = [
            "",
            "  meeting on 2024-02-29  ",
            "29. Juni 2007. Morgen!",
            "Dr. Smith at 12:30.",
            "first|second; third? fourth!",
            "uno\u{a1} dos\u{bf} tres",
            "first\rsecond\nthird",
            "first\u{2026} second\u{2025} third",
            "first\u{3002} second\u{ff1f} third\u{ff01}",
            "first\u{61f} second",
            "20.12.2024",
            ".!?;|\n\r",
            " ",
        ];
        for locale in &data().locales {
            for input in inputs {
                assert_eq!(
                    split_sentence(locale, input),
                    split_sentence_general(locale, input),
                    "{} {input:?}",
                    locale.name
                );
            }
        }
    }

    #[test]
    fn search_translation_retains_go_text_and_sentence_rules() {
        let configuration = Configuration::default().initialized().unwrap();
        for (code, input, expected) in [
            ("en", "Sep 03 2014", "september 03 2014"),
            (
                "en",
                "Aug 06, 2018 05:05 PM CDT",
                "august 06, 2018 05:05 pm cdt",
            ),
            ("fr", "20 F\u{e9}vrier 2012", "20 february 2012"),
            ("zh", "2013\u{5e74}04\u{6708}08\u{65e5}", "2013-04-08"),
        ] {
            let (translation, original) =
                translate_search(&configuration, data().get(code).unwrap(), input);
            assert_eq!(translation[0], expected, "{code}: {input}");
            assert_eq!(original[0], input, "{code}: {input}");
            assert_eq!(translation.len(), original.len());
        }
        assert_eq!(
            split_sentence(data().get("de").unwrap(), "29. Juni 2007. Morgen!"),
            ["29. Juni 2007. Morgen"]
        );
        assert_eq!(
            split_sentence(data().get("de").unwrap(), "29. Juni 2007 endet. Morgen!"),
            ["29. Juni 2007 endet", "Morgen"]
        );
    }

    #[test]
    fn ascii_digit_runs_are_single_tokens_in_every_locale() {
        let mut inputs: Vec<_> = (0..=40).map(|number| number.to_string()).collect();
        inputs.extend(
            [
                "99",
                "123",
                "2024",
                "0000",
                "123456789012345678901234567890",
            ]
            .map(String::from),
        );
        for locale in &data().locales {
            for formatting in [false, true] {
                for input in &inputs {
                    assert_eq!(
                        split(locale, input, formatting, &HashSet::new()),
                        std::slice::from_ref(input),
                        "{} {input} {formatting}",
                        locale.name
                    );
                    assert_eq!(
                        split_general(locale, input, formatting, &HashSet::new()),
                        std::slice::from_ref(input),
                        "{} {input} general",
                        locale.name
                    );
                    assert!(
                        split(locale, input, formatting, &HashSet::from([input.as_str()]))
                            .is_empty(),
                        "{} {input} skipped",
                        locale.name
                    );
                    assert!(
                        split_general(locale, input, formatting, &HashSet::from([input.as_str()]))
                            .is_empty(),
                        "{} {input} general skipped",
                        locale.name
                    );
                }
            }
        }
    }

    #[test]
    fn borrowed_simple_split_matches_general_splitting_in_every_locale() {
        let inputs = [
            "meeting on 2024-02-29",
            "12th March 2024 at 08:30",
            "t12t",
            "3|#=#=#|days ago",
            "past 12 months",
            "20\u{e9}24\u{0662}",
            "2013\u{5e74}04\u{6708}08\u{65e5}",
            "",
        ];
        for locale in &data().locales {
            for formatting in [false, true] {
                for skipped in [HashSet::new(), HashSet::from(["t", "on", "12"])] {
                    for input in inputs {
                        let mut expected = Vec::new();
                        let mut start = 0;
                        let mut digit =
                            input.starts_with(|character: char| character.is_ascii_digit());
                        for (index, character) in input.char_indices() {
                            if character.is_ascii_digit() != digit {
                                expected.extend(split_general(
                                    locale,
                                    &input[start..index],
                                    formatting,
                                    &skipped,
                                ));
                                start = index;
                                digit = character.is_ascii_digit();
                            }
                        }
                        expected.extend(split_general(
                            locale,
                            &input[start..],
                            formatting,
                            &skipped,
                        ));
                        assert_eq!(
                            simple_split(locale, input, formatting, &skipped),
                            expected,
                            "{} {input:?} {formatting}",
                            locale.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn known_word_matcher_preserves_first_occurrences_and_priority() {
        for (words, input, no_word_spacing, expected) in [
            (vec!["may", "mayday"], "mayday", true, Some((0, 3))),
            (vec!["mayday", "may"], "mayday", true, Some((0, 6))),
            (vec!["may", "mayday"], "mayday", false, Some((0, 6))),
            (vec!["abcd", "b"], "abcd", true, Some((0, 4))),
            (vec!["may"], "maybe may", false, None),
            (vec!["may"], "\u{e9}may may", false, None),
            (vec!["may"], "may\u{301} may", false, None),
            (vec!["may"], "1may_", false, Some((1, 4))),
            (vec!["", "may"], "may", false, Some((0, 3))),
            (vec![], "may", false, None),
            (vec!["may"], "   ", false, None),
            (
                vec!["\u{6708}", "\u{5e74}"],
                "2013\u{5e74}04\u{6708}08\u{65e5}",
                true,
                Some((4, 7)),
            ),
        ] {
            let locale: Locale = serde_json::from_value(serde_json::json!({
                "name": "test",
                "date_order": "DMY",
                "no_word_spacing": no_word_spacing,
                "simplifications": [],
                "translations": {},
                "relative_type": {},
                "relative_type_regexes": [],
                "combined": -1,
                "exact_combined": -1,
                "known_words": words,
            }))
            .unwrap();
            assert_eq!(
                known_word(&locale, input),
                expected,
                "{input:?} {:?}",
                locale.known_words
            );
        }
    }

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
