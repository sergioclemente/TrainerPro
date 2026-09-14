# Architecture notes

Companion diagrams to the overview in the [README](../README.md). The full
build spec is [`SPEC.md`](SPEC.md); design rationale and rejected alternatives
are in [`ALTERNATIVES.md`](ALTERNATIVES.md).

> **Scope:** this document describes current implemented flows. The accepted
> connected-workout target architecture is in
> [`workout-platform.md`](workout-platform.md), and the product behavior it
> serves is in [`PRODUCT.md`](PRODUCT.md).

## Workout source normalization

Workout sources normalize into **TrainerPro Workout (TPW)** before persistence.
Local ZWO/ERG/MRC files and WorkoutPlanner's ZWO response are boundary payloads;
What's on Zwift already builds a semantic model. None becomes workout identity.

```mermaid
flowchart TD
    subgraph SRCS["Workout sources - tabs from frontend/sources.ts registry"]
        LOCAL["My Library<br/>local .zwo / .erg / .mrc files"]
        WP["WorkoutPlanner<br/>self-hosted server<br/>Basic Auth, /workout_file ZWO"]
        WOZ["Zwift<br/>whatsonzwift.com fetch<br/>textbars to model"]
        FUTURE["Future source...<br/>one tab component +<br/>one backend module"]
    end

    ADAPTERS["Boundary adapters<br/>ZWO / ERG / MRC / source model"]
    TPW["TPW WorkoutDefinition<br/>validate + canonical JSON"]
    DB[("SQLite<br/>workout_definitions")]
    COMPILE["Compile to<br/>ExecutableWorkout"]
    PLAYER["Player runtime"]
    FIT["FIT file to Garmin Connect"]

    LOCAL -->|file picker / drag-drop| ADAPTERS
    WP -->|GET /workout_file| ADAPTERS
    WOZ --> ADAPTERS
    FUTURE -.-> TPW
    ADAPTERS --> TPW
    TPW --> DB
    DB --> COMPILE
    COMPILE --> PLAYER
    PLAYER --> FIT
```

Adding a source = one frontend tab component registered in `frontend/sources.ts`,
plus a backend module that produces `WorkoutDefinition` or maps its supported
payload through a boundary adapter. Provenance columns (`origin`, `origin_ref`)
remain the current source badges; provider links and sync state replace them in
the connected-provider phase.

## Device connection lifecycle

`Trainer` and `HeartRateMonitor` are stable backend owners. A BLE or simulated
driver implements the corresponding one-link connection trait. Connection
replacement and retry state stay private to the owner, so the player and UI
subscribe once and never receive an optional connection object.

`tp-ble::DeviceManager` owns adapter-level scan and connection coordination.
Public and targeted scans are serialized. Startup uses one scan for all
configured physical roles and begins each device's setup as soon as it is
resolved. Discovery of the other role may continue, and resolved GATT setups
may proceed concurrently. Established links operate concurrently. A foreground
connection cancels public discovery, while an automatic retry waits for it. A
saved HRM lookup is preemptible by trainer recovery. The application hub chooses
when to scan or connect without implementing Bluetooth quiescence itself.

```mermaid
sequenceDiagram
    participant UI
    participant O as Trainer / HeartRateMonitor owner
    participant F as Connection factory
    participant C as Replaceable connection
    participant P as Player / event bridge

    P->>O: subscribe_state + subscribe_measurements
    UI->>O: connect(platform_id)
    O-->>P: DeviceState: Connecting
    O->>F: connect(platform_id)
    F-->>O: Box<...Connection>
    O-->>P: DeviceState: Connected
    C-->>O: ConnectionStatus: Disconnected
    O-->>P: DeviceState: Reconnecting (new generation)
    O->>C: disconnect and stop link tasks
    O->>F: retry connect(platform_id)
    F-->>O: replacement connection
    O-->>P: DeviceState: Connected
    UI->>O: disconnect
    O->>O: cancel retry / invalidate generation
    O-->>P: DeviceState: Disconnected
```

Trainer and heart-rate owners remain separate even though this lifecycle is
similar: trainer loss pauses a ride, while heart-rate loss only clears the
heart-rate measurement.

## Ride data flow

```mermaid
sequenceDiagram
    participant U as Rider
    participant P as Player runtime
    participant E as Engine
    participant T as Trainer owner
    participant J as Journal
    participant G as Garmin Connect

    U->>P: Start
    loop every 250 ms
        P->>E: Tick
        E-->>P: effects: SetTarget, LapBoundary
        P->>T: set_target_power
    end
    T-->>P: measurements: power, cadence
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
| Everything else | — | — | 100 % shared (tp-core, backend, frontend) |

Windows-specific work is validation, not architecture: btleplug's WinRT
backend has its own timing personality (the CoreBluetooth race-guards we
carry — scan quiescing, serialized connects — are kept on both platforms),
peripheral IDs are per-machine on both OSes, and installers need
`bundle.active` + signing decisions. The full test suite (all hardware-free)
runs identically on both platforms.
