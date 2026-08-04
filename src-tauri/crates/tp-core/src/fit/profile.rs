//! FIT profile subset: global message numbers, field numbers, base types,
//! scales, and enum values for the 7 message types we write (SPEC.md §7.2
//! table). Values implemented per the spec table (which matches the FIT SDK
//! Profile for these messages); Garmin's online docs are JS-rendered and not
//! machine-verifiable from here — the FitCSVTool CI gate (§7.3) is the
//! authoritative check. Keep this file data-only (consts and small enums);
//! encoding logic lives in encode.rs.

// ---------------------------------------------------------------------------
// Container / header
// ---------------------------------------------------------------------------

/// Header length in bytes.
pub const HEADER_SIZE: u8 = 14;
/// Protocol version 2.0 (major<<4 | minor).
pub const PROTOCOL_VERSION: u8 = 0x20;
/// Profile version the encoder targets (21.158 → 21158, SDK convention).
pub const PROFILE_VERSION: u16 = 21_158;
/// Literal data-type marker in the header.
pub const DATA_TYPE: &[u8; 4] = b".FIT";

/// Record-header bit marking a definition message.
pub const DEFINITION_HEADER_BIT: u8 = 0x40;
/// Architecture byte in definition records: 0 = little-endian.
pub const ARCH_LITTLE_ENDIAN: u8 = 0;

// ---------------------------------------------------------------------------
// Base types (code as written into definition records) + invalid values
// ---------------------------------------------------------------------------

pub const BASE_ENUM: u8 = 0x00;
pub const BASE_UINT8: u8 = 0x02;
pub const BASE_STRING: u8 = 0x07;
pub const BASE_UINT16: u8 = 0x84;
pub const BASE_UINT32: u8 = 0x86;
pub const BASE_UINT32Z: u8 = 0x8C;

#[allow(dead_code)] // profile data table: kept complete for the base types we emit
pub const INVALID_ENUM: u8 = 0xFF;
pub const INVALID_UINT8: u8 = 0xFF;
pub const INVALID_UINT16: u16 = 0xFFFF;
#[allow(dead_code)]
pub const INVALID_UINT32: u32 = 0xFFFF_FFFF;
#[allow(dead_code)]
pub const INVALID_UINT32Z: u32 = 0x0000_0000;

// ---------------------------------------------------------------------------
// Global message numbers
// ---------------------------------------------------------------------------

pub const MSG_FILE_ID: u16 = 0;
pub const MSG_SESSION: u16 = 18;
pub const MSG_LAP: u16 = 19;
pub const MSG_RECORD: u16 = 20;
pub const MSG_EVENT: u16 = 21;
pub const MSG_DEVICE_INFO: u16 = 23;
pub const MSG_ACTIVITY: u16 = 34;

// ---------------------------------------------------------------------------
// Local message types (one per global, SPEC.md §7.1)
// ---------------------------------------------------------------------------

pub const LOCAL_FILE_ID: u8 = 0;
pub const LOCAL_DEVICE_INFO: u8 = 1;
pub const LOCAL_EVENT: u8 = 2;
pub const LOCAL_RECORD: u8 = 3;
pub const LOCAL_LAP: u8 = 4;
pub const LOCAL_SESSION: u8 = 5;
pub const LOCAL_ACTIVITY: u8 = 6;

// ---------------------------------------------------------------------------
// file_id (0)
// ---------------------------------------------------------------------------

pub const FILE_ID_TYPE: u8 = 0; // enum
pub const FILE_ID_MANUFACTURER: u8 = 1; // uint16
pub const FILE_ID_PRODUCT: u8 = 2; // uint16
pub const FILE_ID_SERIAL_NUMBER: u8 = 3; // uint32z
pub const FILE_ID_TIME_CREATED: u8 = 4; // uint32 (date_time)

/// file enum: 4 = activity.
pub const FILE_TYPE_ACTIVITY: u8 = 4;
/// manufacturer: 255 = development.
pub const MANUFACTURER_DEVELOPMENT: u16 = 255;
/// Our product id.
pub const PRODUCT_TRAINERPRO: u16 = 1;

// ---------------------------------------------------------------------------
// device_info (23)
// ---------------------------------------------------------------------------

pub const DEVICE_INFO_DEVICE_INDEX: u8 = 0; // uint8
pub const DEVICE_INFO_MANUFACTURER: u8 = 2; // uint16
pub const DEVICE_INFO_SOFTWARE_VERSION: u8 = 5; // uint16, scale 100
pub const DEVICE_INFO_PRODUCT_NAME: u8 = 27; // string

/// device_index: 0 = creator.
pub const DEVICE_INDEX_CREATOR: u8 = 0;

/// Fixed byte size (incl. NUL terminator) of the product_name string field —
/// one definition covers every device_info data record.
pub const PRODUCT_NAME_SIZE: u8 = 32;

// ---------------------------------------------------------------------------
// event (21)
// ---------------------------------------------------------------------------

pub const EVENT_TIMESTAMP: u8 = 253; // uint32 (date_time)
pub const EVENT_EVENT: u8 = 0; // enum
pub const EVENT_EVENT_TYPE: u8 = 1; // enum

/// event enum: 0 = timer.
pub const EVENT_TIMER: u8 = 0;
/// event enum: 26 = activity.
pub const EVENT_ACTIVITY: u8 = 26;
/// event_type enum: 0 = start.
pub const EVENT_TYPE_START: u8 = 0;
/// event_type enum: 1 = stop.
pub const EVENT_TYPE_STOP: u8 = 1;
/// event_type enum: 4 = stop_all.
pub const EVENT_TYPE_STOP_ALL: u8 = 4;

// ---------------------------------------------------------------------------
// record (20)
// ---------------------------------------------------------------------------

pub const RECORD_TIMESTAMP: u8 = 253; // uint32 (date_time)
pub const RECORD_HEART_RATE: u8 = 3; // uint8, bpm
pub const RECORD_CADENCE: u8 = 4; // uint8, rpm
pub const RECORD_DISTANCE: u8 = 5; // uint32, scale 100 (m)
pub const RECORD_SPEED: u8 = 6; // uint16, scale 1000 (m/s)
pub const RECORD_POWER: u8 = 7; // uint16, W

/// record.distance scale: stored value = metres × 100.
pub const DISTANCE_SCALE: f64 = 100.0;
/// record.speed scale: stored value = m/s × 1000.
pub const SPEED_SCALE: f64 = 1000.0;

// ---------------------------------------------------------------------------
// lap (19)
// ---------------------------------------------------------------------------

pub const LAP_MESSAGE_INDEX: u8 = 254; // uint16
pub const LAP_TIMESTAMP: u8 = 253; // uint32 = lap end time
pub const LAP_START_TIME: u8 = 2; // uint32
pub const LAP_TOTAL_ELAPSED_TIME: u8 = 7; // uint32, scale 1000 (s)
pub const LAP_TOTAL_TIMER_TIME: u8 = 8; // uint32, scale 1000 (s)
pub const LAP_TOTAL_CALORIES: u8 = 11; // uint16, kcal
pub const LAP_AVG_HEART_RATE: u8 = 15; // uint8
pub const LAP_MAX_HEART_RATE: u8 = 16; // uint8
pub const LAP_AVG_CADENCE: u8 = 17; // uint8
pub const LAP_AVG_POWER: u8 = 19; // uint16
pub const LAP_MAX_POWER: u8 = 20; // uint16

// ---------------------------------------------------------------------------
// session (18)
// ---------------------------------------------------------------------------

pub const SESSION_TIMESTAMP: u8 = 253; // uint32 = session end time
pub const SESSION_START_TIME: u8 = 2; // uint32
pub const SESSION_SPORT: u8 = 5; // enum
pub const SESSION_SUB_SPORT: u8 = 6; // enum
pub const SESSION_TOTAL_ELAPSED_TIME: u8 = 7; // uint32, scale 1000 (s)
pub const SESSION_TOTAL_TIMER_TIME: u8 = 8; // uint32, scale 1000 (s)
pub const SESSION_TOTAL_CALORIES: u8 = 11; // uint16, kcal
pub const SESSION_AVG_HEART_RATE: u8 = 16; // uint8
pub const SESSION_MAX_HEART_RATE: u8 = 17; // uint8
pub const SESSION_AVG_CADENCE: u8 = 18; // uint8
pub const SESSION_AVG_POWER: u8 = 20; // uint16
pub const SESSION_MAX_POWER: u8 = 21; // uint16
pub const SESSION_FIRST_LAP_INDEX: u8 = 25; // uint16
pub const SESSION_NUM_LAPS: u8 = 26; // uint16
pub const SESSION_NORMALIZED_POWER: u8 = 34; // uint16, W
pub const SESSION_TRAINING_STRESS_SCORE: u8 = 35; // uint16, scale 10
pub const SESSION_INTENSITY_FACTOR: u8 = 36; // uint16, scale 1000
pub const SESSION_THRESHOLD_POWER: u8 = 101; // uint16, W

/// sport enum: 2 = cycling.
pub const SPORT_CYCLING: u8 = 2;
/// sub_sport enum: 6 = indoor_cycling.
pub const SUB_SPORT_INDOOR_CYCLING: u8 = 6;

/// session.training_stress_score scale: stored value = TSS × 10.
pub const TSS_SCALE: f64 = 10.0;
/// session.intensity_factor scale: stored value = IF × 1000.
pub const IF_SCALE: f64 = 1000.0;

// ---------------------------------------------------------------------------
// activity (34)
// ---------------------------------------------------------------------------

pub const ACTIVITY_TIMESTAMP: u8 = 253; // uint32
pub const ACTIVITY_TOTAL_TIMER_TIME: u8 = 0; // uint32, scale 1000 (s)
pub const ACTIVITY_NUM_SESSIONS: u8 = 1; // uint16
pub const ACTIVITY_TYPE: u8 = 2; // enum
pub const ACTIVITY_EVENT: u8 = 3; // enum
pub const ACTIVITY_EVENT_TYPE: u8 = 4; // enum
pub const ACTIVITY_LOCAL_TIMESTAMP: u8 = 5; // uint32 (local_date_time)

/// activity enum: 0 = manual.
pub const ACTIVITY_TYPE_MANUAL: u8 = 0;
