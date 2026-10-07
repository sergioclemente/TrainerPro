//! Reusable provider protocols, independent of the TrainerPro application and domain.
//!
//! Each provider exposes its own client, payloads, and errors. Callers supply a
//! Tokio runtime and own credential storage, account coordination, persistence,
//! and any conversion to application-specific workout models.

pub mod garmin;
pub mod intervals_icu;
