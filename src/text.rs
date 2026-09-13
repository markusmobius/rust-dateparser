use std::sync::OnceLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

pub(crate) fn normalize_unicode(input: &str) -> String {
    if input.is_ascii() {
        return input.to_owned();
    }
    static NONSPACING: OnceLock<Regex> = OnceLock::new();
    let expression = NONSPACING.get_or_init(|| Regex::new(r"\p{Mn}+").unwrap());
    let decomposed: String = input.nfkd().collect();
    expression.replace_all(&decomposed, "").nfkc().collect()
}

pub(crate) fn normalize(input: &str) -> String {
    if input.is_ascii() {
        let mut normalized = sanitize_spaces(input);
        normalized.make_ascii_lowercase();
        return normalized;
    }
    sanitize_spaces(
        &normalize_unicode(input)
            .chars()
            .flat_map(char::to_lowercase)
            .collect::<String>(),
    )
}

pub(crate) fn normalize_charset(input: &str) -> String {
    if input.is_ascii() {
        let mut normalized = input.replace('.', "");
        normalized.make_ascii_lowercase();
        return sanitize_spaces(&normalized);
    }
    let normalized: String = input.nfkd().nfkc().collect();
    sanitize_spaces(
        &normalize_apostrophe(&normalized)
            .replace('.', "")
            .to_lowercase(),
    )
}

pub(crate) fn normalize_digits(input: &str) -> String {
    if input.is_ascii() {
        return input.to_owned();
    }
    let mappings = &crate::locale::data().digit_mappings;
    input
        .chars()
        .map(|character| {
            if character.is_ascii() {
                return character;
            }
            mappings
                .binary_search_by_key(&(character as u32), |mapping| mapping[0])
                .ok()
                .and_then(|index| char::from_u32(mappings[index][1]))
                .unwrap_or(character)
        })
        .collect()
}

pub(crate) fn is_digit(character: char) -> bool {
    static DIGIT: OnceLock<Regex> = OnceLock::new();
    if character.is_ascii() {
        return character.is_ascii_digit();
    }
    DIGIT
        .get_or_init(|| Regex::new(r"\p{Nd}").unwrap())
        .is_match(character.encode_utf8(&mut [0; 4]))
}

pub(crate) fn is_number_only(input: &str) -> bool {
    input.chars().all(is_digit)
}

pub(crate) fn is_letter(character: char) -> bool {
    static LETTER: OnceLock<Regex> = OnceLock::new();
    if character.is_ascii() {
        return character.is_ascii_alphabetic();
    }
    LETTER
        .get_or_init(|| Regex::new(r"\p{L}").unwrap())
        .is_match(character.encode_utf8(&mut [0; 4]))
}

pub(crate) fn is_letter_or_mark(character: char) -> bool {
    static LETTER_MARK: OnceLock<Regex> = OnceLock::new();
    if character.is_ascii() {
        return character.is_ascii_alphabetic();
    }
    LETTER_MARK
        .get_or_init(|| Regex::new(r"[\p{L}\p{M}]").unwrap())
        .is_match(character.encode_utf8(&mut [0; 4]))
}

pub(crate) fn sanitize_spaces(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    let mut pending_space = false;
    for character in input.chars().filter(|character| *character != '\u{a0}') {
        if character.is_whitespace() {
            pending_space = !sanitized.is_empty();
        } else {
            if pending_space {
                sanitized.push(' ');
                pending_space = false;
            }
            sanitized.push(character);
        }
    }
    sanitized
}

pub(crate) fn strip_braces(input: &str) -> String {
    input
        .chars()
        .filter(|character| !matches!(character, '{' | '}' | '(' | ')' | '<' | '>' | '[' | ']'))
        .collect()
}

pub(crate) fn normalize_apostrophe(input: &str) -> String {
    input
        .chars()
        .map(|character| match character {
            '\u{2019}' | '\u{02bc}' | '\u{02bb}' | '\u{055a}' | '\u{a78c}' | '\u{2032}'
            | '\u{2035}' | '\u{02b9}' | '\u{ff07}' => '\'',
            _ => character,
        })
        .collect()
}

pub(crate) fn sanitize_date(input: &str) -> String {
    if input.is_ascii() && !input.contains(['.', ':']) {
        return sanitize_spaces(input);
    }
    sanitize_date_general(input)
}

fn sanitize_date_general(input: &str) -> String {
    static EXPRESSIONS: OnceLock<[Regex; 5]> = OnceLock::new();
    let expressions = EXPRESSIONS.get_or_init(|| [
        r"\t|\n|\r|\x{00bb}|,[\t\n\r\x0c ]\x{0432}(?-u:\b)|\x{200e}|\x{b7}|\x{200f}|\x{064e}|\x{064f}",
        r"(?i)([\P{L}\p{N}])\x{0433}\.",
        r"(?i)(\p{N}+)\.[\t\n\r\x0c ]?(\p{N}+)\.[\t\n\r\x0c ]?(\p{N}+)\.( u)?",
        r"(?i)([^\p{N}.\t\n\r\x0c ])\.",
        r"(?i)^.*?on:[\t\n\r\x0c ]+(.*)",
    ].map(|pattern| Regex::new(pattern).unwrap()));
    let input = expressions[0].replace_all(input, " ");
    let input = expressions[1].replace_all(&input, "$1 ");
    let input = expressions[2].replace_all(&input, "$1.$2.$3 ");
    let input = sanitize_spaces(&input);
    let input = expressions[3].replace_all(&input, "$1");
    let input = expressions[4].replace_all(&input, "$1");
    normalize_apostrophe(input.strip_suffix(':').unwrap_or(&input))
        .trim()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_pass_spaces_match_replace_split_join() {
        let characters = (0_u8..=127).map(char::from).chain([
            '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}',
            '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}',
            '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
        ]);
        for character in characters {
            for input in [
                format!(
                    "{character}20\u{a0}24{character}February{character}{character}29{character}"
                ),
                format!("{character}\u{a0}{character}"),
                format!("a{character}\u{a0}{character}b"),
            ] {
                let expected = input
                    .replace('\u{a0}', "")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                assert_eq!(sanitize_spaces(&input), expected, "{input:?}");
            }
        }
    }

    #[test]
    fn ascii_normalization_matches_unicode_rules() {
        assert!(crate::locale::data()
            .digit_mappings
            .iter()
            .all(|mapping| mapping[0] > 127));
        let all_ascii: String = (0_u8..=127).map(char::from).collect();
        let nonspacing = Regex::new(r"\p{Mn}+").unwrap();
        for input in [
            all_ascii.as_str(),
            " Meeting ON 2024-02-29. ",
            "\t12:30\nUTC\r",
            "",
        ] {
            let decomposed: String = input.nfkd().collect();
            let unmarked = nonspacing
                .replace_all(&decomposed, "")
                .nfkc()
                .collect::<String>();
            assert_eq!(normalize_unicode(input), unmarked);
            assert_eq!(
                normalize(input),
                sanitize_spaces(
                    &unmarked
                        .chars()
                        .flat_map(char::to_lowercase)
                        .collect::<String>()
                )
            );
            let charset: String = input.nfkd().nfkc().collect();
            assert_eq!(
                normalize_charset(input),
                sanitize_spaces(
                    &normalize_apostrophe(&charset)
                        .replace('.', "")
                        .to_lowercase()
                )
            );
            assert_eq!(normalize_digits(input), input);
        }
    }

    #[test]
    fn ascii_sanitization_matches_general_rules() {
        for character in (0_u8..=127).map(char::from) {
            for input in [
                character.to_string(),
                format!(" before{character}next 12 months{character}after "),
                format!("Published on:{character}2024.02.29."),
                format!("{character}Aug. 03, 2014{character}:"),
            ] {
                assert_eq!(
                    sanitize_date(&input),
                    sanitize_date_general(&input),
                    "{input:?}"
                );
            }
        }
    }

    #[test]
    fn cleaning_preserves_go_nbsp_removal_and_brace_contents() {
        assert_eq!(
            sanitize_spaces(" 20\u{a0}24\t February\n29 "),
            "2024 February 29"
        );
        assert_eq!(
            strip_braces("(1 day) [ago] {12:00} <UTC>"),
            "1 day ago 12:00 UTC"
        );
    }

    #[test]
    fn normalization_retains_go_digit_and_mark_semantics() {
        assert_eq!(
            normalize(" F\u{e9}vrier\t\u{ff12}\u{ff10}\u{ff12}\u{ff14} "),
            "fevrier 2024"
        );
        assert_eq!(normalize_digits("\u{0662}\u{0660}\u{0662}\u{0664}"), "2024");
        assert_eq!(
            normalize_unicode("\u{0915}\u{093e}\u{0301}"),
            "\u{0915}\u{093e}"
        );
    }
}
