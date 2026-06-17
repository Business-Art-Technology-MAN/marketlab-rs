//! OTL Phase 2 object-tier codegen (AST → execution engines).

mod codegen;

pub use codegen::{
    compile_object_program, AllocatorExecutionEngine, CompiledProgramTier, ObjectCodegenRegistry,
    PortfolioExecutionEngine, SignalExecutionEngine,
};
