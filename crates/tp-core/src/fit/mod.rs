//! FIT Activity file encoder.
//!
//! Input is everything already computed by journal replay + metrics; this
//! module only serializes bytes. Output must decode with Garmin FitCSVTool
//! with zero errors (CI gate, M4).

mod crc;
mod encode;
mod profile;

pub use crc::checksum;
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
