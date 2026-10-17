//! Bounded workout protocol operations. Applications own approval, receipts,
//! account locks, retries and reconciliation; writes are always sent once.
use super::{
    bounded_response, native_headers, positive_id, GarminClient, GarminError, Tokens,
    MAX_JSON_BYTES, MAX_PAGE_SIZE,
};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const WORKOUT_PATH: &str = "/workout-service/workout";
const SCHEDULE_PATH: &str = "/workout-service/schedule";
const RUNNING_SPORT_ID: u8 = 1;
const TIME_CONDITION_ID: u8 = 2;
const REPEAT_CONDITION_ID: u8 = 7;
const OPEN_TARGET_ID: u8 = 1;
const PACE_TARGET_ID: u8 = 6;
const REPEAT_STEP_ID: u8 = 6;
const MAX_STEPS: usize = 100;
const MAX_REPEAT_DEPTH: usize = 3;
const MAX_REPEAT_COUNT: u32 = 100;
const FLOAT_TOLERANCE: f64 = 0.000_01;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunningWorkout {
    pub name: String,
    pub description: String,
    pub steps: Vec<RunningStep>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunningStep {
    Timed {
        seconds: u32,
        role: StepRole,
        target: RunningTarget,
    },
    Repeat {
        count: u32,
        steps: Vec<RunningStep>,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StepRole {
    Warmup,
    Work,
    Recovery,
    Cooldown,
    Rest,
    Other,
}
impl StepRole {
    fn descriptor(self) -> (u8, &'static str) {
        match self {
            Self::Warmup => (1, "warmup"),
            Self::Cooldown => (2, "cooldown"),
            Self::Work => (3, "interval"),
            Self::Recovery => (4, "recovery"),
            Self::Rest => (5, "rest"),
            Self::Other => (7, "other"),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunningTarget {
    Open,
    /// Garmin's custom pace target is a speed range, in meters per second.
    PaceRange {
        slow_mps: f64,
        fast_mps: f64,
    },
}
impl RunningWorkout {
    pub fn payload(&self) -> Result<Value, GarminError> {
        if self.name.trim().is_empty() || self.name.len() > 255 || self.description.len() > 4096 {
            return Err(GarminError::InvalidRequest);
        }
        let mut expanded = 0;
        let (steps, duration) = encode_steps(&self.steps, 0, &mut expanded, &mut 0)?;
        Ok(
            json!({"workoutName":self.name,"description":self.description,
            "sportType":{"sportTypeId":RUNNING_SPORT_ID,"sportTypeKey":"running"},
            "estimatedDurationInSecs":duration,
            "workoutSegments":[{"segmentOrder":1,"sportType":{"sportTypeId":RUNNING_SPORT_ID,"sportTypeKey":"running"},"workoutSteps":steps}]}),
        )
    }
    /// Verify authored semantics while ignoring generated IDs, timestamps and
    /// presentation metadata. Extra targets/cues or repeat behavior are rejected.
    pub fn matches(&self, actual: &Value) -> Result<bool, GarminError> {
        let expected = self.payload()?;
        Ok(actual["workoutName"] == expected["workoutName"]
            && actual["description"] == expected["description"]
            && actual["sportType"]["sportTypeId"] == RUNNING_SPORT_ID
            && actual["workoutSegments"]
                .as_array()
                .is_some_and(|segments| {
                    segments.len() == 1
                        && segments[0]["segmentOrder"] == 1
                        && segments[0]["sportType"]["sportTypeId"] == RUNNING_SPORT_ID
                        && steps_match(
                            &expected["workoutSegments"][0]["workoutSteps"],
                            &segments[0]["workoutSteps"],
                        )
                }))
    }
}
fn encode_steps(
    steps: &[RunningStep],
    depth: usize,
    expanded: &mut usize,
    order: &mut usize,
) -> Result<(Vec<Value>, u32), GarminError> {
    if steps.is_empty() || depth > MAX_REPEAT_DEPTH {
        return Err(GarminError::InvalidRequest);
    }
    let mut duration = 0u32;
    let mut encoded = Vec::new();
    for step in steps {
        *order += 1;
        let step_order = *order;
        let (value, seconds, count) = match step {
            RunningStep::Timed {
                seconds,
                role,
                target,
            } => {
                if *seconds == 0 {
                    return Err(GarminError::InvalidRequest);
                }
                let (id, key) = role.descriptor();
                let mut value = json!({"type":"ExecutableStepDTO","stepOrder":step_order,
                    "stepType":{"stepTypeId":id,"stepTypeKey":key},
                    "endCondition":{"conditionTypeId":TIME_CONDITION_ID,"conditionTypeKey":"time"},
                    "endConditionValue":seconds});
                match target {
                    RunningTarget::Open => {
                        value["targetType"] = json!({"workoutTargetTypeId":OPEN_TARGET_ID,"workoutTargetTypeKey":"no.target"})
                    }
                    RunningTarget::PaceRange { slow_mps, fast_mps } => {
                        if !slow_mps.is_finite()
                            || !fast_mps.is_finite()
                            || *slow_mps <= 0.0
                            || slow_mps > fast_mps
                        {
                            return Err(GarminError::InvalidRequest);
                        }
                        value["targetType"] = json!({"workoutTargetTypeId":PACE_TARGET_ID,"workoutTargetTypeKey":"pace.zone"});
                        value["targetValueOne"] = json!(slow_mps);
                        value["targetValueTwo"] = json!(fast_mps);
                    }
                }
                (value, *seconds, 1)
            }
            RunningStep::Repeat { count, steps } => {
                if !(1..=MAX_REPEAT_COUNT).contains(count) {
                    return Err(GarminError::InvalidRequest);
                }
                let mut children = 0;
                let (nested, seconds) = encode_steps(steps, depth + 1, &mut children, order)?;
                let seconds = seconds
                    .checked_mul(*count)
                    .ok_or(GarminError::InvalidRequest)?;
                let children = children
                    .checked_mul(*count as usize)
                    .ok_or(GarminError::InvalidRequest)?;
                (
                    json!({"type":"RepeatGroupDTO","stepOrder":step_order,
                    "stepType":{"stepTypeId":REPEAT_STEP_ID,"stepTypeKey":"repeat"},
                    "numberOfIterations":count,"workoutSteps":nested,
                    "endCondition":{"conditionTypeId":REPEAT_CONDITION_ID,"conditionTypeKey":"iterations"},
                    "endConditionValue":count,"smartRepeat":false,"skipLastRestStep":false}),
                    seconds,
                    children,
                )
            }
        };
        *expanded = expanded
            .checked_add(count)
            .ok_or(GarminError::InvalidRequest)?;
        if *expanded > MAX_STEPS {
            return Err(GarminError::InvalidRequest);
        }
        duration = duration
            .checked_add(seconds)
            .ok_or(GarminError::InvalidRequest)?;
        encoded.push(value);
    }
    Ok((encoded, duration))
}
fn close(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => (a - b).abs() <= FLOAT_TOLERANCE,
        _ => a == b,
    }
}
fn steps_match(expected: &Value, actual: &Value) -> bool {
    let (Some(expected), Some(actual)) = (expected.as_array(), actual.as_array()) else {
        return false;
    };
    expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(e, a)| {
            if e["type"] != a["type"]
                || e["stepOrder"] != a["stepOrder"]
                || e["stepType"]["stepTypeId"] != a["stepType"]["stepTypeId"]
                || e["endCondition"]["conditionTypeId"] != a["endCondition"]["conditionTypeId"]
                || !close(&e["endConditionValue"], &a["endConditionValue"])
                || a["secondaryTargetType"]
                    .as_object()
                    .is_some_and(|t| t.get("workoutTargetTypeId") != Some(&json!(OPEN_TARGET_ID)))
                || a["description"].as_str().is_some_and(|v| !v.is_empty())
            {
                return false;
            }
            if e["type"] == "RepeatGroupDTO" {
                e["numberOfIterations"] == a["numberOfIterations"]
                    && a["smartRepeat"].as_bool() != Some(true)
                    && a["skipLastRestStep"].as_bool() != Some(true)
                    && steps_match(&e["workoutSteps"], &a["workoutSteps"])
            } else {
                e["targetType"]["workoutTargetTypeId"] == a["targetType"]["workoutTargetTypeId"]
                    && (e["targetType"]["workoutTargetTypeId"] == OPEN_TARGET_ID
                        || (close(&e["targetValueOne"], &a["targetValueOne"])
                            && close(&e["targetValueTwo"], &a["targetValueTwo"])
                            && a["zoneNumber"].as_u64().unwrap_or(0) == 0))
            }
        })
}
pub fn workout_id(body: &Value) -> Result<String, GarminError> {
    body.get("workoutId")
        .and_then(positive_id)
        .ok_or(GarminError::Protocol)
}
pub fn schedule_id(body: &Value) -> Result<String, GarminError> {
    body.get("workoutScheduleId")
        .and_then(positive_id)
        .ok_or(GarminError::Protocol)
}
pub fn schedule_matches(body: &Value, workout: &str, date: &str) -> bool {
    body.get("workout")
        .and_then(|w| workout_id(w).ok())
        .as_deref()
        == Some(workout)
        && body["calendarDate"].as_str() == Some(date)
}
fn valid_id(id: &str) -> Result<(), GarminError> {
    if id.is_empty()
        || id.len() > 20
        || id.starts_with('0')
        || !id.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(GarminError::InvalidRequest);
    }
    Ok(())
}
fn valid_date(date: &str) -> Result<(), GarminError> {
    if date.len() != 10 || chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
        return Err(GarminError::InvalidRequest);
    }
    Ok(())
}
impl GarminClient {
    async fn workout_request(
        &self,
        tokens: &Tokens,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Option<Value>, GarminError> {
        let read = method == Method::GET;
        let mut request = native_headers(
            self.http
                .request(method, format!("{}{path}", self.api_base)),
        )
        .bearer_auth(&tokens.access_token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| {
            if read {
                GarminError::Network
            } else {
                GarminError::WorkoutUncertain
            }
        })?;
        if read && response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let bytes =
            bounded_response(response, MAX_JSON_BYTES)
                .await
                .map_err(|error| match error {
                    GarminError::Authentication
                    | GarminError::Challenge
                    | GarminError::RateLimited => error,
                    _ if !read => GarminError::WorkoutUncertain,
                    _ => error,
                })?;
        if bytes.is_empty() && !read {
            return Ok(Some(Value::Null));
        }
        let body = serde_json::from_slice(&bytes).map_err(|_| {
            if read {
                GarminError::Protocol
            } else {
                GarminError::WorkoutUncertain
            }
        })?;
        Ok(Some(body))
    }
    pub async fn create_running_workout(
        &self,
        tokens: &Tokens,
        workout: &RunningWorkout,
    ) -> Result<String, GarminError> {
        let body = self
            .workout_request(tokens, Method::POST, WORKOUT_PATH, Some(workout.payload()?))
            .await?;
        workout_id(&body.unwrap_or(Value::Null)).map_err(|_| GarminError::WorkoutUncertain)
    }
    pub async fn get_workout(
        &self,
        tokens: &Tokens,
        id: &str,
    ) -> Result<Option<Value>, GarminError> {
        valid_id(id)?;
        let body = self
            .workout_request(tokens, Method::GET, &format!("{WORKOUT_PATH}/{id}"), None)
            .await?;
        if let Some(body) = &body {
            if workout_id(body)?.as_str() != id {
                return Err(GarminError::Protocol);
            }
        }
        Ok(body)
    }
    pub async fn list_workouts(
        &self,
        tokens: &Tokens,
        start: u32,
        limit: u32,
    ) -> Result<Vec<Value>, GarminError> {
        if !(1..=MAX_PAGE_SIZE).contains(&limit) {
            return Err(GarminError::InvalidRequest);
        }
        let body = self
            .workout_request(
                tokens,
                Method::GET,
                &format!("/workout-service/workouts?start={start}&limit={limit}"),
                None,
            )
            .await?
            .ok_or(GarminError::Protocol)?;
        let values = body.as_array().ok_or(GarminError::Protocol)?;
        if values.len() > limit as usize {
            return Err(GarminError::Protocol);
        }
        for item in values {
            workout_id(item)?;
        }
        Ok(values.clone())
    }
    pub async fn schedule_workout(
        &self,
        tokens: &Tokens,
        id: &str,
        date: &str,
    ) -> Result<String, GarminError> {
        valid_id(id)?;
        valid_date(date)?;
        let body = self
            .workout_request(
                tokens,
                Method::POST,
                &format!("{SCHEDULE_PATH}/{id}"),
                Some(json!({"date":date})),
            )
            .await?
            .unwrap_or(Value::Null);
        if !schedule_matches(&body, id, date) {
            return Err(GarminError::WorkoutUncertain);
        }
        schedule_id(&body).map_err(|_| GarminError::WorkoutUncertain)
    }
    pub async fn get_workout_schedule(
        &self,
        tokens: &Tokens,
        id: &str,
    ) -> Result<Option<Value>, GarminError> {
        valid_id(id)?;
        let body = self
            .workout_request(tokens, Method::GET, &format!("{SCHEDULE_PATH}/{id}"), None)
            .await?;
        if let Some(body) = &body {
            if schedule_id(body)?.as_str() != id {
                return Err(GarminError::Protocol);
            }
        }
        Ok(body)
    }
    pub async fn list_workout_schedules(
        &self,
        tokens: &Tokens,
        date: &str,
    ) -> Result<Vec<Value>, GarminError> {
        valid_date(date)?;
        let body = self
            .workout_request(
                tokens,
                Method::GET,
                &format!("{SCHEDULE_PATH}/workoutScheduleByPeriod?startDate={date}&endDate={date}"),
                None,
            )
            .await?
            .ok_or(GarminError::Protocol)?;
        let items = body.as_array().ok_or(GarminError::Protocol)?;
        if items.len() >= MAX_PAGE_SIZE as usize {
            return Err(GarminError::Protocol);
        }
        for item in items {
            schedule_id(item)?;
        }
        Ok(items.clone())
    }
    pub async fn delete_workout_schedule(
        &self,
        tokens: &Tokens,
        id: &str,
    ) -> Result<(), GarminError> {
        valid_id(id)?;
        self.workout_request(
            tokens,
            Method::DELETE,
            &format!("{SCHEDULE_PATH}/{id}"),
            None,
        )
        .await?;
        Ok(())
    }
    pub async fn delete_workout(&self, tokens: &Tokens, id: &str) -> Result<(), GarminError> {
        valid_id(id)?;
        self.workout_request(
            tokens,
            Method::DELETE,
            &format!("{WORKOUT_PATH}/{id}"),
            None,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readback_checks_semantics_and_accepts_generated_metadata_and_rounding() {
        let workout = RunningWorkout {
            name: "synthetic".into(),
            description: "test".into(),
            steps: vec![RunningStep::Repeat {
                count: 2,
                steps: vec![RunningStep::Timed {
                    seconds: 60,
                    role: StepRole::Work,
                    target: RunningTarget::PaceRange {
                        slow_mps: 2.7,
                        fast_mps: 3.0,
                    },
                }],
            }],
        };
        let mut body = workout.payload().unwrap();
        body["workoutId"] = json!(41);
        body["workoutSegments"][0]["workoutSteps"][0]["workoutSteps"][0]["targetValueOne"] =
            json!(2.700001);
        assert!(workout.matches(&body).unwrap());
        for (key, value) in [
            ("numberOfIterations", json!(3)),
            ("smartRepeat", json!(true)),
            ("skipLastRestStep", json!(true)),
        ] {
            let mut altered = body.clone();
            altered["workoutSegments"][0]["workoutSteps"][0][key] = value;
            assert!(!workout.matches(&altered).unwrap());
        }
        let step = &mut body["workoutSegments"][0]["workoutSteps"][0]["workoutSteps"][0];
        step["secondaryTargetType"] = json!({"workoutTargetTypeId":3});
        assert!(!workout.matches(&body).unwrap());
    }
    #[test]
    fn rejects_invalid_and_excessive_workouts_before_writes() {
        let mut workout = RunningWorkout {
            name: "test".into(),
            description: "test".into(),
            steps: vec![],
        };
        assert_eq!(workout.payload(), Err(GarminError::InvalidRequest));
        let timed = RunningStep::Timed {
            seconds: 60,
            role: StepRole::Other,
            target: RunningTarget::Open,
        };
        workout.steps = vec![RunningStep::Repeat {
            count: 101,
            steps: vec![timed.clone()],
        }];
        assert_eq!(workout.payload(), Err(GarminError::InvalidRequest));
        workout.steps = vec![RunningStep::Timed {
            seconds: 60,
            role: StepRole::Work,
            target: RunningTarget::PaceRange {
                slow_mps: 3.0,
                fast_mps: 2.0,
            },
        }];
        assert_eq!(workout.payload(), Err(GarminError::InvalidRequest));
        workout.steps = vec![RunningStep::Repeat {
            count: 100,
            steps: vec![timed.clone(), timed],
        }];
        assert_eq!(workout.payload(), Err(GarminError::InvalidRequest));
    }
}
