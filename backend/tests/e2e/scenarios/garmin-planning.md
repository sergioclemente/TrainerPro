# Garmin scheduled cycling

**Purpose:** Verify Garmin calendar reads, cached execution, and independent plan-source behavior.
**Audience:** QA operators with a separate Garmin test account and scheduled cycling workouts.

ID: `garmin-planning`
Input: human Garmin sign-in/MFA; an existing timed cycling workout scheduled within
seven days before through 42 days after today; simulated trainer and HRM. Use the
[QA runner guide](../README.md). The operator prepares or changes the external
calendar directly in Garmin; TrainerPro only reads it.

1. In Settings → Connections, connect Garmin through its compact browser window.
   Verify automatic completion, account identity, **Sync now**, and sync status.
   Existing upload receipts must survive reconnecting the same account.
2. Open Workouts. Verify the Garmin occurrence appears in Next Up with the
   correct all-day date, title, duration, power targets, cadence, and instructions.
   Refresh twice: the occurrence must not duplicate or become a local Library item.
3. With Intervals.icu also connected, refresh both. Verify one source's failure
   leaves the other's schedules available and reports a separate outcome.
4. Have the operator schedule the same definition again on a different date.
   Refresh and verify two independent occurrences. Move one and edit its definition
   in Garmin; refresh and verify updated content and placement without duplicates.
5. Load a Garmin workout, then have the operator edit it externally and refresh.
   Verify the loaded player's snapshot is unchanged.
6. Restart QA while offline. Verify the cached Garmin workout still opens and
   executes with the simulated trainer, including repeated steps and cadence.
   Complete a short disposable ride and verify Activity history and manual Garmin
   upload remain available after network access returns.
7. Test an unsupported prescription, such as a lap-button or HR-target step.
   Verify a specific sync issue and that valid scheduled cycling workouts remain
   executable. A failed detail/calendar request must preserve the last-good cache.
8. Have the operator remove one Garmin occurrence and refresh successfully.
   Verify only that occurrence is retired. Disconnect Garmin and verify its cached
   schedules leave Next Up while local copies, Activities, Intervals.icu schedules,
   and existing upload receipts remain intact.

PASS requires every live assertion. Record untested steps explicitly; synthetic
fixtures, the running-workout Harness probe, and an empty calendar do not establish
live cycling import or offline scheduled execution.
