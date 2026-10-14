# Rust development guidelines

This repository's Rust code follows **Canonical's Rust best practices**:

- Book: <https://canonical.github.io/rust-best-practices/introduction.html>
- Source: <https://github.com/canonical/rust-best-practices>

Treat that guide as the style baseline for all new Rust in this project.

**Cursor / agents:** also see [AGENTS.md](../AGENTS.md) and `.cursor/rules/`
(`project.mdc`, `rust-canonical.mdc`) so future sessions load these instructions
automatically.

## Preconditions

Before opening a PR that touches Rust:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Or via just:

```bash
just rust-check
just rust-test
```

## Structural discipline

- Prefer directory modules with `mod.rs` that only declare submodules and
  re-exports (no logic in `mod.rs`).
- Library crate (`strimzi-ops-core`): define `Error` and `Result` in `lib.rs`
  immediately after `mod` / `use` declarations.
- Binary crates (`strimzi-ops`, `strimzi-ui`): define `Error` in `error.rs` and
  `Result` in `result.rs`.

## Error and panic discipline

- Use concrete `thiserror` enums in libraries (no `anyhow` / type erasure in
  `strimzi-ops-core`).
- Convert dependency errors at the call boundary into crate-local `Error`.
- Prefer messages shaped like `cannot …` with consistent lowercase phrasing.
- Avoid `.unwrap()` / `.expect()` outside tests and true programmer errors
  (for example a statically known-valid regex).

## Code / function discipline

- Prefer `Self` in inherent impls.
- Scope mutability tightly.
- End unit-returning statements with `;`; for `Result<()>` golden paths use
  `?` and an explicit `Ok(())`.

## Workspace layout

```
Cargo.toml                 # workspace root
crates/strimzi-ops-core/   # library: lint, Connect client, control, monitor, k8s, settings
crates/strimzi-ops/        # binaries: `strimzi-ops` (full CLI) + `strimzi-lint` (compat)
crates/strimzi-ui/         # Axum + Askama + HTMX Dashboard/Control UI
```

Library modules use directory `mod.rs` files. Kafka producer/consumer code is behind the
`kafka` feature on `strimzi-ops-core` (enabled by the CLI and UI). Building with Kafka requires
**librdkafka** (Homebrew: `brew install librdkafka cmake pkg-config`).

Binary crates (`strimzi-ops`, `strimzi-ui`): define `Error` in `error.rs` and `Result` in
`result.rs`.

## Migration plan (agents)

1. Core + lint CLI — done
2. Connect client / monitor / control as library + CLI — done
3a. Axum + HTMX Dashboard + Control (`strimzi-ui`); Streamlit removed — done
3b. Live Monitor (SSE) + kubectl logs in Rust UI — done
4. Python package and `uv` tooling removed — done

Default UI is `just run` / `just ui` (`strimzi-ui`). Monitor streams a duration-bounded Kafka consume session over SSE, with one consumer group per connection; disconnecting cancels consumption. Control Logs uses `kubectl` on `PATH`.

Connect/Debezium metrics live in `strimzi-ops-core::metrics`: a synchronous,
bounded per-worker collector and Prometheus parser with explicit availability
states. The UI owns one application-wide collector, executes each round with
`spawn_blocking`, and serves only cached samples to Monitor browsers. Blocking
HTTP clients are constructed and dropped inside the round, never retained
across an async wait. Worker snapshots share immutable sample storage rather
than copying every metric name and label on each refresh.

Snapshot, streaming, and backpressure projections share a private borrowed-series
accumulator with exact label-count, finite/nonnegative, and optional boolean
validation. Duplicate series are unavailable even on failed workers; other
missing or invalid series on failed workers remain collection errors. Only
available or stale readings retain values. Projection identities, ownership,
ordering, and derived measurements remain specific to each projection.
