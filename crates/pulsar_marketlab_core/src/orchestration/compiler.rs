//! OTL script lexer, AST parser, and vectorized series evaluator for `inputs:script_src`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use thiserror::Error;

/// Compiled OTL transform: maps an upstream price window to an output series.
pub type SeriesClosure = Box<dyn Fn(&[f64]) -> Vec<f64> + Send + Sync>;

/// Multi-channel OTL transform (AOV ports); channel order matches archetype port order.
pub type MultiSeriesClosure = Box<dyn Fn(&[f64]) -> Vec<Vec<f64>> + Send + Sync>;

/// Compiled OTL transform: single series or fixed multi-AOV channels.
pub enum CompiledSeries {
    Single(SeriesClosure),
    Multi(MultiSeriesClosure, Vec<String>),
}

impl CompiledSeries {
    pub fn is_multi(&self) -> bool {
        matches!(self, Self::Multi(..))
    }
}

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
    compile_ctx: ScriptCompileContext,
}

impl Parser {
    fn new(tokens: Vec<Token>, compile_ctx: ScriptCompileContext) -> Self {
        Self {
            tokens,
            cursor: 0,
            compile_ctx,
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
            let key = name.to_ascii_lowercase();
            if self.compile_ctx.series_inputs.contains(&key) || is_input_ident(&name) {
                if self.match_token(Token::LParen) {
                    return Err(CompileError::Evaluation(format!(
                        "input binding `{name}` is not callable"
                    )));
                }
                return Ok(Expr::Input);
            }
            if let Some(&scalar) = self.compile_ctx.scalar_params.get(&key) {
                if self.match_token(Token::LParen) {
                    return Err(CompileError::Evaluation(format!(
                        "scalar parameter `{name}` is not callable"
                    )));
                }
                return Ok(Expr::Literal(scalar));
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
    matches!(
        name,
        "data" | "input" | "close" | "price" | "x" | "source" | "source_stream"
    )
}

/// Parse an OTL expression into an AST.
pub fn parse(input: &str) -> Result<Expr, CompileError> {
    parse_with_context(input, &ScriptCompileContext::default())
}

/// Parse an OTL expression using OSL signature scalar bindings and series input names.
pub fn parse_with_context(input: &str, ctx: &ScriptCompileContext) -> Result<Expr, CompileError> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("identity") {
        return Ok(Expr::Call {
            name: "identity".to_string(),
            args: Vec::new(),
        });
    }
    let tokens = tokenize(input)?;
    Parser::new(tokens, ctx.clone()).parse()
}

/// Compile a parsed AST into a vectorized runtime closure.
pub fn compile(expr: &Expr) -> Result<SeriesClosure, CompileError> {
    let inner = compile_expr(expr)?;
    Ok(Box::new(move |input| inner(Arc::new(input.to_vec()))))
}

/// Shallow scan of OTL script source for dynamic canvas port synthesis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptSignature {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub parameters: Vec<OslParameter>,
}

/// OSL parameter type from a shader signature header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OslParamType {
    Float,
    Int,
    String,
}

/// One typed parameter from an OSL shader signature.
#[derive(Clone, Debug, PartialEq)]
pub struct OslParameter {
    pub name: String,
    pub ty: OslParamType,
    pub is_output: bool,
    pub default_value: Option<f64>,
}

/// Compile-time symbol bindings for OSL shader scalar inputs and series streams.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptCompileContext {
    pub series_inputs: HashSet<String>,
    pub scalar_params: HashMap<String, f64>,
}

impl ScriptCompileContext {
    pub fn from_script_source(source: &str) -> Self {
        let mut ctx = Self::default();
        ctx.register_canonical_series_inputs();
        let stripped = strip_line_comments(source);
        let parameters = collect_osl_parameters(&stripped);
        if parameters.is_empty() {
            let legacy = parse_legacy_fn_main_signature(&stripped);
            ctx.apply_legacy_fn_main_inputs(&legacy.inputs);
            if let Some(body) = extract_fn_main_body(&stripped) {
                ctx.apply_body_scalar_assignments(&body);
            }
            return ctx;
        }
        ctx.apply_osl_parameters(&parameters);
        if let Some(body) = extract_osl_shader_body(&stripped) {
            ctx.apply_body_scalar_assignments(&body);
        }
        ctx
    }

    pub fn with_scalar_overrides(
        mut self,
        overrides: impl IntoIterator<Item = (impl Into<String>, f64)>,
    ) -> Self {
        for (name, value) in overrides {
            let key = name.into().to_ascii_lowercase();
            if self.scalar_params.contains_key(&key) {
                self.scalar_params.insert(key, value);
            }
        }
        self
    }

    fn register_canonical_series_inputs(&mut self) {
        for name in ["data", "input", "close", "price", "x", "source", "source_stream"] {
            self.series_inputs.insert(name.to_string());
        }
    }

    fn apply_osl_parameters(&mut self, parameters: &[OslParameter]) {
        let mut primary_series_registered = false;
        for param in parameters.iter().filter(|param| !param.is_output) {
            let key = param.name.to_ascii_lowercase();
            match param.ty {
                OslParamType::Float if !primary_series_registered => {
                    self.series_inputs.insert(key);
                    primary_series_registered = true;
                }
                OslParamType::Float | OslParamType::Int => {
                    let fallback = match param.ty {
                        OslParamType::Int => 14.0,
                        OslParamType::Float => 1.0,
                        OslParamType::String => continue,
                    };
                    self.scalar_params
                        .insert(key, param.default_value.unwrap_or(fallback));
                }
                OslParamType::String => {}
            }
        }
    }

    fn apply_legacy_fn_main_inputs(&mut self, inputs: &[String]) {
        for (index, name) in inputs.iter().enumerate() {
            let key = name.to_ascii_lowercase();
            if index == 0 {
                self.series_inputs.insert(key);
            } else {
                self.scalar_params.entry(key).or_insert(14.0);
            }
        }
    }

    fn apply_body_scalar_assignments(&mut self, body: &str) {
        for line in body.lines() {
            let Some((name, value)) = parse_simple_scalar_assignment(line.trim()) else {
                continue;
            };
            let key = name.to_ascii_lowercase();
            if self.series_inputs.contains(&key) {
                continue;
            }
            if self.scalar_params.contains_key(&key) {
                self.scalar_params.insert(key, value);
            }
        }
    }
}

/// OSL scalar type keywords recognized in shader parameter lists.
const OSL_TYPES: &[&str] = &["float", "int", "string"];

/// Locate typed OSL/C-style shader parameters and `output` port declarations.
pub fn parse_script_signature(source: &str) -> ScriptSignature {
    if let Some(signature) = parse_osl_shader_signature(source) {
        if !signature.inputs.is_empty() || !signature.outputs.is_empty() {
            return signature;
        }
    }
    parse_legacy_fn_main_signature(source)
}

/// OSL / legacy entry point identifier (`adaptive_trigger`, `main`, …).
pub fn parse_script_entry_point_name(source: &str) -> Option<String> {
    extract_void_function_name(source).or_else(|| {
        if strip_line_comments(source)
            .to_ascii_lowercase()
            .contains("fn main")
        {
            Some("main".to_string())
        } else {
            None
        }
    })
}

/// Canvas / node title derived from the script entry point when present.
pub fn display_name_for_script(source: &str, fallback: &str) -> String {
    parse_script_entry_point_name(source).unwrap_or_else(|| fallback.to_string())
}

/// Non-output scalar uniforms from an OSL signature (excludes the primary float series input).
pub fn parse_script_scalar_uniforms(source: &str) -> Vec<OslParameter> {
    let parameters = collect_osl_parameters(&strip_line_comments(source));
    let mut primary_series_registered = false;
    parameters
        .into_iter()
        .filter(|param| !param.is_output)
        .filter(|param| match param.ty {
            OslParamType::Float if !primary_series_registered => {
                primary_series_registered = true;
                false
            }
            OslParamType::String => false,
            _ => true,
        })
        .collect()
}

/// Update or insert a default value for one scalar uniform in an OSL function signature.
pub fn set_script_uniform_default(source: &str, param_name: &str, value: f64) -> String {
    let Some((start, end)) = osl_function_parameter_list_span(source) else {
        return source.to_string();
    };
    let param_list = source[start..end].trim();
    let key = param_name.to_ascii_lowercase();
    let mut segments = split_top_level_commas(param_list)
        .map(|segment| segment.trim().to_string())
        .collect::<Vec<_>>();
    let mut updated = false;
    for segment in &mut segments {
        let Some(full) = parse_osl_parameter_full(segment) else {
            continue;
        };
        if full.is_output || full.name.to_ascii_lowercase() != key {
            continue;
        }
        *segment = format_osl_parameter_with_default(&full, value);
        updated = true;
        break;
    }
    if !updated {
        return source.to_string();
    }
    let mut out = String::with_capacity(source.len());
    out.push_str(&source[..start]);
    out.push_str(&segments.join(", "));
    out.push_str(&source[end..]);
    out
}

fn extract_void_function_name(source: &str) -> Option<String> {
    let lower = source.to_ascii_lowercase();
    let void_idx = lower.find("void ")?;
    let after_void = source[void_idx + 5..].trim_start();
    let open_paren = after_void.find('(')?;
    let func_name = after_void[..open_paren].trim();
    if func_name.is_empty()
        || !func_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    Some(func_name.to_string())
}

fn osl_function_parameter_list_span(source: &str) -> Option<(usize, usize)> {
    if extract_fn_main_parameter_list(source).is_some() {
        let lower = source.to_ascii_lowercase();
        let main_idx = lower.find("main")?;
        let after_main = &source[main_idx + "main".len()..];
        let open_paren = after_main.find('(')?;
        let close_paren = balanced_delimiter_offset(&after_main[open_paren..], '(', ')')?;
        let start = main_idx + "main".len() + open_paren + 1;
        let end = main_idx + "main".len() + open_paren + close_paren;
        return Some((start, end));
    }
    let lower = source.to_ascii_lowercase();
    let void_idx = lower.find("void ")?;
    let skipped = source[void_idx + 5..].len() - source[void_idx + 5..].trim_start().len();
    let base = void_idx + 5 + skipped;
    let after = &source[base..];
    let open_paren = after.find('(')?;
    let close_paren = balanced_delimiter_offset(&after[open_paren..], '(', ')')?;
    let start = base + open_paren + 1;
    let end = base + open_paren + close_paren;
    Some((start, end))
}

fn format_osl_parameter_with_default(param: &OslParameter, value: f64) -> String {
    let ty = match param.ty {
        OslParamType::Float => "float",
        OslParamType::Int => "int",
        OslParamType::String => "string",
    };
    let default = format_scalar_default(param.ty, value);
    if param.is_output {
        format!("output {ty} {}", param.name)
    } else {
        format!("{ty} {} = {default}", param.name)
    }
}

fn format_scalar_default(ty: OslParamType, value: f64) -> String {
    match ty {
        OslParamType::Int => format!("{}", value.round() as i64),
        OslParamType::Float => {
            if (value - value.round()).abs() <= f64::EPSILON {
                format!("{:.1}", value)
            } else {
                value.to_string()
            }
        }
        OslParamType::String => value.to_string(),
    }
}

fn parse_osl_shader_signature(source: &str) -> Option<ScriptSignature> {
    let param_list = extract_osl_parameter_list(source)?;
    let mut signature = ScriptSignature::default();
    for param in split_top_level_commas(param_list) {
        let Some(full) = parse_osl_parameter_full(param) else {
            continue;
        };
        signature.parameters.push(full.clone());
        if full.is_output {
            signature.outputs.push(full.name);
        } else {
            signature.inputs.push(full.name);
        }
    }
    if signature.inputs.is_empty() && signature.outputs.is_empty() {
        return None;
    }
    Some(signature)
}

fn collect_osl_parameters(source: &str) -> Vec<OslParameter> {
    let Some(param_list) = extract_osl_parameter_list(source) else {
        return Vec::new();
    };
    split_top_level_commas(param_list)
        .filter_map(parse_osl_parameter_full)
        .collect()
}

/// Parameter text for an OSL shader: inside `void name(...)` / `fn main(...)`, or a bare typed header.
fn extract_osl_parameter_list(source: &str) -> Option<&str> {
    if let Some(list) = extract_osl_function_parameter_list(source) {
        return Some(list);
    }
    let open_brace = source.find('{')?;
    let header = source[..open_brace].trim();
    if looks_like_osl_header(header) {
        return Some(header);
    }
    None
}

/// Extract comma-separated OSL parameters from `void name(...)` or legacy `fn main(...)`.
fn extract_osl_function_parameter_list(source: &str) -> Option<&str> {
    extract_fn_main_parameter_list(source).or_else(|| extract_void_function_parameter_list(source))
}

/// Legacy Rust-style `fn main(...)`.
fn extract_fn_main_parameter_list(source: &str) -> Option<&str> {
    let lower = source.to_ascii_lowercase();
    let main_idx = lower.find("main")?;
    let before = lower[..main_idx].trim();
    if !before.ends_with("fn") {
        return None;
    }
    let after_main = &source[main_idx + "main".len()..];
    let open_paren = after_main.find('(')?;
    let close_paren = balanced_delimiter_offset(&after_main[open_paren..], '(', ')')?;
    Some(after_main[open_paren + 1..open_paren + close_paren].trim())
}

/// OSL-style `void shader_name(...)` including `void main(...)`.
fn extract_void_function_parameter_list(source: &str) -> Option<&str> {
    let lower = source.to_ascii_lowercase();
    let void_idx = lower.find("void ")?;
    let after_void = source[void_idx + 5..].trim_start();
    let open_paren = after_void.find('(')?;
    let func_name = after_void[..open_paren].trim();
    if func_name.is_empty()
        || !func_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    let close_paren = balanced_delimiter_offset(&after_void[open_paren..], '(', ')')?;
    Some(after_void[open_paren + 1..open_paren + close_paren].trim())
}

fn params_end_after_void_function(source: &str) -> Option<usize> {
    let lower = source.to_ascii_lowercase();
    let void_idx = lower.find("void ")?;
    let skipped = source[void_idx + 5..].len() - source[void_idx + 5..].trim_start().len();
    let after_void = &source[void_idx + 5 + skipped..];
    let open_paren = after_void.find('(')?;
    let close_paren = balanced_delimiter_offset(&after_void[open_paren..], '(', ')')?;
    Some(void_idx + 5 + skipped + open_paren + close_paren + 1)
}

fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            line.find("//")
                .map(|idx| line[..idx].trim_end())
                .unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn looks_like_osl_header(header: &str) -> bool {
    let lower = header.to_ascii_lowercase();
    OSL_TYPES.iter().any(|ty| lower.contains(ty))
}

fn is_osl_type(token: &str) -> bool {
    OSL_TYPES.contains(&token.to_ascii_lowercase().as_str())
}

/// Parse one OSL parameter: `float source`, `int lookback = 14`, or `output float upper_band`.
fn parse_osl_parameter(param: &str) -> Option<(bool, String)> {
    parse_osl_parameter_full(param).map(|param| (param.is_output, param.name))
}

fn parse_osl_parameter_full(param: &str) -> Option<OslParameter> {
    let mut segment = param.trim().trim_end_matches(';');
    if segment.is_empty() {
        return None;
    }

    let lower = segment.to_ascii_lowercase();
    let is_output = lower.starts_with("output");
    if is_output {
        segment = segment["output".len()..].trim();
    }

    let tokens = segment.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 2 {
        return None;
    }
    let ty = match tokens[0].to_ascii_lowercase().as_str() {
        "float" => OslParamType::Float,
        "int" => OslParamType::Int,
        "string" => OslParamType::String,
        _ => return None,
    };
    let (name, default_value) = parse_osl_param_name_and_default(tokens[1..].join(" ").as_str());
    if name.is_empty() {
        return None;
    }
    Some(OslParameter {
        name,
        ty,
        is_output,
        default_value,
    })
}

fn parse_osl_param_name_and_default(rest: &str) -> (String, Option<f64>) {
    let mut parts = rest.splitn(2, '=');
    let name = parts
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches(',')
        .to_string();
    let default_value = parts
        .next()
        .and_then(|raw| parse_osl_default_literal(raw.trim().trim_end_matches(';').trim_end_matches(',')));
    (name, default_value)
}

fn parse_osl_default_literal(raw: &str) -> Option<f64> {
    raw.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn parse_simple_scalar_assignment(line: &str) -> Option<(String, f64)> {
    if line.is_empty() || line.starts_with("//") {
        return None;
    }
    let lowered = line.to_ascii_lowercase();
    if lowered.starts_with("output ")
        || lowered.starts_with("return ")
        || lowered.starts_with("float ")
        || lowered.starts_with("int ")
        || lowered.starts_with("string ")
    {
        return None;
    }
    let (lhs, rhs) = line.split_once('=')?;
    let name = lhs.trim().split_whitespace().last()?;
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    let value = rhs.trim().trim_end_matches(';').parse::<f64>().ok()?;
    Some((name.to_string(), value))
}

fn parse_legacy_fn_main_signature(source: &str) -> ScriptSignature {
    let mut signature = ScriptSignature::default();
    let Some((param_list, header_end)) = locate_fn_main_signature(source) else {
        return signature;
    };

    signature.inputs = parse_fn_main_param_idents(param_list);

    for line in source[..header_end].lines() {
        let trimmed = line.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.starts_with("output ") {
            if let Some((true, name)) = parse_osl_parameter(trimmed) {
                signature.outputs.push(name);
            }
        }
    }
    signature
}

fn locate_fn_main_signature(source: &str) -> Option<(&str, usize)> {
    let lower = source.to_ascii_lowercase();
    let fn_start = lower.find("fn main")?;
    let after_fn = &source[fn_start..];
    let open_paren = after_fn.find('(')?;
    let params_start = open_paren + 1;
    let close_paren = balanced_delimiter_offset(&after_fn[open_paren..], '(', ')')?;
    let param_list = &after_fn[params_start..open_paren + close_paren];
    let after_params = &after_fn[open_paren + close_paren + 1..];
    let open_brace = after_params.find('{')?;
    let header_end = fn_start + (open_paren + close_paren + 1) + open_brace;
    Some((param_list, header_end))
}

fn parse_fn_main_param_idents(param_list: &str) -> Vec<String> {
    split_top_level_commas(param_list)
        .filter_map(parse_param_ident)
        .collect()
}

fn parse_param_ident(param: &str) -> Option<String> {
    let param = param.trim();
    if param.is_empty() {
        return None;
    }
    let head = param.split('=').next()?.trim();
    let head = head.split(':').next()?.trim();
    let ident = head.split_whitespace().next()?.trim();
    if ident.is_empty() {
        None
    } else {
        Some(ident.to_string())
    }
}

fn split_top_level_commas(input: &str) -> impl Iterator<Item = &str> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                segments.push(input[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    segments.push(input[start..].trim());
    segments.into_iter()
}

fn balanced_delimiter_offset(input: &str, open: char, close: char) -> Option<usize> {
    if !input.starts_with(open) {
        return None;
    }
    let mut depth = 0usize;
    for (index, ch) in input.char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

/// Strip an OSL/C-style shader wrapper or legacy `fn main` block down to a compilable expression.
pub fn normalize_script_for_compile(source: &str) -> String {
    let trimmed = strip_line_comments(source).trim().to_string();

    if is_osl_shader_source(&trimmed) {
        let signature = parse_osl_shader_signature(&trimmed).unwrap_or_default();
        if let Some(body) = extract_osl_shader_body(&trimmed) {
            let body = strip_line_comments(&body);
            if let Some(expr) = extract_compilable_expression_osl(&body, &signature) {
                return expr;
            }
        }
        // Never pass a braced OSL shader body to the expression tokenizer.
        return String::new();
    }

    if trimmed.to_ascii_lowercase().contains("fn main") {
        return extract_fn_main_body(&trimmed)
            .and_then(|body| extract_compilable_expression(&strip_line_comments(&body)))
            .filter(|expr| !expr.is_empty())
            .unwrap_or_else(|| trimmed.to_string());
    }

    trimmed
}

fn is_osl_shader_source(source: &str) -> bool {
    if extract_osl_function_parameter_list(source).is_some() {
        return true;
    }
    let Some(open) = source.find('{') else {
        return false;
    };
    looks_like_osl_header(source[..open].trim())
}

fn extract_osl_shader_body(source: &str) -> Option<String> {
    let open = locate_osl_body_open_brace(source)?;
    let rest = &source[open + 1..];
    let close = balanced_closing_brace_offset(rest)?;
    Some(strip_line_comments(rest[..close].trim()))
}

/// Opening `{` of the shader body (after `void name(...)` / `fn main(...)` when present).
fn locate_osl_body_open_brace(source: &str) -> Option<usize> {
    if extract_fn_main_parameter_list(source).is_some() {
        let lower = source.to_ascii_lowercase();
        let main_idx = lower.find("main")?;
        let after_main = &source[main_idx + "main".len()..];
        let open_paren = after_main.find('(')?;
        let close_paren = balanced_delimiter_offset(&after_main[open_paren..], '(', ')')?;
        let params_end = main_idx + "main".len() + open_paren + close_paren + 1;
        return source[params_end..].find('{').map(|offset| params_end + offset);
    }
    if extract_void_function_parameter_list(source).is_some() {
        let params_end = params_end_after_void_function(source)?;
        return source[params_end..].find('{').map(|offset| params_end + offset);
    }
    source.find('{')
}

/// Extract a compilable expression from an OSL shader body (return or output assignment).
fn extract_compilable_expression_osl(body: &str, signature: &ScriptSignature) -> Option<String> {
    if let Some(expr) = extract_compilable_expression(body) {
        return Some(expr);
    }
    if let Some(expr) = find_earliest_output_assignment_rhs(body, signature) {
        return Some(expr);
    }
    find_last_assignment_rhs(body, "signal")
}

fn find_earliest_output_assignment_rhs(body: &str, signature: &ScriptSignature) -> Option<String> {
    let mut earliest: Option<(usize, String)> = None;
    for output in &signature.outputs {
        if let Some((pos, rhs)) = find_assignment_rhs_at(body, output) {
            if earliest.as_ref().map_or(true, |(best, _)| pos < *best) {
                earliest = Some((pos, rhs));
            }
        }
    }
    earliest.map(|(_, rhs)| rhs)
}

fn find_assignment_rhs_at(body: &str, target: &str) -> Option<(usize, String)> {
    let lower = body.to_ascii_lowercase();
    let target_lower = target.to_ascii_lowercase();
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find(&target_lower) {
        let start = search_from + rel;
        if start > 0 {
            let prev = body.as_bytes()[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                search_from = start + 1;
                continue;
            }
        }
        let after_name = start + target.len();
        let rest = body[after_name..].trim_start();
        if !rest.starts_with('=') {
            search_from = start + 1;
            continue;
        }
        let rhs = rest[1..].trim_start();
        return Some((
            start,
            strip_statement_terminator(take_until_statement_end(rhs)),
        ));
    }
    None
}

fn find_last_assignment_rhs(body: &str, target: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let target_lower = target.to_ascii_lowercase();
    let mut last = None;
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find(&target_lower) {
        let start = search_from + rel;
        if start > 0 {
            let prev = body.as_bytes()[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                search_from = start + 1;
                continue;
            }
        }
        let after_name = start + target.len();
        let rest = body[after_name..].trim_start();
        if !rest.starts_with('=') {
            search_from = start + 1;
            continue;
        }
        let rhs = rest[1..].trim_start();
        last = Some(strip_statement_terminator(take_until_statement_end(rhs)));
        search_from = after_name + 1;
    }
    last
}

/// Pull the final `return` expression from a block, ignoring typed locals and assignments.
fn extract_compilable_expression(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    if let Some(expr) = body.strip_prefix("return") {
        return Some(strip_statement_terminator(expr.trim()));
    }

    if let Some(expr) = find_last_return_expression(body) {
        return Some(expr);
    }

    if !contains_top_level_assignment(body) && !body.contains(';') && !starts_with_osl_type_decl(body) {
        return Some(body.to_string());
    }

    None
}

fn starts_with_osl_type_decl(statement: &str) -> bool {
    let first = statement.trim().split_whitespace().next().unwrap_or("");
    is_osl_type(first)
}

fn find_last_return_expression(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let mut last = None;
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find("return") {
        let token_start = search_from + rel;
        if token_start > 0 {
            let prev = body.as_bytes()[token_start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                search_from = token_start + "return".len();
                continue;
            }
        }
        let after_kw = token_start + "return".len();
        let rest = body[after_kw..].trim_start();
        last = Some(strip_statement_terminator(take_until_statement_end(rest)));
        search_from = after_kw + 1;
    }
    last
}

fn take_until_statement_end(input: &str) -> &str {
    let mut depth_paren = 0i32;
    let mut depth_brace = 0i32;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '{' => depth_brace += 1,
            '}' if depth_brace > 0 => depth_brace -= 1,
            ';' if depth_paren == 0 && depth_brace == 0 => return &input[..index],
            _ => {}
        }
    }
    input
}

fn strip_statement_terminator(input: &str) -> String {
    input.trim().trim_end_matches(';').trim().to_string()
}

fn contains_top_level_assignment(input: &str) -> bool {
    let mut depth_paren = 0i32;
    let mut depth_brace = 0i32;
    let bytes = input.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        match ch {
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '{' => depth_brace += 1,
            '}' if depth_brace > 0 => depth_brace -= 1,
            '=' if depth_paren == 0 && depth_brace == 0 => {
                let prev = bytes.get(index.wrapping_sub(1)).copied().unwrap_or(b' ');
                let next = bytes.get(index + 1).copied().unwrap_or(b' ');
                if prev != b'=' && prev != b'!' && prev != b'<' && prev != b'>' && next != b'=' {
                    return true;
                }
            }
            _ => {}
        }
        index += ch.len_utf8();
    }
    false
}

fn extract_fn_main_body(source: &str) -> Option<String> {
    let lower = source.to_ascii_lowercase();
    let fn_start = lower.find("fn main")?;
    let after = &source[fn_start..];
    let open_paren = after.find('(')?;
    let close_paren = balanced_delimiter_offset(&after[open_paren..], '(', ')')?;
    let after_close = &after[open_paren + close_paren + 1..];
    let open_brace = after_close.find('{')?;
    let rest = &after_close[open_brace + 1..];
    let close_brace = balanced_closing_brace_offset(rest)?;
    Some(rest[..close_brace].trim().to_string())
}

fn balanced_closing_brace_offset(body: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (index, ch) in body.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse and compile `inputs:script_src` in one step.
pub fn compile_script(source: &str) -> Result<SeriesClosure, CompileError> {
    let ctx = ScriptCompileContext::from_script_source(source);
    let normalized = normalize_script_for_compile(source);
    compile(&parse_with_context(&normalized, &ctx)?)
}

/// Parse and compile, returning multi-AOV output when the root call is a channel/oscillator bundle.
pub fn compile_script_multi(source: &str) -> Result<CompiledSeries, CompileError> {
    let ctx = ScriptCompileContext::from_script_source(source);
    let normalized = normalize_script_for_compile(source);
    compile_script_multi_with_context(&normalized, &ctx)
}

/// Parse and compile a normalized expression using OSL signature bindings.
pub fn compile_script_multi_with_context(
    normalized: &str,
    ctx: &ScriptCompileContext,
) -> Result<CompiledSeries, CompileError> {
    let expr = parse_with_context(normalized, ctx)?;
    if let Some((name, args, channels)) = detect_multi_root(&expr) {
        let multi = compile_multi_call(name, args, channels)?;
        return Ok(CompiledSeries::Multi(
            multi,
            channels.iter().map(|c| format!("outputs:{c}")).collect(),
        ));
    }
    Ok(CompiledSeries::Single(compile(&expr)?))
}

fn detect_multi_root(expr: &Expr) -> Option<(&str, &[Expr], &'static [&'static str])> {
    let Expr::Call { name, args } = expr else {
        return None;
    };
    let key = name.as_str();
    match key {
        "ta::bollinger_bands" | "bollinger_bands" => {
            Some((key, args.as_slice(), &["upper_band", "basis_line", "lower_band"]))
        }
        "ta::keltner_channels" | "keltner_channels" => {
            Some((key, args.as_slice(), &["upper_band", "basis_line", "lower_band"]))
        }
        "ta::donchian_channels" | "donchian_channels" => {
            Some((key, args.as_slice(), &["upper_band", "basis_line", "lower_band"]))
        }
        "ta::macd" | "macd" if args.len() >= 3 => {
            Some((key, args.as_slice(), &["oscillator", "signal_line"]))
        }
        "ta::stochastic" | "stochastic" if args.len() >= 3 => {
            Some((key, args.as_slice(), &["oscillator", "signal_line"]))
        }
        _ => None,
    }
}

fn compile_multi_call(
    name: &str,
    args: &[Expr],
    _channels: &[&str],
) -> Result<MultiSeriesClosure, CompileError> {
    let key = name.strip_prefix("ta::").unwrap_or(name);
    match key {
        "bollinger_bands" => {
            if args.len() != 3 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 3,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            let mult = literal_f64(&args[2], "multiplier")?;
            Ok(Box::new(move |input| {
                let data = series(Arc::new(input.to_vec()));
                let (upper, basis, lower) = bollinger_bands(&data, period, mult);
                vec![upper, basis, lower]
            }))
        }
        "keltner_channels" => {
            if args.len() != 3 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 3,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            let mult = literal_f64(&args[2], "multiplier")?;
            Ok(Box::new(move |input| {
                let data = series(Arc::new(input.to_vec()));
                let (upper, basis, lower) = keltner_channels(&data, period, mult);
                vec![upper, basis, lower]
            }))
        }
        "donchian_channels" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Box::new(move |input| {
                let data = series(Arc::new(input.to_vec()));
                let (upper, basis, lower) = donchian_channels(&data, period);
                vec![upper, basis, lower]
            }))
        }
        "macd" => {
            if args.len() != 3 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 3,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let short = literal_usize(&args[1], "short period")?;
            let signal = literal_usize(&args[2], "signal period")?;
            Ok(Box::new(move |input| {
                let data = series(Arc::new(input.to_vec()));
                let (macd_line, _) = macd(&data, short, short.saturating_add(8));
                let signal_line = ema(&macd_line, signal.max(1));
                vec![macd_line, signal_line]
            }))
        }
        "stochastic" => {
            if args.len() != 3 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 3,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            let signal = literal_usize(&args[2], "signal period")?;
            Ok(Box::new(move |input| {
                let data = series(Arc::new(input.to_vec()));
                let (osc, sig) = stochastic(&data, period, signal);
                vec![osc, sig]
            }))
        }
        other => Err(CompileError::UnknownFunction(other.to_string())),
    }
}

fn literal_f64(expr: &Expr, label: &'static str) -> Result<f64, CompileError> {
    match expr {
        Expr::Literal(value) if value.is_finite() => Ok(*value),
        _ => Err(CompileError::ExpectedLiteral { label }),
    }
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
        "ema" | "ta::ema" | "ta_ema" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| ema(&series(input.clone()), period)))
        }
        "rsi" | "ta::rsi" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| rsi(&series(input.clone()), period)))
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
        "wma" | "ta::wma" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| wma(&series(input.clone()), period)))
        }
        "stddev" | "ta::stddev" | "ta_stddev" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| rolling_stddev(&series(input.clone()), period)))
        }
        "variance" | "ta::variance" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| {
                let sd = rolling_stddev(&series(input.clone()), period);
                sd.iter().map(|v| v * v).collect()
            }))
        }
        "atr" | "ta::atr" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| atr(&series(input.clone()), period)))
        }
        "ta::historical_volatility" | "historical_volatility" => {
            if args.len() != 3 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 3,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            let annual = literal_f64(&args[2], "annualization")?;
            Ok(Arc::new(move |input| {
                historical_volatility(&series(input.clone()), period, annual)
            }))
        }
        "tema" | "ta::tema" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| tema(&series(input.clone()), period)))
        }
        "hma" | "ta::hma" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let period = literal_usize(&args[1], "period")?;
            Ok(Arc::new(move |input| hma(&series(input.clone()), period)))
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
        "clamp" => {
            if args.len() != 3 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 3,
                    got: args.len(),
                });
            }
            let series = compile_expr(&args[0])?;
            let min = compile_expr(&args[1])?;
            let max = compile_expr(&args[2])?;
            Ok(Arc::new(move |input| {
                let values = series(input.clone());
                let mins = min(input.clone());
                let maxs = max(input);
                values
                    .iter()
                    .zip(mins.iter())
                    .zip(maxs.iter())
                    .map(|((value, min), max)| value.clamp(*min, *max))
                    .collect()
            }))
        }
        "mix" => {
            if args.len() != 3 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 3,
                    got: args.len(),
                });
            }
            let a = compile_expr(&args[0])?;
            let b = compile_expr(&args[1])?;
            let t = compile_expr(&args[2])?;
            Ok(Arc::new(move |input| {
                let a_vals = a(input.clone());
                let b_vals = b(input.clone());
                let t_vals = t(input);
                a_vals
                    .iter()
                    .zip(b_vals.iter())
                    .zip(t_vals.iter())
                    .map(|((a, b), t)| a * (1.0 - t) + b * t)
                    .collect()
            }))
        }
        "step" => {
            if args.len() != 2 {
                return Err(CompileError::InvalidArgumentCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let edge = compile_expr(&args[0])?;
            let value = compile_expr(&args[1])?;
            Ok(Arc::new(move |input| {
                let edges = edge(input.clone());
                let values = value(input);
                edges
                    .iter()
                    .zip(values.iter())
                    .map(|(edge, value)| if *edge >= 0.0 { *value } else { 0.0 })
                    .collect()
            }))
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
///
/// Causal invariant: `out[i]` depends only on `data[0..=i]` (window `index + 1 - period..=index`).
/// Safe for full-series vector evaluation; no look-ahead beyond bar `i`.
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

/// Relative strength index over `data` with lookback `period`.
pub fn rsi(data: &[f64], period: usize) -> Vec<f64> {
    if period == 0 || data.len() < 2 {
        return vec![f64::NAN; data.len()];
    }
    let mut out = vec![f64::NAN; data.len()];
    if data.len() <= period {
        return out;
    }
    let mut gains = 0.0;
    let mut losses = 0.0;
    for index in 1..=period {
        let delta = data[index] - data[index - 1];
        if delta >= 0.0 {
            gains += delta;
        } else {
            losses -= delta;
        }
    }
    let mut avg_gain = gains / period as f64;
    let mut avg_loss = losses / period as f64;
    out[period] = rs_to_rsi(avg_gain, avg_loss);
    for index in (period + 1)..data.len() {
        let delta = data[index] - data[index - 1];
        let gain = delta.max(0.0);
        let loss = (-delta).max(0.0);
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
        out[index] = rs_to_rsi(avg_gain, avg_loss);
    }
    out
}

fn rs_to_rsi(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss <= f64::EPSILON {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

pub fn rolling_stddev(data: &[f64], period: usize) -> Vec<f64> {
    if period == 0 || data.is_empty() {
        return Vec::new();
    }
    let mut out = vec![f64::NAN; data.len()];
    for index in (period - 1)..data.len() {
        let window = &data[index + 1 - period..=index];
        let mean = window.iter().sum::<f64>() / period as f64;
        let variance = window.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / period as f64;
        out[index] = variance.sqrt();
    }
    out
}

pub fn bollinger_bands(data: &[f64], period: usize, mult: f64) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let basis = sma(data, period);
    let spread = rolling_stddev(data, period);
    let upper = basis
        .iter()
        .zip(&spread)
        .map(|(m, s)| m + mult * s)
        .collect();
    let lower = basis
        .iter()
        .zip(&spread)
        .map(|(m, s)| m - mult * s)
        .collect();
    (upper, basis, lower)
}

pub fn keltner_channels(data: &[f64], period: usize, mult: f64) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let basis = ema(data, period.max(1));
    let band = atr(data, period);
    let upper = basis.iter().zip(&band).map(|(m, a)| m + mult * a).collect();
    let lower = basis.iter().zip(&band).map(|(m, a)| m - mult * a).collect();
    (upper, basis, lower)
}

pub fn donchian_channels(data: &[f64], period: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut upper = vec![f64::NAN; data.len()];
    let mut lower = vec![f64::NAN; data.len()];
    if period == 0 {
        return (upper, data.to_vec(), lower);
    }
    for index in (period - 1)..data.len() {
        let window = &data[index + 1 - period..=index];
        let max = window.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let min = window.iter().copied().fold(f64::INFINITY, f64::min);
        upper[index] = max;
        lower[index] = min;
    }
    let basis = upper
        .iter()
        .zip(&lower)
        .map(|(high, low)| (high + low) * 0.5)
        .collect();
    (upper, basis, lower)
}

pub fn wma(data: &[f64], period: usize) -> Vec<f64> {
    if period == 0 || data.is_empty() {
        return Vec::new();
    }
    let mut out = vec![f64::NAN; data.len()];
    let denom = (period * (period + 1) / 2) as f64;
    for index in (period - 1)..data.len() {
        let window = &data[index + 1 - period..=index];
        let weighted: f64 = window
            .iter()
            .enumerate()
            .map(|(offset, value)| value * (offset + 1) as f64)
            .sum();
        out[index] = weighted / denom;
    }
    out
}

pub fn tema(data: &[f64], period: usize) -> Vec<f64> {
    let e1 = ema(data, period.max(1));
    let e2 = ema(&e1, period.max(1));
    let e3 = ema(&e2, period.max(1));
    e1.iter()
        .zip(&e2)
        .zip(&e3)
        .map(|((a, b), c)| 3.0 * a - 3.0 * b + c)
        .collect()
}

pub fn hma(data: &[f64], period: usize) -> Vec<f64> {
    let half = period.max(1) / 2;
    let sqrt = (period as f64).sqrt() as usize;
    let wma_half = wma(data, half.max(1));
    let wma_full = wma(data, period.max(1));
    let diff: Vec<f64> = wma_half
        .iter()
        .zip(&wma_full)
        .map(|(a, b)| 2.0 * a - b)
        .collect();
    wma(&diff, sqrt.max(1))
}

pub fn atr(data: &[f64], period: usize) -> Vec<f64> {
    if data.len() < 2 {
        return vec![f64::NAN; data.len()];
    }
    let mut tr = vec![0.0; data.len()];
    tr[0] = data[0].abs();
    for index in 1..data.len() {
        tr[index] = (data[index] - data[index - 1]).abs();
    }
    sma(&tr, period.max(1))
}

pub fn historical_volatility(data: &[f64], period: usize, annualization: f64) -> Vec<f64> {
    let scale = annualization.max(1.0).sqrt();
    rolling_stddev(data, period)
        .iter()
        .map(|v| v * scale)
        .collect()
}

pub fn stochastic(data: &[f64], period: usize, signal: usize) -> (Vec<f64>, Vec<f64>) {
    let mut osc = vec![f64::NAN; data.len()];
    if period == 0 {
        return (osc.clone(), osc);
    }
    for index in (period - 1)..data.len() {
        let window = &data[index + 1 - period..=index];
        let high = window.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let low = window.iter().copied().fold(f64::INFINITY, f64::min);
        let close = data[index];
        if (high - low).abs() > f64::EPSILON {
            osc[index] = 100.0 * (close - low) / (high - low);
        }
    }
    let signal_line = sma(&osc, signal.max(1));
    (osc, signal_line)
}

pub fn ema(data: &[f64], period: usize) -> Vec<f64> {
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
    fn sma_is_causal_under_future_price_perturbation() {
        let data: Vec<f64> = (0..20).map(|bar| (bar as f64) + 1.0).collect();
        let baseline = sma(&data, 5);
        for bar in 4..data.len() {
            let mut poisoned = data.clone();
            for value in poisoned.iter_mut().skip(bar + 1) {
                *value = 999.0;
            }
            let perturbed = sma(&poisoned, 5);
            assert_eq!(
                baseline[bar], perturbed[bar],
                "sma at bar {bar} must not read future samples"
            );
        }
    }

    #[test]
    fn compiled_sma_closure_is_causal_under_future_price_perturbation() {
        let data: Vec<f64> = (0..20).map(|bar| (bar as f64) + 1.0).collect();
        let closure = compile_script("sma(data, 5)").expect("compile sma");
        let baseline = closure(&data);
        for bar in 4..data.len() {
            let mut poisoned = data.clone();
            for value in poisoned.iter_mut().skip(bar + 1) {
                *value = 999.0;
            }
            let perturbed = closure(&poisoned);
            assert_eq!(
                baseline[bar], perturbed[bar],
                "compiled sma at bar {bar} must not read future samples"
            );
        }
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

    #[test]
    fn parse_script_signature_reads_osl_typed_parameters() {
        let script = r#"
            float source,
            int lookback,
            int threshold,
            output float signal
        {
            lookback = 14;
            signal = sma(source, 3);
        }"#;
        let signature = parse_script_signature(script);
        assert_eq!(
            signature.inputs,
            vec![
                "source".to_string(),
                "lookback".to_string(),
                "threshold".to_string(),
            ]
        );
        assert_eq!(signature.outputs, vec!["signal".to_string()]);
    }

    #[test]
    fn parse_script_signature_reads_osl_output_ports() {
        let script = r#"
            float source,
            int period,
            output float upper_band,
            output float lower_band
        {
            upper_band = sma(source, period);
            lower_band = source - upper_band;
        }"#;
        let signature = parse_script_signature(script);
        assert_eq!(signature.inputs, vec!["source".to_string(), "period".to_string()]);
        assert_eq!(
            signature.outputs,
            vec!["upper_band".to_string(), "lower_band".to_string()]
        );
    }

    #[test]
    fn parse_script_signature_ignores_osl_body_typed_locals() {
        let script = r#"
            float source,
            output float signal
        {
            float standard_dev = ta::stddev(source, 14);
            signal = sma(source, 3);
        }"#;
        let signature = parse_script_signature(script);
        assert_eq!(signature.inputs, vec!["source".to_string()]);
        assert_eq!(signature.outputs, vec!["signal".to_string()]);
    }

    #[test]
    fn normalize_script_for_compile_extracts_osl_output_assignment() {
        let script = r#"
            float source,
            int lookback,
            output float signal
        {
            float standard_dev = ta::stddev(source, 14);
            lookback = 14;
            signal = sma(source, 3);
        }"#;
        let normalized = normalize_script_for_compile(script);
        assert_eq!(normalized, "sma(source, 3)");
    }

    #[test]
    fn compile_script_compiles_osl_shader_with_scalar_parameters() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0];
        let script = r#"
            float source,
            int lookback,
            output float signal
        {
            lookback = 14;
            signal = sma(source, lookback);
        }"#;
        let closure = compile_script(script).expect("compile");
        let out = closure(&data);
        assert_eq!(out.len(), data.len());
        assert!(out[13].is_finite(), "SMA with signature scalar must compile");
    }

    #[test]
    fn compile_script_compiles_void_main_with_signature_scalars() {
        let script = r#"
void main(
    float source_stream,
    int trend_period,
    output float baseline
) {
    baseline = ta_ema(source_stream, trend_period);
}"#;
        let normalized = normalize_script_for_compile(script);
        assert_eq!(normalized, "ta_ema(source_stream, trend_period)");
        let ctx = ScriptCompileContext::from_script_source(script);
        let expr = parse_with_context(&normalized, &ctx).expect("parse");
        let Expr::Call { args, .. } = expr else {
            panic!("expected ta_ema call");
        };
        assert_eq!(args[1], Expr::Literal(14.0));
        let closure = compile_script(script).expect("compile");
        let data = (0..32).map(|v| v as f64 + 100.0).collect::<Vec<_>>();
        let out = closure(&data);
        assert!(out.iter().any(|value| value.is_finite()));
    }

    #[test]
    fn compile_script_compiles_osl_shader_with_typed_locals() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let script = r#"
            float source,
            output float signal
        {
            float standard_dev = ta::stddev(source, 14);
            signal = sma(source, 3);
        }"#;
        let closure = compile_script(script).expect("compile");
        let out = closure(&data);
        assert_eq!(out[2], 2.0);
    }

    #[test]
    fn parse_script_signature_reads_fn_main_parameters() {
        let signature = parse_script_signature(
            "fn main(source, lookback, threshold) {\n  return sma(source, lookback);\n}",
        );
        assert_eq!(
            signature.inputs,
            vec![
                "source".to_string(),
                "lookback".to_string(),
                "threshold".to_string()
            ]
        );
    }

    #[test]
    fn normalize_script_for_compile_strips_fn_main_wrapper() {
        let normalized = normalize_script_for_compile(
            "fn main(data) {\n  return sma(data, 3)\n}",
        );
        assert_eq!(normalized, "sma(data, 3)");
    }

    #[test]
    fn compile_script_compiles_fn_main_body() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let closure = compile_script("fn main(data) { return sma(data, 3); }").expect("compile");
        let out = closure(&data);
        assert_eq!(out.len(), data.len());
        assert_eq!(out[2], 2.0);
    }

    #[test]
    fn parse_script_signature_reads_default_parameter_values() {
        let signature = parse_script_signature(
            "fn main(source, lookback = 14, threshold = 0.5) { return sma(source, lookback); }",
        );
        assert_eq!(
            signature.inputs,
            vec![
                "source".to_string(),
                "lookback".to_string(),
                "threshold".to_string()
            ]
        );
    }

    #[test]
    fn parse_script_signature_ignores_body_assignments() {
        let signature = parse_script_signature(
            "fn main(source, lookback, threshold) {\n  lookback = 14;\n  threshold = 0.5;\n  return sma(source, lookback);\n}",
        );
        assert_eq!(signature.inputs.len(), 3);
        assert!(signature.outputs.is_empty());
    }

    #[test]
    fn normalize_script_for_compile_skips_local_assignments() {
        let normalized = normalize_script_for_compile(
            "fn main(source, lookback) {\n  lookback = 14;\n  return sma(source, 3);\n}",
        );
        assert_eq!(normalized, "sma(source, 3)");
    }

    #[test]
    fn parse_script_signature_reads_void_main_osl_parameters() {
        let script = r#"
// OTL Test Script: Asymmetric Adaptive Channel Trigger
void main(
    float source_stream,
    int trend_period,
    float vol_multiplier,
    float asymmetry_skew,
    output float upper_band,
    output float baseline,
    output float lower_band
) {
    baseline = ta_ema(source_stream, trend_period);
    upper_band = baseline + vol_multiplier;
}"#;
        let signature = parse_script_signature(script);
        assert_eq!(
            signature.inputs,
            vec![
                "source_stream".to_string(),
                "trend_period".to_string(),
                "vol_multiplier".to_string(),
                "asymmetry_skew".to_string(),
            ]
        );
        assert_eq!(
            signature.outputs,
            vec![
                "upper_band".to_string(),
                "baseline".to_string(),
                "lower_band".to_string(),
            ]
        );
    }

    #[test]
    fn normalize_void_main_osl_shader_strips_braces_and_comments() {
        let script = r#"
// header comment
void main(
    float source_stream,
    int trend_period,
    output float baseline
) {
    // body comment
    baseline = ta_ema(source_stream, 14);
}"#;
        let normalized = normalize_script_for_compile(script);
        assert!(!normalized.contains('{'));
        assert!(!normalized.contains('}'));
        assert_eq!(normalized, "ta_ema(source_stream, 14)");
    }

    #[test]
    fn parse_script_entry_point_name_reads_void_shader_identifier() {
        let script = r#"
void adaptive_trigger(
    float source_stream,
    output float baseline
) {
    baseline = ta_ema(source_stream, 14);
}"#;
        assert_eq!(
            parse_script_entry_point_name(script).as_deref(),
            Some("adaptive_trigger")
        );
    }

    #[test]
    fn set_script_uniform_default_updates_signature_binding() {
        let script = r#"
void adaptive_trigger(
    float source_stream,
    int trend_period,
    output float baseline
) {
    baseline = ta_ema(source_stream, trend_period);
}"#;
        let updated = set_script_uniform_default(script, "trend_period", 21.0);
        assert!(updated.contains("int trend_period = 21"));
    }

    #[test]
    fn compile_script_compiles_void_named_osl_entry_point() {
        let script = r#"
void adaptive_trigger(
    float source_stream,
    int trend_period,
    float vol_multiplier,
    float asymmetry_skew,
    output float upper_band,
    output float baseline,
    output float lower_band
) {
    baseline = ta_ema(source_stream, trend_period);
    float standard_dev = ta_stddev(source_stream, trend_period);
    upper_band = baseline + (standard_dev * vol_multiplier * (1.0 + asymmetry_skew));
    lower_band = baseline - (standard_dev * vol_multiplier * (1.0 - asymmetry_skew));
}"#;
        let signature = parse_script_signature(script);
        let normalized = normalize_script_for_compile(script);
        assert!(
            !signature.outputs.is_empty(),
            "expected output ports, got signature: {signature:?}"
        );
        assert!(
            !normalized.is_empty(),
            "named OSL entry points must yield a compilable expression, got empty; signature: {signature:?}"
        );
        assert_eq!(normalized, "ta_ema(source_stream, trend_period)");
        assert_eq!(signature.inputs.first().map(String::as_str), Some("source_stream"));
        assert_eq!(signature.outputs.len(), 3);
        let _closure = compile_script(script).expect("named OSL entry point must compile");
    }

    #[test]
    fn compile_script_compiles_fn_main_with_local_assignments() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let closure = compile_script(
            "fn main(source, lookback) { lookback = 3; return sma(source, 3); }",
        )
        .expect("compile");
        let out = closure(&data);
        assert_eq!(out[2], 2.0);
    }
}

#[cfg(test)]
#[path = "compiler/tests.rs"]
mod integration_tests;
