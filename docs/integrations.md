# Provider integrations

**Purpose:** Explain connected-account capabilities and behavior.
**Audience:** Riders and contributors working with planning and Activity services.

Manage accounts in **Settings → Connections**. Each provider exposes only the
capabilities TrainerPro supports:

| Provider | Capability | Connection |
|---|---|---|
| Intervals.icu | Read scheduled cycling workouts | Personal API key |
| Garmin Connect | Upload completed Activities on request | Garmin sign-in, with verification code when required |

Credentials live in the OS credential manager, isolated between TrainerPro and
TrainerPro QA. Connection status reflects locally available credentials; it does
not guarantee the provider is reachable. One provider's failure does not hide
other connections. Workout libraries use a separate workflow; see
[WorkoutPlanner](feature-workoutplanner.md) for its server contract.

## Intervals.icu

### Connection and schedules

Connect with a personal API key. Only one Intervals.icu account can be active;
disconnect it before connecting another. The current integration is intended
for personal use; broader distribution requires an OAuth design.

Refresh reads scheduled cycling workouts from seven days before through 42 days
after the current date in the account's time zone. Cached workouts appear in
Next Up before background refresh and remain executable offline. Refresh is
also available from Workouts and Settings.

Only structured Ride and VirtualRide workouts are imported. Relative targets
remain relative to FTP. The provider's structured workout is authoritative;
TrainerPro does not reinterpret description text. Unsupported sports or workout
semantics are reported without failing the entire refresh.

### Ownership and removal

Repeated refreshes update the same local schedules and definitions. Provider
edits update workout content and placement together. Events missing from a
successfully fetched window are retired locally; failed requests preserve the
last-good cache.

Provider-owned workouts stay outside the local Library's deletion and
deduplication rules. Importing or cloning a local copy gives it independent
ownership. Disconnecting removes the credential and retires provider schedules
and cached definitions while preserving Activity history.

TrainerPro does not upload Activities to Intervals.icu or create, edit, move,
or delete its calendar events.

## Garmin Connect

### Connection

Sign in with a Garmin email and password, then enter a verification code if
requested. Passwords and codes are used only for sign-in; session tokens are
retained. Rejected or expired refresh credentials require signing in again.
Browser challenges and rate limits are reported without repeated login attempts.

The connection supports Garmin's `.com` service through an unofficial Web
integration. Garmin may change its behavior independently of TrainerPro.
Manual FIT export remains available. If Garmin requires account setup or upload
consent, complete it on Garmin Connect; TrainerPro never grants it for you.

### Activity uploads

Choose **Upload to Garmin** in Summary or Activities to send the original FIT
file. Uploads are manual; there is no automatic queue or retry. Planned-workout
sync and health-data reads are unsupported. Garmin may forward uploaded rides
to linked services such as Strava, depending on account settings.

**Uploaded to Garmin** appears only after Garmin confirms the created Activity.
TrainerPro waits for processing when necessary without resending the file.
Confirmed uploads are remembered per Activity and Garmin account and cannot be
sent again. Reconnecting the same account retains its upload markers; a different
account has separate history.

A timeout, unrecognized response, or incomplete processing leaves acceptance
uncertain. Check Garmin Connect before explicitly retrying. Duplicate responses
are reported separately and do not create a successful upload marker.

Disconnecting removes local session credentials but preserves Activities and
upload history; it does not revoke tokens at Garmin. Deleting a local Activity
removes its local artifacts and upload record, never the Activity in Garmin.
