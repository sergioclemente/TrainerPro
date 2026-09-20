//! Intervals.icu calendar payloads and their provider-specific TPW adapter.
//!
//! Network, credential, and sync policy do not belong here. This module maps
//! the structured `workout_doc` returned by the provider into TrainerPro's
//! semantic workout model without reparsing the event description.

use std::time::Duration;

use reqwest::StatusCode;
use serde::Deserialize;
use tp_core::consts::TEXT_EVENT_DEFAULT_S;
use tp_core::workout_definition::{
    CyclingCadenceTarget, CyclingPowerTarget, CyclingStep, WorkoutCue, WorkoutDefinition,
    WorkoutDefinitionError, WorkoutFormat, WorkoutPrescription, TPW_VERSION,
};

const WORKOUT_CATEGORY: &str = "WORKOUT";
const RIDE_TYPE: &str = "Ride";
const VIRTUAL_RIDE_TYPE: &str = "VirtualRide";
pub(crate) const PROVIDER_ID: &str = "intervals_icu";
const API_BASE_URL: &str = "https://intervals.icu";
const API_KEY_USERNAME: &str = "API_KEY";
const REQUEST_TIMEOUT_SECONDS: u64 = 30;
const ATHLETE_PATH: &str = "/api/v1/athlete/0";
const CALENDAR_EVENTS_PATH: &str = "/api/v1/athlete/0/events";

pub(crate) struct IntervalsIcuClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IntervalsApiError {
    #[error("Intervals.icu API key is blank")]
    MissingApiKey,
    #[error(
        "invalid Intervals.icu calendar range {oldest_date_local:?} through {newest_date_local:?}; expected ordered YYYY-MM-DD dates"
    )]
    InvalidDateRange {
        oldest_date_local: String,
        newest_date_local: String,
    },
    #[error("could not create the Intervals.icu HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("could not reach Intervals.icu: {0}")]
    Request(#[source] reqwest::Error),
    #[error("Intervals.icu rejected the API credentials")]
    Unauthorized,
    #[error("Intervals.icu returned HTTP {status}")]
    HttpStatus { status: StatusCode },
    #[error("Intervals.icu returned an invalid calendar response: {0}")]
    InvalidResponse(#[source] reqwest::Error),
    #[error("Intervals.icu returned invalid athlete metadata: {message}")]
    InvalidAthlete { message: String },
}

impl IntervalsIcuClient {
    pub(crate) fn new() -> Result<Self, IntervalsApiError> {
        Self::with_base_url(API_BASE_URL)
    }

    fn with_base_url(base_url: &str) -> Result<Self, IntervalsApiError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .user_agent(concat!("TrainerPro/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(IntervalsApiError::Client)?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Validate a personal API key and obtain the non-secret account metadata
    /// required to scope provider identity and timed schedule placement.
    pub(crate) async fn fetch_athlete(
        &self,
        api_key: &str,
    ) -> Result<IntervalsAthlete, IntervalsApiError> {
        if api_key.trim().is_empty() {
            return Err(IntervalsApiError::MissingApiKey);
        }
        let response = self
            .http
            .get(format!("{}{}", self.base_url, ATHLETE_PATH))
            .basic_auth(API_KEY_USERNAME, Some(api_key))
            .send()
            .await
            .map_err(IntervalsApiError::Request)?;
        let athlete: IntervalsAthlete = parse_response(response).await?;
        if athlete.id.trim().is_empty() {
            return Err(IntervalsApiError::InvalidAthlete {
                message: "missing athlete id".into(),
            });
        }
        if athlete.timezone.trim().is_empty() {
            return Err(IntervalsApiError::InvalidAthlete {
                message: "missing athlete timezone".into(),
            });
        }
        Ok(athlete)
    }

    /// Fetch a bounded calendar window without asking Intervals.icu to resolve
    /// relative targets or attach a workout file. The structured `workout_doc`
    /// remains the source consumed by `to_workout_definition`.
    pub(crate) async fn fetch_workout_events(
        &self,
        api_key: &str,
        oldest_date_local: &str,
        newest_date_local: &str,
    ) -> Result<Vec<IntervalsCalendarEvent>, IntervalsApiError> {
        if api_key.trim().is_empty() {
            return Err(IntervalsApiError::MissingApiKey);
        }
        if !is_iso_date(oldest_date_local)
            || !is_iso_date(newest_date_local)
            || oldest_date_local > newest_date_local
        {
            return Err(IntervalsApiError::InvalidDateRange {
                oldest_date_local: oldest_date_local.to_string(),
                newest_date_local: newest_date_local.to_string(),
            });
        }

        let response = self
            .http
            .get(format!("{}{}", self.base_url, CALENDAR_EVENTS_PATH))
            .basic_auth(API_KEY_USERNAME, Some(api_key))
            .query(&[
                ("category", WORKOUT_CATEGORY),
                ("oldest", oldest_date_local),
                ("newest", newest_date_local),
            ])
            .send()
            .await
            .map_err(IntervalsApiError::Request)?;

        parse_response(response).await
    }
}

async fn parse_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, IntervalsApiError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(IntervalsApiError::Unauthorized),
        status if !status.is_success() => Err(IntervalsApiError::HttpStatus { status }),
        _ => response
            .json::<T>()
            .await
            .map_err(IntervalsApiError::InvalidResponse),
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct IntervalsAthlete {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub timezone: String,
}

impl IntervalsAthlete {
    pub(crate) fn display_name(&self) -> Option<&str> {
        let name = self.name.trim();
        (!name.is_empty()).then_some(name)
    }
}

pub(crate) fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

#[derive(Debug, Deserialize)]
pub(crate) struct IntervalsCalendarEvent {
    pub id: i64,
    #[cfg(test)]
    #[serde(default)]
    pub uid: Option<String>,
    #[cfg(test)]
    #[serde(default)]
    pub external_id: Option<serde_json::Value>,
    #[serde(default)]
    pub updated: Option<String>,
    pub start_date_local: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub category: String,
    pub name: String,
    #[serde(default)]
    pub moving_time: Option<u32>,
    #[serde(default)]
    workout_doc: Option<IntervalsWorkoutDoc>,
}

#[derive(Debug, Deserialize)]
struct IntervalsWorkoutDoc {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    duration: Option<u32>,
    #[serde(default)]
    steps: Vec<IntervalsStep>,
}

#[derive(Debug, Deserialize)]
struct IntervalsStep {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    duration: Option<u32>,
    #[serde(default)]
    distance: Option<f64>,
    #[serde(default)]
    until_lap_press: bool,
    #[serde(default)]
    reps: Option<u32>,
    #[serde(default)]
    steps: Vec<IntervalsStep>,
    #[serde(default)]
    ramp: bool,
    #[serde(default)]
    freeride: bool,
    #[serde(default)]
    power: Option<IntervalsTarget>,
    #[serde(default)]
    cadence: Option<IntervalsTarget>,
    #[serde(default)]
    hr: Option<IntervalsTarget>,
    #[serde(default)]
    pace: Option<IntervalsTarget>,
}

#[derive(Debug, Deserialize)]
struct IntervalsTarget {
    units: String,
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IntervalsWorkoutError {
    #[error("Intervals.icu event {event_id} has category {category:?}, not WORKOUT")]
    NotWorkout { event_id: i64, category: String },
    #[error("Intervals.icu event {event_id} has unsupported sport type {event_type:?}")]
    UnsupportedSport { event_id: i64, event_type: String },
    #[error("Intervals.icu event {event_id} has no structured workout_doc")]
    MissingWorkoutDoc { event_id: i64 },
    #[error("invalid Intervals.icu workout at {path}: {message}")]
    Invalid { path: String, message: String },
    #[error(
        "Intervals.icu event {event_id} {duration_source} duration is {provider_seconds}s, but its steps total {definition_seconds}s"
    )]
    DurationMismatch {
        event_id: i64,
        duration_source: &'static str,
        provider_seconds: u32,
        definition_seconds: u32,
    },
    #[error("Intervals.icu workout produced invalid TPW: {0}")]
    InvalidDefinition(#[from] WorkoutDefinitionError),
}

impl IntervalsCalendarEvent {
    pub(crate) fn to_workout_definition(&self) -> Result<WorkoutDefinition, IntervalsWorkoutError> {
        if self.category != WORKOUT_CATEGORY {
            return Err(IntervalsWorkoutError::NotWorkout {
                event_id: self.id,
                category: self.category.clone(),
            });
        }
        if !matches!(self.event_type.as_str(), RIDE_TYPE | VIRTUAL_RIDE_TYPE) {
            return Err(IntervalsWorkoutError::UnsupportedSport {
                event_id: self.id,
                event_type: self.event_type.clone(),
            });
        }

        let doc = self
            .workout_doc
            .as_ref()
            .ok_or(IntervalsWorkoutError::MissingWorkoutDoc { event_id: self.id })?;
        let definition = WorkoutDefinition {
            format: WorkoutFormat::Tpw,
            version: TPW_VERSION,
            title: self.name.trim().to_string(),
            description: doc
                .description
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            training_focus: None,
            prescription: WorkoutPrescription::Cycling {
                steps: convert_steps(&doc.steps, "workout_doc.steps")?,
            },
        };
        let definition_seconds = definition.duration_seconds()?;
        check_duration(self.id, "workout_doc", doc.duration, definition_seconds)?;
        check_duration(self.id, "moving_time", self.moving_time, definition_seconds)?;
        Ok(definition)
    }
}

fn check_duration(
    event_id: i64,
    duration_source: &'static str,
    provider_seconds: Option<u32>,
    definition_seconds: u32,
) -> Result<(), IntervalsWorkoutError> {
    if let Some(provider_seconds) = provider_seconds {
        if provider_seconds != definition_seconds {
            return Err(IntervalsWorkoutError::DurationMismatch {
                event_id,
                duration_source,
                provider_seconds,
                definition_seconds,
            });
        }
    }
    Ok(())
}

fn convert_steps(
    steps: &[IntervalsStep],
    path: &str,
) -> Result<Vec<CyclingStep>, IntervalsWorkoutError> {
    steps
        .iter()
        .enumerate()
        .map(|(index, step)| convert_step(step, &format!("{path}[{index}]")))
        .collect()
}

fn convert_step(step: &IntervalsStep, path: &str) -> Result<CyclingStep, IntervalsWorkoutError> {
    reject_unsupported_step_semantics(step, path)?;

    if let Some(count) = step.reps {
        if step.steps.is_empty() {
            return invalid(
                format!("{path}.steps"),
                "repeat must contain at least one step",
            );
        }
        if step.freeride || step.ramp || step.power.is_some() || step.cadence.is_some() {
            return invalid(path, "repeat must not also define leaf-step targets");
        }
        return Ok(CyclingStep::Repeat {
            count,
            steps: convert_steps(&step.steps, &format!("{path}.steps"))?,
        });
    }

    if !step.steps.is_empty() {
        return invalid(format!("{path}.steps"), "nested steps require reps");
    }
    let duration_seconds = step
        .duration
        .ok_or_else(|| invalid_error(format!("{path}.duration"), "is required"))?;
    let cues = cues(step);

    if step.freeride {
        if step.ramp || step.power.is_some() || step.cadence.is_some() {
            return invalid(
                path,
                "freeride must not also define ramp, power, or cadence",
            );
        }
        return Ok(CyclingStep::FreeRide {
            duration_seconds,
            cues,
        });
    }

    let power = step
        .power
        .as_ref()
        .ok_or_else(|| invalid_error(format!("{path}.power"), "is required"))?;
    let cadence = step
        .cadence
        .as_ref()
        .map(|target| convert_cadence(target, &format!("{path}.cadence")))
        .transpose()?;

    if step.ramp {
        let (start_power, end_power) = convert_ramp_power(power, &format!("{path}.power"))?;
        Ok(CyclingStep::Ramp {
            duration_seconds,
            start_power,
            end_power,
            cadence,
            cues,
        })
    } else {
        Ok(CyclingStep::Steady {
            duration_seconds,
            power: convert_power(power, &format!("{path}.power"))?,
            cadence,
            cues,
        })
    }
}

fn reject_unsupported_step_semantics(
    step: &IntervalsStep,
    path: &str,
) -> Result<(), IntervalsWorkoutError> {
    if step.distance.is_some_and(|distance| distance != 0.0) {
        return invalid(
            format!("{path}.distance"),
            "distance-based steps are not supported",
        );
    }
    if step.until_lap_press {
        return invalid(
            format!("{path}.until_lap_press"),
            "open-duration steps are not supported",
        );
    }
    if step.hr.is_some() {
        return invalid(format!("{path}.hr"), "heart-rate targets are not supported");
    }
    if step.pace.is_some() {
        return invalid(
            format!("{path}.pace"),
            "pace targets are not supported for cycling",
        );
    }
    Ok(())
}

fn cues(step: &IntervalsStep) -> Vec<WorkoutCue> {
    step.text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|message| WorkoutCue {
            offset_seconds: 0,
            message: message.to_string(),
            display_seconds: TEXT_EVENT_DEFAULT_S,
        })
        .into_iter()
        .collect()
}

fn convert_power(
    target: &IntervalsTarget,
    path: &str,
) -> Result<CyclingPowerTarget, IntervalsWorkoutError> {
    match target_shape(target, path)? {
        TargetShape::Exact(value) => match target.units.as_str() {
            "%ftp" => Ok(CyclingPowerTarget::PercentFtp { percent: value }),
            "w" => Ok(CyclingPowerTarget::Watts {
                watts: whole_u16(value, &format!("{path}.value"))?,
            }),
            units => invalid(
                format!("{path}.units"),
                format!("unsupported power units {units:?}"),
            ),
        },
        TargetShape::Range { start, end } => match target.units.as_str() {
            "%ftp" => Ok(CyclingPowerTarget::PercentFtpRange {
                min_percent: start,
                max_percent: end,
            }),
            "w" => Ok(CyclingPowerTarget::WattsRange {
                min_watts: whole_u16(start, &format!("{path}.start"))?,
                max_watts: whole_u16(end, &format!("{path}.end"))?,
            }),
            units => invalid(
                format!("{path}.units"),
                format!("unsupported power units {units:?}"),
            ),
        },
    }
}

fn convert_ramp_power(
    target: &IntervalsTarget,
    path: &str,
) -> Result<(CyclingPowerTarget, CyclingPowerTarget), IntervalsWorkoutError> {
    let TargetShape::Range { start, end } = target_shape(target, path)? else {
        return invalid(path, "ramp power must define start and end values");
    };
    match target.units.as_str() {
        "%ftp" => Ok((
            CyclingPowerTarget::PercentFtp { percent: start },
            CyclingPowerTarget::PercentFtp { percent: end },
        )),
        "w" => Ok((
            CyclingPowerTarget::Watts {
                watts: whole_u16(start, &format!("{path}.start"))?,
            },
            CyclingPowerTarget::Watts {
                watts: whole_u16(end, &format!("{path}.end"))?,
            },
        )),
        units => invalid(
            format!("{path}.units"),
            format!("unsupported power units {units:?}"),
        ),
    }
}

fn convert_cadence(
    target: &IntervalsTarget,
    path: &str,
) -> Result<CyclingCadenceTarget, IntervalsWorkoutError> {
    if target.units != "rpm" {
        return invalid(
            format!("{path}.units"),
            format!("unsupported cadence units {:?}", target.units),
        );
    }
    match target_shape(target, path)? {
        TargetShape::Exact(value) => Ok(CyclingCadenceTarget::Exact {
            rpm: whole_u16(value, &format!("{path}.value"))?,
        }),
        TargetShape::Range { start, end } => Ok(CyclingCadenceTarget::Range {
            min_rpm: whole_u16(start, &format!("{path}.start"))?,
            max_rpm: whole_u16(end, &format!("{path}.end"))?,
        }),
    }
}

enum TargetShape {
    Exact(f64),
    Range { start: f64, end: f64 },
}

fn target_shape(
    target: &IntervalsTarget,
    path: &str,
) -> Result<TargetShape, IntervalsWorkoutError> {
    match (target.value, target.start, target.end) {
        (Some(value), None, None) => Ok(TargetShape::Exact(value)),
        (None, Some(start), Some(end)) => Ok(TargetShape::Range { start, end }),
        (None, None, None) => invalid(path, "target must define value or start and end"),
        _ => invalid(
            path,
            "target must define either value or start and end, not both",
        ),
    }
}

fn whole_u16(value: f64, path: &str) -> Result<u16, IntervalsWorkoutError> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u16::MAX) || value.fract() != 0.0 {
        return invalid(
            path,
            format!("{value} must be a whole number in the u16 range"),
        );
    }
    Ok(value as u16)
}

fn invalid<T>(
    path: impl Into<String>,
    message: impl Into<String>,
) -> Result<T, IntervalsWorkoutError> {
    Err(invalid_error(path, message))
}

fn invalid_error(path: impl Into<String>, message: impl Into<String>) -> IntervalsWorkoutError {
    IntervalsWorkoutError::Invalid {
        path: path.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn stub(response: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 8192];
            let count = socket.read(&mut buffer).unwrap();
            socket.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&buffer[..count]).into_owned()
        });
        (base_url, handle)
    }

    fn http(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn event_with_steps(steps: Vec<IntervalsStep>, duration: u32) -> IntervalsCalendarEvent {
        IntervalsCalendarEvent {
            id: 7,
            uid: Some("fixture-uid".into()),
            external_id: None,
            updated: Some("2030-01-01T00:00:00.000+0000".into()),
            start_date_local: "2030-01-01T00:00:00".into(),
            event_type: VIRTUAL_RIDE_TYPE.into(),
            category: WORKOUT_CATEGORY.into(),
            name: "Fixture".into(),
            moving_time: Some(duration),
            workout_doc: Some(IntervalsWorkoutDoc {
                description: None,
                duration: Some(duration),
                steps,
            }),
        }
    }

    fn leaf_step(power: IntervalsTarget) -> IntervalsStep {
        IntervalsStep {
            text: None,
            duration: Some(60),
            distance: None,
            until_lap_press: false,
            reps: None,
            steps: Vec::new(),
            ramp: false,
            freeride: false,
            power: Some(power),
            cadence: None,
            hr: None,
            pace: None,
        }
    }

    #[test]
    fn production_client_uses_intervals_base_url() {
        let client = IntervalsIcuClient::new().unwrap();
        assert_eq!(client.base_url, API_BASE_URL);
    }

    #[tokio::test]
    async fn fetches_bounded_structured_workouts_with_basic_auth() {
        let body = format!(
            "[{}]",
            include_str!("../../testdata/providers/intervals-icu/scheduled-virtual-ride.json")
        );
        let (base_url, request) = stub(http("200 OK", &body));
        let client = IntervalsIcuClient::with_base_url(&base_url).unwrap();

        let events = client
            .fetch_workout_events("synthetic-key", "2030-01-01", "2030-01-07")
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]
                .to_workout_definition()
                .unwrap()
                .duration_seconds()
                .unwrap(),
            1_740
        );
        let request = request.join().unwrap();
        assert!(request.starts_with(
            "GET /api/v1/athlete/0/events?category=WORKOUT&oldest=2030-01-01&newest=2030-01-07"
        ));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: basic qvbjx0tfwtpzew50agv0awmta2v5"));
        assert!(!request.contains("resolve="));
        assert!(!request.contains("ext="));
        assert!(request
            .to_ascii_lowercase()
            .contains("user-agent: trainerpro/"));
    }

    #[tokio::test]
    async fn fetches_authenticated_athlete_identity_and_timezone() {
        let body = r#"{"id":"i123","name":"Ada Rider","timezone":"Europe/Zurich"}"#;
        let (base_url, request) = stub(http("200 OK", body));
        let client = IntervalsIcuClient::with_base_url(&base_url).unwrap();

        let athlete = client.fetch_athlete("synthetic-key").await.unwrap();

        assert_eq!(
            athlete,
            IntervalsAthlete {
                id: "i123".into(),
                name: "Ada Rider".into(),
                timezone: "Europe/Zurich".into(),
            }
        );
        assert_eq!(athlete.display_name(), Some("Ada Rider"));
        let request = request.join().unwrap();
        assert!(request.starts_with("GET /api/v1/athlete/0 HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: basic qvbjx0tfwtpzew50agv0awmta2v5"));
    }

    #[tokio::test]
    async fn rejected_credentials_have_a_distinct_error() {
        let (base_url, request) = stub(http("401 Unauthorized", "{}"));
        let client = IntervalsIcuClient::with_base_url(&base_url).unwrap();

        assert!(matches!(
            client
                .fetch_workout_events("synthetic-key", "2030-01-01", "2030-01-07")
                .await,
            Err(IntervalsApiError::Unauthorized)
        ));
        request.join().unwrap();
    }

    #[tokio::test]
    async fn blank_keys_and_invalid_ranges_fail_before_network_io() {
        let client = IntervalsIcuClient::with_base_url("http://127.0.0.1:1").unwrap();
        assert!(matches!(
            client.fetch_athlete(" ").await,
            Err(IntervalsApiError::MissingApiKey)
        ));
        assert!(matches!(
            client
                .fetch_workout_events(" ", "2030-01-01", "2030-01-07")
                .await,
            Err(IntervalsApiError::MissingApiKey)
        ));
        assert!(matches!(
            client
                .fetch_workout_events("synthetic-key", "2030-01-07", "2030-01-01")
                .await,
            Err(IntervalsApiError::InvalidDateRange { .. })
        ));
        assert!(matches!(
            client
                .fetch_workout_events("synthetic-key", "20300101", "2030-01-07")
                .await,
            Err(IntervalsApiError::InvalidDateRange { .. })
        ));
    }

    #[tokio::test]
    async fn athlete_metadata_requires_a_timezone() {
        let (base_url, request) = stub(http("200 OK", r#"{"id":"i123","timezone":""}"#));
        let client = IntervalsIcuClient::with_base_url(&base_url).unwrap();

        assert!(matches!(
            client.fetch_athlete("synthetic-key").await,
            Err(IntervalsApiError::InvalidAthlete { .. })
        ));
        request.join().unwrap();
    }

    #[test]
    fn live_virtual_ride_fixture_maps_to_tpw_without_flattening() {
        let event: IntervalsCalendarEvent = serde_json::from_str(include_str!(
            "../../testdata/providers/intervals-icu/scheduled-virtual-ride.json"
        ))
        .unwrap();

        assert_eq!(event.id, 1_000_000);
        assert_eq!(
            event.uid.as_deref(),
            Some("00000000-0000-0000-0000-000000000000")
        );
        assert!(event.external_id.is_none());
        assert!(event.updated.is_some());
        assert_eq!(event.start_date_local, "2030-01-01T00:00:00");

        let definition = event.to_workout_definition().unwrap();
        assert_eq!(definition.title, "TrainerPro API Probe");
        assert_eq!(definition.duration_seconds().unwrap(), 1_740);
        let WorkoutPrescription::Cycling { steps } = &definition.prescription;
        assert_eq!(steps.len(), 5);
        assert!(matches!(
            &steps[0],
            CyclingStep::Ramp {
                start_power: CyclingPowerTarget::PercentFtp { percent: 50.0 },
                end_power: CyclingPowerTarget::PercentFtp { percent: 70.0 },
                cadence: Some(CyclingCadenceTarget::Range {
                    min_rpm: 85,
                    max_rpm: 95
                }),
                ..
            }
        ));
        assert!(matches!(
            &steps[1],
            CyclingStep::Repeat { count: 3, steps } if steps.len() == 2
        ));
        assert!(matches!(
            &steps[2],
            CyclingStep::Steady {
                power: CyclingPowerTarget::PercentFtpRange {
                    min_percent: 75.0,
                    max_percent: 85.0
                },
                cadence: Some(CyclingCadenceTarget::Exact { rpm: 90 }),
                ..
            }
        ));
        assert!(matches!(
            &steps[3],
            CyclingStep::FreeRide {
                duration_seconds: 120,
                ..
            }
        ));
        assert!(matches!(
            &steps[4],
            CyclingStep::Ramp {
                start_power: CyclingPowerTarget::PercentFtp { percent: 65.0 },
                end_power: CyclingPowerTarget::PercentFtp { percent: 45.0 },
                ..
            }
        ));
        assert_eq!(definition.compile().unwrap().segments.len(), 10);
    }

    #[test]
    fn maps_absolute_watts_and_step_text() {
        let mut step = leaf_step(IntervalsTarget {
            units: "w".into(),
            value: None,
            start: Some(200.0),
            end: Some(240.0),
        });
        step.text = Some("  Hold good form  ".into());
        let definition = event_with_steps(vec![step], 60)
            .to_workout_definition()
            .unwrap();
        let WorkoutPrescription::Cycling { steps } = definition.prescription;
        let CyclingStep::Steady { power, cues, .. } = &steps[0] else {
            panic!("expected steady step");
        };
        assert_eq!(
            power,
            &CyclingPowerTarget::WattsRange {
                min_watts: 200,
                max_watts: 240,
            }
        );
        assert_eq!(cues[0].message, "Hold good form");
        assert_eq!(cues[0].display_seconds, TEXT_EVENT_DEFAULT_S);
    }

    #[test]
    fn unsupported_sport_is_distinct_from_malformed_workout() {
        let mut event = event_with_steps(Vec::new(), 0);
        event.event_type = "Run".into();
        assert!(matches!(
            event.to_workout_definition(),
            Err(IntervalsWorkoutError::UnsupportedSport { event_type, .. }) if event_type == "Run"
        ));
    }

    #[test]
    fn unsupported_provider_target_is_not_silently_coerced() {
        let event = event_with_steps(
            vec![leaf_step(IntervalsTarget {
                units: "power_zone".into(),
                value: Some(2.0),
                start: None,
                end: None,
            })],
            60,
        );
        assert!(matches!(
            event.to_workout_definition(),
            Err(IntervalsWorkoutError::Invalid { path, message })
                if path == "workout_doc.steps[0].power.units"
                    && message.contains("power_zone")
        ));
    }

    #[test]
    fn duration_mismatch_is_rejected() {
        let mut event = event_with_steps(
            vec![leaf_step(IntervalsTarget {
                units: "%ftp".into(),
                value: Some(75.0),
                start: None,
                end: None,
            })],
            60,
        );
        event.moving_time = Some(90);
        assert!(matches!(
            event.to_workout_definition(),
            Err(IntervalsWorkoutError::DurationMismatch {
                duration_source: "moving_time",
                provider_seconds: 90,
                definition_seconds: 60,
                ..
            })
        ));
    }

    #[test]
    fn distance_and_open_duration_steps_are_explicitly_unsupported() {
        let power = || IntervalsTarget {
            units: "%ftp".into(),
            value: Some(75.0),
            start: None,
            end: None,
        };
        let mut distance_step = leaf_step(power());
        distance_step.distance = Some(1_000.0);
        let distance_event = event_with_steps(vec![distance_step], 60);
        assert!(matches!(
            distance_event.to_workout_definition(),
            Err(IntervalsWorkoutError::Invalid { path, .. })
                if path == "workout_doc.steps[0].distance"
        ));

        let mut open_step = leaf_step(power());
        open_step.until_lap_press = true;
        let open_event = event_with_steps(vec![open_step], 60);
        assert!(matches!(
            open_event.to_workout_definition(),
            Err(IntervalsWorkoutError::Invalid { path, .. })
                if path == "workout_doc.steps[0].until_lap_press"
        ));
    }
}
