# Intervals.icu inbound schedules

**Purpose:** Define the current Intervals.icu integration contract and the
external evidence it depends on. **Audience:** Engineers maintaining or
revalidating the provider boundary.

## Scope

Intervals.icu is an inbound planning authority. TrainerPro reads scheduled
cycling workouts into an offline executable cache. It does not upload
Activities or create, edit, move, or delete the athlete's Intervals.icu events.

The desktop connection uses a personal API key. Broader multi-user distribution
requires a separately approved OAuth design; it is not part of the current
local/personal path.

## Connection and privacy

The key is validated against `GET /api/v1/athlete/0` using HTTP Basic auth with
the literal username `API_KEY`. It is stored in the OS credential manager under
a service name scoped to the application bundle, keeping production and QA
credentials separate.

SQLite stores only non-secret account identity, display name, IANA time zone,
connection lifecycle, and sync health. One Intervals.icu account may be active
at a time. Connecting another requires disconnecting the current account.

Never put an API key in source, fixtures, documentation, command arguments, or
logs.

## Synchronization

A refresh requests `WORKOUT` events from seven days before through 42 days
after the athlete-local date. The account time zone defines that calendar
context. The request omits `resolve=true` so relative targets such as `%ftp`
remain relative, and it does not request attached workout files.

Only `Ride` and `VirtualRide` events with a structured `workout_doc` enter the
cycling adapter. Unsupported sports or semantics are reported without failing
the complete window. TrainerPro uses the provider's structured result; it does
not reparse description text to invent different workout meaning.

Mapping completes before a database transaction begins. Reconciliation:

- scopes external event identity to the provider connection;
- preserves local schedule and definition IDs across repeated sync;
- updates remote revisions, TPW, and placement together;
- soft-removes previously cached events missing from a complete fetched window;
- retains the last-good cache after network or response failure; and
- keeps provider-owned definitions outside the local Library lifecycle.

Cached Next Up data renders before the background refresh. Users can also
refresh from Workouts or Settings. Disconnect removes the credential and
retires active provider schedules and definitions while preserving Activities.

## Documented provider behavior

The following comes from public Intervals.icu documentation:

- personal-key authentication and the athlete calendar endpoint;
- `oldest`, `newest`, and `category=WORKOUT` request parameters;
- `start_date_local` calendar placement;
- structured `workout_doc` downloads;
- workout-builder descriptions or accepted files as the supported write path;
  and
- `external_id` for objects created by an integration, rather than as the
  identity of arbitrary inbound events.

Sources:

- [Open API overview](https://www.intervals.icu/features/open-api/)
- [API authentication and calendar endpoints](https://forum.intervals.icu/t/api-access-to-intervals-icu/609)
- [Downloading planned workouts](https://forum.intervals.icu/t/downloading-planned-workouts-from-the-api/93737)
- [Uploading and identifying planned workouts](https://forum.intervals.icu/t/uploading-planned-workouts-to-intervals-icu/63624)

## Live evidence and revalidation

Privacy-safe probes in September 2026 confirmed numeric event IDs, revision
timestamps, multi-sport calendars, structured cycling repetitions, `%ftp`
targets and ranges, cadence, ramps, free ride, and coaching text. They also
showed that malformed description syntax can produce a structured result that
differs from the author's intent, reinforcing that `workout_doc` is the inbound
source of truth.

The sanitized fixture at
[`testdata/providers/intervals-icu/scheduled-virtual-ride.json`](../testdata/providers/intervals-icu/scheduled-virtual-ride.json)
contains synthetic identity and text with athlete-derived values removed.

Revalidate before expanding the integration, especially event-ID stability,
revision precision, deletion/cancellation representation, and the availability
of `workout_doc` on workouts synchronized from other services. Treat any future
live investigation as a narrowly scoped task: keep credentials out of source,
arguments, and logs, and do not retain identifying provider payloads.
