//! Phase C Pillar 1 — OSL-inspired signal expression frontend.

mod ast;
pub(crate) mod financial;
mod interpreter;
#[cfg(test)]
mod mock_provider;
mod parser;
mod vector;

pub mod services;

pub use ast::{DslError, DslExpression};
pub use interpreter::{
    compile, compile_formula, evaluate_formula, invoke_closure, CompileContext,
    DEFAULT_CLOSE_PATH,
};
pub use parser::{parse, tokenize, Token, TokenKind};
pub use services::{MarketProviderServices, OtlClosure};
pub use vector::Vector;

#[cfg(test)]
pub use mock_provider::MockMarketProvider;
