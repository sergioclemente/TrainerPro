use tp_ble::codec::{self, ControlPointResponse};

#[test]
fn trainer_control_commands_and_responses_follow_ftms_wire_contract() {
    assert_eq!(codec::request_control(), [0x00]);
    assert_eq!(codec::reset_trainer(), [0x01]);
    assert_eq!(codec::set_target_power(220), [0x05, 220, 0]);
    assert_eq!(codec::set_target_power(300), [0x05, 0x2C, 0x01]);
    assert_eq!(codec::start_or_resume_training(), [0x07]);
    assert_eq!(codec::pause_training(), [0x08, 0x02]);
    assert_eq!(codec::flat_road_simulation(), [0x11, 0, 0, 0, 0, 40, 51]);

    assert_eq!(
        codec::parse_cp_response(&[0x80, 0x05, 0x01]),
        Ok(ControlPointResponse {
            request_opcode: codec::CP_SET_TARGET_POWER,
            result_code: codec::CP_RESULT_SUCCESS,
        })
    );
    assert!(codec::parse_cp_response(&[0x80, 0x05]).is_err());
}

#[test]
fn indoor_bike_data_decodes_live_speed_cadence_and_power() {
    // Flags: speed present, cadence and power present.
    let mut packet = vec![0x44, 0x00];
    packet.extend(2500u16.to_le_bytes()); // 25.00 km/h
    packet.extend(180u16.to_le_bytes()); // 90.0 rpm
    packet.extend(215i16.to_le_bytes());

    let measurement = codec::parse_indoor_bike_data(&packet).unwrap();
    assert_eq!(measurement.speed_kmh, Some(25.0));
    assert_eq!(measurement.cadence_rpm, Some(90.0));
    assert_eq!(measurement.power_w, Some(215));
}

#[test]
fn indoor_bike_data_respects_optional_fields_and_omitted_speed() {
    // The More Data flag suppresses speed; power still follows the flags.
    let mut packet = vec![0x41, 0x00];
    packet.extend(190i16.to_le_bytes());
    let measurement = codec::parse_indoor_bike_data(&packet).unwrap();
    assert_eq!(measurement.speed_kmh, None);
    assert_eq!(measurement.power_w, Some(190));

    // Skip unused fields in flag order before reading the live power value.
    let mut packet = vec![0x5D, 0x03];
    packet.extend(170u16.to_le_bytes()); // cadence: 85.0 rpm
    packet.extend(160u16.to_le_bytes()); // average cadence
    packet.extend([0x10, 0x27, 0x00]); // total distance, u24
    packet.extend(250i16.to_le_bytes()); // power
    packet.extend([1, 0, 2, 0, 3]); // expended energy
    packet.push(145); // heart rate; a dedicated HRM wins
    let measurement = codec::parse_indoor_bike_data(&packet).unwrap();
    assert_eq!(measurement.cadence_rpm, Some(85.0));
    assert_eq!(measurement.power_w, Some(250));
    assert_eq!(measurement.speed_kmh, None);
}

#[test]
fn indoor_bike_data_clamps_negative_power_and_rejects_short_packets() {
    let mut packet = vec![0x41, 0x00];
    packet.extend((-15i16).to_le_bytes());
    assert_eq!(
        codec::parse_indoor_bike_data(&packet).unwrap().power_w,
        Some(0)
    );

    // The flags promise a power value that is absent.
    assert!(codec::parse_indoor_bike_data(&[0x41, 0x00]).is_err());
    assert!(codec::parse_indoor_bike_data(&[0x44]).is_err());
}

#[test]
fn heart_rate_data_handles_both_formats_and_sensor_warmup() {
    assert_eq!(codec::parse_heart_rate(&[0x00, 142]).unwrap(), Some(142));
    let mut packet = vec![0x01];
    packet.extend(300u16.to_le_bytes());
    assert_eq!(codec::parse_heart_rate(&packet).unwrap(), Some(300));
    assert_eq!(codec::parse_heart_rate(&[0x00, 0]).unwrap(), None);
    assert!(codec::parse_heart_rate(&[0x01, 90]).is_err());
}
