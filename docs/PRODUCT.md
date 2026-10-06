# TrainerPro product direction

**Purpose:** Define TrainerPro's durable product promise, vocabulary, and
decisions. **Audience:** Product contributors and engineers deciding what the
application should become.

Current behavior belongs in [the specification](SPEC.md); unfinished sequencing
belongs in [the roadmap](ROADMAP.md).

## Product promise

TrainerPro is an execution-first, source-agnostic indoor-training application.
Its primary job is to answer **“What should I ride now?”**, execute that workout
reliably, record the result, and make the Activity available to the services the
athlete uses.

The Workouts screen leads with **Next Up**, followed by the Library. TrainerPro
is not primarily a calendar, file manager, or large workout-catalog product.

## Principles

1. **Execution comes first.** Starting the relevant workout should take less
   effort than managing workouts.
2. **Next Up is not a calendar.** Dates provide context, but external planning
   services remain the normal scheduling authority.
3. **Local authoring is secondary.** The builder supports basic workouts and
   clone-and-adjust workflows for riders without a planning integration.
   External services own plan generation, calendar editing, and publication.
4. **Files are boundary formats.** ZWO, ERG, MRC, and FIT are useful for
   interchange and export, not as internal workout identity.
5. **Offline execution matters.** Synced workouts remain locally executable
   when their provider is unavailable.
6. **Ownership is explicit.** Provider-owned items and TrainerPro-owned copies
   do not silently overwrite or retire one another.
7. **Integrations expose real capabilities.** Similar-looking providers do not
   justify a symmetric connector framework when their APIs and policies differ.

## Vocabulary

### Workout definitions

| Term | Meaning |
|---|---|
| **Workout definition** | An undated structured prescription containing steps, targets, repetitions, cues, and training metadata. |
| **Workout segment** | A prescribed interval in the executable workout, with its duration and targets. |
| **Training-focus tag** | Athlete-facing intent such as Recovery Ride, Endurance Base, Threshold, or VO2 Max. |

### Planning and execution

| Term | Meaning |
|---|---|
| **Provider connection** | An authorized external account together with its capabilities and sync health. |
| **Scheduled workout** | A workout definition placed on a date or time by a planning authority, with ownership and sync identity. |
| **Workout recommendation** | An uncommitted suggestion. Starting it does not place it on a calendar. |
| **Workout session** | One live execution of a snapshotted workout, including pause state and adjustments. |
| **Workout segment result** | The outcome of one attempt at a workout segment, including entirely skipped attempts that need not produce an activity segment. |

### Recorded activities and export

| Term | Meaning |
|---|---|
| **Activity** | The recorded historical result of a session. FIT is an export representation, not the entity itself. |
| **Activity segment** | A recorded portion of a ride, with actual timing and measurements; revisiting a workout segment can produce another activity segment. |
| **FIT lap** | The export representation of an activity segment. |

There is no persisted queue or `NextUp` entity. Next Up is a read model composed
from scheduled workouts and recommendations.

## Product experience

Next Up is an ordered, horizontally scrollable rail for choosing a ride.
Scheduled workouts appear chronologically with compact date/time context;
recommendations show training purpose rather than an explanation of the ranking
algorithm. The exact retention and ordering rules are specified in
[SPEC.md](SPEC.md).

Scheduled items and recommendations share the same detail, compilation, player,
session, and Activity flow. Starting either snapshots the executable workout so
a later provider edit cannot change the ride in progress or its history.

The Library remains available for deliberate browsing and local ownership.
Provider-owned schedule cache entries do not become removable local-library
items. Cloning or importing an identical definition creates an independently
owned local copy.

## Workout representation

SQLite is the authoritative local store for normalized workout definitions,
schedules, provider links, and Activities. **TrainerPro Workout (TPW)** is the
versioned semantic JSON representation of a workout definition. It is designed
for deterministic software, people, and language models to read and produce.
The normative contract is [TPW/1](TPW.md).

A definition compiles into the validated, possibly flattened workout the
current trainer engine can execute:

```text
WorkoutDefinition -> ExecutableWorkout -> WorkoutSession -> Activity
```

Provider payloads may be retained for diagnostics or loss-aware conversion,
but they are not a second editable source of truth.

## Provider direction

Connected accounts expose the capabilities TrainerPro actually implements:
**plan sources** supply scheduled workouts; **Activity destinations** receive
completed rides. One connection may support both. Library catalogs and local
authoring retain their separate workflows.

- **Intervals.icu** is currently an inbound planning authority. Activity upload
  and calendar writes are outside the current direction.
- **WorkoutPlanner** is a self-hosted definition source and editor.
- **What's on Zwift** is a read-only workout catalog.
- **Garmin Connect** supports explicit completed-Activity uploads through an
  unofficial Web connection. Live account validation gates release; the
  contract is in [feature-garmin.md](feature-garmin.md).
- **TrainingPeaks and Strava** remain access- and evidence-dependent.
- A future **AI planner** is an external planning service. It owns training
  context, plan generation, review, and publication. TrainerPro can consume its
  scheduled workouts and send it completed Activities through these capabilities.

## Non-goals

- A calendar-first planning interface.
- Silent last-writer-wins synchronization.
- Treating a workout file as canonical identity.
- A generic connector abstraction without multiple concrete consumers.
- Plan publication or calendar write-back from TrainerPro.
- Intervals.icu Activity upload or external activity-history reads in the current scope.
- Production OAuth before broad multi-user distribution requires it.

## Open product questions

- How far into the future should scheduled workouts appear in Next Up?
- Which provider should follow Intervals.icu, based on access and user demand?
- What minimum athlete context makes an AI recommendation meaningfully better
  than deterministic local recommendations?
