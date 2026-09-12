#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Digit,
    Letter,
    Other,
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub(crate) text: String,
    pub(crate) kind: Kind,
}

fn kind(character: char) -> Kind {
    if character.is_ascii_digit() || character == ':' {
        Kind::Digit
    } else if character.is_ascii_alphabetic() {
        Kind::Letter
    } else {
        Kind::Other
    }
}

pub(crate) fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    for character in input.chars() {
        let current_kind = kind(character);
        if let Some(last) = tokens.last_mut().filter(|last| last.kind == current_kind) {
            last.text.push(character);
        } else {
            tokens.push(Token {
                text: character.to_string(),
                kind: current_kind,
            });
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_go_ascii_classes() {
        for (input, expected) in [
            ("11 april 2010", vec!["11", " ", "april", " ", "2010"]),
            ("11/12-2013", vec!["11", "/", "12", "-", "2013"]),
            ("10:30:35 PM", vec!["10:30:35", " ", "PM"]),
            ("18:50", vec!["18:50"]),
            (
                "December 23, 2010, 16:50 pm",
                vec![
                    "December", " ", "23", ", ", "2010", ", ", "16:50", " ", "pm",
                ],
            ),
            ("\u{0661}\u{00e9}:30", vec!["\u{0661}\u{00e9}", ":30"]),
        ] {
            let tokens = tokenize(input);
            assert_eq!(
                tokens
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
        assert_eq!(tokenize("\u{0661}\u{00e9}")[0].kind, Kind::Other);
        assert_eq!(tokenize(":")[0].kind, Kind::Digit);
    }
}
