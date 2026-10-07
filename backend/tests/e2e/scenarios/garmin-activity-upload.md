# Garmin completed-Activity upload

**Purpose:** Validate the unofficial Garmin connection through the packaged QA
application. **Audience:** QA operators with a separate Garmin test account.

ID: `garmin-activity-upload`
Input: human Garmin sign-in and MFA; simulated trainer and HRM.

Follow the [QA runner guide](../README.md) for bundle verification. The operator
supplies credentials directly in the application. Never put passwords, tokens,
codes, or identifying response bodies in test artifacts.

1. Open Settings → Connections. Verify Garmin and Intervals.icu load
   independently and Garmin offers sign-in. Empty fields cannot be submitted.
2. The operator signs into the test Garmin account. Complete MFA when required.
   Verify the account name and connected badge. Restart QA and verify the
   connection persists without password entry.
3. Record a disposable simulated cycling ride with two intervals, power and HR.
   End it and verify Save FIT and Upload to Garmin are available, with no manual
   import button, import hint, or file-reveal action.
4. Choose Upload to Garmin. Verify an in-progress label, then Uploaded to
   Garmin. Verify the Activity exists in Garmin with the expected timing,
   laps, power and HR. Check any downstream service configured on this account.
5. Open Activities and restart QA. Both retain the upload marker. Repeated
   requests to upload the same local Activity must not create another remote
   Activity. Disconnect and reconnect the same Garmin account: the marker
   remains. Connecting a different account must not inherit it.
6. Upload another disposable ride after token expiry to exercise refresh.
   Revoke the session in Garmin, then verify a later upload asks for sign-in.
   Reconnecting restores upload capability without losing existing receipts.
7. Try an Activity whose identical FIT was previously imported manually. Verify
   a duplicate/error result, without an invented successful receipt. Check
   Garmin before retrying an uncertain result.
8. Disconnect Garmin. Existing TrainerPro Activities remain accessible, and
   their FIT files remain exportable. Garmin Activities remain in Garmin.

PASS requires all live assertions above; mock tests and packaging alone do not
satisfy this scenario. Report any untested step explicitly.
