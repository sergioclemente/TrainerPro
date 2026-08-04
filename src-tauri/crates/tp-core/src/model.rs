//! Shared workout model. SPEC.md §2. This file is the contract every other
//! module codes against — changes here require touching the spec first.

use serde::{Deserialize, Serialize};

use crate::consts::MAX_TARGET_WATTS;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PowerTarget {
    /// Fraction of FTP, e.g. 0.75 = 75 % FTP. Valid 0.05..=3.0.
    PercentFtp(f64),
    /// Absolute watts (ERG files).
    Watts(u16),
}

impl PowerTarget {
    /// Resolve to whole watts. `intensity` (live bias, 0.50..=1.50) applies
    /// to `PercentFtp` targets only; absolute-watt targets ignore it.
    pub fn resolve(&self, ftp: u16, intensity: f64) -> u16 {
        let w = match self {
            PowerTarget::PercentFtp(frac) => frac * intensity * f64::from(ftp),
            PowerTarget::Watts(w) => f64::from(*w),
        };
        w.round().clamp(0.0, f64::from(MAX_TARGET_WATTS)) as u16
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Segment {
    Steady {
        duration_s: u32,
        power: PowerTarget,
        cadence_rpm: Option<u16>,
    },
    Ramp {
        duration_s: u32,
        start: PowerTarget,
        end: PowerTarget,
        cadence_rpm: Option<u16>,
    },
    /// No ERG target; trainer switches to simulation mode, grade 0 %.
    FreeRide { duration_s: u32 },
}

impl Segment {
    pub fn duration_s(&self) -> u32 {
        match self {
            Segment::Steady { duration_s, .. }
            | Segment::Ramp { duration_s, .. }
            | Segment::FreeRide { duration_s } => *duration_s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextEvent {
    /// Offset from workout start, in active (unpaused) seconds.
    pub offset_s: u32,
    pub message: String,
    pub duration_s: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceFormat {
    Zwo,
    Erg,
    Mrc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workout {
    pub name: String,
    pub description: String,
    pub source_format: SourceFormat,
    /// Flat segment list; interval repeats are pre-expanded by the parser.
    pub segments: Vec<Segment>,
    /// Sorted by `offset_s`; offsets are workout-absolute.
    pub text_events: Vec<TextEvent>,
}

impl Workout {
    pub fn duration_s(&self) -> u32 {
        self.segments.iter().map(Segment::duration_s).sum()
    }

    /// Segment containing active-time offset `t_s`, plus the offset into that
    /// segment. `None` when `t_s` is at/past the end of the workout.
    pub fn segment_at(&self, t_s: u32) -> Option<(usize, u32)> {
        let mut start = 0u32;
        for (i, seg) in self.segments.iter().enumerate() {
            let end = start + seg.duration_s();
            if t_s < end {
                return Some((i, t_s - start));
            }
            start = end;
        }
        None
    }

    /// ERG target in whole watts at offset `t_s`. `None` inside FreeRide or
    /// past the end. Ramps resolve both endpoints then interpolate linearly
    /// in watts by elapsed fraction.
    pub fn target_at(&self, t_s: u32, ftp: u16, intensity: f64) -> Option<u16> {
        let (idx, into) = self.segment_at(t_s)?;
        match &self.segments[idx] {
            Segment::Steady { power, .. } => Some(power.resolve(ftp, intensity)),
            Segment::Ramp {
                duration_s,
                start,
                end,
                ..
            } => {
                let a = f64::from(start.resolve(ftp, intensity));
                let b = f64::from(end.resolve(ftp, intensity));
                let frac = f64::from(into) / f64::from(*duration_s);
                let w = a + (b - a) * frac;
                Some(w.round().clamp(0.0, f64::from(MAX_TARGET_WATTS)) as u16)
            }
            Segment::FreeRide { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wk(segments: Vec<Segment>) -> Workout {
        Workout {
            name: "t".into(),
            description: String::new(),
            source_format: SourceFormat::Zwo,
            segments,
            text_events: vec![],
        }
    }

    #[test]
    fn resolve_applies_intensity_to_percent_only() {
        assert_eq!(PowerTarget::PercentFtp(0.8).resolve(250, 1.0), 200);
        assert_eq!(PowerTarget::PercentFtp(0.8).resolve(250, 1.1), 220);
        assert_eq!(PowerTarget::Watts(200).resolve(250, 1.1), 200);
    }

    #[test]
    fn target_at_ramp_interpolates() {
        let w = wk(vec![Segment::Ramp {
            duration_s: 100,
            start: PowerTarget::Watts(100),
            end: PowerTarget::Watts(200),
            cadence_rpm: None,
        }]);
        assert_eq!(w.target_at(0, 250, 1.0), Some(100));
        assert_eq!(w.target_at(50, 250, 1.0), Some(150));
        assert_eq!(w.target_at(99, 250, 1.0), Some(199));
        assert_eq!(w.target_at(100, 250, 1.0), None); // past end
    }

    #[test]
    fn segment_at_boundaries() {
        let w = wk(vec![
            Segment::Steady {
                duration_s: 60,
                power: PowerTarget::Watts(100),
                cadence_rpm: None,
            },
            Segment::FreeRide { duration_s: 30 },
        ]);
        assert_eq!(w.segment_at(59), Some((0, 59)));
        assert_eq!(w.segment_at(60), Some((1, 0)));
        assert_eq!(w.target_at(60, 250, 1.0), None); // FreeRide
        assert_eq!(w.segment_at(90), None);
        assert_eq!(w.duration_s(), 90);
    }
}
