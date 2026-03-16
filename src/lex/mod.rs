pub mod binary_number;
pub mod decimal_number;
pub mod identifier;
pub mod keyword;
pub mod symbol;

use std::iter::Peekable;

use codespan::{ByteIndex, ByteOffset};

use self::{
    binary_number::BinaryNumber, decimal_number::DecimalNumber, identifier::Identifier,
    keyword::Keyword, symbol::Symbol,
};
use crate::error::{LexError, LexResult, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Keyword(Keyword),
    Symbol(Symbol),
    Identifier(Identifier),
    DecimalNumber(DecimalNumber),
    BinaryNumber(BinaryNumber),
    StringLiteral {
        literal: String,
        is_quote_closed_in_source: bool,
    },
    Remark(String), // For BASIC comments
}

/// A token with its source span for better error reporting
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    token: Token,
    span: Span,
}

impl SpannedToken {
    pub fn new(token: Token, span: Span) -> Self {
        Self { token, span }
    }

    pub fn token(&self) -> &Token {
        &self.token
    }

    pub fn token_mut(&mut self) -> &mut Token {
        &mut self.token
    }

    pub fn span(&self) -> &Span {
        &self.span
    }
}

impl std::fmt::Display for SpannedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.token)
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Keyword(keyword) => write!(f, "{keyword}"),
            Token::Symbol(symbol) => write!(f, "{symbol}"),
            Token::Identifier(identifier) => write!(f, "{identifier}"),
            Token::DecimalNumber(decimal_number) => write!(f, "{decimal_number}"),
            Token::BinaryNumber(binary_number) => write!(f, "{binary_number}"),
            Token::StringLiteral { literal, .. } => write!(f, "\"{literal}\""),
            Token::Remark(remark) => write!(f, "REM {remark}"),
        }
    }
}

pub enum RemarkLexOption {
    TrimWhitespace, // DungeonQuest.bas option
    KeepWhole,      // course.bas option
}

pub struct Lexer<'a> {
    input: Peekable<std::str::Chars<'a>>,
    position: ByteIndex,
    is_done: bool,
    remark_opt: RemarkLexOption,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str, remark_opt: RemarkLexOption) -> Self {
        Lexer {
            input: input.chars().peekable(),
            position: ByteIndex::from(0),
            is_done: false,
            remark_opt,
        }
    }

    fn advance(&mut self) -> Option<char> {
        if let Some(ch) = self.input.next() {
            self.position += ByteOffset::from(ch.len_utf8() as i64);
            Some(ch)
        } else {
            None
        }
    }

    fn peek(&mut self) -> Option<&char> {
        self.input.peek()
    }

    fn span_from(&self, start: ByteIndex) -> Span {
        Span::new(start, self.position)
    }
}

impl Iterator for Lexer<'_> {
    type Item = LexResult<SpannedToken>;

    fn next(&mut self) -> Option<Self::Item> {
        // Skip whitespace (except newlines which are significant in BASIC)
        while let Some(&ch) = self.peek() {
            if ch.is_whitespace() && ch != '\n' {
                self.advance();
            } else {
                break;
            }
        }

        let start_pos = self.position;
        let ch = self.advance();

        if ch.is_none() {
            if self.is_done {
                return None; // Already done
            } else {
                self.is_done = true;
                return Some(Ok(SpannedToken::new(
                    Token::Symbol(Symbol::Eof),
                    self.span_from(start_pos),
                ))); // End of input
            }
        }

        let ch = ch?;

        match ch {
            // Newlines are significant in BASIC
            '\n' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Newline),
                self.span_from(start_pos),
            ))),

            // Single character symbols
            '+' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Add),
                self.span_from(start_pos),
            ))),
            '-' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Sub),
                self.span_from(start_pos),
            ))),
            '*' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Mul),
                self.span_from(start_pos),
            ))),
            '/' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Div),
                self.span_from(start_pos),
            ))),
            '^' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Exp),
                self.span_from(start_pos),
            ))),
            ',' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Comma),
                self.span_from(start_pos),
            ))),
            ';' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Semicolon),
                self.span_from(start_pos),
            ))),
            ':' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Colon),
                self.span_from(start_pos),
            ))),
            '(' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::LParen),
                self.span_from(start_pos),
            ))),
            ')' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::RParen),
                self.span_from(start_pos),
            ))),

            // Comparison operators
            '=' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Eq),
                self.span_from(start_pos),
            ))),
            '<' => {
                if self.peek() == Some(&'=') {
                    self.advance();
                    Some(Ok(SpannedToken::new(
                        Token::Symbol(Symbol::Leq),
                        self.span_from(start_pos),
                    )))
                } else if self.peek() == Some(&'>') {
                    self.advance();
                    Some(Ok(SpannedToken::new(
                        Token::Symbol(Symbol::Neq),
                        self.span_from(start_pos),
                    )))
                } else {
                    Some(Ok(SpannedToken::new(
                        Token::Symbol(Symbol::Lt),
                        self.span_from(start_pos),
                    )))
                }
            }
            '>' => {
                if self.peek() == Some(&'=') {
                    self.advance();
                    Some(Ok(SpannedToken::new(
                        Token::Symbol(Symbol::Geq),
                        self.span_from(start_pos),
                    )))
                } else {
                    Some(Ok(SpannedToken::new(
                        Token::Symbol(Symbol::Gt),
                        self.span_from(start_pos),
                    )))
                }
            }
            '@' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::At),
                self.span_from(start_pos),
            ))),
            '$' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Dollar),
                self.span_from(start_pos),
            ))),
            '#' => Some(Ok(SpannedToken::new(
                Token::Symbol(Symbol::Hash),
                self.span_from(start_pos),
            ))),
            '√' => Some(Ok(SpannedToken::new(
                Token::Keyword(Keyword::Sqr),
                self.span_from(start_pos),
            ))),
            'π' => Some(Ok(SpannedToken::new(
                Token::Keyword(Keyword::Pi),
                self.span_from(start_pos),
            ))),
            // Parse true comments, lines beginning with "'"
            '\'' => {
                while let Some(ch) = self.advance() {
                    if ch == '\n' {
                        break;
                    }
                }
                self.next()
            }

            // String literals
            '"' => {
                let mut string_content = String::new();
                let mut found_closing_quote = false;
                let mut found_closing_newline = false;

                while let Some(&ch) = self.peek() {
                    if ch == '"' {
                        self.advance();
                        found_closing_quote = true;
                        break;
                    }

                    if ch == '\r' {
                        self.advance(); // Skip carriage return
                        continue; // Continue to next character
                    }

                    if ch == '\n' {
                        found_closing_newline = true; // Treat newline as end of string
                        break;
                    }

                    string_content.push(ch);
                    self.advance();
                }

                if !found_closing_quote && !found_closing_newline {
                    Some(Err(LexError::UnterminatedStringLiteral {
                        span: self.span_from(start_pos),
                    }))
                } else {
                    Some(Ok(SpannedToken::new(
                        Token::StringLiteral {
                            literal: string_content,
                            is_quote_closed_in_source: found_closing_quote,
                        },
                        self.span_from(start_pos),
                    )))
                }
            }

            // Binary numbers (hexadecimal with & prefix)
            '&' => {
                let mut hex_digits = String::from("&");
                while let Some(&ch) = self.peek() {
                    if ch.is_ascii_hexdigit() {
                        hex_digits.push(ch.to_ascii_uppercase());
                        self.advance();
                    } else {
                        break;
                    }
                }

                match hex_digits.parse::<BinaryNumber>() {
                    Ok(binary_number) => Some(Ok(SpannedToken::new(
                        Token::BinaryNumber(binary_number),
                        self.span_from(start_pos),
                    ))),
                    Err(_) => Some(Err(LexError::InvalidBinaryNumber {
                        text: hex_digits,
                        span: self.span_from(start_pos),
                    })),
                }
            }

            // Numbers (decimal)
            ch if ch.is_ascii_digit() || ch == '.' => {
                let mut number_str = String::new();
                number_str.push(ch);

                let mut has_dot = ch == '.';
                let mut has_e = false;

                while let Some(&ch) = self.peek() {
                    if ch.is_ascii_digit() {
                        number_str.push(ch);
                        self.advance();
                    } else if ch == '.' && !has_dot && !has_e {
                        has_dot = true;
                        number_str.push(ch);
                        self.advance();
                    } else if (ch == 'E') && !has_e {
                        // Do some lookahead to avoid parsing a keyword starting with E
                        {
                            let mut cloned_iter = self.input.clone();
                            cloned_iter.next(); // Skip the 'E'
                            match cloned_iter.peek() {
                                Some(&next_ch)
                                    if next_ch.is_ascii_digit()
                                        || next_ch == '+'
                                        || next_ch == '-' => {}
                                _ => break, // Not a valid exponent
                            }
                        }

                        has_e = true;
                        number_str.push(ch);
                        self.advance();

                        // Handle optional + or - after E
                        if let Some(&next_ch) = self.peek() {
                            if next_ch == '+' || next_ch == '-' {
                                number_str.push(next_ch);
                                self.advance();
                            }
                        }
                    } else {
                        break;
                    }
                }

                match number_str.parse::<DecimalNumber>() {
                    Ok(decimal_number) => Some(Ok(SpannedToken::new(
                        Token::DecimalNumber(decimal_number),
                        self.span_from(start_pos),
                    ))),
                    Err(_) => Some(Err(LexError::InvalidBinaryNumber {
                        text: number_str,
                        span: self.span_from(start_pos),
                    })),
                }
            }

            // Identifiers and keywords
            ch if ch.is_ascii_alphabetic() => {
                let mut word = String::new();
                word.push(ch.to_ascii_uppercase());

                // We need to do some lookahead, as identifiers don't need to be separated by whitespace
                // for example FOR I = BBTO 10 is a valid line that means:
                // FOR I = BB TO 10
                // So we need to read until we hit a non-alphanumeric character or a dollar sign
                {
                    // This logic is complex so we can handle identifiers next to keywords next to numbers,
                    // for example IF B=AGOSUB3 => IF B=A GOSUB 3
                    let cloned_iter = self.input.clone();
                    for (i, next_ch) in cloned_iter.enumerate() {
                        if (i < 1 && (next_ch.is_ascii_alphanumeric()))
                            || next_ch.is_ascii_alphabetic()
                        {
                            word.push(next_ch.to_ascii_uppercase());
                        } else if next_ch == '$' || next_ch == '#' {
                            word.push(next_ch.to_ascii_uppercase());
                            break;
                        } else {
                            break;
                        }
                    }
                }

                // Check if the word has any keyword as suffix
                for keyword in Keyword::LONGEST_SUFFIX_FIRST.iter() {
                    if word.ends_with(keyword) {
                        let keyword_len = keyword.len();
                        let ident_len = word.len() - keyword_len;
                        if ident_len == 0 {
                            // The whole word is a keyword
                            for _ in 1..keyword_len {
                                self.advance();
                            }

                            let kw = match Keyword::try_from(word.as_str()) {
                                Ok(kw) => kw,
                                Err(_) => {
                                    return Some(Err(LexError::UnexpectedCharacter {
                                        ch: word.chars().next().unwrap_or('?'),
                                        span: self.span_from(start_pos),
                                    }));
                                }
                            };

                            // We need to handle comments in a special way, consume until end of line
                            if kw == Keyword::Rem {
                                let mut comment = String::new();

                                // The space after REM is mandatory
                                if self.advance() != Some(' ') {
                                    return Some(Err(LexError::InvalidRemark {
                                        span: self.span_from(start_pos),
                                    }));
                                }
                                comment.push(' ');

                                match self.remark_opt {
                                    RemarkLexOption::TrimWhitespace => {
                                        // Skip the rest of the whitespace
                                        while self.peek().is_some_and(|&c| c == ' ' || c == '\t') {
                                            self.advance();
                                        }
                                    }
                                    RemarkLexOption::KeepWhole => {}
                                }

                                while let Some(&next_ch) = self.peek() {
                                    if next_ch == '\r' {
                                        self.advance(); // Skip carriage return
                                        continue; // Continue to next character
                                    }

                                    if next_ch == '\n' {
                                        break; // End of comment
                                    }
                                    comment.push(next_ch);
                                    self.advance();
                                }

                                match self.remark_opt {
                                    RemarkLexOption::TrimWhitespace => {
                                        // BLACKJACK behavior:
                                        if comment.len() == 1 {
                                            comment.pop();
                                        }
                                    }
                                    RemarkLexOption::KeepWhole => {}
                                }

                                return Some(Ok(SpannedToken::new(
                                    Token::Remark(comment),
                                    self.span_from(start_pos),
                                )));
                            } else {
                                return Some(Ok(SpannedToken::new(
                                    Token::Keyword(kw),
                                    self.span_from(start_pos),
                                )));
                            }
                        } else {
                            // Remove the identifier from the original iterator
                            for _ in 1..ident_len {
                                self.advance();
                            }

                            return Some(match word[..ident_len].parse::<Identifier>() {
                                Ok(identifier) => Ok(SpannedToken::new(
                                    Token::Identifier(identifier),
                                    self.span_from(start_pos),
                                )),
                                Err(_) => Err(LexError::UnexpectedCharacter {
                                    ch: word.chars().next().unwrap_or('?'),
                                    span: self.span_from(start_pos),
                                }),
                            });
                        }
                    }
                }

                // If no keyword suffix found, treat as identifier
                for _ in 1..word.len() {
                    self.advance(); // Consume the rest of the identifier
                }

                match word.parse::<Identifier>() {
                    Ok(identifier) => Some(Ok(SpannedToken::new(
                        Token::Identifier(identifier),
                        self.span_from(start_pos),
                    ))),
                    Err(_) => Some(Err(LexError::UnexpectedCharacter {
                        ch: word.chars().next().unwrap_or('?'),
                        span: self.span_from(start_pos),
                    })),
                }
            }

            _ => Some(Err(LexError::UnexpectedCharacter {
                ch,
                span: self.span_from(start_pos),
            })),
        }
    }
}
