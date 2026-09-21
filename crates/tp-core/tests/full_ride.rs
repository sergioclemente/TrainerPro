//! Cross-module integration: parse ZWO → engine ride → journal → metrics →
//! FIT encode. No hardware, no wall clock — ticks are scripted.

use std::io::Cursor;

use tp_core::engine::{Effect, Engine, Input, Phase};
use tp_core::fit::{encode_activity, FitActivity};
use tp_core::journal::{
    compute_laps, replay, JournalHeader, JournalWriter, Sample, SessionEvent, SessionEventKind,
};
use tp_core::metrics::session_totals;
use tp_core::parse::parse_zwo;

const ZWO: &str = r#"<workout_file>
  <name>Integration Ride</name>
  <description>2 min test</description>
  <sportType>bike</sportType>
  <workout>
    <Warmup Duration="30" PowerLow="0.4" PowerHigh="0.6"/>
    <IntervalsT Repeat="2" OnDuration="20" OffDuration="10" OnPower="1.0" OffPower="0.5"/>
    <SteadyState Duration="30" Power="0.75">
      <textevent timeoffset="5" message="hold steady"/>
    </SteadyState>
  </workout>
</workout_file>"#;

const FTP: u16 = 250;
const START_UNIX_MS: u64 = 1_750_000_000_000;

#[test]
fn zwo_to_fit_end_to_end() {
    // Parse: 30s warmup + 2×(20s on + 10s off) + 30s steady = 120 s, 6 segments.
    let parsed = parse_zwo(ZWO).expect("zwo parses");
    assert!(parsed.warnings.is_empty(), "unexpected: {:?}", parsed.warnings);
    let workout = parsed.workout;
    assert_eq!(workout.segments.len(), 6);
    assert_eq!(workout.duration_s(), 120);

    // Ride it: 250 ms ticks, journal samples at 1 Hz, lap events from effects.
    let header = JournalHeader {
        workout_session_id: "session-itest-1".into(),
        workout_definition_id: "definition-itest-1".into(),
        scheduled_workout_id: None,
        workout_definition_snapshot_json: r#"{"format":"TPW","version":1}"#.into(),
        started_unix_ms: START_UNIX_MS,
        workout_name: workout.name.clone(),
        ftp_w: FTP,
        weight_kg: 72.0,
        trainer: Some("SimTrainer".into()),
        hrm: None,
        app_ver: "0.1.0".into(),
    };
    let mut journal = JournalWriter::new(Vec::new(), &header).unwrap();
    let mut engine = Engine::new(workout, FTP, 1.0);
    let mut t_ms: u64 = 0;
    let mut current_target: Option<u16> = None;
    let mut texts_shown = 0u32;

    journal
        .write_event(&SessionEvent {
            t_ms,
            kind: SessionEventKind::Start,
            segment_index: None,
        })
        .unwrap();
    for eff in engine.handle(Input::Start) {
        if let Effect::SetTarget(w) = eff {
            current_target = Some(w);
        }
    }

    while engine.phase() == Phase::Riding {
        t_ms += 250;
        for eff in engine.handle(Input::Tick { dt_ms: 250 }) {
            match eff {
                Effect::SetTarget(w) => current_target = Some(w),
                Effect::LapBoundary { seg_idx } => journal
                    .write_event(&SessionEvent {
                        t_ms,
                        kind: SessionEventKind::Lap,
                        segment_index: Some(seg_idx),
                    })
                    .unwrap(),
                Effect::ShowText(_) => texts_shown += 1,
                _ => {}
            }
        }
        if t_ms.is_multiple_of(1000) && engine.phase() == Phase::Riding {
            // "Rider" holds exactly the target; fixed cadence/HR.
            journal
                .write_sample(&Sample {
                    t_ms,
                    power_w: current_target,
                    cadence_rpm: Some(90),
                    heart_rate_bpm: Some(140),
                    target_power_w: current_target,
                    target_cadence_rpm: None,
                })
                .unwrap();
        }
    }
    assert_eq!(engine.phase(), Phase::Finished);
    journal
        .write_event(&SessionEvent {
            t_ms,
            kind: SessionEventKind::End,
            segment_index: None,
        })
        .unwrap();
    assert_eq!(texts_shown, 1, "one textevent should have fired");

    // Replay journal, compute laps + totals.
    let bytes = journal.into_inner();
    let data = replay(Cursor::new(bytes)).expect("journal replays");
    let laps = compute_laps(&data);
    // 6 segments; the final LapBoundary lands at ride end, so no extra
    // partial lap beyond it.
    assert_eq!(laps.len(), 6, "one lap per segment, laps: {laps:?}");
    let totals = session_totals(&data, FTP);
    assert_eq!(totals.elapsed_s, 120);
    assert_eq!(totals.timer_s, 120, "no pauses in this ride");
    let average_power_w = totals.average_power_w.expect("has power");
    // Rough energy check: targets range 100..250 W, so avg must be inside.
    assert!(
        (100..=250).contains(&average_power_w),
        "average_power_w={average_power_w}"
    );
    assert!(
        totals.normalized_power_w.is_some() && totals.training_stress_score.is_some()
    );

    // Encode FIT and structurally validate container + trailing CRC.
    let fit = encode_activity(&FitActivity {
        header: &data.header,
        samples: &data.samples,
        events: &data.events,
        laps: &laps,
        totals: &totals,
        record_distance: false,
    })
    .expect("fit encodes");
    assert_eq!(fit[0], 14, "header size");
    assert_eq!(&fit[8..12], b".FIT");
    let data_size = u32::from_le_bytes(fit[4..8].try_into().unwrap()) as usize;
    assert_eq!(fit.len(), 14 + data_size + 2, "header + data + trailing CRC");
    // FIT property: CRC over (everything incl. stored CRC) == 0 is equivalent
    // to checking the stored value; recompute explicitly instead.
    let stored = u16::from_le_bytes(fit[fit.len() - 2..].try_into().unwrap());
    let computed = tp_core::fit::checksum(&fit[..fit.len() - 2]);
    assert_eq!(stored, computed, "trailing CRC validates");
}
