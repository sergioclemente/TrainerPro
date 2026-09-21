//! tp-core: pure domain logic for TrainerPro.
//!
//! Invariant: this crate has no I/O (journal.rs takes `io::Write`/`io::Read`
//! handles supplied by the caller), no async, and no BLE/tauri dependencies.
//! Everything here is unit-testable without hardware.

pub mod build;
pub mod consts;
pub mod engine;
pub mod fit;
pub mod journal;
pub mod metrics;
pub mod model;
pub mod parse;
pub mod workout_definition;
