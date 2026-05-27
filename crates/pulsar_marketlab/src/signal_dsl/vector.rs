//! Core geometric sample primitive for OTL service payloads.

use std::fmt;

/// Polymorphic numeric payload exchanged through [`super::services::MarketProviderServices`].
#[derive(Clone, Debug, PartialEq)]
pub struct Vector {
    components: Vec<f64>,
}

impl Vector {
    pub fn scalar(value: f64) -> Self {
        Self {
            components: vec![value],
        }
    }

    pub fn from_components(components: Vec<f64>) -> Self {
        Self { components }
    }

    pub fn components(&self) -> &[f64] {
        &self.components
    }

    pub fn as_scalar(&self) -> Option<f64> {
        if self.components.len() == 1 {
            Some(self.components[0])
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}

impl fmt::Display for Vector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for (index, value) in self.components.iter().enumerate() {
            if index > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{value:.4}")?;
        }
        write!(f, "]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_vector_round_trips() {
        let vector = Vector::scalar(42.0);
        assert_eq!(vector.as_scalar(), Some(42.0));
        assert_eq!(vector.len(), 1);
    }
}
