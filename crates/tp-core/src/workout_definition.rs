//! TrainerPro Workout (TPW) definitions and their pure compiler.
//!
//! A definition preserves authoring and provider semantics. Compilation
//! produces the flat, cycling-specific model consumed by the existing player.

use serde::{Deserialize, Serialize};

use crate::consts::{
    MAX_TARGET_WATTS, POWER_FRACTION_MAX, POWER_FRACTION_MIN, TEXT_EVENT_DEFAULT_S,
    WORKOUT_CADENCE_RPM_MAX, WORKOUT_EXECUTABLE_SEGMENTS_MAX, WORKOUT_REPEAT_COUNT_MAX,
    WORKOUT_REPEAT_DEPTH_MAX,
};
use crate::model::{ExecutableWorkout, PowerTarget, Segment, TextEvent};

pub const TPW_FORMAT_NAME: &str = "TPW";
pub const TPW_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkoutDefinition {
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
    #[serde(rename = "TPW")]
    Tpw,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "sport", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkoutPrescription {
    Cycling { steps: Vec<CyclingStep> },
}

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
pub enum WorkoutDefinitionError {
    #[error("invalid workout definition JSON: {message}")]
    InvalidJson { message: String },
    #[error("unsupported workout format {found:?}; supported format is TPW")]
    UnsupportedFormat { found: String },
    #[error("unsupported TPW version {found}; supported version is {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("invalid workout definition at {path}: {message}")]
    Invalid { path: String, message: String },
}

#[derive(Debug, Clone, Copy)]
struct DefinitionStats {
    duration_seconds: u64,
    executable_segments: u64,
}

#[derive(Deserialize)]
struct DefinitionHeader {
    format: String,
    version: u32,
}

impl WorkoutDefinition {
    pub fn from_json(json: &str) -> Result<Self, WorkoutDefinitionError> {
        let header: DefinitionHeader =
            serde_json::from_str(json).map_err(|error| WorkoutDefinitionError::InvalidJson {
                message: error.to_string(),
            })?;
        if header.format != TPW_FORMAT_NAME {
            return Err(WorkoutDefinitionError::UnsupportedFormat {
                found: header.format,
            });
        }
        if header.version != TPW_VERSION {
            return Err(WorkoutDefinitionError::UnsupportedVersion {
                found: header.version,
                supported: TPW_VERSION,
            });
        }
        let definition: Self =
            serde_json::from_str(json).map_err(|error| WorkoutDefinitionError::InvalidJson {
                message: error.to_string(),
            })?;
        definition.validate()?;
        Ok(definition)
    }

    pub fn to_json_pretty(&self) -> Result<String, WorkoutDefinitionError> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|error| WorkoutDefinitionError::InvalidJson {
            message: error.to_string(),
        })
    }

    pub fn validate(&self) -> Result<(), WorkoutDefinitionError> {
        self.validated_stats().map(|_| ())
    }

    fn validated_stats(&self) -> Result<DefinitionStats, WorkoutDefinitionError> {
        if self.version != TPW_VERSION {
            return Err(WorkoutDefinitionError::UnsupportedVersion {
                found: self.version,
                supported: TPW_VERSION,
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

        let WorkoutPrescription::Cycling { steps } = &self.prescription;
        let stats = validate_steps(steps, "prescription.steps", 0)?;
        if stats.duration_seconds > u64::from(u32::MAX) {
            return invalid(
                "prescription.steps",
                "compiled duration exceeds the supported range",
            );
        }
        if stats.executable_segments > WORKOUT_EXECUTABLE_SEGMENTS_MAX {
            return invalid(
                "prescription.steps",
                format!(
                    "expands to more than {WORKOUT_EXECUTABLE_SEGMENTS_MAX} executable segments"
                ),
            );
        }
        Ok(stats)
    }

    pub fn duration_seconds(&self) -> Result<u32, WorkoutDefinitionError> {
        let stats = self.validated_stats()?;
        Ok(stats.duration_seconds as u32)
    }

    pub fn compile(&self) -> Result<ExecutableWorkout, WorkoutDefinitionError> {
        self.validated_stats()?;

        let mut segments = Vec::new();
        let mut text_events = Vec::new();
        let mut offset_seconds = 0u32;
        let WorkoutPrescription::Cycling { steps } = &self.prescription;
        compile_steps(steps, &mut offset_seconds, &mut segments, &mut text_events);
        text_events.sort_by_key(|cue| cue.offset_s);

        Ok(ExecutableWorkout {
            name: self.title.clone(),
            description: self.description.clone(),
            segments,
            text_events,
        })
    }
}

fn invalid<T>(
    path: impl Into<String>,
    message: impl Into<String>,
) -> Result<T, WorkoutDefinitionError> {
    Err(WorkoutDefinitionError::Invalid {
        path: path.into(),
        message: message.into(),
    })
}

fn validate_steps(
    steps: &[CyclingStep],
    path: &str,
    repeat_depth: usize,
) -> Result<DefinitionStats, WorkoutDefinitionError> {
    if steps.is_empty() {
        return invalid(path, "must contain at least one step");
    }

    let mut total = DefinitionStats {
        duration_seconds: 0,
        executable_segments: 0,
    };
    for (index, step) in steps.iter().enumerate() {
        let step_path = format!("{path}[{index}]");
        let stats = validate_step(step, &step_path, repeat_depth)?;
        total.duration_seconds = total
            .duration_seconds
            .checked_add(stats.duration_seconds)
            .ok_or_else(|| WorkoutDefinitionError::Invalid {
                path: path.into(),
                message: "duration overflows".into(),
            })?;
        total.executable_segments = total
            .executable_segments
            .checked_add(stats.executable_segments)
            .ok_or_else(|| WorkoutDefinitionError::Invalid {
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
) -> Result<DefinitionStats, WorkoutDefinitionError> {
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
            validate_cues(cues, *duration_seconds, path)?;
            Ok(leaf_stats(*duration_seconds))
        }
        CyclingStep::FreeRide {
            duration_seconds,
            cues,
        } => {
            validate_duration(*duration_seconds, path)?;
            validate_cues(cues, *duration_seconds, path)?;
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
                .ok_or_else(|| WorkoutDefinitionError::Invalid {
                    path: path.into(),
                    message: "duration overflows".into(),
                })?;
            let executable_segments = inner
                .executable_segments
                .checked_mul(u64::from(*count))
                .ok_or_else(|| WorkoutDefinitionError::Invalid {
                    path: path.into(),
                    message: "expanded segment count overflows".into(),
                })?;
            Ok(DefinitionStats {
                duration_seconds,
                executable_segments,
            })
        }
    }
}

fn leaf_stats(duration_seconds: u32) -> DefinitionStats {
    DefinitionStats {
        duration_seconds: u64::from(duration_seconds),
        executable_segments: 1,
    }
}

fn validate_leaf(
    duration_seconds: u32,
    power: &CyclingPowerTarget,
    cadence: Option<&CyclingCadenceTarget>,
    cues: &[WorkoutCue],
    path: &str,
) -> Result<(), WorkoutDefinitionError> {
    validate_duration(duration_seconds, path)?;
    validate_power(power, &format!("{path}.power"))?;
    if let Some(cadence) = cadence {
        validate_cadence(cadence, &format!("{path}.cadence"))?;
    }
    validate_cues(cues, duration_seconds, path)
}

fn validate_duration(duration_seconds: u32, path: &str) -> Result<(), WorkoutDefinitionError> {
    if duration_seconds == 0 {
        return invalid(format!("{path}.duration_seconds"), "must be at least 1");
    }
    Ok(())
}

fn validate_power(target: &CyclingPowerTarget, path: &str) -> Result<(), WorkoutDefinitionError> {
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

fn validate_percent(percent: f64, path: &str) -> Result<(), WorkoutDefinitionError> {
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

fn validate_watts(watts: u16, path: &str) -> Result<(), WorkoutDefinitionError> {
    if watts > MAX_TARGET_WATTS {
        return invalid(path, format!("must not exceed {MAX_TARGET_WATTS} watts"));
    }
    Ok(())
}

fn validate_cadence(
    cadence: &CyclingCadenceTarget,
    path: &str,
) -> Result<(), WorkoutDefinitionError> {
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

fn validate_rpm(rpm: u16, path: &str) -> Result<(), WorkoutDefinitionError> {
    if rpm == 0 || rpm > WORKOUT_CADENCE_RPM_MAX {
        return invalid(
            path,
            format!("must be between 1 and {WORKOUT_CADENCE_RPM_MAX} rpm"),
        );
    }
    Ok(())
}

fn validate_cues(
    cues: &[WorkoutCue],
    duration_seconds: u32,
    path: &str,
) -> Result<(), WorkoutDefinitionError> {
    for (index, cue) in cues.iter().enumerate() {
        let cue_path = format!("{path}.cues[{index}]");
        if cue.offset_seconds >= duration_seconds {
            return invalid(
                format!("{cue_path}.offset_seconds"),
                "must fall within its step",
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

fn compile_steps(
    steps: &[CyclingStep],
    offset_seconds: &mut u32,
    segments: &mut Vec<Segment>,
    text_events: &mut Vec<TextEvent>,
) {
    for step in steps {
        match step {
            CyclingStep::Steady {
                duration_seconds,
                power,
                cadence,
                cues,
            } => {
                compile_cues(cues, *offset_seconds, text_events);
                segments.push(Segment::Steady {
                    duration_s: *duration_seconds,
                    power: compile_power(power),
                    cadence_rpm: cadence.as_ref().map(compile_cadence),
                });
                *offset_seconds += duration_seconds;
            }
            CyclingStep::Ramp {
                duration_seconds,
                start_power,
                end_power,
                cadence,
                cues,
            } => {
                compile_cues(cues, *offset_seconds, text_events);
                segments.push(Segment::Ramp {
                    duration_s: *duration_seconds,
                    start: compile_power(start_power),
                    end: compile_power(end_power),
                    cadence_rpm: cadence.as_ref().map(compile_cadence),
                });
                *offset_seconds += duration_seconds;
            }
            CyclingStep::FreeRide {
                duration_seconds,
                cues,
            } => {
                compile_cues(cues, *offset_seconds, text_events);
                segments.push(Segment::FreeRide {
                    duration_s: *duration_seconds,
                });
                *offset_seconds += duration_seconds;
            }
            CyclingStep::Repeat { count, steps } => {
                for _ in 0..*count {
                    compile_steps(steps, offset_seconds, segments, text_events);
                }
            }
        }
    }
}

fn compile_cues(cues: &[WorkoutCue], step_start: u32, text_events: &mut Vec<TextEvent>) {
    text_events.extend(cues.iter().map(|cue| TextEvent {
        offset_s: step_start + cue.offset_seconds,
        message: cue.message.clone(),
        duration_s: cue.display_seconds,
    }));
}

fn compile_power(target: &CyclingPowerTarget) -> PowerTarget {
    match target {
        CyclingPowerTarget::PercentFtp { percent } => PowerTarget::PercentFtp(percent / 100.0),
        CyclingPowerTarget::Watts { watts } => PowerTarget::Watts(*watts),
        CyclingPowerTarget::PercentFtpRange {
            min_percent,
            max_percent,
        } => PowerTarget::PercentFtp((min_percent + max_percent) / 200.0),
        CyclingPowerTarget::WattsRange {
            min_watts,
            max_watts,
        } => PowerTarget::Watts(midpoint(*min_watts, *max_watts)),
    }
}

fn compile_cadence(target: &CyclingCadenceTarget) -> u16 {
    match target {
        CyclingCadenceTarget::Exact { rpm } => *rpm,
        CyclingCadenceTarget::Range { min_rpm, max_rpm } => midpoint(*min_rpm, *max_rpm),
    }
}

fn midpoint(min: u16, max: u16) -> u16 {
    (u32::from(min) + u32::from(max)).div_ceil(2) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(steps: Vec<CyclingStep>) -> WorkoutDefinition {
        WorkoutDefinition {
            format: WorkoutFormat::Tpw,
            version: TPW_VERSION,
            title: "Aerobic Builder".into(),
            description: "Smooth, controlled work".into(),
            training_focus: Some("Endurance Base".into()),
            prescription: WorkoutPrescription::Cycling { steps },
        }
    }

    fn steady(duration_seconds: u32, percent: f64) -> CyclingStep {
        CyclingStep::Steady {
            duration_seconds,
            power: CyclingPowerTarget::PercentFtp { percent },
            cadence: None,
            cues: vec![],
        }
    }

    #[test]
    fn json_is_explicit_versioned_and_round_trips() {
        let workout = definition(vec![CyclingStep::Steady {
            duration_seconds: 600,
            power: CyclingPowerTarget::PercentFtpRange {
                min_percent: 65.0,
                max_percent: 75.0,
            },
            cadence: Some(CyclingCadenceTarget::Range {
                min_rpm: 85,
                max_rpm: 95,
            }),
            cues: vec![WorkoutCue {
                offset_seconds: 0,
                message: "Relax your shoulders".into(),
                display_seconds: 10,
            }],
        }]);

        let json = workout.to_json_pretty().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            serde_json::json!({
                "format": "TPW",
                "version": 1,
                "title": "Aerobic Builder",
                "description": "Smooth, controlled work",
                "training_focus": "Endurance Base",
                "prescription": {
                    "sport": "cycling",
                    "steps": [{
                        "type": "steady",
                        "duration_seconds": 600,
                        "power": {
                            "type": "percent_ftp_range",
                            "min_percent": 65.0,
                            "max_percent": 75.0
                        },
                        "cadence": {
                            "type": "range",
                            "min_rpm": 85,
                            "max_rpm": 95
                        },
                        "cues": [{
                            "offset_seconds": 0,
                            "message": "Relax your shoulders",
                            "display_seconds": 10
                        }]
                    }]
                }
            })
        );
        assert_eq!(WorkoutDefinition::from_json(&json).unwrap(), workout);
    }

    #[test]
    fn json_defaults_optional_metadata_collections_and_cue_duration() {
        let json = r#"{
          "format": "TPW",
          "version": 1,
          "title": "Endurance",
          "prescription": {
            "sport": "cycling",
            "steps": [{
              "type": "steady",
              "duration_seconds": 600,
              "power": {"type": "percent_ftp", "percent": 70},
              "cues": [{"offset_seconds": 0, "message": "Settle in"}]
            }]
          }
        }"#;

        let workout = WorkoutDefinition::from_json(json).unwrap();
        assert_eq!(workout.description, "");
        assert_eq!(workout.training_focus, None);
        let WorkoutPrescription::Cycling { steps } = &workout.prescription;
        let CyclingStep::Steady { cadence, cues, .. } = &steps[0] else {
            panic!("expected steady step");
        };
        assert_eq!(*cadence, None);
        assert_eq!(cues[0].display_seconds, TEXT_EVENT_DEFAULT_S);
    }

    #[test]
    fn compiles_nested_repeats_ranges_and_step_local_cues() {
        let workout = definition(vec![
            steady(300, 55.0),
            CyclingStep::Repeat {
                count: 2,
                steps: vec![
                    CyclingStep::Steady {
                        duration_seconds: 60,
                        power: CyclingPowerTarget::WattsRange {
                            min_watts: 299,
                            max_watts: 300,
                        },
                        cadence: Some(CyclingCadenceTarget::Range {
                            min_rpm: 89,
                            max_rpm: 90,
                        }),
                        cues: vec![WorkoutCue {
                            offset_seconds: 10,
                            message: "Strong and smooth".into(),
                            display_seconds: 8,
                        }],
                    },
                    steady(120, 50.0),
                ],
            },
        ]);

        let executable = workout.compile().unwrap();
        assert_eq!(executable.name, "Aerobic Builder");
        assert_eq!(executable.duration_s(), 660);
        assert_eq!(executable.segments.len(), 5);
        assert_eq!(
            executable.segments[1],
            Segment::Steady {
                duration_s: 60,
                power: PowerTarget::Watts(300),
                cadence_rpm: Some(90),
            }
        );
        assert_eq!(
            executable
                .text_events
                .iter()
                .map(|cue| cue.offset_s)
                .collect::<Vec<_>>(),
            vec![310, 490]
        );
    }

    #[test]
    fn compiled_definition_drives_the_existing_engine() {
        let workout = definition(vec![CyclingStep::Steady {
            duration_seconds: 60,
            power: CyclingPowerTarget::PercentFtpRange {
                min_percent: 65.0,
                max_percent: 75.0,
            },
            cadence: None,
            cues: vec![],
        }]);
        let mut engine = crate::engine::Engine::new(workout.compile().unwrap(), 200, 1.0);

        assert_eq!(
            engine.handle(crate::engine::Input::Start),
            vec![
                crate::engine::Effect::TrainerStart,
                crate::engine::Effect::SetTarget(140),
            ]
        );
    }

    #[test]
    fn rejects_unsupported_versions_invalid_ranges_and_unsafe_repeats() {
        let mut workout = definition(vec![steady(60, 75.0)]);
        workout.version += 1;
        assert!(matches!(
            workout.validate(),
            Err(WorkoutDefinitionError::UnsupportedVersion { .. })
        ));

        let bad_range = definition(vec![CyclingStep::Steady {
            duration_seconds: 60,
            power: CyclingPowerTarget::PercentFtpRange {
                min_percent: 90.0,
                max_percent: 80.0,
            },
            cadence: None,
            cues: vec![],
        }]);
        assert!(matches!(
            bad_range.validate(),
            Err(WorkoutDefinitionError::Invalid { path, .. })
                if path == "prescription.steps[0].power"
        ));

        let explosive = definition(vec![CyclingStep::Repeat {
            count: WORKOUT_REPEAT_COUNT_MAX,
            steps: vec![CyclingStep::Repeat {
                count: WORKOUT_REPEAT_COUNT_MAX,
                steps: vec![CyclingStep::Repeat {
                    count: 2,
                    steps: vec![steady(1, 75.0)],
                }],
            }],
        }]);
        assert!(matches!(
            explosive.validate(),
            Err(WorkoutDefinitionError::Invalid { path, .. })
                if path == "prescription.steps"
        ));

        let mut deeply_nested = steady(1, 75.0);
        for _ in 0..=WORKOUT_REPEAT_DEPTH_MAX {
            deeply_nested = CyclingStep::Repeat {
                count: 1,
                steps: vec![deeply_nested],
            };
        }
        assert!(matches!(
            definition(vec![deeply_nested]).validate(),
            Err(WorkoutDefinitionError::Invalid { message, .. })
                if message.contains("nesting")
        ));
    }

    #[test]
    fn rejects_unknown_json_fields_instead_of_silently_dropping_them() {
        let json = r#"{
          "format": "TPW",
          "version": 1,
          "title": "Endurance",
          "prescription": {
            "sport": "cycling",
            "steps": [{
              "type": "steady",
              "duration_seconds": 600,
              "power": {"type": "percent_ftp", "percent": 70},
              "mystery_target": 42
            }]
          }
        }"#;
        assert!(matches!(
            WorkoutDefinition::from_json(json),
            Err(WorkoutDefinitionError::InvalidJson { .. })
        ));
    }

    #[test]
    fn future_json_versions_are_reported_before_their_fields_are_parsed() {
        let json = r#"{
          "format": "TPW",
          "version": 2,
          "brand_new_v2_field": true
        }"#;
        assert!(matches!(
            WorkoutDefinition::from_json(json),
            Err(WorkoutDefinitionError::UnsupportedVersion {
                found: 2,
                supported: TPW_VERSION
            })
        ));
    }

    #[test]
    fn other_formats_are_reported_before_their_fields_are_parsed() {
        let json = r#"{
          "format": "ZWO",
          "version": 1,
          "zwo_specific_field": true
        }"#;
        assert!(matches!(
            WorkoutDefinition::from_json(json),
            Err(WorkoutDefinitionError::UnsupportedFormat { found }) if found == "ZWO"
        ));
    }
}
