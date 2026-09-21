# Provider integration status

> This document records current implementation and access status. The broader
> provider product direction is in [`PRODUCT.md`](PRODUCT.md), with sequencing
> in [`ROADMAP.md`](ROADMAP.md).

TrainerPro currently produces a Garmin-compatible FIT file for every completed
activity. The canonical FIT and its session journal stay in the application
data directory; the user may also configure an export folder, save a copy
elsewhere, reveal the file, or open Garmin Connect for manual upload.

## Garmin Connect

Direct Garmin Connect synchronization is planned but not implemented. The
project is waiting for Garmin Developer Program access before committing to the
authentication, token-custody, and server responsibilities that direct sync
requires. Manual FIT upload remains the supported path in the meantime.

## Intervals.icu

TrainerPro has a pure, fixture-tested adapter from Intervals.icu's structured
cycling `workout_doc` into TPW and a bounded calendar client using the
documented personal-API-key authentication. Settings → Connections validates
the account and exposes sync health. The API key lives in the OS credential
manager, scoped separately for production and QA; SQLite contains only the
non-secret athlete identity, time zone, lifecycle, and sync status.

Connecting triggers an initial refresh. The Workouts screen renders its cached
Next Up projection immediately and attempts one background refresh per app run;
the user can also refresh from Workouts or Settings. Each completed response
transactionally and idempotently reconciles the window from 7 days before to 42
days after the athlete-local date. A failed fetch leaves the last-good workouts
available offline. Disconnecting removes the credential and retires that
account's active cached schedules without deleting Activity history. A missed
schedule remains in Next Up through seven calendar days after its scheduled
date; older rows remain stored but leave the projection.

This is the personal/local authentication path. Intervals.icu's guidance says a
distributed integration should use OAuth. Personal API keys remain the current
scope; before broad multi-user distribution, TrainerPro must confirm a safe
native-app OAuth flow with Intervals.icu or introduce an appropriate token-
exchange service. There is no Activity upload, schedule write-back, webhook, or
outbound retry queue, and those capabilities are not in the current plan. The
unused `activities.icu_activity_id` column does not imply an Activity-sync
commitment.

Read-only API assumptions and the privacy-safe live probe are documented in
[`intervals-icu-discovery.md`](intervals-icu-discovery.md).

The accepted current direction is deliberately narrower than the earlier
round-trip proposal: Intervals.icu is an inbound planning authority that feeds
an offline executable cache. Personal API-key reads and the scheduled-workout
shape are validated. Activity upload, calendar writes, webhooks, and conflict
resolution are deferred rather than unfinished gates.

Preserve these invariants:

- Already-synced workouts remain executable offline.
- Provider-owned workouts are not silently overwritten by local changes.
- Credentials are disabled by default and stored through the provider-
  connection design rather than ad hoc settings.

The old proposed `icu_test` / `icu_upload_activity` command surface remains
superseded. Intervals.icu uses the concrete inbound connector and sync model in
[`workout-platform.md`](workout-platform.md); it does not justify a generic
connector framework.
