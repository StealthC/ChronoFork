//! Recursive-descent parser for the ChronoFork script language.
//!
//! The grammar is the line-oriented subset documented by ADR-0011 §3.5. Blocks
//! introduce no scope; the parser only produces a typed AST and reports syntax
//! errors with a 1-based line and column.

use crate::error::{ParseError, ParseErrorKind};
use crate::lexer::{Keyword, Punct, Token, TokenKind, lex};

/// Maximum supported expression, call and block nesting depth.
pub(crate) const MAX_NESTING_DEPTH: usize = 64;

/// A parsed script ready for evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Program {
    pub(crate) statements: Vec<Stmt>,
}

/// One executable statement and the 1-based line it starts on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Stmt {
    pub(crate) line: usize,
    pub(crate) kind: StmtKind,
}

/// The executable form of a statement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StmtKind {
    Let {
        name: String,
        value: Expr,
    },
    Set {
        name: String,
        value: Expr,
    },
    Print {
        value: Expr,
    },
    Assert {
        value: Expr,
    },
    If {
        condition: Expr,
        then_block: Vec<Stmt>,
        else_block: Option<Vec<Stmt>>,
    },
    Repeat {
        count: Expr,
        body: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    Call(Expr),
}

/// An expression node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Expr {
    Int(i64),
    Bool(bool),
    Text(String),
    Var(String),
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

/// Unary operators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnaryOp {
    Negate,
    Not,
}

/// Binary operators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// Parses a complete source string into a typed program.
pub(crate) fn parse_source(source: &str) -> Result<Program, ParseError> {
    let tokens = lex(source)?;
    parse_tokens(tokens)
}

fn parse_tokens(tokens: Vec<Token>) -> Result<Program, ParseError> {
    let mut parser = Parser {
        tokens,
        index: 0,
        depth: 0,
    };
    parser.parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
    depth: usize,
}

impl Parser {
    fn current(&self) -> &Token {
        self.tokens
            .get(self.index)
            .unwrap_or_else(|| &self.tokens[self.tokens.len() - 1])
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();
        if !matches!(token.kind, TokenKind::Eof) {
            self.index = self.index.saturating_add(1);
        }
        token
    }

    fn at_eof(&self) -> bool {
        matches!(self.current().kind, TokenKind::Eof)
    }

    fn at_punct(&self, punct: Punct) -> bool {
        matches!(self.current().kind, TokenKind::Punct(found) if found == punct)
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Keyword(found) if found == keyword
        )
    }

    fn at_newline(&self) -> bool {
        matches!(self.current().kind, TokenKind::Newline)
    }

    fn error(&self, kind: ParseErrorKind, token: &Token) -> ParseError {
        ParseError::new(kind, token.line, token.column)
    }

    fn expected(&self, expected: &'static str) -> ParseError {
        let token = self.current();
        if self.at_eof() {
            self.error(ParseErrorKind::UnexpectedEndOfFile { expected }, token)
        } else {
            self.error(ParseErrorKind::ExpectedToken { expected }, token)
        }
    }

    fn unexpected(&self, expected: &'static str) -> ParseError {
        let token = self.current();
        if self.at_eof() {
            self.error(ParseErrorKind::UnexpectedEndOfFile { expected }, token)
        } else {
            self.error(
                ParseErrorKind::UnexpectedToken {
                    found: token.kind.describe(),
                },
                token,
            )
        }
    }

    fn expect_punct(&mut self, punct: Punct) -> Result<Token, ParseError> {
        if self.at_punct(punct) {
            Ok(self.advance())
        } else {
            Err(self.expected(punct.describe()))
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword) -> Result<Token, ParseError> {
        if self.at_keyword(keyword) {
            Ok(self.advance())
        } else {
            Err(self.expected(keyword.describe()))
        }
    }

    fn expect_identifier(&mut self) -> Result<(String, Token), ParseError> {
        let token = self.current().clone();
        if let TokenKind::Ident(name) = &token.kind {
            self.advance();
            Ok((name.clone(), token))
        } else {
            Err(self.expected("an identifier"))
        }
    }

    fn expect_newline(&mut self) -> Result<(), ParseError> {
        if self.at_newline() {
            self.advance();
            Ok(())
        } else if self.at_eof() {
            Ok(())
        } else {
            let token = self.current();
            Err(self.error(ParseErrorKind::MissingNewline, token))
        }
    }

    fn skip_newlines(&mut self) {
        while self.at_newline() {
            self.advance();
        }
    }

    fn enter_nesting(&mut self) -> Result<(), ParseError> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_NESTING_DEPTH {
            let token = self.current();
            return Err(self.error(ParseErrorKind::NestingTooDeep, token));
        }
        Ok(())
    }

    fn leave_nesting(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        self.skip_newlines();
        let mut statements = Vec::new();
        while !self.at_eof() {
            if self.at_keyword(Keyword::Else) {
                let token = self.current();
                return Err(self.error(ParseErrorKind::StrayElse, token));
            }
            statements.push(self.parse_statement()?);
            self.skip_newlines();
        }
        if statements.is_empty() {
            return Err(ParseError::new(ParseErrorKind::EmptyProgram, 1, 1));
        }
        Ok(Program { statements })
    }

    fn parse_statement(&mut self) -> Result<Stmt, ParseError> {
        let token = self.current().clone();
        let line = token.line;
        let kind = match token.kind {
            TokenKind::Keyword(Keyword::Let) => {
                self.advance();
                self.parse_assignment(true)?
            }
            TokenKind::Keyword(Keyword::Set) => {
                self.advance();
                self.parse_assignment(false)?
            }
            TokenKind::Keyword(Keyword::Print) => {
                self.advance();
                let value = self.parse_expr()?;
                self.expect_newline()?;
                StmtKind::Print { value }
            }
            TokenKind::Keyword(Keyword::Assert) => {
                self.advance();
                let value = self.parse_expr()?;
                self.expect_newline()?;
                StmtKind::Assert { value }
            }
            TokenKind::Keyword(Keyword::If) => return self.parse_if(line),
            TokenKind::Keyword(Keyword::Repeat) => {
                self.advance();
                let count = self.parse_expr()?;
                let body = self.parse_block()?;
                StmtKind::Repeat { count, body }
            }
            TokenKind::Keyword(Keyword::While) => {
                self.advance();
                let condition = self.parse_expr()?;
                let body = self.parse_block()?;
                StmtKind::While { condition, body }
            }
            TokenKind::Keyword(Keyword::Else) => {
                return Err(self.error(ParseErrorKind::StrayElse, &token));
            }
            TokenKind::Ident(_) => {
                let call = self.parse_call()?;
                self.expect_newline()?;
                StmtKind::Call(call)
            }
            _ => return Err(self.unexpected("a statement")),
        };
        Ok(Stmt { line, kind })
    }

    fn parse_assignment(&mut self, declare: bool) -> Result<StmtKind, ParseError> {
        let (name, _) = self.expect_identifier()?;
        self.expect_punct(Punct::Assign)?;
        let value = self.parse_expr()?;
        self.expect_newline()?;
        let kind = if declare {
            StmtKind::Let { name, value }
        } else {
            StmtKind::Set { name, value }
        };
        Ok(kind)
    }

    fn parse_if(&mut self, line: usize) -> Result<Stmt, ParseError> {
        self.expect_keyword(Keyword::If)?;
        let condition = self.parse_expr()?;
        let then_block = self.parse_block()?;
        let else_block = if self.at_keyword(Keyword::Else) {
            self.advance();
            if self.at_keyword(Keyword::If) {
                let nested_line = self.current().line;
                Some(vec![self.parse_if(nested_line)?])
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };
        Ok(Stmt {
            line,
            kind: StmtKind::If {
                condition,
                then_block,
                else_block,
            },
        })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.enter_nesting()?;
        let result = self.parse_block_inner();
        self.leave_nesting();
        result
    }

    fn parse_block_inner(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.expect_punct(Punct::LBrace)?;
        self.skip_newlines();
        let mut statements = Vec::new();
        loop {
            if self.at_punct(Punct::RBrace) {
                self.advance();
                self.skip_newlines();
                return Ok(statements);
            }
            if self.at_eof() {
                let token = self.current();
                return Err(self.error(
                    ParseErrorKind::UnexpectedEndOfFile { expected: "'}'" },
                    token,
                ));
            }
            if self.at_keyword(Keyword::Else) {
                let token = self.current();
                return Err(self.error(ParseErrorKind::StrayElse, token));
            }
            statements.push(self.parse_statement()?);
            self.skip_newlines();
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.enter_nesting()?;
        let result = self.parse_or();
        self.leave_nesting();
        result
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut expression = self.parse_and()?;
        while self.at_punct(Punct::OrOr) {
            self.advance();
            let right = self.parse_and()?;
            expression = binary(BinaryOp::Or, expression, right);
        }
        Ok(expression)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut expression = self.parse_comparison()?;
        while self.at_punct(Punct::AndAnd) {
            self.advance();
            let right = self.parse_comparison()?;
            expression = binary(BinaryOp::And, expression, right);
        }
        Ok(expression)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let expression = self.parse_addition()?;
        let operator = match &self.current().kind {
            TokenKind::Punct(Punct::EqEq) => Some(BinaryOp::Eq),
            TokenKind::Punct(Punct::NotEq) => Some(BinaryOp::Ne),
            TokenKind::Punct(Punct::Lt) => Some(BinaryOp::Lt),
            TokenKind::Punct(Punct::Le) => Some(BinaryOp::Le),
            TokenKind::Punct(Punct::Gt) => Some(BinaryOp::Gt),
            TokenKind::Punct(Punct::Ge) => Some(BinaryOp::Ge),
            _ => None,
        };
        match operator {
            Some(operator) => {
                self.advance();
                let right = self.parse_addition()?;
                Ok(binary(operator, expression, right))
            }
            None => Ok(expression),
        }
    }

    fn parse_addition(&mut self) -> Result<Expr, ParseError> {
        let mut expression = self.parse_multiplication()?;
        loop {
            let operator = match &self.current().kind {
                TokenKind::Punct(Punct::Plus) => Some(BinaryOp::Add),
                TokenKind::Punct(Punct::Minus) => Some(BinaryOp::Sub),
                _ => None,
            };
            let Some(operator) = operator else {
                return Ok(expression);
            };
            self.advance();
            let right = self.parse_multiplication()?;
            expression = binary(operator, expression, right);
        }
    }

    fn parse_multiplication(&mut self) -> Result<Expr, ParseError> {
        let mut expression = self.parse_unary()?;
        loop {
            let operator = match &self.current().kind {
                TokenKind::Punct(Punct::Star) => Some(BinaryOp::Mul),
                TokenKind::Punct(Punct::Slash) => Some(BinaryOp::Div),
                TokenKind::Punct(Punct::Percent) => Some(BinaryOp::Rem),
                _ => None,
            };
            let Some(operator) = operator else {
                return Ok(expression);
            };
            self.advance();
            let right = self.parse_unary()?;
            expression = binary(operator, expression, right);
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        self.enter_nesting()?;
        let result = self.parse_unary_inner();
        self.leave_nesting();
        result
    }

    fn parse_unary_inner(&mut self) -> Result<Expr, ParseError> {
        let operator = match &self.current().kind {
            TokenKind::Punct(Punct::Minus) => Some(UnaryOp::Negate),
            TokenKind::Punct(Punct::Bang) => Some(UnaryOp::Not),
            _ => None,
        };
        if let Some(operator) = operator {
            self.advance();
            let operand = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: operator,
                operand: Box::new(operand),
            });
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Int(value) => {
                self.advance();
                Ok(Expr::Int(value))
            }
            TokenKind::Text(text) => {
                self.advance();
                Ok(Expr::Text(text))
            }
            TokenKind::Keyword(Keyword::True) => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            TokenKind::Keyword(Keyword::False) => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            TokenKind::Ident(_) => {
                let (name, _) = self.expect_identifier()?;
                if self.at_punct(Punct::LParen) {
                    self.parse_call_arguments(name)
                } else {
                    Ok(Expr::Var(name))
                }
            }
            TokenKind::Punct(Punct::LParen) => {
                self.advance();
                let expression = self.parse_expr()?;
                self.expect_punct(Punct::RParen)?;
                Ok(expression)
            }
            _ => Err(self.unexpected("an expression")),
        }
    }

    fn parse_call(&mut self) -> Result<Expr, ParseError> {
        let (name, token) = self.expect_identifier()?;
        if !self.at_punct(Punct::LParen) {
            return Err(self.error(
                ParseErrorKind::ExpectedToken {
                    expected: "a builtin call",
                },
                &token,
            ));
        }
        self.parse_call_arguments(name)
    }

    fn parse_call_arguments(&mut self, name: String) -> Result<Expr, ParseError> {
        self.expect_punct(Punct::LParen)?;
        let mut args = Vec::new();
        if self.at_punct(Punct::RParen) {
            self.advance();
            return Ok(Expr::Call { name, args });
        }
        loop {
            args.push(self.parse_expr()?);
            if self.at_punct(Punct::Comma) {
                self.advance();
                continue;
            }
            self.expect_punct(Punct::RParen)?;
            return Ok(Expr::Call { name, args });
        }
    }
}

fn binary(operator: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op: operator,
        left: Box::new(left),
        right: Box::new(right),
    }
}
