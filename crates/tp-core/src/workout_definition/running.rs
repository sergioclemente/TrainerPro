//! Running intent shared with Harness; it is not a treadmill/player model.

use serde::{Deserialize, Serialize};

use super::{invalid, validate_cues, DefinitionStats, WorkoutCue, WorkoutDefinitionError};
use crate::consts::{WORKOUT_REPEAT_COUNT_MAX, WORKOUT_REPEAT_DEPTH_MAX};

/// Representation guardrails, not recommended training targets.
const HEART_RATE_BPM_MAX: u16 = 300;
const RUNNING_CADENCE_STEPS_PER_MINUTE_MAX: u16 = 400;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunningStep {
    Step {
        end: RunningEnd,
        #[serde(default)]
        target: RunningTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cadence: Option<RunningCadenceTarget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<RunningRole>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cues: Vec<WorkoutCue>,
    },
    Repeat {
        count: u32,
        steps: Vec<RunningStep>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunningEnd {
    Time { duration_seconds: u32 },
    Distance { distance_meters: u32 },
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunningTarget {
    #[default]
    Open,
    Pace {
        seconds_per_km: f64,
    },
    PaceRange {
        fast_seconds_per_km: f64,
        slow_seconds_per_km: f64,
    },
    HeartRate {
        bpm: u16,
    },
    HeartRateRange {
        min_bpm: u16,
        max_bpm: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunningCadenceTarget {
    Exact {
        steps_per_minute: u16,
    },
    Range {
        min_steps_per_minute: u16,
        max_steps_per_minute: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunningRole {
    Warmup,
    Work,
    Recovery,
    Cooldown,
    Rest,
}

pub(super) fn validate_steps(
    steps: &[RunningStep],
    path: &str,
    repeat_depth: usize,
) -> Result<DefinitionStats, WorkoutDefinitionError> {
    if steps.is_empty() {
        return invalid(path, "must contain at least one step");
    }
    let mut total = DefinitionStats {
        duration_seconds: 0,
        distance_meters: 0,
        executable_segments: 0,
    };
    for (index, step) in steps.iter().enumerate() {
        let path = format!("{path}[{index}]");
        let stats = match step {
            RunningStep::Step {
                end,
                target,
                cadence,
                cues,
                ..
            } => {
                let (duration_seconds, distance_meters) = match end {
                    RunningEnd::Time { duration_seconds } => {
                        if *duration_seconds == 0 {
                            return invalid(
                                format!("{path}.end.duration_seconds"),
                                "must be at least 1",
                            );
                        }
                        validate_cues(cues, Some(*duration_seconds), &path)?;
                        (u64::from(*duration_seconds), 0)
                    }
                    RunningEnd::Distance { distance_meters } => {
                        if *distance_meters == 0 {
                            return invalid(
                                format!("{path}.end.distance_meters"),
                                "must be at least 1",
                            );
                        }
                        // A start cue is valid without assuming a duration or speed.
                        validate_cues(cues, None, &path)?;
                        (0, u64::from(*distance_meters))
                    }
                };
                validate_target(target, &format!("{path}.target"))?;
                if let Some(cadence) = cadence {
                    validate_cadence(cadence, &format!("{path}.cadence"))?;
                }
                DefinitionStats {
                    duration_seconds,
                    distance_meters,
                    executable_segments: 1,
                }
            }
            RunningStep::Repeat { count, steps } => {
                if *count == 0 || *count > WORKOUT_REPEAT_COUNT_MAX {
                    return invalid(
                        format!("{path}.count"),
                        format!("must be between 1 and {WORKOUT_REPEAT_COUNT_MAX}"),
                    );
                }
                if repeat_depth >= WORKOUT_REPEAT_DEPTH_MAX {
                    return invalid(
                        &path,
                        format!("repeat nesting must not exceed {WORKOUT_REPEAT_DEPTH_MAX} levels"),
                    );
                }
                let inner = validate_steps(steps, &format!("{path}.steps"), repeat_depth + 1)?;
                let scale = |value: u64| {
                    value.checked_mul(u64::from(*count)).ok_or_else(|| {
                        WorkoutDefinitionError::Invalid {
                            path: path.clone(),
                            message: "expanded prescription overflows".into(),
                        }
                    })
                };
                DefinitionStats {
                    duration_seconds: scale(inner.duration_seconds)?,
                    distance_meters: scale(inner.distance_meters)?,
                    executable_segments: scale(inner.executable_segments)?,
                }
            }
        };
        let add = |a: u64, b: u64| {
            a.checked_add(b)
                .ok_or_else(|| WorkoutDefinitionError::Invalid {
                    path: path.clone(),
                    message: "expanded prescription overflows".into(),
                })
        };
        total.duration_seconds = add(total.duration_seconds, stats.duration_seconds)?;
        total.distance_meters = add(total.distance_meters, stats.distance_meters)?;
        total.executable_segments = add(total.executable_segments, stats.executable_segments)?;
    }
    Ok(total)
}

fn validate_target(target: &RunningTarget, path: &str) -> Result<(), WorkoutDefinitionError> {
    match target {
        RunningTarget::Open => Ok(()),
        RunningTarget::Pace { seconds_per_km } => validate_pace(*seconds_per_km, path),
        RunningTarget::PaceRange {
            fast_seconds_per_km,
            slow_seconds_per_km,
        } => {
            validate_pace(*fast_seconds_per_km, path)?;
            validate_pace(*slow_seconds_per_km, path)?;
            if fast_seconds_per_km > slow_seconds_per_km {
                return invalid(path, "fast pace seconds must not exceed slow pace seconds");
            }
            Ok(())
        }
        RunningTarget::HeartRate { bpm } => validate_bpm(*bpm, path),
        RunningTarget::HeartRateRange { min_bpm, max_bpm } => {
            validate_bpm(*min_bpm, path)?;
            validate_bpm(*max_bpm, path)?;
            if min_bpm > max_bpm {
                return invalid(
                    path,
                    "minimum heart rate must not exceed maximum heart rate",
                );
            }
            Ok(())
        }
    }
}

fn validate_pace(seconds_per_km: f64, path: &str) -> Result<(), WorkoutDefinitionError> {
    if !seconds_per_km.is_finite() || seconds_per_km <= 0.0 {
        return invalid(path, "pace must be positive finite seconds per kilometer");
    }
    Ok(())
}

fn validate_bpm(bpm: u16, path: &str) -> Result<(), WorkoutDefinitionError> {
    if bpm == 0 || bpm > HEART_RATE_BPM_MAX {
        return invalid(
            path,
            format!("heart rate must be between 1 and {HEART_RATE_BPM_MAX} bpm"),
        );
    }
    Ok(())
}

fn validate_cadence(
    cadence: &RunningCadenceTarget,
    path: &str,
) -> Result<(), WorkoutDefinitionError> {
    let (min, max) = match cadence {
        RunningCadenceTarget::Exact { steps_per_minute } => (*steps_per_minute, *steps_per_minute),
        RunningCadenceTarget::Range {
            min_steps_per_minute,
            max_steps_per_minute,
        } => (*min_steps_per_minute, *max_steps_per_minute),
    };
    if min == 0 || max > RUNNING_CADENCE_STEPS_PER_MINUTE_MAX || min > max {
        return invalid(path, format!("cadence must be an ordered range within 1–{RUNNING_CADENCE_STEPS_PER_MINUTE_MAX} steps/minute (both feet)"));
    }
    Ok(())
}
