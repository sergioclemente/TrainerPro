# WorkoutSpec format

**Purpose:** Define the normative **WorkoutSpec/1** JSON format and validation rules.
**Audience:** Producers and consumers of TrainerPro workout definitions.

WorkoutSpec is a declarative JSON format for authoring structured workouts.
It is TrainerPro's canonical replacement for ZWO as the internal workout format. It describes what an athlete should do, independent of the
provider that supplied it, when it is scheduled, and whether it has been
performed. WorkoutSpec is UTF-8 JSON and is designed to be straightforward for people,
deterministic software, and LLMs to read and produce.

`WorkoutSpec` is the in-code representation of a WorkoutSpec document. When a
WorkoutSpec document is exchanged as a file, the suggested suffix is `.workout-spec.json`.
Files are not part of TrainerPro's normal workout workflow or identity.

## Naming compatibility

WorkoutSpec replaces the original name TPW without changing version `1` or
prescription semantics. Writers emit `"format": "WorkoutSpec"`; readers accept
both `"WorkoutSpec"` and the exact legacy identifier `"TPW"`. Unknown identifiers
and unsupported versions remain errors. Parsing a legacy document and serializing
it produces the canonical name. This changes the envelope, not workout intent.

TrainerPro retains existing database columns, IDs, and journal snapshot fields.
Reading historical data does not rewrite it. Local-library deduplication treats
the old and new format identifiers as equivalent without modifying workout text.

## Document envelope

A WorkoutSpec/1 document is a JSON object with these fields:

| Field | Required | Meaning |
|---|---:|---|
| `format` | yes | `"WorkoutSpec"`; readers also accept legacy `"TPW"` |
| `version` | yes | Integer `1` |
| `title` | yes | Non-blank workout title |
| `description` | no | Description; defaults to an empty string |
| `training_focus` | no | Non-blank athlete-facing tag such as `"Endurance Base"` |
| `prescription` | yes | A sport-discriminated prescription |

Unknown fields are invalid. Provider identity, schedule placement, remote
revision, recommendation metadata, and activity measurements are not WorkoutSpec
fields.

This is a complete WorkoutSpec/1 document:

```json
{
  "format": "WorkoutSpec",
  "version": 1,
  "title": "Aerobic Builder",
  "description": "Smooth, controlled work",
  "training_focus": "Endurance Base",
  "prescription": {
    "sport": "cycling",
    "steps": [
      {
        "type": "steady",
        "duration_seconds": 600,
        "power": {
          "type": "percent_ftp_range",
          "min_percent": 65,
          "max_percent": 75
        },
        "cadence": {
          "type": "range",
          "min_rpm": 85,
          "max_rpm": 95
        },
        "cues": [
          {
            "offset_seconds": 0,
            "message": "Relax your shoulders",
            "display_seconds": 10
          }
        ]
      },
      {
        "type": "repeat",
        "count": 3,
        "steps": [
          {
            "type": "ramp",
            "duration_seconds": 60,
            "start_power": { "type": "percent_ftp", "percent": 90 },
            "end_power": { "type": "watts", "watts": 300 }
          },
          {
            "type": "free_ride",
            "duration_seconds": 120
          }
        ]
      }
    ]
  }
}
```

## Cycling prescription

The cycling prescription remains unchanged. A cycling-only consumer must reject
other sports before caching or execution:

```json
{
  "sport": "cycling",
  "steps": [
    {
      "type": "free_ride",
      "duration_seconds": 300
    }
  ]
}
```

`steps` must contain at least one cycling step. Step objects use a `type`
discriminator and accept only the fields defined for that type.

### `steady`

Holds one power target for `duration_seconds`.

| Field | Required | Meaning |
|---|---:|---|
| `type` | yes | `"steady"` |
| `duration_seconds` | yes | Positive integer duration |
| `power` | yes | Cycling power target |
| `cadence` | no | Cycling cadence target |
| `cues` | no | Coaching cues; defaults to an empty list |

### `ramp`

Moves linearly from `start_power` to `end_power` over `duration_seconds`.

| Field | Required | Meaning |
|---|---:|---|
| `type` | yes | `"ramp"` |
| `duration_seconds` | yes | Positive integer duration |
| `start_power` | yes | Cycling power target at the start |
| `end_power` | yes | Cycling power target at the end |
| `cadence` | no | Cycling cadence target |
| `cues` | no | Coaching cues; defaults to an empty list |

### `free_ride`

An open interval: duration without a power target. The trainer runs in
simulation mode. A cadence target may still be prescribed.

| Field | Required | Meaning |
|---|---:|---|
| `type` | yes | `"free_ride"` |
| `duration_seconds` | yes | Positive integer duration |
| `cadence` | no | Cycling cadence target |
| `cues` | no | Coaching cues; defaults to an empty list |

### `repeat`

Repeats its nested `steps` in order.

| Field | Required | Meaning |
|---|---:|---|
| `type` | yes | `"repeat"` |
| `count` | yes | Integer from 1 through 100 |
| `steps` | yes | Non-empty list of cycling steps, including nested repeats |

## Cycling targets

Power targets use the following tagged shapes:

| `type` | Fields | Meaning |
|---|---|---|
| `percent_ftp` | `percent` | Exact percentage of functional threshold power |
| `watts` | `watts` | Exact whole-watt target |
| `percent_ftp_range` | `min_percent`, `max_percent` | Inclusive percentage range |
| `watts_range` | `min_watts`, `max_watts` | Inclusive whole-watt range |

FTP percentages must be finite numbers from 5 through 300. Watt values must
be integers from 0 through 2,000. A range minimum must not exceed its maximum.

Cadence targets use these tagged shapes:

| `type` | Fields | Meaning |
|---|---|---|
| `exact` | `rpm` | Exact whole-rpm target |
| `range` | `min_rpm`, `max_rpm` | Inclusive whole-rpm range |

Cadence values must be integers from 1 through 300 rpm. A range minimum must
not exceed its maximum.

## Running prescription

WorkoutSpec is currently a private format shared by TrainerPro and Harness. Running is
an additive `prescription.sport = "running"` variant within version **1**;
existing cycling documents keep their exact structure and meaning.

A running prescription has a non-empty `steps` array. Each step is either:

- `{"type":"step", "end":..., "target":...}`: one effort, recovery, or other segment.
- `{"type":"repeat", "count":3, "steps":[...]}`: repeat a non-empty group, using the same count/depth/resource limits as cycling.

For a leaf step, `end` is required and has exactly one form:

| Type | Fields | Meaning |
|---|---|---|
| `time` | `duration_seconds` | Positive whole seconds |
| `distance` | `distance_meters` | Positive whole meters |

`target` defaults to `{"type":"open"}`. Supported forms are:

| Type | Fields | Meaning |
|---|---|---|
| `open` | none | No numerical intensity target |
| `pace` | `seconds_per_km` | Positive finite seconds per kilometer |
| `pace_range` | `fast_seconds_per_km`, `slow_seconds_per_km` | Positive finite values, fast ≤ slow |
| `heart_rate` | `bpm` | Whole beats/minute, 1–300 |
| `heart_rate_range` | `min_bpm`, `max_bpm` | Ordered inclusive whole-bpm range, 1–300 |

Optional `cadence` is `{"type":"exact","steps_per_minute":175}` or
`{"type":"range","min_steps_per_minute":170,"max_steps_per_minute":180}`.
Values are whole steps/minute, counting **both feet**, within 1–400. Cycling
`rpm` must not be reused for running cadence. These numeric bounds are format
guardrails, not coaching recommendations.

Optional `role` is one of `warmup`, `work`, `recovery`, `cooldown`, or `rest`.
It records authored intent; its absence does not authorize inferring a role.
Optional `cues` use the cue fields below. Time-ended steps support offsets
within their duration. Distance-ended steps support only `offset_seconds: 0`:
their duration is unknown, so a time offset cannot be validated from pace intent.

For example, this prescription expresses three timed efforts:

```json
{
  "sport": "running",
  "steps": [{
    "type": "repeat", "count": 3,
    "steps": [{
      "type": "step", "role": "work",
      "end": {"type": "time", "duration_seconds": 480},
      "target": {"type": "pace_range", "fast_seconds_per_km": 300, "slow_seconds_per_km": 330}
    }, {
      "type": "step", "role": "recovery",
      "end": {"type": "time", "duration_seconds": 90},
      "target": {"type": "open"}
    }]
  }]
}
```

Complete synthetic contract examples live in `testdata/workout-spec/`. A distance-ended
prescription has no exact `duration_seconds`; the reader reports unknown
duration even if a pace target exists. Targets are intent, not execution facts.

## Coaching cues

A cue belongs to a cycling `steady`, `ramp`, or `free_ride` step, or a running `step`:

| Field | Required | Meaning |
|---|---:|---|
| `offset_seconds` | yes | Whole seconds from the start of its containing step |
| `message` | yes | Non-blank text |
| `display_seconds` | no | Positive integer display time; defaults to 10 seconds |

`offset_seconds` must be less than the containing step's duration. Cues inside
a repeat occur once for every repetition.

## Validation and resource limits

A conforming WorkoutSpec/1 reader rejects:

- an unsupported `format` or `version`;
- unknown fields or variants;
- invalid metadata, targets, cues, durations, or ranges;
- more than eight nested repeat levels;
- a prescription that expands past 10,000 executable segments; or
- expanded time-ended duration greater than 4,294,967,295 seconds; or
- expanded distance-ended distance greater than 4,294,967,295 meters.

These bounds protect execution and resource use. They are not provider
defaults or coaching recommendations.

## Compilation for cycling execution

TrainerPro compiles a validated **cycling** WorkoutSpec/1 definition into its flat
`ExecutableWorkout` model before starting a session. Compilation is pure and
deterministic:

- nested repeats are expanded in order;
- cue offsets become workout-absolute offsets and are sorted;
- percentage ranges compile to their arithmetic midpoint;
- integer watt and cadence ranges compile to their midpoint, rounding a `.5`
  midpoint upward; and
- the original WorkoutSpec definition retains its ranges and nested structure.

The midpoint behavior is an execution policy required by the current ERG
player, which accepts one power setpoint and one displayed cadence target. It
does not redefine the semantic range stored in WorkoutSpec.

Running passes shared format validation but returns an explicit unsupported-sport
error from the cycling compiler. The desktop checks this capability before saving
a definition and before starting execution. Format validity does not imply that
a destination can perform or translate a workout.

## Versioning and sport extensions

Readers inspect `format` and `version` before interpreting the remainder of a
document. Readers reject unsupported versions without trying to partially consume
them. Because all current producers and consumers are privately controlled,
additive variants can be coordinated without changing `version: 1`. An older
cycling-only reader already rejects the unknown running variant. Consumers must
also enforce their own execution and provider capabilities.

Breaking existing semantics or supporting independently deployed external
consumers may require a new version. Keep the current version field for that
future boundary; no format migration is needed for this running extension.

## Provider normalization

A provider adapter maps supported source semantics into WorkoutSpec and reports
unsupported or lossy semantics explicitly. It must not silently discard a
target or step that changes workout meaning. Provider payloads may be retained
outside WorkoutSpec as inert diagnostic or round-trip snapshots.

The provider remains authoritative for a provider-owned definition. WorkoutSpec is the
normalized offline representation, not a claim that TrainerPro owns or can
write the remote object.

## Non-goals

WorkoutSpec/1 does not define:

- provider accounts, IDs, revisions, capabilities, or sync state;
- schedule dates, recommendation rank, or Next Up placement;
- workout-session state or recorded activity measurements;
- a universal superset of every provider format; or
- treadmill control, running power targets, zone lookup, or open-duration steps.

Those concepts either have a different lifecycle or need a later version with
a real provider or execution consumer.
