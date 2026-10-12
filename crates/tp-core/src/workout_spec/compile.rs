//! Conversion between WorkoutSpec cycling prescriptions and the flat player model.

use super::{
    invalid, CyclingCadenceTarget, CyclingPowerTarget, CyclingStep, WorkoutCue, WorkoutFormat,
    WorkoutPrescription, WorkoutSpec, WorkoutSpecError, WORKOUT_SPEC_VERSION,
};
use crate::model::{ExecutableWorkout, PowerTarget, TextEvent, WorkoutSegment};

impl WorkoutSpec {
    /// Normalize the current flat cycling execution model into WorkoutSpec.
    ///
    /// This preserves executable meaning but cannot recover authoring
    /// structure, such as repeats or target ranges, that was already flattened
    /// by a boundary parser.
    pub fn from_executable(workout: ExecutableWorkout) -> Result<Self, WorkoutSpecError> {
        let ExecutableWorkout {
            name,
            description,
            segments,
            mut text_events,
        } = workout;
        text_events.sort_by_key(|event| event.offset_s);

        let mut steps = Vec::with_capacity(segments.len());
        let mut event_index = 0usize;
        let mut step_start_seconds = 0u64;
        for (step_index, segment) in segments.into_iter().enumerate() {
            let duration_seconds = segment.duration_s();
            let step_end_seconds = step_start_seconds + u64::from(duration_seconds);
            let mut cues = Vec::new();

            while let Some(event) = text_events.get(event_index) {
                let event_offset = u64::from(event.offset_s);
                if event_offset < step_start_seconds {
                    return invalid(
                        format!("text_events[{event_index}].offset_s"),
                        "must not precede its executable segment",
                    );
                }
                if event_offset >= step_end_seconds {
                    break;
                }
                cues.push(WorkoutCue {
                    offset_seconds: (event_offset - step_start_seconds) as u32,
                    message: event.message.clone(),
                    display_seconds: event.duration_s,
                });
                event_index += 1;
            }

            let step = match segment {
                WorkoutSegment::Steady {
                    power, cadence_rpm, ..
                } => CyclingStep::Steady {
                    duration_seconds,
                    power: definition_power(power),
                    cadence: cadence_rpm.map(|rpm| CyclingCadenceTarget::Exact { rpm }),
                    cues,
                },
                WorkoutSegment::Ramp {
                    start,
                    end,
                    cadence_rpm,
                    ..
                } => CyclingStep::Ramp {
                    duration_seconds,
                    start_power: definition_power(start),
                    end_power: definition_power(end),
                    cadence: cadence_rpm.map(|rpm| CyclingCadenceTarget::Exact { rpm }),
                    cues,
                },
                WorkoutSegment::FreeRide { cadence_rpm, .. } => CyclingStep::FreeRide {
                    duration_seconds,
                    cadence: cadence_rpm.map(|rpm| CyclingCadenceTarget::Exact { rpm }),
                    cues,
                },
            };
            steps.push(step);
            step_start_seconds = step_end_seconds;

            if duration_seconds == 0 {
                return invalid(
                    format!("prescription.steps[{step_index}].duration_seconds"),
                    "must be at least 1",
                );
            }
        }

        if event_index != text_events.len() {
            return invalid(
                format!("text_events[{event_index}].offset_s"),
                "must fall within the executable workout",
            );
        }

        let definition = Self {
            format: WorkoutFormat::WorkoutSpec,
            version: WORKOUT_SPEC_VERSION,
            title: name,
            description,
            training_focus: None,
            prescription: WorkoutPrescription::Cycling { steps },
        };
        definition.validate()?;
        Ok(definition)
    }

    pub fn compile(&self) -> Result<ExecutableWorkout, WorkoutSpecError> {
        self.validated_stats()?;

        let mut segments = Vec::new();
        let mut text_events = Vec::new();
        let mut offset_seconds = 0u32;
        let WorkoutPrescription::Cycling { steps } = &self.prescription else {
            return Err(WorkoutSpecError::UnsupportedExecutionSport {
                sport: "running".into(),
            });
        };
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

fn definition_power(target: PowerTarget) -> CyclingPowerTarget {
    match target {
        PowerTarget::PercentFtp(fraction) => CyclingPowerTarget::PercentFtp {
            percent: fraction * 100.0,
        },
        PowerTarget::Watts(watts) => CyclingPowerTarget::Watts { watts },
    }
}

fn compile_steps(
    steps: &[CyclingStep],
    offset_seconds: &mut u32,
    segments: &mut Vec<WorkoutSegment>,
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
                segments.push(WorkoutSegment::Steady {
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
                segments.push(WorkoutSegment::Ramp {
                    duration_s: *duration_seconds,
                    start: compile_power(start_power),
                    end: compile_power(end_power),
                    cadence_rpm: cadence.as_ref().map(compile_cadence),
                });
                *offset_seconds += duration_seconds;
            }
            CyclingStep::FreeRide {
                duration_seconds,
                cadence,
                cues,
            } => {
                compile_cues(cues, *offset_seconds, text_events);
                segments.push(WorkoutSegment::FreeRide {
                    duration_s: *duration_seconds,
                    cadence_rpm: cadence.as_ref().map(compile_cadence),
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

/// Single value a `min..=max` range compiles to (rounding up). Shared with the
/// ZWO reader so a `CadenceLow`/`CadenceHigh` pair lands on the same rpm.
pub(crate) fn midpoint(min: u16, max: u16) -> u16 {
    (u32::from(min) + u32::from(max)).div_ceil(2) as u16
}
