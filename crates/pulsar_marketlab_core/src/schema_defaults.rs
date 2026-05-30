//! Parsed default attribute values from the canonical `schema.usda` layer.

use std::collections::HashMap;

use crate::FINANCIAL_SCHEMA_USDA;

/// Map `inputs:attr` (and bare attribute names) to their schema default literal text.
pub fn financial_schema_defaults() -> HashMap<String, String> {
    let mut defaults = HashMap::new();
    for line in FINANCIAL_SCHEMA_USDA.lines() {
        let line = line.trim();
        for prefix in ["bool ", "token ", "string ", "double ", "float ", "int "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                if let Some((name, value)) = rest.split_once('=') {
                    let name = name.trim().to_string();
                    let value = value.trim().trim_end_matches('(').trim().to_string();
                    if name.starts_with("inputs:") {
                        defaults.insert(name, value);
                    }
                }
                break;
            }
        }
    }
    defaults
}
