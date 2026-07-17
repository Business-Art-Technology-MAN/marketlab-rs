//! Multi-series evaluation context for vector OTL (GA + dual-input shaders).

use std::collections::HashMap;

/// Runtime inputs for a compiled vector OTL closure.
#[derive(Clone, Debug, Default)]
pub struct SeriesEvalContext<'a> {
    pub primary: &'a [f64],
    pub named: HashMap<&'a str, &'a [f64]>,
    pub constituents: Vec<&'a [f64]>,
}

impl<'a> SeriesEvalContext<'a> {
    pub fn primary_only(primary: &'a [f64]) -> Self {
        Self {
            primary,
            named: HashMap::new(),
            constituents: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.primary.len()
    }

    pub fn series_named(&self, name: &str) -> &'a [f64] {
        self.named.get(name).copied().unwrap_or(self.primary)
    }
}

/// Owned buffer for closure evaluation (stores copied series).
#[derive(Clone, Debug, Default)]
pub struct SeriesEvalBuffer {
    pub primary: Vec<f64>,
    pub named: HashMap<String, Vec<f64>>,
    pub constituents: Vec<Vec<f64>>,
}

impl SeriesEvalBuffer {
    pub fn from_slices(
        primary: &[f64],
        named: &HashMap<String, Vec<f64>>,
        constituents: &[Vec<f64>],
    ) -> Self {
        Self {
            primary: primary.to_vec(),
            named: named.clone(),
            constituents: constituents.to_vec(),
        }
    }

    pub fn context(&self) -> SeriesEvalContext<'_> {
        let named_refs: HashMap<&str, &[f64]> = self
            .named
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        let constituent_refs: Vec<&[f64]> = self.constituents.iter().map(|v| v.as_slice()).collect();
        SeriesEvalContext {
            primary: &self.primary,
            named: named_refs,
            constituents: constituent_refs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_series_fallback_to_primary() {
        let primary = vec![1.0, 2.0, 3.0];
        let ctx = SeriesEvalContext::primary_only(&primary);
        assert_eq!(ctx.series_named("market"), primary.as_slice());
    }
}
