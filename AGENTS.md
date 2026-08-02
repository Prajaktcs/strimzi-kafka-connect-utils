# Agent instructions

This file orients Cursor (and humans) for work in this repository.

## Read first

1. [docs/rust-best-practices.md](docs/rust-best-practices.md) — Rust style for this project
2. [Canonical Rust best practices](https://canonical.github.io/rust-best-practices/introduction.html) — upstream guide we adopt
3. `.cursor/rules/` — Cursor rules loaded automatically in sessions

## Project goal

Strimzi Ops: lint, monitor, and control Kafka Connect on Kubernetes/Strimzi.

## Stack

Rust-only application code under `crates/`:

| Crate | Role |
|-------|------|
| `strimzi-ops-core` | lint, Connect client, control, monitor, k8s helpers, settings |
| `strimzi-ops` | CLI (`lint`, connectors, cluster, snapshot, monitor) + `strimzi-lint` compat bin |
| `strimzi-ui` | Axum + Askama + HTMX web UI |

Python has been removed from the project.

## Commands

```bash
just rust-check          # fmt + clippy -D warnings
just rust-test
just run                 # ensures port-forwards, then Rust UI
just ui                  # same as just run (strimzi-ui on :8501)
just port-forward-all    # (re)start + health-check Connect :8083
just status-forwards
just doctor
just lint-config <file>
cargo run -p strimzi-ops -- connectors list --connect-url http://127.0.0.1:8083
```

Control/monitor builds need system **librdkafka** (and cmake when building `rdkafka` from source). On macOS: `brew install librdkafka cmake pkg-config`.

Control **Logs** in the UI shells out to **kubectl** (must be on `PATH` and configured for the cluster).

## Non-negotiables for Rust PRs

- Follow Canonical structural, error/panic, and code discipline (see docs above)
- No type-erased errors in library crates
- Pass `just rust-check` and `just rust-test`
