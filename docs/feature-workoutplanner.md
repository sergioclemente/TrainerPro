# WorkoutPlanner connected library

**Purpose:** Define the current contract between TrainerPro and a self-hosted
WorkoutPlanner server. **Audience:** Engineers maintaining either side of the
integration.

## Product role

WorkoutPlanner owns authoring and its saved workout catalog. TrainerPro lists,
previews, and executes cycling workouts. TrainerPro never parses the planner's
DSL or implements its editor; ZWO is the boundary format and TPW is the local
canonical definition.

The integration is an optional Library source, not a scheduling authority.

## Configuration and authentication

Settings → Libraries stores an enabled flag, server URL, and optional HTTP
Basic Auth username/password in the existing source configuration. The Test
action uses the unsaved form values so a connection can be checked before Save.

Requests have a 30-second timeout and one retry to tolerate a sleeping server.
Authentication rejection, transport failure, malformed list responses, and
invalid workouts remain distinct errors.

## HTTP contract

TrainerPro consumes two endpoints:

- `GET /workouts` returns the saved catalog. TrainerPro accepts a bare array or
  a `data`/`workouts` wrapper, tolerates numeric fields encoded as strings, and
  filters to Bike (`sport_type == 1`). Rows supply ID, title, DSL value, tags,
  duration, optional TSS, and optional creation time.
- `GET /workout_file?wid=<id>&format=zwo&ftp=<watts>` returns the selected
  workout as ZWO. The current athlete FTP is supplied for any content whose
  export depends on it.

Basic Auth is attached when a username is configured. WorkoutPlanner should
return 401 for rejected credentials, 404 for a missing workout, and 422 when a
workout cannot be rendered as the requested format.

## Cache and execution

The catalog is cached in SQLite and shown on startup before a successful
network refresh. Per-workout previews are keyed by a hash of the DSL and stored
with the fetched ZWO, allowing a previously previewed workout to be ridden
offline.

Preview and Ride share the same boundary:

```text
WorkoutPlanner DSL -> server-generated ZWO -> TrainerPro parser -> TPW
```

Ride persists or reuses the local definition, records WorkoutPlanner
provenance without overwriting an existing source's provenance, and loads the
ordinary player. Parser warnings use the shared toast path.

**Edit in Planner** opens the web editor with title, cycling sport, and DSL
query parameters. There is no stable ID deep-link contract today, so editing
remains owned by the planner and saving behavior depends on that application.

## Validation

Stub-server tests cover list parsing, Basic Auth, unreachable servers, error
mapping, and ZWO parsing. Manual validation should cover connection testing,
cached startup, preview, Ride, offline fallback after a preview, and opening the
editor against the actual server deployment.
