# TrainerPro behavior specification

**Purpose:** Define current observable behavior and acceptance expectations.
**Audience:** Engineers, reviewers, and testers changing the application.

See [PRODUCT.md](PRODUCT.md) for intent, [architecture.md](architecture.md) for
ownership, and [CONTRIBUTING.md](../CONTRIBUTING.md) for verification rules.

## Application surface

TrainerPro is a dark-theme desktop application with **Workouts**, **Build**,
**Devices**, **Activities**, and **Settings** navigation. Selecting a workout
opens detail; loading it opens the Player; ending a ride opens Summary.
macOS is the validated platform. Windows BLE and installer validation remain
open; Linux is unsupported. Outstanding gates are in [ROADMAP.md](ROADMAP.md).

## Workouts and Next Up

Workouts leads with a horizontally scrollable Next Up rail, then the Library.
Next Up lists active, unfulfilled schedules in local calendar order, followed
by up to three recommendations. Future schedules have no display cutoff beyond
what is cached. Missed schedules remain visible through seven calendar days
after their date, then leave the rail but remain stored. Linking an Activity
fulfills its schedule.

Each schedule's cutoff and labels use its placement time zone, then its account
zone, then machine-local fallback. Recommendations rank the preceding 180 days
of Activity history by frequency, then recency, excluding already-scheduled
definitions. They show a training-focus tag or a generic structured-ride label.
Both item types share detail and execution; schedule identity follows the ride.

## Library, imports, and authoring

The local Library contains TrainerPro-owned definitions. Provider-scheduled
cache entries remain executable through Next Up but are excluded from local
listing, deduplication, and deletion. Importing ZWO, ERG, or MRC converts to TPW,
reports warnings, and deduplicates equivalent local definitions without managing
or deleting the original file.

Opt-in WorkoutPlanner and What's on Zwift libraries have separate tabs and
normalize workouts to TPW. Cached content can remain available offline.
Build supports steady intervals, ramps, free ride, and repetitions, with
validation, duration, graph, and estimated metrics before saving. Detail and
Build show relative targets with watts resolved from current FTP.

## Devices

Devices has Smart Trainer, Heart Rate, and Controller slots. **Scan for devices**
starts shared discovery; results are deduplicated and displayed in arrival order.
Trainer and HRM slots support connect, disconnect, and forget. Saved FTMS trainers
and BLE HRMs reconnect in the background. Connecting cancels an active public scan.

Controller input defaults to **Trainer controls**. Wahoo BIKE SHIFT controls share
the trainer link; disabling input never disconnects the trainer. A paired left
Zwift Ride controller carries both handles on one bonded link and overrides
trainer input without automatic fallback. Disconnecting or forgetting the active
paired controller disables input; disconnect retains its pairing. Forgetting an
inactive pairing leaves trainer controls active.

Wahoo left steering and Ride A pause/resume; hold right steering or Ride Y to talk.
Other buttons are unassigned. Controller loss cancels a hold without pausing the
ride; recovery requires a fresh press. Protocol support targets BIKE SHIFT and
Ride firmware 1.2.0. Bridged or separate-side Ride connections, music controls,
and virtual shifting are excluded. Simulators implement the same device contracts
but do not establish physical compatibility or recovery.

## Workout execution

Loading the Player creates a session and snapshots the workout. Controls support
start, pause, explicit resume, skip, end, ERG toggling, and 50–150% intensity.
Steady/ramp power, free ride, cadence targets, and coaching cues are supported.
Right-clicking the graph selects any interval before or during riding. Leaving
an interval and passing over others records skips; revisiting records another
attempt and, when ridden, another activity segment.

Keyboard controls: hold Space to talk, `s` to skip, `e` to toggle ERG, `d` to
show/hide detailed stats, and Up/Down for intensity. Space never starts, pauses,
or resumes a ride.
The Player shows interval and ride clocks, targets, power, cadence, heart rate,
work, average/normalized power, intensity factor, and training stress.

Displayed power is averaged over three seconds; recording retains unsmoothed
measurements. The graph uses journal samples for ridden power. Prescribed and
ridden cadence share a right-side rpm scale; both scale and trace are hidden when
no cadence is prescribed.
The cursor reaches the active interval's top. Open intervals are hatched, labelled
“open,” and may prescribe cadence.

Trainer-control loss pauses the ride. Recovery restores control and the current
target but requires explicit resume. HRM loss clears only heart-rate data and
never pauses the ride.

## Recording and Activities

A crash-tolerant journal records one-second samples while riding, plus pause,
resume, interval, and session information. Ending replays it, computes summaries
and activity segments, creates FIT, and inserts an Activity. Storage/FIT failures
report an error and preserve the journal whenever recovery is possible.

The FIT file carries the ride and its plan. Records hold power, cadence, heart
rate, and optional speed/distance. Each activity segment becomes a lap with
averages, maxima, Normalized Power, work, and a link to the workout step it rode;
a lap cut short by ending the ride has no link. The workout and its steps are
embedded with the step title, duration, power range (% FTP or watts), cadence
target, and intensity (warm-up, active, rest, cool-down, other). The rider's
weight and FTP are included so consumers can scale zones and W/kg.

Summary offers Save FIT, connected Activity-destination upload, and Done, with
no browser-import or file-reveal action. Activities shows date, workout, duration,
power, training metrics, and available HR; it supports FIT reveal, upload, and
deletion. A configured export directory receives an additional FIT copy.

### Estimated indoor speed and distance

Distance recording defaults on for new installations and preserves saved choices.
Loading snapshots the preference and rider weight; legacy journals without it omit
speed/distance. Existing FIT files and upload receipts are unchanged.

The flat-road estimate uses measured power and combined rider/bicycle mass,
including acceleration, drag, rolling resistance, and coasting. Speed starts at
rest. Zero power permits coasting; missing power resets speed without adding
distance. Pauses freeze speed/distance; resume preserves momentum, as do interval,
intensity, and ERG changes.

Samples cover at most the preceding second, bounded by the previous sample and
latest start/resume. Unsampled active gaps of at least two seconds reset momentum;
paused time does not count. Missing time and time after the final sample add no
distance; overlapping samples receive no double credit. Boundary-spanning distance
is apportioned to activity segments. FIT records, laps, and session totals share
one estimate. Average speed includes all timer time, including coasting and missing
measurements; zero-duration averages are absent. Live speed/distance is not displayed.

## Settings and connections

Settings manages FTP, weight, distance recording, FIT export, voice, libraries,
and provider connections. [Integrations](integrations.md) defines account setup,
inbound Intervals.icu schedules, manual Garmin uploads, and disconnect behavior.
The builder never publishes calendars. [WorkoutPlanner](feature-workoutplanner.md)
remains a separate optional library configured with a URL and optional Basic Auth.

## Failure behavior

Parse failures identify unsupported content. Bluetooth permission, adapter,
compatibility, refused-control, and lost-control failures remain distinguishable.
Provider errors expose sync health; invalid credentials direct riders to Settings
without revealing secrets. Commands provide stable error codes and readable
messages; asynchronous runtime outcomes use the shared toast path.

## Workout voice and timeline

Voice defaults on, checks/requests microphone permission at startup, and immediately
releases the permission-check stream. Bundled models run on-device without runtime
downloads; audio/transcripts are neither stored nor sent.

Holding Space in a focused Player, or a mapped controller button in a visible
Player (even unfocused), captures one utterance. Losing focus cancels keyboard
capture; hidden/minimized windows cannot capture. Release submits at most one
validated command; silence and speech pauses do nothing. Capture cancels
after 15 seconds, on Player exit, disable, input loss, or release during microphone
startup. Pending interpretation is discarded; recovery requires a fresh press.

Voice shares UI actions for start, pause, resume, skip, intensity, and ERG.
End-like commands pause and require manual completion. Skip while paused advances
without restarting the trainer. Disabling voice releases microphone and models;
idle releases the microphone while keeping models ready. Errors must preserve
pointer/keyboard operation. Repeated failures expose Retry and Settings disable;
missing models identify the failing source without blocking the Player.

The rail retains device indicators and a session-only timeline. UI and voice
actions share labels on the user's side; ride events align opposite. The timeline
ends with the current interval's duration, power/cadence targets, and time left.
A midpoint handle collapses it; the choice persists across rides. Unmatched
attempts remain transient in the composer.
