# MarketLab — Agent Technical Brief

**Purpose:** Onboard AI coding agents to MarketLab’s architecture, SRD lineage, OTL DAG model, and DSL realities.  
**Repo:** `MarketLab` (Rust workspace)  
**Last aligned with codebase:** May 2026  

---

## 1. What MarketLab Is

MarketLab is a **pure-Rust desktop trading-system workbench** built on **GPUI 0.2.x**. It treats quantitative pipelines like a **Digital Content Creation (DCC) graph**—similar in spirit to Blender node editors and **usdtweak**-style USD inspectors—not like a Bloomberg terminal.

Core idea: **author a pipeline visually**, **serialize it to OpenUSD** as the structural source of truth, and **execute it** against a high-frequency temporal stage (OHLC bars, playhead, portfolio ledger).

| Metaphor | MarketLab equivalent |
|----------|---------------------|
| Blender node tree | Three-tier canvas DAG (Asset → OTL → Portfolio) |
| USD stage / layer stack | `openusd::Stage` + `WorkspaceContext` |
| Shader network (OSL) | OTL (Open Trading Language) scripts on `OtlOperator` prims |
| Render / composite | Portfolio integrator + execution ledger |
| Look-dev at playhead | Playhead-scoped DSL evaluation + VectorTA fallback |

---

## 2. Technology Stack

| Layer | Technology | Notes |
|-------|------------|-------|
| UI shell | **GPUI 0.2.2** + `gpui-component` | Immediate-mode panes, deferred USD writes on mouse-up |
| UI crate | `pulsar_marketlab_ui` | Workstation layout, `WorkspaceContext`, stage composer, node canvas frame |
| App / orchestration | `pulsar_marketlab` (lib + binary) | Canvas, wiring, playhead, CSV ingestion, stage bridge |
| Core engine | `pulsar_marketlab_core` | USD schema embed, `MarketLabGraphEngine`, vectorized OTL compiler |
| Structural plane | **`openusd` 0.3.0** (`mxpv/openusd`) | LIVRPS composition, layer stack, prim specs—**no C++ USD** |
| Graph algorithms | `petgraph` | Topological sort for USD-derived execution order |
| TA fallback | `vector-ta` | ~340 CPU indicators when no OTL script is set |
| Concurrency | GPUI background executor | Graph recompile, playhead eval, CSV workers—**never block render tick** |

**Important:** OpenUSD in this project is **read/composition oriented**. High-frequency time samples live in **`MarketStage`**, not in USD layers (by design—see Split-Plane below).

---

## 3. SRD Inspiration & Document Map

MarketLab requirements are captured as **SRD (System Requirements Document)** files under `.cursor/rules/`. Agents should treat these as **intent specs**; **`crates/` source is ground truth** when they diverge.

### Layered engine SRDs (bottom → top)

| SRD | Module | Responsibility |
|-----|--------|----------------|
| **SRD 1 — Stage** | `trading_stage/` (`MarketStage`) | Path-addressable prims, `f64` time samples, causal forward-fill |
| **SRD 2 — Engine** | `execution_engine/` | Kahn DAG, mixed-frequency stride grid, simulation ledger |
| **SRD 3 — Kernel** | `signal_kernel/` | VectorTA SIMD surface, GA grade channels |
| **SRD 4 — UI** | `workspace_state.rs`, `ui/` | `TradingSystemWorkspace`, canvas, inspectors, playhead |
| **SRD 5 — USD schema** | `pulsar_marketlab_core/resources/usd/schema.usda` | `FinancialAsset`, `OtlOperator`, `PortfolioIntegrator` |

### Phase / pillar docs (cross-cutting)

| Document | Topic |
|----------|-------|
| `Phase B Pillar 1 OpenUSD-Inspired Time-Series Stage.md` | Temporal plane design |
| `Phase C Pillar 1 OSL-Inspired Evaluation Language Frontend.md` | `signal_dsl/` playhead OTL |
| `three-tiered node registry taxonomy.md` | Canvas node tiers + port validation |
| `Tier-2 OTL Standard Library.md` | Stdlib intrinsics (`mix`, `clamp`, `step`, financial) |
| `genuine, pure-Rust native OpenUSD.md` | Split-plane hybrid engine mandate |
| `USDTweak_Inspiredchanges.md` | DCC chrome, deferred writes, stage composer layout |
| `Build the USD Stage Graph Compilation Core.md` | `MarketLabGraphEngine` |
| `Integrate Graph Engine Invalidation with Workspace Context.md` | Background recompile loop |

### UI / interaction SRDs

| Document | Topic |
|----------|-------|
| `marketlab_srd_1.md` | Canvas drag, ports, selection (early UI SRD) |
| `Implement GPUI-Native Workstation Workspace Layout.md` | Tri-pane workstation shell |
| `UI Interaction & Engine Compilation Alignment.md` | Selection sync, invalidation |

---

## 4. Split-Plane Architecture (Structural vs Temporal)

> **Canonical reference:** [`docs/architecture/openusd_integration.md`](../../docs/architecture/openusd_integration.md) — full OpenUSD boundary guide, dual-bridge risks, and production milestones.

This is the **central architectural constraint**. Do not collapse these planes.

```
┌─────────────────────────────────────────────────────────────────┐
│  STRUCTURAL PLANE (OpenUSD)                                     │
│  UsdStageBridge / ManagedUsdStage / WorkspaceContext            │
│  • Prim hierarchy, relationships, active flags                  │
│  • Schema classes, variant tokens, ui:canvas:pos                │
│  • Layer stack + edit-target (UI overlay; full edit API TBD)    │
│  • Low-frequency metadata; reopened or overlay-mutated          │
└───────────────────────────┬─────────────────────────────────────┘
                            │ ProductionStageProvider
                            │ (active? → sample timeline)
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  TEMPORAL / EXECUTION PLANE (MarketStage)                        │
│  • OHLCV time samples per asset path                            │
│  • Playhead coordinate, bar-index scrubbing                     │
│  • Execution ledger (cash, positions)                           │
│  • High-frequency sweeps; bypasses USD time-sampling gaps       │
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  GRAPH ENGINE PLANE (MarketLabGraphEngine)                      │
│  • Compiled from USD topology (petgraph)                        │
│  • Vectorized OTL over full price series                      │
│  • Emits ComputedAttributeStream → WorkspaceContext             │
└─────────────────────────────────────────────────────────────────┘
```

**`ProductionStageProvider`** (`stage_bridge/production_provider.rs`) implements `MarketProviderServices`:
- USD answers *whether* a prim participates (`active`, structure).
- `MarketStage` answers *what happened over time* (samples, ledger).

---

## 5. Crate Layout & Responsibilities

```
pulsar_marketlab (binary)
├── graph_compiler/     VisualNode, NodeConnection, wiring validation, SharedPipelineGraph
├── canvas_compose.rs   Canvas → inline USDA
├── canvas_hydrate.rs   USDA → canvas restore
├── workspace_state.rs  TradingSystemWorkspace — sync hub
├── signal_dsl/         Playhead OTL (OSL-inspired)
├── execution_engine/   Layer 2 simulation
├── trading_stage/      MarketStage + scene path conventions
├── stage_bridge/       UsdStageBridge, ProductionStageProvider
├── technical_analysis.rs  VectorTA registry + playhead helpers
└── ui/                 Trait impls for pulsar_marketlab_ui panes

pulsar_marketlab_ui
├── workspace/context.rs       WorkspaceContext, ManagedUsdStage, MVU mutations
├── workspace/stage_composer.rs  Layer stack + metadata tree-table
├── workspace/node_canvas.rs     DCC canvas frame, grid paint, Blender layout constants
├── workspace/graph_engine.rs    Invalidation observer + background recompile
└── workspace/param_inspector.rs Global overview + OTL editor shell

pulsar_marketlab_core
├── resources/usd/schema.usda   Financial schema (embedded in lib)
└── orchestration/
    ├── engine.rs    MarketLabGraphEngine
    └── compiler.rs  Vectorized OTL lexer/parser for inputs:script_src
```

---

## 6. OTL DAG — How the Graph Is Constructed

### 6.1 Three-tier node taxonomy

Implemented in `graph_compiler/registry.rs` as `NodeType`:

| Tier | Rust variant | USD `typeName` | Executable? | Role |
|------|--------------|----------------|-------------|------|
| **1** | `AssetAdaptor { prim_path }` | `FinancialAsset` | No | Structural instrument reference; CSV/Yahoo source |
| **2** | `OtlShader { script }` | `OtlOperator` | Yes | OTL formula, VectorTA indicator, or stdlib |
| **3** | `TerminalIntegrator { engine_target }` | `PortfolioIntegrator` | Yes | Terminal sink (`"portfolio"`, `"vector_ta"`, …) |

Canvas state: **`VisualNode`** + **`NodeConnection`** (port-indexed edges).  
Snapshot: **`PipelineGraphSnapshot`** — adds `execution_order`, `dag_valid`, `wiring_valid`, `wiring_errors`.  
Thread-safe mirror: **`SharedPipelineGraph`** (used by playhead workers).

### 6.2 Port wire kinds (`PortWireKind`)

| Kind | Semantics |
|------|-----------|
| `StructuralPath` | Tier-1 USD path reference (asset → OTL `inputs:underlying`) |
| `NumericSignal` | Tier-2 scalar/vector signal |
| `Aov` | Arbitrary Output Variable from OTL shader (`aov:confidence`, etc.) |

**Port assignment rules (summary):**
- **AssetAdaptor:** outputs `StructuralPath` only; no inputs.
- **OtlShader:** input 0 = `StructuralPath`; further inputs = `NumericSignal`; outputs split numeric vs AOV based on `aov_outputs`.
- **TerminalIntegrator:** inputs are `NumericSignal` or `Aov` (by port label); outputs `NumericSignal`.

### 6.3 Valid tier topology (`tier_topology_allows`)

Allowed edges (from → to):
- Asset → OTL Shader  
- Asset → Portfolio *(direct buy-and-hold path)*  
- OTL → OTL  
- OTL → Terminal Integrator  
- Portfolio → Portfolio *(nested portfolios)*  

**Kind rule:** ports must match kinds, **except** `StructuralPath → NumericSignal` is allowed for Asset → Portfolio direct wiring.

Invalid wiring is recorded in `wiring_errors` but USDA composition may still proceed; **`dag_valid`** is separate (cycle detection via Kahn).

### 6.4 DAG compilation — two compilers

| Path | Where | Input | Output |
|------|-------|-------|--------|
| **Canvas DAG** | `execution_engine/mod.rs` | `VisualNode[]`, connections | `ExecutionGraph.execution_order` |
| **USD DAG** | `pulsar_marketlab_core/orchestration/engine.rs` | `StageGraphSnapshot` from live stage | `MarketLabGraphEngine` + petgraph toposort |

Both use **Kahn topological sort**. Cycles → `dag_valid = false` / `GraphEngineError::CycleDetected`.

### 6.5 Blender-inspired spatial layout

Constants in `pulsar_marketlab_ui/workspace/node_canvas.rs`:
- **Column 0** = assets, **1** = OTL shaders, **2** = portfolios (`blender_slot_position(tier, row)`).
- **`VisualNode.collapsed`** → capsule pill; only wired sockets shown on perimeter.
- **`ui:canvas:pos`** custom attribute persists node position in composed USDA.

Scene root: **`/MarketLab`** (`trading_stage/scene.rs`). Legacy paths `/assets/`, `/analytics/`, `/portfolios/` still appear in older fixtures.

---

## 7. OTL DSL — Two Frontends (Critical Reality)

MarketLab has **two OTL evaluators** with different ASTs, numeric types, and execution contexts. Agents must not conflate them.

### 7.1 Playhead DSL — `signal_dsl/` (Phase C, OSL-inspired)

**When used:** Interactive playhead, sidebar diagnostics, per-bar TA display, `evaluate_formula()` in hot path.

| Piece | Location |
|-------|----------|
| Lexer/parser | `signal_dsl/parser.rs` |
| AST | `signal_dsl/ast.rs` — `DslExpression` |
| Compiler | `signal_dsl/interpreter.rs` → `OtlClosure = Arc<dyn Fn(&dyn MarketProviderServices, f64) -> Option<Vector>>` |
| Services trait | `signal_dsl/services.rs` — `MarketProviderServices` (OSL `RenderServices` analogue) |

**Variables:** `close`, `open`, `high`, `low`, `volume`, absolute paths, `global::*`, `portfolio::cash`.

**Functions (representative):**
- `sma(...)`, `ta::rsi`, `ta::sma`
- `rtn::log`, `vol::realized`, `vol::parkinson`
- `clamp`, `mix`, `step`
- `integrator::{name}(...)`

**Numeric type:** predominantly **`f32`** at playhead boundary.

**Script precedence on canvas OTL nodes:**
1. `VisualNode.dsl_formula` (user-edited in Param Inspector)  
2. `NodeType::OtlShader.script`  
3. VectorTA fallback via `ta_indicator_id` + `ta_lookback_period`

Test exemplar: `"close - sma(3)"` over a rolling window (`graph_compiler/tests.rs`).

### 7.2 Vectorized series OTL — `pulsar_marketlab_core/orchestration/compiler.rs`

**When used:** `MarketLabGraphEngine::compile_otl_scripts()` for **full-timeline** sweeps after USD invalidation.

| Piece | Role |
|-------|------|
| Tokenizer/parser | Reads `inputs:script_src` from USD prim |
| AST | Separate `Expr` tree |
| Output | `SeriesClosure = Box<dyn Fn(&[f64]) -> Vec<f64> + Send + Sync>` |

**Input identifiers:** `data`, `input`, `close`, `price`, `x` → input series.

**Functions:** `sma`/`ta::sma`, `macd`/`ta::macd`, `cross`/`ta::cross`, `identity`.

**Numeric type:** **`f64`** series.

**Reality:** Function surface is **narrower** than playhead DSL. Scripts authored for interactive eval may not compile on the graph engine without porting.

### 7.3 VectorTA fallback

`technical_analysis.rs` — registry-driven indicators (`rsi`, `ema`, `sma`, …).  
When no OTL script resolves, canvas writes shorthand like `rsi(period=14)` into `inputs:script_src` during `canvas_compose.rs`.

### 7.4 USD serialization of scripts

`canvas_compose.rs` → `otl_script_src()` picks DSL / script / indicator shorthand and writes **`inputs:script_src`** on `OtlOperator` prims.  
Relationships:
- Asset → OTL: `inputs:underlying`  
- Asset/OTL → Portfolio: `inputs:sources`  
- OTL → OTL: `inputs:sources` or `inputs:underlying` (per edge kind)

---

## 8. End-to-End Data Flow

```
User edits canvas
       │
       ▼
sync_pipeline_graph()                          [workspace_state.rs]
       ├── SharedPipelineGraph ← nodes + connections + validation
       └── publish_canvas_to_usd_stage()
                 ├── compose_pipeline_usda()
                 ├── reload UsdStageBridge
                 └── reload WorkspaceContext (preserve selection, edit-target)
       │
       ├── sync_workspace_ledger() → Stage Composer rows
       │
       ├── recompute_playhead_diagnostics() → signal_dsl / VectorTA @ playhead
       │
       └── WorkspaceContext.engine_cache_generation++
                 │
                 ▼
       graph_engine invalidation worker       [pulsar_marketlab_ui/graph_engine.rs]
                 ├── build_stage_graph_snapshot(ManagedUsdStage)
                 ├── MarketLabGraphEngine::compile_from_stage()
                 ├── execute_timeline()
                 └── replace_computed_streams(ComputedAttributeStream[])
```

**Reverse path (open document):** `.usda` file → `hydrate_canvas_from_stage()` → repopulate `VisualNode` / connections from prim relationships + `ui:canvas:pos`.

**CSV assets:** spawn node → background Yahoo/CSV ingestion → samples into `MarketStage` keyed by asset prim path.

---

## 9. OpenUSD Schema (Structural Vocabulary)

File: `crates/pulsar_marketlab_core/resources/usd/schema.usda`

| Class | Key inputs | Key outputs / rels |
|-------|------------|-------------------|
| `FinancialAsset` | `symbol`, `asset_class`, `provider`, `active` | `rel inputs:underlying` |
| `OtlOperator` | `id`, `script_src`, `active` | `rel inputs:underlying`, `outputs:signal` |
| `PortfolioIntegrator` | `id` (allocation token), `initial_capital`, `rebalance_frequency` | `rel inputs:sources`, `outputs:portfolio_wealth` |

Allocation tokens: `Allocation::HierarchicalRiskParity`, `Allocation::EqualWeight`, `Allocation::MeanVariance`.

Embedded at compile time via `FINANCIAL_SCHEMA_USDA` in `pulsar_marketlab_core/src/lib.rs`.

---

## 10. UI Workstation Model (Agent Constraints)

| Pane | Host trait | Key behavior |
|------|------------|--------------|
| Stage Composer | `StageComposerPane` | Layer stack, metadata grid, `select_stage_path()` — **always defer** context writes |
| Node Canvas | `NodeCanvasPane` | DCC grid, capsule mode, inline dropdowns; selection sync via `ui_selection_generation` |
| Param Inspector | `ParamInspectorPane` | OTL editor when OTL selected; **global overview** when no `selected_path` |
| Render Viewport | `RenderViewportPane` | Playhead, charts (partially wired) |

**Interaction rules (from USDTweak / newGUI rules):**
- Do **not** write USD on every mouse-move; use `cx.defer()` and commit on mouse-up for sliders/dropdowns.
- Do **not** run OTL compile / graph sort on UI render thread—use `cx.background_executor().spawn()`.
- Selection is unified: canvas node ↔ stage tree via `WorkspaceContext.selected_path` + `ui_selection_generation`.

---

## 11. Resolved Technical Debt (May 2026)

| Former gap | Resolution |
|------------|------------|
| **Dual OTL evaluators** | Shared `resolve_otl_script_src()` + `compile_unified_script()` in `pulsar_marketlab_core/orchestration/script_resolve.rs`. Graph engine normalizes playhead syntax (`sma(3)`, `rsi(period=14)`) before vectorized compile. Core compiler extended with `rsi`, `ema`, `clamp`, `mix`, `step`. |
| **Edit targets UI-only** | `ManagedUsdStage` tracks `edit_target_layer`; attribute overlays are keyed by layer. `set_edit_target_layer()` syncs `WorkspaceContext` ↔ stage; `modify_attribute` writes scoped opinions. |
| **Performance placeholders** | Param Inspector global overview shows graph revision, computed stream count, last compile ms, playhead eval status, stage overlay KiB. |
| **Invalid wires in USDA** | `validated_connections()` filters compose input; `publish_canvas_to_usd_stage()` blocks reload when `!wiring_valid` or `!dag_valid` and logs reasons to status panel. |

### Remaining limitations

| Area | Reality |
|------|---------|
| **Playhead vs series AST** | Still two parsers (`signal_dsl` vs `orchestration/compiler`); unified via resolution + normalization bridge, not single AST. |
| **Disk-layer edit targets** | Overlays are in-memory per layer id; flushing opinions into physical sublayers requires openusd edit-target API (future). |
| **Full OTL parity** | Playhead DSL financial/integrator functions not all ported to series compiler yet. |

---

## 12. Agent Playbook — Adding Features Safely

### Canvas / graph features
1. Extend `NodeType` / ports in `graph_compiler/registry.rs` if new tier behavior.  
2. Update `tier_topology_allows`, `connection_is_valid`, port kind helpers.  
3. Mirror in `canvas_compose.rs` (USD relationships) and `canvas_hydrate.rs` (restore).  
4. Add tests in `graph_compiler/tests.rs`.

### OTL / script features
1. Decide execution context: **playhead** (`signal_dsl`), **timeline** (`orchestration/compiler.rs`), or **both**.  
2. Add parser + interpreter + tests in the appropriate crate.  
3. Wire `otl_script_src()` / `effective_otl_script()` precedence if canvas-visible.  
4. Extend `ProductionStageProvider::execute_integrator` if new integrator names are exposed.

### USD / stage features
1. Schema changes → `schema.usda` + `canvas_compose` attribute writers.  
2. Structural mutations → `WorkspaceContext::modify_attribute` (invalidates engine cache).  
3. Never block UI thread on `Stage::open`.

### UI features
1. Implement pane traits in `pulsar_marketlab/src/ui/*.rs`.  
2. Keep visual tokens in `pulsar_marketlab_ui/src/theme.rs`.  
3. Observe `WorkspaceContext` via `install_ui_selection_observer`.

---

## 13. Key File Index (Quick Navigation)

| Concern | Path |
|---------|------|
| Node taxonomy & wiring | `crates/pulsar_marketlab/src/graph_compiler/registry.rs` |
| Canvas snapshot | `crates/pulsar_marketlab/src/graph_compiler/mod.rs` |
| Canvas ↔ USD compose | `crates/pulsar_marketlab/src/canvas_compose.rs` |
| Canvas ↔ USD hydrate | `crates/pulsar_marketlab/src/canvas_hydrate.rs` |
| Workspace sync hub | `crates/pulsar_marketlab/src/workspace_state.rs` |
| Playhead OTL | `crates/pulsar_marketlab/src/signal_dsl/` |
| Timeline OTL | `crates/pulsar_marketlab_core/src/orchestration/compiler.rs` |
| USD graph engine | `crates/pulsar_marketlab_core/src/orchestration/engine.rs` |
| Graph invalidation UI | `crates/pulsar_marketlab_ui/src/workspace/graph_engine.rs` |
| Split-plane provider | `crates/pulsar_marketlab/src/stage_bridge/production_provider.rs` |
| Temporal stage | `crates/pulsar_marketlab/src/trading_stage/market_stage.rs` |
| Scene paths | `crates/pulsar_marketlab/src/trading_stage/scene.rs` |
| USD schema | `crates/pulsar_marketlab_core/resources/usd/schema.usda` |
| VectorTA | `crates/pulsar_marketlab/src/technical_analysis.rs` |
| E2E spec test | `crates/pulsar_marketlab/tests/end_to_end_core_spec.rs` |

---

## 14. Glossary

| Term | Meaning |
|------|---------|
| **OTL** | Open Trading Language — expression layer for signal transforms on `OtlOperator` nodes |
| **OTL DAG** | Directed acyclic graph of three-tier nodes, validated and topologically sorted for execution |
| **AOV** | Arbitrary Output Variable — named extra output port on OTL shaders |
| **Prim** | OpenUSD scene graph node at a path (e.g. `/MarketLab/SPY`) |
| **Playhead** | Current bar index / stage time coordinate driving interactive eval |
| **LIVRPS** | USD composition strength ordering (Local, Inherited, Variant, Reference, Payload, Specialize) |
| **DCC** | Digital Content Creation — UI/UX reference (Blender, usdtweak) |
| **MVU** | Model-View-Update — GPUI entity pattern for `WorkspaceContext` |

---

*When this brief conflicts with source code, trust the code and update this document.*
