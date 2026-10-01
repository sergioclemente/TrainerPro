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

Devices has Trainer, Heart Rate Monitor, and Controller slots. A slot can scan,
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

### Handlebar controls

The Controller source defaults to Trainer controls, following the selected
trainer. Wahoo BIKE SHIFT input shares the trainer connection; disabling its
input never disconnects the trainer. Alternatively, select a paired controller;
currently this is the left Zwift Ride controller, which carries both handles
over one bonded link. Selecting the paired controller overrides trainer input
without automatic fallback. Forgetting an active paired controller disables
input until another source is selected. Removing a saved controller while using
trainer controls leaves trainer input active. Disconnecting the paired controller
turns input off but keeps the saved pairing.

Wahoo left steering and Ride A pause/resume; hold Wahoo right steering or Ride Y
to talk. Other handlebar buttons are unassigned. Controller loss cancels a held
utterance without pausing the workout. Input recovery requires a fresh press.
Protocol support targets BIKE SHIFT and Ride firmware 1.2.0; physical hardware and
firmware compatibility remain manual validation gates. Bridged controllers,
separate-side Ride connections, music control, and virtual shifting are excluded.

## Workout execution

Starting a workout creates a session identity and snapshots the canonical
definition. The pure engine advances on ticks and emits effects; the backend
runtime owns clocks, device commands, events, and recording.

The player supports:

- start, pause, explicit resume, skip, and end;
- go to any interval from the graph (right-click), forward or backward, before
  or during the ride; the interval left and any passed over are recorded as
  skipped, and a re-ridden interval records a further result and lap;
- intensity adjustment from 50% through 150%;
- ERG enable/disable;
- steady and ramp power targets, free ride, cadence targets, and coaching cues;
- interval countdown, elapsed and remaining time, power, cadence, heart rate,
  work, average power, normalized power, intensity factor, and training stress.

Keyboard controls are hold Space for push-to-talk, `S` for skip, and Up/Down
for intensity. Space does not start, pause, or resume a workout. Display power uses a three-second rolling average; recording retains
the unsmoothed measurement stream. The player graph draws the ridden power from
the same one-second samples the journal records, and shows each interval's
cadence target against an rpm scale on its right edge. Open intervals (free
ride, no power target) draw as a hatched placeholder block rather than a zone
bar; they may carry a cadence target, and the Player labels them "open".

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

The completion screen offers Save FIT, upload to a connected Activity provider,
and Done. It has no manual browser import or file-reveal action.

Activities lists completed rides with their date, workout, duration, power,
training metrics, and available heart-rate data. Users can reveal the FIT file,
upload it to a connected Garmin account, and delete an Activity. Confirmed Garmin
uploads are remembered per Activity and account. Upload errors never automatically resend the file. A
configured export directory receives an additional FIT copy.

## Settings and connections

Settings manages athlete FTP and weight, distance recording, FIT export,
optional workout libraries, and provider connections. A connection exposes
plan-source and/or Activity-destination actions. Authentication status reflects
local credential availability, not a live connectivity probe; unavailable
credentials on one connection do not hide other connections.

Next Up refreshes connected plan sources independently and keeps cached results
available on failure. Refresh windows use each source's account time zone;
schedule cutoffs and labels use placement time zone, then account time zone,
then the machine's local zone. Activity uploads target a specific connected
account, with upload markers remembered for that Activity/account pair.
The local builder does not publish plans or modify external calendars.

Garmin Connect sign-in supports verification codes and stores session tokens in
the OS credential manager. Expired or revoked sessions require sign-in again.
Uploads are manual and may flow onward to services linked to Garmin. The
unofficial integration and live validation gate are described in
[feature-garmin.md](feature-garmin.md).

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

## Workout voice and timeline

Voice is enabled by default. At startup it checks microphone permission and
requests it if needed, immediately releasing the permission-check stream.
Settings can disable voice. Bundled models run on-device without downloads;
audio and transcripts are not stored or sent.
Holding Space in a focused Player, or a handlebar button in a visible Player,
captures one utterance. Hidden or minimized windows cannot capture. Releasing
submits at most one validated command; silence and pauses within a hold do
nothing. Capture cancels after 15 seconds, on Player exit, Settings disable,
input loss, or release during microphone startup. Pending interpretation is
discarded, and recovery requires a new press.

Start, pause, resume, skip, intensity, and ERG commands
use the same actions as UI controls. End-like voice commands pause; ending the
ride remains manual. Skip also advances the interval while paused without
restarting the trainer.

Settings disable releases the voice runtimes. Voice errors must not prevent
pointer or keyboard control.
Repeated capture or model failures expose Retry and the Settings disable path;
missing bundled models report a source-specific error without blocking the Player.

The workout rail retains device indicators and a session-only timeline. UI and
voice actions share user-aligned command labels; ride events align opposite.
Unmatched attempts remain transient in the composer rather than filling history.
