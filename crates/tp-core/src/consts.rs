//! Single source for tunable constants. SPEC.md §14.

/// Engine tick interval driven by the player runtime.
pub const ENGINE_TICK_MS: u64 = 250;
/// Recorder sample rate (samples per second).
pub const SAMPLE_HZ: u32 = 1;
/// Display power smoothing window.
pub const SMOOTH_WINDOW_S: u32 = 3;
/// Re-send current ERG target even when unchanged.
pub const ERG_KEEPALIVE_S: u64 = 10;
/// FTMS control point response timeout and retry count.
pub const CP_TIMEOUT_MS: u64 = 2_000;
pub const CP_RETRIES: u32 = 1;
/// Absolute target power clamp (trainer-reported max may lower this).
pub const MAX_TARGET_WATTS: u16 = 2_000;
/// Live intensity bias bounds and step.
pub const INTENSITY_MIN: f64 = 0.50;
pub const INTENSITY_MAX: f64 = 1.50;
pub const INTENSITY_STEP: f64 = 0.01;
/// Reconnect schedule: initial delays, then `RECONNECT_STEADY_S` forever.
pub const RECONNECT_SCHEDULE_S: [u64; 5] = [0, 1, 2, 5, 10];
pub const RECONNECT_STEADY_S: u64 = 15;
/// Default on-screen duration for workout text events.
pub const TEXT_EVENT_DEFAULT_S: u32 = 10;
/// Normalized Power rolling window.
pub const NP_WINDOW_S: u32 = 30;
/// Seconds between Unix epoch and FIT epoch (1989-12-31T00:00:00Z).
pub const FIT_EPOCH_OFFSET_S: u64 = 631_065_600;
/// Power fraction sanity bounds for parsed workout targets (of FTP).
pub const POWER_FRACTION_MIN: f64 = 0.05;
pub const POWER_FRACTION_MAX: f64 = 3.0;
/// Highest cadence accepted by the semantic workout model.
pub const WORKOUT_CADENCE_RPM_MAX: u16 = 300;
/// Most repetitions accepted for one semantic workout repeat.
pub const WORKOUT_REPEAT_COUNT_MAX: u32 = 100;
/// Deepest supported nesting of semantic workout repeats.
pub const WORKOUT_REPEAT_DEPTH_MAX: usize = 8;
/// Largest flat workout the semantic compiler will allocate.
pub const WORKOUT_EXECUTABLE_SEGMENTS_MAX: u64 = 10_000;
