//! Concrete cycling authoring, sharing bounded timed/repeat framing with running.
use super::*;
const CYCLING_SPORT_ID: u8 = 2;
const POWER_TARGET_ID: u8 = 2;
const CADENCE_TARGET_ID: u8 = 3;
const PERCENT_UNIT_ID: u16 = 253;
const MAX_POWER_WATTS: u16 = 2000;
const MAX_FTP_PERCENT: f64 = 300.0;
const MAX_CADENCE_RPM: u16 = 300;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "sport", content = "workout", rename_all = "snake_case")]
pub enum AuthoredWorkout {
    Running(RunningWorkout),
    Cycling(CyclingWorkout),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CyclingWorkout {
    pub name: String,
    pub description: String,
    pub steps: Vec<CyclingStep>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CyclingStep {
    Timed {
        seconds: u32,
        power: Option<CyclingPower>,
        cadence: Option<(u16, u16)>,
    },
    Repeat {
        count: u32,
        steps: Vec<CyclingStep>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "unit", rename_all = "snake_case", deny_unknown_fields)]
pub enum CyclingPower {
    Watts { low: u16, high: u16 },
    PercentFtp { low: f64, high: f64 },
}
impl AuthoredWorkout {
    pub fn name(&self) -> &str {
        match self {
            Self::Running(w) => &w.name,
            Self::Cycling(w) => &w.name,
        }
    }
    pub fn payload(&self) -> Result<Value, GarminError> {
        match self {
            Self::Running(w) => w.payload(),
            Self::Cycling(w) => w.payload(),
        }
    }
    pub fn matches(&self, actual: &Value) -> Result<bool, GarminError> {
        match self {
            Self::Running(w) => w.matches(actual),
            Self::Cycling(w) => w.matches(actual),
        }
    }
}
impl CyclingStep {
    fn skeleton(&self, depth: usize) -> Result<RunningStep, GarminError> {
        if depth > MAX_REPEAT_DEPTH {
            return Err(GarminError::InvalidRequest);
        }
        Ok(match self {
            Self::Timed {
                seconds,
                power,
                cadence,
            } => {
                if let Some(power) = power {
                    match power {
                        CyclingPower::Watts { low, high }
                            if low <= high && *high <= MAX_POWER_WATTS => {}
                        CyclingPower::PercentFtp { low, high }
                            if low.is_finite()
                                && high.is_finite()
                                && *low >= 0.0
                                && low <= high
                                && *high <= MAX_FTP_PERCENT => {}
                        _ => return Err(GarminError::InvalidRequest),
                    }
                }
                if cadence.is_some_and(|(lo, hi)| lo == 0 || lo > hi || hi > MAX_CADENCE_RPM) {
                    return Err(GarminError::InvalidRequest);
                }
                RunningStep::Timed {
                    seconds: *seconds,
                    role: StepRole::Other,
                    target: RunningTarget::Open,
                }
            }
            Self::Repeat { count, steps } => RunningStep::Repeat {
                count: *count,
                steps: steps
                    .iter()
                    .map(|s| s.skeleton(depth + 1))
                    .collect::<Result<_, _>>()?,
            },
        })
    }
}
fn decorate(steps: &[CyclingStep], payload: &mut Value) {
    for (step, value) in steps.iter().zip(payload.as_array_mut().unwrap()) {
        match step {
            CyclingStep::Repeat { steps, .. } => decorate(steps, &mut value["workoutSteps"]),
            CyclingStep::Timed { power, cadence, .. } => {
                if let Some(power) = power {
                    value["targetType"] = json!({"workoutTargetTypeId":POWER_TARGET_ID,"workoutTargetTypeKey":"power.zone"});
                    let (lo, hi) = match power {
                        CyclingPower::Watts { low, high } => (json!(low), json!(high)),
                        CyclingPower::PercentFtp { low, high } => {
                            value["targetValueUnit"] =
                                json!({"unitId":PERCENT_UNIT_ID,"unitKey":"percent","factor":1.0});
                            (json!(low), json!(high))
                        }
                    };
                    value["targetValueOne"] = lo;
                    value["targetValueTwo"] = hi;
                }
                if let Some((lo, hi)) = cadence {
                    value["secondaryTargetType"] = json!({"workoutTargetTypeId":CADENCE_TARGET_ID,"workoutTargetTypeKey":"cadence"});
                    value["secondaryTargetValueOne"] = json!(lo);
                    value["secondaryTargetValueTwo"] = json!(hi);
                }
            }
        }
    }
}
impl CyclingWorkout {
    pub fn payload(&self) -> Result<Value, GarminError> {
        let mut value = RunningWorkout {
            name: self.name.clone(),
            description: self.description.clone(),
            steps: self
                .steps
                .iter()
                .map(|s| s.skeleton(0))
                .collect::<Result<_, _>>()?,
        }
        .payload()?;
        let sport = json!({"sportTypeId":CYCLING_SPORT_ID,"sportTypeKey":"cycling"});
        value["sportType"] = sport.clone();
        value["workoutSegments"][0]["sportType"] = sport;
        decorate(
            &self.steps,
            &mut value["workoutSegments"][0]["workoutSteps"],
        );
        Ok(value)
    }
    pub fn matches(&self, actual: &Value) -> Result<bool, GarminError> {
        let expected = self.payload()?;
        Ok(actual["workoutName"] == expected["workoutName"]
            && actual["description"] == expected["description"]
            && actual["sportType"]["sportTypeId"] == CYCLING_SPORT_ID
            && actual["workoutSegments"].as_array().is_some_and(|s| {
                s.len() == 1
                    && s[0]["segmentOrder"] == 1
                    && s[0]["sportType"]["sportTypeId"] == CYCLING_SPORT_ID
                    && cycling_steps_match(
                        &expected["workoutSegments"][0]["workoutSteps"],
                        &s[0]["workoutSteps"],
                    )
            }))
    }
}
fn cycling_steps_match(expected: &Value, actual: &Value) -> bool {
    let (Some(expected), Some(actual)) = (expected.as_array(), actual.as_array()) else {
        return false;
    };
    expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(e, a)| {
            if ["type", "stepOrder", "numberOfIterations"]
                .iter()
                .any(|k| !close(&e[*k], &a[*k]))
                || e["stepType"]["stepTypeId"] != a["stepType"]["stepTypeId"]
                || e["endCondition"]["conditionTypeId"] != a["endCondition"]["conditionTypeId"]
                || !close(&e["endConditionValue"], &a["endConditionValue"])
                || a["smartRepeat"].as_bool() == Some(true)
                || a["skipLastRestStep"].as_bool() == Some(true)
                || a["description"].as_str().is_some_and(|s| !s.is_empty())
            {
                return false;
            }
            if e["type"] == "RepeatGroupDTO" {
                return cycling_steps_match(&e["workoutSteps"], &a["workoutSteps"]);
            }
            for (kind, one, two, unit, zone) in [
                (
                    "targetType",
                    "targetValueOne",
                    "targetValueTwo",
                    "targetValueUnit",
                    "zoneNumber",
                ),
                (
                    "secondaryTargetType",
                    "secondaryTargetValueOne",
                    "secondaryTargetValueTwo",
                    "secondaryTargetValueUnit",
                    "secondaryZoneNumber",
                ),
            ] {
                let expected = e[kind]["workoutTargetTypeId"]
                    .as_u64()
                    .unwrap_or(u64::from(OPEN_TARGET_ID));
                let actual = a[kind]["workoutTargetTypeId"]
                    .as_u64()
                    .unwrap_or(u64::from(OPEN_TARGET_ID));
                if expected != actual || a[zone].as_u64().unwrap_or(0) != 0 {
                    return false;
                }
                if expected != u64::from(OPEN_TARGET_ID)
                    && (!close(&e[one], &a[one])
                        || !close(&e[two], &a[two])
                        || e[unit]["unitId"] != a[unit]["unitId"]
                        || !close(&e[unit]["factor"], &a[unit]["factor"]))
                {
                    return false;
                }
            }
            true
        })
}
impl GarminClient {
    pub async fn create_workout(
        &self,
        tokens: &Tokens,
        workout: &AuthoredWorkout,
    ) -> Result<String, GarminError> {
        let body = self
            .workout_request(tokens, Method::POST, WORKOUT_PATH, Some(workout.payload()?))
            .await?;
        workout_id(&body.unwrap_or(Value::Null)).map_err(|_| GarminError::WorkoutUncertain)
    }
}
