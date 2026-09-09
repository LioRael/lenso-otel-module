# Lenso OpenTelemetry Plugin

A removable OpenTelemetry Plugin for Lenso applications. It consumes the
published Kernel and Native Execution Adapter Interfaces; observability
semantics do not live in the portable core.

The source was extracted from `LioRael/lenso` at monorepo commit
`67d21499548d07e92c2f6529d7c8345e58c067d9` under ADR 0064. Imported subtrees
retain their relevant Git history.

## OTLP/HTTP exporter

Native Hosts can inject the included Protobuf exporter without putting its
endpoint, bearer token, or service identity in a Resolved App Plan:

```rust,ignore
use lenso_otel_plugin::{OtelPluginFactory, OtlpHttpExporter};

let exporter = OtlpHttpExporter::new(
    "http://127.0.0.1:4318",
    &private_token,
    "sample-web",
)?;
let factory = OtelPluginFactory::new(runtime_diagnostics, exporter)
    .with_telemetry_capability();
```

The exporter sends binary OTLP/HTTP Protobuf to `/v1/traces` and `/v1/logs`.
It maps an ended monotonic span onto the export-time Unix clock while preserving
its duration; HTTP root attributes select the OTLP server-span kind. Metrics and
unfinished spans are explicitly rejected in this first implementation. Export
failure remains generation-local telemetry loss and never changes App business
outcomes.

## Validation

```sh
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
bun test fixtures/otel/trace-context-conformance.test.ts
```

## Release

Publication uses `release-plz` and crates.io Trusted Publishing from
`.github/workflows/release-plz.yml`. A release PR is the version boundary; the
release job receives only GitHub OIDC authority and never a registry token.
