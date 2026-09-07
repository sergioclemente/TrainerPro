# Agent guidance

Read [`README.md`](README.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md) first.
Use [`docs/SPEC.md`](docs/SPEC.md) for intended behavior,
[`docs/ALTERNATIVES.md`](docs/ALTERNATIVES.md) for settled decisions, and
[`docs/architecture.md`](docs/architecture.md) for system flows. Do not
duplicate those documents here.

## Constraints

- Keep `tp-core` pure: no I/O, async, BLE, or Tauri dependencies.
- Keep hardware behavior behind `Trainer` and `HeartRateMonitor`; the
  simulator must be able to exercise it.
- Extend the abstraction that already owns a behavior. Do not add parallel
  update channels, mirrored connection state, or a common enum when the
  existing device/status abstractions already express it.
- New abstractions need multiple real consumers, an invariant, or a clear
  ownership boundary. A one-call helper or test convenience is not enough.
- Keep trainer and HRM paths separate when their policies differ; similar code
  alone is not a reason to unify them.
- Keep changes scoped. Do not mix unrelated work into an existing PR branch or
  fix unrelated formatting/lint failures.

## What you cannot infer from the repository

- A retained device object (`Some`/`Arc`) proves ownership, not connectivity.
  Read the device's status stream for live state.
- An `async` function is not necessarily slow or blocking. Do not add
  `tokio::join!` or a timeout without evidence that it improves behavior.
- On CoreBluetooth, an open notification stream does not prove the link is
  alive; adapter disconnect events are authoritative.
- Simulator and automated-test success does not prove physical BLE recovery.
  Hardware disconnect/reconnect remains a manual validation gate.
- Documentation may describe intended behavior while code/tests show current
  behavior. Surface meaningful drift and reconcile it; do not silently assume
  either is correct.

## Conventions

- Ongoing device state is event-driven through the existing `DeviceStatus`
  watch stream. Use a transport connectivity probe only at a one-off boundary,
  not for polling.
- Runtime commands that already report user-facing outcomes through Tauri
  events use the existing `toast` event. Add a oneshot/result path only when
  the caller must consume that result to choose its next action.
- A sensor-specific event may clear only that sensor's stale fields. Persist
  derived telemetry in runtime state when another event path must re-emit it.
- Names should expose state, units, and time windows. Avoid boolean-sounding
  names for optional timestamps; prefer shapes such as `ride_started_at` and
  `power_smoothed_3s`.
- Never introduce magic numeric or duration literals. Use a descriptive named
  constant at the narrowest useful scope, or `tp-core::consts` for shared
  product constants.
- Treat Rust serialized payloads and `src/ipc.ts` as one API; update every
  producer and consumer together.

## Verification

- Run focused tests while iterating. For changes spanning Rust and TypeScript,
  finish with `cargo test --workspace` from `src-tauri/` and `npm run build`
  from the repository root.
- Device fault tests should obtain the status receiver before injecting the
  fault and prove that the existing stream observes the transition.
- Check repository-wide formatting before applying it. If the baseline fails
  outside the diff, do not create unrelated churn; report it and validate the
  changed scope.
- For a local macOS `.app`, run
  `npm run tauri build -- --bundles app`. Read `CFBundleExecutable` from
  `Contents/Info.plist` instead of guessing the executable name.
- Before committing, inspect `git diff --check`, the changed-file list, and
  `git status`. Do not include `dist/`, `target/`, credentials, or generated
  application bundles.
