//! OTL three-tier object AST (`signal`, `allocator`, `portfolio`).

/// Compile-time intent for a declared OTL object block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OtlObjectKind {
    Signal,
    Allocator,
    Portfolio,
    /// OSL / legacy `shader` and `fn main` scripts (back-compat).
    LegacyShader,
}

impl OtlObjectKind {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Signal => "signal",
            Self::Allocator => "allocator",
            Self::Portfolio => "portfolio",
            Self::LegacyShader => "shader",
        }
    }
}

/// Typed port or property in an object signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtlType {
    Float,
    Int,
    String,
    Closure,
    ClosureArray,
}

impl OtlType {
    pub fn is_closure_array(self) -> bool {
        matches!(self, Self::ClosureArray)
    }

    pub fn is_closure(self) -> bool {
        matches!(self, Self::Closure | Self::ClosureArray)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyDeclaration {
    pub direction: PortDirection,
    pub ty: OtlType,
    pub name: String,
    /// Scalar default for `input int` / `input float` ports (`fast = 10`).
    pub default_value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Assign { target: String, expr: String },
    Return { expr: String },
    Raw { text: String },
}

/// One `signal` / `allocator` / `portfolio` / legacy shader declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct OtlObjectDeclaration {
    pub kind: OtlObjectKind,
    pub name: String,
    pub inputs: Vec<PropertyDeclaration>,
    pub outputs: Vec<PropertyDeclaration>,
    pub body: Vec<Statement>,
}

/// Parsed OTL source file containing one or more object declarations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OtlProgram {
    pub objects: Vec<OtlObjectDeclaration>,
}

impl OtlProgram {
    pub fn object(&self, name: &str) -> Option<&OtlObjectDeclaration> {
        self.objects
            .iter()
            .find(|object| object.name.eq_ignore_ascii_case(name))
    }

    pub fn primary_object(&self) -> Option<&OtlObjectDeclaration> {
        self.objects.first()
    }
}
