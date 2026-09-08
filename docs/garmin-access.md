# Garmin and intervals.icu integration status

TrainerPro currently produces a Garmin-compatible FIT file for every completed
ride. The canonical FIT and its journal stay in the application data directory;
the user may also configure an export folder, save a copy elsewhere, reveal the
file, or open Garmin Connect for manual upload.

## Garmin Connect

Direct Garmin Connect synchronization is planned but not implemented. The
project is waiting for Garmin Developer Program access before committing to the
authentication, token-custody, and server responsibilities that direct sync
requires. Manual FIT upload remains the supported path in the meantime.

## intervals.icu

An optional intervals.icu post-ride upload is also planned but not implemented.
The database already reserves `rides.icu_activity_id`; that column remains
unused until the integration is built. There is currently no intervals.icu
credential setting, backend client, IPC command, automatic upload, or retry UI.

When implemented, the integration must preserve these invariants:

- Local FIT and journal files remain authoritative.
- Upload is best-effort and cannot make ride finalization fail.
- A failed upload remains manually retryable from Summary or History.
- Credentials are disabled by default and stored with the same care as other
  source credentials.
- The ride UUID should be used as an external identifier only after live API
  validation proves retries are idempotent.

The proposed command surface is `icu_test(cfg)` and
`icu_upload_ride(ride_id)`. Keep intervals.icu as an export sink; it must not be
folded into the workout-source plugin layer.
