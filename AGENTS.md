# Agent instructions

This file orients Cursor (and humans) for work in this repository.

## Read first

1. [docs/rust-best-practices.md](docs/rust-best-practices.md) — Rust style for this project
2. [Canonical Rust best practices](https://canonical.github.io/rust-best-practices/introduction.html) — upstream guide we adopt
3. `.cursor/rules/` — Cursor rules loaded automatically in sessions

## Project goal

Strimzi Ops: lint, monitor, and control Kafka Connect on Kubernetes/Strimzi.

## Migration (Python → Rust)

| Step | Scope | Status |
|------|--------|--------|
| 1 | `strimzi-ops-core` lint + schema; CLI lint | Done |
| 2 | Connect REST + control/monitor + `strimzi-ops` CLI | Done |
| 3a | Axum + HTMX Dashboard + Control UI (`strimzi-ui`); Streamlit removed | Started |
| 3b | Live Monitor + kubectl logs in the Rust UI | Planned |

Prefer putting new domain logic in Rust crates under `crates/`. Python remains for `strimzi-lint` and transitional library helpers.

## Commands

```bash
just rust-check          # fmt + clippy -D warnings
just rust-test
just run                 # Rust UI (alias for just ui)
just ui                  # strimzi-ui on :8501
just lint-config-rust <file>
cargo run -p strimzi-ops -- connectors list --connect-url http://localhost:8083
just lint-config <file>  # Python CLI (still supported)
```

Control/monitor builds need system **librdkafka** (and cmake when building `rdkafka` from source). On macOS: `brew install librdkafka cmake pkg-config`.

## Non-negotiables for Rust PRs

- Follow Canonical structural, error/panic, and code discipline (see docs above)
- No type-erased errors in library crates
- Pass `just rust-check` and `just rust-test`
