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
    assert!(decode_activity(&corrupt).is_err());
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
    for offset in [0, 10, 12] {
        let message = [
            definition.as_slice(),
            &[15],
            &(fit_start + offset).to_le_bytes(),
        ]
        .concat();
        assert!(matches!(
            decode_activity(&append(CYCLING, &message)),
            Err(FitDecodeError::InvalidTiming)
        ));
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
