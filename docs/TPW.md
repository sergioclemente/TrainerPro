# TrainerPro Workout (TPW) format

**Purpose:** Define the normative **TPW/1** JSON format and validation rules.
**Audience:** Producers and consumers of TrainerPro workout definitions.

TrainerPro Workout, abbreviated **TPW**, is TrainerPro's semantic workout
definition format. It describes what an athlete should do, independent of the
provider that supplied it, when it is scheduled, and whether it has been
performed. TPW is UTF-8 JSON and is designed to be straightforward for people,
deterministic software, and LLMs to read and produce.

`WorkoutDefinition` is the in-code representation of a TPW document. When a
TPW document is exchanged as a file, the suggested suffix is `.tpw.json`.
Files are not part of TrainerPro's normal workout workflow or identity.

## Document envelope

A TPW/1 document is a JSON object with these fields:

| Field | Required | Meaning |
|---|---:|---|
| `format` | yes | Exact string `"TPW"` |
| `version` | yes | Integer `1` |
| `title` | yes | Non-blank workout title |
| `description` | no | Description; defaults to an empty string |
| `training_focus` | no | Non-blank athlete-facing tag such as `"Endurance Base"` |
| `prescription` | yes | A sport-discriminated prescription |

Unknown fields are invalid. Provider identity, schedule placement, remote
revision, recommendation metadata, and activity measurements are not TPW
fields.

This is a complete TPW/1 document:

```json
{
  "format": "TPW",
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

TPW/1 defines one prescription sport:

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

Specifies duration without a power or cadence target.

| Field | Required | Meaning |
|---|---:|---|
| `type` | yes | `"free_ride"` |
| `duration_seconds` | yes | Positive integer duration |
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

## Coaching cues

A cue belongs to a `steady`, `ramp`, or `free_ride` step:

| Field | Required | Meaning |
|---|---:|---|
| `offset_seconds` | yes | Whole seconds from the start of its containing step |
| `message` | yes | Non-blank text |
| `display_seconds` | no | Positive integer display time; defaults to 10 seconds |

`offset_seconds` must be less than the containing step's duration. Cues inside
a repeat occur once for every repetition.

## Validation and resource limits

A conforming TPW/1 reader rejects:

- an unsupported `format` or `version`;
- unknown fields or variants;
- invalid metadata, targets, cues, durations, or ranges;
- more than eight nested repeat levels;
- a prescription that expands past 10,000 executable segments; or
- a compiled duration greater than 4,294,967,295 seconds.

These bounds protect execution and resource use. They are not provider
defaults or coaching recommendations.

## Compilation for cycling execution

TrainerPro compiles a validated TPW/1 definition into its flat
`ExecutableWorkout` model before starting a session. Compilation is pure and
deterministic:

- nested repeats are expanded in order;
- cue offsets become workout-absolute offsets and are sorted;
- percentage ranges compile to their arithmetic midpoint;
- integer watt and cadence ranges compile to their midpoint, rounding a `.5`
  midpoint upward; and
- the original TPW definition retains its ranges and nested structure.

The midpoint behavior is an execution policy required by the current ERG
player, which accepts one power setpoint and one displayed cadence target. It
does not redefine the semantic range stored in TPW.

## Versioning and sport extensions

Readers inspect `format` and `version` before interpreting the remainder of a
document. A reader that supports only TPW/1 rejects another version without
trying to partially consume it. Changes that add a sport, add fields, or alter
meaning require a new TPW version; future readers may support multiple
versions.

The sport discriminator lives inside `prescription` so each sport can have a
coherent step and target vocabulary. TPW/1 supports only `cycling`. A future
version can add a `running` prescription with the commanded speed or pace and
incline needed for treadmill control, without adding meaningless optional
running fields to cycling steps. The precise running schema is intentionally
deferred until a treadmill execution path or selected provider supplies real
requirements. A `running` prescription is therefore invalid TPW/1 today.

## Provider normalization

A provider adapter maps supported source semantics into TPW and reports
unsupported or lossy semantics explicitly. It must not silently discard a
target or step that changes workout meaning. Provider payloads may be retained
outside TPW as inert diagnostic or round-trip snapshots.

The provider remains authoritative for a provider-owned definition. TPW is the
normalized offline representation, not a claim that TrainerPro owns or can
write the remote object.

## Non-goals

TPW/1 does not define:

- provider accounts, IDs, revisions, capabilities, or sync state;
- schedule dates, recommendation rank, or Next Up placement;
- workout-session state or recorded activity measurements;
- a universal superset of every provider format; or
- running, treadmill, distance-based, heart-rate, or open-duration steps.

Those concepts either have a different lifecycle or need a later version with
a real provider or execution consumer.
