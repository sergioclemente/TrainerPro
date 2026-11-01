use tp_core::fit::{checksum, decode_activity, FitDecodeError, FitSport, TimerState};

const CYCLING: &[u8] = include_bytes!("../../../testdata/fit/cycling.fit");
const RUNNING: &[u8] = include_bytes!("../../../testdata/fit/running.fit");

#[test]
fn sdk_activity_preserves_units_invalid_values_zero_and_explicit_enhanced_speed() {
    let activity = decode_activity(CYCLING).unwrap();
    assert_eq!(activity.sport, FitSport::Cycling);
    assert_eq!(activity.start_unix_seconds, 1767225600);
    assert_eq!(activity.duration_seconds, 10.0);
    assert_eq!(activity.summary.timer_seconds, Some(8.0));
    assert_eq!(activity.summary.distance_meters, Some(25.0));
    assert_eq!(activity.records.len(), 6);
    assert_eq!(activity.records[0].power_watts, Some(0.0));
    assert_eq!(activity.records[2].power_watts, None);
    assert_eq!(activity.records[2].cadence, None);
    assert_eq!(activity.records[0].cadence, Some(90.5));
    assert_eq!(activity.records[0].speed_meters_per_second, Some(3.123));
    assert_eq!(activity.laps[0].start_seconds, Some(0.0));
    assert_eq!(activity.laps[0].end_seconds, Some(10.0));
    assert!(activity.laps[0].usable_window_bounds);
    assert_eq!(activity.timer_events[1].state, TimerState::Paused);
    assert_eq!(activity.timer_events[2].elapsed_seconds, 6.0);
    assert_eq!(activity.timer_events[2].state, TimerState::Active);
}

#[test]
fn running_cadence_is_both_feet_steps_per_minute() {
    let activity = decode_activity(RUNNING).unwrap();
    assert_eq!(activity.sport, FitSport::Running);
    assert_eq!(activity.records[0].cadence, Some(181.0));
    assert!(activity.records.iter().all(|r| r.power_watts.is_none()));
}

#[test]
fn corrupt_truncated_chained_and_oversized_files_are_rejected() {
    let mut corrupt = CYCLING.to_vec();
    corrupt[20] ^= 1;
    assert_eq!(
        decode_activity(&corrupt).unwrap_err().code(),
        "fit_invalid_encoding"
    );
    for length in [0, 1, 10, 12, CYCLING.len() - 2, CYCLING.len() - 1] {
        assert!(decode_activity(&CYCLING[..length]).is_err());
    }
    let chained = [CYCLING, CYCLING].concat();
    assert!(decode_activity(&chained).is_err());
    assert!(matches!(
        decode_activity(&vec![0; 32 * 1024 * 1024 + 1]),
        Err(FitDecodeError::LimitExceeded)
    ));
}

/// Append a validly framed message to an SDK fixture, then rebuild both CRCs.
fn append(bytes: &[u8], message: &[u8]) -> Vec<u8> {
    let mut result = bytes[..bytes.len() - 2].to_vec();
    result.extend(message);
    let size = (result.len() - 14) as u32;
    result[4..8].copy_from_slice(&size.to_le_bytes());
    let crc = checksum(&result[..12]);
    result[12..14].copy_from_slice(&crc.to_le_bytes());
    let crc = checksum(&result);
    result.extend(crc.to_le_bytes());
    result
}

#[test]
fn duplicate_or_out_of_session_records_are_rejected_even_with_valid_crc() {
    // Local message 15: record with timestamp only, uint32 little endian.
    let definition = [0x4f, 0, 0, 20, 0, 1, 253, 4, 0x86];
    let fit_start: u32 = 1767225600 - 631065600;
    for (offset, code) in [
        (0, "fit_record_timestamp_backwards"),
        (10, "fit_record_timestamp_repeated"),
        (13, "fit_record_outside_session"),
    ] {
        let message = [
            definition.as_slice(),
            &[15],
            &(fit_start + offset).to_le_bytes(),
        ]
        .concat();
        assert_eq!(
            decode_activity(&append(CYCLING, &message))
                .unwrap_err()
                .code(),
            code
        );
    }
}

#[test]
fn timestamp_precision_tolerance_preserves_fractional_session_end() {
    let definition = [0x4f, 0, 0, 20, 0, 1, 253, 4, 0x86];
    let timestamp: u32 = 1767225600 - 631065600 + 11;
    let message = [definition.as_slice(), &[15], &timestamp.to_le_bytes()].concat();
    let activity = decode_activity(&append(CYCLING, &message)).unwrap();
    assert_eq!(activity.duration_seconds, 10.0);
    assert_eq!(activity.records.last().unwrap().elapsed_seconds, 11.0);
}

/// Independent wire fixture with one session, timestamped power records and timer events.
fn timing_fixture(duration_ms: u32, records: &[(u32, u16)], stop_offset: u32) -> Vec<u8> {
    const FIT_START: u32 = 1_136_160_000;
    let mut data = vec![0x40, 0, 0, 0, 0, 1, 0, 1, 0, 0, 4]; // file_id: activity
                                                             // session: start_time, total_elapsed_time, sport
    data.extend([0x41, 0, 0, 18, 0, 3, 2, 4, 0x86, 7, 4, 0x86, 5, 1, 0, 1]);
    data.extend(FIT_START.to_le_bytes());
    data.extend(duration_ms.to_le_bytes());
    data.push(2); // cycling
                  // record: timestamp, power
    data.extend([0x42, 0, 0, 20, 0, 2, 253, 4, 0x86, 7, 2, 0x84]);
    for (offset, power) in records {
        data.push(2);
        data.extend(FIT_START.saturating_add(*offset).to_le_bytes());
        data.extend(power.to_le_bytes());
    }
    // timer stop_all event: timestamp, event, event_type
    data.extend([0x43, 0, 0, 21, 0, 3, 253, 4, 0x86, 0, 1, 0, 1, 1, 0, 3]);
    data.extend((FIT_START + stop_offset).to_le_bytes());
    data.extend([0, 4]);
    let mut bytes = vec![12, 0x20, 0, 0];
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(b".FIT");
    bytes.extend(data);
    bytes.extend(checksum(&bytes).to_le_bytes());
    bytes
}

#[test]
fn terminal_record_and_stop_excesses_up_to_two_seconds_preserve_declared_duration() {
    for (duration_ms, stop) in [(1_270_326, 1272), (2_098_697, 2100), (3_076_194, 3078)] {
        let bytes = timing_fixture(duration_ms, &[(0, 60), (stop, 100)], stop);
        let activity = decode_activity(&bytes).unwrap();
        let duration = f64::from(duration_ms) / 1000.0;
        assert_eq!(activity.duration_seconds, duration);
        assert_eq!(activity.records[1].elapsed_seconds, f64::from(stop));
        assert_eq!(activity.timer_events[0].elapsed_seconds, f64::from(stop));
        assert_eq!(
            activity.terminal_timestamp_excess_seconds(),
            f64::from(stop) - duration
        );
    }
    let bytes = timing_fixture(10_326, &[(0, 60), (11, 100)], 11);
    let activity = decode_activity(&bytes).unwrap();
    assert_eq!(activity.duration_seconds, 10.326);
    assert_eq!(activity.records[1].elapsed_seconds, 11.0);
    assert_eq!(activity.timer_events[0].elapsed_seconds, 11.0);
}

#[test]
fn terminal_tolerance_is_inclusive_and_larger_record_or_timer_excesses_still_fail() {
    let accepted = timing_fixture(10_000, &[(0, 60), (12, 100)], 12);
    assert_eq!(
        decode_activity(&accepted)
            .unwrap()
            .terminal_timestamp_excess_seconds(),
        2.0
    );
    // Just one millisecond beyond the allowance must fail, independently for records/events.
    let record = timing_fixture(9_999, &[(0, 60), (12, 100)], 9);
    assert_eq!(
        decode_activity(&record).unwrap_err().code(),
        "fit_record_outside_session"
    );
    let timer = timing_fixture(9_999, &[(0, 60), (1, 100)], 12);
    assert_eq!(
        decode_activity(&timer).unwrap_err().code(),
        "fit_timer_outside_session"
    );
}

#[test]
fn same_second_conflicting_or_identical_records_are_not_silently_selected() {
    for power in [60, 100] {
        let bytes = timing_fixture(10_000, &[(0, 60), (0, power), (1, 100)], 10);
        assert_eq!(
            decode_activity(&bytes).unwrap_err().code(),
            "fit_record_timestamp_repeated"
        );
    }
    let bytes = timing_fixture(10_000, &[(1, 60), (0, 100)], 10);
    assert_eq!(
        decode_activity(&bytes).unwrap_err().code(),
        "fit_record_timestamp_backwards"
    );
    let bytes = timing_fixture(10_000, &[(u32::MAX, 60)], 10);
    assert_eq!(
        decode_activity(&bytes).unwrap_err().code(),
        "fit_missing_timestamp"
    );
    let bytes = timing_fixture(0, &[(0, 60)], 0);
    assert_eq!(
        decode_activity(&bytes).unwrap_err().code(),
        "fit_invalid_duration"
    );
}

#[test]
fn more_than_one_session_is_rejected() {
    // Additional empty session is still a second session.
    let message = [0x4f, 0, 0, 18, 0, 0, 15];
    assert!(matches!(
        decode_activity(&append(CYCLING, &message)),
        Err(FitDecodeError::UnsupportedActivity)
    ));
}

/// Change only the file envelope; record definitions and data come from the SDK.
fn with_header_size(size: u8) -> Vec<u8> {
    const BASE_HEADER_SIZE: usize = 12;
    const STANDARD_HEADER_SIZE: usize = 14;
    const CRC_SIZE: usize = 2;
    let mut bytes = CYCLING[..BASE_HEADER_SIZE].to_vec();
    bytes[0] = size;
    if usize::from(size) >= STANDARD_HEADER_SIZE {
        let crc = checksum(&bytes);
        bytes.extend_from_slice(&crc.to_le_bytes());
    }
    // Nonzero opaque extension bytes must neither become records nor evade CRC.
    bytes.resize(usize::from(size), 0xa5);
    bytes.extend_from_slice(&CYCLING[STANDARD_HEADER_SIZE..CYCLING.len() - CRC_SIZE]);
    let crc = checksum(&bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes
}

#[test]
fn variable_length_headers_preserve_the_decoded_activity() {
    let expected = serde_json::to_value(decode_activity(CYCLING).unwrap()).unwrap();
    for size in [12, 13, 14, 16, 64, 255] {
        let bytes = with_header_size(size);
        let actual = decode_activity(&bytes).unwrap_or_else(|e| panic!("header {size}: {e}"));
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            expected,
            "header {size}"
        );
    }
}

#[test]
fn extended_header_corruption_and_invalid_lengths_are_rejected() {
    const EXTENDED_HEADER_SIZE: usize = 16;
    let valid = with_header_size(EXTENDED_HEADER_SIZE as u8);
    for offset in [1, 12, 14, EXTENDED_HEADER_SIZE + 20, valid.len() - 1] {
        let mut corrupt = valid.clone();
        corrupt[offset] ^= 1;
        assert!(matches!(
            decode_activity(&corrupt),
            Err(FitDecodeError::InvalidEncoding)
        ));
    }
    for length in [12, 14, 15, 16, valid.len() - 1] {
        assert!(decode_activity(&valid[..length]).is_err());
    }
    for declared_size in [0, 11, 15, 17, 255] {
        let mut corrupt = valid.clone();
        corrupt[0] = declared_size;
        // A correct checksum must not mask an inconsistent declared length.
        let end = corrupt.len() - 2;
        let crc = checksum(&corrupt[..end]);
        corrupt[end..].copy_from_slice(&crc.to_le_bytes());
        assert!(decode_activity(&corrupt).is_err());
    }
    assert!(decode_activity(&[valid.as_slice(), valid.as_slice()].concat()).is_err());
}

#[test]
fn standard_header_crc_validation_and_optional_zero_are_preserved() {
    const HEADER_CRC_OFFSET: usize = 12;
    for stored_crc in [0_u16, 1_u16] {
        let mut bytes = CYCLING.to_vec();
        bytes[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 2].copy_from_slice(&stored_crc.to_le_bytes());
        let end = bytes.len() - 2;
        let crc = checksum(&bytes[..end]);
        bytes[end..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(decode_activity(&bytes).is_ok(), stored_crc == 0);
    }
}

#[test]
fn explicit_workout_instructions_and_execution_settings_are_not_inferred_from_laps() {
    // Independent standard message definitions: workout, workout_step, zones_target.
    let workout = [
        0x4f, 0, 0, 26, 0, 2, 6, 2, 0x84, 8, 8, 7, 15, 1, 0, b'T', b'e', b's', b't', 0, 0, 0, 0,
    ];
    let mut bytes = append(CYCLING, &workout);
    let definition = [
        0x4f, 0, 0, 27, 0, 7, 254, 2, 0x84, 1, 1, 0, 2, 4, 0x86, 3, 1, 0, 4, 4, 0x86, 5, 4, 0x86,
        6, 4, 0x86,
    ];
    let step = [
        &[15, 0, 0, 0][..],
        &60_000_u32.to_le_bytes(),
        &[4][..],
        &0_u32.to_le_bytes(),
        &90_u32.to_le_bytes(),
        &100_u32.to_le_bytes(),
    ]
    .concat();
    bytes = append(&bytes, &[definition.as_slice(), &step].concat());
    let zones = [0x4f, 0, 0, 7, 0, 1, 3, 2, 0x84, 15, 250, 0];
    bytes = append(&bytes, &zones);
    let activity = decode_activity(&bytes).unwrap();
    assert_eq!(activity.execution_ftp_watts, Some(250.0));
    let instructions = activity.instructions.unwrap();
    assert_eq!(instructions.name.as_deref(), Some("Test"));
    assert!(instructions.steps[0].supported);
    assert_eq!(instructions.steps[0].duration_seconds, Some(60.0));
    match &instructions.steps[0].target {
        tp_core::fit::FitInstructionTarget::PowerRange { low, high, unit } => {
            assert_eq!((*low, *high), (90.0, 100.0));
            assert_eq!(unit, "%FTP");
        }
        _ => panic!("explicit power range missing"),
    }
    assert!(activity.laps[0].workout_step_index.is_none());
    assert!(decode_activity(CYCLING).unwrap().instructions.is_none());
}

#[test]
fn workout_steps_must_follow_the_declared_contiguous_index_sequence() {
    // Independent wire definitions: a workout declaring two timed power steps.
    let workout = [0x4f, 0, 0, 26, 0, 1, 6, 2, 0x84, 15, 2, 0];
    let definition = [
        0x4f, 0, 0, 27, 0, 7, 254, 2, 0x84, 1, 1, 0, 2, 4, 0x86, 3, 1, 0, 4, 4, 0x86, 5, 4, 0x86,
        6, 4, 0x86,
    ];
    const STEP_DURATION_MS: u32 = 60_000;
    const TARGET_PERCENT_FTP: u32 = 90;
    for (indices, limited) in [
        (vec![0_u16, 1], false),
        (vec![0, 2], true),
        (vec![1, 0], true),
        (vec![0, 0], true),
        (vec![1, 2], true),
        (vec![0], true),
        (vec![0, 1, 2], true),
        (vec![0, u16::MAX], true),
    ] {
        let mut bytes = append(CYCLING, &workout);
        bytes = append(&bytes, &definition);
        for index in &indices {
            let step = [
                &[15][..],
                &index.to_le_bytes(),
                &[0][..],
                &STEP_DURATION_MS.to_le_bytes(),
                &[4][..],
                &0_u32.to_le_bytes(),
                &TARGET_PERCENT_FTP.to_le_bytes(),
                &TARGET_PERCENT_FTP.to_le_bytes(),
            ]
            .concat();
            bytes = append(&bytes, &step);
        }
        let instructions = decode_activity(&bytes).unwrap().instructions.unwrap();
        assert_eq!(instructions.declared_step_count, Some(2));
        assert_eq!(instructions.steps.len(), indices.len());
        assert_eq!(
            instructions.limitations.len() > 1,
            limited,
            "indices {indices:?}"
        );
    }
}

/// Independent wire definition for one explicit workout step. Absent fields really
/// are absent; u32::MAX exercises FIT invalid sentinels rather than numeric zero.
fn instruction_fixture(
    source: &[u8],
    duration_type: u8,
    target_type: Option<u8>,
    target_value: Option<u32>,
    low: Option<u32>,
    high: Option<u32>,
) -> Vec<u8> {
    let workout = [0x4f, 0, 0, 26, 0, 1, 6, 2, 0x84, 15, 1, 0];
    let mut fields = vec![
        (254, 0x84, 0_u16.to_le_bytes().to_vec()),
        (1, 0, vec![duration_type]),
        (2, 0x86, 60_000_u32.to_le_bytes().to_vec()),
    ];
    if let Some(kind) = target_type {
        fields.push((3, 0, vec![kind]));
    }
    for (field, value) in [(4, target_value), (5, low), (6, high)] {
        if let Some(value) = value {
            fields.push((field, 0x86, value.to_le_bytes().to_vec()));
        }
    }
    let mut wire = vec![0x4f, 0, 0, 27, 0, fields.len() as u8];
    for (field, base_type, bytes) in &fields {
        wire.extend([*field, bytes.len() as u8, *base_type]);
    }
    wire.push(15);
    for (_, _, bytes) in fields {
        wire.extend(bytes);
    }
    append(&append(source, &workout), &wire)
}

#[test]
fn running_custom_speed_targets_become_ordered_pace_without_rounding() {
    for (low, high) in [(2500, 4000), (3704, 3704)] {
        let bytes = instruction_fixture(RUNNING, 0, Some(0), Some(0), Some(low), Some(high));
        let instructions = decode_activity(&bytes).unwrap().instructions.unwrap();
        let step = &instructions.steps[0];
        assert!(step.supported);
        assert_eq!(step.duration_seconds, Some(60.0));
        assert_eq!(instructions.limitations.len(), 1);
        let target = serde_json::to_value(&step.target).unwrap();
        assert_eq!(target["kind"], "pace_range");
        assert_eq!(target["fast_seconds_per_km"], 1_000_000.0 / f64::from(high));
        assert_eq!(target["slow_seconds_per_km"], 1_000_000.0 / f64::from(low));
    }
    let bytes = instruction_fixture(CYCLING, 0, Some(0), Some(0), Some(2500), Some(4000));
    let instructions = decode_activity(&bytes).unwrap().instructions.unwrap();
    assert!(!instructions.steps[0].supported);
    assert!(matches!(
        instructions.steps[0].target,
        tp_core::fit::FitInstructionTarget::Unsupported
    ));
}

#[test]
fn heart_rate_targets_preserve_absolute_and_relative_bases_including_exact_bounds() {
    for (low, high, expected_low, expected_high, unit) in [
        (225, 260, 125.0, 160.0, "bpm"),
        (250, 250, 150.0, 150.0, "bpm"),
        (75, 85, 75.0, 85.0, "%HRmax"),
        (100, 100, 100.0, 100.0, "%HRmax"),
        (0, 0, 0.0, 0.0, "%HRmax"),
        (101, 101, 1.0, 1.0, "bpm"),
    ] {
        let bytes = instruction_fixture(RUNNING, 0, Some(1), Some(0), Some(low), Some(high));
        let instructions = decode_activity(&bytes).unwrap().instructions.unwrap();
        let step = &instructions.steps[0];
        assert!(step.supported);
        assert_eq!(
            serde_json::to_value(&step.target).unwrap(),
            serde_json::json!({
                "kind":"heart_rate_range", "low":expected_low, "high":expected_high, "unit":unit
            })
        );
    }
}

#[test]
fn missing_invalid_zoned_and_mixed_basis_targets_stay_unsupported() {
    let cases = [
        (None, Some(0), Some(2500), Some(4000)),
        (Some(0), None, Some(2500), Some(4000)),
        (Some(0), Some(1), Some(2500), Some(4000)), // named speed zone
        (Some(0), Some(0), None, Some(4000)),
        (Some(0), Some(0), Some(2500), None),
        (Some(0), Some(0), Some(0), Some(4000)),
        (Some(0), Some(0), Some(4000), Some(2500)),
        (Some(0), Some(0), Some(2500), Some(u32::MAX)),
        (Some(1), None, Some(225), Some(260)),
        (Some(1), Some(2), Some(225), Some(260)), // named HR zone
        (Some(1), Some(0), None, Some(260)),
        (Some(1), Some(0), Some(225), None),
        (Some(1), Some(0), Some(260), Some(225)),
        (Some(1), Some(0), Some(90), Some(250)), // percentage and bpm
        (Some(1), Some(0), Some(100), Some(101)),
        (Some(1), Some(0), Some(u32::MAX), Some(u32::MAX)),
        (Some(255), Some(0), Some(225), Some(260)),
    ];
    for (kind, value, low, high) in cases {
        let bytes = instruction_fixture(RUNNING, 0, kind, value, low, high);
        let instructions = decode_activity(&bytes).unwrap().instructions.unwrap();
        assert!(
            !instructions.steps[0].supported,
            "{kind:?}, {value:?}, {low:?}, {high:?}"
        );
        assert!(matches!(
            instructions.steps[0].target,
            tp_core::fit::FitInstructionTarget::Unsupported
        ));
        assert_eq!(instructions.limitations.len(), 2);
    }
    // An explicit open target remains open; missing target data is never open.
    let bytes = instruction_fixture(RUNNING, 0, Some(2), None, None, None);
    let instructions = decode_activity(&bytes).unwrap().instructions.unwrap();
    assert!(instructions.steps[0].supported);
    assert!(matches!(
        instructions.steps[0].target,
        tp_core::fit::FitInstructionTarget::Open
    ));
}

#[test]
fn decoded_targets_do_not_establish_supported_distance_or_repeat_endings() {
    for (kind, low, high) in [(0, 2500, 4000), (1, 225, 260)] {
        let bytes = instruction_fixture(RUNNING, 1, Some(kind), Some(0), Some(low), Some(high));
        let instructions = decode_activity(&bytes).unwrap().instructions.unwrap();
        assert!(instructions.steps[0].duration_seconds.is_none());
        assert!(!instructions.steps[0].supported);
        assert!(!matches!(
            instructions.steps[0].target,
            tp_core::fit::FitInstructionTarget::Unsupported
        ));
        let bytes = instruction_fixture(RUNNING, 6, Some(kind), Some(0), Some(low), Some(high));
        let instructions = decode_activity(&bytes).unwrap().instructions.unwrap();
        assert!(!instructions.steps[0].supported);
        assert!(matches!(
            instructions.steps[0].target,
            tp_core::fit::FitInstructionTarget::Unsupported
        ));
    }
}

#[test]
fn malformed_float_target_bounds_do_not_create_pace_or_hr_values() {
    for target_type in [0, 1] {
        for invalid in [f32::NAN, f32::INFINITY, -1.0, 0.5] {
            let definition = [
                0x4f, 0, 0, 27, 0, 7, 254, 2, 0x84, 1, 1, 0, 2, 4, 0x86, 3, 1, 0, 4, 4, 0x86, 5, 4,
                0x88, 6, 4, 0x88,
            ];
            let mut wire = definition.to_vec();
            wire.extend([15, 0, 0, 0]);
            wire.extend(60_000_u32.to_le_bytes());
            wire.push(target_type);
            wire.extend(0_u32.to_le_bytes());
            wire.extend(invalid.to_le_bytes());
            wire.extend(4000_f32.to_le_bytes());
            let instructions = decode_activity(&append(RUNNING, &wire))
                .unwrap()
                .instructions
                .unwrap();
            assert!(!instructions.steps[0].supported);
            assert!(matches!(
                instructions.steps[0].target,
                tp_core::fit::FitInstructionTarget::Unsupported
            ));
        }
    }
}
