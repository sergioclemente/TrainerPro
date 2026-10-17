//! WorkoutSpec documents, validation, and cycling compilation.
//!
//! A definition preserves authoring and provider semantics. Compilation
//! produces the flat, cycling-specific model consumed by the existing player.

use serde::{Deserialize, Serialize};

use crate::consts::{TEXT_EVENT_DEFAULT_S, WORKOUT_EXECUTABLE_SEGMENTS_MAX};

mod compile;
mod cycling;
pub mod running;

pub(crate) use compile::midpoint;
pub use cycling::{CyclingCadenceTarget, CyclingPowerTarget, CyclingStep};
pub use running::{RunningCadenceTarget, RunningEnd, RunningRole, RunningStep, RunningTarget};

pub const WORKOUT_SPEC_FORMAT_NAME: &str = "WorkoutSpec";
pub const WORKOUT_SPEC_VERSION: u32 = 1;
const LEGACY_FORMAT_NAME: &str = "TPW";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkoutSpec {
    pub format: WorkoutFormat,
    pub version: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_focus: Option<String>,
    pub prescription: WorkoutPrescription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkoutFormat {
    #[serde(rename = "WorkoutSpec", alias = "TPW")]
    WorkoutSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "sport", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkoutPrescription {
    Cycling { steps: Vec<CyclingStep> },
    Running { steps: Vec<RunningStep> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkoutCue {
    /// Offset from the start of the containing leaf step.
    pub offset_seconds: u32,
    pub message: String,
    #[serde(default = "default_cue_display_seconds")]
    pub display_seconds: u32,
}

fn default_cue_display_seconds() -> u32 {
    TEXT_EVENT_DEFAULT_S
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkoutSpecError {
    #[error("invalid workout definition JSON: {message}")]
    InvalidJson { message: String },
    #[error("unsupported workout format {found:?}; supported format is WorkoutSpec")]
    UnsupportedFormat { found: String },
    #[error("unsupported WorkoutSpec version {found}; supported version is {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("the cycling player cannot execute a {sport} prescription")]
    UnsupportedExecutionSport { sport: String },
    #[error("duration is unknown because the prescription contains distance-ended steps")]
    UnknownDuration,
    #[error("invalid workout definition at {path}: {message}")]
    Invalid { path: String, message: String },
}

#[derive(Debug, Clone, Copy)]
struct DefinitionStats {
    duration_seconds: u64,
    distance_meters: u64,
    expanded_steps: u64,
}

#[derive(Deserialize)]
struct DefinitionHeader {
    format: String,
    version: u32,
}

impl WorkoutSpec {
    pub fn from_json(json: &str) -> Result<Self, WorkoutSpecError> {
        let header: DefinitionHeader =
            serde_json::from_str(json).map_err(|error| WorkoutSpecError::InvalidJson {
                message: error.to_string(),
            })?;
        if header.format != WORKOUT_SPEC_FORMAT_NAME && header.format != LEGACY_FORMAT_NAME {
            return Err(WorkoutSpecError::UnsupportedFormat {
                found: header.format,
            });
        }
        if header.version != WORKOUT_SPEC_VERSION {
            return Err(WorkoutSpecError::UnsupportedVersion {
                found: header.version,
                supported: WORKOUT_SPEC_VERSION,
            });
        }
        let definition: Self =
            serde_json::from_str(json).map_err(|error| WorkoutSpecError::InvalidJson {
                message: error.to_string(),
            })?;
        definition.validate()?;
        Ok(definition)
    }

    pub fn to_json_pretty(&self) -> Result<String, WorkoutSpecError> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|error| WorkoutSpecError::InvalidJson {
            message: error.to_string(),
        })
    }

    pub fn validate(&self) -> Result<(), WorkoutSpecError> {
        self.validated_stats().map(|_| ())
    }

    fn validated_stats(&self) -> Result<DefinitionStats, WorkoutSpecError> {
        if self.version != WORKOUT_SPEC_VERSION {
            return Err(WorkoutSpecError::UnsupportedVersion {
                found: self.version,
                supported: WORKOUT_SPEC_VERSION,
            });
        }
        if self.title.trim().is_empty() {
            return invalid("title", "must not be blank");
        }
        if self
            .training_focus
            .as_deref()
            .is_some_and(|focus| focus.trim().is_empty())
        {
            return invalid("training_focus", "must not be blank when present");
        }

        let stats = match &self.prescription {
            WorkoutPrescription::Cycling { steps } => {
                cycling::validate_steps(steps, "prescription.steps", 0)?
            }
            WorkoutPrescription::Running { steps } => {
                running::validate_steps(steps, "prescription.steps", 0)?
            }
        };
        if stats.duration_seconds > u64::from(u32::MAX) {
            return invalid(
                "prescription.steps",
                "compiled duration exceeds the supported range",
            );
        }
        if stats.expanded_steps > WORKOUT_EXECUTABLE_SEGMENTS_MAX {
            return invalid(
                "prescription.steps",
                format!(
                    "expands to more than {WORKOUT_EXECUTABLE_SEGMENTS_MAX} executable segments"
                ),
            );
        }
        if stats.distance_meters > u64::from(u32::MAX) {
            return invalid(
                "prescription.steps",
                "expanded distance exceeds the supported range",
            );
        }
        Ok(stats)
    }

    pub fn duration_seconds(&self) -> Result<u32, WorkoutSpecError> {
        let stats = self.validated_stats()?;
        if stats.distance_meters > 0 {
            return Err(WorkoutSpecError::UnknownDuration);
        }
        Ok(stats.duration_seconds as u32)
    }
}

fn invalid<T>(path: impl Into<String>, message: impl Into<String>) -> Result<T, WorkoutSpecError> {
    Err(WorkoutSpecError::Invalid {
        path: path.into(),
        message: message.into(),
    })
}

fn validate_cues(
    cues: &[WorkoutCue],
    duration_seconds: Option<u32>,
    path: &str,
) -> Result<(), WorkoutSpecError> {
    for (index, cue) in cues.iter().enumerate() {
        let cue_path = format!("{path}.cues[{index}]");
        if duration_seconds.is_some_and(|duration| cue.offset_seconds >= duration) {
            return invalid(
                format!("{cue_path}.offset_seconds"),
                "must fall within its step",
            );
        }
        if duration_seconds.is_none() && cue.offset_seconds != 0 {
            return invalid(
                format!("{cue_path}.offset_seconds"),
                "distance-ended steps support step-start cues only",
            );
        }
        if cue.message.trim().is_empty() {
            return invalid(format!("{cue_path}.message"), "must not be blank");
        }
        if cue.display_seconds == 0 {
            return invalid(format!("{cue_path}.display_seconds"), "must be at least 1");
        }
    }
    Ok(())
}
