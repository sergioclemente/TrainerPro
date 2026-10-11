//! Decode the standard activity subset without filesystem, clock, or timezone access.
//! fitparser owns binary framing, definitions, base types, invalid sentinels and CRCs.
//! We interpret raw standard fields explicitly: profile component expansion must not
//! replace an explicitly recorded enhanced speed or confuse developer fields with it.

use super::profile::*;
use fitparser::{
    de::{FitDataMessage, FitObject, FitStreamProcessor},
    Value,
};
use serde::{Deserialize, Serialize};

pub const FIT_DECODER_VERSION: &str = "tp-fit-activity/1;fitparser/0.11.0";
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_RECORDS: usize = 200_000;
const MAX_MESSAGES: usize = 500_000;
// FIT timestamps are whole seconds; session durations have millisecond precision.
const TIMESTAMP_TOLERANCE_SECONDS: f64 = 1.0;
const RUNNING_STEPS_PER_CYCLE: f64 = 2.0;
const TIMESTAMP: u8 = 253;
const MILLISECONDS_PER_SECOND: f64 = 1000.0;
const SPORT_RUNNING: f64 = 1.0;
const ENHANCED_SPEED: u8 = 73;
const FRACTIONAL_CADENCE: u8 = 53;
const FRACTIONAL_CADENCE_SCALE: f64 = 128.0;
const MSG_FIELD_DESCRIPTION: u16 = 206;
const COMPRESSED_TIMESTAMP_MASK: u32 = 31;
const COMPRESSED_TIMESTAMP_PERIOD: u32 = 32;

#[derive(Debug, thiserror::Error)]
pub enum FitDecodeError {
    #[error("Invalid FIT framing, checksum, or message encoding")]
    InvalidEncoding,
    #[error("FIT activity exceeds decoding limits")]
    LimitExceeded,
    #[error("Expected one cycling or running FIT activity session")]
    UnsupportedActivity,
    #[error("FIT activity has missing or inconsistent timing")]
    InvalidTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitSport {
    Cycling,
    Running,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitSummary {
    pub elapsed_seconds: Option<f64>,
    pub timer_seconds: Option<f64>,
    pub distance_meters: Option<f64>,
    pub mean_power_watts: Option<f64>,
    pub mean_heart_rate_bpm: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedRecord {
    pub elapsed_seconds: f64,
    pub power_watts: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    pub speed_meters_per_second: Option<f64>,
    /// Cycling revolutions/minute or running steps/minute (both feet).
    pub cadence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedLap {
    pub number: usize,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub usable_window_bounds: bool,
    pub summary: FitSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerState {
    Active,
    Paused,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerEvent {
    pub elapsed_seconds: f64,
    pub state: TimerState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedActivity {
    pub start_unix_seconds: i64,
    pub duration_seconds: f64,
    pub sport: FitSport,
    pub summary: FitSummary,
    pub records: Vec<DecodedRecord>,
    pub laps: Vec<DecodedLap>,
    pub timer_events: Vec<TimerEvent>,
}

fn number(message: &FitDataMessage, field: u8) -> Option<f64> {
    let value = match message.fields().get(&field)? {
        Value::Enum(v) | Value::UInt8(v) | Value::Byte(v) => f64::from(*v),
        Value::UInt16(v) => f64::from(*v),
        Value::UInt32(v) | Value::UInt32z(v) => f64::from(*v),
        Value::Float32(v) => f64::from(*v),
        Value::Float64(v) => *v,
        _ => return None,
    };
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn timestamp(message: &FitDataMessage, field: u8) -> Option<u32> {
    match message.fields().get(&field) {
        Some(Value::UInt32(v)) => Some(*v),
        _ => None,
    }
}

fn summary(message: &FitDataMessage, lap: bool) -> FitSummary {
    FitSummary {
        elapsed_seconds: number(message, SESSION_TOTAL_ELAPSED_TIME)
            .map(|v| v / MILLISECONDS_PER_SECOND),
        timer_seconds: number(message, SESSION_TOTAL_TIMER_TIME)
            .map(|v| v / MILLISECONDS_PER_SECOND),
        distance_meters: number(message, SESSION_TOTAL_DISTANCE).map(|v| v / DISTANCE_SCALE),
        mean_power_watts: number(
            message,
            if lap {
                LAP_AVG_POWER
            } else {
                SESSION_AVG_POWER
            },
        ),
        mean_heart_rate_bpm: number(
            message,
            if lap {
                LAP_AVG_HEART_RATE
            } else {
                SESSION_AVG_HEART_RATE
            },
        ),
    }
}

/// Decode exactly one CRC-validated FIT activity. No partial result on failure.
/// Unknown messages are skipped; packed components, developer measurements, GPS,
/// device identifiers and separate HR streams are not part of this activity view.
pub fn decode_activity(bytes: &[u8]) -> Result<DecodedActivity, FitDecodeError> {
    use FitDecodeError::*;
    if bytes.len() > MAX_BYTES {
        return Err(LimitExceeded);
    }
    if bytes.len() < 14 || !matches!(bytes[0], 12 | 14) || &bytes[8..12] != b".FIT" {
        return Err(InvalidEncoding);
    }
    let data_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if usize::from(bytes[0])
        .checked_add(data_size)
        .and_then(|n| n.checked_add(2))
        != Some(bytes.len())
    {
        return Err(InvalidEncoding);
    }
    let mut processor = FitStreamProcessor::new();
    let mut remaining = bytes;
    let mut session = None;
    let mut files = 0;
    let mut records = Vec::new();
    let mut laps = Vec::new();
    let mut events = Vec::new();
    let mut base_timestamp: Option<u32> = None;
    let mut crc_seen = false;
    let mut messages = 0;
    while !remaining.is_empty() {
        let (rest, object) = processor
            .deserialize_next(remaining)
            .map_err(|_| InvalidEncoding)?;
        if rest.len() >= remaining.len() {
            return Err(InvalidEncoding);
        }
        remaining = rest;
        match object {
            FitObject::Crc(_) => crc_seen = true,
            FitObject::DataMessage(mut message) => {
                messages += 1;
                if messages > MAX_MESSAGES {
                    return Err(LimitExceeded);
                }
                if let Some(time) = timestamp(&message, TIMESTAMP) {
                    base_timestamp = Some(time);
                }
                if let Some(offset) = message.time_offset() {
                    let base = base_timestamp.ok_or(InvalidTiming)?;
                    let offset = u32::from(offset);
                    let time = (base & !COMPRESSED_TIMESTAMP_MASK) + offset;
                    let time = if offset < base & COMPRESSED_TIMESTAMP_MASK {
                        time.checked_add(COMPRESSED_TIMESTAMP_PERIOD)
                            .ok_or(InvalidTiming)?
                    } else {
                        time
                    };
                    message.fields_mut().insert(TIMESTAMP, Value::UInt32(time));
                    base_timestamp = Some(time);
                }
                match message.global_message_number() {
                    MSG_FILE_ID => {
                        files += 1;
                        if number(&message, FILE_ID_TYPE) != Some(f64::from(FILE_TYPE_ACTIVITY)) {
                            return Err(UnsupportedActivity);
                        }
                    }
                    MSG_SESSION => {
                        if session.replace(message).is_some() {
                            return Err(UnsupportedActivity);
                        }
                    }
                    MSG_RECORD => {
                        if records.len() >= MAX_RECORDS {
                            return Err(LimitExceeded);
                        }
                        records.push(message);
                    }
                    MSG_LAP => laps.push(message),
                    MSG_EVENT if number(&message, EVENT_EVENT) == Some(f64::from(EVENT_TIMER)) => {
                        events.push(message)
                    }
                    // Register developer field types so the binary parser can consume them.
                    // Field-description messages have no date/time fields; no local timezone
                    // conversion or general profile/component expansion is invoked.
                    MSG_FIELD_DESCRIPTION => {
                        processor
                            .decode_message(message)
                            .map_err(|_| InvalidEncoding)?;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if !crc_seen {
        return Err(InvalidEncoding);
    }
    let session = session.ok_or(UnsupportedActivity)?;
    if files != 1 || records.is_empty() {
        return Err(UnsupportedActivity);
    }
    let sport = match number(&session, SESSION_SPORT) {
        Some(v) if v == f64::from(SPORT_CYCLING) => FitSport::Cycling,
        Some(SPORT_RUNNING) => FitSport::Running,
        _ => return Err(UnsupportedActivity),
    };
    let start = timestamp(&session, SESSION_START_TIME).ok_or(InvalidTiming)?;
    let summary = summary(&session, false);
    let duration = summary
        .elapsed_seconds
        .filter(|v| *v > 0.0)
        .ok_or(InvalidTiming)?;
    let elapsed = |time: u32| f64::from(time) - f64::from(start);
    let mut previous = -1.0;
    let records = records
        .iter()
        .map(|message| {
            let offset = elapsed(timestamp(message, TIMESTAMP).ok_or(InvalidTiming)?);
            if offset < 0.0 || offset > duration + TIMESTAMP_TOLERANCE_SECONDS || offset <= previous
            {
                return Err(InvalidTiming);
            }
            previous = offset;
            let cadence = number(message, RECORD_CADENCE).map(|v| {
                let cycles = v + number(message, FRACTIONAL_CADENCE).unwrap_or(0.0)
                    / FRACTIONAL_CADENCE_SCALE;
                match sport {
                    FitSport::Cycling => cycles,
                    FitSport::Running => cycles * RUNNING_STEPS_PER_CYCLE,
                }
            });
            Ok(DecodedRecord {
                elapsed_seconds: offset,
                power_watts: number(message, RECORD_POWER),
                heart_rate_bpm: number(message, RECORD_HEART_RATE),
                speed_meters_per_second: number(message, ENHANCED_SPEED)
                    .or_else(|| number(message, RECORD_SPEED))
                    .map(|v| v / SPEED_SCALE),
                cadence,
            })
        })
        .collect::<Result<Vec<_>, FitDecodeError>>()?;
    let laps = laps
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let summary = self::summary(message, true);
            let start_seconds = timestamp(message, LAP_START_TIME).map(elapsed);
            let end_seconds = start_seconds
                .zip(summary.elapsed_seconds)
                .map(|(s, d)| s + d);
            DecodedLap {
                number: index + 1,
                start_seconds,
                end_seconds,
                usable_window_bounds: start_seconds
                    .zip(end_seconds)
                    .is_some_and(|(s, e)| 0.0 <= s && s < e && e <= duration),
                summary,
            }
        })
        .collect();
    let mut timer_events = events
        .iter()
        .map(|message| {
            let offset = elapsed(timestamp(message, TIMESTAMP).ok_or(InvalidTiming)?);
            if offset < 0.0 || offset > duration + TIMESTAMP_TOLERANCE_SECONDS {
                return Err(InvalidTiming);
            }
            let state = match number(message, EVENT_EVENT_TYPE) {
                Some(0.0) => TimerState::Active,
                Some(1.0 | 4.0 | 8.0 | 9.0) => TimerState::Paused,
                _ => TimerState::Unknown,
            };
            Ok(TimerEvent {
                elapsed_seconds: offset,
                state,
            })
        })
        .collect::<Result<Vec<_>, FitDecodeError>>()?;
    // Stable order preserves multiple transitions at the same timestamp.
    timer_events.sort_by(|a, b| a.elapsed_seconds.total_cmp(&b.elapsed_seconds));
    Ok(DecodedActivity {
        start_unix_seconds: i64::from(start) + crate::consts::FIT_EPOCH_OFFSET_S as i64,
        duration_seconds: duration,
        sport,
        summary,
        records,
        laps,
        timer_events,
    })
}
