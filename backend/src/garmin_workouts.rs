//! Garmin cycling prescriptions to WorkoutSpec. No transport or persistence.

use serde_json::Value;
use tp_core::consts::{TEXT_EVENT_DEFAULT_S, WORKOUT_REPEAT_DEPTH_MAX};
use tp_core::workout_spec::{
    CyclingCadenceTarget, CyclingPowerTarget, CyclingStep, WorkoutCue, WorkoutFormat,
    WorkoutPrescription, WorkoutSpec, WORKOUT_SPEC_VERSION,
};

const CYCLING_SPORT_ID: u64 = 2;
const TIME_CONDITION_ID: u64 = 2;
const REPEAT_CONDITION_ID: u64 = 7;
const OPEN_TARGET_ID: u64 = 1;
const POWER_TARGET_ID: u64 = 2;
const CADENCE_TARGET_ID: u64 = 3;
const PERCENT_UNIT_ID: u64 = 253;
const UNIT_FACTOR: f64 = 1.0;
const NO_EQUIPMENT_OR_STROKE_ID: u64 = 0;

#[derive(Debug, thiserror::Error)]
pub(crate) enum GarminWorkoutError {
    #[error("Only cycling workouts can be executed by TrainerPro")]
    UnsupportedSport,
    #[error("Unsupported Garmin prescription at {path}: {message}")]
    Unsupported { path: String, message: String },
    #[error(transparent)]
    InvalidDefinition(#[from] tp_core::workout_spec::WorkoutSpecError),
}

type R<T> = Result<T, GarminWorkoutError>;

fn unsupported(path: &str, message: &str) -> GarminWorkoutError {
    GarminWorkoutError::Unsupported {
        path: path.into(),
        message: message.into(),
    }
}

pub(crate) fn to_workout_spec(workout: &Value) -> R<WorkoutSpec> {
    if !descriptor(
        &workout["sportType"],
        "sportTypeId",
        "sportTypeKey",
        CYCLING_SPORT_ID,
        "cycling",
    ) {
        return Err(GarminWorkoutError::UnsupportedSport);
    }
    let segments = workout["workoutSegments"]
        .as_array()
        .ok_or_else(|| unsupported("workoutSegments", "structured segments are required"))?;
    if segments.len() != 1
        || segments[0]["segmentOrder"].as_u64() != Some(1)
        || !descriptor(
            &segments[0]["sportType"],
            "sportTypeId",
            "sportTypeKey",
            CYCLING_SPORT_ID,
            "cycling",
        )
    {
        return Err(unsupported(
            "workoutSegments",
            "one cycling segment is required",
        ));
    }
    let mut order = 0;
    let spec = WorkoutSpec {
        format: WorkoutFormat::WorkoutSpec,
        version: WORKOUT_SPEC_VERSION,
        title: workout["workoutName"]
            .as_str()
            .ok_or_else(|| unsupported("workoutName", "title is required"))?
            .trim()
            .into(),
        description: optional_text(&workout["description"], "description")?.into(),
        training_focus: None,
        prescription: WorkoutPrescription::Cycling {
            steps: convert_steps(&segments[0]["workoutSteps"], "workoutSteps", 0, &mut order)?,
        },
    };
    spec.validate()?;
    Ok(spec)
}

fn descriptor(value: &Value, id_field: &str, key_field: &str, id: u64, key: &str) -> bool {
    value[id_field].as_u64() == Some(id) && value[key_field].as_str() == Some(key)
}

fn optional_text<'a>(value: &'a Value, path: &str) -> R<&'a str> {
    if value.is_null() {
        return Ok("");
    }
    value
        .as_str()
        .map(str::trim)
        .ok_or_else(|| unsupported(path, "expected text"))
}

fn positive_whole(value: &Value, path: &str) -> R<u32> {
    let number = value
        .as_f64()
        .ok_or_else(|| unsupported(path, "positive whole number is required"))?;
    if !number.is_finite() || number <= 0.0 || number.fract() != 0.0 || number > u32::MAX as f64 {
        return Err(unsupported(path, "positive whole number is required"));
    }
    Ok(number as u32)
}

fn convert_steps(value: &Value, path: &str, depth: usize, order: &mut u32) -> R<Vec<CyclingStep>> {
    if depth > WORKOUT_REPEAT_DEPTH_MAX {
        return Err(unsupported(path, "repeat nesting exceeds player support"));
    }
    let steps = value
        .as_array()
        .ok_or_else(|| unsupported(path, "steps are required"))?;
    steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let path = format!("{path}[{index}]");
            let step_order = positive_whole(&step["stepOrder"], &path)?;
            if step_order <= *order {
                return Err(unsupported(&path, "step ordering is ambiguous"));
            }
            *order = step_order;
            reject_extra_semantics(step, &path)?;
            match step["type"].as_str() {
                Some("RepeatGroupDTO") => {
                    if !descriptor(
                        &step["endCondition"],
                        "conditionTypeId",
                        "conditionTypeKey",
                        REPEAT_CONDITION_ID,
                        "iterations",
                    ) || step["smartRepeat"].as_bool().unwrap_or(false)
                        || step["skipLastRestStep"].as_bool().unwrap_or(false)
                    {
                        return Err(unsupported(
                            &path,
                            "only fixed repeats without skipped rests are supported",
                        ));
                    }
                    for field in ["smartRepeat", "skipLastRestStep"] {
                        if !step[field].is_null() && !step[field].is_boolean() {
                            return Err(unsupported(&path, "invalid repeat behavior"));
                        }
                    }
                    if !optional_text(&step["description"], &path)?.is_empty()
                        || !target_is_open(step, false)?
                        || !target_is_open(step, true)?
                    {
                        return Err(unsupported(
                            &path,
                            "repeat groups cannot also prescribe targets or instructions",
                        ));
                    }
                    let count = positive_whole(&step["numberOfIterations"], &path)?;
                    if positive_whole(&step["endConditionValue"], &path)? != count {
                        return Err(unsupported(&path, "repeat counts disagree"));
                    }
                    Ok(CyclingStep::Repeat {
                        count,
                        steps: convert_steps(
                            &step["workoutSteps"],
                            &format!("{path}.workoutSteps"),
                            depth + 1,
                            order,
                        )?,
                    })
                }
                Some("ExecutableStepDTO") => {
                    if !step["numberOfIterations"].is_null()
                        || ["smartRepeat", "skipLastRestStep"].iter().any(|field| {
                            !step[*field].is_null() && step[*field].as_bool() != Some(false)
                        })
                    {
                        return Err(unsupported(&path, "leaf step also defines repeat behavior"));
                    }
                    if !descriptor(
                        &step["endCondition"],
                        "conditionTypeId",
                        "conditionTypeKey",
                        TIME_CONDITION_ID,
                        "time",
                    ) {
                        return Err(unsupported(&path, "only time-ended steps are supported"));
                    }
                    if step
                        .get("workoutSteps")
                        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
                    {
                        return Err(unsupported(&path, "leaf step contains nested instructions"));
                    }
                    let duration_seconds = positive_whole(&step["endConditionValue"], &path)?;
                    let mut power = None;
                    let mut cadence = None;
                    for secondary in [false, true] {
                        if target_is_open(step, secondary)? {
                            continue;
                        }
                        let (kind, one, two, zone, unit) = target_fields(secondary);
                        let target = &step[kind];
                        if !step[zone].is_null() && step[zone].as_u64() != Some(0) {
                            return Err(unsupported(
                                &path,
                                "zone numbers require a provider zone lookup",
                            ));
                        }
                        let low = step[one]
                            .as_f64()
                            .ok_or_else(|| unsupported(&path, "target range is required"))?;
                        let high = step[two]
                            .as_f64()
                            .ok_or_else(|| unsupported(&path, "target range is required"))?;
                        if !low.is_finite() || !high.is_finite() || low > high {
                            return Err(unsupported(
                                &path,
                                "target range must be finite and ordered",
                            ));
                        }
                        if descriptor(
                            target,
                            "workoutTargetTypeId",
                            "workoutTargetTypeKey",
                            POWER_TARGET_ID,
                            "power.zone",
                        ) {
                            if power.is_some() {
                                return Err(unsupported(
                                    &path,
                                    "multiple power targets are ambiguous",
                                ));
                            }
                            // Garmin custom power is watts when targetValueUnit is
                            // null. Percent FTP uses an explicit percent unit object.
                            // Provenance: github.com/elias-ramzi/ClaudeCyclingMCP/blob/main/docs/garmin-schema.md
                            power = Some(if step[unit].is_null() {
                                let min_watts = whole_u16(low, &path)?;
                                let max_watts = whole_u16(high, &path)?;
                                if min_watts == max_watts {
                                    CyclingPowerTarget::Watts { watts: min_watts }
                                } else {
                                    CyclingPowerTarget::WattsRange {
                                        min_watts,
                                        max_watts,
                                    }
                                }
                            } else if descriptor(
                                &step[unit],
                                "unitId",
                                "unitKey",
                                PERCENT_UNIT_ID,
                                "percent",
                            ) && step[unit]["factor"].as_f64() == Some(UNIT_FACTOR)
                            {
                                if low == high {
                                    CyclingPowerTarget::PercentFtp { percent: low }
                                } else {
                                    CyclingPowerTarget::PercentFtpRange {
                                        min_percent: low,
                                        max_percent: high,
                                    }
                                }
                            } else {
                                return Err(unsupported(&path, "unsupported power target unit"));
                            });
                        } else if descriptor(
                            target,
                            "workoutTargetTypeId",
                            "workoutTargetTypeKey",
                            CADENCE_TARGET_ID,
                            "cadence",
                        ) {
                            if cadence.is_some() || !step[unit].is_null() {
                                return Err(unsupported(
                                    &path,
                                    "ambiguous cadence target or unsupported unit",
                                ));
                            }
                            let min_rpm = whole_u16(low, &path)?;
                            let max_rpm = whole_u16(high, &path)?;
                            cadence = Some(if min_rpm == max_rpm {
                                CyclingCadenceTarget::Exact { rpm: min_rpm }
                            } else {
                                CyclingCadenceTarget::Range { min_rpm, max_rpm }
                            });
                        } else {
                            return Err(unsupported(
                                &path,
                                "only explicit power and cadence targets are supported",
                            ));
                        }
                    }
                    let message = optional_text(&step["description"], &path)?;
                    let cues = if message.is_empty() {
                        vec![]
                    } else {
                        vec![WorkoutCue {
                            offset_seconds: 0,
                            message: message.into(),
                            display_seconds: TEXT_EVENT_DEFAULT_S,
                        }]
                    };
                    Ok(match power {
                        Some(power) => CyclingStep::Steady {
                            duration_seconds,
                            power,
                            cadence,
                            cues,
                        },
                        None => CyclingStep::FreeRide {
                            duration_seconds,
                            cadence,
                            cues,
                        },
                    })
                }
                _ => Err(unsupported(&path, "unknown step type")),
            }
        })
        .collect()
}

fn whole_u16(value: f64, path: &str) -> R<u16> {
    if value < 0.0 || value > u16::MAX as f64 || value.fract() != 0.0 {
        return Err(unsupported(path, "target requires an unsigned whole value"));
    }
    Ok(value as u16)
}

fn target_fields(
    secondary: bool,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    if secondary {
        (
            "secondaryTargetType",
            "secondaryTargetValueOne",
            "secondaryTargetValueTwo",
            "secondaryZoneNumber",
            "secondaryTargetValueUnit",
        )
    } else {
        (
            "targetType",
            "targetValueOne",
            "targetValueTwo",
            "zoneNumber",
            "targetValueUnit",
        )
    }
}

fn target_is_open(step: &Value, secondary: bool) -> R<bool> {
    let (kind, one, two, zone, unit) = target_fields(secondary);
    let target = &step[kind];
    if target.is_null()
        || descriptor(
            target,
            "workoutTargetTypeId",
            "workoutTargetTypeKey",
            OPEN_TARGET_ID,
            "no.target",
        )
    {
        if [one, two, zone, unit]
            .iter()
            .any(|field| !step[*field].is_null() && step[*field].as_f64() != Some(0.0))
        {
            return Err(unsupported(kind, "open target also contains target values"));
        }
        return Ok(true);
    }
    Ok(false)
}

fn reject_extra_semantics(step: &Value, path: &str) -> R<()> {
    // Garmin's web editor includes neutral descriptors on cycling steps.
    // A nonzero ID is a prescription and must not be silently discarded.
    for (field, id_field) in [
        ("equipmentType", "equipmentTypeId"),
        ("strokeType", "strokeTypeId"),
    ] {
        if !step[field].is_null()
            && step[field][id_field].as_u64() != Some(NO_EQUIPMENT_OR_STROKE_ID)
        {
            return Err(unsupported(
                &format!("{path}.{field}"),
                "prescribed equipment or stroke is not supported",
            ));
        }
    }
    // Presentation fields can vary without changing execution. Any unfamiliar
    // non-null step field needs review rather than silently losing a prescription.
    const FIELDS: &[&str] = &[
        "type",
        "stepId",
        "stepOrder",
        "stepType",
        "endCondition",
        "endConditionValue",
        "targetType",
        "targetValueOne",
        "targetValueTwo",
        "zoneNumber",
        "targetValueUnit",
        "secondaryTargetType",
        "secondaryTargetValueOne",
        "secondaryTargetValueTwo",
        "secondaryZoneNumber",
        "secondaryTargetValueUnit",
        "description",
        "numberOfIterations",
        "workoutSteps",
        "smartRepeat",
        "skipLastRestStep",
        "childStepId",
        "preferredEndConditionUnit",
        "equipmentType",
        "strokeType",
    ];
    let object = step
        .as_object()
        .ok_or_else(|| unsupported(path, "step must be an object"))?;
    if let Some((name, _)) = object
        .iter()
        .find(|(key, value)| !FIELDS.contains(&key.as_str()) && !value.is_null())
    {
        return Err(unsupported(
            &format!("{path}.{name}"),
            "unrecognized step semantics",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
