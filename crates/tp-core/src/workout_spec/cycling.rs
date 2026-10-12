//! Cycling prescriptions and their validation.

use serde::{Deserialize, Serialize};

use super::{invalid, validate_cues, DefinitionStats, WorkoutCue, WorkoutSpecError};
use crate::consts::{
    MAX_TARGET_WATTS, POWER_FRACTION_MAX, POWER_FRACTION_MIN, WORKOUT_CADENCE_RPM_MAX,
    WORKOUT_REPEAT_COUNT_MAX, WORKOUT_REPEAT_DEPTH_MAX,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CyclingStep {
    Steady {
        duration_seconds: u32,
        power: CyclingPowerTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cadence: Option<CyclingCadenceTarget>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cues: Vec<WorkoutCue>,
    },
    Ramp {
        duration_seconds: u32,
        start_power: CyclingPowerTarget,
        end_power: CyclingPowerTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cadence: Option<CyclingCadenceTarget>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cues: Vec<WorkoutCue>,
    },
    FreeRide {
        duration_seconds: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cadence: Option<CyclingCadenceTarget>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cues: Vec<WorkoutCue>,
    },
    Repeat {
        count: u32,
        steps: Vec<CyclingStep>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CyclingPowerTarget {
    PercentFtp { percent: f64 },
    Watts { watts: u16 },
    PercentFtpRange { min_percent: f64, max_percent: f64 },
    WattsRange { min_watts: u16, max_watts: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CyclingCadenceTarget {
    Exact { rpm: u16 },
    Range { min_rpm: u16, max_rpm: u16 },
}

pub(super) fn validate_steps(
    steps: &[CyclingStep],
    path: &str,
    repeat_depth: usize,
) -> Result<DefinitionStats, WorkoutSpecError> {
    if steps.is_empty() {
        return invalid(path, "must contain at least one step");
    }

    let mut total = DefinitionStats {
        duration_seconds: 0,
        distance_meters: 0,
        expanded_steps: 0,
    };
    for (index, step) in steps.iter().enumerate() {
        let step_path = format!("{path}[{index}]");
        let stats = validate_step(step, &step_path, repeat_depth)?;
        total.duration_seconds = total
            .duration_seconds
            .checked_add(stats.duration_seconds)
            .ok_or_else(|| WorkoutSpecError::Invalid {
                path: path.into(),
                message: "duration overflows".into(),
            })?;
        total.expanded_steps = total
            .expanded_steps
            .checked_add(stats.expanded_steps)
            .ok_or_else(|| WorkoutSpecError::Invalid {
                path: path.into(),
                message: "expanded segment count overflows".into(),
            })?;
    }
    Ok(total)
}

fn validate_step(
    step: &CyclingStep,
    path: &str,
    repeat_depth: usize,
) -> Result<DefinitionStats, WorkoutSpecError> {
    match step {
        CyclingStep::Steady {
            duration_seconds,
            power,
            cadence,
            cues,
        } => {
            validate_leaf(*duration_seconds, power, cadence.as_ref(), cues, path)?;
            Ok(leaf_stats(*duration_seconds))
        }
        CyclingStep::Ramp {
            duration_seconds,
            start_power,
            end_power,
            cadence,
            cues,
        } => {
            validate_duration(*duration_seconds, path)?;
            validate_power(start_power, &format!("{path}.start_power"))?;
            validate_power(end_power, &format!("{path}.end_power"))?;
            if let Some(cadence) = cadence {
                validate_cadence(cadence, &format!("{path}.cadence"))?;
            }
            validate_cues(cues, Some(*duration_seconds), path)?;
            Ok(leaf_stats(*duration_seconds))
        }
        CyclingStep::FreeRide {
            duration_seconds,
            cadence,
            cues,
        } => {
            validate_duration(*duration_seconds, path)?;
            if let Some(cadence) = cadence {
                validate_cadence(cadence, &format!("{path}.cadence"))?;
            }
            validate_cues(cues, Some(*duration_seconds), path)?;
            Ok(leaf_stats(*duration_seconds))
        }
        CyclingStep::Repeat { count, steps } => {
            if *count == 0 || *count > WORKOUT_REPEAT_COUNT_MAX {
                return invalid(
                    format!("{path}.count"),
                    format!("must be between 1 and {WORKOUT_REPEAT_COUNT_MAX}"),
                );
            }
            if repeat_depth >= WORKOUT_REPEAT_DEPTH_MAX {
                return invalid(
                    path,
                    format!("repeat nesting must not exceed {WORKOUT_REPEAT_DEPTH_MAX} levels"),
                );
            }
            let inner = validate_steps(steps, &format!("{path}.steps"), repeat_depth + 1)?;
            let duration_seconds = inner
                .duration_seconds
                .checked_mul(u64::from(*count))
                .ok_or_else(|| WorkoutSpecError::Invalid {
                    path: path.into(),
                    message: "duration overflows".into(),
                })?;
            let expanded_steps = inner
                .expanded_steps
                .checked_mul(u64::from(*count))
                .ok_or_else(|| WorkoutSpecError::Invalid {
                    path: path.into(),
                    message: "expanded segment count overflows".into(),
                })?;
            Ok(DefinitionStats {
                duration_seconds,
                distance_meters: 0,
                expanded_steps,
            })
        }
    }
}

fn leaf_stats(duration_seconds: u32) -> DefinitionStats {
    DefinitionStats {
        duration_seconds: u64::from(duration_seconds),
        distance_meters: 0,
        expanded_steps: 1,
    }
}

fn validate_leaf(
    duration_seconds: u32,
    power: &CyclingPowerTarget,
    cadence: Option<&CyclingCadenceTarget>,
    cues: &[WorkoutCue],
    path: &str,
) -> Result<(), WorkoutSpecError> {
    validate_duration(duration_seconds, path)?;
    validate_power(power, &format!("{path}.power"))?;
    if let Some(cadence) = cadence {
        validate_cadence(cadence, &format!("{path}.cadence"))?;
    }
    validate_cues(cues, Some(duration_seconds), path)
}

fn validate_duration(duration_seconds: u32, path: &str) -> Result<(), WorkoutSpecError> {
    if duration_seconds == 0 {
        return invalid(format!("{path}.duration_seconds"), "must be at least 1");
    }
    Ok(())
}

fn validate_power(target: &CyclingPowerTarget, path: &str) -> Result<(), WorkoutSpecError> {
    match target {
        CyclingPowerTarget::PercentFtp { percent } => {
            validate_percent(*percent, &format!("{path}.percent"))
        }
        CyclingPowerTarget::Watts { watts } => validate_watts(*watts, &format!("{path}.watts")),
        CyclingPowerTarget::PercentFtpRange {
            min_percent,
            max_percent,
        } => {
            validate_percent(*min_percent, &format!("{path}.min_percent"))?;
            validate_percent(*max_percent, &format!("{path}.max_percent"))?;
            if min_percent > max_percent {
                return invalid(path, "minimum percent must not exceed maximum percent");
            }
            Ok(())
        }
        CyclingPowerTarget::WattsRange {
            min_watts,
            max_watts,
        } => {
            validate_watts(*min_watts, &format!("{path}.min_watts"))?;
            validate_watts(*max_watts, &format!("{path}.max_watts"))?;
            if min_watts > max_watts {
                return invalid(path, "minimum watts must not exceed maximum watts");
            }
            Ok(())
        }
    }
}

fn validate_percent(percent: f64, path: &str) -> Result<(), WorkoutSpecError> {
    let min_percent = POWER_FRACTION_MIN * 100.0;
    let max_percent = POWER_FRACTION_MAX * 100.0;
    if !percent.is_finite() || percent < min_percent || percent > max_percent {
        return invalid(
            path,
            format!("must be between {min_percent:.0} and {max_percent:.0} percent of FTP"),
        );
    }
    Ok(())
}

fn validate_watts(watts: u16, path: &str) -> Result<(), WorkoutSpecError> {
    if watts > MAX_TARGET_WATTS {
        return invalid(path, format!("must not exceed {MAX_TARGET_WATTS} watts"));
    }
    Ok(())
}

fn validate_cadence(cadence: &CyclingCadenceTarget, path: &str) -> Result<(), WorkoutSpecError> {
    match cadence {
        CyclingCadenceTarget::Exact { rpm } => validate_rpm(*rpm, &format!("{path}.rpm")),
        CyclingCadenceTarget::Range { min_rpm, max_rpm } => {
            validate_rpm(*min_rpm, &format!("{path}.min_rpm"))?;
            validate_rpm(*max_rpm, &format!("{path}.max_rpm"))?;
            if min_rpm > max_rpm {
                return invalid(path, "minimum cadence must not exceed maximum cadence");
            }
            Ok(())
        }
    }
}

fn validate_rpm(rpm: u16, path: &str) -> Result<(), WorkoutSpecError> {
    if rpm == 0 || rpm > WORKOUT_CADENCE_RPM_MAX {
        return invalid(
            path,
            format!("must be between 1 and {WORKOUT_CADENCE_RPM_MAX} rpm"),
        );
    }
    Ok(())
}
