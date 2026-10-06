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
}
