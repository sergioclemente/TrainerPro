# Provider capabilities

**Purpose:** Verify connection capabilities through the packaged QA application.
**Audience:** QA operators with existing isolated test accounts.

ID: `provider-capabilities`
Input: connected Intervals.icu and Garmin QA accounts, an existing Garmin upload
receipt, and simulated trainer/HRM devices. Follow the QA identity rules in
CONTRIBUTING.md. Use disposable simulated rides for uploads.

1. Upgrade an existing QA database to the new packaged app. Verify both account
   identities remain connected and existing Activity upload markers remain.
2. In Settings → Connections, verify each account has its own status and actions:
   Intervals.icu offers plan synchronization; Garmin offers Activity uploads.
   Refresh Intervals.icu and verify its result and sync time without changing
   Garmin's connection or receipts.
3. Open Workouts. Verify cached Next Up items render, refresh completes, and
   existing provider schedules remain executable. Labels use the schedule's
   effective time zone.
4. Record a short disposable ride with the simulators. Verify Summary offers
   Save FIT and the connected Activity destination, with no manual import button,
   import hint, or file-reveal action. Upload once;
   verify the confirmed marker appears in Summary and Activities.
5. Restart QA. Verify the migrated and new upload markers remain, and that
   already-confirmed Activities offer no repeated upload action.
6. On a fresh test account setup, verify each provider's own authentication form
   and the common connection view after sign-in. Continue the provider-specific
   scenario for MFA, reconnect, and credential-revocation validation.

The 2026-09-30 packaged smoke check verified existing account reuse, migration of
an existing receipt, live Intervals.icu refresh, and a new disposable Garmin
upload through the shared capability commands. Fresh sign-in and the remaining
Garmin authentication/onward-sync gates are tracked in `garmin-activity-upload`.
Independent-source failures, capability rejection, stale connection IDs,
per-source date boundaries, and transfer coordination are covered by automated
tests; the smoke check does not claim multiple live plan-source implementations.
