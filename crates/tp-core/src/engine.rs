//! Player engine: pure state machine. SPEC.md §5.1.
//!
//! Deterministic: same Input sequence ⇒ same Effect stream. No clocks — the
//! runtime owns time and feeds `Tick`.
//!
//! Rules (see spec for full text):
//! - Tick advances only in Riding.
//! - Segment rollover: LapBoundary + first target of new segment (or
//!   EnterFreeRide). Workout end: WorkoutComplete + TrainerReset, phase →
//!   Finished.
//! - Ramp targets recompute each Tick; SetTarget emitted only when the
//!   rounded watt value changed (dedup lives HERE, not in BLE).
//! - Start: Ready→Riding, emits TrainerStart + initial target/freeride.
//! - Pause: TrainerStop. Resume: TrainerStart + re-emit current target.
//! - SkipSegment: jump to next boundary (LapBoundary); skip on last segment
//!   behaves like End.
//! - SetIntensity clamps to INTENSITY_MIN..=MAX, re-emits target if changed.
//! - Text events fire when active time crosses offset_s (paused excluded);
//!   each fires exactly once.

use crate::consts::{INTENSITY_MAX, INTENSITY_MIN, MAX_TARGET_WATTS};
use crate::model::{Segment, TextEvent, Workout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Ready,
    Riding,
    Paused,
    Finished,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Start,
    Pause,
    Resume,
    SkipSegment,
    SetIntensity(f64),
    Tick { dt_ms: u64 },
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    SetTarget(u16),
    EnterFreeRide,
    TrainerStart,
    TrainerStop,
    TrainerReset,
    /// Emitted when leaving a segment (rollover, skip). `seg_idx` = the
    /// segment just finished.
    LapBoundary { seg_idx: usize },
    ShowText(TextEvent),
    WorkoutComplete,
}

#[derive(Debug)]
pub struct Engine {
    // Implementation-private fields; public surface is new()/handle()/getters.
    workout: Workout,
    ftp: u16,
    intensity: f64,
    phase: Phase,
    active_ms: u64,
    /// Milliseconds actually ridden. Unlike `active_ms` this only ever
    /// advances by a tick, so skipping forward does not credit the rider with
    /// time they never spent pedalling.
    ridden_ms: u64,
    /// Index of the segment currently being ridden. Only meaningful while
    /// `phase` is Riding/Paused (and before Finished).
    seg_idx: usize,
    /// Active milliseconds elapsed inside the current segment.
    seg_elapsed_ms: u64,
    /// Last ERG target sent to the trainer (dedup source). `None` while in
    /// FreeRide or before the first target was emitted.
    last_target: Option<u16>,
    /// True while riding inside a FreeRide segment (EnterFreeRide sent).
    in_free_ride: bool,
    /// Index into `workout.text_events` of the next event not yet fired.
    next_text: usize,
}

impl Engine {
    pub fn new(workout: Workout, ftp: u16, intensity: f64) -> Self {
        Engine {
            workout,
            ftp,
            intensity: intensity.clamp(INTENSITY_MIN, INTENSITY_MAX),
            phase: Phase::Ready,
            active_ms: 0,
            ridden_ms: 0,
            seg_idx: 0,
            seg_elapsed_ms: 0,
            last_target: None,
            in_free_ride: false,
            next_text: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn active_ms(&self) -> u64 {
        self.active_ms
    }
    /// Time actually spent riding: excludes pauses (like `active_ms`) and also
    /// excludes spans jumped over by a skip.
    pub fn ridden_ms(&self) -> u64 {
        self.ridden_ms
    }
    pub fn intensity(&self) -> f64 {
        self.intensity
    }
    pub fn workout(&self) -> &Workout {
        &self.workout
    }
    /// Index of the current segment (None once finished).
    pub fn segment_index(&self) -> Option<usize> {
        self.workout.segment_at((self.active_ms / 1000) as u32).map(|(i, _)| i)
    }

    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        let mut fx = Vec::new();
        match input {
            Input::Start => {
                if self.phase != Phase::Ready {
                    return fx;
                }
                self.phase = Phase::Riding;
                fx.push(Effect::TrainerStart);
                if self.workout.segments.is_empty() {
                    self.finish(&mut fx);
                    return fx;
                }
                self.retarget(&mut fx, true);
                self.fire_texts(&mut fx);
            }
            Input::Pause => {
                if self.phase != Phase::Riding {
                    return fx;
                }
                self.phase = Phase::Paused;
                fx.push(Effect::TrainerStop);
            }
            Input::Resume => {
                if self.phase != Phase::Paused {
                    return fx;
                }
                self.phase = Phase::Riding;
                fx.push(Effect::TrainerStart);
                self.retarget(&mut fx, true);
            }
            Input::SkipSegment => {
                if self.phase != Phase::Riding {
                    return fx;
                }
                fx.push(Effect::LapBoundary { seg_idx: self.seg_idx });
                // Jump active time to the boundary of the current segment.
                let boundary = self.active_ms - self.seg_elapsed_ms + self.seg_dur_ms(self.seg_idx);
                self.active_ms = boundary;
                self.seg_elapsed_ms = 0;
                self.seg_idx += 1;
                if self.seg_idx >= self.workout.segments.len() {
                    // Skip on the last segment behaves like End.
                    self.finish(&mut fx);
                    return fx;
                }
                // Text events inside the skipped span are dropped, not fired;
                // an event landing exactly on the boundary still fires below.
                while self.next_text < self.workout.text_events.len()
                    && text_offset_ms(&self.workout.text_events[self.next_text]) < self.active_ms
                {
                    self.next_text += 1;
                }
                self.retarget(&mut fx, true);
                self.fire_texts(&mut fx);
            }
            Input::SetIntensity(v) => {
                if self.phase == Phase::Finished {
                    return fx;
                }
                self.intensity = v.clamp(INTENSITY_MIN, INTENSITY_MAX);
                if self.phase == Phase::Riding {
                    // Dedup: emits only if the resolved watt value changed.
                    self.retarget(&mut fx, false);
                }
            }
            Input::Tick { dt_ms } => {
                if self.phase != Phase::Riding {
                    return fx;
                }
                self.active_ms += dt_ms;
                self.ridden_ms += dt_ms;
                self.seg_elapsed_ms += dt_ms;
                self.fire_texts(&mut fx);
                let mut crossed = false;
                while self.seg_elapsed_ms >= self.seg_dur_ms(self.seg_idx) {
                    self.seg_elapsed_ms -= self.seg_dur_ms(self.seg_idx);
                    fx.push(Effect::LapBoundary { seg_idx: self.seg_idx });
                    self.seg_idx += 1;
                    crossed = true;
                    if self.seg_idx >= self.workout.segments.len() {
                        // Clamp overshoot so active time never exceeds the
                        // workout length on natural completion.
                        self.active_ms = self.total_ms();
                        self.finish(&mut fx);
                        return fx;
                    }
                }
                self.retarget(&mut fx, crossed);
            }
            Input::End => {
                if !matches!(self.phase, Phase::Riding | Phase::Paused) {
                    return fx;
                }
                self.finish(&mut fx);
            }
        }
        fx
    }

    /// Transition to Finished, emitting the completion effects.
    fn finish(&mut self, fx: &mut Vec<Effect>) {
        self.phase = Phase::Finished;
        fx.push(Effect::WorkoutComplete);
        fx.push(Effect::TrainerReset);
    }

    /// Emit the effect that brings the trainer in line with the current
    /// position: `SetTarget` (deduped against the last sent value unless
    /// `force`) or `EnterFreeRide` (once per entry unless `force`).
    fn retarget(&mut self, fx: &mut Vec<Effect>, force: bool) {
        match self.current_target() {
            None => {
                if force || !self.in_free_ride {
                    fx.push(Effect::EnterFreeRide);
                }
                self.in_free_ride = true;
                self.last_target = None;
            }
            Some(w) => {
                if force || self.in_free_ride || self.last_target != Some(w) {
                    fx.push(Effect::SetTarget(w));
                }
                self.in_free_ride = false;
                self.last_target = Some(w);
            }
        }
    }

    /// ERG target in whole watts at the current position, computed at
    /// millisecond precision (ramps interpolate by elapsed-ms fraction).
    /// `None` inside FreeRide.
    fn current_target(&self) -> Option<u16> {
        match &self.workout.segments[self.seg_idx] {
            Segment::Steady { power, .. } => Some(power.resolve(self.ftp, self.intensity)),
            Segment::Ramp {
                duration_s,
                start,
                end,
                ..
            } => {
                let a = f64::from(start.resolve(self.ftp, self.intensity));
                let b = f64::from(end.resolve(self.ftp, self.intensity));
                let dur_ms = u64::from(*duration_s) * 1000;
                let frac = if dur_ms == 0 {
                    0.0
                } else {
                    self.seg_elapsed_ms as f64 / dur_ms as f64
                };
                let w = a + (b - a) * frac;
                Some(w.round().clamp(0.0, f64::from(MAX_TARGET_WATTS)) as u16)
            }
            Segment::FreeRide { .. } => None,
        }
    }

    /// Fire every not-yet-fired text event whose offset has been reached.
    fn fire_texts(&mut self, fx: &mut Vec<Effect>) {
        let cap = self.active_ms.min(self.total_ms());
        while self.next_text < self.workout.text_events.len() {
            let ev = &self.workout.text_events[self.next_text];
            if text_offset_ms(ev) > cap {
                break;
            }
            fx.push(Effect::ShowText(ev.clone()));
            self.next_text += 1;
        }
    }

    fn seg_dur_ms(&self, idx: usize) -> u64 {
        u64::from(self.workout.segments[idx].duration_s()) * 1000
    }

    fn total_ms(&self) -> u64 {
        u64::from(self.workout.duration_s()) * 1000
    }
}

fn text_offset_ms(ev: &TextEvent) -> u64 {
    u64::from(ev.offset_s) * 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PowerTarget, SourceFormat};

    fn wk(segments: Vec<Segment>) -> Workout {
        wk_with_texts(segments, vec![])
    }

    fn wk_with_texts(segments: Vec<Segment>, text_events: Vec<TextEvent>) -> Workout {
        Workout {
            name: "t".into(),
            description: String::new(),
            source_format: SourceFormat::Zwo,
            segments,
            text_events,
        }
    }

    fn steady(duration_s: u32, watts: u16) -> Segment {
        Segment::Steady {
            duration_s,
            power: PowerTarget::Watts(watts),
            cadence_rpm: None,
        }
    }

    fn steady_pct(duration_s: u32, frac: f64) -> Segment {
        Segment::Steady {
            duration_s,
            power: PowerTarget::PercentFtp(frac),
            cadence_rpm: None,
        }
    }

    fn ramp(duration_s: u32, start_w: u16, end_w: u16) -> Segment {
        Segment::Ramp {
            duration_s,
            start: PowerTarget::Watts(start_w),
            end: PowerTarget::Watts(end_w),
            cadence_rpm: None,
        }
    }

    fn free(duration_s: u32) -> Segment {
        Segment::FreeRide { duration_s }
    }

    fn text(offset_s: u32, msg: &str) -> TextEvent {
        TextEvent {
            offset_s,
            message: msg.into(),
            duration_s: 10,
        }
    }

    fn tick(e: &mut Engine) -> Vec<Effect> {
        e.handle(Input::Tick { dt_ms: 250 })
    }

    // ---- Start ----

    #[test]
    fn start_emits_trainer_start_and_initial_target() {
        let mut e = Engine::new(wk(vec![steady(60, 150)]), 250, 1.0);
        assert_eq!(e.phase(), Phase::Ready);
        let fx = e.handle(Input::Start);
        assert_eq!(fx, vec![Effect::TrainerStart, Effect::SetTarget(150)]);
        assert_eq!(e.phase(), Phase::Riding);
        assert_eq!(e.segment_index(), Some(0));
    }

    #[test]
    fn start_into_freeride_emits_enter_freeride() {
        let mut e = Engine::new(wk(vec![free(60)]), 250, 1.0);
        let fx = e.handle(Input::Start);
        assert_eq!(fx, vec![Effect::TrainerStart, Effect::EnterFreeRide]);
    }

    #[test]
    fn start_fires_offset_zero_text_event() {
        let ev = text(0, "hello");
        let mut e = Engine::new(wk_with_texts(vec![steady(60, 100)], vec![ev.clone()]), 250, 1.0);
        let fx = e.handle(Input::Start);
        assert_eq!(
            fx,
            vec![
                Effect::TrainerStart,
                Effect::SetTarget(100),
                Effect::ShowText(ev)
            ]
        );
        // Does not fire again.
        assert_eq!(tick(&mut e), vec![]);
    }

    #[test]
    fn start_on_empty_workout_completes_immediately() {
        let mut e = Engine::new(wk(vec![]), 250, 1.0);
        let fx = e.handle(Input::Start);
        assert_eq!(
            fx,
            vec![
                Effect::TrainerStart,
                Effect::WorkoutComplete,
                Effect::TrainerReset
            ]
        );
        assert_eq!(e.phase(), Phase::Finished);
    }

    // ---- Ramp re-targeting + dedup ----

    #[test]
    fn ramp_retargets_only_on_whole_watt_changes() {
        // 100 -> 108 W over 2 s => +4 W/s => +1 W per 250 ms tick.
        let mut e = Engine::new(wk(vec![ramp(2, 100, 108)]), 250, 1.0);
        assert_eq!(
            e.handle(Input::Start),
            vec![Effect::TrainerStart, Effect::SetTarget(100)]
        );
        assert_eq!(tick(&mut e), vec![Effect::SetTarget(101)]);
        assert_eq!(tick(&mut e), vec![Effect::SetTarget(102)]);
        assert_eq!(tick(&mut e), vec![Effect::SetTarget(103)]);
    }

    #[test]
    fn slow_ramp_dedups_unchanged_targets() {
        // 100 -> 104 W over 4 s => +0.25 W per 250 ms tick; the rounded
        // value changes only when crossing a .5 boundary.
        let mut e = Engine::new(wk(vec![ramp(4, 100, 104)]), 250, 1.0);
        e.handle(Input::Start); // SetTarget(100)
        assert_eq!(tick(&mut e), vec![]); // 100.25 -> 100
        assert_eq!(tick(&mut e), vec![Effect::SetTarget(101)]); // 100.5 -> 101
        assert_eq!(tick(&mut e), vec![]); // 100.75 -> 101
        assert_eq!(tick(&mut e), vec![]); // 101.0 -> 101
        assert_eq!(tick(&mut e), vec![]); // 101.25 -> 101
        assert_eq!(tick(&mut e), vec![Effect::SetTarget(102)]); // 101.5 -> 102
    }

    #[test]
    fn steady_segment_ticks_emit_nothing() {
        let mut e = Engine::new(wk(vec![steady(10, 200)]), 250, 1.0);
        e.handle(Input::Start);
        for _ in 0..8 {
            assert_eq!(tick(&mut e), vec![]);
        }
        assert_eq!(e.active_ms(), 2000);
    }

    // ---- Segment rollover ----

    #[test]
    fn rollover_emits_lap_boundary_and_new_target() {
        let mut e = Engine::new(wk(vec![steady(1, 100), steady(1, 150)]), 250, 1.0);
        e.handle(Input::Start);
        assert_eq!(tick(&mut e), vec![]);
        assert_eq!(tick(&mut e), vec![]);
        assert_eq!(tick(&mut e), vec![]);
        // 4th tick reaches t = 1000 ms: boundary.
        assert_eq!(
            tick(&mut e),
            vec![Effect::LapBoundary { seg_idx: 0 }, Effect::SetTarget(150)]
        );
        assert_eq!(e.segment_index(), Some(1));
    }

    #[test]
    fn rollover_emits_target_even_when_watts_unchanged() {
        let mut e = Engine::new(wk(vec![steady(1, 100), steady(1, 100)]), 250, 1.0);
        e.handle(Input::Start);
        for _ in 0..3 {
            tick(&mut e);
        }
        assert_eq!(
            tick(&mut e),
            vec![Effect::LapBoundary { seg_idx: 0 }, Effect::SetTarget(100)]
        );
    }

    #[test]
    fn huge_tick_crosses_multiple_boundaries_and_finishes() {
        let mut e = Engine::new(wk(vec![steady(1, 100), steady(1, 150)]), 250, 1.0);
        e.handle(Input::Start);
        let fx = e.handle(Input::Tick { dt_ms: 5000 });
        assert_eq!(
            fx,
            vec![
                Effect::LapBoundary { seg_idx: 0 },
                Effect::LapBoundary { seg_idx: 1 },
                Effect::WorkoutComplete,
                Effect::TrainerReset
            ]
        );
        assert_eq!(e.phase(), Phase::Finished);
        // Active time clamped to workout length despite overshoot.
        assert_eq!(e.active_ms(), 2000);
    }

    // ---- FreeRide ----

    #[test]
    fn freeride_entry_emits_enter_freeride_and_no_targets_inside() {
        let mut e = Engine::new(wk(vec![steady(1, 100), free(2)]), 250, 1.0);
        e.handle(Input::Start);
        for _ in 0..3 {
            assert_eq!(tick(&mut e), vec![]);
        }
        assert_eq!(
            tick(&mut e),
            vec![Effect::LapBoundary { seg_idx: 0 }, Effect::EnterFreeRide]
        );
        // No SetTarget (and no repeated EnterFreeRide) inside FreeRide.
        for _ in 0..7 {
            assert_eq!(tick(&mut e), vec![]);
        }
    }

    #[test]
    fn leaving_freeride_reemits_erg_target() {
        let mut e = Engine::new(wk(vec![free(1), steady(1, 100)]), 250, 1.0);
        e.handle(Input::Start);
        for _ in 0..3 {
            tick(&mut e);
        }
        assert_eq!(
            tick(&mut e),
            vec![Effect::LapBoundary { seg_idx: 0 }, Effect::SetTarget(100)]
        );
    }

    // ---- Pause / Resume ----

    #[test]
    fn pause_resume_cycle() {
        let mut e = Engine::new(wk(vec![steady(10, 180)]), 250, 1.0);
        e.handle(Input::Start);
        tick(&mut e);
        assert_eq!(e.handle(Input::Pause), vec![Effect::TrainerStop]);
        assert_eq!(e.phase(), Phase::Paused);
        // Ticks ignored while paused: no effects, no time advance.
        assert_eq!(tick(&mut e), vec![]);
        assert_eq!(tick(&mut e), vec![]);
        assert_eq!(e.active_ms(), 250);
        // Resume re-emits the current target even though it is unchanged.
        assert_eq!(
            e.handle(Input::Resume),
            vec![Effect::TrainerStart, Effect::SetTarget(180)]
        );
        assert_eq!(e.phase(), Phase::Riding);
    }

    #[test]
    fn resume_inside_freeride_reenters_freeride() {
        let mut e = Engine::new(wk(vec![free(10)]), 250, 1.0);
        e.handle(Input::Start);
        e.handle(Input::Pause);
        assert_eq!(
            e.handle(Input::Resume),
            vec![Effect::TrainerStart, Effect::EnterFreeRide]
        );
    }

    // ---- SkipSegment ----

    #[test]
    fn skip_mid_segment_jumps_to_next_boundary() {
        let mut e = Engine::new(wk(vec![steady(10, 100), steady(10, 200)]), 250, 1.0);
        e.handle(Input::Start);
        tick(&mut e); // t = 250 ms, mid-segment
        let fx = e.handle(Input::SkipSegment);
        assert_eq!(
            fx,
            vec![Effect::LapBoundary { seg_idx: 0 }, Effect::SetTarget(200)]
        );
        assert_eq!(e.active_ms(), 10_000); // jumped to boundary
        assert_eq!(e.segment_index(), Some(1));
        assert_eq!(e.phase(), Phase::Riding);
    }

    #[test]
    fn skip_does_not_credit_ridden_time() {
        let mut e = Engine::new(wk(vec![steady(10, 100), steady(10, 200)]), 250, 1.0);
        e.handle(Input::Start);
        tick(&mut e); // 250 ms actually ridden
        e.handle(Input::SkipSegment); // position jumps 9.75 s, the rider does not
        assert_eq!(e.active_ms(), 10_000);
        assert_eq!(e.ridden_ms(), 250);
        tick(&mut e);
        assert_eq!(e.ridden_ms(), 500);
    }

    #[test]
    fn pausing_does_not_advance_ridden_time() {
        let mut e = Engine::new(wk(vec![steady(10, 100)]), 250, 1.0);
        e.handle(Input::Start);
        tick(&mut e);
        e.handle(Input::Pause);
        tick(&mut e); // ignored while paused
        assert_eq!(e.ridden_ms(), 250);
        e.handle(Input::Resume);
        tick(&mut e);
        assert_eq!(e.ridden_ms(), 500);
    }

    #[test]
    fn skip_on_last_segment_ends_the_workout() {
        let mut e = Engine::new(wk(vec![steady(10, 100)]), 250, 1.0);
        e.handle(Input::Start);
        tick(&mut e);
        let fx = e.handle(Input::SkipSegment);
        assert_eq!(
            fx,
            vec![
                Effect::LapBoundary { seg_idx: 0 },
                Effect::WorkoutComplete,
                Effect::TrainerReset
            ]
        );
        assert_eq!(e.phase(), Phase::Finished);
        // Everything is ignored once finished.
        assert_eq!(tick(&mut e), vec![]);
        assert_eq!(e.handle(Input::SkipSegment), vec![]);
    }

    #[test]
    fn skip_drops_text_events_in_the_skipped_span() {
        let skipped = text(5, "skipped");
        let later = text(12, "later");
        let mut e = Engine::new(
            wk_with_texts(vec![steady(10, 100), steady(10, 200)], vec![skipped, later.clone()]),
            250,
            1.0,
        );
        e.handle(Input::Start);
        let fx = e.handle(Input::SkipSegment);
        assert!(!fx.iter().any(|f| matches!(f, Effect::ShowText(_))));
        // Ride to t = 12 s of active time (boundary was 10 s): event fires once.
        for _ in 0..7 {
            assert_eq!(tick(&mut e), vec![]);
        }
        assert_eq!(tick(&mut e), vec![Effect::ShowText(later)]);
    }

    // ---- SetIntensity ----

    #[test]
    fn set_intensity_retargets_percent_segments_immediately() {
        let mut e = Engine::new(wk(vec![steady_pct(60, 0.8)]), 250, 1.0);
        e.handle(Input::Start); // 0.8 * 250 = 200 W
        assert_eq!(e.handle(Input::SetIntensity(1.1)), vec![Effect::SetTarget(220)]);
        assert!((e.intensity() - 1.1).abs() < 1e-12);
    }

    #[test]
    fn set_intensity_clamps_to_bounds() {
        let mut e = Engine::new(wk(vec![steady_pct(60, 1.0)]), 200, 1.0);
        e.handle(Input::Start); // 200 W
        assert_eq!(e.handle(Input::SetIntensity(9.0)), vec![Effect::SetTarget(300)]);
        assert_eq!(e.intensity(), INTENSITY_MAX);
        assert_eq!(e.handle(Input::SetIntensity(0.01)), vec![Effect::SetTarget(100)]);
        assert_eq!(e.intensity(), INTENSITY_MIN);
    }

    #[test]
    fn set_intensity_noop_when_target_unchanged() {
        // Absolute-watt targets ignore intensity: no SetTarget.
        let mut e = Engine::new(wk(vec![steady(60, 150)]), 250, 1.0);
        e.handle(Input::Start);
        assert_eq!(e.handle(Input::SetIntensity(1.2)), vec![]);
        assert!((e.intensity() - 1.2).abs() < 1e-12);
        // Same clamped value twice on a percent segment: second is a no-op.
        let mut e2 = Engine::new(wk(vec![steady_pct(60, 1.0)]), 200, 1.0);
        e2.handle(Input::Start);
        assert_eq!(e2.handle(Input::SetIntensity(1.1)), vec![Effect::SetTarget(220)]);
        assert_eq!(e2.handle(Input::SetIntensity(1.1)), vec![]);
    }

    #[test]
    fn set_intensity_in_freeride_emits_nothing() {
        let mut e = Engine::new(wk(vec![free(60)]), 250, 1.0);
        e.handle(Input::Start);
        assert_eq!(e.handle(Input::SetIntensity(1.2)), vec![]);
    }

    #[test]
    fn set_intensity_while_paused_updates_and_applies_on_resume() {
        let mut e = Engine::new(wk(vec![steady_pct(60, 1.0)]), 200, 1.0);
        e.handle(Input::Start); // 200 W
        e.handle(Input::Pause);
        assert_eq!(e.handle(Input::SetIntensity(1.25)), vec![]);
        assert_eq!(
            e.handle(Input::Resume),
            vec![Effect::TrainerStart, Effect::SetTarget(250)]
        );
    }

    #[test]
    fn new_clamps_initial_intensity() {
        let e = Engine::new(wk(vec![steady(60, 100)]), 250, 7.0);
        assert_eq!(e.intensity(), INTENSITY_MAX);
    }

    // ---- Text events ----

    #[test]
    fn text_event_fires_exactly_once_at_offset() {
        let ev = text(1, "go");
        let mut e = Engine::new(wk_with_texts(vec![steady(60, 100)], vec![ev.clone()]), 250, 1.0);
        e.handle(Input::Start);
        assert_eq!(tick(&mut e), vec![]); // 250
        assert_eq!(tick(&mut e), vec![]); // 500
        assert_eq!(tick(&mut e), vec![]); // 750
        assert_eq!(tick(&mut e), vec![Effect::ShowText(ev)]); // 1000
        for _ in 0..8 {
            assert_eq!(tick(&mut e), vec![]);
        }
    }

    #[test]
    fn text_event_does_not_fire_during_pause() {
        let ev = text(1, "go");
        let mut e = Engine::new(wk_with_texts(vec![steady(60, 100)], vec![ev.clone()]), 250, 1.0);
        e.handle(Input::Start);
        tick(&mut e); // 250
        tick(&mut e); // 500
        e.handle(Input::Pause);
        // Wall-clock time passes but active time does not: no fire.
        for _ in 0..10 {
            assert_eq!(tick(&mut e), vec![]);
        }
        e.handle(Input::Resume);
        assert_eq!(tick(&mut e), vec![]); // 750
        assert_eq!(tick(&mut e), vec![Effect::ShowText(ev)]); // 1000 active
    }

    #[test]
    fn multiple_text_events_fire_in_order() {
        let a = text(1, "a");
        let b = text(1, "b");
        let c = text(2, "c");
        let mut e = Engine::new(
            wk_with_texts(vec![steady(60, 100)], vec![a.clone(), b.clone(), c.clone()]),
            250,
            1.0,
        );
        e.handle(Input::Start);
        // One big tick past both offsets: all fire, in order, once.
        let fx = e.handle(Input::Tick { dt_ms: 2000 });
        assert_eq!(
            fx,
            vec![Effect::ShowText(a), Effect::ShowText(b), Effect::ShowText(c)]
        );
        assert_eq!(tick(&mut e), vec![]);
    }

    // ---- Completion ----

    #[test]
    fn natural_completion_emits_complete_and_reset() {
        let mut e = Engine::new(wk(vec![steady(1, 120)]), 250, 1.0);
        e.handle(Input::Start);
        for _ in 0..3 {
            assert_eq!(tick(&mut e), vec![]);
        }
        assert_eq!(
            tick(&mut e),
            vec![
                Effect::LapBoundary { seg_idx: 0 },
                Effect::WorkoutComplete,
                Effect::TrainerReset
            ]
        );
        assert_eq!(e.phase(), Phase::Finished);
        assert_eq!(e.active_ms(), 1000);
        assert_eq!(e.segment_index(), None);
    }

    #[test]
    fn end_input_finishes_from_riding_and_paused() {
        let mut e = Engine::new(wk(vec![steady(60, 100)]), 250, 1.0);
        e.handle(Input::Start);
        tick(&mut e);
        assert_eq!(
            e.handle(Input::End),
            vec![Effect::WorkoutComplete, Effect::TrainerReset]
        );
        assert_eq!(e.phase(), Phase::Finished);
        // Early end keeps the actual active time (no clamp to total).
        assert_eq!(e.active_ms(), 250);

        let mut e2 = Engine::new(wk(vec![steady(60, 100)]), 250, 1.0);
        e2.handle(Input::Start);
        e2.handle(Input::Pause);
        assert_eq!(
            e2.handle(Input::End),
            vec![Effect::WorkoutComplete, Effect::TrainerReset]
        );
        assert_eq!(e2.phase(), Phase::Finished);
    }

    // ---- Phase-invalid inputs ----

    #[test]
    fn phase_invalid_inputs_are_ignored() {
        // Ready: everything except Start/SetIntensity is invalid.
        let mut e = Engine::new(wk(vec![steady(60, 100)]), 250, 1.0);
        assert_eq!(e.handle(Input::Pause), vec![]);
        assert_eq!(e.handle(Input::Resume), vec![]);
        assert_eq!(e.handle(Input::SkipSegment), vec![]);
        assert_eq!(tick(&mut e), vec![]);
        assert_eq!(e.handle(Input::End), vec![]);
        assert_eq!(e.phase(), Phase::Ready);
        assert_eq!(e.active_ms(), 0);

        // Riding: Start and Resume are invalid.
        e.handle(Input::Start);
        assert_eq!(e.handle(Input::Start), vec![]);
        assert_eq!(e.handle(Input::Resume), vec![]);

        // Paused: Start, Pause, SkipSegment, Tick are invalid.
        e.handle(Input::Pause);
        assert_eq!(e.handle(Input::Start), vec![]);
        assert_eq!(e.handle(Input::Pause), vec![]);
        assert_eq!(e.handle(Input::SkipSegment), vec![]);
        assert_eq!(tick(&mut e), vec![]);
        assert_eq!(e.phase(), Phase::Paused);

        // Finished: everything is invalid.
        e.handle(Input::End);
        assert_eq!(e.phase(), Phase::Finished);
        assert_eq!(e.handle(Input::Start), vec![]);
        assert_eq!(e.handle(Input::Pause), vec![]);
        assert_eq!(e.handle(Input::Resume), vec![]);
        assert_eq!(e.handle(Input::SkipSegment), vec![]);
        assert_eq!(e.handle(Input::SetIntensity(1.2)), vec![]);
        assert_eq!(tick(&mut e), vec![]);
        assert_eq!(e.handle(Input::End), vec![]);
    }

    // ---- Full workout end-to-end snapshot ----

    #[test]
    fn full_small_workout_effect_stream_snapshot() {
        let hello = text(0, "hello");
        let go = text(1, "go");
        let w = wk_with_texts(
            vec![steady(1, 100), ramp(2, 100, 108), free(1)],
            vec![hello.clone(), go.clone()],
        );
        let mut e = Engine::new(w, 250, 1.0);

        let mut stream: Vec<Vec<Effect>> = Vec::new();
        stream.push(e.handle(Input::Start));
        for _ in 0..16 {
            stream.push(tick(&mut e));
        }

        let expected: Vec<Vec<Effect>> = vec![
            // Start
            vec![
                Effect::TrainerStart,
                Effect::SetTarget(100),
                Effect::ShowText(hello),
            ],
            vec![], // t=250
            vec![], // t=500
            vec![], // t=750
            // t=1000: text at offset 1 s, boundary into ramp (start 100 W).
            vec![
                Effect::ShowText(go),
                Effect::LapBoundary { seg_idx: 0 },
                Effect::SetTarget(100),
            ],
            vec![Effect::SetTarget(101)], // t=1250 (250 ms into 4 W/s ramp)
            vec![Effect::SetTarget(102)], // t=1500
            vec![Effect::SetTarget(103)], // t=1750
            vec![Effect::SetTarget(104)], // t=2000
            vec![Effect::SetTarget(105)], // t=2250
            vec![Effect::SetTarget(106)], // t=2500
            vec![Effect::SetTarget(107)], // t=2750
            // t=3000: boundary into FreeRide.
            vec![Effect::LapBoundary { seg_idx: 1 }, Effect::EnterFreeRide],
            vec![], // t=3250
            vec![], // t=3500
            vec![], // t=3750
            // t=4000: workout complete.
            vec![
                Effect::LapBoundary { seg_idx: 2 },
                Effect::WorkoutComplete,
                Effect::TrainerReset,
            ],
        ];
        assert_eq!(stream, expected);
        assert_eq!(e.phase(), Phase::Finished);
        assert_eq!(e.active_ms(), 4000);
    }
}
