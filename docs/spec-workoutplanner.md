# TrainerPro ⇄ WorkoutPlanner Integration Spec (V1 — "Connected Library")

Chosen design: mock `assets/mocks/mock-v1-connected-library.svg` — a
**WorkoutPlanner tab** inside TrainerPro's Workouts screen: log in once, list
the planner's workouts, **Ride** them in TrainerPro, **Edit ↗** them in the
planner's web editor.

This document has two independent parts:
- **Part A** — changes to [WorkoutPlanner](https://github.com/sergioclemente/WorkoutPlanner)
  (a self-hosted server). Written to be handed to a
  separate agent/workspace; it needs no knowledge of TrainerPro.
- **Part B** — the TrainerPro side. Depends only on Part A's HTTP contracts.

## 0. Architecture & non-duplication rule

```
WorkoutPlanner (source of truth: authoring, storage)   TrainerPro (execution)
┌─────────────────────────────┐                        ┌──────────────────────┐
│ DSL text  ──ZwiftDataVisitor┼── GET /workout_file ──▶│ existing ZWO parser  │
│ workouts table ─────────────┼── GET /workouts ──────▶│ planner tab (list)   │
│ web editor ◀────────────────┼── Edit ↗ deep link ────┤                      │
└─────────────────────────────┘                        └──────────────────────┘
```

- **ZWO is the interchange format.** WorkoutPlanner already generates it
  (`src/visitor.ts` → `ZwiftDataVisitor`); TrainerPro already parses it
  (`tp-core::parse::zwo`). Neither side writes new format code.
- TrainerPro NEVER parses the planner's DSL and NEVER implements an editor.
- WorkoutPlanner never learns about trainers, BLE, or FIT.

## 0.1 Is the existing API good enough? (assessment)

| Need | Existing API | Verdict |
|---|---|---|
| List workouts w/ metadata | `GET /workouts` (id, title, value, tags, duration_sec, tss, sport_type) | ✅ sufficient as-is |
| Get a ridable structured workout | ZWO exporter exists but only client-side/email — **no HTTP endpoint** | ❌ gap → **A1 (required)** |
| Auth for a desktop client | optional HTTP Basic (currently likely disabled → API is public) | ⚠️ acceptable mechanism; should be enabled → **A3** |
| Open a workout in the editor | no stable `?wid=` deep link; editor loads from `w=`/`t=` query params | ⚠️ workable today (TrainerPro reconstructs the URL); cleaner with **A4** |
| Change detection for sync | no `updated_ts`/ETag; `creation_ts` not returned | ⚠️ workable (hash the DSL); nicer with **A2** |
| "Today's workout" / calendar | no date/plan concept in the data model | ❌ out of scope for V1 (see A5/Phase 2) |
| Mark rides completed | nothing | ❌ Phase 2 (**A5**) |

Bottom line: one required endpoint (A1) unblocks everything; A2–A4 are small
quality improvements; A5 is the future plan-vs-actual loop.

---

# Part A — WorkoutPlanner changes (separate workspace)

Repo facts the implementer needs (verified 2026-07-20): raw Node `http` server,
handlers registered in `handler_map` in `src/server.ts` (~line 372); all
existing endpoints are GET + query-string; DB wrapper in `src/model_server.ts`
(`workouts` table: id, title, value=DSL text, tags, creation_ts, duration_sec,
tss, sport_type 0=Swim/1=Bike/2=Run); exporters in `src/visitor.ts`
(`ZwiftDataVisitor` ~line 464, `MRCCourseDataVisitor` ~line 537) surfaced via
`WorkoutBuilder` in `src/builder.ts` (`getZWOFile()`, `getMRCFile()`,
`getZWOFileName()`…). Tests are Mocha; build via `./build.sh`.

## A1 (REQUIRED) — `GET /workout_file`

Return a single workout rendered as a structured workout file.

```
GET /workout_file?wid=<id>&format=<zwo|mrc>&ftp=<watts?>
```

- `wid` (required): numeric workout id.
- `format` (required): `zwo` or `mrc`.
- `ftp` (optional, default 200): some visitor output embeds absolute values for
  pace-based content; for Bike/%FTP content the ZWO stays FTP-relative. Accept
  and pass through to `UserProfile` like `/compute_workout` does.

Responses:
- `200` — body is the file text.
  - `Content-Type: application/octet-stream`
  - `Content-Disposition: attachment; filename="<builder's file name>"`
  - `X-Workout-Title: <title>` (convenience header, URL-encoded)
- `404` — `wid` not found (body `{"error":"not_found"}`).
- `400` — bad/missing params (body `{"error":"bad_request","message":…}`).
- `422` — workout exists but can't render in the requested format (e.g. ZWO
  for a Swim workout): `{"error":"unsupported_sport"}`.

Implementation sketch (mirrors the existing `/send_mail` attachment path,
`server.ts` ~lines 108–128): load row by id → `WorkoutBuilder(sport_type,
output_unit, value).withTitle(title)` → `getZWOFile()`/`getMRCFile()` +
`get*FileName()`. Honor Basic Auth exactly like every other handler
(`checkBasicAuth`). Add Mocha tests: happy path zwo + mrc, 404, 422 for
swim→zwo, and a snapshot of ZWO XML for a known DSL (steady + repeat + ramp)
so exporter regressions surface.

## A2 (RECOMMENDED) — extend `GET /workouts` response

Add `creation_ts` to the selected columns/JSON. Additive, backward-compatible.
Gives clients a cheap "newest first" sort and a weak freshness signal.

## A3 (RECOMMENDED) — turn on Basic Auth in production

`fly secrets set BASIC_AUTH_USER=… BASIC_AUTH_PASSWORD=…` — the code path
already exists (`checkBasicAuth`). Currently the API is effectively public.
Document the values for TrainerPro's settings screen. No code changes.

## A4 (NICE-TO-HAVE) — editor deep link by id

Support `?wid=<id>` on the editor page: on load, if `wid` is present and no
`w=` param, fetch the workout (new tiny `GET /workout?wid=` or reuse
`/workouts` + client filter) and populate editor state. Lets external tools
link `https://…/?wid=123` without shipping the whole DSL in the URL. Until
this exists, TrainerPro reconstructs `?t=<title>&st=<sport>&w=<urlencoded DSL>`
from data it already has — functional but produces long URLs and edits are
disconnected from the saved row unless the user re-saves with the same title.

## A5 (PHASE 2 — design only, do not build yet) — completed-ride log

To eventually close the plan-vs-actual loop:
`GET /log_ride?wid=&date=&duration_sec=&tss=&avg_power=&np=` inserting into a
new `ride_log` table. Needs product decisions (per-user model, calendar) —
park it, but avoid schema choices in A1–A4 that would block it.

---

# Part B — TrainerPro changes

## B1. Settings

New keys (settings table, SPEC.md §8 conventions):
- `planner` → JSON `{ "url": "https://your-workoutplanner.example.com",
  "user": "", "pass": "", "enabled": false }`
- Settings screen section "WorkoutPlanner": URL field, user/pass fields
  (Basic Auth; may be blank), **Test connection** button.
- v1 stores the password in SQLite plaintext like other settings; macOS
  Keychain is a later hardening item — note it in the UI copy? No: keep quiet,
  it's a local single-user file. Backlog item only.

## B2. Backend: planner client + IPC

New module `backend/src/planner.rs` (HTTP via `reqwest`, add dependency
with `default-features = false, features = ["rustls-tls"]`):

```
planner_test()            -> { ok, workout_count }        // GET /workouts, HEAD-ish probe
planner_list()            -> Vec<PlannerWorkout>          // GET /workouts, filter sport_type==1 (Bike)
planner_ride(wid)         -> PlayerState                  // fetch + import + load (below)
planner_edit_url(wid)     -> String                       // constructed editor URL
```

`PlannerWorkout` = `{ wid, title, duration_s, tss, tags, dsl }` (dsl kept for
the edit-URL construction; never parsed).

**`planner_ride` pipeline (reuse, don't re-implement):**
1. `GET /workout_file?wid=&format=zwo&ftp=<profile ftp>` (Basic Auth header
   when configured).
2. Write body to a temp file → run the **existing** `import_workout` path
   (sha256 dedup means re-riding an unchanged workout reuses the library row).
3. Tag the row: new nullable columns on `workouts`:
   `origin TEXT` (`'planner'`), `origin_id INTEGER` (wid). Migration #2.
4. `load_workout` it → return `PlayerState` (same flow as clicking a local
   card; all existing guards apply).

**Cold starts (Fly auto-stop):** requests use a 30 s timeout and one retry;
UI shows "Waking up WorkoutPlanner…" state during the first attempt (see B3).

**Errors:** map to existing `AppError` codes — `planner_unreachable`,
`planner_auth` (401), `planner_bad_workout` (422/parse failure). Parse
warnings from the ZWO surface as toasts like file imports do.

## B3. UI (per mock v1)

Library screen gets tabs: **My Library** (exactly today's content) |
**WorkoutPlanner**. Planner tab states:
1. **Not configured** → centered CTA "Connect to WorkoutPlanner" → opens
   Settings section.
2. **Configured, loading** → spinner; after 5 s switch copy to
   "Waking up the planner (free tier sleeps)…".
3. **List** → banner (server host, count, `Sync` button, last-synced time,
   `Settings` link) + rows: title, duration, TSS, tags; buttons **Ride**
   (primary) and **Edit ↗**. No thumbnails in v1 (no parsed model; revisit
   only if A1 grows a JSON-steps format).
4. **Error** → inline message + Retry; auth errors point at Settings.

Rows are fetched live on tab open + manual Sync; cache the last list in
memory only (no DB cache in v1 — the list is one HTTP call).

**Edit ↗:** open (via opener plugin)
`{url}/?t=<title>&st=1&w=<urlencoded dsl>` — or `{url}/?wid=<id>` once A4
ships (feature-detect: config flag `planner.wid_links` defaulting false).

## B4. Testing

- Rust: `planner.rs` unit tests against a local `tiny_http`/hyper stub
  serving canned `/workouts` + `/workout_file` (happy, 401, 422, timeout).
- The stub's ZWO fixture must round-trip through `parse_zwo` — guards the
  interchange contract from the consuming side.
- Manual gate: real server — sync, ride a planner workout on the sim trainer
  end-to-end, verify dedup on second ride, verify Edit ↗ opens the editor
  populated.

## B5. Milestones

| # | Item | Depends on |
|---|---|---|
| WP-1 | Part A1 endpoint (+A2, A3) in WorkoutPlanner | — (other workspace) |
| TP-1 | Settings section + planner.rs client + test-connection | none (stub-testable) |
| TP-2 | Library tabs + planner list UI (states 1–4) | TP-1 |
| TP-3 | Ride pipeline (import tagging migration) + Edit ↗ | TP-1, WP-1 |
| — | Phase 2: results push-back (A5), planner thumbnails, keychain | later |

## Open questions (non-blocking)

1. Multi-sport: planner stores Swim/Run too — v1 filters to Bike. Show
   others greyed-out or hide entirely? (Spec says: hide.)
2. Should Ride-imported planner workouts appear in My Library afterwards?
   (Spec says: yes — they're real library rows, labeled with a small
   "from WorkoutPlanner" badge via `origin`.)
3. `ftp` param on `/workout_file`: TrainerPro sends profile FTP for
   correctness of any absolute-unit content; confirm the visitor honors it
   (A1 implementer: please note behavior in the endpoint's response headers
   or README).
