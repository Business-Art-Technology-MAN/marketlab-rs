//! OTL object declaration parser.

use thiserror::Error;

use super::ast::{
    OtlObjectDeclaration, OtlProgram, OtlType, PortDirection, PropertyDeclaration, Statement,
};
use super::lexer::{object_kind_from_token, Token};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("expected object kind (signal, allocator, portfolio, shader)")]
    ExpectedObjectKind,
    #[error("expected identifier")]
    ExpectedIdent,
    #[error("expected `{0}`")]
    ExpectedToken(&'static str),
    #[error("unsupported legacy script without object declaration")]
    LegacyScriptOnly,
}

pub fn parse_program(source: &str) -> Result<OtlProgram, ParseError> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Ok(OtlProgram::default());
    }
    let tokens = super::lexer::tokenize(trimmed);
    if !starts_with_object_kind(&tokens) {
        return Ok(OtlProgram {
            objects: vec![super::canonical::ingest_unwrapped_source(trimmed)],
        });
    }
    if matches!(
        tokens.iter().find(|token| !matches!(token, Token::Eof)),
        Some(Token::Shader)
    ) && super::canonical::shader_uses_osl_parameter_syntax(trimmed)
    {
        return Ok(OtlProgram {
            objects: vec![super::canonical::shader_object_from_osl_source(trimmed)],
        });
    }
    let mut parser = Parser::new(tokens, trimmed.to_string());
    let mut objects = Vec::new();
    while !parser.at_eof() {
        objects.push(parser.parse_object()?);
    }
    Ok(OtlProgram { objects })
}

fn starts_with_object_kind(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .find(|token| !matches!(token, Token::Eof))
        .and_then(object_kind_from_token)
        .is_some()
}

struct Parser {
    tokens: Vec<Token>,
    source: String,
    source_scan_from: usize,
    index: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>, source: String) -> Self {
        Self {
            tokens,
            source,
            source_scan_from: 0,
            index: 0,
        }
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.index).unwrap_or(&Token::Eof)
    }

    fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if !matches!(token, Token::Eof) {
            self.index += 1;
        }
        token
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.bump() {
            Token::Ident(name) => Ok(name),
            // Port and object names may reuse tier keywords (e.g. `output float signal`).
            Token::Signal => Ok("signal".to_string()),
            Token::Allocator => Ok("allocator".to_string()),
            Token::Portfolio => Ok("portfolio".to_string()),
            Token::Shader => Ok("shader".to_string()),
            Token::Return => Ok("return".to_string()),
            Token::Eof => Err(ParseError::UnexpectedEof),
            _ => Err(ParseError::ExpectedIdent),
        }
    }

    fn parse_object(&mut self) -> Result<OtlObjectDeclaration, ParseError> {
        let kind = object_kind_from_token(self.peek()).ok_or(ParseError::ExpectedObjectKind)?;
        self.bump();
        let name = self.expect_ident()?;
        self.expect(Token::LParen)?;
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        while !matches!(self.peek(), Token::RParen | Token::Eof) {
            let direction = match self.bump() {
                Token::Input => PortDirection::Input,
                Token::Output => PortDirection::Output,
                Token::Eof => return Err(ParseError::UnexpectedEof),
                _ => return Err(ParseError::ExpectedToken("input or output")),
            };
            let ty = self.parse_type()?;
            let port_name = self.expect_ident()?;
            let default_value = self.parse_optional_port_default()?;
            let property = PropertyDeclaration {
                direction,
                ty,
                name: port_name,
                default_value,
            };
            match direction {
                PortDirection::Input => inputs.push(property),
                PortDirection::Output => outputs.push(property),
            }
            if matches!(self.peek(), Token::Comma) {
                self.bump();
            }
        }
        self.expect(Token::RParen)?;
        self.expect(Token::LBrace)?;
        let body = self.parse_body_raw()?;
        while !matches!(self.peek(), Token::RBrace | Token::Eof) {
            self.bump();
        }
        self.expect(Token::RBrace)?;
        Ok(OtlObjectDeclaration {
            kind,
            name,
            inputs,
            outputs,
            body,
        })
    }

    fn parse_optional_port_default(&mut self) -> Result<Option<f64>, ParseError> {
        if !matches!(self.peek(), Token::Assign) {
            return Ok(None);
        }
        self.bump();
        match self.bump() {
            Token::Ident(raw) => raw
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Some)
                .ok_or(ParseError::ExpectedToken("numeric default")),
            Token::Eof => Err(ParseError::UnexpectedEof),
            _ => Err(ParseError::ExpectedToken("numeric default")),
        }
    }

    fn parse_type(&mut self) -> Result<OtlType, ParseError> {
        let base = match self.bump() {
            Token::FloatType => OtlType::Float,
            Token::IntType => OtlType::Int,
            Token::StringType => OtlType::String,
            Token::ClosureType => OtlType::Closure,
            Token::Eof => return Err(ParseError::UnexpectedEof),
            _ => return Err(ParseError::ExpectedToken("type")),
        };
        if matches!(self.peek(), Token::LBracket) {
            self.bump();
            self.expect(Token::RBracket)?;
            return Ok(OtlType::ClosureArray);
        }
        Ok(base)
    }

    fn parse_body_raw(&mut self) -> Result<Vec<Statement>, ParseError> {
        let slice = &self.source[self.source_scan_from..];
        let open = slice.find('{').ok_or(ParseError::UnexpectedEof)?;
        let body = super::canonical::extract_braced_body(&slice[open..])
            .ok_or(ParseError::UnexpectedEof)?;
        let close = open + body.len() + 2;
        self.source_scan_from += close;
        Ok(super::canonical::parse_body_assignments(&body))
    }

    fn expect(&mut self, expected: Token) -> Result<(), ParseError> {
        if self.peek() == &expected {
            self.bump();
            Ok(())
        } else {
            Err(ParseError::ExpectedToken(match expected {
                Token::LParen => "(",
                Token::RParen => ")",
                Token::LBrace => "{",
                Token::RBrace => "}",
                Token::Semicolon => ";",
                Token::Assign => "=",
                Token::RBracket => "]",
                _ => "token",
            }))
        }
    }
}
