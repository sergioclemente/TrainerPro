# TrainerPro behavior specification

**Purpose:** Define TrainerPro's current observable behavior and acceptance
contract. **Audience:** Engineers, reviewers, and testers changing the shipped
application.

Product intent is defined in [PRODUCT.md](PRODUCT.md). Technical ownership and
flows are defined in [architecture.md](architecture.md). Code and tests remain
authoritative for internal types, wire payloads, database schema, and constants.

## Application surface

TrainerPro is a dark-theme desktop application with primary navigation for
**Workouts**, **Build**, **Devices**, **Activities**, and **Settings**. Loading a
workout opens its detail view; starting it opens the full player. Ending a ride
opens a summary.

macOS is the validated platform. The shared implementation includes Windows
support, but physical BLE recovery and installer behavior remain manual release
gates. Linux is unsupported.

## Workouts and Next Up

The Workouts screen begins with a horizontally scrollable **Next Up** rail and
keeps the Library below it.

Next Up contains, in order:

1. active, unfulfilled scheduled workouts in local calendar order; then
2. up to three recommendations derived from recent Activity frequency.

A scheduled workout remains visible from its scheduled date through the next
seven calendar days. Older missed schedules remain stored for traceability but
leave Next Up. The future display horizon is intentionally not capped beyond
the schedules available locally.

The backend determines the retention date using the active Intervals.icu
account's IANA time zone, falling back to the machine-local zone when no account
is connected. A schedule disappears after an Activity links to it.

Recommendations use the preceding 180 days of Activity history, rank by
frequency and then recency, and exclude definitions already scheduled in the
projection. They display the definition's training-focus tag, or a generic
structured-ride label when none exists.

Opening either item shows the shared workout detail. Starting a scheduled item
preserves its schedule identity through the session and resulting Activity.

## Library, imports, and authoring

The local Library lists TrainerPro-owned definitions. Provider-scheduled cache
definitions remain executable through Next Up but are not local Library items,
deduplication candidates, or deletable local workouts.

The Library imports ZWO, ERG, and MRC files. Import converts supported content
to canonical TPW, reports parser warnings, and deduplicates equivalent local
definitions. The original file is not managed or deleted by TrainerPro.

Enabled WorkoutPlanner and What's on Zwift sources appear as separate Library
tabs. Their payloads are normalized into TPW before execution. Cached source
data may remain available when a remote source is temporarily unavailable.

Build creates cycling workouts from steady intervals, ramps, free ride, and
repetitions. It validates required fields and displays duration, graph, and
estimated metrics before saving a TrainerPro-owned definition. Detail and Build
views show relative targets together with watts resolved from the current FTP.

## Devices

Devices has independent Trainer and Heart Rate Monitor slots. A slot can scan,
connect, disconnect, and forget a saved device. Discovery results are deduplicated
and ordered by signal strength.

TrainerPro supports FTMS trainers and BLE heart-rate monitors. Saved devices
reconnect in the background. Live state comes from the device status stream;
retaining a device object does not imply connectivity.

A foreground connection cancels a public scan and waits for scan cleanup.
Trainer and HRM connections may proceed concurrently when their setup does not
compete for the same scan. CoreBluetooth disconnect events are authoritative.

The simulated trainer and HRM implement the production connection contracts and
support fault injection. Simulator success does not replace physical Bluetooth
disconnect/reconnect validation.

## Workout execution

Starting a workout creates a session identity and snapshots the canonical
definition. The pure engine advances on ticks and emits effects; the backend
runtime owns clocks, device commands, events, and recording.

The player supports:

- start, pause, explicit resume, skip, and end;
- intensity adjustment from 50% through 150%;
- ERG enable/disable;
- steady and ramp power targets, free ride, cadence targets, and coaching cues;
- interval countdown, elapsed and remaining time, power, cadence, heart rate,
  work, average power, normalized power, intensity factor, and training stress.

Keyboard controls are Space for pause/resume, `S` for skip, and Up/Down for
intensity. Display power uses a three-second rolling average; recording retains
the unsmoothed measurement stream.

Loss of trainer control pauses the ride and starts reconnect attempts. Recovery
reapplies control state and the current target, but never resumes the timer
without the rider's explicit action. HRM loss clears only heart-rate data and
does not pause trainer execution.

## Recording and Activities

A ride writes a crash-tolerant JSONL journal at one-second cadence. Samples are
recorded while riding, not while paused. Pauses, resumes, interval boundaries,
and session metadata are retained so replay can reconstruct elapsed and timer
time correctly.

Ending a ride replays the journal, calculates summaries and interval laps,
encodes a Garmin-compatible FIT file, and inserts one Activity. If FIT encoding
fails, TrainerPro reports the error and preserves the journal for recovery.

Activities lists completed rides with their date, workout, duration, power,
training metrics, and available heart-rate data. Users can reveal or save the
FIT file, open Garmin Connect for manual upload, and delete an Activity. A
configured export directory receives an additional FIT copy.

## Settings and connections

Settings manages athlete FTP and weight, distance recording, FIT export,
optional workout libraries, and provider connections.

Intervals.icu connection uses a personal API key stored in the OS credential
manager. The application stores only non-secret account identity, time zone,
sync health, and provider-scoped schedule state in SQLite. Cached workouts
render before background refresh and remain executable offline. Disconnecting
retires provider schedules while preserving Activity history. The full contract
is in [feature-intervals-icu.md](feature-intervals-icu.md).

WorkoutPlanner and What's on Zwift are opt-in library sources. WorkoutPlanner
configuration supports URL and optional Basic Auth credentials; its behavior is
defined in [feature-workoutplanner.md](feature-workoutplanner.md).

## Failure behavior

- Parse failures identify the unsupported file or workout content.
- Bluetooth permission, unavailable adapter, incompatible trainer, refused
  control, and lost control remain distinguishable user-facing failures.
- Network or provider failures retain last-good cached data and expose sync
  health instead of deleting workouts.
- Invalid credentials point the rider to Settings without echoing secrets.
- Storage or FIT failures preserve the journal whenever recovery remains
  possible.
- Commands return stable error codes and human-readable messages; runtime
  outcomes that do not require caller branching use the shared toast event.

## Acceptance

Automated tests must cover pure parsing, compilation, engine effects, metrics,
FIT output, database reconciliation, source adapters, simulator faults, and a
complete simulated ride. Changes spanning Rust and TypeScript must pass the
workspace tests and frontend production build.

Physical trainer disconnect/reconnect, platform Bluetooth behavior, packaged
application startup, and Garmin FIT import remain manual validation gates when
the affected subsystem changes.
