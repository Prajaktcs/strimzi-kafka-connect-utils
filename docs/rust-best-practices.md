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
- Binary crate (`strimzi-ops`): define `Error` in `error.rs` and `Result` in
  `result.rs` (shared lib used by `strimzi-ops` and `strimzi-lint` bins).

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
crates/strimzi-ops-core/   # library: lint, Connect client, control, monitor
crates/strimzi-ops/        # binaries: `strimzi-ops` (full CLI) + `strimzi-lint` (compat)
```

Library modules use directory `mod.rs` files. Kafka producer/consumer code is behind the
`kafka` feature on `strimzi-ops-core` (enabled by the CLI). Building with Kafka requires
**librdkafka** (Homebrew: `brew install librdkafka cmake pkg-config`).

## Migration plan (agents)

1. Core + lint CLI — done
2. Connect client / monitor / control as library + CLI — current
3. UI last — durable stack preferred; RustView only for disposable prototypes

Python remains the UI until step 3.
