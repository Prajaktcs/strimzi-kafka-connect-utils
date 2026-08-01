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
| 1 | `strimzi-ops-core` + `strimzi-lint` CLI | Started |
| 2 | Connect REST client + control/monitor library APIs | Planned |
| 3 | UI (prefer durable web stack over RustView for production) | Planned |

Keep Python UI/CLI usable until Rust replacements exist. Prefer putting new domain logic in Rust crates under `crates/`.

## Commands

```bash
just rust-check          # fmt + clippy -D warnings
just rust-test
just lint-config-rust <file>
just lint-config <file>  # Python CLI (still supported)
```

## Non-negotiables for Rust PRs

- Follow Canonical structural, error/panic, and code discipline (see docs above)
- No type-erased errors in library crates
- Pass `just rust-check` and `just rust-test`
