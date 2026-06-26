# CLAUDE.md

Guidance for Claude Code working in this repository.

## What Zenoh is

Zenoh ("zeno") is a zero-overhead pub/sub/query protocol unifying data in motion, data at rest, and computations. Eclipse Foundation project. Rust workspace, ~40 crates. Edition 2021, MSRV **1.75.0** (pinned in `Cargo.toml`), toolchain `1.93.0` (`rust-toolchain.toml`). License EPL-2.0 OR Apache-2.0.

## Workspace layout

- `zenoh/` — primary library + public API (`Session`, `Publisher`, `Subscriber`, `Querier`, `Queryable`).
- `zenoh-ext/` — extensions (advanced pub/sub, caching, serialization, group mgmt).
- `zenohd/` — standalone router daemon.
- `commons/` — 19 internal support crates. **Not a public API** — breaking changes any release.
  - `zenoh-protocol` (msg defs), `zenoh-codec` (wire codec), `zenoh-buffers` (zero-copy), `zenoh-keyexpr` (key-expr parse/match), `zenoh-config`, `zenoh-runtime`, `zenoh-sync`, `zenoh-task`, `zenoh-result`, `zenoh-crypto`, `zenoh-shm`, `zenoh-collections`, `zenoh-macros`, `zenoh-util`, `zenoh-stats`, `zenoh-core`, `zenoh-test`.
- `io/` — transport + links.
  - `io/zenoh-transport` — transport protocol / connection mgmt.
  - `io/zenoh-link`, `io/zenoh-link-commons` — link abstraction.
  - `io/zenoh-links/zenoh-link-*` — TCP, UDP, QUIC, QUIC-datagram, TLS, WebSocket, Unix socket/pipe, Serial, vSock.
- `plugins/` — `zenoh-plugin-trait` (loader), `zenoh-plugin-rest`, `zenoh-plugin-storage-manager`, `zenoh-backend-traits`, examples.
- `examples/` — usage reference.

## Routing internals (where to look)

- `zenoh/src/api/session.rs` — entry point (`zenoh::open(config)`), Session lifecycle.
- `zenoh/src/net/routing/`
  - `dispatcher/` — resource table + dispatch: `resource.rs`, `face.rs` (connection endpoints), `pubsub.rs`, `queries.rs`, `interests.rs`, `tables.rs`.
  - `hat/` — **H**ow **A**m **T**hat: mode-specific routing behavior. Subdirs `client/`, `peer/`, `router/`, `broker/`.
  - `gateway.rs`, `namespace.rs`, `interceptor/`.
- `zenoh/src/net/runtime/` — `orchestrator.rs` (scouting/connection mgmt), `adminspace.rs`.
- Node role = `WhatAmI` enum (router / peer / client).
- Config schema: `DEFAULT_CONFIG.json5` (root). JSON5 format. Routing knobs under `routing.*`.

## Build / test / lint

Build:
```bash
cargo build --release --all-targets
cargo +1.75.0 check --release          # MSRV check
```

Format (nightly rustfmt required):
```bash
rustfmt +nightly --check --config "unstable_features=true,imports_granularity=Crate,group_imports=StdExternalCrate,skip_children=true" $(git ls-files '*.rs')
```

Clippy (no warnings allowed):
```bash
cargo +stable clippy --all-targets --all-features -- --deny warnings
```

Test (uses nextest):
```bash
cargo nextest run -p zenoh -F test
cargo nextest run --all-targets --features test,unstable,internal
cargo nextest run <test_name>           # single test
cargo test --doc
```

Other CI gates: `cargo deny check licenses`, `cargo machete` (unused deps), `cargo semver-checks`, `taplo fmt --check` (TOML).

## Conventions

- Clippy `--deny warnings` — keep it clean.
- rustfmt: `imports_granularity=Crate`, `group_imports=StdExternalCrate`, field-init shorthand, try shorthand (`rustfmt.toml`).
- `clippy.toml` allows interior mutability on `Resource`.
- Feature flags: `unstable` (breaking APIs), `internal` (not public), `shared-memory`, `stats`, `transport_*` (per protocol), `auth_*`, `transport_compression`, `transport_multilink`, `plugins`.
- Pre-commit hooks in `.pre-commit-config.yaml`.

## Notes

- `commons/` and `io/` crates are internal — don't treat their APIs as stable.
- Default config is large (`DEFAULT_CONFIG.json5`, ~49KB) and is the source of truth for tunables.
