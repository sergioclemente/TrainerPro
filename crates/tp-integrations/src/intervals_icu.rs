//! Intervals.icu HTTP client and provider-native athlete/calendar payloads.
//! Callers own credentials, workout interpretation, date windows, and persistence.

use std::time::Duration;

use reqwest::StatusCode;
use serde::Deserialize;

const WORKOUT_CATEGORY: &str = "WORKOUT";
pub const PROVIDER_ID: &str = "intervals_icu";
const API_BASE_URL: &str = "https://intervals.icu";
const API_KEY_USERNAME: &str = "API_KEY";
const REQUEST_TIMEOUT_SECONDS: u64 = 30;
const ATHLETE_PATH: &str = "/api/v1/athlete/0";
const CALENDAR_EVENTS_PATH: &str = "/api/v1/athlete/0/events";

pub struct IntervalsIcuClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum IntervalsApiError {
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
    /// Create a production client identified by the calling application's user-agent.
    pub fn new(user_agent: &str) -> Result<Self, IntervalsApiError> {
        Self::with_base_url(API_BASE_URL, user_agent)
    }

    fn with_base_url(base_url: &str, user_agent: &str) -> Result<Self, IntervalsApiError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .user_agent(user_agent)
            .build()
            .map_err(IntervalsApiError::Client)?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Validate a personal API key and obtain the non-secret account metadata
    /// required to scope provider identity and timed schedule placement.
    pub async fn fetch_athlete(
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
    /// retains provider semantics for the caller to interpret.
    pub async fn fetch_workout_events(
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
pub struct IntervalsAthlete {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub timezone: String,
}

impl IntervalsAthlete {
    pub fn display_name(&self) -> Option<&str> {
        let name = self.name.trim();
        (!name.is_empty()).then_some(name)
    }
}

/// Check the provider's YYYY-MM-DD wire shape, without calendar-date validation.
pub fn is_iso_date(value: &str) -> bool {
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
pub struct IntervalsCalendarEvent {
    pub id: i64,
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
    pub workout_doc: Option<IntervalsWorkoutDoc>,
}

#[derive(Debug, Deserialize)]
pub struct IntervalsWorkoutDoc {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub steps: Vec<IntervalsStep>,
}

#[derive(Debug, Deserialize)]
pub struct IntervalsStep {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub distance: Option<f64>,
    #[serde(default)]
    pub until_lap_press: bool,
    #[serde(default)]
    pub reps: Option<u32>,
    #[serde(default)]
    pub steps: Vec<IntervalsStep>,
    #[serde(default)]
    pub ramp: bool,
    #[serde(default)]
    pub freeride: bool,
    #[serde(default)]
    pub power: Option<IntervalsTarget>,
    #[serde(default)]
    pub cadence: Option<IntervalsTarget>,
    #[serde(default)]
    pub hr: Option<IntervalsTarget>,
    #[serde(default)]
    pub pace: Option<IntervalsTarget>,
}

#[derive(Debug, Deserialize)]
pub struct IntervalsTarget {
    pub units: String,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
}

// Private endpoint injection keeps HTTP tests local; payload tests use the public API.
#[cfg(test)]
mod tests {
    use super::*;
    const TEST_USER_AGENT: &str = "OtherApp/1.0";
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

    #[tokio::test]
    async fn fetches_bounded_structured_workouts_with_basic_auth() {
        let body = format!(
            "[{}]",
            include_str!("../../../testdata/providers/intervals-icu/scheduled-virtual-ride.json")
        );
        let (base_url, request) = stub(http("200 OK", &body));
        let client = IntervalsIcuClient::with_base_url(&base_url, TEST_USER_AGENT).unwrap();

        let events = client
            .fetch_workout_events("synthetic-key", "2030-01-01", "2030-01-07")
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, 1_000_000);
        let request = request.join().unwrap();
        assert!(request.starts_with(
            "GET /api/v1/athlete/0/events?category=WORKOUT&oldest=2030-01-01&newest=2030-01-07"
        ));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: basic qvbjx0tfwtpzew50agv0awmta2v5"));
        assert!(!request.contains("resolve="));
        assert!(!request.contains("ext="));
        assert!(request.to_ascii_lowercase().contains(&format!(
            "user-agent: {}",
            TEST_USER_AGENT.to_ascii_lowercase()
        )));
    }

    #[tokio::test]
    async fn fetches_authenticated_athlete_identity_and_timezone() {
        let body = r#"{"id":"i123","name":"Ada Rider","timezone":"Europe/Zurich"}"#;
        let (base_url, request) = stub(http("200 OK", body));
        let client = IntervalsIcuClient::with_base_url(&base_url, TEST_USER_AGENT).unwrap();

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
        let client = IntervalsIcuClient::with_base_url(&base_url, TEST_USER_AGENT).unwrap();

        assert!(matches!(
            client
                .fetch_workout_events("synthetic-key", "2030-01-01", "2030-01-07")
                .await,
            Err(IntervalsApiError::Unauthorized)
        ));
        request.join().unwrap();
    }

    #[tokio::test]
    async fn athlete_metadata_requires_a_timezone() {
        let (base_url, request) = stub(http("200 OK", r#"{"id":"i123","timezone":""}"#));
        let client = IntervalsIcuClient::with_base_url(&base_url, TEST_USER_AGENT).unwrap();

        assert!(matches!(
            client.fetch_athlete("synthetic-key").await,
            Err(IntervalsApiError::InvalidAthlete { .. })
        ));
        request.join().unwrap();
    }
}
