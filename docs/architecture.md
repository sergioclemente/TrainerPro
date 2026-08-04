# Architecture notes

Companion diagrams to the overview in the [README](../README.md). The full
build spec is [`SPEC.md`](SPEC.md); design rationale and rejected alternatives
are in [`ALTERNATIVES.md`](ALTERNATIVES.md).

## Workout source plugin interface

Workout providers are plugins around one interchange format: **ZWO**. A source
either *is* a ZWO file (local import), *serves* ZWO (WorkoutPlanner), or
*builds a workout model and serializes it* with tp-core's ZWO writer
(whatsonzwift). Every source funnels into the same pipeline — none of them
touch import, database, or player code directly.

```mermaid
flowchart TD
    subgraph SRCS["Workout sources - tabs from src/sources.ts registry"]
        LOCAL["My Library<br/>local .zwo / .erg / .mrc files"]
        WP["WorkoutPlanner<br/>self-hosted server<br/>Basic Auth, /workout_file ZWO"]
        WOZ["Zwift<br/>whatsonzwift.com fetch<br/>textbars to model"]
        FUTURE["Future source...<br/>one tab component +<br/>one backend module"]
    end

    ZWO["ZWO text<br/>the interchange format"]
    PIPE["sources::ride_from_zwo<br/>import + sha256 dedup<br/>provenance tag: origin, origin_ref"]
    LIB[("Workout library<br/>files + SQLite")]
    PLAYER["Player runtime"]
    FIT["FIT file to Garmin Connect"]

    LOCAL -->|file picker / drag-drop| PIPE
    WP -->|GET /workout_file| ZWO
    WOZ -->|to_zwo| ZWO
    FUTURE -.-> ZWO
    ZWO --> PIPE
    PIPE --> LIB
    LIB --> PLAYER
    PLAYER --> FIT
```

Adding a source = one frontend tab component registered in `src/sources.ts`,
plus a backend module that produces ZWO text and calls
`sources::ride_from_zwo`. Provenance columns (`origin`, `origin_ref`) tag
imported rows so the library shows badges and future features (results
push-back, re-sync) know where a workout came from.

## Ride data flow

```mermaid
sequenceDiagram
    participant U as Rider
    participant P as Player runtime
    participant E as Engine
    participant T as Trainer
    participant J as Journal
    participant G as Garmin Connect

    U->>P: Start
    loop every 250 ms
        P->>E: Tick
        E-->>P: effects: SetTarget, LapBoundary
        P->>T: set_target_power
    end
    T-->>P: telemetry: power, cadence
    P->>J: 1 Hz samples + lap events
    U->>P: End ride
    P->>J: replay journal
    P->>P: laps, NP, IF, TSS
    P->>P: encode .FIT
    U->>G: upload .FIT
```

## Cross-platform status

| Layer | macOS (shipped) | Windows (planned) | Shared? |
|---|---|---|---|
| Shell / webview | WKWebView | WebView2 (bootstrapper via NSIS) | Tauri 2 config |
| BLE | CoreBluetooth via btleplug | WinRT via btleplug | drivers + traits unchanged |
| Everything else | — | — | 100 % shared (tp-core, tp-app, frontend) |

Windows-specific work is validation, not architecture: btleplug's WinRT
backend has its own timing personality (the CoreBluetooth race-guards we
carry — scan quiescing, serialized connects — are kept on both platforms),
peripheral IDs are per-machine on both OSes, and installers need
`bundle.active` + signing decisions. The full test suite (all hardware-free)
runs identically on both platforms.
