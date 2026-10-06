//! Pure flat-road motion and journal replay for estimated indoor distance.
use crate::consts::SAMPLE_HZ;
use crate::journal::{ActivitySegment, SessionEventKind, SessionRecording};

const BIKE_MASS_KG: f64 = 9.0;
const AIR_DENSITY_KG_M3: f64 = 1.225;
const DRAG_AREA_M2: f64 = 0.32;
const ROLLING_RESISTANCE: f64 = 0.005;
const GRAVITY_M_S2: f64 = 9.81;
const MILLIS_PER_SECOND: u64 = 1000;
const SAMPLE_PERIOD_MS: u64 = MILLIS_PER_SECOND / SAMPLE_HZ as u64;
const MOTION_GAP_MS: u64 = 2 * SAMPLE_PERIOD_MS;
const MAX_STEP_MS: u64 = 100;
const SOLVER_ITERATIONS: usize = 64;

#[derive(Debug, Clone, Copy, Default)]
pub struct MotionPoint {
    pub speed_m_s: f64,
    pub distance_m: f64,
}

/// Energy is authoritative; speed is derived, avoiding divergent motion state.
pub struct FlatRoadMotion {
    mass_kg: f64,
    energy_j: f64,
    distance_m: f64,
}

impl FlatRoadMotion {
    pub fn new(rider_kg: f64) -> Self {
        Self {
            mass_kg: if rider_kg.is_finite() {
                rider_kg.max(0.0)
            } else {
                0.0
            } + BIKE_MASS_KG,
            energy_j: 0.0,
            distance_m: 0.0,
        }
    }

    pub fn point(&self) -> MotionPoint {
        MotionPoint {
            speed_m_s: (2.0 * self.energy_j / self.mass_kg).sqrt(),
            distance_m: self.distance_m,
        }
    }

    /// Discontinuities discard momentum, never accumulated distance.
    pub fn stop(&mut self) {
        self.energy_j = 0.0;
    }

    pub fn advance(&mut self, power_w: u16, mut duration_ms: u64) -> MotionPoint {
        while duration_ms > 0 {
            let step_ms = duration_ms.min(MAX_STEP_MS);
            duration_ms -= step_ms;
            let dt_s = step_ms as f64 / MILLIS_PER_SECOND as f64;
            let initial_speed = self.point().speed_m_s;
            let available_j = self.energy_j + f64::from(power_w) * dt_s;
            // E + dt * resistance(sqrt(2E/m)) = available energy is monotone
            // on a flat road, with its nonnegative root in [0, available].
            let (mut low, mut high) = (0.0, available_j);
            for _ in 0..SOLVER_ITERATIONS {
                let energy = (low + high) / 2.0;
                let speed = (2.0 * energy / self.mass_kg).sqrt();
                let resistance_w = 0.5 * AIR_DENSITY_KG_M3 * DRAG_AREA_M2 * speed.powi(3)
                    + ROLLING_RESISTANCE * self.mass_kg * GRAVITY_M_S2 * speed;
                if energy + dt_s * resistance_w > available_j {
                    high = energy;
                } else {
                    low = energy;
                }
            }
            self.energy_j = (low + high) / 2.0;
            self.distance_m += (initial_speed + self.point().speed_m_s) * 0.5 * dt_s;
        }
        self.point()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MotionSummary {
    pub distance_m: f64,
    pub max_speed_m_s: f64,
}

impl MotionSummary {
    pub fn average_speed_m_s(&self, timer_ms: u64) -> Option<f64> {
        (timer_ms > 0).then(|| self.distance_m * MILLIS_PER_SECOND as f64 / timer_ms as f64)
    }
}

/// Records and activity segments retain the input order. All summaries use the same motion
/// integration; no independent segment replays or momentum resets at segment boundaries.
#[derive(Debug)]
pub struct MotionTrace {
    pub records: Vec<MotionPoint>,
    pub activity_segments: Vec<MotionSummary>,
    pub session: MotionSummary,
}

/// Each measurement covers up to one nominal period immediately before its
/// timestamp, bounded by the preceding sample and the latest start/resume.
/// Missing time is never filled. Equal timestamps contribute zero duration.
/// Resume precedes a tied sample; Pause follows it, matching FIT timer events.
/// Pauses freeze motion; only unsampled active time counts toward a gap reset.
pub fn replay_motion(
    data: &SessionRecording,
    activity_segments: &[ActivitySegment],
) -> MotionTrace {
    let mut motion = FlatRoadMotion::new(data.header.weight_kg);
    let mut trace = MotionTrace {
        records: Vec::with_capacity(data.samples.len()),
        activity_segments: vec![MotionSummary::default(); activity_segments.len()],
        session: MotionSummary::default(),
    };
    let mut events: Vec<_> = data.events.iter().collect();
    events.sort_by_key(|e| {
        (
            e.t_ms,
            match e.kind {
                SessionEventKind::Start | SessionEventKind::Resume => 0,
                _ => 2,
            },
        )
    });
    let mut events = events.into_iter().peekable();
    let mut active = true;
    let mut active_start_ms = 0;
    let mut previous_ms: Option<u64> = None;
    let mut timeline_ms = 0;
    let mut unsampled_active_ms = 0u64;
    for sample in &data.samples {
        while let Some(event) = events.peek() {
            if event.t_ms > sample.t_ms
                || (event.t_ms == sample.t_ms
                    && !matches!(
                        event.kind,
                        SessionEventKind::Start | SessionEventKind::Resume
                    ))
            {
                break;
            }
            let event = events.next().unwrap();
            if active {
                unsampled_active_ms += event.t_ms.saturating_sub(timeline_ms);
            }
            timeline_ms = timeline_ms.max(event.t_ms);
            match event.kind {
                SessionEventKind::Start => {
                    active = true;
                    active_start_ms = event.t_ms;
                    unsampled_active_ms = 0;
                    motion.stop();
                }
                SessionEventKind::Resume => {
                    active = true;
                    active_start_ms = event.t_ms;
                }
                SessionEventKind::Pause | SessionEventKind::End => {
                    active = false;
                }
                _ => {}
            }
        }
        if active {
            unsampled_active_ms += sample.t_ms.saturating_sub(timeline_ms);
            if unsampled_active_ms >= MOTION_GAP_MS {
                motion.stop();
            }
            unsampled_active_ms = 0;
        }
        timeline_ms = timeline_ms.max(sample.t_ms);
        let window_start = sample
            .t_ms
            .saturating_sub(SAMPLE_PERIOD_MS)
            .max(previous_ms.unwrap_or(active_start_ms))
            .max(active_start_ms);
        if let Some(power) = sample.power_w.filter(|_| active) {
            // Integrate the overlap with each segment, so partial periods crossing
            // boundaries retain their distance and use the correct timer time.
            for (segment, summary) in activity_segments.iter().zip(&mut trace.activity_segments) {
                let start = window_start.max(segment.start_ms);
                let end = sample.t_ms.min(segment.end_ms);
                if end > start {
                    let before = motion.point();
                    let after = motion.advance(power, end - start);
                    summary.distance_m += after.distance_m - before.distance_m;
                    summary.max_speed_m_s = summary
                        .max_speed_m_s
                        .max(before.speed_m_s)
                        .max(after.speed_m_s);
                }
            }
        } else if active {
            motion.stop();
        }
        trace.records.push(motion.point());
        previous_ms = Some(previous_ms.unwrap_or(0).max(sample.t_ms));
    }
    trace.session.distance_m = motion.point().distance_m;
    trace.session.max_speed_m_s = trace
        .activity_segments
        .iter()
        .map(|segment| segment.max_speed_m_s)
        .fold(0.0, f64::max);
    trace
}
