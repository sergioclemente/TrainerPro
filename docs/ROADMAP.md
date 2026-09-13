# TrainerPro roadmap

Status: product-outcome sequence, not a release-date commitment. The target
product is defined in [`PRODUCT.md`](PRODUCT.md); the target software shape and
suggested PR cuts are in [`workout-platform.md`](workout-platform.md).

The roadmap deliberately separates foundations, user-visible value, provider
sync, and the future AI service. Each phase should leave the app usable and be
delivered through reviewable pull requests.

## Phase 0 — Align the product and transition plan

Outcome: one durable vocabulary and an explicit replacement for the historical
file/library-first direction.

- Define WorkoutDefinition, ScheduledWorkout, WorkoutRecommendation,
  WorkoutSession, Activity, ProviderConnection, and ProviderLink.
- Establish Next Up as a list of scheduled workouts and recommendations.
- Establish SQLite and semantic JSON as the target workout source of truth.
- Label current-state documentation so it is not mistaken for target design.

Acceptance: the product, roadmap, and implementation documents agree and all
remaining disagreements are listed as open questions.

## Phase 1 — Data and domain foundation

Outcome: the application has one data-access boundary and a provider-neutral
workout definition that compiles into the existing execution engine.

- Move raw SQL out of IPC commands, device/runtime services, and provider code
  into focused backend data-access modules.
- Add a versioned semantic WorkoutDefinition JSON model in `tp-core` and a pure
  compiler to the executable workout model.
- Make SQLite authoritative for workout definitions and their provenance.
- Treat ZWO, ERG, MRC, provider text, and provider JSON as import/export
  adapters.
- Preserve `tp-core` purity and use the existing backend crate for I/O; no new
  data-model crate is planned.

Acceptance: a workout can be created from semantic JSON, persisted, loaded,
compiled, and ridden without a file being its identity. Existing format
adapters have focused conversion tests.

## Phase 2 — Next Up and activity language

Outcome: TrainerPro opens on an execution-first list rather than a workout
library.

- Add scheduled-workout persistence and queries.
- Add the Next Up read model.
- Produce initial local favorite recommendations from eligible workout/activity
  history using a deterministic frequency heuristic.
- Show training-focus tags such as Recovery Ride or Endurance Base; do not show
  ranking explanations as the tag.
- Build the Next Up list UI and make it the default screen.
- Move catalog/library browsing to a secondary Browse surface.
- Use Activity consistently for completed recordings in product copy and API
  names as those surfaces are touched.

Acceptance: with no provider connected, activity history can produce useful
recommendations; with scheduled fixtures present, both item kinds appear in a
single list and start the same workout-session flow.

## Phase 3 — Provider foundation and Intervals.icu inbound sync

Outcome: scheduled workouts from an external planning authority are locally
available and remain ridable offline.

- Add provider connections, capability declarations, provider links, external
  revisions, sync cursors, and observable sync status.
- Implement Intervals.icu authentication and connection validation.
- Pull a bounded scheduled-workout horizon and normalize its workout-builder
  descriptions or downloadable representations into WorkoutDefinition JSON.
- Make repeated sync idempotent and handle remote deletion explicitly.
- Surface provider ownership and sync health without making the user manage
  cache records.

Acceptance: connect, initial sync, incremental refresh, restart offline, and
ride a scheduled Intervals.icu workout successfully. A repeated sync creates no
duplicates.

## Phase 4 — Intervals.icu round trip

Outcome: TrainerPro closes the planned-versus-performed loop without silent
conflicts.

- Upload completed activities where authorized.
- Support explicit publishing of TrainerPro-owned workout definitions or
  schedule changes where the provider contract permits it.
- Detect divergent provider revisions and surface conflicts; do not silently
  merge structured workouts or use last-writer-wins.
- Define retry and idempotency behavior for every outbound operation.

Acceptance: outbound retries are safe, provider-owned definitions remain
provider-owned, and conflicting edits require an explicit user decision.

## Phase 5 — Additional providers

Outcome: the capability model proves it can support asymmetric integrations.

Candidate order depends on access and user evidence:

- adapt WorkoutPlanner and What’s on Zwift to the canonical definition pipeline;
- add TrainingPeaks scheduled-workout support if partner access is available;
- add Garmin training/activity capabilities when program access permits; and
- add Strava as an activity integration unless its public planning API changes.

Acceptance: a provider implements only its real capabilities; no connector is
forced to pretend it supports schedules, recommendations, catalogs, and
activities symmetrically.

## Phase 6 — AI coach service

Outcome: an AI provider can use TrainerPro context and tools to supply adaptive
workout recommendations and, with confirmation, plan changes.

- Host a private remote MCP service backed by the same product model.
- Expose the minimum useful read context: athlete profile and constraints,
  recent activities, current schedule, and available workout definitions.
- Expose proposal/mutation tools with narrow schemas and approval boundaries.
- Build a ChatGPT app visualization for workout structure and relevant training
  context.
- Evaluate recommendation quality, continuity, privacy, and operating model
  before broader availability.

Acceptance: the AI coach can propose a validated, visualized recommendation,
the user can start it through the ordinary session flow, and the resulting
activity is available as context for the next interaction.

## Cross-cutting gates

Every phase must preserve:

- deterministic workout execution in pure `tp-core` logic;
- trainer and heart-rate hardware behind their existing connection contracts;
- offline access to already-synced executable workouts;
- visible provider ownership and sync failures;
- transactional and idempotent persistence at retry boundaries;
- typed Rust/TypeScript IPC changes made together; and
- focused tests plus the repository-wide verification required by
  [`CONTRIBUTING.md`](../CONTRIBUTING.md).

Physical Bluetooth recovery remains a manual gate when a phase touches the
player or device lifecycle.
