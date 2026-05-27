//! OTL lexer and recursive-descent parser.

use std::fmt;

use super::ast::{DslError, DslExpression};

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Number(f32),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Comma,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Number,
    Ident,
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Comma,
    Eof,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Number => write!(f, "number"),
            TokenKind::Ident => write!(f, "identifier"),
            TokenKind::Plus => write!(f, "'+'"),
            TokenKind::Minus => write!(f, "'-'"),
            TokenKind::Star => write!(f, "'*'"),
            TokenKind::Slash => write!(f, "'/'"),
            TokenKind::LParen => write!(f, "'('"),
            TokenKind::RParen => write!(f, "')'"),
            TokenKind::Comma => write!(f, "','"),
            TokenKind::Eof => write!(f, "end of input"),
        }
    }
}

impl Token {
    fn kind(&self) -> TokenKind {
        match self {
            Token::Number(_) => TokenKind::Number,
            Token::Ident(_) => TokenKind::Ident,
            Token::Plus => TokenKind::Plus,
            Token::Minus => TokenKind::Minus,
            Token::Star => TokenKind::Star,
            Token::Slash => TokenKind::Slash,
            Token::LParen => TokenKind::LParen,
            Token::RParen => TokenKind::RParen,
            Token::Comma => TokenKind::Comma,
        }
    }
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, DslError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DslError::EmptyInput);
    }

    let mut tokens = Vec::new();
    let bytes = trimmed.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }

        match byte {
            b'+' => {
                tokens.push(Token::Plus);
                index += 1;
            }
            b'-' => {
                tokens.push(Token::Minus);
                index += 1;
            }
            b'*' => {
                tokens.push(Token::Star);
                index += 1;
            }
            b'/' => {
                tokens.push(Token::Slash);
                index += 1;
            }
            b'(' => {
                tokens.push(Token::LParen);
                index += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                index += 1;
            }
            b',' => {
                tokens.push(Token::Comma);
                index += 1;
            }
            b'0'..=b'9' | b'.' => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_digit() || bytes[index] == b'.')
                {
                    index += 1;
                }
                let slice = &trimmed[start..index];
                let value = slice
                    .parse::<f32>()
                    .map_err(|_| DslError::Evaluation(format!("invalid number `{slice}`")))?;
                tokens.push(Token::Number(value));
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = index;
                index += 1;
                while index < bytes.len() {
                    let next = bytes[index];
                    if next.is_ascii_alphanumeric() || next == b'_' || next == b':' {
                        index += 1;
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Ident(trimmed[start..index].to_ascii_lowercase()));
            }
            _ => {
                return Err(DslError::Evaluation(format!(
                    "unexpected character `{}`",
                    trimmed.get(index..=index).unwrap_or("")
                )));
            }
        }
    }

    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            cursor: 0,
        }
    }

    fn parse(mut self) -> Result<DslExpression, DslError> {
        let expr = self.parse_expression()?;
        if !self.is_at_end() {
            return Err(DslError::UnexpectedToken {
                expected: "end of expression",
                got: self.peek_kind(),
            });
        }
        Ok(expr)
    }

    fn parse_expression(&mut self) -> Result<DslExpression, DslError> {
        let mut left = self.parse_term()?;
        while self.match_any(&[Token::Plus, Token::Minus]) {
            let op = match self.previous() {
                Token::Plus => '+',
                Token::Minus => '-',
                _ => unreachable!(),
            };
            let right = self.parse_term()?;
            left = DslExpression::BinaryOp(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<DslExpression, DslError> {
        let mut left = self.parse_factor()?;
        while self.match_any(&[Token::Star, Token::Slash]) {
            let op = match self.previous() {
                Token::Star => '*',
                Token::Slash => '/',
                _ => unreachable!(),
            };
            let right = self.parse_factor()?;
            left = DslExpression::BinaryOp(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<DslExpression, DslError> {
        if self.match_token(Token::Minus) {
            let inner = self.parse_factor()?;
            return Ok(DslExpression::BinaryOp(
                Box::new(DslExpression::Literal(0.0)),
                '-',
                Box::new(inner),
            ));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<DslExpression, DslError> {
        if let Some(Token::Number(value)) = self.peek().cloned() {
            self.advance();
            return Ok(DslExpression::Literal(value));
        }

        if let Some(Token::Ident(name)) = self.peek().cloned() {
            self.advance();
            if self.match_token(Token::LParen) {
                let args = self.parse_argument_list()?;
                self.expect(TokenKind::RParen, "')'")?;
                return Ok(DslExpression::FunctionCall { name, args });
            }
            return Ok(DslExpression::Variable(name));
        }

        if self.match_token(Token::LParen) {
            let expr = self.parse_expression()?;
            self.expect(TokenKind::RParen, "')'")?;
            return Ok(expr);
        }

        Err(DslError::UnexpectedToken {
            expected: "number, identifier, or '('",
            got: self.peek_kind(),
        })
    }

    fn parse_argument_list(&mut self) -> Result<Vec<DslExpression>, DslError> {
        if self.check(TokenKind::RParen) {
            return Ok(Vec::new());
        }
        let mut args = vec![self.parse_expression()?];
        while self.match_token(Token::Comma) {
            args.push(self.parse_expression()?);
        }
        Ok(args)
    }

    fn match_token(&mut self, token: Token) -> bool {
        if let Token::Number(_) = token {
            if matches!(self.peek(), Some(Token::Number(_))) {
                self.advance();
                return true;
            }
            return false;
        }
        if self.peek() == Some(&token) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn match_any(&mut self, candidates: &[Token]) -> bool {
        candidates
            .iter()
            .any(|candidate| self.match_token(candidate.clone()))
    }

    fn expect(&mut self, kind: TokenKind, expected: &'static str) -> Result<(), DslError> {
        if self.check(kind) {
            self.advance();
            Ok(())
        } else {
            Err(DslError::UnexpectedToken {
                expected,
                got: self.peek_kind(),
            })
        }
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.peek_kind() == kind
    }

    fn advance(&mut self) {
        if !self.is_at_end() {
            self.cursor += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        self.cursor >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }

    fn peek_kind(&self) -> TokenKind {
        self.peek().map_or(TokenKind::Eof, Token::kind)
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.cursor - 1]
    }
}

pub fn parse(input: &str) -> Result<DslExpression, DslError> {
    let tokens = tokenize(input)?;
    Parser::new(tokens).parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_parses_operators_and_identifiers() {
        let tokens = tokenize("close + 2.5 * rsi(14)").unwrap();
        assert!(matches!(tokens[0], Token::Ident(ref name) if name == "close"));
        assert!(matches!(tokens[1], Token::Plus));
        assert!(matches!(tokens[2], Token::Number(_)));
    }

    #[test]
    fn parse_respects_multiplication_precedence() {
        let expr = parse("1 + 2 * 3").unwrap();
        assert_eq!(
            expr,
            DslExpression::BinaryOp(
                Box::new(DslExpression::Literal(1.0)),
                '+',
                Box::new(DslExpression::BinaryOp(
                    Box::new(DslExpression::Literal(2.0)),
                    '*',
                    Box::new(DslExpression::Literal(3.0)),
                )),
            )
        );
    }
}
