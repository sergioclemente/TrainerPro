# TrainerPro roadmap

**Purpose:** Record unfinished product outcomes, their order, and external
gates. **Audience:** Maintainers choosing the next line of work.

The durable destination is [PRODUCT.md](PRODUCT.md). Shipped behavior belongs
in [SPEC.md](SPEC.md), not here. This roadmap has no date commitments.

## Now — desktop reliability and release readiness

- Validate physical trainer and HRM disconnect/reconnect behavior whenever the
  device lifecycle changes; simulator coverage is necessary but not sufficient.
- Complete Windows BLE and packaged-app validation.
- Establish macOS and Windows signing/notarization before presenting builds as
  broadly installable releases.
- Continue focused usability and recovery improvements around the execution
  flow without expanding TrainerPro into a calendar or content platform.

## Next — evidence-driven provider expansion

- Select another planning or activity provider only when API access and user
  demand justify it.
- Treat TrainingPeaks and Garmin as access-gated integrations.
- Consider Strava only for capabilities its public API actually exposes.
- Prove each concrete capability before extracting shared connector
  abstractions.

## Later — adaptive coaching

- Expose the minimum useful athlete, schedule, Activity, and workout context to
  a private coaching service.
- Let the coach propose validated recommendations or plan changes through
  narrow tools with explicit confirmation for durable mutations.
- Evaluate recommendation quality, privacy, continuity, and operating cost
  before broader distribution.

## Deferred and gated

- Intervals.icu Activity upload, schedule publishing/editing, webhooks,
  conflict resolution, and outbound retry queues are not currently planned.
- Production Intervals.icu OAuth is required only if TrainerPro moves beyond
  the personal/local distribution model.
- Direct Garmin synchronization depends on Garmin Developer Program access;
  manual FIT upload remains the supported path.
- The future display horizon for Next Up remains an open product decision.

Every future change must preserve deterministic `tp-core` execution, offline
access to cached workouts, explicit ownership, observable sync failures, and
the verification rules in [CONTRIBUTING.md](../CONTRIBUTING.md).
