//! Semantic validation pass for OTL three-tier object declarations.

use thiserror::Error;

use super::ast::{OtlObjectDeclaration, OtlObjectKind, OtlProgram, PortDirection, Statement};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("signal `{name}` cannot accept closure array input `{port}`")]
    SignalClosureArrayInput { name: String, port: String },
    #[error("signal `{name}` must declare at least one closure output")]
    SignalMissingClosureOutput { name: String },
    #[error("allocator `{name}` must declare at least one closure input")]
    AllocatorMissingClosureInput { name: String },
    #[error("allocator `{name}` must declare a closure output")]
    AllocatorMissingClosureOutput { name: String },
    #[error("allocator `{name}` cannot reference portfolio state `{call}`")]
    AllocatorPortfolioStateAccess { name: String, call: String },
    #[error("portfolio `{name}` must declare at least one closure input")]
    PortfolioMissingClosureInput { name: String },
}

pub fn validate_program(program: &OtlProgram) -> Result<(), ValidationError> {
    for object in &program.objects {
        validate_object(object)?;
    }
    Ok(())
}

pub fn validate_object(object: &OtlObjectDeclaration) -> Result<(), ValidationError> {
    match object.kind {
        OtlObjectKind::Signal => validate_signal(object),
        OtlObjectKind::Allocator => validate_allocator(object),
        OtlObjectKind::Portfolio => validate_portfolio(object),
        OtlObjectKind::LegacyShader => Ok(()),
    }
}

fn validate_signal(object: &OtlObjectDeclaration) -> Result<(), ValidationError> {
    for input in object.inputs.iter().filter(|port| port.direction == PortDirection::Input) {
        if input.ty.is_closure_array() {
            return Err(ValidationError::SignalClosureArrayInput {
                name: object.name.clone(),
                port: input.name.clone(),
            });
        }
    }
    let has_closure_output = object.outputs.iter().any(|port| port.ty.is_closure());
    if !has_closure_output {
        return Err(ValidationError::SignalMissingClosureOutput {
            name: object.name.clone(),
        });
    }
    Ok(())
}

fn validate_allocator(object: &OtlObjectDeclaration) -> Result<(), ValidationError> {
    let has_closure_input = object
        .inputs
        .iter()
        .any(|port| port.direction == PortDirection::Input && port.ty.is_closure());
    if !has_closure_input {
        return Err(ValidationError::AllocatorMissingClosureInput {
            name: object.name.clone(),
        });
    }
    let has_closure_output = object.outputs.iter().any(|port| port.ty.is_closure());
    if !has_closure_output {
        return Err(ValidationError::AllocatorMissingClosureOutput {
            name: object.name.clone(),
        });
    }
    for statement in &object.body {
        if statement_references_portfolio_info(statement) {
            return Err(ValidationError::AllocatorPortfolioStateAccess {
                name: object.name.clone(),
                call: "portfolio_info".to_string(),
            });
        }
    }
    Ok(())
}

fn validate_portfolio(object: &OtlObjectDeclaration) -> Result<(), ValidationError> {
    let has_closure_input = object
        .inputs
        .iter()
        .any(|port| port.direction == PortDirection::Input && port.ty.is_closure());
    if !has_closure_input && object.kind != OtlObjectKind::LegacyShader {
        return Err(ValidationError::PortfolioMissingClosureInput {
            name: object.name.clone(),
        });
    }
    Ok(())
}

fn statement_references_portfolio_info(statement: &Statement) -> bool {
    let text = match statement {
        Statement::Assign { expr, .. } | Statement::Return { expr } | Statement::Raw { text: expr } => {
            expr
        }
    };
    text.to_ascii_lowercase().contains("portfolio_info")
}
