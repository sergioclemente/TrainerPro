//! Explicit instructions embedded in FIT; never inferred from titles or laps.
use fitparser::{de::FitDataMessage, Value};
use serde::{Deserialize, Serialize};
const WORKOUT_NAME: u8 = 8;
const STEP_INDEX: u8 = 254;
const DURATION_TYPE: u8 = 1;
const DURATION_VALUE: u8 = 2;
const POWER_TARGET: f64 = 4.0;
const CADENCE_TARGET: f64 = 3.0;
const OPEN_TARGET: f64 = 2.0;
const WATTS_OFFSET: f64 = 1000.0;
const MILLIS_PER_SECOND: f64 = 1000.0;
const STEP_TARGET_TYPE: u8 = 3;
const STEP_TARGET_VALUE: u8 = 4;
const STEP_TARGET_LOW: u8 = 5;
const STEP_TARGET_HIGH: u8 = 6;
const STEP_SECONDARY_TYPE: u8 = 19;
const STEP_SECONDARY_VALUE: u8 = 20;
const STEP_SECONDARY_LOW: u8 = 21;
const STEP_SECONDARY_HIGH: u8 = 22;
const STEP_INTENSITY: u8 = 7;
const WORKOUT_STEP_COUNT: u8 = 6;
const TIME_DURATION: f64 = 0.0;
const CUSTOM_TARGET: f64 = 0.0;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitInstructions {
    pub name: Option<String>,
    pub declared_step_count: Option<u16>,
    /// Preserves wire order; limitations flag indices outside the declared sequence.
    pub steps: Vec<FitInstructionStep>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitInstructionStep {
    pub index: Option<u16>,
    pub duration_seconds: Option<f64>,
    pub target: FitInstructionTarget,
    pub cadence_rpm: Option<(f64, f64)>,
    pub intensity: Option<u8>,
    pub supported: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FitInstructionTarget {
    Open,
    PowerRange { low: f64, high: f64, unit: String },
    Unsupported,
}
fn number(message: &FitDataMessage, field: u8) -> Option<f64> {
    super::decode::number(message, field)
}
pub(super) fn index(message: &FitDataMessage, field: u8) -> Option<u16> {
    number(message, field)
        .filter(|n| *n >= 0.0 && *n < f64::from(u16::MAX) && n.fract() == 0.0)
        .map(|n| n as u16)
}
pub(super) fn decode(
    workout: Option<&FitDataMessage>,
    steps: &[FitDataMessage],
) -> Option<FitInstructions> {
    if workout.is_none() && steps.is_empty() {
        return None;
    }
    let decoded: Vec<_> = steps
        .iter()
        .map(|s| {
            let target = match number(s, STEP_TARGET_TYPE) {
                Some(OPEN_TARGET) => FitInstructionTarget::Open,
                Some(POWER_TARGET) if number(s, STEP_TARGET_VALUE) == Some(CUSTOM_TARGET) => {
                    match number(s, STEP_TARGET_LOW).zip(number(s, STEP_TARGET_HIGH)) {
                        Some((lo, hi)) if lo <= hi && hi <= WATTS_OFFSET => {
                            FitInstructionTarget::PowerRange {
                                low: lo,
                                high: hi,
                                unit: "%FTP".into(),
                            }
                        }
                        Some((lo, hi)) if lo <= hi && lo > WATTS_OFFSET => {
                            FitInstructionTarget::PowerRange {
                                low: lo - WATTS_OFFSET,
                                high: hi - WATTS_OFFSET,
                                unit: "W".into(),
                            }
                        }
                        _ => FitInstructionTarget::Unsupported,
                    }
                }
                _ => FitInstructionTarget::Unsupported,
            };
            let duration_seconds = if number(s, DURATION_TYPE) == Some(TIME_DURATION) {
                number(s, DURATION_VALUE)
                    .filter(|v| *v > 0.0)
                    .map(|v| v / MILLIS_PER_SECOND)
            } else {
                None
            };
            let cadence_rpm = if number(s, STEP_SECONDARY_TYPE) == Some(CADENCE_TARGET)
                && number(s, STEP_SECONDARY_VALUE) == Some(CUSTOM_TARGET)
            {
                number(s, STEP_SECONDARY_LOW)
                    .zip(number(s, STEP_SECONDARY_HIGH))
                    .filter(|(lo, hi)| *lo > 0.0 && lo <= hi)
            } else {
                None
            };
            let secondary_supported = number(s, STEP_SECONDARY_TYPE).is_none()
                || number(s, STEP_SECONDARY_TYPE) == Some(OPEN_TARGET)
                || cadence_rpm.is_some();
            let index = index(s, STEP_INDEX);
            let supported = index.is_some()
                && duration_seconds.is_some()
                && !matches!(target, FitInstructionTarget::Unsupported)
                && secondary_supported;
            FitInstructionStep {
                index,
                duration_seconds,
                target,
                cadence_rpm,
                intensity: number(s, STEP_INTENSITY).map(|v| v as u8),
                supported,
            }
        })
        .collect();
    let name = workout.and_then(|w| match w.fields().get(&WORKOUT_NAME) {
        Some(Value::String(s)) => Some(s.trim_end_matches('\0').to_owned()),
        _ => None,
    });
    let declared_step_count = workout.and_then(|w| index(w, WORKOUT_STEP_COUNT));
    let mut limitations=vec!["Embedded FIT instructions are a recorded representation, not the original workout document or proof of a scheduled occurrence.".into()];
    if declared_step_count.map(usize::from) != Some(decoded.len())
        || decoded.iter().enumerate().any(|(position, step)| {
            !step.supported || step.index.map(usize::from) != Some(position)
        })
    {
        limitations.push("Instructions contain missing, duplicate, out-of-order, or unsupported steps; target interpretation is limited.".into());
    }
    Some(FitInstructions {
        name,
        declared_step_count,
        steps: decoded,
        limitations,
    })
}
