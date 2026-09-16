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
cycling `workout_doc` into TPW. There is not yet a provider connection:
the database reserves `activities.icu_activity_id`, but there is no credential
setting, backend client, IPC command, automatic upload, schedule pull, or retry
UI.

Read-only API assumptions and the privacy-safe live probe are documented in
[`intervals-icu-discovery.md`](intervals-icu-discovery.md).

The accepted direction is broader than the original post-ride-export proposal:
Intervals.icu is the first candidate planning authority for inbound scheduled
workouts, offline execution, activity upload, and an explicitly designed
two-way sync. Its open API, external IDs, and calendar webhooks make that worth
proving. Personal API-key reads and the scheduled-workout shape are now
validated; production OAuth/token custody, polling/webhooks, conflicts, and
device-export behavior remain implementation gates.

When implemented, preserve these invariants:

- A provider failure cannot make activity finalization fail.
- Already-synced workouts remain executable offline.
- Failed outbound operations are visible and retryable.
- Stable external identities are validated against the live API before relying
  on retry idempotency.
- Provider-owned workouts are not silently overwritten by local changes.
- Credentials are disabled by default and stored through the provider-
  connection design rather than ad hoc settings.

The old proposed `icu_test` / `icu_upload_activity`-only command surface and
export-sink-only restriction are superseded. Intervals.icu should use the
capability-specific connector and sync model in
[`workout-platform.md`](workout-platform.md).
