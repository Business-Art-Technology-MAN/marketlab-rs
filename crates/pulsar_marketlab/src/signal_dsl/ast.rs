//! OTL abstract syntax tree nodes.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum DslExpression {
    Literal(f32),
    Variable(String),
    BinaryOp(Box<DslExpression>, char, Box<DslExpression>),
    FunctionCall {
        name: String,
        args: Vec<DslExpression>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DslError {
    EmptyInput,
    UnexpectedEof,
    UnexpectedToken { expected: &'static str, got: super::parser::TokenKind },
    UnknownVariable(String),
    UnknownFunction(String),
    InvalidArgumentCount { name: String, expected: usize, got: usize },
    EmptyWindow,
    DivisionByZero,
    Evaluation(String),
}

impl fmt::Display for DslError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DslError::EmptyInput => write!(f, "expression must not be empty"),
            DslError::UnexpectedEof => write!(f, "unexpected end of expression"),
            DslError::UnexpectedToken { expected, got } => {
                write!(f, "expected {expected}, found {got}")
            }
            DslError::UnknownVariable(name) => write!(f, "unknown variable `{name}`"),
            DslError::UnknownFunction(name) => write!(f, "unknown function `{name}`"),
            DslError::InvalidArgumentCount { name, expected, got } => write!(
                f,
                "function `{name}` expects {expected} argument(s), got {got}"
            ),
            DslError::EmptyWindow => write!(f, "market window is empty"),
            DslError::DivisionByZero => write!(f, "division by zero"),
            DslError::Evaluation(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for DslError {}
