//! OTL object-declaration lexer (`signal`, `allocator`, `portfolio`).

use super::ast::OtlObjectKind;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Signal,
    Allocator,
    Portfolio,
    Shader,
    Input,
    Output,
    Return,
    Ident(String),
    FloatType,
    IntType,
    StringType,
    ClosureType,
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Semicolon,
    Comma,
    Assign,
    Eof,
}

pub fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\r' | '\n' => {}
            '/' if chars.peek() == Some(&'/') => {
                while chars.next().is_some_and(|c| c != '\n') {}
            }
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            '{' => tokens.push(Token::LBrace),
            '}' => tokens.push(Token::RBrace),
            '[' => tokens.push(Token::LBracket),
            ']' => tokens.push(Token::RBracket),
            ';' => tokens.push(Token::Semicolon),
            ',' => tokens.push(Token::Comma),
            '=' => tokens.push(Token::Assign),
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut ident = String::new();
                ident.push(c);
                while chars
                    .peek()
                    .is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_' || *next == ':')
                {
                    ident.push(chars.next().expect("peeked"));
                }
                tokens.push(keyword_or_ident(&ident));
            }
            c if c.is_ascii_digit() => {
                let mut number = String::new();
                number.push(c);
                while chars.peek().is_some_and(|next| next.is_ascii_digit() || *next == '.') {
                    number.push(chars.next().expect("peeked"));
                }
                tokens.push(Token::Ident(number));
            }
            _ => {}
        }
    }
    tokens.push(Token::Eof);
    tokens
}

fn keyword_or_ident(ident: &str) -> Token {
    match ident.to_ascii_lowercase().as_str() {
        "signal" => Token::Signal,
        "allocator" => Token::Allocator,
        "portfolio" => Token::Portfolio,
        "shader" => Token::Shader,
        "input" => Token::Input,
        "output" => Token::Output,
        "return" => Token::Return,
        "float" => Token::FloatType,
        "int" => Token::IntType,
        "string" => Token::StringType,
        "closure" => Token::ClosureType,
        other => Token::Ident(other.to_string()),
    }
}

pub fn object_kind_from_token(token: &Token) -> Option<OtlObjectKind> {
    match token {
        Token::Signal => Some(OtlObjectKind::Signal),
        Token::Allocator => Some(OtlObjectKind::Allocator),
        Token::Portfolio => Some(OtlObjectKind::Portfolio),
        Token::Shader => Some(OtlObjectKind::LegacyShader),
        _ => None,
    }
}
