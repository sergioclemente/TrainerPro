# TrainerPro product direction

Status: accepted product direction for the connected-workout work. This
document defines the product vocabulary and durable decisions. It describes
where TrainerPro is going; [`SPEC.md`](SPEC.md) continues to describe current
implemented behavior until the roadmap migrations land.

## Product promise

TrainerPro is an execution-first, source-agnostic indoor training app. Its
primary job is to answer **“What should I ride now?”**, execute that workout
exceptionally well, record the result, and return the activity to the services
the athlete uses.

The primary experience is **Next Up**, a list of scheduled workouts and
recommendations. TrainerPro is not a calendar application, a workout-file
manager, or a large workout-library product.

## Product principles

1. **Execution comes first.** Finding and starting the relevant workout should
   take less effort than managing workouts.
2. **The home screen is a list, not a calendar.** Dates provide context on
   scheduled items, but TrainerPro does not require a calendar workflow.
3. **External planning is the normal case.** Most workout definitions and
   schedules are expected to come from connected services.
4. **Local planning is secondary.** TrainerPro supports cloning and adjusting
   a workout, but it does not need to compete with full planning products.
5. **Files are boundary formats.** ZWO, FIT, ERG, MRC, or any future format may
   be useful for a provider, device, import, or export. None is the product's
   internal workout identity.
6. **Offline execution matters.** Synced workouts are cached locally and remain
   ridable when their provider is unavailable.
7. **Ownership is explicit.** A provider-owned item and a TrainerPro-owned
   clone do not silently overwrite one another.

## Product vocabulary

| Term | Meaning |
|---|---|
| **Workout definition** | A structured prescription: sport, title, steps, targets, repetitions, cues, and training metadata. It is not dated and contains no execution measurements. In UI copy, “workout” may be used as shorthand. |
| **Scheduled workout** | A workout assigned to a date or date/time by a planning authority. It references or contains a workout definition and carries provider identity and sync state. |
| **Workout recommendation** | An uncommitted workout suggestion. It may be produced locally, by a provider, or by an AI coach. Starting it does not implicitly place it on a calendar. |
| **Training-focus tag** | A short athlete-facing label describing the workout's intended training character, for example **Recovery Ride**, **Endurance Base**, **Tempo**, **Threshold**, or **VO2 Max**. It is not an explanation of why the recommender selected the workout. |
| **Workout session** | A particular live execution of a snapshotted workout definition, including ready/riding/paused/finished state and live adjustments. |
| **Activity** | The recorded historical result of riding, including measurements, summaries, and links to the originating session or scheduled workout. FIT is one representation of an activity, not the entity itself. |
| **Provider connection** | The athlete's authorized connection to an external service, including its capabilities and sync state. |
| **Provider link** | The identity and revision metadata relating a local entity to the corresponding provider object. |

There is deliberately no `QueuedWorkout` concept. An item is either scheduled
or merely recommended. There is also no persisted `NextUp` entity: **Next Up is
a read model and UI projection** assembled from scheduled workouts and
recommendations.

## Next Up

Next Up is TrainerPro's home screen and the default route into a workout
session. It is a vertically ordered list optimized for choosing and starting a
ride, not for editing a training calendar.

The list contains:

- scheduled workouts in chronological order, with compact relative date/time
  context such as **Today**, **Tomorrow**, or **Friday 18:00**; and
- workout recommendations, presented with a training-focus tag rather than an
  algorithmic “recommended because…” explanation.

The first recommender can be deliberately simple and deterministic: choose a
small set of favorite workout definitions from activity history based on how
often they are ridden. That frequency is a ranking implementation detail and
should not become the athlete-facing tag. A frequently selected endurance
workout still appears as **Endurance Base**, not **Frequently ridden**.

Later recommenders, including an AI coach, use the same product contract. They
may produce better selections and richer workout definitions without requiring
a new home screen or execution flow.

Open UX policies that should be decided with the Next Up feature include the
display horizon for future scheduled workouts, how long an overdue workout
remains prominent, and whether scheduled and recommended items are visually
grouped or interleaved. None of those choices turns the list into a calendar.

## Definition, execution, and history

The domain flow is:

```text
WorkoutDefinition
    -> compile and snapshot for the athlete
ExecutableWorkout
    -> start
WorkoutSession
    -> record
Activity
```

A rich workout definition may preserve repetitions and provider-neutral
meaning. An executable workout is the validated, possibly flattened form the
current trainer engine can control. Starting either a scheduled workout or a
recommendation snapshots the executable definition into a workout session so a
later provider edit cannot change the ride already in progress or its history.

`tp-core` remains the pure domain crate. `WorkoutSession` is a domain concept,
not a replacement name for the crate. The current flat `Workout` model is
conceptually an executable workout, while the current `Engine` implements the
pure state-machine portion of a workout session.

## Provider model

Providers expose capabilities rather than conforming to one artificially
symmetric interface. Relevant capabilities include:

- read or write scheduled workouts;
- browse workout definitions or catalogs;
- read or write activities;
- read athlete profile, zones, wellness, or training context;
- supply workout recommendations; and
- support incremental sync, webhooks, or external revision identifiers.

A provider may implement only one capability. Current candidate roles are:

| Provider | Likely role |
|---|---|
| **Intervals.icu** | Planning authority, scheduled-workout sync, activity exchange, and strongest early candidate for two-way sync |
| **TrainingPeaks** | Planning authority and activity exchange; public integration remains subject to partner access |
| **WorkoutPlanner** | Self-hosted workout-definition source; scheduling support depends on its future model |
| **What’s on Zwift** | Read-only workout catalog |
| **Garmin** | Workout/device destination and activity source; publishing TrainerPro activities depends on available program access |
| **Strava** | Activity platform; its public API does not currently expose planned workouts or UI recommendations |
| **TrainerPro AI coach** | Coach and recommendation producer, with optional schedule-writing tools |

These roles are direction, not a promise that every provider offers every API.
Provider capability research must be refreshed when its integration is built.

Research snapshot (2026-09-13):

- Intervals.icu documents planned-workout management, webhooks, OAuth, and
  external-ID mapping in its [Open API overview](https://www.intervals.icu/features/open-api/),
  and documents its parsed native workout representation in a
  [download guide](https://forum.intervals.icu/t/downloading-planned-workouts-from-the-api/93737).
- TrainingPeaks documents its partner-access model in its
  [API overview](https://help.trainingpeaks.com/hc/en-us/articles/234441128-TrainingPeaks-API)
  and its planned/actual workout shape in the
  [Workout object](https://github.com/TrainingPeaks/PartnersAPI/wiki/Workouts-Object).
- Garmin documents workout/plan publication in its
  [Training API](https://developer.garmin.com/gc-developer-program/training-api/)
  and completed records separately in its
  [Activity API](https://developer.garmin.com/gc-developer-program/activity-api/).
- Strava's [public API reference](https://developers.strava.com/docs/reference/)
  exposes activities but no planned-workout or recommendation resource.

## Workout representation and ownership

SQLite is the authoritative local store for normalized workout definitions,
schedules, activities, provider links, and sync state. **TrainerPro Workout
(TPW)** is the versioned semantic JSON format representing a workout definition
inside SQLite and across TrainerPro APIs. It should be straightforward for
deterministic code and LLMs to read and produce. TPW/1 is specified in
[`TPW.md`](TPW.md).

The JSON model should borrow proven ideas from provider step trees, including
nested repetitions, time or distance lengths, target ranges, ramps, cadence,
and coaching text. It must remain TrainerPro-owned and versioned rather than
adopting a provider's undocumented internal schema.

Intervals.icu's text workout-builder syntax is an important adapter: it is
compact, readable, LLM-friendly, and the supported way to author structured
workouts through its API. Its parsed `workout_doc` is useful input, but direct
`workout_doc` writes are not a dependable public interchange contract. Neither
representation is TrainerPro's canonical model.

Authority is singular for each editable definition:

- for a provider-owned workout, the provider revision is authoritative and
  TrainerPro's normalized JSON is an offline cache;
- cloning or modifying one creates a TrainerPro-owned definition; and
- publishing a TrainerPro-owned definition to a provider is explicit and
  creates or updates a provider link.

Provider payloads may be retained for diagnostics or loss-aware round trips,
but they are not a second editable source of truth.

## AI coach direction

The future AI coach is a service, initially potentially privately hosted. Its
premise is that the athlete already has an AI relationship and TrainerPro
supplies the domain tools, context, deterministic validation, and workout
visualization needed to turn that AI into a useful coach.

The closed loop is:

```text
goals + availability + profile + recent activities + current schedule
    -> coach recommendation or proposed schedule change
    -> inspect, visualize, and optionally adjust
    -> workout session
    -> activity and updated context
```

A remote MCP server and ChatGPT app are a natural first delivery path. OpenAI's
[developer platform](https://developers.openai.com/) supports extending
ChatGPT with MCP servers and optional UI. Read tools can expose profile,
schedule, activities, and definitions. Mutating tools can propose
recommendations, schedules, or TrainerPro-owned definitions and must retain
explicit user confirmation where the action changes durable state. The AI
produces coaching decisions; TrainerPro continues to own validation, execution,
recording, provider sync, and visualization.

## Product decisions

The following are accepted unless new product evidence changes them:

1. TrainerPro is execution-first, not file-, library-, or calendar-first.
2. Next Up is the home experience and is a list, not a calendar view.
3. Next Up contains scheduled workouts and workout recommendations only; there
   is no workout queue.
4. Recommendations display a training-focus tag, not a selection reason.
5. Workout definition, scheduled workout, workout session, and activity are
   distinct concepts.
6. SQLite plus versioned TPW semantic JSON is the canonical local workout
   store.
7. Workout and activity file formats are integration details at system
   boundaries.
8. External providers are expected to supply most workouts; local authoring is
   primarily clone-and-adjust.
9. Provider capabilities and ownership are explicit; two-way sync must not
   create silent last-writer-wins behavior.
10. The AI coach builds on the same recommendation, definition, session, and
    activity contracts as non-AI providers.

## Open product questions

- What is the initial normalized training-focus vocabulary, and when should a
  provider-specific label be preserved rather than mapped?
- What are the overdue and future-horizon rules for scheduled items in Next Up?
- Can more than one provider be an active planning authority, or does the first
  release select exactly one?
- Which local changes are session-only, which create a clone, and which may be
  explicitly published back?
- Which athlete context is necessary and appropriate for an AI coach, and what
  retention/privacy controls must accompany it?

## Non-goals for the connected-workout program

- A full calendar editor inside TrainerPro.
- A large locally curated workout catalog.
- Making users manage workout files as part of their normal workflow.
- Perfect backward compatibility with the current development database or
  imported workout library. Preserve data when cheap, but it is not a P0 gate.
- A new crate solely to hold the data model; new boundaries should earn their
  existence through ownership or multiple consumers.
