# Repository Guidelines

## Project Overview

Strimzi Ops is a Rust platform to **lint, monitor, and control** Kafka Connect on Kubernetes/Strimzi:

- **Lint** — `strimzi-lint` CLI validates connector YAML/JSON against built-in rules plus per-connector-class schema checks
- **Monitor** — consumes Debezium notification topics (Kafka) and tracks snapshot lifecycle
- **Control** — triggers snapshots via Debezium Signals, pauses/resumes/restarts connectors, exports Strimzi `KafkaConnector` YAML, streams Connect pod logs
- **Web UI** — Axum + Askama + HTMX (Dashboard, Control, timed Monitor) on `127.0.0.1:8501`

Rust only (Python tooling removed). All domain logic lives in `strimzi-ops-core`; binaries are thin wrappers.

## Architecture & Data Flow

Cargo workspace (edition 2021, `rust-version = "1.75"`, resolver 2) with three crates:

| Crate | Role |
|---|---|
| `crates/strimzi-ops-core` | Library: linter, schema validation, Connect REST client, control, monitor, k8s helpers, settings |
| `crates/strimzi-ops` | Bins `strimzi-ops` + `strimzi-lint` (clap) |
| `crates/strimzi-ui` | Axum web UI (bin `strimzi-ui`) |

Key data flows:

- **Lint**: file → `parse_config_text` (YAML→JSON normalization, unwraps `{name, config}` REST payloads) → `ConnectorLinter` (5 built-in rules + `.lintrc.toml` severities/exemptions + inline `# lint-disable:`) → `validate_schema` → `ValidationReport`. Errors fail; warnings fail only with `--strict`; `--json` emits the structured report (`validate.rs`, `linter/`).
- **Connect REST**: `ConnectClient` (`connect/client.rs`) is a **synchronous** reqwest client against port 8083; non-2xx → `Error::ConnectHttp` with a port-forward hint.
- **Control snapshot**: `SnapshotTrigger` reads `signal.kafka.topic` from the connector config (default `debezium.signals`), produces an `execute-snapshot` signal via rdkafka, and falls back to restarting task 0 on failure (`control/snapshot.rs`).
- **Monitor**: `NotificationMonitor` (rdkafka `BaseConsumer`) → JSON events → `SnapshotTracker` state machine: STARTED → IN_PROGRESS → COMPLETED/ABORTED (`monitor/`).
- **UI**: `AppState` holds only `Vec<ConnectCluster>`; each request resolves a cluster then builds a fresh `ConnectClient` inside `tokio::spawn_blocking` — never hold the blocking client across an await (`state.rs`, `blocking.rs`). Routes: `/c/{cluster}/dashboard|monitor|control`; the logs page shells out to `kubectl`.

## Key Directories

- `crates/strimzi-ops-core/src/` — domain library: `linter/` (engine, rules, config, directives), `validate.rs`, `schema.rs`, `parse.rs`, `connect/`, `control/`, `monitor/`, `k8s.rs` (kubectl shell-out), `settings.rs`
- `crates/strimzi-ops/src/` — CLI: `lib.rs` (clap `Cli` + subcommands), `main.rs`, `bin/strimzi_lint.rs`
- `crates/strimzi-ui/src/` — `main.rs`, `state.rs`, `blocking.rs`, `routes/` (dashboard/control/monitor), `views.rs` (Askama + Error→HTTP mapping), `paths.rs`
- `crates/strimzi-ui/tests/http_routes.rs` — sole integration test dir
- `k8s/` — local-dev stack (namespace `kafka`: Strimzi 1.1.0, Kafka 4.3.0, Connect 4.3.0 with Debezium 3.6.0 + Iceberg sink, Postgres 18.4, Garage S3, Nessie catalog) + `deploy.sh`/`destroy.sh`/`Dockerfile.connect`
- `scripts/local-dev.sh` — helpers behind the port-forward/secrets/cluster just recipes
- `examples/` — sample connectors (Debezium Postgres, Iceberg sink, lint-exemption demo); linted by CI
- `docs/rust-best-practices.md`, `.cursor/rules/` — style standard (must follow)

## Development Commands

```bash
just rust-check          # cargo fmt --all -- --check + cargo clippy --workspace --all-targets --all-features -- -D warnings
just rust-test           # cargo test --workspace --all-features
just ui                  # background port-forwards + strimzi-ui on :8501
just lint-config <file>  # run strimzi-lint against one connector config
just setup               # full local stack: Colima → build Connect image → k8s deploy → secrets → forwards → UI
just port-forward-all    # background forwards (state in .local/port-forwards/), verifies Connect :8083
just status-forwards / just stop-forwards
just doctor              # pods + forward status + HTTP health checks
just destroy             # tear down the k8s stack
```

Ports: Connect `8083`, Kafka `9092`, Postgres `5432`, Garage S3 `3900`, Nessie `19120`, UI `8501`.

## Code Conventions & Common Patterns

- Follow Canonical Rust best practices (`docs/rust-best-practices.md`). Workspace lints forbid `unsafe` and enable clippy `pedantic` (allow-list fixed in root `Cargo.toml`). `just rust-check` + `just rust-test` are the pre-finish gate.
- **Error handling**: concrete `thiserror` enums; no `anyhow`/`Box<dyn Error>` in `strimzi-ops-core`. Core defines `Error` + `Result<T>` in `lib.rs`; each binary crate has an `error.rs` + `result.rs` pair. The UI maps `Error` to HTTP statuses in `views.rs`. Error messages are shaped `cannot …` (lowercase).
- No `.unwrap()`/`.expect()` outside tests.
- `mod.rs` files are thin re-exports only — no logic.
- **Blocking vs async**: `ConnectClient` and rdkafka are synchronous. Only `strimzi-ui` is async (`#[tokio::main]`) and must wrap blocking calls in `with_connect_client`/`spawn_blocking`.
- **DI & state**: constructor injection (`ConnectClient::new(url)`, `SnapshotTrigger::new(client, bootstrap)`); no global singletons; handlers take `State<AppState>`. Settings come from `LoadConfig` (secrets.toml) + `merge_overrides` + `require_*()` accessors that fail with `MissingSetting`.
- **Feature gating**: all rdkafka code sits behind the `kafka` cargo feature (default off in core; both binaries enable it). Non-kafka builds compile `#[cfg]` stubs returning `Error::KafkaFeatureDisabled`.
- Naming: `*Client`/`*Monitor`/`*Tracker`/`*Trigger` for actors, `*Config`, `*Page` (Askama), `*Result`; `HtmlResult` for UI handlers.
- Style: `?` on golden paths, explicit `Ok(())` on `Result<()>`, prefer `Self` in inherent impls, tight mutability scopes.
- Connector YAML must lint clean: `just lint-config <file>`; rule severity/exemption overrides live in `.lintrc.toml`; per-file `# lint-disable: rule1, rule2` comments are supported.

## Important Files

- `Cargo.toml` — workspace members, shared deps, lints (clippy pedantic allow-list)
- `crates/strimzi-ops-core/src/lib.rs` — core `Error` enum + module map/re-exports
- `crates/strimzi-ops-core/src/settings.rs` — `secrets.toml` contract (`[kafka]` `connect_url`/`bootstrap_servers`, `[storage]` S3) + CLI overrides
- `crates/strimzi-ops/src/lib.rs` — clap CLI surface (subcommands, global flags)
- `crates/strimzi-ui/src/main.rs`, `crates/strimzi-ui/src/routes/mod.rs` — server bootstrap, route table
- `justfile`, `.github/workflows/ci.yml` — local and CI command surface
- `.lintrc.toml` — `strimzi-lint` rule config (not a Python linter)
- `secrets.toml` (gitignored; template `secrets.toml.example`)
- `k8s/Dockerfile.connect` — local Connect image (`my-connect-cluster:0.0.3`)

## Runtime/Tooling Preferences

- **Rust**: stable toolchain, floor 1.75 (`rust-version`); no pinned `rust-toolchain*` file. Use workspace dependencies (`dep.workspace = true`) and don't pin versions in crate manifests.
- **Build deps**: rdkafka 0.37 builds from source (`cmake-build` feature) → **cmake required**; CI additionally installs `pkg-config libssl-dev libcurl4-openssl-dev libsasl2-dev libzstd-dev`.
- **Local stack tooling**: `kubectl` (UI logs, `--from-k8s` discovery), `docker buildx`, and Colima are assumed by the justfile k8s recipes.
- **No Python** in application code; stale caches (`.mypy_cache/`, `__pycache__/`, …) are gitignored leftovers.

## Testing & QA

- Run: `just rust-test` (= `cargo test --workspace --all-features`). No coverage expectations.
- Unit tests: 14 inline `#[cfg(test)]` modules across `strimzi-ops-core` (sync `#[test]`) plus one in `strimzi-ui/src/routes/dashboard.rs`.
- Connect REST is mocked with **httpmock** (`MockServer` + `mock.assert()`); UI route tests use **tower** `ServiceExt::oneshot` against the axum router in `crates/strimzi-ui/tests/http_routes.rs` (8 `#[tokio::test]`, no real server).
- Fixtures are inline `json!`/string literals; `examples/` files are not test fixtures — CI lints them instead.
- No `#[ignore]`d or cluster-dependent tests; kafka-feature code is tested via its fallback paths.
- CI (`.github/workflows/ci.yml`, single `rust` job): `cargo fmt --check` → `clippy -D warnings` → `cargo test` → `strimzi-lint` over every `examples/*.{yaml,yml,json}`.
- Test style: snake_case verb-first names, `use super::*`, shared `fn map(Value)` helper in the schema/validate/rules tests.
