# Engineering Directive: FinanceDatabase Ingestion Layer

## Context
We are integrating JerBouma/FinanceDatabase data matrices to turn our read-only `sp500_universe.usda` file into a rich, structured metadata repository.

## Rules & Structural Boundaries

1. **Enforce Split-Layer Layer Composition:**
   Never write code that combines external raw data directly into the active user composition files. Maintain the metadata context inside dedicated sublayer targets (`finance_database_equities.usda`). Mount them to the system workspace using the root `subLayers` array parameter block.

2. **Maintain Flat Stable Paths:**
   Every single asset primitive extracted from the catalog must be assigned a stable, lowercased leaf identifier string matching the format: `node_asset_{sanitized_ticker}`. 

3. **Schema Compliance:**
   All ingested variables (such as GICS sectors, country locations, or CUSIP keys) must be written as custom string properties under the `info:*` namespace explicitly specified inside `schema.usda`.

4. **Zero Heap Calculation Loop Policy:**
   These metadata properties are for structure, filtering, and user display overlays. The core execution engine thread must not parse or loop over these string primitives during high-performance backtesting simulation sweeps.
