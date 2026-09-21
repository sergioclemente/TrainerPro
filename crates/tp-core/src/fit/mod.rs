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

use crate::journal::{JournalHeader, Lap, Sample, SessionEvent};
use crate::metrics::SessionTotals;

/// Borrowed view of a completed activity, ready to serialize.
pub struct FitActivity<'a> {
    pub header: &'a JournalHeader,
    pub samples: &'a [Sample],
    pub events: &'a [SessionEvent],
    pub laps: &'a [Lap],
    pub totals: &'a SessionTotals,
    /// When true, emit speed+distance records from the virtual flat-road
    /// model; v1 default is false (no speed/distance fields at all).
    pub record_distance: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum FitError {
    #[error("fit encode: {0}")]
    Encode(String),
}
