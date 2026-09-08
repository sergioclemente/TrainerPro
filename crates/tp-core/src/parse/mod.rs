//! Workout file parsers. SPEC.md §3.

pub mod ergmrc;
pub mod zwo;

pub use ergmrc::parse_ergmrc;
pub use zwo::parse_zwo;

use crate::model::Workout;

/// Non-fatal issue found while parsing (unknown element, clamped value…).
/// Surfaced once in the import UI; never aborts a parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub workout: Workout,
    pub warnings: Vec<ParseWarning>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Malformed input; message names the offending line/element.
    #[error("parse failed: {0}")]
    Invalid(String),
    /// Structurally fine but produced no ridable segments.
    #[error("workout contains no segments")]
    Empty,
}
