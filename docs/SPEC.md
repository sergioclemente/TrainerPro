# TrainerPro — Implementation Spec (v1)

A macOS-first desktop indoor-cycling workout player. Load a structured workout
file (ZWO / ERG / MRC), control a Wahoo smart trainer over BLE FTMS in ERG
mode, record the ride, and export a Garmin-compatible .FIT activity file.

Design rationale and rejected alternatives live in `ALTERNATIVES.md`. This
document is the build plan: when it and an implementer disagree, fix this
document first.

---

## 0. Locked decisions

| Decision | Choice |
|---|---|
| Stack | Tauri 2, Rust core, React + TypeScript UI |
| Trainer protocol | **FTMS only** (target hardware: KICKR Core / v5+ / Move / Bike). Wahoo legacy driver: backlog, not v1 |
| OS | **macOS first** (M1–M4). Windows port in M5. Linux: unsupported |
| Sensors | Trainer (power/cadence/speed) + BLE HR strap. No power match, no ANT+ |
| Player UI | Dashboard only |
| Garmin export | FIT file + manual upload today; direct sync awaits Garmin Developer Program access — see [`garmin-access.md`](garmin-access.md) |
| Distance in FIT | **Off by default** (setting exists; virtual flat-road model when on) |
| FreeRide segments | Switch trainer to simulation mode, grade 0 %; record only, no target |
| Recording | JSONL journal during ride → FIT encoded at ride end |
| Storage | Files are truth; SQLite is index/cache |

---

## 1. Repository layout

```
TrainerPro/
├── Cargo.toml                  # Rust workspace
├── backend/                    # Tauri lifecycle, IPC, I/O, and runtime
│   ├── src/
│   │   ├── main.rs
│   │   ├── app_error.rs
│   │   ├── app_state.rs
│   │   ├── commands/
│   │   │   ├── mod.rs
│   │   │   ├── device.rs
│   │   │   ├── player.rs
│   │   │   ├── ride_history.rs
│   │   │   ├── settings.rs
│   │   │   └── workout.rs
│   │   ├── database.rs
│   │   ├── device_hub.rs
│   │   ├── device_owner.rs
│   │   ├── heart_rate_monitor.rs
│   │   ├── player_runtime.rs
│   │   ├── trainer.rs
│   │   ├── whatsonzwift_source.rs
│   │   ├── workout_planner_source.rs
│   │   ├── workout_source_cache.rs
│   │   └── workout_sources.rs
│   └── tauri.conf.json
├── crates/
│   ├── tp-core/                # PURE: no I/O, no BLE, no tauri deps
│   │   ├── src/model.rs        # Workout, Segment, PowerTarget…
│   │   ├── src/parse/zwo.rs
│   │   ├── src/parse/ergmrc.rs
│   │   ├── src/engine.rs       # player state machine
│   │   ├── src/metrics.rs      # NP/IF/TSS, smoothing, zone calc
│   │   ├── src/journal.rs      # JSONL read/write
│   │   └── src/fit/            # encoder: profile.rs (generated), encode.rs, crc.rs
│   └── tp-ble/                 # btleplug drivers
│       ├── src/traits.rs       # per-link device contracts
│       ├── src/codec.rs        # pure FTMS / HR packet codecs
│       ├── src/device_manager.rs
│       ├── src/ftms_trainer_connection.rs
│       ├── src/ble_heart_rate_connection.rs
│       ├── src/connection_tasks.rs
│       ├── src/sim_trainer.rs
│       ├── src/sim_hrm.rs
│       └── tests/              # public simulator contract tests
├── frontend/                   # React app (Vite + TypeScript)
│   ├── screens/  (Library, Devices, Player, Summary, History, Settings)
│   ├── components/ (WorkoutGraph, MetricTile, IntervalStrip, DeviceCard…)
│   ├── ipc.ts                  # typed command wrappers + event subscriptions
│   └── state.ts                # zustand store fed by events
├── tools/                      # maintained internal command-line utilities
└── testdata/
    ├── workouts/               # parser corpus: real .zwo/.erg/.mrc files
    └── fit-golden/             # expected FitCSVTool output snapshots
```

Rule that keeps this honest: **`tp-core` has zero async and zero I/O deps** —
every function is callable from a plain unit test. `tp-ble` depends on
`tp-core`; the backend depends on both.

---

## 2. Core data model (`tp-core::model`)

```rust
pub enum PowerTarget { PercentFtp(f64) /* 0.05..=3.0 */, Watts(u16) }

pub enum Segment {
    Steady   { duration_s: u32, power: PowerTarget, cadence_rpm: Option<u16> },
    Ramp     { duration_s: u32, start: PowerTarget, end: PowerTarget, cadence_rpm: Option<u16> },
    FreeRide { duration_s: u32 },
}

pub struct TextEvent { pub offset_s: u32, pub message: String, pub duration_s: u32 /* default 10 */ }

pub enum SourceFormat { Zwo, Erg, Mrc }

pub struct Workout {
    pub name: String,
    pub description: String,
    pub source_format: SourceFormat,
    pub segments: Vec<Segment>,       // flat; repeats pre-expanded
    pub text_events: Vec<TextEvent>,  // offsets relative to workout start
}

impl Workout {
    pub fn duration_s(&self) -> u32;
    /// Resolve target at absolute offset t (None inside FreeRide).
    /// intensity is the live bias, 0.50..=1.50, applied to PercentFtp only.
    pub fn target_at(&self, t_s: u32, ftp: u16, intensity: f64) -> Option<u16>;
    pub fn estimate_if_tss(&self, ftp: u16) -> (f64, f64); // for library display
}
```

`target_at` semantics: locate segment containing `t`; Steady → resolve power;
Ramp → linear interpolation by elapsed fraction, resolved per-endpoint then
interpolated in watts; round half-up to whole watts; clamp 0..=2000. Mixed
`Watts` targets ignore intensity bias (bias applies to `PercentFtp` only).

---

## 3. Workout parsers (`tp-core::parse`)

General contract: `parse_zwo(&str) -> Result<Workout, ParseError>` /
`parse_ergmrc(&str, ext_hint) -> Result<Workout, ParseError>`. A file either
parses into ≥1 segment or fails with a message naming the line/element.
Unknown elements/attributes are **collected as warnings, never errors**;
warnings surface once in the import UI.

### 3.1 ZWO (Zwift XML)

Root: `<workout_file>`; metadata from `<name>`, `<description>`, `<author>`.
Workout body: children of `<workout>`, in document order:

| Element | Attributes used | Maps to |
|---|---|---|
| `SteadyState` | `Duration` (s), `Power` (fraction of FTP), `Cadence` | Steady |
| `IntervalsT` | `Repeat`, `OnDuration`, `OffDuration`, `OnPower`, `OffPower`, `Cadence`, `CadenceResting` | Expanded: Repeat × (Steady(on), Steady(off)) |
| `Warmup` | `Duration`, `PowerLow`, `PowerHigh`, `Cadence` | Ramp PowerLow → PowerHigh |
| `Cooldown` | `Duration`, `PowerLow`, `PowerHigh`, `Cadence` | Ramp PowerHigh → PowerLow |
| `Ramp` | same as Warmup | Ramp PowerLow → PowerHigh |
| `FreeRide` | `Duration` | FreeRide |
| `MaxEffort`, `SolidState`, others | — | Warn; map to Steady at 100 % FTP if a Duration exists, else skip |

- All `Power*` values are FTP fractions (`0.75` = 75 % FTP) → `PercentFtp`.
- ⚠️ **Cooldown direction is a known community ambiguity** (attribute names
  don't state direction). Spec rule: Cooldown ramps high→low. M1 includes a
  corpus check: if real Zwift-exported cooldowns contradict this, flip the
  rule and update this table — do not special-case per file.
- `<textevent timeoffset="…" message="…" duration="…"/>` children: offsets are
  relative to the *parent segment's start*; convert to workout-absolute at
  parse time. Missing `duration` → 10 s.
- Reject: zero/negative durations, power fraction outside 0.05–3.0 (clamp
  with warning), empty `<workout>`.

### 3.2 ERG / MRC (text)

Structure (CompuTrainer heritage; tab- or space-separated):

```
[COURSE HEADER]
VERSION = 2
UNITS = ENGLISH
DESCRIPTION = Sweet Spot 3x12
FILE NAME = ss3x12.mrc
MINUTES PERCENT          ← column spec: PERCENT ⇒ %FTP, WATTS ⇒ absolute
[END COURSE HEADER]
[COURSE DATA]
0.00   45
10.00  45
10.00  88
22.00  88
[END COURSE DATA]
[COURSE TEXT]            ← optional: offset_s  message  duration_s
120  Find a rhythm  10
[END COURSE TEXT]
```

- Unit source of truth: the `MINUTES WATTS|PERCENT` column line. Fallback if
  absent: extension (`.erg` ⇒ WATTS, `.mrc` ⇒ PERCENT). Warn on mismatch.
- Consecutive data rows (t₁,p₁) → (t₂,p₂): if t₂ > t₁, emit segment of
  `duration = (t₂−t₁)·60`; p₁ = p₂ → Steady, else Ramp p₁→p₂. Rows with
  t₂ = t₁ (vertical step) set the next segment's start power, emit nothing.
- Times are decimal **minutes**; tolerate `,` decimal separator and CRLF.
- Reject: non-monotonic time, < 2 data rows, unparseable row (with line number).

### 3.3 Import flow

`import_workout(path)`: read file → parse → copy original into
`<appdata>/workouts/<uuid>.<ext>` → compute duration/IF/TSS estimate + graph
polyline (array of `[t_s, pct_ftp]` breakpoints) → insert SQLite row → return
summary + warnings. Duplicate detection: SHA-256 of file bytes; re-import of
identical bytes returns the existing entry.

---

## 4. BLE layer (`tp-ble`)

### 4.1 Connection traits (everything above this file is hardware-agnostic)

```rust
#[async_trait]
pub trait TrainerConnection: Send + Sync {
    async fn disconnect(&mut self) -> Result<(), BleError>;
    async fn probe_connection(&self) -> Result<bool, BleError>; // one-off boundary probe
    async fn set_target_power(&self, watts: u16) -> Result<(), BleError>;
    async fn set_flat_road_simulation(&self) -> Result<(), BleError>; // FreeRide
    async fn start_or_resume_training(&self) -> Result<(), BleError>;
    async fn pause_training(&self) -> Result<(), BleError>;
    async fn reset_trainer(&self) -> Result<(), BleError>;
    fn subscribe_measurements(&self) -> broadcast::Receiver<TrainerMeasurement>;
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus>;
}
pub struct TrainerMeasurement { pub power_w: Option<u16>, pub cadence_rpm: Option<f32>,
                                pub speed_kmh: Option<f32> }
pub enum ConnectionStatus { Disconnected, Connecting, Connected }
```

`HeartRateConnection` provides the same disconnect, measurement, and status
shape with `HeartRateMeasurement { bpm: u16 }`. These traits describe one
replaceable link; they do not own selection or reconnection policy.

### 4.2 FTMS driver (`ftms_trainer_connection.rs`)

Service `0x1826`. Characteristics used:

| Char | UUID | Direction |
|---|---|---|
| Fitness Machine Feature | `0x2ACC` | read once |
| Indoor Bike Data | `0x2AD2` | notify |
| Fitness Machine Control Point (CP) | `0x2AD9` | write + indicate |
| Fitness Machine Status | `0x2ADA` | notify |

**Connect sequence** (all steps must succeed or the connect fails with a
specific error):
1. Discover services/chars; require `0x1826` with `0x2AD2` + `0x2AD9`.
2. Read `0x2ACC`; require Target-Power-Setting supported bit; store max power
   if the Supported Power Range char (`0x2AD8`) exists.
3. Subscribe: CP indications, Indoor Bike Data, Machine Status.
4. Write CP `[0x00]` (Request Control) → await indication `[0x80, 0x00, 0x01]`.
5. Report `Connected`.

**Control Point ops used** (1-byte opcode + LE params; response indication is
`[0x80, req_op, result]`, result `0x01` = success):

| Op | Bytes | When |
|---|---|---|
| Request Control | `00` | connect / after reconnect |
| Reset | `01` | end of ride |
| Set Target Power | `05` + sint16 LE watts | ERG target changes + keep-alive |
| Start/Resume | `07` | ride start / resume |
| Stop/Pause | `08 02` | pause |
| Set Indoor Bike Simulation | `11` + wind sint16(0.001 m/s)=0, grade sint16(0.01 %)=0, crr uint8(0.0001)=40, cw uint8(0.01)=51 | entering FreeRide |

Serialize CP writes: one in flight, 2 s response timeout, one retry, then
surface `ControlLost` (player auto-pauses). Non-success result codes map to
typed errors (`0x02` NotSupported, `0x03` InvalidParam, `0x05` NotPermitted).

**Indoor Bike Data parse** (`0x2AD2`): first 2 bytes = flags (uint16 LE), then
fields present in this order when their bit is set:

| Bit | Field | Type/scale |
|---|---|---|
| 0 = 0 | Instantaneous Speed | uint16, 0.01 km/h (present when bit0 is 0) |
| 1 | Average Speed | uint16 (skip) |
| 2 | Instantaneous Cadence | uint16, 0.5 rpm |
| 3 | Average Cadence | uint16 (skip) |
| 4 | Total Distance | uint24 m (skip) |
| 5 | Resistance Level | sint16 (skip) |
| 6 | Instantaneous Power | sint16 W |
| 7+ | Avg Power / Energy / HR / MET / times | parse-and-skip per spec sizes |

Parser must walk flags in order and skip unset/unused fields by size — never
assume fixed offsets. Malformed packet (short buffer): drop packet, count it,
`warn!`; 10 consecutive malformed → treat as disconnect.

### 4.3 Heart rate driver (`ble_heart_rate_connection.rs`)

Service `0x180D`, char `0x2A37` notify. Flags byte bit 0: 0 ⇒ uint8 bpm at
offset 1; 1 ⇒ uint16 LE at offset 1. Ignore RR/energy fields. HR of 0 is
reported as `None` (sensor warming up).

### 4.4 Device ownership and discovery

- **Scan**: btleplug central scan, filter to advertised services `0x1826`
  (role Trainer) / `0x180D` (role HRM); emit `{platform_id, name, rssi, role}`
  deduped, sorted by RSSI. Stop scan on connect or explicit stop.
- **Adapter coordination**: `tp-ble::DeviceManager` owns lazy adapter
  initialization, public-scan cancellation, and exclusive scan/connection
  setup. A connection request cancels and awaits an active public scan;
  trainer and HRM setup are serialized, but established links and their
  notification streams operate concurrently.
- **Owners**: the backend keeps one long-lived `Trainer` and one long-lived
  `HeartRateMonitor`. Each owns at most one selected device and privately
  replaces an `Option<Box<dyn ...Connection>>`. Consumers retain the owner
  and its stable `DeviceState`/measurement subscriptions across reconnects;
  `DeviceState` carries a `DeviceStatus` that adds `Reconnecting { attempt }`
  to the link-level states. They do
  not swap optional connection handles. V1 deliberately has one HRM owner;
  multi-HRM support can later compose several owners without changing the
  per-connection contract.
- **Saved devices**: table `devices(role PRIMARY KEY, platform_id, name)` —
  exactly one trainer + one HRM. On app start, make two silent attempts to
  reconnect each saved device; the Devices screen remains the manual fallback.
- **Reconnect on drop**: attempts at +0 s, 1, 2, 5, 10, then every 15 s
  forever until user cancels. On success: re-run connect sequence incl.
  Request Control and emit `Connected`; Resume re-sends the current target.
  Player behavior on drop is in §5.4.
- macOS note: btleplug returns opaque peripheral UUIDs that are stable
  per-machine — store those, never MAC addresses.

### 4.5 Simulator (`sim_trainer.rs`, `sim_hrm.rs`)

`SimTrainer` implements `TrainerConnection`; drives all dev/CI work:
- Power response: 4 Hz ticks, `p += (target − p)·(1 − e^(−dt/τ))`, τ = 1.5 s,
  plus N(0, 5 W) noise, floor 0.
- Cadence: 88 ± 3 rpm while target > 0; 0 when target = 0 for > 5 s.
- Fault injection API: `drop_connection()`, `delay_cp_responses(ms)`,
  `emit_malformed_packet()`, `refuse_control()`.
`SimHrm`: 60 bpm + 1.1 × (power/FTP) × 110, first-order lag τ = 25 s, noise.
Selected via env var `TP_SIM=1` (dev) and directly in tests.

---

## 5. Workout engine (`tp-core::engine`) + backend player runtime

### 5.1 Engine (pure)

```rust
pub enum Phase { Idle, Ready, Riding, Paused, Finished }
pub struct Engine { workout: Workout, ftp: u16, intensity: f64, phase: Phase,
                    active_ms: u64, seg_idx: usize, seg_elapsed_ms: u64 }

pub enum Input  { Start, Pause, Resume, SkipSegment, SetIntensity(f64), Tick { dt_ms: u64 }, End }
pub enum Effect { SetTarget(u16), EnterFreeRide, TrainerStart, TrainerStop, TrainerReset,
                  LapBoundary { seg_idx: usize }, ShowText(TextEvent), WorkoutComplete }

impl Engine { pub fn handle(&mut self, input: Input) -> Vec<Effect>; }
```

Rules:
- `Tick` advances only in `Riding`. Segment rollover emits `LapBoundary` +
  the new segment's first target (or `EnterFreeRide`).
- Ramp targets recompute on every Tick; a `SetTarget` effect is emitted only
  when the rounded watt value changed (dedup lives here, not in BLE).
- `SkipSegment` jumps to next boundary (emits `LapBoundary`); skip on last
  segment = `End`.
- Two clocks, and they diverge: `active_ms` is the **position** in the workout
  (skips jump it forward, drives the graph cursor and "remaining"), while
  `ridden_ms` is time **actually pedalled** — advanced only by `Tick`, so
  neither a pause nor a skipped span credits it. The player's big clock shows
  `ridden_ms`; `completed_pct` uses `active_ms`.
- `SetIntensity` clamps to 0.50–1.50 and emits a fresh `SetTarget`.
- Text events fire when `active_ms` crosses `offset_s` (paused time excluded).
- Deterministic: same inputs ⇒ same effects. No clocks inside — the runtime
  owns time. Unit-test the whole workout by feeding Ticks.

### 5.2 Player runtime (async, in the backend)

- Tick loop: 250 ms interval → `Engine::handle(Tick)` → execute effects
  against the stable `Trainer` owner.
- **Keep-alive**: independent 10 s timer re-sends last target while Riding
  (some firmware drops ERG after CP silence).
- **Recorder sampling**: 1 Hz wall-clock tick reads the latest measurement snapshot
  (see §6). Trainer notifications update the snapshot at native rate.
- UI player-measurement event: forwarded per BLE notification, throttled to max 4 Hz.
- **Live totals** (avg power, NP, TSS, EF, kcal on `player_state`) are computed
  from the same 1 Hz series and the same `metrics` functions as the post-ride
  totals in §6 — the in-ride number and the summary number cannot disagree.
  TSS uses moving time (`ridden_ms`); kcal is the kJ figure (cycling
  convention); EF needs an HRM and is `null` without one.

### 5.3 Controls surface (exact v1 set)

Start · Pause · Resume · Skip interval · Intensity ±1 % (buttons + `↑`/`↓`
keys, range 50–150 %) · End ride (confirm dialog if < 100 % complete).
No auto-pause in v1 (post-v1 flag). Extend-cooldown: cut from v1.

### 5.4 Disconnect mid-ride

Trainer drop while Riding: engine gets `Pause` (auto), banner "Trainer
reconnecting (attempt n)…" with Cancel. On reconnect: control re-acquired,
target re-sent, banner offers Resume (no auto-resume — the rider may have
gotten off). HRM drop: non-blocking toast; ride continues, HR samples `None`.

---

## 6. Recorder & journal (`tp-core::journal`)

Journal path: `<appdata>/rides/<ride_uuid>.jsonl`, created at Start, fsync'd
per line. Line types (one JSON object per line):

```jsonl
{"h":{"ride_id":"…","started_unix_ms":…,"workout_name":"…","ftp":250,"weight_kg":72.0,
      "trainer":"KICKR CORE 1234","hrm":"TICKR 5678","app_ver":"0.1.0"}}
{"s":{"t":1234567,"p":215,"c":92,"hr":148,"tgt":220}}      // 1 Hz; t = ms since start; absent key = no data
{"e":{"t":…,"k":"start"|"pause"|"resume"|"lap"|"freeride_enter"|"end","seg":4}}
```

Sample values: `p` watts (trainer instantaneous), `c` rpm rounded, `hr` bpm,
`tgt` current target (absent in FreeRide). Samples continue during Paused?
**No** — recording pauses with the timer; pause/resume events bracket the gap.

Ride end: runtime replays the journal → computes laps (between `lap`/boundary
events), session totals, NP (30 s rolling avg → mean of 4th powers → 4th
root), IF = NP/FTP, TSS = duration_s × NP × IF / (FTP × 3600) × 100, kJ =
Σpower/1000 → encodes FIT (§7) → inserts `rides` row. Crash recovery: on app
start, any journal without a matching ride row gets the same replay path
("Recovered ride" toast).

---

## 7. FIT encoder (`tp-core::fit`)

### 7.1 Container format

- Header (14 bytes): size=14, protocol `0x20`, profile version (LE u16),
  data size (LE u32), literal `.FIT`, header CRC (LE u16).
- Body: definition records + data records, little-endian throughout.
- Trailer: CRC-16 (FIT nibble-table algorithm, per SDK §"CRC") over
  header+body.
- Timestamps: `fit_ts = unix_s − 631_065_600` (FIT epoch 1989-12-31T00:00Z).

One local message type per global message (7 defs total, defined on first
use). Field values use FIT scales/offsets; invalid/absent = base-type invalid
value (e.g. `0xFF` for uint8).

### 7.2 Message sequence (write order)

| # | Message (global msg no.) | Fields (field no. — orientation, regenerate from SDK Profile) |
|---|---|---|
| 1 | `file_id` (0) | type(0)=4 activity · manufacturer(1)=255 development · product(2)=1 · serial(3)=random-per-install · time_created(4) |
| 2 | `device_info` (23) ×1–3 | device_index(0) · manufacturer(2) · product_name(27)="TrainerPro"/trainer name/HRM name · software_version(5) |
| 3 | `event` (21) | timestamp(253) · event(0)=0 timer · event_type(1)=0 start |
| 4 | `record` (20) × N (1 Hz) | timestamp(253) · heart_rate(3) u8 · cadence(4) u8 · power(7) u16 · [speed(6) 1000·m/s + distance(5) 100·m only if distance setting on] |
| — | `event` stop_all(4)/start pairs around each pause, interleaved chronologically | |
| 5 | `lap` (19) × M | message_index(254) · start_time(2) · timestamp(253)=end · total_elapsed_time(7, s×1000) · total_timer_time(8) · avg/max power(19/20) · avg/max HR(15/16) · avg cadence(17) · total_calories(11) |
| 6 | `session` (18) | start_time(2) · timestamp(253) · sport(5)=2 cycling · sub_sport(6)=6 indoor_cycling · total_elapsed/timer_time(7/8) · avg/max power(20/21) · avg/max HR(16/17) · avg cadence(18) · total_calories(11) · num_laps(26) · normalized_power(34) · training_stress_score(35, ×10) · intensity_factor(36, ×1000) · threshold_power(101)=FTP · first_lap_index(25)=0 |
| 7 | `activity` (34) | timestamp(253) · total_timer_time(0) · num_sessions(1)=1 · type(2)=0 manual · event(3)=26 activity · event_type(4)=1 stop · local_timestamp(5) |

Field numbers/scales in this table are for orientation; **`fit/profile.rs` is
generated from the FIT SDK Profile spreadsheet** (build-time script or checked
in generated file), and that generated code is authoritative. Lap boundaries:
one lap per engine `LapBoundary` + final partial lap; elapsed vs timer time
differ by paused duration.

### 7.3 Acceptance (CI-enforced, M4)

- `FitCSVTool.jar` (FIT SDK) decodes with **zero errors/warnings**; decoded
  totals equal recorder totals exactly (snapshot test in `testdata/fit-golden/`).
- Manual gate: upload to Garmin Connect → type "Indoor Cycling", duration,
  laps, power+HR charts, and Training Load all present. Also manually import
  the FIT into Strava and intervals.icu once per release.

### 7.4 Export UX

Ride end → Summary screen: workout graph with actual power overlay, per-lap
table, totals. Buttons: **Save .FIT…** (dialog, default
`TrainerPro_<workout>_<yyyy-mm-dd>.fit`) · **Reveal in Finder** · **Open
Garmin Connect** (`https://connect.garmin.com/modern/import-data` in default
browser). FIT is also always auto-saved at `<appdata>/rides/<ride_uuid>.fit`.

---

## 8. Persistence (backend)

App data dir: `~/Library/Application Support/com.trainerpro.desktop/` (Tauri
`app_data_dir`); subdirs `workouts/`, `rides/`; DB `trainerpro.sqlite3`. The
development QA flavor uses `com.trainerpro.desktop.qa` so its data remains
isolated from an installed production app.

```sql
CREATE TABLE workouts (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
  source_format TEXT NOT NULL, file_path TEXT NOT NULL, sha256 TEXT NOT NULL UNIQUE,
  duration_s INTEGER NOT NULL, est_if REAL, est_tss REAL,
  graph_json TEXT NOT NULL, imported_at INTEGER NOT NULL);

CREATE TABLE rides (
  id TEXT PRIMARY KEY, workout_id TEXT REFERENCES workouts(id) ON DELETE SET NULL,
  workout_name TEXT NOT NULL, started_at INTEGER NOT NULL,
  elapsed_s INTEGER NOT NULL, timer_s INTEGER NOT NULL,
  avg_power INTEGER, max_power INTEGER, np INTEGER, if_ REAL, tss REAL,
  avg_hr INTEGER, max_hr INTEGER, avg_cadence INTEGER, kj INTEGER,
  ftp_used INTEGER NOT NULL, intensity_final REAL NOT NULL,
  fit_path TEXT NOT NULL, journal_path TEXT NOT NULL, completed_pct REAL NOT NULL,
  icu_activity_id TEXT);  -- reserved by v5 for the planned intervals.icu integration

CREATE TABLE devices (role TEXT PRIMARY KEY CHECK(role IN ('trainer','hrm')),
  platform_id TEXT NOT NULL, name TEXT NOT NULL, last_connected_at INTEGER);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);  -- JSON values
```

Settings keys & defaults: `profile` `{"ftp":200,"weight_kg":75.0,"name":""}` ·
`record_distance` `false` · `intensity_default` `1.0` · `export_dir` `null` ·
`sources` (the workout-library provider registry below).

**Workout-library providers** (`sources` key): a plugin registry keyed by
provider id, each `{"enabled":bool,"values":{field:str}}` (`SourceConfig`).
Built-in defaults: both `woz` (Zwift) and `planner` are disabled until the user
opts in; their value bags start empty. Fields are declared per-provider by the
frontend descriptor (`frontend/sources.ts`) and rendered by one generic config
form on the **Libraries** screen; adding a provider is a descriptor entry +
fetch code, no schema change.
Enabled gates both the Workouts tab and any fetch. The old typed `planner`
settings key migrates into `sources.planner` on first load (creds preserved).
This registry is the reuse surface for trainer-coach's own load sources.
Migrations: `user_version` pragma + numbered migration list from day one.

---

## 9. Tauri IPC surface

Commands (Rust `#[tauri::command]`; TS wrappers in `frontend/ipc.ts`; all return
`Result<T, AppError>` where `AppError = { code: string, message: string }`):

```
workouts:  import_workout(path) -> {summary, warnings[]} · create_workout(draft)
           list_workouts() -> Summary[] · get_workout_detail(id) · delete_workout(id)
devices:   start_scan(role) · connect_device(role, platform_id, name?)
           disconnect_device(role) · forget_device(role) · get_device_state() -> DeviceSlot[]
player:    load_workout(id) -> PlayerState · start_ride() · pause_ride() · resume_ride()
           skip_segment() · set_intensity(pct) · set_erg(enabled)
           end_ride() -> RideSummary · clear_ride() · get_player_state()
history:   list_rides() -> RideRow[] · delete_ride(id)
           save_fit_as(id, dest_path) · reveal_fit(id) · open_garmin_import()
sources:   source_test(id, values) -> {ok, detail}   (provider connection test;
           dispatches by id — only testable providers, e.g. planner)
           planner_cached() · planner_list() · planner_preview(wid) · planner_ride(wid)
           planner_open_editor(wid) · woz_collections(force) · woz_workouts(collection, force)
           woz_ride(collection, idx) · woz_open_page(collection)
profile:   get_settings() -> Settings · update_settings(settings)
```

**Planned intervals.icu upload:** this remains an intended post-ride export
sink, not a workout source, but is not implemented in the current command,
settings, or UI surfaces. The reserved `rides.icu_activity_id` column remains
unused. Once work resumes, completed FITs should upload best-effort while the
local FIT/journal remain authoritative; a failed upload should expose a manual
retry rather than compromise ride finalization. The proposed commands are
`icu_test(cfg)` and `icu_upload_ride(ride_id)`, with a disabled-by-default
credential setting. Confirm authentication and server-side `external_id`
deduplication against the live API before relying on retry idempotency. See
[`garmin-access.md`](garmin-access.md) for current integration status.

Events (Rust → UI, `tauri::Emitter`):

| Event | Payload | Rate |
|---|---|---|
| `player_measurement` | `{power_w, cadence_rpm, heart_rate_bpm, power_smoothed_3s_w}` | ≤ 4 Hz |
| `player_state` | `{phase, seg_idx, seg_remaining_s, elapsed_s, ride_s, intensity, lap_avg_power, np, tss, ef, kcal}` | 1 Hz + on transitions |
| `device_status` | `{role, status, attempt?, name?}` | on change |
| `device_measurement` | `{role, power_w?, cadence_rpm?, heart_rate_bpm?}` | trainer 1 Hz / HRM notifications |
| `scan_result` | `{role, platform_id, name, rssi}` | as found |
| `text_event` | `{message, duration_s}` | on fire |
| `ride_finished` | `RideSummary` | once |
| `toast` | `{level, message}` | as needed |

UI state = zustand store hydrated by `get_*` commands, updated by events.
No polling from the UI.

---

## 10. UI screens (v1 exact scope)

Navigation: left rail — Library · Devices · History · Settings; Player takes
over full window when a ride is loaded.

1. **Library**: workout cards (name, duration, est TSS/IF, graph thumbnail
   from `graph_json`), import button + drag-drop target, workout builder,
   delete via context menu. Builder target fields and interval descriptions
   show both % FTP and the resolved watts for the current profile FTP. Click →
   Player in `Ready` (or device-connect prompt if no trainer).
2. **Devices**: two slots (Trainer / HRM): saved device card with status dot,
   or Scan flow (list by RSSI, click to pair). Forget button.
3. **Player** (§6.3 of the product layout): top ⅓ workout graph (zone-colored
   bars, progress cursor, next-interval label, text-event overlay); middle:
   three tiles — power (3 s smoothed, huge) with the live watt target and an
   intensity badge when ≠ 100 % underneath + ±5 % over/under coloring,
   cadence, HR; workout interval descriptions show both % FTP and resolved
   watts; bottom strip: interval countdown (largest number on screen),
   interval avg power, elapsed/remaining, kJ. Controls row: pause/resume, skip,
   ±intensity, end. Keyboard: space = pause/resume, `s` = skip, `↑/↓` =
   intensity.
4. **Summary** (post-ride): §7.4.
5. **History**: table of rides (date, workout, duration, avg P, NP, TSS,
   avg HR) → row click = Summary view for that ride (re-read from journal).
6. **Settings**: FTP, weight, record-distance toggle, app version.

Styling: dark theme only in v1. Readable at 2 m: metric tiles ≥ 96 pt numerals.

---

## 11. Error taxonomy (surfaced, not invented ad hoc)

| code | Trigger | UX |
|---|---|---|
| `parse_failed` | unparseable workout file | import dialog with line/element detail |
| `bt_unavailable` | Bluetooth off / permission denied | full-screen prompt on Devices (macOS: Info.plist `NSBluetoothAlwaysUsageDescription` required) |
| `trainer_incompatible` | no FTMS service / no target-power bit | "This trainer doesn't support FTMS control" + device name |
| `control_refused` | Request Control result ≠ success | hint: close Zwift/other apps holding control |
| `control_lost` | CP timeout ×2 mid-ride | auto-pause + reconnect banner (§5.4) |
| `fit_encode_failed` | encoder error at ride end | ride row still written, journal preserved, "Retry export" on Summary |
| `disk_full` / io | writes fail | block ride start; toast during ride, journal keeps trying |

Logging: `tracing` with rolling file in appdata `logs/`; BLE packet-level at
`debug`. "Report a problem" = reveal log folder.

---

## 12. Testing

- **`tp-core` unit tests**: parser corpus (`testdata/workouts/` — collect ≥20
  real ZWO incl. every element in §3.1 table, ≥10 ERG/MRC); engine fed
  scripted Ticks over full workouts (snapshot effect streams); NP/TSS against
  hand-computed fixtures; FIT encoder → decode with FitCSVTool in CI (Java in
  CI image) → snapshot CSV.
- **Integration (no hardware)**: full ride against `SimTrainer` — load →
  ride 10 min compressed (Ticks driven, not wall-clock) → end → assert FIT
  totals; fault-injection suite: drop @ minute 3 → reconnect → resume →
  assert single continuous ride with pause bracket.
- **Manual hardware gate (per milestone from M2)**: KICKR Core over macOS
  CoreBluetooth — 45-min real workout hands-free; kill Bluetooth mid-ride
  and recover; Garmin Connect upload (M4).
- CI: GitHub Actions, macOS runner; `cargo test` + `cargo clippy -D warnings`
  + frontend typecheck + FIT golden tests on every push.

---

## 13. Milestones & acceptance

| # | Deliverable | Done when |
|---|---|---|
| M0 | Scaffold: Tauri app boots, workspace crates, CI green, SQLite migrations run | `pnpm tauri dev` shows shell; CI passes |
| M1 | Parsers + Library UI + graph thumbnails | corpus parses clean; ZWO cooldown-direction check resolved; import UX incl. warnings works |
| M2 | BLE: scan/pair/save, FTMS driver, live measurement view, manual target slider (dev screen), SimTrainer | holds 150 W ±5 W on real KICKR Core for 5 min; survives BT toggle; fault-injection tests green |
| M3 | Engine + Player UI + HRM + keep-alive + reconnect flow | full 45-min workout hands-free on hardware incl. ramps, skip, intensity, text events |
| M4 | Recorder → journal → FIT → Summary/History → export UX | FitCSVTool zero-error in CI; Garmin Connect upload shows laps/charts/load; crash-recovery replay works |
| M5 | Polish + Windows: WinRT BLE pass, installers (mac notarized DMG, Windows MSI + signing), app icon, onboarding empty-states | fresh machine (both OS) → install → pair → ride → Garmin upload with no dev tools |

Phase 2 (not scheduled): Garmin Connect API auto-sync (awaiting developer
program access), intervals.icu post-ride upload, Strava OAuth upload, Wahoo
legacy driver if demand appears, power match, FIT-workout import. See
[`garmin-access.md`](garmin-access.md) for integration status.

---

## 14. Constants (single source: `tp-core::consts`)

| Constant | Value |
|---|---|
| Engine tick | 250 ms |
| Record sample rate | 1 Hz |
| UI measurement throttle | 4 Hz |
| Power smoothing (display) | 3 s rolling mean |
| ERG keep-alive | 10 s |
| CP response timeout / retries | 2 s / 1 retry |
| Target clamp | 0–2000 W (and trainer max if reported) |
| Intensity range / step | 50–150 % / 1 % |
| Reconnect schedule | 0, 1, 2, 5, 10 s, then 15 s forever |
| Text event default duration | 10 s |
| NP window | 30 s |
| FIT epoch offset | 631 065 600 s |
| Sim: τ power / HR | 1.5 s / 25 s |
