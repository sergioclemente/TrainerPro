# Provider capabilities

**Purpose:** Verify connection capabilities through the packaged QA application.
**Audience:** QA operators with existing isolated test accounts.

ID: `provider-capabilities`
Input: a pre-removal QA database with Intervals.icu and Garmin accounts, synced
workouts, a local workout copy, an Activity with a stored snapshot, an existing
Garmin upload receipt, and simulated trainer/HRM devices. Follow the [QA runner guide](../README.md).
Use disposable simulated rides for uploads.

1. Upgrade the QA database to the new packaged app. Verify Intervals.icu is
   absent from Connections and its schedules are absent from Next Up and Library.
   Garmin remains connected; recorded Activities, local copies, snapshots, FIT
   export, and existing Garmin upload markers remain available.
2. In Settings → Connections, verify Garmin is the only provider and offers
   plan synchronization and Activity uploads. Refresh and verify sync status
   without changing the account identity or upload receipts.
3. Open Workouts. Verify cached Next Up items render, refresh completes, and
   existing provider schedules remain executable. Labels use the schedule's
   effective time zone.
4. Record a short disposable ride with the simulators. Verify Summary offers
   Save FIT and the connected Activity destination, with no manual import button,
   import hint, or file-reveal action. Upload once;
   verify the confirmed marker appears in Summary and Activities.
5. Restart QA. Verify the migrated and new upload markers remain, and that
   already-confirmed Activities offer no repeated upload action.
6. On a fresh test account setup, verify Garmin's authentication form
   and connection view after sign-in. Continue the provider-specific
   [Garmin scenario](garmin-activity-upload.md) for browser sign-in, reconnect, and
   credential-revocation validation.

Source failures, removed-provider rejection, stale connection IDs,
per-source date boundaries, and transfer coordination have automated coverage;
complete the [Garmin planning scenario](garmin-planning.md) to establish the live
Garmin schedule boundary.
