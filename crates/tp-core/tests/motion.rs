use std::collections::HashMap;
use tp_core::fit::{encode_activity, FitActivity};
use tp_core::journal::{
    compute_activity_segments, replay, JournalHeader, JournalWriter, Sample, SessionEvent,
    SessionEventKind as Event, SessionRecording,
};
use tp_core::metrics::session_totals;
use tp_core::motion::{replay_motion, FlatRoadMotion};

fn recording(samples: &[(u64, Option<u16>)], events: &[(u64, Event)]) -> SessionRecording {
    SessionRecording {
        header: JournalHeader {
            workout_session_id: "motion-test".into(),
            workout_definition_id: "definition-test".into(),
            scheduled_workout_id: None,
            workout_definition_snapshot_json: "{}".into(),
            started_unix_ms: 1_700_000_000_000,
            workout_name: "Motion".into(),
            ftp_w: 250,
            weight_kg: 75.0,
            record_distance: true,
            trainer: None,
            hrm: None,
            app_ver: "0.1.0".into(),
        },
        samples: samples
            .iter()
            .map(|&(t_ms, power_w)| Sample {
                t_ms,
                power_w,
                cadence_rpm: None,
                heart_rate_bpm: None,
                target_power_w: None,
                target_cadence_rpm: None,
            })
            .collect(),
        events: events
            .iter()
            .map(|&(t_ms, kind)| SessionEvent {
                t_ms,
                kind,
                segment_index: (kind == Event::WorkoutSegmentEnd).then_some(0),
            })
            .collect(),
    }
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

#[test]
fn accelerates_to_equilibrium_and_coasts() {
    let mut motion = FlatRoadMotion::new(75.0);
    assert_eq!(motion.point().speed_m_s, 0.0);
    let first = motion.advance(250, 1000);
    let cruising = motion.advance(250, 600_000);
    assert!(first.speed_m_s > 0.0 && first.speed_m_s < cruising.speed_m_s);
    // Independently evaluate the specified resistance law at equilibrium.
    let v = cruising.speed_m_s;
    let resistance = 0.5 * 1.225 * 0.32 * v.powi(3) + 0.005 * 84.0 * 9.81 * v;
    assert!((resistance - 250.0).abs() < 0.001);
    let coast = motion.advance(0, 1000);
    assert!(coast.speed_m_s > 0.0 && coast.speed_m_s < v);
    assert!(coast.distance_m > cruising.distance_m);
    let stopped = motion.advance(0, 600_000);
    assert!(stopped.speed_m_s < 0.001);
    motion.stop();
    assert_eq!(motion.point().speed_m_s, 0.0);
    close(motion.point().distance_m, stopped.distance_m);
}

#[test]
fn heavier_rider_accelerates_slower_and_extreme_power_remains_finite() {
    let light = FlatRoadMotion::new(50.0).advance(250, 1000);
    let heavy = FlatRoadMotion::new(100.0).advance(250, 1000);
    assert!(light.speed_m_s > heavy.speed_m_s);
    for power in [0, 1, 250, u16::MAX] {
        let mut motion = FlatRoadMotion::new(75.0);
        let point = motion.advance(power, 600_000);
        assert!(point.speed_m_s.is_finite() && point.speed_m_s >= 0.0);
        assert!(point.distance_m.is_finite() && point.distance_m >= 0.0);
    }
}

#[test]
fn zero_power_coasts_but_missing_power_discards_momentum() {
    let data = recording(
        &[
            (1000, Some(250)),
            (2000, Some(0)),
            (3000, None),
            (4000, Some(250)),
        ],
        &[(0, Event::Start), (4000, Event::End)],
    );
    let trace = replay_motion(&data, &compute_activity_segments(&data));
    assert!(trace.records[1].speed_m_s > 0.0);
    assert!(trace.records[1].distance_m > trace.records[0].distance_m);
    assert_eq!(trace.records[2].speed_m_s, 0.0);
    close(trace.records[2].distance_m, trace.records[1].distance_m);
    close(trace.records[3].speed_m_s, trace.records[0].speed_m_s);
}

#[test]
fn gap_and_catch_up_samples_do_not_fabricate_time() {
    let data = recording(
        &[
            (1000, Some(250)),
            (5000, Some(250)),
            (5000, Some(250)),
            (5010, Some(250)),
        ],
        &[(0, Event::Start), (5010, Event::End)],
    );
    let trace = replay_motion(&data, &compute_activity_segments(&data));
    close(trace.records[1].speed_m_s, trace.records[0].speed_m_s);
    close(
        trace.records[1].distance_m,
        2.0 * trace.records[0].distance_m,
    );
    close(trace.records[2].distance_m, trace.records[1].distance_m);
    assert!(trace.records[3].distance_m - trace.records[2].distance_m < 0.1);
}

#[test]
fn short_pause_freezes_motion_and_resume_clips_first_sample() {
    // Manual and fault pauses share exactly this journal contract. Include
    // samples tied to both event boundaries to exercise ordering.
    let data = recording(
        &[
            (1000, Some(250)),
            (1100, Some(250)),
            (1200, Some(250)),
            (1300, Some(250)),
        ],
        &[
            (0, Event::Start),
            (1100, Event::Pause),
            (1200, Event::Resume),
            (1350, Event::End),
        ],
    );
    let trace = replay_motion(&data, &compute_activity_segments(&data));
    assert!(trace.records[1].speed_m_s > trace.records[0].speed_m_s);
    close(trace.records[2].speed_m_s, trace.records[1].speed_m_s);
    close(trace.records[2].distance_m, trace.records[1].distance_m);
    let expected = FlatRoadMotion::new(75.0).advance(250, 1200);
    close(trace.records[3].speed_m_s, expected.speed_m_s);
    close(trace.records[3].distance_m, expected.distance_m);
}

#[test]
fn long_pause_preserves_speed_and_resume_can_coast() {
    let data = recording(
        &[(1000, Some(250)), (600_000, Some(0)), (601_000, Some(0))],
        &[
            (0, Event::Start),
            (1000, Event::Pause),
            (600_000, Event::Resume),
            (601_000, Event::End),
        ],
    );
    let activity_segments = compute_activity_segments(&data);
    let trace = replay_motion(&data, &activity_segments);
    close(trace.records[1].speed_m_s, trace.records[0].speed_m_s);
    close(trace.records[1].distance_m, trace.records[0].distance_m);
    let mut expected = FlatRoadMotion::new(75.0);
    expected.advance(250, 1000);
    let coast = expected.advance(0, 1000);
    close(trace.records[2].speed_m_s, coast.speed_m_s);
    close(trace.session.distance_m, coast.distance_m);
    assert!(trace.records[2].speed_m_s < trace.records[1].speed_m_s);
    assert!(trace.records[2].distance_m > trace.records[1].distance_m);
    let totals = session_totals(&data, data.header.ftp_w);
    let bytes = encode_activity(&FitActivity {
        header: &data.header,
        samples: &data.samples,
        events: &data.events,
        laps: &activity_segments,
        totals: &totals,
        motion: Some(&trace),
        workout: None,
    })
    .unwrap();
    let messages = decode(&bytes);
    let records: Vec<_> = messages
        .iter()
        .filter(|(id, _)| *id == 20)
        .map(|(_, fields)| fields)
        .collect();
    assert_eq!(records[0][&6], records[1][&6]);
    assert_eq!(records[0][&5], records[1][&5]);
    let session = &messages.iter().find(|(id, _)| *id == 18).unwrap().1;
    assert_eq!(session[&8], 2000);
}

#[test]
fn pauses_do_not_hide_missing_active_time_before_or_after_resume() {
    for (pause_ms, sample_ms) in [(2500, 10_500), (1000, 12_000)] {
        let data = recording(
            &[(1000, Some(250)), (sample_ms, Some(250))],
            &[
                (0, Event::Start),
                (pause_ms, Event::Pause),
                (10_000, Event::Resume),
                (sample_ms, Event::End),
            ],
        );
        let trace = replay_motion(&data, &compute_activity_segments(&data));
        let credited_ms = (sample_ms - 10_000).min(1000);
        let expected = FlatRoadMotion::new(75.0).advance(250, credited_ms);
        close(trace.records[1].speed_m_s, expected.speed_m_s);
        close(
            trace.records[1].distance_m - trace.records[0].distance_m,
            expected.distance_m,
        );
    }
}

#[test]
fn repeated_pauses_without_samples_preserve_momentum_but_missing_power_resets_it() {
    for resumed_power in [Some(250), None] {
        let data = recording(
            &[(1000, Some(250)), (20_000, resumed_power)],
            &[
                (0, Event::Start),
                (1000, Event::Pause),
                (10_000, Event::Resume),
                (10_100, Event::Pause),
                (20_000, Event::Resume),
                (20_100, Event::End),
            ],
        );
        let trace = replay_motion(&data, &compute_activity_segments(&data));
        let expected_speed = if resumed_power.is_some() {
            trace.records[0].speed_m_s
        } else {
            0.0
        };
        close(trace.records[1].speed_m_s, expected_speed);
        close(trace.records[1].distance_m, trace.records[0].distance_m);
    }
}

#[test]
fn activity_segment_boundaries_split_distance_without_resetting_motion() {
    let samples: Vec<_> = (1..=20).map(|i| (i * 1000, Some(250))).collect();
    let mut data = recording(&samples, &[(0, Event::Start), (20_000, Event::End)]);
    let whole = replay_motion(&data, &compute_activity_segments(&data));
    data.events.insert(
        1,
        SessionEvent {
            t_ms: 19_900,
            kind: Event::WorkoutSegmentEnd,
            segment_index: Some(0),
        },
    );
    let activity_segments = compute_activity_segments(&data);
    let split = replay_motion(&data, &activity_segments);
    close(split.session.distance_m, whole.session.distance_m);
    close(
        split.records.last().unwrap().speed_m_s,
        whole.records.last().unwrap().speed_m_s,
    );
    close(
        split
            .activity_segments
            .iter()
            .map(|segment| segment.distance_m)
            .sum(),
        split.session.distance_m,
    );
    for (summary, segment) in split.activity_segments.iter().zip(&activity_segments) {
        assert!(summary.average_speed_m_s(segment.timer_ms).unwrap() <= summary.max_speed_m_s);
    }
    assert!(split.activity_segments[1].distance_m > 0.0);
}

#[test]
fn start_end_and_unpaired_pause_do_not_extrapolate() {
    let data = recording(
        &[(100, Some(250)), (200, Some(250))],
        &[(0, Event::Start), (250, Event::Pause), (10_000, Event::End)],
    );
    let activity_segments = compute_activity_segments(&data);
    let trace = replay_motion(&data, &activity_segments);
    let expected = FlatRoadMotion::new(75.0).advance(250, 200);
    close(trace.session.distance_m, expected.distance_m);
    assert_eq!(activity_segments[0].timer_ms, 250);
    assert!(
        trace
            .session
            .average_speed_m_s(activity_segments[0].timer_ms)
            .unwrap()
            <= trace.session.max_speed_m_s
    );
    assert_eq!(trace.session.average_speed_m_s(0), None);
}

#[test]
fn header_snapshots_enablement_and_legacy_journals_default_disabled() {
    let mut data = recording(&[], &[]);
    for enabled in [true, false] {
        data.header.record_distance = enabled;
        let journal = JournalWriter::new(Vec::new(), &data.header)
            .unwrap()
            .into_inner();
        let replayed = replay(journal.as_slice()).unwrap();
        assert_eq!(replayed.header.record_distance, enabled);
    }
    let mut header = serde_json::to_value(&data.header).unwrap();
    header.as_object_mut().unwrap().remove("record_distance");
    let legacy = format!("{{\"h\":{header}}}\n");
    assert!(!replay(legacy.as_bytes()).unwrap().header.record_distance);
}

// Independent decoder for the public FIT output. Field identifiers below are
// the FIT profile's wire contract, intentionally not imported from the encoder.
fn decode(bytes: &[u8]) -> Vec<(u16, HashMap<u8, u64>)> {
    assert_eq!(tp_core::fit::checksum(bytes), 0);
    let mut definitions = HashMap::new();
    let mut messages = Vec::new();
    let mut cursor = bytes[0] as usize;
    while cursor < bytes.len() - 2 {
        let header = bytes[cursor];
        cursor += 1;
        let local = header & 0x0f;
        if header & 0x40 != 0 {
            let global = u16::from_le_bytes([bytes[cursor + 2], bytes[cursor + 3]]);
            let count = bytes[cursor + 4] as usize;
            cursor += 5;
            let mut fields = Vec::new();
            for _ in 0..count {
                fields.push((bytes[cursor], bytes[cursor + 1] as usize));
                cursor += 3;
            }
            definitions.insert(local, (global, fields));
        } else {
            let (global, fields) = &definitions[&local];
            let mut values = HashMap::new();
            for &(field, size) in fields {
                if size <= 8 {
                    let value = bytes[cursor..cursor + size]
                        .iter()
                        .rev()
                        .fold(0u64, |n, b| (n << 8) | u64::from(*b));
                    values.insert(field, value);
                }
                cursor += size;
            }
            messages.push((*global, values));
        }
    }
    messages
}

#[test]
fn fit_records_laps_and_session_share_distance_and_precise_timer_time() {
    let samples: Vec<_> = (0..=60)
        .map(|i| (i * 1000, Some(if i < 30 { 250 } else { 0 })))
        .collect();
    let mut events = vec![(0, Event::Start)];
    events.extend((1..60).map(|i| (i * 1000 + 123, Event::WorkoutSegmentEnd)));
    events.push((60_250, Event::End));
    let data = recording(&samples, &events);
    let activity_segments = compute_activity_segments(&data);
    let trace = replay_motion(&data, &activity_segments);
    let totals = session_totals(&data, data.header.ftp_w);
    for motion in [Some(&trace), None] {
        let bytes = encode_activity(&FitActivity {
            header: &data.header,
            samples: &data.samples,
            events: &data.events,
            laps: &activity_segments,
            totals: &totals,
            motion,
            workout: None,
        })
        .unwrap();
        let messages = decode(&bytes);
        let records: Vec<_> = messages
            .iter()
            .filter(|(id, _)| *id == 20)
            .map(|(_, v)| v)
            .collect();
        let fit_laps: Vec<_> = messages
            .iter()
            .filter(|(id, _)| *id == 19)
            .map(|(_, v)| v)
            .collect();
        let session = &messages.iter().find(|(id, _)| *id == 18).unwrap().1;
        assert_eq!(session[&8], 60_250);
        if motion.is_some() {
            assert!(records.windows(2).all(|r| r[0][&5] <= r[1][&5]));
            assert_eq!(session[&9], records.last().unwrap()[&5]);
            assert_eq!(
                session[&9],
                fit_laps.iter().map(|segment| segment[&9]).sum::<u64>()
            );
            assert_eq!(
                session[&15],
                fit_laps.iter().map(|segment| segment[&14]).max().unwrap()
            );
            let expected_avg = (trace.session.distance_m / 60.25 * 1000.0).round() as u64;
            assert_eq!(session[&14], expected_avg);
            for segment in &fit_laps {
                assert!(segment[&13] <= segment[&14]);
            }
        } else {
            for record in records {
                assert!(!record.contains_key(&5) && !record.contains_key(&6));
            }
            for segment in fit_laps {
                assert!(
                    !segment.contains_key(&9)
                        && !segment.contains_key(&13)
                        && !segment.contains_key(&14)
                );
            }
            assert!(
                !session.contains_key(&9)
                    && !session.contains_key(&14)
                    && !session.contains_key(&15)
            );
        }
    }
}

#[test]
fn truncated_journal_ends_at_last_sample_without_extrapolation() {
    let data = recording(
        &[(1000, Some(250)), (2000, Some(250))],
        &[(0, Event::Start)],
    );
    let mut writer = JournalWriter::new(Vec::new(), &data.header).unwrap();
    writer.write_event(&data.events[0]).unwrap();
    for sample in &data.samples {
        writer.write_sample(sample).unwrap();
    }
    let mut bytes = writer.into_inner();
    bytes.extend_from_slice(b"{\"s\":{\"t_ms\":3000");
    let recovered = replay(bytes.as_slice()).unwrap();
    let activity_segments = compute_activity_segments(&recovered);
    let trace = replay_motion(&recovered, &activity_segments);
    assert_eq!(activity_segments.last().unwrap().end_ms, 2000);
    close(
        trace.session.distance_m,
        FlatRoadMotion::new(75.0).advance(250, 2000).distance_m,
    );
}

#[test]
fn empty_fit_has_invalid_average_speed_and_mismatched_trace_is_rejected() {
    let data = recording(&[], &[(0, Event::Start), (0, Event::End)]);
    let activity_segments = compute_activity_segments(&data);
    let trace = replay_motion(&data, &activity_segments);
    let totals = session_totals(&data, data.header.ftp_w);
    let mut activity = FitActivity {
        header: &data.header,
        samples: &data.samples,
        events: &data.events,
        laps: &activity_segments,
        totals: &totals,
        motion: Some(&trace),
        workout: None,
    };
    let messages = decode(&encode_activity(&activity).unwrap());
    let session = &messages.iter().find(|(id, _)| *id == 18).unwrap().1;
    assert_eq!(session[&9], 0);
    assert_eq!(session[&14], u16::MAX as u64);
    let other = recording(&[(1000, Some(250))], &[(0, Event::Start)]);
    activity.samples = &other.samples;
    assert!(encode_activity(&activity).is_err());
}
