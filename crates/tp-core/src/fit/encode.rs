//! FIT container encoding: 14-byte header, definition + data records
//! (little-endian), trailing CRC-16. Message sequence per SPEC.md §7.2:
//! file_id → device_info → event(start) → records (1 Hz, with stop/start
//! event pairs interleaved chronologically at pauses) → laps → session →
//! activity. Timestamps: unix_s − FIT_EPOCH_OFFSET_S.

use super::profile as p;
use super::{crc, FitError, FitRide};
use crate::consts::FIT_EPOCH_OFFSET_S;
use crate::journal::RideEventKind;

/// One field in a definition record: (field number, size, base type).
#[derive(Clone, Copy)]
struct FieldDef {
    num: u8,
    size: u8,
    base: u8,
}

const fn f(num: u8, size: u8, base: u8) -> FieldDef {
    FieldDef { num, size, base }
}

pub fn encode_activity(ride: &FitRide) -> Result<Vec<u8>, FitError> {
    let h = ride.header;
    let start_ms = h.started_unix_ms;
    let ts_start = fit_ts(start_ms)?;

    let mut body: Vec<u8> = Vec::with_capacity(1024 + ride.samples.len() * 12);

    // -- 1. file_id ---------------------------------------------------------
    let file_id_fields = [
        f(p::FILE_ID_TYPE, 1, p::BASE_ENUM),
        f(p::FILE_ID_MANUFACTURER, 2, p::BASE_UINT16),
        f(p::FILE_ID_PRODUCT, 2, p::BASE_UINT16),
        f(p::FILE_ID_SERIAL_NUMBER, 4, p::BASE_UINT32Z),
        f(p::FILE_ID_TIME_CREATED, 4, p::BASE_UINT32),
    ];
    write_definition(&mut body, p::LOCAL_FILE_ID, p::MSG_FILE_ID, &file_id_fields);
    body.push(p::LOCAL_FILE_ID);
    body.push(p::FILE_TYPE_ACTIVITY);
    put_u16(&mut body, p::MANUFACTURER_DEVELOPMENT);
    put_u16(&mut body, p::PRODUCT_TRAINERPRO);
    put_u32(&mut body, serial_from(&h.ride_id));
    put_u32(&mut body, ts_start);

    // -- 2. device_info ×1–3 ------------------------------------------------
    let device_info_fields = [
        f(p::DEVICE_INFO_DEVICE_INDEX, 1, p::BASE_UINT8),
        f(p::DEVICE_INFO_MANUFACTURER, 2, p::BASE_UINT16),
        f(p::DEVICE_INFO_SOFTWARE_VERSION, 2, p::BASE_UINT16),
        f(p::DEVICE_INFO_PRODUCT_NAME, p::PRODUCT_NAME_SIZE, p::BASE_STRING),
    ];
    write_definition(
        &mut body,
        p::LOCAL_DEVICE_INFO,
        p::MSG_DEVICE_INFO,
        &device_info_fields,
    );
    let mut devices: Vec<(&str, u16)> = vec![("TrainerPro", software_version(&h.app_ver))];
    if let Some(t) = h.trainer.as_deref() {
        devices.push((t, p::INVALID_UINT16));
    }
    if let Some(hrm) = h.hrm.as_deref() {
        devices.push((hrm, p::INVALID_UINT16));
    }
    for (idx, (name, sw)) in devices.iter().enumerate() {
        body.push(p::LOCAL_DEVICE_INFO);
        body.push(p::DEVICE_INDEX_CREATOR + idx as u8);
        put_u16(&mut body, p::MANUFACTURER_DEVELOPMENT);
        put_u16(&mut body, *sw);
        put_string(&mut body, name, p::PRODUCT_NAME_SIZE as usize);
    }

    // -- 3. event: timer start ----------------------------------------------
    let event_fields = [
        f(p::EVENT_TIMESTAMP, 4, p::BASE_UINT32),
        f(p::EVENT_EVENT, 1, p::BASE_ENUM),
        f(p::EVENT_EVENT_TYPE, 1, p::BASE_ENUM),
    ];
    write_definition(&mut body, p::LOCAL_EVENT, p::MSG_EVENT, &event_fields);
    let start_t_ms = ride
        .events
        .iter()
        .find(|e| e.kind == RideEventKind::Start)
        .map(|e| e.t_ms)
        .unwrap_or(0);
    write_timer_event(&mut body, fit_ts(start_ms + start_t_ms)?, p::EVENT_TYPE_START);

    // -- 4. records (1 Hz), pause stop/start pairs interleaved --------------
    let mut record_fields = vec![
        f(p::RECORD_TIMESTAMP, 4, p::BASE_UINT32),
        f(p::RECORD_HEART_RATE, 1, p::BASE_UINT8),
        f(p::RECORD_CADENCE, 1, p::BASE_UINT8),
        f(p::RECORD_POWER, 2, p::BASE_UINT16),
    ];
    if ride.record_distance {
        record_fields.push(f(p::RECORD_SPEED, 2, p::BASE_UINT16));
        record_fields.push(f(p::RECORD_DISTANCE, 4, p::BASE_UINT32));
    }
    write_definition(&mut body, p::LOCAL_RECORD, p::MSG_RECORD, &record_fields);

    // Merge samples with pause/resume events, chronological by t_ms. On a
    // timestamp tie a Resume precedes samples (recording restarts at that
    // instant) and a Pause follows them (last sample belongs to the active
    // span).
    enum Item {
        Sample(usize),
        Pause(u64),
        Resume(u64),
    }
    let mut items: Vec<(u64, u8, Item)> = ride
        .samples
        .iter()
        .enumerate()
        .map(|(i, s)| (s.t_ms, 1u8, Item::Sample(i)))
        .collect();
    for e in ride.events {
        match e.kind {
            RideEventKind::Pause => items.push((e.t_ms, 2, Item::Pause(e.t_ms))),
            RideEventKind::Resume => items.push((e.t_ms, 0, Item::Resume(e.t_ms))),
            _ => {}
        }
    }
    items.sort_by_key(|(t, rank, _)| (*t, *rank));

    let flat = FlatRoad::new(h.weight_kg);
    let mut distance_m = 0.0f64;
    for (_, _, item) in &items {
        match item {
            Item::Sample(i) => {
                let s = &ride.samples[*i];
                body.push(p::LOCAL_RECORD);
                put_u32(&mut body, fit_ts(start_ms + s.t_ms)?);
                body.push(opt_u8(s.hr));
                body.push(opt_u8(s.cadence));
                put_u16(&mut body, s.power.unwrap_or(p::INVALID_UINT16));
                if ride.record_distance {
                    let v = flat.speed_ms(f64::from(s.power.unwrap_or(0)));
                    distance_m += v; // 1 Hz → v m/s × 1 s
                    put_u16(&mut body, scale_u16(v, p::SPEED_SCALE));
                    put_u32(&mut body, scale_u32(distance_m, p::DISTANCE_SCALE));
                }
            }
            Item::Pause(t) => {
                write_timer_event(&mut body, fit_ts(start_ms + t)?, p::EVENT_TYPE_STOP_ALL)
            }
            Item::Resume(t) => {
                write_timer_event(&mut body, fit_ts(start_ms + t)?, p::EVENT_TYPE_START)
            }
        }
    }

    // -- 5. laps ×M ---------------------------------------------------------
    let lap_fields = [
        f(p::LAP_MESSAGE_INDEX, 2, p::BASE_UINT16),
        f(p::LAP_TIMESTAMP, 4, p::BASE_UINT32),
        f(p::LAP_START_TIME, 4, p::BASE_UINT32),
        f(p::LAP_TOTAL_ELAPSED_TIME, 4, p::BASE_UINT32),
        f(p::LAP_TOTAL_TIMER_TIME, 4, p::BASE_UINT32),
        f(p::LAP_TOTAL_CALORIES, 2, p::BASE_UINT16),
        f(p::LAP_AVG_HEART_RATE, 1, p::BASE_UINT8),
        f(p::LAP_MAX_HEART_RATE, 1, p::BASE_UINT8),
        f(p::LAP_AVG_CADENCE, 1, p::BASE_UINT8),
        f(p::LAP_AVG_POWER, 2, p::BASE_UINT16),
        f(p::LAP_MAX_POWER, 2, p::BASE_UINT16),
    ];
    write_definition(&mut body, p::LOCAL_LAP, p::MSG_LAP, &lap_fields);
    for (i, lap) in ride.laps.iter().enumerate() {
        body.push(p::LOCAL_LAP);
        put_u16(&mut body, i as u16);
        put_u32(&mut body, fit_ts(start_ms + lap.end_ms)?);
        put_u32(&mut body, fit_ts(start_ms + lap.start_ms)?);
        put_u32(&mut body, ms_u32(lap.end_ms - lap.start_ms)?); // scale 1000 = ms
        put_u32(&mut body, ms_u32(lap.timer_ms)?);
        put_u16(&mut body, lap.calories_kcal);
        body.push(opt_u8(lap.avg_hr));
        body.push(opt_u8(lap.max_hr));
        body.push(opt_u8(lap.avg_cadence));
        put_u16(&mut body, lap.avg_power.unwrap_or(p::INVALID_UINT16));
        put_u16(&mut body, lap.max_power.unwrap_or(p::INVALID_UINT16));
    }

    // -- 6. session ---------------------------------------------------------
    let t = ride.totals;
    let elapsed_ms = u64::from(t.elapsed_s) * 1000;
    let ts_end = fit_ts(start_ms + elapsed_ms)?;
    let session_fields = [
        f(p::SESSION_TIMESTAMP, 4, p::BASE_UINT32),
        f(p::SESSION_START_TIME, 4, p::BASE_UINT32),
        f(p::SESSION_SPORT, 1, p::BASE_ENUM),
        f(p::SESSION_SUB_SPORT, 1, p::BASE_ENUM),
        f(p::SESSION_TOTAL_ELAPSED_TIME, 4, p::BASE_UINT32),
        f(p::SESSION_TOTAL_TIMER_TIME, 4, p::BASE_UINT32),
        f(p::SESSION_TOTAL_CALORIES, 2, p::BASE_UINT16),
        f(p::SESSION_AVG_HEART_RATE, 1, p::BASE_UINT8),
        f(p::SESSION_MAX_HEART_RATE, 1, p::BASE_UINT8),
        f(p::SESSION_AVG_CADENCE, 1, p::BASE_UINT8),
        f(p::SESSION_AVG_POWER, 2, p::BASE_UINT16),
        f(p::SESSION_MAX_POWER, 2, p::BASE_UINT16),
        f(p::SESSION_FIRST_LAP_INDEX, 2, p::BASE_UINT16),
        f(p::SESSION_NUM_LAPS, 2, p::BASE_UINT16),
        f(p::SESSION_NORMALIZED_POWER, 2, p::BASE_UINT16),
        f(p::SESSION_TRAINING_STRESS_SCORE, 2, p::BASE_UINT16),
        f(p::SESSION_INTENSITY_FACTOR, 2, p::BASE_UINT16),
        f(p::SESSION_THRESHOLD_POWER, 2, p::BASE_UINT16),
    ];
    write_definition(&mut body, p::LOCAL_SESSION, p::MSG_SESSION, &session_fields);
    body.push(p::LOCAL_SESSION);
    put_u32(&mut body, ts_end);
    put_u32(&mut body, ts_start);
    body.push(p::SPORT_CYCLING);
    body.push(p::SUB_SPORT_INDOOR_CYCLING);
    put_u32(&mut body, ms_u32(elapsed_ms)?);
    put_u32(&mut body, ms_u32(u64::from(t.timer_s) * 1000)?);
    put_u16(&mut body, t.kj.min(u32::from(u16::MAX - 1)) as u16); // kJ ≈ kcal
    body.push(opt_u8(t.avg_hr));
    body.push(opt_u8(t.max_hr));
    body.push(opt_u8(t.avg_cadence));
    put_u16(&mut body, t.avg_power.unwrap_or(p::INVALID_UINT16));
    put_u16(&mut body, t.max_power.unwrap_or(p::INVALID_UINT16));
    put_u16(&mut body, 0); // first_lap_index
    put_u16(&mut body, ride.laps.len() as u16);
    put_u16(&mut body, t.np.unwrap_or(p::INVALID_UINT16));
    put_u16(
        &mut body,
        t.tss
            .map(|v| scale_u16(v, p::TSS_SCALE))
            .unwrap_or(p::INVALID_UINT16),
    );
    put_u16(
        &mut body,
        t.if_
            .map(|v| scale_u16(v, p::IF_SCALE))
            .unwrap_or(p::INVALID_UINT16),
    );
    put_u16(&mut body, h.ftp);

    // -- 7. activity --------------------------------------------------------
    let activity_fields = [
        f(p::ACTIVITY_TIMESTAMP, 4, p::BASE_UINT32),
        f(p::ACTIVITY_TOTAL_TIMER_TIME, 4, p::BASE_UINT32),
        f(p::ACTIVITY_NUM_SESSIONS, 2, p::BASE_UINT16),
        f(p::ACTIVITY_TYPE, 1, p::BASE_ENUM),
        f(p::ACTIVITY_EVENT, 1, p::BASE_ENUM),
        f(p::ACTIVITY_EVENT_TYPE, 1, p::BASE_ENUM),
        f(p::ACTIVITY_LOCAL_TIMESTAMP, 4, p::BASE_UINT32),
    ];
    write_definition(&mut body, p::LOCAL_ACTIVITY, p::MSG_ACTIVITY, &activity_fields);
    body.push(p::LOCAL_ACTIVITY);
    put_u32(&mut body, ts_end);
    put_u32(&mut body, ms_u32(u64::from(t.timer_s) * 1000)?);
    put_u16(&mut body, 1);
    body.push(p::ACTIVITY_TYPE_MANUAL);
    body.push(p::EVENT_ACTIVITY);
    body.push(p::EVENT_TYPE_STOP);
    // tp-core has no timezone knowledge; local_timestamp = UTC timestamp.
    put_u32(&mut body, ts_end);

    // -- container: header + body + trailing CRC ----------------------------
    let data_size: u32 = body
        .len()
        .try_into()
        .map_err(|_| FitError::Encode("body exceeds u32 data size".into()))?;
    let mut out = Vec::with_capacity(14 + body.len() + 2);
    out.push(p::HEADER_SIZE);
    out.push(p::PROTOCOL_VERSION);
    out.extend_from_slice(&p::PROFILE_VERSION.to_le_bytes());
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(p::DATA_TYPE);
    let header_crc = crc::checksum(&out[..12]);
    out.extend_from_slice(&header_crc.to_le_bytes());
    out.extend_from_slice(&body);
    let file_crc = crc::checksum(&out);
    out.extend_from_slice(&file_crc.to_le_bytes());
    Ok(out)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// FIT timestamp (seconds since 1989-12-31T00:00Z) from unix milliseconds.
fn fit_ts(unix_ms: u64) -> Result<u32, FitError> {
    let unix_s = unix_ms / 1000;
    let fit_s = unix_s
        .checked_sub(FIT_EPOCH_OFFSET_S)
        .ok_or_else(|| FitError::Encode(format!("timestamp {unix_s}s predates FIT epoch")))?;
    u32::try_from(fit_s).map_err(|_| FitError::Encode("timestamp beyond u32 FIT range".into()))
}

fn write_definition(body: &mut Vec<u8>, local: u8, global: u16, fields: &[FieldDef]) {
    body.push(p::DEFINITION_HEADER_BIT | local);
    body.push(0); // reserved
    body.push(p::ARCH_LITTLE_ENDIAN);
    body.extend_from_slice(&global.to_le_bytes());
    body.push(fields.len() as u8);
    for fd in fields {
        body.push(fd.num);
        body.push(fd.size);
        body.push(fd.base);
    }
}

fn write_timer_event(body: &mut Vec<u8>, ts: u32, event_type: u8) {
    body.push(p::LOCAL_EVENT);
    put_u32(body, ts);
    body.push(p::EVENT_TIMER);
    body.push(event_type);
}

fn put_u16(body: &mut Vec<u8>, v: u16) {
    body.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(body: &mut Vec<u8>, v: u32) {
    body.extend_from_slice(&v.to_le_bytes());
}

/// NUL-terminated string padded with zeros to a fixed `size`; over-long
/// names are truncated on a char boundary to `size - 1` bytes.
fn put_string(body: &mut Vec<u8>, s: &str, size: usize) {
    let max = size - 1;
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let bytes = &s.as_bytes()[..end];
    body.extend_from_slice(bytes);
    body.extend(std::iter::repeat_n(0u8, size - bytes.len()));
}

fn opt_u8(v: Option<u16>) -> u8 {
    v.map(|x| x.min(u16::from(u8::MAX - 1)) as u8)
        .unwrap_or(p::INVALID_UINT8)
}

/// Milliseconds value into a uint32 scale-1000 seconds field (value is
/// already in ms because scale 1000 × seconds = milliseconds).
fn ms_u32(ms: u64) -> Result<u32, FitError> {
    u32::try_from(ms).map_err(|_| FitError::Encode("duration exceeds u32 ms".into()))
}

fn scale_u16(v: f64, scale: f64) -> u16 {
    (v * scale).round().clamp(0.0, f64::from(u16::MAX - 1)) as u16
}

fn scale_u32(v: f64, scale: f64) -> u32 {
    (v * scale).round().clamp(0.0, f64::from(u32::MAX - 1)) as u32
}

/// Deterministic per-install serial (FNV-1a over the ride id; uint32z, so
/// never 0). tp-core has no persistence — the ride id stands in for a true
/// per-install random serial.
fn serial_from(id: &str) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in id.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    if h == 0 {
        1
    } else {
        h
    }
}

/// `"major.minor[.patch]"` → uint16 with FIT scale 100 (e.g. "1.2" → 120,
/// displayed by Garmin as 1.20). Unparseable → invalid.
fn software_version(app_ver: &str) -> u16 {
    let mut parts = app_ver.split('.');
    let major: u16 = match parts.next().and_then(|s| s.parse().ok()) {
        Some(v) => v,
        None => return p::INVALID_UINT16,
    };
    let minor: u16 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = if minor < 10 { minor * 10 } else { minor.min(99) };
    major.saturating_mul(100).saturating_add(minor).min(u16::MAX - 1)
}

/// Virtual flat-road speed model: solve P = k_a·v³ + k_r·v (aero + rolling
/// resistance, no slope/wind) for v via Newton's method. Constants: air
/// density 1.225 kg/m³, CdA 0.32 m², Crr 0.005, bike mass 9 kg, g 9.81.
struct FlatRoad {
    k_aero: f64,
    k_roll: f64,
}

impl FlatRoad {
    fn new(rider_kg: f64) -> Self {
        let mass = rider_kg.max(0.0) + 9.0;
        FlatRoad {
            k_aero: 0.5 * 1.225 * 0.32,
            k_roll: 0.005 * mass * 9.81,
        }
    }

    fn speed_ms(&self, power_w: f64) -> f64 {
        if power_w <= 0.0 {
            return 0.0;
        }
        let mut v = 8.0; // m/s starting guess
        for _ in 0..25 {
            let fx = self.k_aero * v * v * v + self.k_roll * v - power_w;
            let dfx = 3.0 * self.k_aero * v * v + self.k_roll;
            let next = v - fx / dfx;
            v = if next > 0.0 { next } else { v / 2.0 };
        }
        v.max(0.0)
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit::FitRide;
    use crate::journal::{JournalHeader, Lap, RideEvent, RideEventKind, Sample};
    use crate::metrics::SessionTotals;

    // -- minimal FIT decoder ------------------------------------------------

    #[derive(Clone)]
    struct DecField {
        num: u8,
        raw: Vec<u8>,
    }

    #[derive(Clone)]
    struct DecMsg {
        global: u16,
        fields: Vec<DecField>,
    }

    impl DecMsg {
        fn raw(&self, num: u8) -> Option<&[u8]> {
            self.fields
                .iter()
                .find(|f| f.num == num)
                .map(|f| f.raw.as_slice())
        }
        /// Little-endian unsigned decode of a field (1/2/4 bytes).
        fn uint(&self, num: u8) -> u64 {
            let raw = self.raw(num).expect("field present");
            raw.iter()
                .rev()
                .fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
        }
        fn string(&self, num: u8) -> String {
            let raw = self.raw(num).expect("field present");
            let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
            String::from_utf8(raw[..end].to_vec()).unwrap()
        }
        fn has(&self, num: u8) -> bool {
            self.raw(num).is_some()
        }
    }

    /// Walk definition + data records; returns messages in file order.
    fn decode(buf: &[u8]) -> Vec<DecMsg> {
        assert!(buf.len() >= 16, "file too small");
        let header_size = buf[0] as usize;
        assert_eq!(header_size, 14);
        let data_size =
            u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        assert_eq!(data_size, buf.len() - header_size - 2, "data_size field");
        let body = &buf[header_size..buf.len() - 2];

        struct Def {
            global: u16,
            fields: Vec<(u8, u8)>, // (num, size)
        }
        let mut defs: Vec<Option<Def>> = (0..16).map(|_| None).collect();
        let mut msgs = Vec::new();
        let mut i = 0usize;
        while i < body.len() {
            let hdr = body[i];
            i += 1;
            assert_eq!(hdr & 0x80, 0, "compressed-timestamp headers unexpected");
            let local = (hdr & 0x0F) as usize;
            if hdr & 0x40 != 0 {
                // definition record
                let _reserved = body[i];
                let arch = body[i + 1];
                assert_eq!(arch, 0, "little-endian expected");
                let global = u16::from_le_bytes([body[i + 2], body[i + 3]]);
                let nfields = body[i + 4] as usize;
                i += 5;
                let mut fields = Vec::with_capacity(nfields);
                for _ in 0..nfields {
                    fields.push((body[i], body[i + 1]));
                    // body[i+2] = base type, unused by this decoder
                    i += 3;
                }
                defs[local] = Some(Def { global, fields });
            } else {
                let def = defs[local].as_ref().expect("data before definition");
                let mut fields = Vec::with_capacity(def.fields.len());
                for (num, size) in &def.fields {
                    let raw = body[i..i + *size as usize].to_vec();
                    i += *size as usize;
                    fields.push(DecField { num: *num, raw });
                }
                msgs.push(DecMsg {
                    global: def.global,
                    fields,
                });
            }
        }
        assert_eq!(i, body.len(), "trailing bytes in body");
        msgs
    }

    // -- synthetic ride -----------------------------------------------------

    /// 2026-01-01T00:00:00Z.
    const START_UNIX_S: u64 = 1_767_225_600;
    const TS0: u64 = START_UNIX_S - 631_065_600; // FIT epoch conversion

    fn header() -> JournalHeader {
        JournalHeader {
            ride_id: "ride-abc".into(),
            started_unix_ms: START_UNIX_S * 1000,
            workout_name: "2x20".into(),
            ftp: 250,
            weight_kg: 75.0,
            trainer: Some("KICKR".into()),
            hrm: Some("HRM-Dual".into()),
            app_ver: "1.2.3".into(),
        }
    }

    /// 10 samples (t = 0..=9 s), pause 9.5 s → resume 14.5 s, 5 samples
    /// (t = 15..=19 s). Sample at t=3 has no HR.
    fn samples() -> Vec<Sample> {
        let mut v = Vec::new();
        for t in 0u64..10 {
            v.push(Sample {
                t_ms: t * 1000,
                power: Some(200 + t as u16),
                cadence: Some(90),
                hr: if t == 3 { None } else { Some(140) },
                target: Some(200),
            });
        }
        for t in 15u64..20 {
            v.push(Sample {
                t_ms: t * 1000,
                power: Some(210),
                cadence: Some(92),
                hr: Some(145),
                target: Some(210),
            });
        }
        v
    }

    fn events() -> Vec<RideEvent> {
        let e = |t_ms, kind, seg| RideEvent { t_ms, kind, seg };
        vec![
            e(0, RideEventKind::Start, None),
            e(5_000, RideEventKind::Lap, Some(0)),
            e(9_500, RideEventKind::Pause, None),
            e(14_500, RideEventKind::Resume, None),
            e(19_500, RideEventKind::End, None),
        ]
    }

    fn laps() -> Vec<Lap> {
        vec![
            Lap {
                start_ms: 0,
                end_ms: 5_000,
                timer_ms: 5_000,
                avg_power: Some(202),
                max_power: Some(204),
                avg_hr: Some(140),
                max_hr: Some(140),
                avg_cadence: Some(90),
                calories_kcal: 1,
            },
            Lap {
                start_ms: 5_000,
                end_ms: 19_500,
                timer_ms: 9_500,
                avg_power: Some(208),
                max_power: Some(210),
                avg_hr: Some(143),
                max_hr: Some(145),
                avg_cadence: Some(91),
                calories_kcal: 2,
            },
        ]
    }

    fn totals() -> SessionTotals {
        SessionTotals {
            elapsed_s: 20,
            timer_s: 15,
            avg_power: Some(205),
            max_power: Some(210),
            np: Some(207),
            if_: Some(0.828),
            tss: Some(2.86),
            avg_hr: Some(142),
            max_hr: Some(145),
            avg_cadence: Some(90),
            kj: 3,
        }
    }

    fn encode(record_distance: bool) -> Vec<u8> {
        let h = header();
        let s = samples();
        let e = events();
        let l = laps();
        let t = totals();
        encode_activity(&FitRide {
            header: &h,
            samples: &s,
            events: &e,
            laps: &l,
            totals: &t,
            record_distance,
        })
        .unwrap()
    }

    // -- container tests ----------------------------------------------------

    #[test]
    fn header_is_14_bytes_and_valid() {
        let buf = encode(false);
        assert_eq!(buf[0], 14, "header size");
        assert_eq!(buf[1], 0x20, "protocol version");
        let profile = u16::from_le_bytes([buf[2], buf[3]]);
        assert_eq!(profile, p::PROFILE_VERSION);
        let data_size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        assert_eq!(data_size, buf.len() - 14 - 2, "data size excludes header+CRC");
        assert_eq!(&buf[8..12], b".FIT");
        let hdr_crc = u16::from_le_bytes([buf[12], buf[13]]);
        assert_eq!(hdr_crc, crc::checksum(&buf[..12]), "header CRC");
        assert_eq!(crc::checksum(&buf[..14]), 0, "header self-validates");
    }

    #[test]
    fn trailing_crc_validates() {
        let buf = encode(false);
        let trailer = u16::from_le_bytes([buf[buf.len() - 2], buf[buf.len() - 1]]);
        assert_eq!(trailer, crc::checksum(&buf[..buf.len() - 2]));
        assert_eq!(crc::checksum(&buf), 0, "whole file self-validates");
    }

    // -- message sequence + content -----------------------------------------

    #[test]
    fn message_order_per_spec() {
        let msgs = decode(&encode(false));
        let globals: Vec<u16> = msgs.iter().map(|m| m.global).collect();
        assert_eq!(globals[0], p::MSG_FILE_ID);
        assert_eq!(&globals[1..4], &[p::MSG_DEVICE_INFO; 3]);
        assert_eq!(globals[4], p::MSG_EVENT, "timer start event");
        // Middle: records + pause events only.
        let tail_start = globals.len() - 4;
        for g in &globals[5..tail_start] {
            assert!(
                *g == p::MSG_RECORD || *g == p::MSG_EVENT,
                "only record/event between start and laps, got {g}"
            );
        }
        assert_eq!(
            &globals[tail_start..],
            &[p::MSG_LAP, p::MSG_LAP, p::MSG_SESSION, p::MSG_ACTIVITY]
        );
    }

    #[test]
    fn file_id_and_device_info() {
        let msgs = decode(&encode(false));
        let fid = &msgs[0];
        assert_eq!(fid.uint(p::FILE_ID_TYPE), 4, "file type activity");
        assert_eq!(fid.uint(p::FILE_ID_MANUFACTURER), 255, "development");
        assert_eq!(fid.uint(p::FILE_ID_PRODUCT), 1);
        assert_ne!(fid.uint(p::FILE_ID_SERIAL_NUMBER), 0, "uint32z serial nonzero");
        assert_eq!(fid.uint(p::FILE_ID_TIME_CREATED), TS0);

        let devs: Vec<&DecMsg> =
            msgs.iter().filter(|m| m.global == p::MSG_DEVICE_INFO).collect();
        assert_eq!(devs.len(), 3);
        assert_eq!(devs[0].string(p::DEVICE_INFO_PRODUCT_NAME), "TrainerPro");
        assert_eq!(devs[1].string(p::DEVICE_INFO_PRODUCT_NAME), "KICKR");
        assert_eq!(devs[2].string(p::DEVICE_INFO_PRODUCT_NAME), "HRM-Dual");
        for (i, d) in devs.iter().enumerate() {
            assert_eq!(d.uint(p::DEVICE_INFO_DEVICE_INDEX), i as u64);
            assert_eq!(d.uint(p::DEVICE_INFO_MANUFACTURER), 255);
        }
        // "1.2.3" → 1.20 → 120; peripheral versions unknown → invalid.
        assert_eq!(devs[0].uint(p::DEVICE_INFO_SOFTWARE_VERSION), 120);
        assert_eq!(devs[1].uint(p::DEVICE_INFO_SOFTWARE_VERSION), 0xFFFF);
    }

    #[test]
    fn records_1hz_with_fit_epoch_timestamps() {
        let msgs = decode(&encode(false));
        let recs: Vec<&DecMsg> =
            msgs.iter().filter(|m| m.global == p::MSG_RECORD).collect();
        assert_eq!(recs.len(), 15, "one record per sample");
        // unix → FIT epoch conversion on the very first record
        assert_eq!(recs[0].uint(p::RECORD_TIMESTAMP), TS0);
        // 1 Hz within each active span, 5 s hole at the pause
        for (i, r) in recs.iter().enumerate() {
            let expect = if i < 10 { TS0 + i as u64 } else { TS0 + 5 + i as u64 };
            assert_eq!(r.uint(p::RECORD_TIMESTAMP), expect, "record {i}");
        }
        // content
        assert_eq!(recs[0].uint(p::RECORD_POWER), 200);
        assert_eq!(recs[9].uint(p::RECORD_POWER), 209);
        assert_eq!(recs[0].uint(p::RECORD_CADENCE), 90);
        assert_eq!(recs[0].uint(p::RECORD_HEART_RATE), 140);
    }

    #[test]
    fn absent_hr_encodes_invalid_0xff() {
        let msgs = decode(&encode(false));
        let recs: Vec<&DecMsg> =
            msgs.iter().filter(|m| m.global == p::MSG_RECORD).collect();
        // t=3 sample has hr: None
        assert_eq!(recs[3].uint(p::RECORD_TIMESTAMP), TS0 + 3);
        assert_eq!(recs[3].raw(p::RECORD_HEART_RATE).unwrap(), &[0xFF]);
    }

    #[test]
    fn pause_stop_start_pair_interleaved_chronologically() {
        let msgs = decode(&encode(false));
        let events: Vec<(usize, &DecMsg)> = msgs
            .iter()
            .enumerate()
            .filter(|(_, m)| m.global == p::MSG_EVENT)
            .collect();
        assert_eq!(events.len(), 3, "start + stop/start pair");

        let (start_i, start) = events[0];
        assert_eq!(start.uint(p::EVENT_EVENT), 0, "timer");
        assert_eq!(start.uint(p::EVENT_EVENT_TYPE), 0, "start");
        assert_eq!(start.uint(p::EVENT_TIMESTAMP), TS0);

        let (stop_i, stop) = events[1];
        assert_eq!(stop.uint(p::EVENT_EVENT), 0);
        assert_eq!(stop.uint(p::EVENT_EVENT_TYPE), 4, "stop_all");
        assert_eq!(stop.uint(p::EVENT_TIMESTAMP), TS0 + 9, "pause at 9.5 s");

        let (resume_i, resume) = events[2];
        assert_eq!(resume.uint(p::EVENT_EVENT_TYPE), 0, "start");
        assert_eq!(resume.uint(p::EVENT_TIMESTAMP), TS0 + 14, "resume at 14.5 s");

        // Chronological interleave: start event before every record; the
        // stop/start pair sits between the last pre-pause record (ts TS0+9)
        // and the first post-pause record (ts TS0+15).
        let rec_idx: Vec<(usize, u64)> = msgs
            .iter()
            .enumerate()
            .filter(|(_, m)| m.global == p::MSG_RECORD)
            .map(|(i, m)| (i, m.uint(p::RECORD_TIMESTAMP)))
            .collect();
        let last_pre = rec_idx.iter().find(|(_, t)| *t == TS0 + 9).unwrap().0;
        let first_post = rec_idx.iter().find(|(_, t)| *t == TS0 + 15).unwrap().0;
        assert!(start_i < rec_idx[0].0);
        assert!(last_pre < stop_i && stop_i < resume_i && resume_i < first_post);
    }

    #[test]
    fn lap_messages() {
        let msgs = decode(&encode(false));
        let laps_dec: Vec<&DecMsg> =
            msgs.iter().filter(|m| m.global == p::MSG_LAP).collect();
        assert_eq!(laps_dec.len(), 2);

        let l0 = laps_dec[0];
        assert_eq!(l0.uint(p::LAP_MESSAGE_INDEX), 0);
        assert_eq!(l0.uint(p::LAP_START_TIME), TS0);
        assert_eq!(l0.uint(p::LAP_TIMESTAMP), TS0 + 5);
        assert_eq!(l0.uint(p::LAP_TOTAL_ELAPSED_TIME), 5_000, "s × 1000");
        assert_eq!(l0.uint(p::LAP_TOTAL_TIMER_TIME), 5_000);
        assert_eq!(l0.uint(p::LAP_AVG_POWER), 202);
        assert_eq!(l0.uint(p::LAP_MAX_POWER), 204);
        assert_eq!(l0.uint(p::LAP_AVG_HEART_RATE), 140);
        assert_eq!(l0.uint(p::LAP_AVG_CADENCE), 90);
        assert_eq!(l0.uint(p::LAP_TOTAL_CALORIES), 1);

        let l1 = laps_dec[1];
        assert_eq!(l1.uint(p::LAP_MESSAGE_INDEX), 1);
        assert_eq!(l1.uint(p::LAP_START_TIME), TS0 + 5);
        assert_eq!(l1.uint(p::LAP_TIMESTAMP), TS0 + 19);
        assert_eq!(l1.uint(p::LAP_TOTAL_ELAPSED_TIME), 14_500);
        assert_eq!(l1.uint(p::LAP_TOTAL_TIMER_TIME), 9_500, "pause excluded");
        assert_eq!(l1.uint(p::LAP_MAX_HEART_RATE), 145);
    }

    #[test]
    fn session_message_sport_and_scaled_totals() {
        let msgs = decode(&encode(false));
        let s = msgs.iter().find(|m| m.global == p::MSG_SESSION).unwrap();
        assert_eq!(s.uint(p::SESSION_SPORT), 2, "cycling");
        assert_eq!(s.uint(p::SESSION_SUB_SPORT), 6, "indoor_cycling");
        assert_eq!(s.uint(p::SESSION_START_TIME), TS0);
        assert_eq!(s.uint(p::SESSION_TIMESTAMP), TS0 + 20);
        assert_eq!(s.uint(p::SESSION_TOTAL_ELAPSED_TIME), 20_000);
        assert_eq!(s.uint(p::SESSION_TOTAL_TIMER_TIME), 15_000);
        assert_eq!(s.uint(p::SESSION_AVG_POWER), 205);
        assert_eq!(s.uint(p::SESSION_MAX_POWER), 210);
        assert_eq!(s.uint(p::SESSION_AVG_HEART_RATE), 142);
        assert_eq!(s.uint(p::SESSION_MAX_HEART_RATE), 145);
        assert_eq!(s.uint(p::SESSION_AVG_CADENCE), 90);
        assert_eq!(s.uint(p::SESSION_TOTAL_CALORIES), 3, "kJ ≈ kcal");
        assert_eq!(s.uint(p::SESSION_NUM_LAPS), 2);
        assert_eq!(s.uint(p::SESSION_FIRST_LAP_INDEX), 0);
        assert_eq!(s.uint(p::SESSION_NORMALIZED_POWER), 207);
        assert_eq!(s.uint(p::SESSION_TRAINING_STRESS_SCORE), 29, "2.86 × 10 rounded");
        assert_eq!(s.uint(p::SESSION_INTENSITY_FACTOR), 828, "0.828 × 1000");
        assert_eq!(s.uint(p::SESSION_THRESHOLD_POWER), 250, "FTP");
    }

    #[test]
    fn activity_message() {
        let msgs = decode(&encode(false));
        let a = msgs.last().unwrap();
        assert_eq!(a.global, p::MSG_ACTIVITY);
        assert_eq!(a.uint(p::ACTIVITY_TIMESTAMP), TS0 + 20);
        assert_eq!(a.uint(p::ACTIVITY_TOTAL_TIMER_TIME), 15_000);
        assert_eq!(a.uint(p::ACTIVITY_NUM_SESSIONS), 1);
        assert_eq!(a.uint(p::ACTIVITY_TYPE), 0, "manual");
        assert_eq!(a.uint(p::ACTIVITY_EVENT), 26, "activity");
        assert_eq!(a.uint(p::ACTIVITY_EVENT_TYPE), 1, "stop");
        assert_eq!(a.uint(p::ACTIVITY_LOCAL_TIMESTAMP), TS0 + 20);
    }

    // -- record_distance toggle ---------------------------------------------

    #[test]
    fn record_distance_off_emits_no_speed_or_distance() {
        let msgs = decode(&encode(false));
        for m in msgs.iter().filter(|m| m.global == p::MSG_RECORD) {
            assert!(!m.has(p::RECORD_SPEED), "no speed field");
            assert!(!m.has(p::RECORD_DISTANCE), "no distance field");
        }
    }

    #[test]
    fn record_distance_on_emits_monotonic_distance_and_speed() {
        let msgs = decode(&encode(true));
        let recs: Vec<&DecMsg> =
            msgs.iter().filter(|m| m.global == p::MSG_RECORD).collect();
        assert_eq!(recs.len(), 15);
        let mut prev = 0u64;
        for r in &recs {
            let speed = r.uint(p::RECORD_SPEED);
            let dist = r.uint(p::RECORD_DISTANCE);
            assert!(speed > 0, "positive speed at >0 W");
            assert!(dist >= prev, "distance monotonic");
            assert!(dist > 0);
            prev = dist;
        }
        // ~200 W on the flat should be plausibly 7–13 m/s
        let v0 = recs[0].uint(p::RECORD_SPEED) as f64 / 1000.0;
        assert!((5.0..15.0).contains(&v0), "plausible speed, got {v0} m/s");
    }

    // -- edge cases ---------------------------------------------------------

    #[test]
    fn pre_fit_epoch_start_is_an_error() {
        let mut h = header();
        h.started_unix_ms = 1000; // 1970
        let s = samples();
        let e = events();
        let l = laps();
        let t = totals();
        let err = encode_activity(&FitRide {
            header: &h,
            samples: &s,
            events: &e,
            laps: &l,
            totals: &t,
            record_distance: false,
        });
        assert!(err.is_err());
    }

    #[test]
    fn empty_ride_encodes_and_validates() {
        let h = JournalHeader {
            trainer: None,
            hrm: None,
            ..header()
        };
        let t = SessionTotals {
            elapsed_s: 0,
            timer_s: 0,
            avg_power: None,
            max_power: None,
            np: None,
            if_: None,
            tss: None,
            avg_hr: None,
            max_hr: None,
            avg_cadence: None,
            kj: 0,
        };
        let buf = encode_activity(&FitRide {
            header: &h,
            samples: &[],
            events: &[],
            laps: &[],
            totals: &t,
            record_distance: false,
        })
        .unwrap();
        assert_eq!(crc::checksum(&buf), 0);
        let msgs = decode(&buf);
        let globals: Vec<u16> = msgs.iter().map(|m| m.global).collect();
        assert_eq!(
            globals,
            vec![
                p::MSG_FILE_ID,
                p::MSG_DEVICE_INFO,
                p::MSG_EVENT,
                p::MSG_SESSION,
                p::MSG_ACTIVITY
            ]
        );
        let s = msgs.iter().find(|m| m.global == p::MSG_SESSION).unwrap();
        assert_eq!(s.uint(p::SESSION_NUM_LAPS), 0);
        assert_eq!(s.raw(p::SESSION_NORMALIZED_POWER).unwrap(), &[0xFF, 0xFF]);
        assert_eq!(s.raw(p::SESSION_AVG_HEART_RATE).unwrap(), &[0xFF]);
    }

    #[test]
    fn absent_power_and_cadence_encode_invalid() {
        let h = header();
        let s = vec![Sample {
            t_ms: 0,
            power: None,
            cadence: None,
            hr: None,
            target: None,
        }];
        let t = totals();
        let buf = encode_activity(&FitRide {
            header: &h,
            samples: &s,
            events: &[],
            laps: &[],
            totals: &t,
            record_distance: false,
        })
        .unwrap();
        let msgs = decode(&buf);
        let r = msgs.iter().find(|m| m.global == p::MSG_RECORD).unwrap();
        assert_eq!(r.raw(p::RECORD_POWER).unwrap(), &[0xFF, 0xFF]);
        assert_eq!(r.raw(p::RECORD_CADENCE).unwrap(), &[0xFF]);
        assert_eq!(r.raw(p::RECORD_HEART_RATE).unwrap(), &[0xFF]);
    }

    #[test]
    fn long_device_name_truncated_on_char_boundary() {
        let mut h = header();
        // 3-byte chars; 31-byte budget cuts mid-char without the boundary fix
        h.trainer = Some("トレーナー très long name ééééééé".into());
        let s = samples();
        let t = totals();
        let buf = encode_activity(&FitRide {
            header: &h,
            samples: &s,
            events: &[],
            laps: &[],
            totals: &t,
            record_distance: false,
        })
        .unwrap();
        let msgs = decode(&buf);
        let devs: Vec<&DecMsg> =
            msgs.iter().filter(|m| m.global == p::MSG_DEVICE_INFO).collect();
        // decoding asserts valid UTF-8; also must fit in the fixed field
        let name = devs[1].string(p::DEVICE_INFO_PRODUCT_NAME);
        assert!(name.len() < p::PRODUCT_NAME_SIZE as usize);
        assert!(h.trainer.as_ref().unwrap().starts_with(&name));
    }

    #[test]
    fn one_definition_per_global() {
        let buf = encode(false);
        let body = &buf[14..buf.len() - 2];
        let mut def_globals = Vec::new();
        let mut defs: Vec<Option<Vec<(u8, u8)>>> = (0..16).map(|_| None).collect();
        let mut i = 0usize;
        while i < body.len() {
            let hdr = body[i];
            i += 1;
            let local = (hdr & 0x0F) as usize;
            if hdr & 0x40 != 0 {
                let global = u16::from_le_bytes([body[i + 2], body[i + 3]]);
                let n = body[i + 4] as usize;
                i += 5;
                let mut fields = Vec::new();
                for _ in 0..n {
                    fields.push((body[i], body[i + 1]));
                    i += 3;
                }
                def_globals.push(global);
                defs[local] = Some(fields);
            } else {
                let fields = defs[local].as_ref().unwrap();
                i += fields.iter().map(|(_, s)| *s as usize).sum::<usize>();
            }
        }
        let mut sorted = def_globals.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), def_globals.len(), "each global defined once");
        assert_eq!(def_globals.len(), 7, "7 definitions");
    }
}
