//! OTL script lexer, AST parser, and vectorized series evaluator for `inputs:script_src`.

use std::fmt;
use std::sync::Arc;

use thiserror::Error;

/// Compiled OTL transform: maps an upstream price window to an output series.
pub type SeriesClosure = Box<dyn Fn(&[f64]) -> Vec<f64> + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal(f64),
    Input,
    Binary(Box<Expr>, BinOp, Box<Expr>),
    Call { name: String, args: Vec<Expr> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Number(f64),
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

#[derive(Debug, Error, PartialEq)]
pub enum CompileError {
    #[error("expression must not be empty")]
    EmptyInput,
    #[error("expected {expected}, found {got}")]
    UnexpectedToken { expected: &'static str, got: TokenKind },
    #[error("unknown identifier `{0}`")]
    UnknownIdentifier(String),
    #[error("unknown function `{0}`")]
    UnknownFunction(String),
    #[error("function `{name}` expects {expected} argument(s), got {got}")]
    InvalidArgumentCount {
        name: String,
        expected: usize,
        got: usize,
    },
    #[error("expected literal {label}, got expression")]
    ExpectedLiteral { label: &'static str },
    #[error("period must be positive")]
    InvalidPeriod,
    #[error("division by zero in series math")]
    DivisionByZero,
    #[error("{0}")]
    Evaluation(String),
}

/// Tokenize an OTL expression string from `inputs:script_src`.
pub fn tokenize(input: &str) -> Result<Vec<Token>, CompileError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CompileError::EmptyInput);
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
                    .parse::<f64>()
                    .map_err(|_| CompileError::Evaluation(format!("invalid number `{slice}`")))?;
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
                return Err(CompileError::Evaluation(format!(
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

    fn parse(mut self) -> Result<Expr, CompileError> {
        let expr = self.parse_expression()?;
        if !self.is_at_end() {
            return Err(CompileError::UnexpectedToken {
                expected: "end of expression",
                got: self.peek_kind(),
            });
        }
        Ok(expr)
    }

    fn parse_expression(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_term()?;
        while self.match_any(&[Token::Plus, Token::Minus]) {
            let op = match self.previous() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => unreachable!(),
            };
            let right = self.parse_term()?;
            left = Expr::Binary(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_factor()?;
        while self.match_any(&[Token::Star, Token::Slash]) {
            let op = match self.previous() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                _ => unreachable!(),
            };
            let right = self.parse_factor()?;
            left = Expr::Binary(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Expr, CompileError> {
        if self.match_token(Token::Minus) {
            let inner = self.parse_factor()?;
            return Ok(Expr::Binary(
                Box::new(Expr::Literal(0.0)),
                BinOp::Sub,
                Box::new(inner),
            ));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        if let Some(Token::Number(value)) = self.peek().cloned() {
            self.advance();
            return Ok(Expr::Literal(value));
        }

        if let Some(Token::Ident(name)) = self.peek().cloned() {
            self.advance();
            if is_input_ident(&name) {
                if self.match_token(Token::LParen) {
                    return Err(CompileError::Evaluation(format!(
                        "input binding `{name}` is not callable"
                    )));
                }
                return Ok(Expr::Input);
            }
            if self.match_token(Token::LParen) {
                let args = self.parse_argument_list()?;
                self.expect(TokenKind::RParen, "')'")?;
                return Ok(Expr::Call { name, args });
            }
            return Err(CompileError::UnknownIdentifier(name));
        }

        if self.match_token(Token::LParen) {
            let expr = self.parse_expression()?;
            self.expect(TokenKind::RParen, "')'")?;
            return Ok(expr);
        }

        Err(CompileError::UnexpectedToken {
            expected: "number, identifier, or '('",
            got: self.peek_kind(),
        })
    }

    fn parse_argument_list(&mut self) -> Result<Vec<Expr>, CompileError> {
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

    fn expect(&mut self, kind: TokenKind, expected: &'static str) -> Result<(), CompileError> {
        if self.check(kind) {
            self.advance();
            Ok(())
        } else {
            Err(CompileError::UnexpectedToken {
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

fn is_input_ident(name: &str) -> bool {
    matches!(name, "data" | "input" | "close" | "price" | "x")
}

/// Parse an OTL expression into an AST.
pub fn parse(input: &str) -> Result<Expr, CompileError> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("identity") {
        return Ok(Expr::Call {
            name: "identity".to_string(),
            args: Vec::new(),
        });
    }
    let tokens = tokenize(input)?;
    Parser::new(tokens).parse()
}

/// Compile a parsed AST into a vectorized runtime closure.
pub fn compile(expr: &Expr) -> Result<SeriesClosure, CompileError> {
    let inner = compile_expr(expr)?;
    Ok(Box::new(move |input| inner(Arc::new(input.to_vec()))))
}

/// Parse and compile `inputs:script_src` in one step.
pub fn compile_script(source: &str) -> Result<SeriesClosure, CompileError> {
    compile(&parse(source)?)
}

fn compile_expr(expr: &Expr) -> Result<Arc<dyn Fn(Arc<Vec<f64>>) -> Vec<f64> + Send + Sync>, CompileError> {
    match expr {
        Expr::Literal(value) => {
            let scalar = *value;
            Ok(Arc::new(move |input| vec![scalar; input.len()]))
        }
        Expr::Input => Ok(Arc::new(|input| (*input).clone())),
        Expr::Binary(left, op, right) => {
            let left = compile_expr(left)?;
            let right = compile_expr(right)?;
            let op = *op;
            Ok(Arc::new(move |input| {
                let lhs = left(input.clone());
                let rhs = right(input);
                zip_binary(&lhs, &rhs, op)
            }))
        }
        Expr::Call { name, args } => compile_call(name, args),
    }
}

fn compile_call(
    name: &str,
    args: &[Expr],
) -> Result<Arc<dyn Fn(Arc<Vec<f64>>) -> Vec<f64> + Send + Sync>, CompileError> {
    match name {
        "sma" | "ta::sma" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| sma(&series(input.clone()), period)))
        }
        "macd" | "ta::macd" => {
            if args.len() != 3 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 3,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let short = literal_usize(&args[1], "short period")?;
            let long = literal_usize(&args[2], "long period")?;
            Ok(Arc::new(move |input| {
                let (macd_line, _) = macd(&series(input.clone()), short, long);
                macd_line
            }))
        }
        "cross" | "ta::cross" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let left = compile_expr(&args[0])?;
            let right = compile_expr(&args[1])?;
            Ok(Arc::new(move |input| {
                let a = left(input.clone());
                let b = right(input);
                cross(&a, &b)
            }))
        }
        "identity" => {
            if !args.is_empty() {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 0,
                    got: args.len(),
                });
            }
            Ok(Arc::new(|input| (*input).clone()))
        }
        other => Err(CompileError::UnknownFunction(other.to_string())),
    }
}

fn literal_usize(expr: &Expr, label: &'static str) -> Result<usize, CompileError> {
    match expr {
        Expr::Literal(value) => {
            if *value <= 0.0 || !value.is_finite() || value.fract() != 0.0 {
                return Err(CompileError::InvalidPeriod);
            }
            Ok(*value as usize)
        }
        _ => Err(CompileError::ExpectedLiteral { label }),
    }
}

fn zip_binary(left: &[f64], right: &[f64], op: BinOp) -> Vec<f64> {
    let len = left.len().max(right.len());
    let mut out = Vec::with_capacity(len);
    for index in 0..len {
        let lhs = *left.get(index).unwrap_or(&f64::NAN);
        let rhs = *right.get(index).unwrap_or(&f64::NAN);
        out.push(match op {
            BinOp::Add => lhs + rhs,
            BinOp::Sub => lhs - rhs,
            BinOp::Mul => lhs * rhs,
            BinOp::Div => {
                if rhs.abs() <= f64::EPSILON {
                    f64::NAN
                } else {
                    lhs / rhs
                }
            }
        });
    }
    out
}

/// Simple moving average over `data` with lookback `period`.
pub fn sma(data: &[f64], period: usize) -> Vec<f64> {
    if period == 0 || data.is_empty() {
        return Vec::new();
    }
    let mut out = vec![f64::NAN; data.len()];
    for index in (period - 1)..data.len() {
        let window = &data[index + 1 - period..=index];
        out[index] = window.iter().sum::<f64>() / period as f64;
    }
    out
}

/// MACD line and slow EMA component.
pub fn macd(data: &[f64], short: usize, long: usize) -> (Vec<f64>, Vec<f64>) {
    let fast = ema(data, short.max(1));
    let slow = ema(data, long.max(1));
    let macd_line = fast
        .iter()
        .zip(&slow)
        .map(|(fast_value, slow_value)| fast_value - slow_value)
        .collect();
    (macd_line, slow)
}

/// Crossover detector: `1.0` bullish cross, `-1.0` bearish cross, else `0.0`.
pub fn cross(a: &[f64], b: &[f64]) -> Vec<f64> {
    let len = a.len().min(b.len());
    let mut out = vec![0.0; len];
    for index in 1..len {
        let prev_a = a[index - 1];
        let prev_b = b[index - 1];
        let curr_a = a[index];
        let curr_b = b[index];
        if prev_a <= prev_b && curr_a > curr_b {
            out[index] = 1.0;
        } else if prev_a >= prev_b && curr_a < curr_b {
            out[index] = -1.0;
        }
    }
    out
}

fn ema(data: &[f64], period: usize) -> Vec<f64> {
    if data.is_empty() {
        return Vec::new();
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut out = Vec::with_capacity(data.len());
    let mut state = data[0];
    out.push(state);
    for &value in &data[1..] {
        state = alpha * value + (1.0 - alpha) * state;
        out.push(state);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_parses_window_functions() {
        let tokens = tokenize("cross(sma(data, 10), sma(data, 30))").unwrap();
        assert!(matches!(tokens.first(), Some(Token::Ident(name)) if name == "cross"));
    }

    #[test]
    fn parse_respects_multiplication_precedence() {
        let expr = parse("data + 2 * 3").unwrap();
        assert_eq!(
            expr,
            Expr::Binary(
                Box::new(Expr::Input),
                BinOp::Add,
                Box::new(Expr::Binary(
                    Box::new(Expr::Literal(2.0)),
                    BinOp::Mul,
                    Box::new(Expr::Literal(3.0)),
                )),
            )
        );
    }

    #[test]
    fn sma_warmup_leaves_prefix_nan() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let out = sma(&data, 3);
        assert!(out[0].is_nan());
        assert!(out[1].is_nan());
        assert_eq!(out[2], 2.0);
        assert_eq!(out[4], 4.0);
    }

    #[test]
    fn macd_returns_equal_length_vectors() {
        let data = (0..20).map(|v| v as f64).collect::<Vec<_>>();
        let (line, slow) = macd(&data, 3, 5);
        assert_eq!(line.len(), data.len());
        assert_eq!(slow.len(), data.len());
    }

    #[test]
    fn cross_detects_bullish_crossover() {
        let a = vec![0.0, 0.0, 2.0];
        let b = vec![1.0, 1.0, 1.0];
        let out = cross(&a, &b);
        assert_eq!(out, vec![0.0, 0.0, 1.0]);
    }

    #[test]
    fn compile_script_runs_sma_pipeline() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let closure = compile_script("sma(data, 3)").expect("compile");
        let out = closure(&data);
        assert_eq!(out.len(), data.len());
        assert!(out[1].is_nan());
        assert_eq!(out[2], 2.0);
    }

    #[test]
    fn compile_script_supports_cross_of_averages() {
        let data = (0..40).map(|v| v as f64).collect::<Vec<_>>();
        let closure = compile_script("cross(sma(data, 5), sma(data, 10))").expect("compile");
        let out = closure(&data);
        assert_eq!(out.len(), data.len());
    }
}
