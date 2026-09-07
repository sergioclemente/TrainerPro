# TrainerPro — Design Alternatives & Rationale

Companion to `SPEC.md`. Each section covers one decision the spec baked in:
the options that were on the table, their trade-offs, why the spec picked what
it picked, and **what would change my mind**. Decisions are ordered by how
much they'd cost to reverse later.

---

## D1. App shell / platform stack

*Spec picked: Tauri 2 (Rust core + React/TS UI).* Hardest to reverse — this is
the architecture.

| Option | Pros | Cons |
|---|---|---|
| **Tauri 2 + Rust core** | Small binaries (~10 MB vs ~150 MB); BLE, FIT encoding, and recording live in native code off the UI thread; memory-safe systems language for the reliability-critical parts; installer/signing story fine on both OSes | Rust learning curve if you're not already fluent; smaller ecosystem than Node; IPC boundary between core and UI you have to design |
| **Electron + Node core** | You likely already know the whole stack; huge ecosystem; fastest iteration; Web Bluetooth *or* Node BLE (noble) available | Node BLE libraries (noble/abandonware forks) are chronically under-maintained on macOS/Windows; big memory/disk footprint; BLE + timers on the same event loop as the UI |
| **Electron + Web Bluetooth** | No native BLE code at all; Chromium handles pairing | Desktop Web Bluetooth is the flakiest of all options: chooser-dialog UX, weak reconnect control, background-throttling risks for the 1 Hz loop |
| **Native per-OS (SwiftUI + WinUI)** | Best BLE stacks (CoreBluetooth is the gold standard); best OS integration; smallest runtime overhead | Two complete codebases; doubles every feature forever; only worth it for a mac-only product |
| **Flutter** | One codebase incl. future mobile; `flutter_blue_plus` is decent | Desktop BLE support is the weakest of Flutter's platforms; Dart ecosystem thin for FIT; desktop Flutter still second-class |
| **.NET MAUI / Avalonia + C#** | Good WinRT BLE on Windows; solid language | macOS BLE via MAUI is rough; smaller community for this app shape |

**Why Tauri:** the product's hard parts are exactly the parts that want native
code — a BLE session that must survive 60–90 min without a hiccup, and
byte-level FIT encoding. Rust/btleplug talks directly to CoreBluetooth/WinRT.

**What would change my mind:** if you're a strong TypeScript dev with no Rust
and want to move fast solo, **Electron + noble-style BLE is a legitimate
choice** — TrainerDay and others ship on it. You trade binary size and some
BLE robustness for iteration speed in a language you know. The FIT encoder is
equally writable in TS (Garmin's official JS SDK now includes an encoder,
which actually makes FIT *easier* in the JS world — see D5). If mac-only
forever were acceptable, native Swift would beat both.

---

## D2. Trainer control protocol

*Spec picked: FTMS primary, Wahoo proprietary fallback.*

| Option | Pros | Cons |
|---|---|---|
| **FTMS (BLE standard, 0x1826)** | Open spec; all Wahoo trainers 2017+; same code path later works for Tacx/Elite/JetBlack/etc. for free; well-documented control ops | Pre-2017 KICKRs lack it; minor firmware quirks per vendor (hence keep-alive) |
| **Wahoo proprietary control point** | Works on *every* KICKR back to v1; exposes some Wahoo extras (e.g. ERG mode power smoothing toggle) | Undocumented (community reverse-engineered); Wahoo-only forever; couples you to one vendor's whims |
| **ANT+ FE-C via USB dongle** | The other industry standard; rock-solid; some trainers had FE-C before FTMS | Requires user to own a USB ANT+ dongle; USB driver work per OS; declining relevance as BLE won |
| **Wahoo cloud/companion APIs** | — | Not applicable: no public API controls a trainer in real time; control is BLE-local by design |

**Why FTMS-first:** it's the only option that is simultaneously (a) documented,
(b) sufficient for ERG mode, and (c) not a dead end when you add other trainer
brands. The proprietary fallback is quarantined behind the same trait, so its
blast radius is one driver file.

**What would change my mind:** if your own trainer is a pre-2017 KICKR, the
"fallback" becomes the primary and moves from M5 to M2. Conversely, if no
target user has a legacy KICKR, cut the fallback entirely (Open Question #1 in
the spec). ANT+ only re-enters if you later care about ANT-only power meters.

---

## D3. Garmin export path

*Spec picked: valid FIT + manual upload for v1; API sync phase 2.*

| Option | Pros | Cons |
|---|---|---|
| **FIT file + manual upload** | Zero external dependencies, zero approval, works day one; forces you to get the FIT file *right*, which every other path needs anyway | User drags a file per ride — friction for daily riders |
| **Garmin Connect Developer API** | The real thing: rides appear automatically, training load/status update seamlessly | Application + approval gate (weeks, business justification); OAuth token custody effectively requires running a small backend — your first server, with uptime/privacy obligations |
| **Strava OAuth upload → Garmin via user's existing bridge** | Open API, no approval; many riders already auto-sync Strava⇄Garmin ecosystems | Strava→Garmin direction does **not** exist (Garmin only pushes *to* Strava), so this only helps users whose "source of truth" is Strava, not Garmin |
| **intervals.icu / SyncMyTracks-style intermediary** | intervals.icu has an open API and a loyal cyclist user base | Still doesn't push *into* Garmin Connect; same directionality problem |
| **"Email the FIT file" / iCloud-Drive-watch folders** | Garmin's mobile app can import files; watch-folder hacks exist | Fragile, undocumented, poor UX |

**The uncomfortable truth this table surfaces:** *nothing* gets an activity
into Garmin Connect automatically except Garmin's own gated API. Every
intermediary only syncs *out of* Garmin, not in. So the real decision is only
about **timing**: manual upload now, and **apply for the Garmin developer
program immediately** (it's free; the cost is lead time) so phase 2 isn't
blocked.

**What would change my mind:** nothing changes v1. **Update (2026-07):** the
Garmin developer program has since closed to new applicants and is
enterprise-only, so the "apply early" advice above is no longer actionable —
see `garmin-access.md` for current status and the intervals.icu path, which
covers both directions today.

---

## D4. Player UI scope

*Spec picked: dashboard-only (TrainerRoad-style).*

| Option | Pros | Cons |
|---|---|---|
| **Dashboard only** | Weeks not months; every pixel serves the workout; proven category (TrainerRoad built a business on it) | Less "fun"; no wow factor for demos |
| **Dashboard + 2D ambient layer** (route progress, simple rider animation) | Cheap engagement boost; can be bolted on later without rearchitecting | Scope creep magnet; art assets |
| **3D world (true Zwift-like)** | The dream; social/game loop | 10–50× the effort: 3D engine, content pipeline, physics, probably multiplayer expectations. Not a v1 for one person/small team, full stop |
| **Video-synced workouts** (Sufferfest/Wahoo SYSTM style) | Engaging without 3D | Licensing/producing video content is its own business |

**Why dashboard:** ERG-mode users stare at the interval countdown, not
scenery. The whole product loop (file → ride → FIT → Garmin) is provable
without a single 3D asset.

**What would change my mind:** if the product vision is really "Zwift
competitor" rather than "workout executor," the architecture bet changes too
(game engine — Unity/Bevy — becomes the shell, and D1 gets re-decided). Decide
that *before* M3, not after.

---

## D5. FIT encoding implementation

*Spec picked: hand-rolled Rust encoder over vendored FIT SDK profile subset.*

| Option | Pros | Cons |
|---|---|---|
| **Own minimal Rust encoder** | ~500 lines for the 7 message types needed; no dependency risk; full control over exactly what bytes go out | You own the CRC/scale/offset bugs until FitCSVTool says otherwise |
| **Garmin official FIT SDK (C or C++ via FFI)** | Canonical, always current | FFI plumbing from Rust; SDK's C API is clunky; heavyweight for 7 message types |
| **Garmin official JS SDK (`@garmin/fitsdk`)** | Official *and* has an encoder; trivial if the core were JS | Only helps in the Electron world (see D1); running JS from a Rust core is silly |
| **Community Rust crates** | Free | Nearly all are decode-only; the encoding ones are unmaintained/incomplete — audit before trusting |
| **Emit TCX instead of FIT** | Dead-simple XML | Second-class in Garmin Connect: no sub-sport fidelity, larger files, some fields (e.g. NP/TSS) don't carry; FIT is the native currency |

**Why own encoder:** the subset is small and frozen (activity files haven't
changed shape in years), the validation oracle is free (FitCSVTool in CI), and
dependency-rot in fitness-format crates is a real observed problem.

**What would change my mind:** choosing Electron in D1 flips this instantly to
`@garmin/fitsdk` — official encoder, right language, done.

---

## D6. Workout format scope

*Spec picked: ZWO + ERG/MRC in, expanded flat model internally.*

| Option | Pros | Cons |
|---|---|---|
| **ZWO + ERG/MRC** (spec) | Covers Zwift's ecosystem + the classic ERG world = the vast majority of shared workout files | Two parsers to maintain |
| **ZWO only** | One parser; Zwift files are the most shared | ERG/MRC is trivial (tab-separated text) — cutting it saves almost nothing |
| **Also FIT *workout* files (.fit WKO)** | Garmin's own structured-workout format; users could pull workouts *from* Garmin | FIT workout parsing is more work than ZWO+ERG combined; niche demand; clean phase-2 add |
| **Also TrainingPeaks/intervals.icu API import** | Live calendar sync — big for structured-plan athletes | Requires accounts/OAuth; a product feature, not a parser; phase 2+ |
| **Own workout builder UI** | No files needed at all | A whole editor product; and files are the interchange lingua franca anyway |

**What would change my mind:** if you personally live on TrainingPeaks or a
Garmin calendar, pull FIT-workout import or the relevant API forward — "ride
today's planned workout" is a killer daily-use feature.

---

## D7. Storage & data layer

*Spec picked: files on disk (workouts, FIT, journal) + SQLite index.*

| Option | Pros | Cons |
|---|---|---|
| **Files + SQLite** (spec) | Rides/workouts are portable artifacts the user can grab; SQLite for listing/history queries; zero ops | Two sources of truth to keep consistent (mitigated: files are truth, DB is cache) |
| **SQLite only (blobs in DB)** | Single store | Users lose direct access to their FIT files; export friction for the *core deliverable* |
| **Plain files + JSON index** | Simplest possible | History queries/aggregates get painful as rides accumulate |
| **Cloud backend from day one** | Multi-device history | Accounts, privacy, hosting, for a v1 with zero cloud features — pure liability |

**What would change my mind:** phase 2's Garmin API work forces a small
backend anyway (token custody); *that* is the natural moment to add optional
cloud history, not v1.

---

## D8. Recording robustness model

*Spec picked: JSONL journal during ride → encode FIT at ride end.*

| Option | Pros | Cons |
|---|---|---|
| **Journal → encode at end** (spec) | Crash mid-ride loses ~nothing; FIT summary fields (avg/max/NP/laps) are trivially computed with full data in hand; journal doubles as a debug trace | FIT appears only at ride end (fine — that's when it's needed) |
| **Stream FIT directly during ride** | One artifact | FIT headers/CRC/summary messages make append-safe streaming genuinely fiddly; a crash corrupts the file precisely when you can't retry |
| **In-memory → write at end** | Simplest code | A 90-min ride lost to one crash is the single worst user experience this product can produce |

**What would change my mind:** essentially nothing; this one's cheap insurance
with no real downside.

---

## D9. Sensor scope (revisiting your picks)

*You picked: trainer sensors + HR strap. Flagging the one nearby decision with
real consequences:*

- **Power match** (separate power meter as truth source, app offsets trainer
  target to compensate): deferred — correct for v1, but note that riders who
  own crank/pedal meters often see 5–10 W trainer-vs-meter discrepancies, and
  it's a common feature request in this category. A second device owner behind
  its own connection contract is the phase-2 shape; nothing in v1 blocks it.
- **ANT+ sensors**: skipped entirely (needs USB dongle support). Only
  reconsider if a must-have sensor in your stable is ANT-only.

---

## Summary of the load-bearing decisions

If you only pressure-test three, make it these:

1. **D1 (Tauri/Rust vs Electron/TS)** — decided by *your* language comfort
   more than by anything technical; both ship real products in this category.
   Note D5 flips with it.
2. **D2 fallback scope** — decided by which physical trainer(s) you actually
   own; it reorders milestone M2/M5 content.
3. **D3 timing** — the only path into Garmin is Garmin; apply for API access
   now regardless of everything else.
