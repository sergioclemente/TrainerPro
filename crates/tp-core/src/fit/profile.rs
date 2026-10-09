//! FIT profile subset: global message numbers, field numbers, base types,
//! scales, and enum values for the eleven message types we write. Values match
//! the FIT SDK profile for these messages; FitCSVTool validation is the
//! authoritative compatibility check. Keep this file data-only (consts and
//! small enums); encoding logic lives in `encode.rs`.

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

pub const INVALID_ENUM: u8 = 0xFF;
pub const INVALID_UINT8: u8 = 0xFF;
pub const INVALID_UINT16: u16 = 0xFFFF;
pub const INVALID_UINT32: u32 = 0xFFFF_FFFF;
#[allow(dead_code)] // profile data table: kept complete for the base types we emit
pub const INVALID_UINT32Z: u32 = 0x0000_0000;

// ---------------------------------------------------------------------------
// Global message numbers
// ---------------------------------------------------------------------------

pub const MSG_FILE_ID: u16 = 0;
pub const MSG_USER_PROFILE: u16 = 3;
pub const MSG_ZONES_TARGET: u16 = 7;
pub const MSG_SESSION: u16 = 18;
pub const MSG_LAP: u16 = 19;
pub const MSG_RECORD: u16 = 20;
pub const MSG_EVENT: u16 = 21;
pub const MSG_DEVICE_INFO: u16 = 23;
pub const MSG_WORKOUT: u16 = 26;
pub const MSG_WORKOUT_STEP: u16 = 27;
pub const MSG_ACTIVITY: u16 = 34;

// ---------------------------------------------------------------------------
// Local message types (one per global message type).
// ---------------------------------------------------------------------------

pub const LOCAL_FILE_ID: u8 = 0;
pub const LOCAL_DEVICE_INFO: u8 = 1;
pub const LOCAL_EVENT: u8 = 2;
pub const LOCAL_RECORD: u8 = 3;
pub const LOCAL_LAP: u8 = 4;
pub const LOCAL_SESSION: u8 = 5;
pub const LOCAL_ACTIVITY: u8 = 6;
pub const LOCAL_USER_PROFILE: u8 = 7;
pub const LOCAL_ZONES_TARGET: u8 = 8;
pub const LOCAL_WORKOUT: u8 = 9;
pub const LOCAL_WORKOUT_STEP: u8 = 10;

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
// user_profile (3)
// ---------------------------------------------------------------------------

pub const USER_PROFILE_WEIGHT: u8 = 4; // uint16, scale 10 (kg)

/// user_profile.weight scale: stored value = kg × 10.
pub const WEIGHT_SCALE: f64 = 10.0;

// ---------------------------------------------------------------------------
// zones_target (7)
// ---------------------------------------------------------------------------

pub const ZONES_TARGET_FUNCTIONAL_THRESHOLD_POWER: u8 = 3; // uint16, W
pub const ZONES_TARGET_PWR_CALC_TYPE: u8 = 7; // enum

/// pwr_calc_type enum: 1 = percent_ftp.
pub const PWR_CALC_TYPE_PERCENT_FTP: u8 = 1;

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
// workout (26)
// ---------------------------------------------------------------------------

pub const WORKOUT_SPORT: u8 = 4; // enum
pub const WORKOUT_NUM_VALID_STEPS: u8 = 6; // uint16
pub const WORKOUT_WKT_NAME: u8 = 8; // string
pub const WORKOUT_SUB_SPORT: u8 = 11; // enum

/// Fixed byte size (incl. NUL terminator) of the workout name string field.
pub const WKT_NAME_SIZE: u8 = 64;

// ---------------------------------------------------------------------------
// workout_step (27)
// ---------------------------------------------------------------------------

pub const WORKOUT_STEP_MESSAGE_INDEX: u8 = 254; // uint16
pub const WORKOUT_STEP_NAME: u8 = 0; // string
pub const WORKOUT_STEP_DURATION_TYPE: u8 = 1; // enum
pub const WORKOUT_STEP_DURATION_VALUE: u8 = 2; // uint32, scale 1000 (s) for time
pub const WORKOUT_STEP_TARGET_TYPE: u8 = 3; // enum
pub const WORKOUT_STEP_TARGET_VALUE: u8 = 4; // uint32
pub const WORKOUT_STEP_CUSTOM_TARGET_LOW: u8 = 5; // uint32
pub const WORKOUT_STEP_CUSTOM_TARGET_HIGH: u8 = 6; // uint32
pub const WORKOUT_STEP_INTENSITY: u8 = 7; // enum
pub const WORKOUT_STEP_SECONDARY_TARGET_TYPE: u8 = 19; // enum
pub const WORKOUT_STEP_SECONDARY_TARGET_VALUE: u8 = 20; // uint32
pub const WORKOUT_STEP_SECONDARY_CUSTOM_TARGET_LOW: u8 = 21; // uint32
pub const WORKOUT_STEP_SECONDARY_CUSTOM_TARGET_HIGH: u8 = 22; // uint32

/// Fixed byte size (incl. NUL terminator) of the step name string field; a
/// ramp between watt targets over an hour ("Cool-down · 1:01:00 @ 2000 W→
/// 2000 W") must fit without truncation.
pub const WKT_STEP_NAME_SIZE: u8 = 48;

/// wkt_step_duration enum: 0 = time (duration_value in ms).
pub const WKT_STEP_DURATION_TIME: u8 = 0;
/// wkt_step_target enum: 2 = open.
pub const WKT_STEP_TARGET_OPEN: u8 = 2;
/// wkt_step_target enum: 3 = cadence.
pub const WKT_STEP_TARGET_CADENCE: u8 = 3;
/// wkt_step_target enum: 4 = power.
pub const WKT_STEP_TARGET_POWER: u8 = 4;
/// target_value marking a custom low/high range instead of a zone.
pub const WKT_STEP_TARGET_CUSTOM: u32 = 0;
/// workout_power: values at or below this are % FTP; above, watts offset by it.
pub const WORKOUT_POWER_WATTS_OFFSET: u32 = 1000;

/// intensity enum: 0 = active.
pub const INTENSITY_ACTIVE: u8 = 0;
/// intensity enum: 1 = rest.
pub const INTENSITY_REST: u8 = 1;
/// intensity enum: 2 = warmup.
pub const INTENSITY_WARMUP: u8 = 2;
/// intensity enum: 3 = cooldown.
pub const INTENSITY_COOLDOWN: u8 = 3;
/// intensity enum: 6 = other.
pub const INTENSITY_OTHER: u8 = 6;

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
pub const LAP_TOTAL_DISTANCE: u8 = 9; // uint32, scale 100 (m)
pub const LAP_AVG_SPEED: u8 = 13; // uint16, scale 1000 (m/s)
pub const LAP_MAX_SPEED: u8 = 14; // uint16, scale 1000 (m/s)
pub const LAP_TOTAL_CALORIES: u8 = 11; // uint16, kcal
pub const LAP_AVG_HEART_RATE: u8 = 15; // uint8
pub const LAP_MAX_HEART_RATE: u8 = 16; // uint8
pub const LAP_AVG_CADENCE: u8 = 17; // uint8
pub const LAP_MAX_CADENCE: u8 = 18; // uint8
pub const LAP_AVG_POWER: u8 = 19; // uint16
pub const LAP_MAX_POWER: u8 = 20; // uint16
pub const LAP_INTENSITY: u8 = 23; // enum
pub const LAP_LAP_TRIGGER: u8 = 24; // enum
pub const LAP_SPORT: u8 = 25; // enum
pub const LAP_NORMALIZED_POWER: u8 = 33; // uint16, W
pub const LAP_SUB_SPORT: u8 = 39; // enum
pub const LAP_TOTAL_WORK: u8 = 41; // uint32, J
pub const LAP_WKT_STEP_INDEX: u8 = 71; // uint16

/// lap_trigger enum: 7 = session_end.
pub const LAP_TRIGGER_SESSION_END: u8 = 7;
/// lap_trigger enum: 8 = fitness_equipment (the workout moved on).
pub const LAP_TRIGGER_FITNESS_EQUIPMENT: u8 = 8;

// ---------------------------------------------------------------------------
// session (18)
// ---------------------------------------------------------------------------

pub const SESSION_TIMESTAMP: u8 = 253; // uint32 = session end time
pub const SESSION_START_TIME: u8 = 2; // uint32
pub const SESSION_SPORT: u8 = 5; // enum
pub const SESSION_SUB_SPORT: u8 = 6; // enum
pub const SESSION_TOTAL_ELAPSED_TIME: u8 = 7; // uint32, scale 1000 (s)
pub const SESSION_TOTAL_TIMER_TIME: u8 = 8; // uint32, scale 1000 (s)
pub const SESSION_TOTAL_DISTANCE: u8 = 9; // uint32, scale 100 (m)
pub const SESSION_AVG_SPEED: u8 = 14; // uint16, scale 1000 (m/s)
pub const SESSION_MAX_SPEED: u8 = 15; // uint16, scale 1000 (m/s)
pub const SESSION_TOTAL_CALORIES: u8 = 11; // uint16, kcal
pub const SESSION_AVG_HEART_RATE: u8 = 16; // uint8
pub const SESSION_MAX_HEART_RATE: u8 = 17; // uint8
pub const SESSION_AVG_CADENCE: u8 = 18; // uint8
pub const SESSION_MAX_CADENCE: u8 = 19; // uint8
pub const SESSION_AVG_POWER: u8 = 20; // uint16
pub const SESSION_MAX_POWER: u8 = 21; // uint16
pub const SESSION_FIRST_LAP_INDEX: u8 = 25; // uint16
pub const SESSION_NUM_LAPS: u8 = 26; // uint16
pub const SESSION_NORMALIZED_POWER: u8 = 34; // uint16, W
pub const SESSION_TRAINING_STRESS_SCORE: u8 = 35; // uint16, scale 10
pub const SESSION_INTENSITY_FACTOR: u8 = 36; // uint16, scale 1000
pub const SESSION_TOTAL_WORK: u8 = 48; // uint32, J
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
