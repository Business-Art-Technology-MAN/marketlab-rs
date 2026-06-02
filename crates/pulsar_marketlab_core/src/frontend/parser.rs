//! OTL object declaration parser.

use thiserror::Error;

use super::ast::{
    OtlObjectDeclaration, OtlObjectKind, OtlProgram, OtlType, PortDirection, PropertyDeclaration,
    Statement,
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
            objects: vec![legacy_shader_from_source(trimmed)],
        });
    }
    let mut parser = Parser::new(tokens);
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

fn legacy_shader_from_source(source: &str) -> OtlObjectDeclaration {
    let signature = crate::parse_script_signature(source);
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for name in signature.inputs {
        inputs.push(PropertyDeclaration {
            direction: PortDirection::Input,
            ty: if inputs.is_empty() {
                OtlType::Float
            } else {
                OtlType::Int
            },
            name,
        });
    }
    for name in signature.outputs {
        outputs.push(PropertyDeclaration {
            direction: PortDirection::Output,
            ty: OtlType::Closure,
            name,
        });
    }
    OtlObjectDeclaration {
        kind: OtlObjectKind::LegacyShader,
        name: crate::display_name_for_script(source, "legacy_shader"),
        inputs,
        outputs,
        body: vec![Statement::Raw {
            text: source.to_string(),
        }],
    }
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, index: 0 }
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
            let property = PropertyDeclaration {
                direction,
                ty,
                name: port_name,
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
        let body = self.parse_body()?;
        self.expect(Token::RBrace)?;
        Ok(OtlObjectDeclaration {
            kind,
            name,
            inputs,
            outputs,
            body,
        })
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

    fn parse_body(&mut self) -> Result<Vec<Statement>, ParseError> {
        let mut statements = Vec::new();
        while !matches!(self.peek(), Token::RBrace | Token::Eof) {
            if matches!(self.peek(), Token::Return) {
                self.bump();
                let expr = self.parse_until(&[Token::Semicolon, Token::RBrace])?;
                if matches!(self.peek(), Token::Semicolon) {
                    self.bump();
                }
                statements.push(Statement::Return { expr });
                continue;
            }
            let target = self.expect_ident()?;
            self.expect(Token::Assign)?;
            let expr = self.parse_until(&[Token::Semicolon, Token::RBrace])?;
            if matches!(self.peek(), Token::Semicolon) {
                self.bump();
            }
            statements.push(Statement::Assign { target, expr });
        }
        Ok(statements)
    }

    fn parse_until(&mut self, stop: &[Token]) -> Result<String, ParseError> {
        let start = self.index;
        while !matches!(self.peek(), Token::Eof) && !stop.iter().any(|token| self.peek() == token)
        {
            self.bump();
        }
        let mut rendered = String::new();
        for token in &self.tokens[start..self.index] {
            use Token::*;
            match token {
                Ident(text) => rendered.push_str(text),
                Signal => rendered.push_str("signal"),
                Allocator => rendered.push_str("allocator"),
                Portfolio => rendered.push_str("portfolio"),
                Shader => rendered.push_str("shader"),
                Input => rendered.push_str("input"),
                Output => rendered.push_str("output"),
                Return => rendered.push_str("return"),
                FloatType => rendered.push_str("float"),
                IntType => rendered.push_str("int"),
                StringType => rendered.push_str("string"),
                ClosureType => rendered.push_str("closure"),
                LBracket => rendered.push('['),
                RBracket => rendered.push(']'),
                LParen => rendered.push('('),
                RParen => rendered.push(')'),
                LBrace => rendered.push('{'),
                RBrace => rendered.push('}'),
                Semicolon => rendered.push(';'),
                Comma => rendered.push(','),
                Assign => rendered.push('='),
                Eof => {}
            }
            rendered.push(' ');
        }
        Ok(rendered.trim().to_string())
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
