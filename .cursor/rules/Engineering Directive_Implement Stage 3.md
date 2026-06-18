# Engineering Directive: Complete Stage 3 Snapshot Hydration

1. Update `ComposedAssetMeta` in your snapshot engine to include fields for `sector`, `industry`, `market_cap_class`, `currency`, `country`, and `user_label`.
2. Update the snapshot hydration logic to pull these properties using the `info:*` namespace from your compiled `UsdStage`.
3. Wire these variables into the Param Inspector's UI grid underneath the primary asset attribute rows.
4. Verify that running our new test pass verifies successfully with zero regressions across our existing layout modules.
