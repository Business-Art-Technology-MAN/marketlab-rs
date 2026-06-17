//! Layer opinion tray state for the unified topology dopesheet.

use std::collections::HashMap;

use pulsar_marketlab_core::{SESSION_LAYER_FILENAME, WORKSTATION_LAYER_STACK};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LayerDisplayState {
    Active,
    Muted,
    Isolated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerDescriptor {
    pub filename: String,
    pub display_name: String,
    /// Lower index = stronger opinion in the LIVRPS stack (session strongest).
    pub priority_order: usize,
    pub state: LayerDisplayState,
    pub is_user_editable: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LayerStackControlState {
    layer_states: HashMap<String, LayerDisplayState>,
    layer_order: Vec<String>,
    isolated_layer: Option<String>,
}

impl LayerStackControlState {
    pub fn new() -> Self {
        Self {
            layer_states: HashMap::new(),
            layer_order: WORKSTATION_LAYER_STACK
                .iter()
                .map(|layer| (*layer).to_string())
                .collect(),
            isolated_layer: None,
        }
    }

    pub fn state_for(&self, filename: &str) -> LayerDisplayState {
        if self.isolated_layer.as_deref() == Some(filename) {
            return LayerDisplayState::Isolated;
        }
        self.layer_states
            .get(filename)
            .copied()
            .unwrap_or(LayerDisplayState::Active)
    }

    pub fn toggle_mute(&mut self, filename: &str) {
        let next = match self.state_for(filename) {
            LayerDisplayState::Active => LayerDisplayState::Muted,
            LayerDisplayState::Muted => LayerDisplayState::Active,
            LayerDisplayState::Isolated => LayerDisplayState::Active,
        };
        if next == LayerDisplayState::Active {
            self.layer_states.remove(filename);
            if self.isolated_layer.as_deref() == Some(filename) {
                self.isolated_layer = None;
            }
        } else {
            self.layer_states.insert(filename.to_string(), next);
            if self.isolated_layer.as_deref() == Some(filename) {
                self.isolated_layer = None;
            }
        }
    }

    pub fn isolate_layer(&mut self, filename: &str) {
        self.isolated_layer = Some(filename.to_string());
        self.layer_states.remove(filename);
    }

    pub fn clear_isolation(&mut self) {
        self.isolated_layer = None;
    }

    pub fn reorder(&mut self, from: usize, to: usize) {
        if from >= self.layer_order.len() || to >= self.layer_order.len() || from == to {
            return;
        }
        let item = self.layer_order.remove(from);
        self.layer_order.insert(to, item);
    }

    pub fn ordered_layers(&self) -> &[String] {
        &self.layer_order
    }

    pub fn sync_ordered_layers(&mut self, filenames: Vec<String>) {
        if filenames.is_empty() {
            return;
        }
        self.layer_order = filenames;
    }
}

pub fn layer_display_name(identifier: &str) -> String {
    std::path::Path::new(identifier)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(identifier)
        .to_string()
}

pub fn build_layer_descriptors(state: &LayerStackControlState) -> Vec<LayerDescriptor> {
    state
        .ordered_layers()
        .iter()
        .enumerate()
        .map(|(priority_order, filename)| LayerDescriptor {
            filename: filename.clone(),
            display_name: layer_display_name(filename),
            priority_order,
            state: state.state_for(filename),
            is_user_editable: filename == SESSION_LAYER_FILENAME,
        })
        .collect()
}
