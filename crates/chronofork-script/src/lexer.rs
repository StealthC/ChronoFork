//! Line-oriented lexer for the ChronoFork script language.
//!
//! The lexer is total over byte input: every character either produces a token
//! or a typed [`ParseError`] with a 1-based line and column. Newlines are
//! statement separators, except inside parentheses where they are ignored.

use crate::error::{ParseError, ParseErrorKind};
use crate::limits::MAX_TEXT_BYTES;

/// Reserved words of the language.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Keyword {
    Let,
    Set,
    Print,
    Assert,
    If,
    Else,
    Repeat,
    While,
    True,
    False,
}

impl Keyword {
    pub(crate) const fn describe(self) -> &'static str {
        match self {
            Self::Let => "'let'",
            Self::Set => "'set'",
            Self::Print => "'print'",
            Self::Assert => "'assert'",
            Self::If => "'if'",
            Self::Else => "'else'",
            Self::Repeat => "'repeat'",
            Self::While => "'while'",
            Self::True => "'true'",
            Self::False => "'false'",
        }
    }

    fn from_identifier(identifier: &str) -> Option<Self> {
        match identifier {
            "let" => Some(Self::Let),
            "set" => Some(Self::Set),
            "print" => Some(Self::Print),
            "assert" => Some(Self::Assert),
            "if" => Some(Self::If),
            "else" => Some(Self::Else),
            "repeat" => Some(Self::Repeat),
            "while" => Some(Self::While),
            "true" => Some(Self::True),
            "false" => Some(Self::False),
            _ => None,
        }
    }
}

/// Punctuation and operator tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Punct {
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Assign,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
}

impl Punct {
    pub(crate) const fn describe(self) -> &'static str {
        match self {
            Self::Plus => "'+'",
            Self::Minus => "'-'",
            Self::Star => "'*'",
            Self::Slash => "'/'",
            Self::Percent => "'%'",
            Self::EqEq => "'=='",
            Self::NotEq => "'!='",
            Self::Lt => "'<'",
            Self::Le => "'<='",
            Self::Gt => "'>'",
            Self::Ge => "'>='",
            Self::AndAnd => "'&&'",
            Self::OrOr => "'||'",
            Self::Bang => "'!'",
            Self::Assign => "'='",
            Self::LParen => "'('",
            Self::RParen => "')'",
            Self::LBrace => "'{'",
            Self::RBrace => "'}'",
            Self::Comma => "','",
        }
    }
}

/// A lexical token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TokenKind {
    Int(i64),
    Text(String),
    Ident(String),
    Keyword(Keyword),
    Punct(Punct),
    Newline,
    Eof,
}

impl TokenKind {
    pub(crate) const fn describe(&self) -> &'static str {
        match self {
            Self::Int(_) => "an integer",
            Self::Text(_) => "a string",
            Self::Ident(_) => "an identifier",
            Self::Keyword(keyword) => keyword.describe(),
            Self::Punct(punct) => punct.describe(),
            Self::Newline => "a newline",
            Self::Eof => "end of file",
        }
    }
}

/// A token with its 1-based source position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) line: usize,
    pub(crate) column: usize,
}

impl Token {
    const fn new(kind: TokenKind, line: usize, column: usize) -> Self {
        Self { kind, line, column }
    }
}

/// Lexes a complete source string into tokens ending with an end-of-file token.
pub(crate) fn lex(source: &str) -> Result<Vec<Token>, ParseError> {
    Lexer::new(source).lex_all()
}

struct Lexer {
    characters: Vec<char>,
    index: usize,
    line: usize,
    column: usize,
    parenthesis_depth: usize,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            characters: source.chars().collect(),
            index: 0,
            line: 1,
            column: 1,
            parenthesis_depth: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.characters.get(self.index).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.index
            .checked_add(1)
            .and_then(|index| self.characters.get(index))
            .copied()
    }

    fn advance(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.index = self.index.saturating_add(1);
        if character == '\n' {
            self.line = self.line.saturating_add(1);
            self.column = 1;
        } else {
            self.column = self.column.saturating_add(1);
        }
        Some(character)
    }

    fn error(&self, kind: ParseErrorKind, line: usize, column: usize) -> ParseError {
        ParseError::new(kind, line, column)
    }

    fn lex_all(mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            let (line, column) = (self.line, self.column);
            let Some(character) = self.peek() else {
                tokens.push(Token::new(TokenKind::Eof, line, column));
                return Ok(tokens);
            };
            match character {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                    if self.parenthesis_depth == 0 {
                        tokens.push(Token::new(TokenKind::Newline, line, column));
                    }
                }
                '#' => self.skip_comment(),
                '"' => tokens.push(self.lex_string()?),
                '0'..='9' => tokens.push(self.lex_number()?),
                character if is_identifier_start(character) => {
                    tokens.push(self.lex_identifier());
                }
                _ => tokens.push(self.lex_punctuation()?),
            }
        }
    }

    fn skip_comment(&mut self) {
        while let Some(character) = self.peek() {
            if character == '\n' {
                return;
            }
            self.advance();
        }
    }

    fn lex_string(&mut self) -> Result<Token, ParseError> {
        let (line, column) = (self.line, self.column);
        self.advance();
        let mut text = String::new();
        loop {
            let Some(character) = self.peek() else {
                return Err(self.error(ParseErrorKind::UnterminatedString, line, column));
            };
            match character {
                '"' => {
                    self.advance();
                    return Ok(Token::new(TokenKind::Text(text), line, column));
                }
                '\n' => {
                    return Err(self.error(ParseErrorKind::UnterminatedString, line, column));
                }
                '\\' => {
                    let (escape_line, escape_column) = (self.line, self.column);
                    self.advance();
                    let Some(escaped) = self.advance() else {
                        return Err(self.error(ParseErrorKind::UnterminatedString, line, column));
                    };
                    let decoded = match escaped {
                        '\\' => '\\',
                        '"' => '"',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => {
                            return Err(self.error(
                                ParseErrorKind::InvalidEscape(other),
                                escape_line,
                                escape_column,
                            ));
                        }
                    };
                    text.push(decoded);
                }
                _ => {
                    self.advance();
                    text.push(character);
                }
            }
            if text.len() > MAX_TEXT_BYTES {
                return Err(self.error(ParseErrorKind::StringTooLong, line, column));
            }
        }
    }

    /// Lexes a decimal integer or a `0x`/`0X` hexadecimal integer.
    ///
    /// Both forms must fit `i64`; the grammar and ADR-0011 §3.5 document the
    /// hexadecimal prefix as the only non-decimal integer literal form.
    fn lex_number(&mut self) -> Result<Token, ParseError> {
        let (line, column) = (self.line, self.column);
        let hexadecimal = self.peek() == Some('0') && matches!(self.peek_next(), Some('x' | 'X'));
        let mut digits = String::new();
        if hexadecimal {
            self.advance();
            self.advance();
            while let Some(character) = self.peek() {
                if character.is_ascii_hexdigit() {
                    digits.push(character);
                    self.advance();
                } else {
                    break;
                }
            }
            if digits.is_empty() {
                return Err(self.error(ParseErrorKind::InvalidNumber, line, column));
            }
            match i64::from_str_radix(&digits, 16) {
                Ok(value) => Ok(Token::new(TokenKind::Int(value), line, column)),
                Err(_) => Err(self.error(ParseErrorKind::InvalidNumber, line, column)),
            }
        } else {
            while let Some(character) = self.peek() {
                if character.is_ascii_digit() {
                    digits.push(character);
                    self.advance();
                } else {
                    break;
                }
            }
            match digits.parse::<i64>() {
                Ok(value) => Ok(Token::new(TokenKind::Int(value), line, column)),
                Err(_) => Err(self.error(ParseErrorKind::InvalidNumber, line, column)),
            }
        }
    }

    fn lex_identifier(&mut self) -> Token {
        let (line, column) = (self.line, self.column);
        let mut identifier = String::new();
        while let Some(character) = self.peek() {
            if is_identifier_continue(character) {
                identifier.push(character);
                self.advance();
            } else {
                break;
            }
        }
        let kind = match Keyword::from_identifier(&identifier) {
            Some(keyword) => TokenKind::Keyword(keyword),
            None => TokenKind::Ident(identifier),
        };
        Token::new(kind, line, column)
    }

    fn lex_punctuation(&mut self) -> Result<Token, ParseError> {
        let (line, column) = (self.line, self.column);
        let character = self.advance().ok_or_else(|| {
            self.error(
                ParseErrorKind::UnexpectedEndOfFile {
                    expected: "a token",
                },
                line,
                column,
            )
        })?;
        let punct = match character {
            '+' => Punct::Plus,
            '-' => Punct::Minus,
            '*' => Punct::Star,
            '/' => Punct::Slash,
            '%' => Punct::Percent,
            '=' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Punct::EqEq
                } else {
                    Punct::Assign
                }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Punct::NotEq
                } else {
                    Punct::Bang
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Punct::Le
                } else {
                    Punct::Lt
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Punct::Ge
                } else {
                    Punct::Gt
                }
            }
            '&' if self.peek() == Some('&') => {
                self.advance();
                Punct::AndAnd
            }
            '|' if self.peek() == Some('|') => {
                self.advance();
                Punct::OrOr
            }
            '(' => {
                self.parenthesis_depth = self.parenthesis_depth.saturating_add(1);
                Punct::LParen
            }
            ')' => {
                self.parenthesis_depth = self.parenthesis_depth.saturating_sub(1);
                Punct::RParen
            }
            '{' => Punct::LBrace,
            '}' => Punct::RBrace,
            ',' => Punct::Comma,
            other => return Err(self.error(ParseErrorKind::UnknownCharacter(other), line, column)),
        };
        Ok(Token::new(TokenKind::Punct(punct), line, column))
    }
}

fn is_identifier_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_'
}

fn is_identifier_continue(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}
