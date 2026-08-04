//! Power metrics: NP/IF/TSS, session totals, zones, library estimates.
//! SPEC.md §6 (totals), §2 (estimates). Pure functions only.

use crate::consts::NP_WINDOW_S;
use crate::journal::{RideData, RideEventKind};
use crate::model::Workout;

/// Coggan-style 7-zone mapping from watts at a given FTP.
/// Boundaries (% FTP): Z1 <55, Z2 55–75, Z3 76–90, Z4 91–105, Z5 106–120,
/// Z6 121–150, Z7 >150. Returns 1..=7.
pub fn zone_for(power_w: u16, ftp: u16) -> u8 {
    if ftp == 0 {
        // Degenerate: no FTP to scale against. 0 W is trivially Z1; any
        // positive power is an unbounded fraction of FTP → Z7.
        return if power_w == 0 { 1 } else { 7 };
    }
    let pct = f64::from(power_w) * 100.0 / f64::from(ftp);
    if pct < 55.0 {
        1
    } else if pct <= 75.0 {
        2
    } else if pct <= 90.0 {
        3
    } else if pct <= 105.0 {
        4
    } else if pct <= 120.0 {
        5
    } else if pct <= 150.0 {
        6
    } else {
        7
    }
}

/// Normalized Power over a 1 Hz power series (missing samples = 0 W):
/// 30 s rolling mean (NP_WINDOW_S) → mean of 4th powers → 4th root.
/// Series shorter than the window: NP = plain average (rounded).
pub fn normalized_power(power_1hz: &[u16]) -> u16 {
    let n = power_1hz.len();
    if n == 0 {
        return 0;
    }
    let w = NP_WINDOW_S as usize;
    if n < w {
        let sum: u64 = power_1hz.iter().map(|&p| u64::from(p)).sum();
        return (sum as f64 / n as f64).round() as u16;
    }
    // Sliding 30-sample window over the series: n - w + 1 windows.
    let mut window_sum: u64 = power_1hz[..w].iter().map(|&p| u64::from(p)).sum();
    let mut sum_pow4 = (window_sum as f64 / w as f64).powi(4);
    for i in w..n {
        window_sum += u64::from(power_1hz[i]);
        window_sum -= u64::from(power_1hz[i - w]);
        sum_pow4 += (window_sum as f64 / w as f64).powi(4);
    }
    let mean_pow4 = sum_pow4 / (n - w + 1) as f64;
    mean_pow4.powf(0.25).round() as u16
}

pub fn intensity_factor(np: u16, ftp: u16) -> f64 {
    if ftp == 0 {
        return 0.0;
    }
    f64::from(np) / f64::from(ftp)
}

/// TSS = timer_s × NP × IF / (FTP × 3600) × 100.
pub fn tss(timer_s: u32, np: u16, ftp: u16) -> f64 {
    if ftp == 0 {
        return 0.0;
    }
    let if_ = intensity_factor(np, ftp);
    f64::from(timer_s) * f64::from(np) * if_ / (f64::from(ftp) * 3600.0) * 100.0
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionTotals {
    /// Wall-clock ride span (includes pauses).
    pub elapsed_s: u32,
    /// Moving time (pauses excluded).
    pub timer_s: u32,
    pub avg_power: Option<u16>,
    pub max_power: Option<u16>,
    pub np: Option<u16>,
    pub if_: Option<f64>,
    pub tss: Option<f64>,
    pub avg_hr: Option<u16>,
    pub max_hr: Option<u16>,
    pub avg_cadence: Option<u16>,
    /// Total work in kJ (Σ power × 1 s / 1000).
    pub kj: u32,
}

/// Round a millisecond timeline value to whole seconds (half-up).
fn ms_to_s(ms: u64) -> u32 {
    ((ms + 500) / 1000) as u32
}

/// Average (rounded half-up) and max over present values; (None, None) if
/// no value is present.
fn avg_max(values: impl Iterator<Item = u16>) -> (Option<u16>, Option<u16>) {
    let mut sum = 0u64;
    let mut n = 0u64;
    let mut max = 0u16;
    for v in values {
        sum += u64::from(v);
        n += 1;
        if v > max {
            max = v;
        }
    }
    if n == 0 {
        (None, None)
    } else {
        (Some((sum as f64 / n as f64).round() as u16), Some(max))
    }
}

/// Session totals from a replayed ride. Timer time = elapsed minus
/// pause→resume gaps (from events). Power/HR/cadence averages are over
/// samples where the field is present; NP treats absent power as 0.
pub fn session_totals(data: &RideData, ftp: u16) -> SessionTotals {
    // Ride span: from t = 0 (start) to the latest timestamp seen on any
    // event or sample (the End event, when present, is the latest).
    let end_ms = data
        .events
        .iter()
        .map(|e| e.t_ms)
        .chain(data.samples.iter().map(|s| s.t_ms))
        .max()
        .unwrap_or(0);

    // Sum pause→resume gaps; a trailing unmatched pause (ride ended while
    // paused) extends to the end of the ride.
    let mut paused_ms = 0u64;
    let mut pause_start: Option<u64> = None;
    for e in &data.events {
        match e.kind {
            RideEventKind::Pause => {
                if pause_start.is_none() {
                    pause_start = Some(e.t_ms);
                }
            }
            RideEventKind::Resume => {
                if let Some(p) = pause_start.take() {
                    paused_ms += e.t_ms.saturating_sub(p);
                }
            }
            _ => {}
        }
    }
    if let Some(p) = pause_start {
        paused_ms += end_ms.saturating_sub(p);
    }

    let elapsed_s = ms_to_s(end_ms);
    let timer_s = ms_to_s(end_ms.saturating_sub(paused_ms));

    let (avg_power, max_power) = avg_max(data.samples.iter().filter_map(|s| s.power));
    let (avg_hr, max_hr) = avg_max(data.samples.iter().filter_map(|s| s.hr));
    let (avg_cadence, _) = avg_max(data.samples.iter().filter_map(|s| s.cadence));

    // kJ: each 1 Hz sample contributes power × 1 s joules; absent power = 0 J.
    let joules: u64 = data
        .samples
        .iter()
        .filter_map(|s| s.power)
        .map(u64::from)
        .sum();
    let kj = ((joules as f64) / 1000.0).round() as u32;

    // NP over the full 1 Hz series (absent power = 0 W), only meaningful if
    // any power data exists at all.
    let np = if avg_power.is_some() {
        let series: Vec<u16> = data.samples.iter().map(|s| s.power.unwrap_or(0)).collect();
        Some(normalized_power(&series))
    } else {
        None
    };
    let if_ = match np {
        Some(np) if ftp > 0 => Some(intensity_factor(np, ftp)),
        _ => None,
    };
    let tss_v = match np {
        Some(np) if ftp > 0 => Some(tss(timer_s, np, ftp)),
        _ => None,
    };

    SessionTotals {
        elapsed_s,
        timer_s,
        avg_power,
        max_power,
        np,
        if_,
        tss: tss_v,
        avg_hr,
        max_hr,
        avg_cadence,
        kj,
    }
}

/// Library-display estimate for a workout at a given FTP: simulate the
/// target power series at 1 Hz (FreeRide counts as 0 W), run NP/IF/TSS.
/// Returns (IF, TSS).
pub fn estimate_if_tss(workout: &Workout, ftp: u16) -> (f64, f64) {
    let duration_s = workout.duration_s();
    if duration_s == 0 || ftp == 0 {
        return (0.0, 0.0);
    }
    let series: Vec<u16> = (0..duration_s)
        .map(|t| workout.target_at(t, ftp, 1.0).unwrap_or(0))
        .collect();
    let np = normalized_power(&series);
    (intensity_factor(np, ftp), tss(duration_s, np, ftp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{JournalHeader, RideEvent, Sample};
    use crate::model::{PowerTarget, Segment, SourceFormat};

    const EPS: f64 = 1e-9;

    fn header() -> JournalHeader {
        JournalHeader {
            ride_id: "r1".into(),
            started_unix_ms: 1_700_000_000_000,
            workout_name: "test".into(),
            ftp: 250,
            weight_kg: 72.0,
            trainer: Some("SimTrainer".into()),
            hrm: None,
            app_ver: "0.1.0".into(),
        }
    }

    fn sample(t_s: u64, power: Option<u16>) -> Sample {
        Sample {
            t_ms: t_s * 1000,
            power,
            cadence: None,
            hr: None,
            target: None,
        }
    }

    fn event(t_ms: u64, kind: RideEventKind) -> RideEvent {
        RideEvent {
            t_ms,
            kind,
            seg: None,
        }
    }

    fn workout(segments: Vec<Segment>) -> Workout {
        Workout {
            name: "w".into(),
            description: String::new(),
            source_format: SourceFormat::Zwo,
            segments,
            text_events: vec![],
        }
    }

    // ---------- zone_for ----------

    #[test]
    fn zones_at_every_threshold() {
        let ftp = 200;
        // Z1 < 55 %: 55 % of 200 = 110 W.
        assert_eq!(zone_for(0, ftp), 1);
        assert_eq!(zone_for(109, ftp), 1); // 54.5 %
        assert_eq!(zone_for(110, ftp), 2); // 55.0 %
        // Z2 ends at 75 % = 150 W.
        assert_eq!(zone_for(150, ftp), 2); // 75.0 %
        assert_eq!(zone_for(151, ftp), 3); // 75.5 %
        // Z3 ends at 90 % = 180 W.
        assert_eq!(zone_for(180, ftp), 3);
        assert_eq!(zone_for(181, ftp), 4);
        // Z4 ends at 105 % = 210 W.
        assert_eq!(zone_for(210, ftp), 4);
        assert_eq!(zone_for(211, ftp), 5);
        // Z5 ends at 120 % = 240 W.
        assert_eq!(zone_for(240, ftp), 5);
        assert_eq!(zone_for(241, ftp), 6);
        // Z6 ends at 150 % = 300 W.
        assert_eq!(zone_for(300, ftp), 6);
        assert_eq!(zone_for(301, ftp), 7);
        assert_eq!(zone_for(2000, ftp), 7);
    }

    #[test]
    fn zones_with_zero_ftp_do_not_panic() {
        assert_eq!(zone_for(0, 0), 1);
        assert_eq!(zone_for(100, 0), 7);
    }

    // ---------- normalized_power ----------

    #[test]
    fn np_equals_avg_for_constant_power() {
        // Constant series: every 30 s rolling mean equals P, so the mean of
        // 4th powers is P^4 and the 4th root is P — NP must equal the average.
        let series = vec![200u16; 120];
        assert_eq!(normalized_power(&series), 200);
        // Also for a constant series exactly one window long.
        assert_eq!(normalized_power(&[137u16; 30]), 137);
    }

    #[test]
    fn np_hand_computed_two_window_series() {
        // Series: 30 samples of 0 W, then one sample of 300 W (31 samples).
        // Rolling 30 s windows (31 - 30 + 1 = 2):
        //   window 0 = samples[0..30]  → mean = 0
        //   window 1 = samples[1..31]  → mean = (29·0 + 300)/30 = 10
        // Mean of 4th powers = (0^4 + 10^4)/2 = 5000
        // NP = 5000^(1/4) = 8.4089…  → rounds to 8.
        let mut series = vec![0u16; 30];
        series.push(300);
        assert_eq!(normalized_power(&series), 8);
    }

    #[test]
    fn np_hand_computed_step_series() {
        // Series: 30 samples @ 100 W then 30 samples @ 300 W (60 samples).
        // There are 60 - 30 + 1 = 31 rolling windows. Window k (start index
        // k, k = 0..=30) contains (30-k) samples of 100 and k samples of 300:
        //   mean_k = (100·(30-k) + 300·k)/30 = 100 + 200k/30
        // NP = round( ( Σ_k mean_k^4 / 31 )^(1/4) ).
        // Expected value computed here from the closed form above,
        // independently of the implementation's sliding-window code.
        let series: Vec<u16> = std::iter::repeat_n(100u16, 30)
            .chain(std::iter::repeat_n(300u16, 30))
            .collect();
        let sum_pow4: f64 = (0..=30)
            .map(|k| (100.0 + 200.0 * k as f64 / 30.0).powi(4))
            .sum();
        let expected = (sum_pow4 / 31.0).powf(0.25).round() as u16;
        // Sanity: the window means average to 200, and the 4th-power
        // weighting pushes NP above that mean, but below the max mean (300).
        // The exact value from the closed form is 223
        // ( (Σ mean_k^4 / 31)^(1/4) = 222.98… → 223 ).
        assert_eq!(expected, 223);
        assert_eq!(normalized_power(&series), expected);
    }

    #[test]
    fn np_series_shorter_than_window_is_plain_average() {
        // 3 samples < 30: NP = round((100+200+250)/3) = round(183.33) = 183.
        assert_eq!(normalized_power(&[100, 200, 250]), 183);
        // 2 samples: round(150.0) = 150.
        assert_eq!(normalized_power(&[100, 200]), 150);
        // 29 samples (one short of the window) still averages.
        let series = vec![100u16; 29];
        assert_eq!(normalized_power(&series), 100);
        // Rounding half-up: (100+101)/2 = 100.5 → 101.
        assert_eq!(normalized_power(&[100, 101]), 101);
    }

    #[test]
    fn np_empty_series_is_zero() {
        assert_eq!(normalized_power(&[]), 0);
    }

    #[test]
    fn np_exactly_one_window() {
        // 30 samples: single window, NP = its mean = (15·0 + 15·200)/30 = 100.
        let series: Vec<u16> = std::iter::repeat_n(0u16, 15)
            .chain(std::iter::repeat_n(200u16, 15))
            .collect();
        assert_eq!(normalized_power(&series), 100);
    }

    // ---------- intensity_factor / tss ----------

    #[test]
    fn if_and_tss_fixtures() {
        // 1 h at exactly FTP → IF 1.0, TSS 100.
        assert!((intensity_factor(250, 250) - 1.0).abs() < EPS);
        assert!((tss(3600, 250, 250) - 100.0).abs() < EPS);
        // Half an hour at FTP → TSS 50.
        assert!((tss(1800, 250, 250) - 50.0).abs() < EPS);
        // 1 h at NP 200, FTP 250: IF 0.8, TSS = 3600·200·0.8/(250·3600)·100 = 64.
        assert!((intensity_factor(200, 250) - 0.8).abs() < EPS);
        assert!((tss(3600, 200, 250) - 64.0).abs() < EPS);
        // Zero timer → zero TSS.
        assert!((tss(0, 250, 250)).abs() < EPS);
    }

    #[test]
    fn if_and_tss_zero_ftp_guard() {
        assert!((intensity_factor(200, 0)).abs() < EPS);
        assert!((tss(3600, 200, 0)).abs() < EPS);
    }

    // ---------- session_totals ----------

    #[test]
    fn session_totals_one_hour_at_ftp() {
        // 3600 samples @ 250 W, End event at exactly 1 h.
        let samples: Vec<Sample> = (0..3600).map(|t| sample(t, Some(250))).collect();
        let events = vec![
            event(0, RideEventKind::Start),
            event(3_600_000, RideEventKind::End),
        ];
        let data = RideData {
            header: header(),
            samples,
            events,
        };
        let t = session_totals(&data, 250);
        assert_eq!(t.elapsed_s, 3600);
        assert_eq!(t.timer_s, 3600);
        assert_eq!(t.avg_power, Some(250));
        assert_eq!(t.max_power, Some(250));
        assert_eq!(t.np, Some(250));
        assert!((t.if_.unwrap() - 1.0).abs() < EPS);
        assert!((t.tss.unwrap() - 100.0).abs() < EPS);
        // kJ = 3600 s × 250 W / 1000 = 900.
        assert_eq!(t.kj, 900);
        assert_eq!(t.avg_hr, None);
        assert_eq!(t.max_hr, None);
        assert_eq!(t.avg_cadence, None);
    }

    #[test]
    fn session_totals_with_pause_gap_and_sparse_hr_cadence() {
        // Timeline: ride 10 s → pause 10 s → ride 5 s → end at 25 s.
        // Samples only while moving: t = 0..9 (@100 W) and t = 20..24 (@200 W).
        let mut samples: Vec<Sample> = Vec::new();
        for t in 0..10u64 {
            let mut s = sample(t, Some(100));
            // HR present on the first 6 samples only: 4×140 then 2×150.
            s.hr = if t < 4 {
                Some(140)
            } else if t < 6 {
                Some(150)
            } else {
                None
            };
            // Cadence present on even seconds only.
            s.cadence = if t % 2 == 0 { Some(90) } else { None };
            samples.push(s);
        }
        for t in 20..25u64 {
            samples.push(sample(t, Some(200)));
        }
        let events = vec![
            event(0, RideEventKind::Start),
            event(10_000, RideEventKind::Pause),
            event(20_000, RideEventKind::Resume),
            event(25_000, RideEventKind::End),
        ];
        let data = RideData {
            header: header(),
            samples,
            events,
        };
        let t = session_totals(&data, 200);

        // Elapsed includes the pause; timer excludes it.
        assert_eq!(t.elapsed_s, 25);
        assert_eq!(t.timer_s, 15);

        // avg power over the 15 samples with power:
        // (10·100 + 5·200)/15 = 2000/15 = 133.33 → 133.
        assert_eq!(t.avg_power, Some(133));
        assert_eq!(t.max_power, Some(200));

        // 15-sample series is shorter than the 30 s NP window → NP = avg.
        assert_eq!(t.np, Some(133));
        assert!((t.if_.unwrap() - 133.0 / 200.0).abs() < EPS);
        let expected_tss = 15.0 * 133.0 * (133.0 / 200.0) / (200.0 * 3600.0) * 100.0;
        assert!((t.tss.unwrap() - expected_tss).abs() < EPS);

        // HR averaged over the 6 present samples: (4·140 + 2·150)/6 = 143.33 → 143.
        assert_eq!(t.avg_hr, Some(143));
        assert_eq!(t.max_hr, Some(150));
        // Cadence over 5 present samples (t = 0,2,4,6,8), all 90.
        assert_eq!(t.avg_cadence, Some(90));

        // kJ accumulation: 10·100 + 5·200 = 2000 J → 2 kJ.
        assert_eq!(t.kj, 2);
    }

    #[test]
    fn session_totals_ride_ending_while_paused() {
        // Pause at 10 s, End at 15 s with no Resume: the trailing paused
        // stretch counts toward elapsed but not timer.
        let samples: Vec<Sample> = (0..10).map(|t| sample(t, Some(150))).collect();
        let events = vec![
            event(0, RideEventKind::Start),
            event(10_000, RideEventKind::Pause),
            event(15_000, RideEventKind::End),
        ];
        let data = RideData {
            header: header(),
            samples,
            events,
        };
        let t = session_totals(&data, 200);
        assert_eq!(t.elapsed_s, 15);
        assert_eq!(t.timer_s, 10);
        assert_eq!(t.avg_power, Some(150));
    }

    #[test]
    fn session_totals_multiple_pauses() {
        let samples: Vec<Sample> = (0..5)
            .chain(10..15)
            .chain(25..30)
            .map(|t| sample(t, Some(100)))
            .collect();
        let events = vec![
            event(0, RideEventKind::Start),
            event(5_000, RideEventKind::Pause),
            event(10_000, RideEventKind::Resume),
            event(15_000, RideEventKind::Pause),
            event(25_000, RideEventKind::Resume),
            event(30_000, RideEventKind::End),
        ];
        let data = RideData {
            header: header(),
            samples,
            events,
        };
        let t = session_totals(&data, 200);
        assert_eq!(t.elapsed_s, 30);
        assert_eq!(t.timer_s, 15); // 30 − (5 + 10) paused
    }

    #[test]
    fn session_totals_missing_power_entirely() {
        // HR-only ride: no power on any sample → power-derived fields None,
        // kJ 0; HR still aggregates.
        let samples: Vec<Sample> = (0..40)
            .map(|t| {
                let mut s = sample(t, None);
                s.hr = Some(120);
                s
            })
            .collect();
        let events = vec![
            event(0, RideEventKind::Start),
            event(40_000, RideEventKind::End),
        ];
        let data = RideData {
            header: header(),
            samples,
            events,
        };
        let t = session_totals(&data, 250);
        assert_eq!(t.avg_power, None);
        assert_eq!(t.max_power, None);
        assert_eq!(t.np, None);
        assert_eq!(t.if_, None);
        assert_eq!(t.tss, None);
        assert_eq!(t.kj, 0);
        assert_eq!(t.avg_hr, Some(120));
        assert_eq!(t.max_hr, Some(120));
    }

    #[test]
    fn session_totals_gap_samples_treated_as_zero_for_np() {
        // 30 samples @ 300 W then 30 samples with power absent. NP must use
        // 0 W for the absent half, while avg_power uses present samples only.
        let mut samples: Vec<Sample> = (0..30).map(|t| sample(t, Some(300))).collect();
        for t in 30..60u64 {
            samples.push(sample(t, None));
        }
        let events = vec![
            event(0, RideEventKind::Start),
            event(60_000, RideEventKind::End),
        ];
        let data = RideData {
            header: header(),
            samples,
            events,
        };
        let t = session_totals(&data, 250);
        assert_eq!(t.avg_power, Some(300)); // present samples only
        let series: Vec<u16> = std::iter::repeat_n(300u16, 30)
            .chain(std::iter::repeat_n(0u16, 30))
            .collect();
        assert_eq!(t.np, Some(normalized_power(&series)));
        assert!(t.np.unwrap() < 300); // zeros drag NP below the present-avg
        assert_eq!(t.kj, 9); // 30 × 300 J = 9000 J
    }

    #[test]
    fn session_totals_empty_ride() {
        let data = RideData {
            header: header(),
            samples: vec![],
            events: vec![
                event(0, RideEventKind::Start),
                event(5_000, RideEventKind::End),
            ],
        };
        let t = session_totals(&data, 250);
        assert_eq!(t.elapsed_s, 5);
        assert_eq!(t.timer_s, 5);
        assert_eq!(t.avg_power, None);
        assert_eq!(t.np, None);
        assert_eq!(t.if_, None);
        assert_eq!(t.tss, None);
        assert_eq!(t.kj, 0);
    }

    #[test]
    fn session_totals_zero_ftp_skips_if_tss() {
        let samples: Vec<Sample> = (0..10).map(|t| sample(t, Some(150))).collect();
        let data = RideData {
            header: header(),
            samples,
            events: vec![event(0, RideEventKind::Start)],
        };
        let t = session_totals(&data, 0);
        assert_eq!(t.np, Some(150));
        assert_eq!(t.if_, None);
        assert_eq!(t.tss, None);
    }

    #[test]
    fn ms_to_s_rounds_half_up() {
        assert_eq!(ms_to_s(0), 0);
        assert_eq!(ms_to_s(499), 0);
        assert_eq!(ms_to_s(500), 1);
        assert_eq!(ms_to_s(25_499), 25);
        assert_eq!(ms_to_s(25_500), 26);
    }

    // ---------- estimate_if_tss ----------

    #[test]
    fn estimate_one_hour_at_ftp() {
        let w = workout(vec![Segment::Steady {
            duration_s: 3600,
            power: PowerTarget::PercentFtp(1.0),
            cadence_rpm: None,
        }]);
        let (if_, tss_v) = estimate_if_tss(&w, 250);
        assert!((if_ - 1.0).abs() < EPS);
        assert!((tss_v - 100.0).abs() < EPS);
    }

    #[test]
    fn estimate_with_freeride_counts_as_zero_watts() {
        // 60 s steady @ 200 W + 60 s FreeRide at FTP 200. The FreeRide half
        // must enter the series as 0 W, so the estimate matches NP/IF/TSS of
        // the explicit [200×60, 0×60] series over the full 120 s duration.
        let w = workout(vec![
            Segment::Steady {
                duration_s: 60,
                power: PowerTarget::Watts(200),
                cadence_rpm: None,
            },
            Segment::FreeRide { duration_s: 60 },
        ]);
        let ftp = 200;
        let series: Vec<u16> = std::iter::repeat_n(200u16, 60)
            .chain(std::iter::repeat_n(0u16, 60))
            .collect();
        let np = normalized_power(&series);
        let (if_, tss_v) = estimate_if_tss(&w, ftp);
        assert!((if_ - intensity_factor(np, ftp)).abs() < EPS);
        assert!((tss_v - tss(120, np, ftp)).abs() < EPS);
        // FreeRide zeros must pull IF strictly below the steady-only 1.0.
        assert!(if_ < 1.0);
        assert!(if_ > 0.0);
    }

    #[test]
    fn estimate_ramp_matches_target_series() {
        let w = workout(vec![Segment::Ramp {
            duration_s: 300,
            start: PowerTarget::PercentFtp(0.5),
            end: PowerTarget::PercentFtp(1.0),
            cadence_rpm: None,
        }]);
        let ftp = 200;
        let series: Vec<u16> = (0..300).map(|t| w.target_at(t, ftp, 1.0).unwrap()).collect();
        let np = normalized_power(&series);
        let (if_, tss_v) = estimate_if_tss(&w, ftp);
        assert!((if_ - intensity_factor(np, ftp)).abs() < EPS);
        assert!((tss_v - tss(300, np, ftp)).abs() < EPS);
        // Ramp 50 % → 100 % averages ~75 %; NP-weighted IF sits in (0.5, 1.0).
        assert!(if_ > 0.5 && if_ < 1.0);
    }

    #[test]
    fn estimate_degenerate_inputs() {
        let empty = workout(vec![]);
        assert_eq!(estimate_if_tss(&empty, 250), (0.0, 0.0));
        let w = workout(vec![Segment::Steady {
            duration_s: 60,
            power: PowerTarget::Watts(200),
            cadence_rpm: None,
        }]);
        assert_eq!(estimate_if_tss(&w, 0), (0.0, 0.0));
        // All-FreeRide workout: series is all zeros → IF 0, TSS 0.
        let fr = workout(vec![Segment::FreeRide { duration_s: 120 }]);
        assert_eq!(estimate_if_tss(&fr, 250), (0.0, 0.0));
    }
}
