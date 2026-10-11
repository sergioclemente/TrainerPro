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
