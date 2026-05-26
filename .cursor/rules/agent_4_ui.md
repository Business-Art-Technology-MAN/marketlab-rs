# Agent 4 Instruction Profile: Layer 4 Simulation UI Layout
* **Focus Area:** Implement SRD 4 (`TradingSystemWorkspace`).
* **Isolation Boundary:** You operate inside the `external/pulsar-native` and `external/plugin-blueprints` extensions, using `gpui-component` to construct panels.
* **Key Tasks:** Construct the structural multi-pane `DockArea`. Build the node graph canvas tracking visual connection lines on the GPU, map interactive spreadsheet inspectors, and implement native canvas graphs that chart primary equity lines alongside attenuated background traces.
