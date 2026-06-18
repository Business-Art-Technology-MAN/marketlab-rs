# Engineering Directive: Implement Stage 2 Ingestion Binary

1. Create `crates/pulsar_marketlab_core/src/bin/ingest_universe.rs` matching our stream-parsed, buffer-wrapped file writing design.
2. Register the `ingest_universe` binary block inside `crates/pulsar_marketlab_core/Cargo.toml`.
3. Ensure the sanitization routine lowercases and converts all invalid characters (`.`, `-`, `/`) cleanly to underscores.
4. Verify that running `cargo check --bin ingest_universe` compiles perfectly with no dangling references or borrow checker complaints.
