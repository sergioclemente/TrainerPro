use tp_core::journal::{compute_activity_segments, replay, SessionEventKind};

#[test]
fn workout_segment_boundaries_replay_as_activity_segments_with_skips_and_revisits() {
    // Segment 1 is skipped without riding; segment 0 is visited twice.
    let journal = concat!(
        "{\"h\":{\"workout_session_id\":\"ride\",\"workout_definition_id\":\"workout\",",
        "\"workout_definition_snapshot_json\":\"{}\",\"started_unix_ms\":1700000000000,",
        "\"workout_name\":\"Test\",\"ftp_w\":250,\"weight_kg\":75.0,\"app_ver\":\"0.1.0\"}}\n",
        "{\"e\":{\"t_ms\":0,\"kind\":\"start\"}}\n",
        "{\"s\":{\"t_ms\":0,\"power_w\":100}}\n",
        "{\"e\":{\"t_ms\":1000,\"kind\":\"workout_segment_end\",\"segment_index\":0}}\n",
        "{\"e\":{\"t_ms\":1000,\"kind\":\"workout_segment_end\",\"segment_index\":1}}\n",
        "{\"s\":{\"t_ms\":1000,\"power_w\":200}}\n",
        "{\"e\":{\"t_ms\":2000,\"kind\":\"workout_segment_end\",\"segment_index\":0}}\n",
        "{\"e\":{\"t_ms\":2000,\"kind\":\"end\"}}\n",
    );
    let recording = replay(journal.as_bytes()).unwrap();
    let boundaries: Vec<_> = recording
        .events
        .iter()
        .filter(|event| event.kind == SessionEventKind::WorkoutSegmentEnd)
        .collect();
    assert_eq!(boundaries.len(), 3);
    assert_eq!(boundaries[0].segment_index, boundaries[2].segment_index);
    let encoded = serde_json::to_value(boundaries[0]).unwrap();
    assert_eq!(encoded["kind"], "workout_segment_end");
    assert_eq!(encoded["segment_index"], 0);

    let segments = compute_activity_segments(&recording);
    assert_eq!(segments.len(), 2);
    assert_eq!((segments[0].start_ms, segments[0].end_ms), (0, 1000));
    assert_eq!((segments[1].start_ms, segments[1].end_ms), (1000, 2000));
    assert_eq!(segments[0].average_power_w, Some(100));
    assert_eq!(segments[1].average_power_w, Some(200));
    // Each activity segment links to the step whose end event closed it: the
    // first event at a boundary is the ridden step, later ones are skips.
    assert_eq!(segments[0].workout_segment_index, Some(0));
    assert_eq!(segments[1].workout_segment_index, Some(0), "revisited");
}

#[test]
fn activity_segments_summarize_work_normalized_power_and_cadence() {
    // 0–30 s: 200 W at 90 rpm; 30–40 s: 300 W, cadence 100 then absent.
    let mut journal = String::from(concat!(
        "{\"h\":{\"workout_session_id\":\"ride\",\"workout_definition_id\":\"workout\",",
        "\"workout_definition_snapshot_json\":\"{}\",\"started_unix_ms\":1700000000000,",
        "\"workout_name\":\"Test\",\"ftp_w\":250,\"weight_kg\":75.0,\"app_ver\":\"0.1.0\"}}\n",
        "{\"e\":{\"t_ms\":0,\"kind\":\"start\"}}\n",
    ));
    for t in 0..30u64 {
        journal.push_str(&format!(
            "{{\"s\":{{\"t_ms\":{},\"power_w\":200,\"cadence_rpm\":90}}}}\n",
            t * 1000
        ));
    }
    journal.push_str(
        "{\"e\":{\"t_ms\":30000,\"kind\":\"workout_segment_end\",\"segment_index\":0}}\n",
    );
    journal.push_str("{\"s\":{\"t_ms\":30000,\"power_w\":300,\"cadence_rpm\":100}}\n");
    for t in 31..40u64 {
        journal.push_str(&format!(
            "{{\"s\":{{\"t_ms\":{},\"power_w\":300}}}}\n",
            t * 1000
        ));
    }
    // Stopped inside step 1: no segment-end event closes the final segment.
    journal.push_str("{\"e\":{\"t_ms\":40000,\"kind\":\"end\"}}\n");

    let recording = replay(journal.as_bytes()).unwrap();
    let segments = compute_activity_segments(&recording);
    assert_eq!(segments.len(), 2);

    let first = segments[0];
    assert_eq!(first.workout_segment_index, Some(0));
    assert_eq!(first.work_j, 30 * 200, "Σ power × 1 s");
    assert_eq!(first.calories_kcal, 6, "kJ ≈ kcal");
    assert_eq!(first.normalized_power_w, Some(200), "steady 30 s window");
    assert_eq!(first.max_cadence_rpm, Some(90));

    let last = segments[1];
    assert_eq!(
        last.workout_segment_index, None,
        "ride ended inside the step"
    );
    assert_eq!(last.work_j, 10 * 300);
    assert_eq!(
        last.normalized_power_w,
        Some(300),
        "short segment: plain average"
    );
    assert_eq!(last.average_cadence_rpm, Some(100), "present samples only");
    assert_eq!(last.max_cadence_rpm, Some(100));
}
