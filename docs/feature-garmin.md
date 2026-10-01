# Garmin Connect

**Purpose:** Define the unofficial Garmin connection and completed-Activity
upload contract. **Audience:** Riders and integration maintainers.

## Capability

P0 uploads an existing TrainerPro FIT Activity to Garmin Connect when the rider
chooses **Upload to Garmin** in the ride summary or Activities. It supports the
`.com` Garmin service. Planned workouts, automatic uploads, queues, health-data
reads, and schedule sync are outside this capability.

The connection uses Garmin's undocumented services. It is not an approved
Garmin Developer Program integration. Garmin can change sign-in or upload
behavior independently of TrainerPro; manual FIT export and browser import
remain available.

## Account and upload behavior

Sign in under Settings → Connections with a Garmin email and password, then
enter Garmin's verification code if requested. The password and code are used
only for sign-in. Session tokens are retained in the OS credential manager,
isolated between TrainerPro and TrainerPro QA. Rejected or expired refresh
credentials require sign-in again. Browser challenges and rate limits are
reported without repeatedly attempting login. Garmin may require account setup
or permission to upload activity data even after successful sign-in. A failed
upload with this requirement directs the rider to review it on Garmin Connect;
TrainerPro never grants consent on the rider's behalf.

Each upload sends the original recorded FIT file. The generic upload route may
forward the Activity to services linked to Garmin, including Strava; the
actual behavior must be checked against the account's settings.

A successful response with a remote Activity ID records an upload receipt for
that Activity and Garmin account. TrainerPro then shows **Uploaded to Garmin**
and does not resend it. Reconnecting the same account retains these receipts;
a different account has separate upload history. A timeout or unrecognized
response cannot prove whether Garmin accepted the file. Check Garmin Connect
before explicitly retrying. Duplicate responses are reported separately and
do not invent a successful receipt.

Garmin may accept a file before processing completes. TrainerPro checks that
upload's completion resource for a bounded period without resending the file.
Acceptance alone does not create a receipt; completion must identify the
created Activity. If completion remains unconfirmed, check Garmin Connect.

Disconnecting removes local session credentials while preserving Activities
and upload receipts. It does not revoke previously issued tokens at Garmin.
Deleting a local Activity removes its local artifacts and receipt; it does not
delete the uploaded Garmin Activity.

## Protocol reference and release gate

The Rust implementation follows the mobile SSO, DI token, and generic upload
protocol in [python-garminconnect](https://github.com/cyberjunky/python-garminconnect),
inspected on 2026-09-29. Relevant upstream sources are
[authentication](https://github.com/cyberjunky/python-garminconnect/blob/master/garminconnect/client.py)
and [Activity upload](https://github.com/cyberjunky/python-garminconnect/blob/master/garminconnect/__init__.py).
TrainerPro implements the plain HTTP mobile sign-in flow. It does not reproduce
the upstream TLS impersonation and browser fallback strategies.
Asynchronous completion follows the status-resource protocol in
[garminexport](https://github.com/petergardfjall/garminexport/blob/master/garminexport/garminclient.py),
verified against Garmin's live HTTP 202 and 201 responses on 2026-09-30.

Mock-server tests validate the implemented protocol, not Garmin's acceptance
of it. Before declaring P0 complete, use TrainerPro QA and a separate Garmin
account to validate password sign-in, MFA, app restart/session reuse, refresh,
upload of a disposable simulated ride, duplicate handling, and any onward
sync. Verify the uploaded timing, power, laps, and heart-rate data in Garmin.
Live QA validation on 2026-09-30 verified sign-in, session reuse, and upload of
a disposable simulated FIT after the operator completed Garmin's account
consent. Garmin returned HTTP 202, then HTTP 201 with the created Activity ID.
TrainerPro saved the receipt and retained the upload marker in Summary and
Activities after restart. Garmin preserved the 30-second duration, 103 W
average/normalized power, 78 bpm average HR, and recorded laps. Garmin reported
zero moving duration for this stationary ride; the elapsed duration was intact.
A previously uploaded disposable FIT returned a duplicate error without an
invented receipt. The earlier consent failure also created no receipt.

Live MFA, token refresh/revocation, same/different-account reconnection, and
onward sync remain unverified. Their applicable local/protocol behavior has
automated coverage; that does not satisfy the full live release gate.
