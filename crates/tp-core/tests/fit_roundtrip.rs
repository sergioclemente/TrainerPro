//! Exercise the shared reader against exported instructions and partial laps.

use tp_core::fit::{decode_activity, encode_activity, FitActivity, FitInstructionTarget};
use tp_core::journal::{
    compute_activity_segments, JournalHeader, Sample, SessionEvent, SessionEventKind,
    SessionRecording,
};
use tp_core::metrics::session_totals;
use tp_core::model::{ExecutableWorkout, PowerTarget, WorkoutSegment};

const FTP_W: u16 = 250;
const STEP_DURATION_S: u32 = 10;
const RIDE_DURATION_S: u32 = 15;
const MS_PER_SECOND: u64 = 1000;
const START_UNIX_MS: u64 = 1_750_000_000_000;

#[test]
fn watt_and_open_targets_round_trip_without_linking_a_partial_lap() {
    let workout = ExecutableWorkout {
        name: "Watts and free ride".into(),
        description: String::new(),
        segments: vec![
            WorkoutSegment::Steady {
                duration_s: STEP_DURATION_S,
                power: PowerTarget::Watts(275),
                cadence_rpm: Some(95),
            },
            WorkoutSegment::FreeRide {
                duration_s: STEP_DURATION_S,
                cadence_rpm: None,
            },
        ],
        text_events: vec![],
    };
    let recording = SessionRecording {
        header: JournalHeader {
            workout_session_id: "fit-roundtrip".into(),
            workout_definition_id: "watts-and-free-ride".into(),
            scheduled_workout_id: None,
            workout_definition_snapshot_json: String::new(),
            started_unix_ms: START_UNIX_MS,
            started_utc_offset_s: None,
            workout_name: workout.name.clone(),
            ftp_w: FTP_W,
            weight_kg: 72.0,
            record_distance: false,
            trainer: None,
            hrm: None,
            app_ver: "test".into(),
        },
        samples: (0..RIDE_DURATION_S)
            .map(|second| Sample {
                t_ms: u64::from(second) * MS_PER_SECOND,
                power_w: Some(200),
                cadence_rpm: Some(90),
                heart_rate_bpm: None,
                target_power_w: (second < STEP_DURATION_S).then_some(275),
                target_cadence_rpm: (second < STEP_DURATION_S).then_some(95),
            })
            .collect(),
        events: vec![
            SessionEvent {
                t_ms: 0,
                kind: SessionEventKind::Start,
                segment_index: None,
            },
            SessionEvent {
                t_ms: u64::from(STEP_DURATION_S) * MS_PER_SECOND,
                kind: SessionEventKind::WorkoutSegmentEnd,
                segment_index: Some(0),
            },
            SessionEvent {
                t_ms: u64::from(RIDE_DURATION_S) * MS_PER_SECOND,
                kind: SessionEventKind::End,
                segment_index: None,
            },
        ],
    };
    let laps = compute_activity_segments(&recording);
    let totals = session_totals(&recording, FTP_W);
    for plan in [Some(&workout), None] {
        let bytes = encode_activity(&FitActivity {
            header: &recording.header,
            samples: &recording.samples,
            events: &recording.events,
            laps: &laps,
            totals: &totals,
            motion: None,
            workout: plan,
        })
        .unwrap();
        let decoded = decode_activity(&bytes).unwrap();
        assert_eq!(decoded.laps.len(), 2);
        assert_eq!(
            decoded.laps[1].workout_step_index, None,
            "ride ended inside a step"
        );
        if plan.is_some() {
            let instructions = decoded.instructions.unwrap();
            assert!(instructions.steps.iter().all(|step| step.supported));
            assert_eq!(instructions.steps.len(), 2);
            assert!(matches!(instructions.steps[0].target,
                FitInstructionTarget::PowerRange { low: 275.0, high: 275.0, ref unit } if unit == "W"));
            assert_eq!(instructions.steps[0].cadence_rpm, Some((95.0, 95.0)));
            assert!(matches!(
                instructions.steps[1].target,
                FitInstructionTarget::Open
            ));
            assert_eq!(instructions.steps[1].cadence_rpm, None);
            assert_eq!(decoded.laps[0].workout_step_index, Some(0));
        } else {
            assert!(
                decoded.instructions.is_none(),
                "laps do not imply instructions"
            );
            assert!(decoded
                .laps
                .iter()
                .all(|lap| lap.workout_step_index.is_none()));
        }
    }
}
