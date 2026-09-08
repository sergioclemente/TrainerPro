# Contributing to TrainerPro

Thanks for your interest! This is a small project with strong architectural
opinions — a quick read here will save you a rewritten PR.

## Ground rules

- **Open an issue before building a feature.** Small bug fixes and doc fixes
  can go straight to a PR.
- Good areas for contribution: workout sources, exporters, parsers for new
  file formats, device drivers (behind the existing traits), Windows
  validation, tests for currently-untested modules.
- Decisions recorded in [`docs/ALTERNATIVES.md`](docs/ALTERNATIVES.md) are
  settled unless you bring new information — re-litigating them in a PR
  without an issue first won't go well.

## Dev setup

Prerequisites: [Rust](https://rustup.rs) (stable), Node 20+, and on macOS the
Xcode Command Line Tools.

```bash
npm install
npm run tauri:qa
```

This launches **TrainerPro QA** with an isolated data directory. The regular
TrainerPro bundle remains the day-to-day app and keeps its workouts, rides,
and paired devices separate. Prefer the simulated devices in QA, and do not
connect both app flavors to the same physical trainer at once.

No trainer needed: pair **Simulated KICKR** / **Simulated HRM** from the
Devices screen. The simulator implements the same traits as the real drivers,
including fault injection.

## Before you push

```bash
cargo test --workspace
cargo fmt --all
cargo clippy --workspace
npx tsc --noEmit
```

New parser, engine, metrics, or FIT code must come with unit tests — `tp-core`
is pure and hardware-free, so there's no excuse not to.

## Architecture guardrails

Two invariants keep this codebase testable. PRs that break them will be asked
to restructure, however good the feature is:

1. **`tp-core` stays zero-I/O, zero-async, zero-BLE/Tauri.** If your change
   needs I/O, a clock, or a network, it belongs in `tp-app` (the Tauri crate);
   `tp-core` gets the pure logic and the tests.
2. **Hardware only behind the `TrainerConnection` / `HeartRateConnection` traits**
   (`crates/tp-ble/src/traits.rs`). If the simulator can't exercise your
   change, redesign it until it can.

Two conventions worth knowing:

- **ZWO is the interchange format.** Every workout source ultimately produces
  ZWO text and funnels through `sources::ride_from_zwo` — sources never touch
  import, database, or player code directly.
- **The UI is push-only.** The frontend never polls; state arrives via the
  Tauri event stream (see `wireEvents()` in `frontend/state.ts`).

## Adding a workout source

1. Backend: a module that produces ZWO text and calls
   `sources::ride_from_zwo` (see `planner.rs` / `woz.rs` as examples).
   Config lives in the schemaless `SourceConfig` bag — no DB migration needed.
2. Frontend: one tab component + one descriptor appended to
   `frontend/sources.ts`. The Libraries settings form is generated from the
   descriptor's `fields`.

See [`docs/architecture.md`](docs/architecture.md) for the diagram.

## Releases

Maintainer-run: tag `v*` triggers `.github/workflows/release.yml`, which
builds unsigned macOS/Windows installers and attaches them to a GitHub
release.
