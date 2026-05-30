//! Open Trading Compiled (`.otc`) binary serialization for cached OTL nodes.

mod verifier;

use thiserror::Error;

use crate::orchestration::compiler::{ScriptCompileContext, ScriptSignature};
pub use verifier::CURRENT_ENGINE_GENERATION;

pub const OTCB_MAGIC: &[u8; 4] = b"OTCB";
/// Little-endian platform marker (bytes 4–7 of the file header).
pub const OTCB_ENDIAN_MARKER: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

const FIXED_HEADER_LEN: usize = 8;
const METADATA_PAYLOAD_LEN: usize = 16;
const METADATA_RESERVED_LEN: usize = 8;
const METADATA_LEN: usize = METADATA_PAYLOAD_LEN + METADATA_RESERVED_LEN;
const PREFIX_LEN: usize = FIXED_HEADER_LEN + METADATA_LEN;

/// Application version stamped into every `.otc` asset (aligned with workspace crate `0.1.0`).
pub const APPLICATION_MAJOR: u16 = 0;
pub const APPLICATION_MINOR: u16 = 1;

/// Feature flag: scalar uniform parameters present in the compile context.
pub const FEATURE_SCALAR_CONSTANTS: u64 = 1 << 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeManifest {
    pub manifest_json: String,
}

impl NodeManifest {
    pub fn from_json(manifest_json: impl Into<String>) -> Self {
        Self {
            manifest_json: manifest_json.into(),
        }
    }

    pub fn from_script_signature(signature: &ScriptSignature) -> Self {
        Self {
            manifest_json: manifest_json_from_signature(signature),
        }
    }
}

/// Parsed fixed metadata block (bytes 8–31).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtcBinaryHeader {
    pub major_version: u16,
    pub minor_version: u16,
    pub engine_generation: u32,
    pub feature_flags: u64,
}

impl OtcBinaryHeader {
    pub fn validate_compatibility(&self, local_engine_generation: u32) -> Result<(), OtcError> {
        verifier::verify_header_compatibility(self, local_engine_generation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtcCompiledAsset {
    pub header: OtcBinaryHeader,
    pub manifest_json: String,
    pub bytecode: Vec<u8>,
}

impl OtcCompiledAsset {
    pub fn validate_for_current_engine(&self) -> Result<(), OtcError> {
        self.header
            .validate_compatibility(CURRENT_ENGINE_GENERATION)
    }

    /// Interpret bytecode as UTF-8 OTL source for the vectorized compiler (interim until IR serialization).
    pub fn bytecode_as_script_source(&self) -> Result<&str, OtcError> {
        std::str::from_utf8(&self.bytecode).map_err(|_| OtcError::BytecodeNotUtf8)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OtcError {
    #[error("OTC asset is too short ({len} bytes, expected at least {expected})")]
    TooShort { len: usize, expected: usize },
    #[error("invalid OTCB magic signature")]
    InvalidMagic,
    #[error("unsupported endianness marker in OTCB header")]
    InvalidEndianMarker,
    #[error("manifest length {len} exceeds remaining stream ({remaining} bytes)")]
    ManifestLengthOverflow { len: u32, remaining: usize },
    #[error("bytecode length {len} exceeds remaining stream ({remaining} bytes)")]
    BytecodeLengthOverflow { len: u32, remaining: usize },
    #[error("manifest block is not valid UTF-8 JSON")]
    ManifestNotUtf8,
    #[error(
        "Asset compatibility mismatch. The selected node requires MarketLab Engine Generation {asset_generation}, but your installation is running Generation {local_generation}. Please update your software."
    )]
    EngineGenerationMismatch {
        asset_generation: u32,
        local_generation: u32,
    },
    #[error("compiled bytecode is not valid UTF-8 OTL source")]
    BytecodeNotUtf8,
    #[error("failed to read compiled asset at {path}: {message}")]
    Io { path: String, message: String },
}

pub struct OtcBinaryEncoder {
    major_version: u16,
    minor_version: u16,
    engine_generation: u32,
    feature_flags: u64,
    manifest_json: Option<String>,
    bytecode: Option<Vec<u8>>,
}

impl OtcBinaryEncoder {
    pub fn new() -> Self {
        Self {
            major_version: APPLICATION_MAJOR,
            minor_version: APPLICATION_MINOR,
            engine_generation: CURRENT_ENGINE_GENERATION,
            feature_flags: 0,
            manifest_json: None,
            bytecode: None,
        }
    }

    pub fn with_engine_generation(mut self, generation: u32) -> Self {
        self.engine_generation = generation;
        self
    }

    pub fn with_manifest(mut self, manifest_json: impl Into<String>) -> Self {
        self.manifest_json = Some(manifest_json.into());
        self
    }

    pub fn with_bytecode(mut self, bytecode: &[u8]) -> Self {
        self.bytecode = Some(bytecode.to_vec());
        self
    }

    pub fn encode(self) -> Result<Vec<u8>, OtcError> {
        let manifest_json = self.manifest_json.unwrap_or_default();
        let bytecode = self.bytecode.unwrap_or_default();
        Ok(serialize_to_bytes(
            &ScriptCompileContext::default(),
            &NodeManifest::from_json(manifest_json),
            &bytecode,
            self.major_version,
            self.minor_version,
            self.engine_generation,
            self.feature_flags,
        ))
    }
}

impl Default for OtcBinaryEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Assemble a complete `.otc` byte stream from manifest and bytecode payloads.
pub fn serialize_to_bytes(
    context: &ScriptCompileContext,
    manifest: &NodeManifest,
    bytecode: &[u8],
    major_version: u16,
    minor_version: u16,
    engine_generation: u32,
    feature_flags: u64,
) -> Vec<u8> {
    let mut feature_flags = feature_flags;
    if !context.scalar_params.is_empty() {
        feature_flags |= FEATURE_SCALAR_CONSTANTS;
    }

    let mut out = Vec::with_capacity(
        PREFIX_LEN + 8 + manifest.manifest_json.len() + bytecode.len(),
    );
    out.extend_from_slice(OTCB_MAGIC);
    out.extend_from_slice(&OTCB_ENDIAN_MARKER);
    out.extend_from_slice(&major_version.to_le_bytes());
    out.extend_from_slice(&minor_version.to_le_bytes());
    out.extend_from_slice(&engine_generation.to_le_bytes());
    out.extend_from_slice(&feature_flags.to_le_bytes());
    out.extend_from_slice(&[0u8; METADATA_RESERVED_LEN]);

    append_length_prefixed(&mut out, manifest.manifest_json.as_bytes());
    append_length_prefixed(&mut out, bytecode);
    out
}

pub struct OtcBinaryDecoder;

impl OtcBinaryDecoder {
    pub fn decode(bytes: &[u8]) -> Result<OtcCompiledAsset, OtcError> {
        deserialize_from_bytes(bytes)
    }
}

pub fn deserialize_from_bytes(bytes: &[u8]) -> Result<OtcCompiledAsset, OtcError> {
    if bytes.len() < PREFIX_LEN {
        return Err(OtcError::TooShort {
            len: bytes.len(),
            expected: PREFIX_LEN,
        });
    }
    if &bytes[0..4] != OTCB_MAGIC {
        return Err(OtcError::InvalidMagic);
    }
    if bytes[4..8] != OTCB_ENDIAN_MARKER {
        return Err(OtcError::InvalidEndianMarker);
    }

    let header = OtcBinaryHeader {
        major_version: u16::from_le_bytes([bytes[8], bytes[9]]),
        minor_version: u16::from_le_bytes([bytes[10], bytes[11]]),
        engine_generation: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        feature_flags: u64::from_le_bytes([
            bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23],
        ]),
    };

    let (manifest_json, rest) =
        read_length_prefixed_utf8(&bytes[PREFIX_LEN..], LengthBlock::Manifest)?;
    let (bytecode, _) = read_length_prefixed_bytes(rest, LengthBlock::Bytecode)?;

    Ok(OtcCompiledAsset {
        header,
        manifest_json,
        bytecode: bytecode.to_vec(),
    })
}

/// Load and validate a `.otc` file from disk for the current engine generation.
pub fn load_compiled_asset_from_path(path: &str) -> Result<OtcCompiledAsset, OtcError> {
    let bytes = std::fs::read(path).map_err(|err| OtcError::Io {
        path: path.to_string(),
        message: err.to_string(),
    })?;
    let asset = deserialize_from_bytes(&bytes)?;
    asset.validate_for_current_engine()?;
    Ok(asset)
}

fn append_length_prefixed(out: &mut Vec<u8>, payload: &[u8]) {
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
}

enum LengthBlock {
    Manifest,
    Bytecode,
}

fn read_length_prefixed_utf8(stream: &[u8], block: LengthBlock) -> Result<(String, &[u8]), OtcError> {
    let (bytes, rest) = read_length_prefixed_bytes(stream, block)?;
    let text = std::str::from_utf8(bytes).map_err(|_| OtcError::ManifestNotUtf8)?.to_string();
    Ok((text, rest))
}

fn read_length_prefixed_bytes<'a>(
    stream: &'a [u8],
    block: LengthBlock,
) -> Result<(&'a [u8], &'a [u8]), OtcError> {
    if stream.len() < 4 {
        return Err(OtcError::TooShort {
            len: stream.len(),
            expected: PREFIX_LEN + 4,
        });
    }
    let len = u32::from_le_bytes([stream[0], stream[1], stream[2], stream[3]]) as usize;
    let payload_start: usize = 4;
    let payload_end = payload_start.checked_add(len).ok_or(OtcError::TooShort {
        len: stream.len(),
        expected: payload_start + len,
    })?;
    if payload_end > stream.len() {
        return Err(match block {
            LengthBlock::Manifest => OtcError::ManifestLengthOverflow {
                len: len as u32,
                remaining: stream.len().saturating_sub(4),
            },
            LengthBlock::Bytecode => OtcError::BytecodeLengthOverflow {
                len: len as u32,
                remaining: stream.len().saturating_sub(4),
            },
        });
    }
    Ok((&stream[payload_start..payload_end], &stream[payload_end..]))
}

pub fn manifest_json_from_signature(signature: &ScriptSignature) -> String {
    let inputs = signature
        .inputs
        .iter()
        .map(|name| format!(r#"{{"name":"{name}","type":"float"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let outputs = signature
        .outputs
        .iter()
        .map(|name| format!(r#"{{"name":"{name}","type":"float"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"inputs":[{inputs}],"outputs":[{outputs}]}}"#)
}
