use super::*;

#[test]
fn cp_builders() {
    assert_eq!(request_control(), [0x00]);
    assert_eq!(reset_trainer(), [0x01]);
    assert_eq!(set_target_power(220), [0x05, 220, 0]);
    assert_eq!(set_target_power(300), [0x05, 0x2C, 0x01]);
    assert_eq!(start_or_resume_training(), [0x07]);
    assert_eq!(pause_training(), [0x08, 0x02]);
    assert_eq!(flat_road_simulation(), [0x11, 0, 0, 0, 0, 40, 51]);
}

#[test]
fn cp_response_parses_and_rejects_short() {
    assert_eq!(
        parse_cp_response(&[0x80, 0x05, 0x01]),
        Ok(ControlPointResponse {
            request_opcode: CP_SET_TARGET_POWER,
            result_code: CP_RESULT_SUCCESS,
        })
    );
    assert!(parse_cp_response(&[0x80, 0x05]).is_err());
}

#[test]
fn ibd_speed_cadence_power() {
    // flags: bit0=0 (speed present), bit2 cadence, bit6 power = 0x0044
    let mut p = vec![0x44, 0x00];
    p.extend(2500u16.to_le_bytes()); // 25.00 km/h
    p.extend(180u16.to_le_bytes()); // 90.0 rpm
    p.extend(215i16.to_le_bytes());
    let d = parse_indoor_bike_data(&p).unwrap();
    assert_eq!(d.speed_kmh, Some(25.0));
    assert_eq!(d.cadence_rpm, Some(90.0));
    assert_eq!(d.power_w, Some(215));
}

#[test]
fn ibd_more_data_bit_suppresses_speed() {
    // flags: bit0=1 (no speed), bit6 power = 0x0041
    let mut p = vec![0x41, 0x00];
    p.extend(190i16.to_le_bytes());
    let d = parse_indoor_bike_data(&p).unwrap();
    assert_eq!(d.speed_kmh, None);
    assert_eq!(d.power_w, Some(190));
}

#[test]
fn ibd_skips_unused_fields_in_bit_order() {
    // bit0=1, bit2 cadence, bit3 avg cadence, bit4 distance(u24),
    // bit6 power, bit8 energy(5), bit9 hr = 0x035D
    let mut p = vec![0x5D, 0x03];
    p.extend(170u16.to_le_bytes()); // cadence 85.0
    p.extend(160u16.to_le_bytes()); // avg cadence (skip)
    p.extend([0x10, 0x27, 0x00]); // distance u24 (skip)
    p.extend(250i16.to_le_bytes()); // power
    p.extend([1, 0, 2, 0, 3]); // energy (skip)
    p.push(145); // hr (skip — dedicated HRM wins)
    let d = parse_indoor_bike_data(&p).unwrap();
    assert_eq!(d.cadence_rpm, Some(85.0));
    assert_eq!(d.power_w, Some(250));
    assert_eq!(d.speed_kmh, None);
}

#[test]
fn ibd_negative_power_clamps_to_zero() {
    let mut p = vec![0x41, 0x00];
    p.extend((-15i16).to_le_bytes());
    assert_eq!(parse_indoor_bike_data(&p).unwrap().power_w, Some(0));
}

#[test]
fn ibd_short_packet_errors() {
    // Claims power present but omits the bytes.
    assert!(parse_indoor_bike_data(&[0x41, 0x00]).is_err());
    assert!(parse_indoor_bike_data(&[0x44]).is_err());
}

#[test]
fn hr_both_formats_and_warmup_zero() {
    assert_eq!(parse_heart_rate(&[0x00, 142]).unwrap(), Some(142));
    let mut p = vec![0x01];
    p.extend(300u16.to_le_bytes());
    assert_eq!(parse_heart_rate(&p).unwrap(), Some(300));
    assert_eq!(parse_heart_rate(&[0x00, 0]).unwrap(), None);
    assert!(parse_heart_rate(&[0x01, 90]).is_err()); // u16 claimed, 1 byte
}
