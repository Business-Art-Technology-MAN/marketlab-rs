//! Fail-fast compatibility checks for `.otc` bytecode assets.

use super::{OtcBinaryHeader, OtcError};

/// Runtime engine generation for this MarketLab build (increment on breaking VectorTA / graph changes).
pub const CURRENT_ENGINE_GENERATION: u32 = 2;

/// Validate header fields after the fixed 32-byte prefix has been parsed.
pub fn verify_header_compatibility(
    header: &OtcBinaryHeader,
    local_engine_generation: u32,
) -> Result<(), OtcError> {
    if header.engine_generation > local_engine_generation {
        return Err(OtcError::EngineGenerationMismatch {
            asset_generation: header.engine_generation,
            local_generation: local_engine_generation,
        });
    }
    Ok(())
}
