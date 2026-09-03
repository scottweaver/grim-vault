// Vendored from tq-univault crates/univault-core/src/stats/format.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Text plumbing for a stat display engine: the game's `%`-style
//! format specs (ported from `TQVaultAE`'s
//! `ItemAttributeProvider.ConvertFormat` + `ItemProvider.Format`),
//! `{^X}` color-tag handling, word wrap, and the arithmetic
//! evaluator for `itemcost.dbr` requirement equations. Both games use
//! the same `{%.0f0}` language and colour letters.

/// One parsed placeholder or literal run of a format string.
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Literal(String),
    Arg {
        index: usize,
        decimals: Option<u8>,
        sign: bool,
    },
}

/// A parsed format string, ready to render with [`FormatSpec::format`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FormatSpec {
    pieces: Vec<Piece>,
}

/// A value handed to [`FormatSpec::format`].
#[derive(Debug, Clone)]
pub enum Arg {
    Number(f32),
    Text(String),
}

impl FormatSpec {
    /// Whether the spec contains any placeholder — the reference's
    /// `label.IndexOf('{') >= 0` test after conversion.
    #[must_use]
    pub fn has_args(&self) -> bool {
        self.pieces
            .iter()
            .any(|piece| matches!(piece, Piece::Arg { .. }))
    }

    /// Renders the spec; placeholders whose index has no argument
    /// vanish, as in the reference.
    #[must_use]
    pub fn format(&self, args: &[Arg]) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Literal(text) => out.push_str(text),
                Piece::Arg {
                    index,
                    decimals,
                    sign,
                } => {
                    let Some(arg) = args.get(*index) else {
                        continue;
                    };
                    match arg {
                        Arg::Text(text) => out.push_str(text),
                        Arg::Number(value) => push_number(&mut out, *value, *decimals, *sign),
                    }
                }
            }
        }
        out
    }
}

fn push_number(out: &mut String, value: f32, decimals: Option<u8>, sign: bool) {
    use std::fmt::Write as _;

    // The reference strips the forced sign when it would double up on
    // negatives.
    if sign && value >= 0.0 {
        out.push('+');
    }
    match decimals {
        Some(places) => {
            let _ = write!(out, "{value:.places$}", places = usize::from(places));
        }
        None if value.fract() == 0.0 => {
            // A whole-valued f32 converts exactly; the reference casts too.
            #[allow(clippy::cast_possible_truncation)]
            let whole = value as i64;
            let _ = write!(out, "{whole}");
        }
        None => {
            let _ = write!(out, "{value}");
        }
    }
}

/// Parses a format string: `%`-specs become placeholders, braces
/// are grouping noise and vanish, `{^N}` becomes a newline, and all
/// other color tags are dropped (lines are single-colored here).
#[must_use]
pub fn parse_format(raw: &str) -> FormatSpec {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' || c == '}' {
            if c == '{'
                && matches!(chars.get(i + 1), Some('^'))
                && let Some(close) = chars[i..].iter().position(|&ch| ch == '}')
            {
                if chars
                    .get(i + 2)
                    .is_some_and(|tag| tag.eq_ignore_ascii_case(&'n'))
                {
                    literal.push('\n');
                }
                i += close + 1;
                continue;
            }
            i += 1;
            continue;
        }
        if c == '^' && chars.get(i + 1).is_some_and(|ch| ch.is_alphanumeric()) {
            i += 2;
            continue;
        }
        if c == '%'
            && let Some((piece, consumed)) = parse_spec(&chars[i..])
        {
            if !literal.is_empty() {
                pieces.push(Piece::Literal(std::mem::take(&mut literal)));
            }
            pieces.push(piece);
            i += consumed;
            continue;
        }
        literal.push(c);
        i += 1;
    }
    if !literal.is_empty() {
        pieces.push(Piece::Literal(literal));
    }
    FormatSpec { pieces }
}

/// `%(sign?.(digits)?)?[sdft](index)` — the reference's
/// `ConvertFormatRegEx`, matched at the head of `chars`.
fn parse_spec(chars: &[char]) -> Option<(Piece, usize)> {
    let mut i = 1;
    let mut sign = false;
    let mut decimals = None;
    let precision_start = i;
    if matches!(chars.get(i), Some('+' | '-')) {
        sign = chars[i] == '+';
        i += 1;
    }
    if matches!(chars.get(i), Some('.')) {
        i += 1;
        if let Some(digit) = chars
            .get(i)
            .and_then(|c| c.to_digit(10))
            .and_then(|digit| u8::try_from(digit).ok())
        {
            decimals = Some(digit);
            i += 1;
        } else {
            decimals = Some(0);
        }
    } else {
        // No dot: the precision group requires one, so rewind.
        i = precision_start;
        sign = false;
    }
    let kind = *chars.get(i)?;
    if !matches!(kind, 's' | 'd' | 'f' | 't') {
        return None;
    }
    i += 1;
    let index = usize::try_from(chars.get(i)?.to_digit(10)?).ok()?;
    i += 1;
    let decimals = if matches!(kind, 'd' | 'f') {
        decimals
    } else {
        None
    };
    Some((
        Piece::Arg {
            index,
            decimals,
            sign,
        },
        i,
    ))
}

/// The palette char of a leading `{^X}` / `^X` color tag, if any.
#[must_use]
pub fn leading_color(text: &str) -> Option<char> {
    let bytes: Vec<char> = text.chars().take(4).collect();
    match bytes.as_slice() {
        ['{', '^', tag, '}', ..] | ['^', tag, ..] => Some(tag.to_ascii_uppercase()),
        _ => None,
    }
}

/// Removes every `{^X}` / `^X` color tag.
#[must_use]
pub fn strip_color_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{'
            && matches!(chars.get(i + 1), Some('^'))
            && matches!(chars.get(i + 3), Some('}'))
        {
            i += 4;
            continue;
        }
        if chars[i] == '^' && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric()) {
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Greedy word wrap at spaces — the reference's
/// `StringHelper.WrapWords`.
#[must_use]
pub fn wrap_words(text: &str, columns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > columns {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Evaluates an `itemcost.dbr` requirement equation: `+ - * / ^`,
/// parentheses, unary minus, numbers, and identifiers resolved via
/// `lookup`. Returns `None` on any unknown identifier or syntax
/// error — the requirement is then skipped, as the reference does on
/// its evaluate exceptions.
#[must_use]
pub fn eval_equation(expr: &str, lookup: &dyn Fn(&str) -> Option<f64>) -> Option<f64> {
    let tokens = tokenize(expr, lookup)?;
    let mut parser = Parser {
        tokens,
        position: 0,
    };
    let value = parser.expression(0)?;
    (parser.position == parser.tokens.len()).then_some(value)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Number(f64),
    Operator(char),
    Open,
    Close,
}

fn tokenize(expr: &str, lookup: &dyn Fn(&str) -> Option<f64>) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let number: String = chars[start..i].iter().collect();
            tokens.push(Token::Number(number.parse().ok()?));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            tokens.push(Token::Number(lookup(&ident)?));
        } else if matches!(c, '+' | '-' | '*' | '/' | '^') {
            tokens.push(Token::Operator(c));
            i += 1;
        } else if c == '(' {
            tokens.push(Token::Open);
            i += 1;
        } else if c == ')' {
            tokens.push(Token::Close);
            i += 1;
        } else {
            return None;
        }
    }
    Some(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn expression(&mut self, min_binding: u8) -> Option<f64> {
        let mut left = self.atom()?;
        while let Some(Token::Operator(op)) = self.peek() {
            let (binding, right_binding) = match op {
                '+' | '-' => (1, 2),
                '*' | '/' => (3, 4),
                '^' => (6, 5),
                _ => return None,
            };
            if binding < min_binding {
                break;
            }
            self.position += 1;
            let right = self.expression(right_binding)?;
            left = match op {
                '+' => left + right,
                '-' => left - right,
                '*' => left * right,
                '/' => left / right,
                '^' => left.powf(right),
                _ => return None,
            };
        }
        Some(left)
    }

    fn atom(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Number(value) => {
                self.position += 1;
                Some(value)
            }
            Token::Operator('-') => {
                self.position += 1;
                Some(-self.atom()?)
            }
            Token::Operator('+') => {
                self.position += 1;
                self.atom()
            }
            Token::Open => {
                self.position += 1;
                let value = self.expression(0)?;
                match self.peek() {
                    Some(Token::Close) => {
                        self.position += 1;
                        Some(value)
                    }
                    _ => None,
                }
            }
            Token::Operator(_) | Token::Close => None,
        }
    }

    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.position).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(spec: &str, args: &[Arg]) -> String {
        parse_format(spec).format(args)
    }

    #[test]
    fn real_game_specs_format_like_the_reference() {
        assert_eq!(fmt("{%.0f0}", &[Arg::Number(12.6)]), "13");
        assert_eq!(
            fmt("{%.0f0} ~ {%.0f1}", &[Arg::Number(12.0), Arg::Number(31.0)]),
            "12 ~ 31"
        );
        assert_eq!(
            fmt("{%.1f0}% Chance of", &[Arg::Number(15.0)]),
            "15.0% Chance of"
        );
        assert_eq!(
            fmt("{%+.0f0} Strength", &[Arg::Number(24.0)]),
            "+24 Strength"
        );
        assert_eq!(
            fmt("{%+.0f0} Strength", &[Arg::Number(-10.0)]),
            "-10 Strength"
        );
        assert_eq!(
            fmt(
                "{+%d0} to {%s1}",
                &[Arg::Number(2.0), Arg::Text("Warfare".into())]
            ),
            "+2 to Warfare"
        );
        assert_eq!(
            fmt(
                "Required {%s0}: {%.0f1}",
                &[Arg::Text("Strength".into()), Arg::Number(120.0)]
            ),
            "Required Strength: 120"
        );
        assert_eq!(
            fmt(
                "{%s0 - %d1 / %d2}",
                &[
                    Arg::Text("Relic".into()),
                    Arg::Number(3.0),
                    Arg::Number(5.0)
                ]
            ),
            "Relic - 3 / 5"
        );
        assert_eq!(
            fmt("with {%+.0f0}% Improved Duration", &[Arg::Number(50.0)]),
            "with +50% Improved Duration"
        );
    }

    #[test]
    fn labels_without_specs_have_no_args() {
        assert!(!parse_format("Fire Damage").has_args());
        assert!(parse_format("{%.0f0}% Fire Resistance").has_args());
    }

    #[test]
    fn color_tags_are_recognized_and_stripped() {
        assert_eq!(leading_color("{^E}+10 Strength"), Some('E'));
        assert_eq!(leading_color("^y+10"), Some('Y'));
        assert_eq!(leading_color("+10 Strength"), None);
        assert_eq!(strip_color_tags("{^E}+10 ^WStrength"), "+10 Strength");
        assert_eq!(fmt("{^N}Armor", &[]), "\nArmor");
    }

    #[test]
    fn wrap_words_wraps_greedily() {
        assert_eq!(
            wrap_words("one two three four", 9),
            vec![
                "one two".to_string(),
                "three".to_string(),
                "four".to_string()
            ]
        );
    }

    #[test]
    fn equations_evaluate_with_identifiers() {
        let lookup = |name: &str| match name {
            "itemLevel" => Some(20.0),
            "totalAttCount" => Some(4.0),
            _ => None,
        };
        assert_eq!(
            eval_equation("50+((itemLevel-1)*7.75)", &lookup),
            Some(197.25)
        );
        assert_eq!(
            eval_equation("(totalAttCount * 5) ^ 2", &lookup),
            Some(400.0)
        );
        assert_eq!(eval_equation("2 ^ 3 ^ 2", &lookup), Some(512.0));
        assert_eq!(eval_equation("-(3 + 2) * 4", &lookup), Some(-20.0));
        assert_eq!(eval_equation("unknownVar + 1", &lookup), None);
        assert_eq!(eval_equation("3 +", &lookup), None);
    }
}
