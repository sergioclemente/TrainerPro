# Intervals.icu inbound discovery

Status: read-only API discovery for the first connected-provider integration.
This evidence now backs the implemented personal-key account validation,
bounded calendar client, `workout_doc`-to-TPW adapter, and transactional inbound
sync. The current product scope is inbound-only; OAuth is deferred until broad
multi-user distribution, and outbound capabilities are not currently planned.

## Confirmed public contract

- Personal API keys use HTTP Basic authentication with the literal username
  `API_KEY`. A distributed TrainerPro integration should use OAuth rather than
  collecting personal keys.
- `GET /api/v1/athlete/0/events` addresses the athlete belonging to the
  credential. Explicit `oldest` and `newest` local-date parameters bound the
  calendar request, and `category=WORKOUT` selects planned workouts.
- Event `start_date_local` is a floating local timestamp. TrainerPro treats
  midnight as date-only placement and combines a non-midnight value with the
  connected athlete's time zone.
- Omitting `resolve=true` preserves relative targets such as `%ftp`. Omitting
  `ext` avoids attaching a workout file; the structured `workout_doc` is enough
  for discovery.
- Calendar-event `id` is the candidate provider-scoped identity for inbound
  sync. `external_id` is intended for objects created by our own integration
  and must not replace the provider event ID for arbitrary inbound workouts.
- `workout_doc` documents nested steps, repetitions, time and distance
  durations, ramps, power, heart rate, pace, cadence, and coaching text. It is
  a useful read adapter, but Intervals.icu's supported write contract remains
  its workout-builder description or accepted workout files.

Primary sources:

- [Open API overview](https://www.intervals.icu/features/open-api/)
- [API authentication and calendar endpoints](https://forum.intervals.icu/t/api-access-to-intervals-icu/609)
- [Downloading planned workouts](https://forum.intervals.icu/t/downloading-planned-workouts-from-the-api/93737)
- [Uploading and identifying planned workouts](https://forum.intervals.icu/t/uploading-planned-workouts-to-intervals-icu/63624)

## Live discovery

Use [`tools/intervals-icu-discover`](../tools/intervals-icu-discover) with an
explicit, narrow date range. The key is read only from the environment and fed
to `curl` over standard input, so it is neither persisted nor placed in the
process arguments. The normal output reports field and semantic coverage only;
it does not print names, dates, IDs, descriptions, or targets.

```sh
read -s INTERVALS_ICU_API_KEY
export INTERVALS_ICU_API_KEY
tools/intervals-icu-discover 2026-09-01 2026-10-01
unset INTERVALS_ICU_API_KEY
```

Instead of the report-only call, an optional third argument writes a sanitized
fixture. Keep the first output outside the repository and inspect it before
deciding that it is safe and representative enough to commit:

```sh
tools/intervals-icu-discover 2026-09-01 2026-10-01 \
  /private/tmp/intervals-workouts.sanitized.json
```

The sanitizer retains the workout tree but replaces identifying text, IDs, and
dates, and removes resolved athlete values and profile-derived metadata.

## Evidence still required

Live probes on 2026-09-16:

- events had numeric `id`, a `uid`, an `updated` field, no `external_id`, and a
  structured `workout_doc`;
- the calendar contained both `Run` and `VirtualRide`, confirming that inbound
  filtering cannot equate cycling with the single event type `Ride`;
- run documents included nested repetitions, distance and computed duration,
  warmup/cooldown ramps, coaching text, and pace targets using `secs`;
- cycling documents included nested repetitions, `%ftp` exact and range
  targets, cadence exact and range targets in `rpm`, ramps, and free ride; and
- event- and document-level summary fields included athlete-derived values, so
  the fixture sanitizer explicitly removes them along with resolved targets.

This confirms that an Intervals calendar is multi-sport even though TPW/1 and
TrainerPro execution are cycling-only. Initial inbound sync must explicitly
ignore or retain-as-unsupported non-cycling events rather than failing the
whole sync or coercing them into cycling workouts.

The deliberately created cycling probe exposed a separate provider-boundary
rule. Its description requested `Main set 3x`, but the returned
`workout_doc` contained those two steps only once and reported the corresponding
21-minute duration. TrainerPro must execute the structured provider result and
surface malformed or lossy provider definitions; it must not silently reparse
description text to invent different semantics.

Changing the repeat header to a standalone `3x` produced the expected nested
`reps: 3` node and a 1,740-second total. The resulting hand-reviewed,
sanitized event is
[`testdata/providers/intervals-icu/scheduled-virtual-ride.json`](../testdata/providers/intervals-icu/scheduled-virtual-ride.json).
It contains only synthetic identity/date/text fields and the deliberate probe's
workout semantics; athlete-derived summaries and resolved targets are absent.

Questions retained for any future live revalidation or provider expansion (not
gates for the current personal-key scope):

1. Is calendar-event `id` stable across ordinary edits?
2. Does `updated` reliably change after an edit, and with what precision?
3. Do cycling workouts created by sync from another provider always include
   `workout_doc`?
4. How are deletion, cancellation, all-day placement, and timed placement
   represented?

Provider-scoped persistence preserves stable local identities, remote revision,
last-good data, and bounded deletion semantics. The desktop connection stores
the API key in the OS credential manager, syncs a 7-day lookback and 42-day
lookahead window, and exposes lifecycle/health through Settings. A broader
distribution effort may choose to repeat the end-to-end connect, edit, delete,
offline restart, and ride pass before undertaking production OAuth work; it is
not a gate for the current personal-key scope.
