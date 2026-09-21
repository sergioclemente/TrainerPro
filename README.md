# TrainerPro

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

TrainerPro is a desktop indoor-cycling workout player. It finds the next
workout, controls an FTMS smart trainer in ERG mode, records the ride, and
produces a Garmin-compatible FIT activity.

- **Execution-first Workouts screen:** scheduled workouts and recommendations
  appear in Next Up, with the local and connected libraries below.
- **Reliable trainer control:** FTMS ERG targets, ramps, keep-alive,
  auto-pause, and reconnect behavior.
- **Crash-safe recording:** a 1 Hz journal is converted to FIT at ride end,
  with laps and locally computed NP, IF, and TSS.
- **Offline-capable sources:** local ZWO/ERG/MRC files, WorkoutPlanner,
  What's on Zwift, and cached Intervals.icu schedules.
- **Hardware-free development:** simulated trainer and heart-rate devices use
  the same interfaces as physical BLE hardware.

## Status

| Platform | Status |
|---|---|
| macOS | Working; distributed builds are currently unsigned |
| Windows | Shared implementation exists; hardware and installer validation remain |
| Linux | Not supported |

## Install

Download the latest macOS build from
[Releases](https://github.com/sergioclemente/TrainerPro/releases). Because the
build is unsigned, first launch requires right-clicking the application and
choosing **Open**. Windows packages similarly show an unknown-publisher warning.

## Architecture

The React frontend talks to a Rust Tauri backend. `tp-core` contains pure
workout, engine, recording, metrics, and FIT logic. `tp-ble` owns BLE drivers,
connection traits, scanning, and simulators. SQLite is authoritative for TPW
workout definitions, schedules, provider state, and the Activity index; session
journals and FIT files remain durable activity artifacts.

See [the architecture guide](docs/architecture.md) for boundaries and flows.

## Documentation

| Document | Use it for |
|---|---|
| [Product](docs/PRODUCT.md) | Product direction, vocabulary, decisions, and non-goals |
| [Behavior spec](docs/SPEC.md) | Current user-visible behavior and acceptance expectations |
| [Architecture](docs/architecture.md) | Technical ownership, invariants, and system flows |
| [TPW](docs/TPW.md) | Normative TrainerPro Workout JSON format |
| [Roadmap](docs/ROADMAP.md) | Unfinished work and external gates |
| [Intervals.icu](docs/feature-intervals-icu.md) | Inbound schedule-sync contract |
| [WorkoutPlanner](docs/feature-workoutplanner.md) | Connected-library contract |

Documentation purpose and maintenance rules are in
[CONTRIBUTING.md](CONTRIBUTING.md).

## Repository layout

```text
frontend/       React UI, state, and typed IPC client
backend/        Tauri host, persistence, integrations, and runtime services
crates/tp-core/ Pure workout, engine, journal, metrics, and FIT domain logic
crates/tp-ble/  BLE contracts, drivers, device manager, and simulators
docs/           Product, behavior, architecture, and feature contracts
```

## Contributing

Local setup, QA workflows, architectural guardrails, documentation rules, and
verification commands are in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
