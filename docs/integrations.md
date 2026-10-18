# Provider integrations

**Purpose:** Explain connected-account capabilities and behavior.
**Audience:** Riders and contributors working with planning and Activity services.

Manage accounts in **Settings → Connections**. Each provider exposes only the
capabilities TrainerPro supports:

| Provider       | Capability                                                              | Connection             |
| -------------- | ----------------------------------------------------------------------- | ---------------------- |
| Intervals.icu  | Read scheduled cycling workouts                                         | Personal API key       |
| Garmin Connect | Read scheduled cycling workouts; upload completed Activities on request | Garmin browser sign-in |

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
catalog identity rules. Importing or cloning a local copy gives it independent
ownership. Disconnecting removes the credential and retires provider schedules
and cached definitions while preserving Activity history.

TrainerPro does not upload Activities to Intervals.icu or create, edit, move,
or delete its calendar events.

## Garmin Connect

### Connection

Choose **Sign in to Garmin** to open Garmin's website in a compact, resizable
private browser window. Enter passwords, verification codes, and browser challenges
on Garmin's page. Successful sign-in returns to TrainerPro automatically; closing
the window or choosing **Cancel sign-in** cancels the attempt. TrainerPro verifies
the account and retains session tokens in the system credential manager. Rejected
or expired refresh credentials require signing in again. Rate limits are reported
without repeated login attempts or a password-login fallback.

The desktop browser path has passed a user-operated macOS QA sign-in and reconnect
check. This does not establish Windows sign-in or general provider availability.
The shared Rust client's explicit browser-ticket exchange remains available to
Harness; neither application imports cookies from the user's normal browser.

The connection supports Garmin's `.com` service through an unofficial Web
integration. Garmin may change its behavior independently of TrainerPro.
Manual FIT export remains available. If Garmin requires account setup or upload
consent, complete it on Garmin Connect; TrainerPro never grants it for you.

### Scheduled cycling workouts

Garmin also supplies scheduled workouts to Next Up. Sync runs after connection,
once when Workouts first opens, and through **Refresh** or **Sync now**. The window
covers seven days before through 42 days after today. Garmin calendar dates remain
all-day dates; the account time zone is used when available, otherwise the machine's
local zone. This connection currently provides no Garmin account time-zone metadata.

Supported prescriptions have timed cycling steps, fixed repeats, explicit watt or
%FTP targets and ranges, optional cadence targets, free ride, and step instructions.
Relative targets remain relative to TrainerPro FTP. Garmin's unit metadata distinguishes
percentages from watts; values are never guessed from magnitude. Target ranges keep
their semantic bounds and use the existing player's midpoint rule during execution.
HR targets, unresolved zone numbers, distance/lap-button conditions, and unfamiliar
step semantics are reported as unsupported. Daily suggestions and separate Coach-plan
discovery are outside this integration.

Occurrences have separate identities from reusable definitions. Refreshes update
cached occurrences and retire missing events only after complete calendar and detail
reads. Failed reads preserve the cache; conversion issues preserve the affected
workout and appear in sync status. Cached workouts execute offline and stay outside
the Library. Provider edits cannot change loaded rides or Activity snapshots.
Garmin and Intervals.icu remain independent, including duplicate rides.

A bounded macOS QA check verified live timed-cycling import with repeated watt
targets, duplicate-free refresh, cached loading after a restart with provider
requests blocked, simulated target transitions, and local Activity/FIT creation.
Intervals were skipped to shorten the ride. Live calendar edits/removals, account
switching, and Windows behavior remain separate validation boundaries.

Disconnecting retires Garmin-owned schedules and cached definitions while preserving
local copies, Activities, and upload receipts. TrainerPro never creates, edits, moves,
or deletes Garmin calendar entries or workout definitions.

### Activity uploads

Choose **Upload to Garmin** in Summary or Activities to send the original FIT
file. Once Garmin has created the Activity, TrainerPro names it after the
workout and sets its description from the workout's description; if that
second step fails the upload still counts and a warning says the default
"Indoor Cycling" title was kept. The Activity's time is the rider's local time
at the start of the ride. Uploads are manual; there is no automatic queue or
retry. Health-data reads are unsupported. Garmin may forward uploaded rides
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

### Shared activity reads

The shared client provides bounded activity discovery, summary, lap, chart-detail,
and original-file reads for Harness, not desktop history sync. Date-filtered pages
use Garmin calendar bounds; callers pad them and filter UTC timestamps. Operations
read one bounded response without pagination or hidden retries, never forward
credentials through redirects, and preserve original bytes. Callers own encrypted
credentials, refresh serialization, provenance, ZIP/FIT verification, and persistence.

The shared client also provides explicit cycling FTP and running lactate-threshold
response reads for Harness. These do not add desktop health-data reads; Harness owns
observation validation, dates, encrypted connection storage, and retained provenance.

### Shared workout protocol

The shared Rust client supports timed cycling/running-workout creation, read-back,
calendar placement, discovery, and removal operations for Harness's administrator
rehearsal. Workout authoring and calendar writes are not exposed by the desktop application;
bounded schedule and definition reads also support desktop planning. The caller owns
approval, persistent receipts, account coordination, and reconciliation; protocol
writes are sent once. Timed steps, repeats, open targets, and custom pace ranges
are supported for running; cycling authoring supports timed steady/free steps,
fixed repeats, watt/%FTP ranges and optional cadence. Cycling authoring remains
automatically tested but live-unverified. A bounded Harness live check verified running-workout creation/read-back and
calendar placement, and provider cleanup. Device receipt remains unverified because
the test account had no paired device. Mocked tests do not establish provider
availability or device receipt.
