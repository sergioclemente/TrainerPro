//! Pure FTMS / Heart Rate packet codecs. SPEC.md §4.2–§4.3. No BLE deps —
//! everything here is unit-testable byte manipulation.

use crate::traits::TrainerData;

// --- UUIDs (16-bit shorthand expanded to full base UUIDs by the drivers) ---
pub const SVC_FTMS: u16 = 0x1826;
pub const CHR_FITNESS_MACHINE_FEATURE: u16 = 0x2ACC;
pub const CHR_INDOOR_BIKE_DATA: u16 = 0x2AD2;
pub const CHR_SUPPORTED_POWER_RANGE: u16 = 0x2AD8;
pub const CHR_FTMS_CONTROL_POINT: u16 = 0x2AD9;
pub const CHR_FTMS_STATUS: u16 = 0x2ADA;
pub const SVC_HEART_RATE: u16 = 0x180D;
pub const CHR_HEART_RATE_MEASUREMENT: u16 = 0x2A37;

/// Fitness Machine Feature (0x2ACC) target-setting bit for power targets
/// (second uint32, bit 3: "Power Target Setting Supported").
pub const TARGET_SETTING_POWER_BIT: u32 = 1 << 3;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("packet too short: need {need} bytes, got {got}")]
    Short { need: usize, got: usize },
}

// ---------------------------------------------------------------------------
// FTMS Control Point (0x2AD9) — request builders
// ---------------------------------------------------------------------------

pub const CP_REQUEST_CONTROL: u8 = 0x00;
pub const CP_RESET: u8 = 0x01;
pub const CP_SET_TARGET_POWER: u8 = 0x05;
pub const CP_START_RESUME: u8 = 0x07;
pub const CP_STOP_PAUSE: u8 = 0x08;
pub const CP_SET_INDOOR_BIKE_SIM: u8 = 0x11;
pub const CP_RESPONSE: u8 = 0x80;

pub const CP_RESULT_SUCCESS: u8 = 0x01;

pub fn request_control() -> Vec<u8> {
    vec![CP_REQUEST_CONTROL]
}

pub fn reset() -> Vec<u8> {
    vec![CP_RESET]
}

pub fn set_target_power(watts: u16) -> Vec<u8> {
    let w = i16::try_from(watts).unwrap_or(i16::MAX).to_le_bytes();
    vec![CP_SET_TARGET_POWER, w[0], w[1]]
}

pub fn start_resume() -> Vec<u8> {
    vec![CP_START_RESUME]
}

/// FTMS stop/pause parameter: 0x01 = stop, 0x02 = pause. We pause (the spec
/// §4.2 table's `08 01` shorthand notwithstanding — pause is the semantic we
/// want mid-ride; the trainer keeps its state).
pub fn pause() -> Vec<u8> {
    vec![CP_STOP_PAUSE, 0x02]
}

/// Simulation parameters, grade 0 % (FreeRide): wind 0 m/s, grade 0.00 %,
/// crr 0.0040 (40 × 0.0001), cw 0.51 kg/m (51 × 0.01). SPEC §4.2 table.
pub fn sim_grade_zero() -> Vec<u8> {
    let wind = 0i16.to_le_bytes();
    let grade = 0i16.to_le_bytes();
    vec![CP_SET_INDOOR_BIKE_SIM, wind[0], wind[1], grade[0], grade[1], 40, 51]
}

/// Parse a control-point response indication `[0x80, req_op, result]`.
/// Returns `(request_opcode, result_code)`.
pub fn parse_cp_response(data: &[u8]) -> Result<(u8, u8), CodecError> {
    if data.len() < 3 {
        return Err(CodecError::Short { need: 3, got: data.len() });
    }
    // data[0] must be 0x80; tolerate and let the caller match request ops.
    Ok((data[1], data[2]))
}

// ---------------------------------------------------------------------------
// Indoor Bike Data (0x2AD2)
// ---------------------------------------------------------------------------

/// Flag-walking parser per SPEC §4.2: fields appear in bit order when set;
/// never assume fixed offsets. Unused fields are skipped by size.
pub fn parse_indoor_bike_data(data: &[u8]) -> Result<TrainerData, CodecError> {
    let mut r = Reader::new(data);
    let flags = r.u16()?;
    let mut out = TrainerData { power_w: None, cadence_rpm: None, speed_kmh: None };

    // Bit 0 ("More Data") INVERTED: instantaneous speed present when 0.
    if flags & (1 << 0) == 0 {
        out.speed_kmh = Some(f32::from(r.u16()?) * 0.01);
    }
    if flags & (1 << 1) != 0 {
        r.skip(2)?; // average speed
    }
    if flags & (1 << 2) != 0 {
        out.cadence_rpm = Some(f32::from(r.u16()?) * 0.5);
    }
    if flags & (1 << 3) != 0 {
        r.skip(2)?; // average cadence
    }
    if flags & (1 << 4) != 0 {
        r.skip(3)?; // total distance, uint24
    }
    if flags & (1 << 5) != 0 {
        r.skip(2)?; // resistance level
    }
    if flags & (1 << 6) != 0 {
        let p = r.i16()?;
        out.power_w = Some(p.max(0) as u16);
    }
    if flags & (1 << 7) != 0 {
        r.skip(2)?; // average power
    }
    if flags & (1 << 8) != 0 {
        r.skip(5)?; // expended energy: total u16 + per-hour u16 + per-min u8
    }
    if flags & (1 << 9) != 0 {
        r.skip(1)?; // heart rate (we use a dedicated HRM)
    }
    if flags & (1 << 10) != 0 {
        r.skip(1)?; // metabolic equivalent
    }
    if flags & (1 << 11) != 0 {
        r.skip(2)?; // elapsed time
    }
    if flags & (1 << 12) != 0 {
        r.skip(2)?; // remaining time
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Heart Rate Measurement (0x2A37)
// ---------------------------------------------------------------------------

/// Returns `None` for a 0 bpm reading (sensor warming up, SPEC §4.3).
pub fn parse_heart_rate(data: &[u8]) -> Result<Option<u16>, CodecError> {
    let mut r = Reader::new(data);
    let flags = r.u8()?;
    let bpm = if flags & 0x01 != 0 { r.u16()? } else { u16::from(r.u8()?) };
    Ok(if bpm == 0 { None } else { Some(bpm) })
}

// ---------------------------------------------------------------------------

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        if self.pos + n > self.data.len() {
            return Err(CodecError::Short { need: self.pos + n, got: self.data.len() });
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> Result<(), CodecError> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, CodecError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn i16(&mut self) -> Result<i16, CodecError> {
        let b = self.take(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp_builders() {
        assert_eq!(request_control(), [0x00]);
        assert_eq!(set_target_power(220), [0x05, 220, 0]);
        assert_eq!(set_target_power(300), [0x05, 0x2C, 0x01]);
        assert_eq!(pause(), [0x08, 0x02]);
        assert_eq!(sim_grade_zero(), [0x11, 0, 0, 0, 0, 40, 51]);
    }

    #[test]
    fn cp_response_parses_and_rejects_short() {
        assert_eq!(parse_cp_response(&[0x80, 0x05, 0x01]), Ok((0x05, 0x01)));
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
}
