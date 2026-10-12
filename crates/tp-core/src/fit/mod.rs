//! Pure FIT activity encoding and decoding.
//!
//! The writer serializes journal replay and precomputed metrics. The reader
//! validates bytes and exposes the standard activity subset without provider,
//! storage, timezone or sample-support policy dependencies.

mod crc;
mod decode;
mod encode;
mod profile;

pub use crc::checksum;
pub use decode::{
    decode_activity, DecodedActivity, DecodedLap, DecodedRecord, FitDecodeError, FitSport,
    FitSummary, TimerEvent, TimerState, FIT_DECODER_VERSION,
};
pub use encode::encode_activity;

use crate::journal::{ActivitySegment, JournalHeader, Sample, SessionEvent};
use crate::metrics::SessionTotals;

/// Borrowed view of a completed activity, ready to serialize.
pub struct FitActivity<'a> {
    pub header: &'a JournalHeader,
    pub samples: &'a [Sample],
    pub events: &'a [SessionEvent],
    /// Recorded activity segments, serialized as FIT lap messages.
    pub laps: &'a [ActivitySegment],
    pub totals: &'a SessionTotals,
    /// Precomputed motion from journal replay, absent when disabled.
    pub motion: Option<&'a crate::motion::MotionTrace>,
}

#[derive(Debug, thiserror::Error)]
pub enum FitError {
    #[error("fit encode: {0}")]
    Encode(String),
}
