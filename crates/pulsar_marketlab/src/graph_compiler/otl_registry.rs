//! Registered OTL object declarations keyed by canvas/script name.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use pulsar_marketlab_core::{
    compile_object_program, OtlObjectDeclaration, OtlObjectKind, OtlProgram, FrontendError,
};

#[derive(Debug, Clone, Default)]
pub struct OtlObjectRegistry {
    objects: HashMap<String, Arc<OtlObjectDeclaration>>,
}

impl OtlObjectRegistry {
    pub fn register(
        &mut self,
        name: &str,
        kind: OtlObjectKind,
        declaration: OtlObjectDeclaration,
    ) -> Result<(), RegistryError> {
        if declaration.kind != kind {
            return Err(RegistryError::KindMismatch {
                name: name.to_string(),
                expected: kind,
                actual: declaration.kind,
            });
        }
        self.objects
            .insert(name.to_string(), Arc::new(declaration));
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<OtlObjectDeclaration>> {
        self.objects.get(name).cloned()
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("OTL object `{name}` kind mismatch: expected {expected:?}, got {actual:?}")]
    KindMismatch {
        name: String,
        expected: OtlObjectKind,
        actual: OtlObjectKind,
    },
    #[error("OTL object `{name}` not found in parsed program")]
    MissingDeclaration { name: String },
    #[error(transparent)]
    Frontend(#[from] FrontendError),
}

static GLOBAL_REGISTRY: LazyLock<RwLock<OtlObjectRegistry>> =
    LazyLock::new(|| RwLock::new(OtlObjectRegistry::default()));

fn registry_write() -> std::sync::RwLockWriteGuard<'static, OtlObjectRegistry> {
    GLOBAL_REGISTRY
        .write()
        .expect("OTL object registry lock poisoned")
}

/// Register a parsed OTL object declaration in the workspace registry.
pub fn register_otl_object(
    name: &str,
    kind: OtlObjectKind,
    declaration: OtlObjectDeclaration,
) -> Result<(), RegistryError> {
    registry_write().register(name, kind, declaration)
}

/// Parse OTL source, validate semantics, and register the first declared object.
pub fn register_otl_object_from_source(
    name: &str,
    kind: OtlObjectKind,
    source: &str,
) -> Result<OtlProgram, RegistryError> {
    let program = compile_object_program(source)?;
    let declaration = program
        .objects
        .first()
        .cloned()
        .ok_or_else(|| RegistryError::MissingDeclaration {
            name: name.to_string(),
        })?;
    register_otl_object(name, kind, declaration)?;
    Ok(program)
}

pub fn otl_object_registry_snapshot() -> OtlObjectRegistry {
    GLOBAL_REGISTRY
        .read()
        .map(|registry| registry.clone())
        .unwrap_or_default()
}

/// Map canvas node taxonomy to OTL object intent.
pub fn otl_object_kind_for_node(node: &super::VisualNode) -> Option<OtlObjectKind> {
    match &node.node_type {
        super::NodeType::OtlShader { .. } => Some(OtlObjectKind::LegacyShader),
        super::NodeType::TaUberSignal { .. } => Some(OtlObjectKind::Signal),
        super::NodeType::TerminalIntegrator { engine_target } if engine_target == "portfolio" => {
            Some(OtlObjectKind::Portfolio)
        }
        super::NodeType::TerminalIntegrator { .. } => Some(OtlObjectKind::Allocator),
        super::NodeType::AssetAdaptor { .. } => None,
    }
}
