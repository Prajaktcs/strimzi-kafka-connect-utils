# Release checkpoint

This is a verification gate, not a production-readiness claim or an automated
publish. Do not bump a version or tag a release until the evidence below belongs
to the exact revision being released.

## Supported configurations

| Surface | Supported scope | Required evidence |
| --- | --- | --- |
| Local pipeline | Repository-pinned Strimzi/Kafka/Connect, PostgreSQL, Debezium, Iceberg and RustFS stack | Isolated backlog/recovery smoke and independent Iceberg row reads |
| Sink lag | Reachable advertised broker addresses; Connect and observer Kafka cluster IDs agree | Complete partition offsets, positive paused backlog and zero committed lag after recovery |
| Kafka security | Plaintext by default; explicit TLS or SASL observer configuration as documented in the root README | Successful authenticated collection; wrong trust/credentials must fail without exposing secrets |
| Core without Kafka | `strimzi-ops-core --no-default-features` | Core test gate; Kafka operations remain explicitly unavailable |
| Web UI | Local Axum/HTMX interface | Dashboard, Control and Monitor exercised against the intended cluster |

Use [runtime pin provenance](../k8s/README.md#runtime-pin-provenance) for exact
component versions and rollback constraints. The single-node HTTP storage stack
and shared local development credentials are not a supported production setup.
Consumer isolation/cluster overrides and unsupported topic regex constructs must
remain explicitly unavailable, never silently approximated.

## Reproducible gates

1. Start the existing local stack without deleting PVCs or resetting offsets.
   On an already provisioned Colima installation: `just ensure-cluster`, then
   `just port-forward-all`. For first-time provisioning use `just setup`.
2. Run `just release-check`: format/clippy, workspace tests, no-Kafka core tests,
   release compilation and example connector linting.
3. Run `just smoke-pipeline` against the healthy local stack. Require a baseline
   with known commits, positive lag while the isolated sink is paused, valid zero
   lag after recovery, and exact expected rows read independently from Iceberg.
   Preserve the command output and its evidence directory. A `RUNNING` connector
   or zero committed lag alone does not prove downstream durability.
4. With the intended secure Kafka configuration, verify fresh lag collection and
   failure on incorrect CA/credentials. Confirm errors, Debug output and UI pages
   contain no credentials. Plaintext smoke evidence does not establish TLS/SASL
   compatibility.
5. Open the Dashboard, Control and Monitor pages; verify that unavailable data is
   not shown as zero and stale observations are labeled historical. Check paused
   backlog and recovered lag in Monitor.
6. Save revision, OS/architecture, Rust version, component pins, commands, results,
   and smoke evidence location in release notes. Record unexercised configurations
   explicitly; do not call them verified.

The CI workflow can also be run manually from GitHub Actions. A manual run applies
the normal gates, tests the no-Kafka core, builds optimized binaries, and uploads
Linux binaries in a tar archive, preserving executable permissions after
extraction. It does not create a release, run a Kubernetes pipeline, or certify
configurations that were not exercised.

## Rollback and resource safety

Retain the prior revision and compatible PostgreSQL, Kafka/Connect and RustFS
state before upgrades. Do not delete PVCs, reset consumer groups or replay source
history as part of a release check. Follow the deployment guide's rollback steps.
Smoke resources are uniquely named: inspect their evidence and use only the
smoke's documented targeted cleanup. Never use namespace-wide deletion for
verification cleanup.
