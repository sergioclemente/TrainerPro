//! FIT container encoding: 14-byte header, definition + data records
//! (little-endian), trailing CRC-16. Message sequence:
//! file_id → device_info → user_profile → zones_target → workout →
//! workout_step×N → event(start) → records (1 Hz, with stop/start event
//! pairs interleaved chronologically at pauses) → laps → session → activity.
//! Timestamps: unix_s − FIT_EPOCH_OFFSET_S.

use super::profile as p;
use super::{crc, FitActivity, FitError};
use crate::consts::FIT_EPOCH_OFFSET_S;
use crate::journal::SessionEventKind;
use crate::model::{PowerTarget, StepRole, WorkoutSegment};

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

pub fn encode_activity(activity: &FitActivity) -> Result<Vec<u8>, FitError> {
    if let Some(motion) = activity.motion {
        if motion.records.len() != activity.samples.len()
            || motion.activity_segments.len() != activity.laps.len()
        {
            return Err(FitError::Encode(
                "motion trace does not match activity".into(),
            ));
        }
    }
    let h = activity.header;
    let start_ms = h.started_unix_ms;
    let ts_start = fit_ts(start_ms)?;

    let mut body: Vec<u8> = Vec::with_capacity(1024 + activity.samples.len() * 12);

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
    put_u32(&mut body, serial_from(&h.workout_session_id));
    put_u32(&mut body, ts_start);

    // -- 2. device_info ×1–3 ------------------------------------------------
    let device_info_fields = [
        f(p::DEVICE_INFO_DEVICE_INDEX, 1, p::BASE_UINT8),
        f(p::DEVICE_INFO_MANUFACTURER, 2, p::BASE_UINT16),
        f(p::DEVICE_INFO_SOFTWARE_VERSION, 2, p::BASE_UINT16),
        f(
            p::DEVICE_INFO_PRODUCT_NAME,
            p::PRODUCT_NAME_SIZE,
            p::BASE_STRING,
        ),
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

    // -- 2b. user_profile + zones_target: the rider the metrics are scaled to
    if h.weight_kg > 0.0 {
        let user_profile_fields = [f(p::USER_PROFILE_WEIGHT, 2, p::BASE_UINT16)];
        write_definition(
            &mut body,
            p::LOCAL_USER_PROFILE,
            p::MSG_USER_PROFILE,
            &user_profile_fields,
        );
        body.push(p::LOCAL_USER_PROFILE);
        put_u16(&mut body, scale_u16(h.weight_kg, p::WEIGHT_SCALE));
    }
    if h.ftp_w > 0 {
        let zones_target_fields = [
            f(
                p::ZONES_TARGET_FUNCTIONAL_THRESHOLD_POWER,
                2,
                p::BASE_UINT16,
            ),
            f(p::ZONES_TARGET_PWR_CALC_TYPE, 1, p::BASE_ENUM),
        ];
        write_definition(
            &mut body,
            p::LOCAL_ZONES_TARGET,
            p::MSG_ZONES_TARGET,
            &zones_target_fields,
        );
        body.push(p::LOCAL_ZONES_TARGET);
        put_u16(&mut body, h.ftp_w);
        body.push(p::PWR_CALC_TYPE_PERCENT_FTP);
    }

    // -- 2c. workout + workout_step×N: the plan, one step per executable
    //        segment (repeats arrive pre-expanded), so lap.wkt_step_index is
    //        the journal's workout segment index.
    let step_roles: Option<Vec<StepRole>> = activity.workout.map(|w| w.step_roles(h.ftp_w));
    if let (Some(workout), Some(roles)) = (activity.workout, step_roles.as_ref()) {
        let workout_fields = [
            f(p::WORKOUT_SPORT, 1, p::BASE_ENUM),
            f(p::WORKOUT_SUB_SPORT, 1, p::BASE_ENUM),
            f(p::WORKOUT_NUM_VALID_STEPS, 2, p::BASE_UINT16),
            f(p::WORKOUT_WKT_NAME, p::WKT_NAME_SIZE, p::BASE_STRING),
        ];
        write_definition(&mut body, p::LOCAL_WORKOUT, p::MSG_WORKOUT, &workout_fields);
        body.push(p::LOCAL_WORKOUT);
        body.push(p::SPORT_CYCLING);
        body.push(p::SUB_SPORT_INDOOR_CYCLING);
        put_u16(&mut body, count_u16(workout.segments.len())?);
        put_string(&mut body, &h.workout_name, p::WKT_NAME_SIZE as usize);

        let step_fields = [
            f(p::WORKOUT_STEP_MESSAGE_INDEX, 2, p::BASE_UINT16),
            f(p::WORKOUT_STEP_NAME, p::WKT_STEP_NAME_SIZE, p::BASE_STRING),
            f(p::WORKOUT_STEP_DURATION_TYPE, 1, p::BASE_ENUM),
            f(p::WORKOUT_STEP_DURATION_VALUE, 4, p::BASE_UINT32),
            f(p::WORKOUT_STEP_TARGET_TYPE, 1, p::BASE_ENUM),
            f(p::WORKOUT_STEP_TARGET_VALUE, 4, p::BASE_UINT32),
            f(p::WORKOUT_STEP_CUSTOM_TARGET_LOW, 4, p::BASE_UINT32),
            f(p::WORKOUT_STEP_CUSTOM_TARGET_HIGH, 4, p::BASE_UINT32),
            f(p::WORKOUT_STEP_INTENSITY, 1, p::BASE_ENUM),
            f(p::WORKOUT_STEP_SECONDARY_TARGET_TYPE, 1, p::BASE_ENUM),
            f(p::WORKOUT_STEP_SECONDARY_TARGET_VALUE, 4, p::BASE_UINT32),
            f(
                p::WORKOUT_STEP_SECONDARY_CUSTOM_TARGET_LOW,
                4,
                p::BASE_UINT32,
            ),
            f(
                p::WORKOUT_STEP_SECONDARY_CUSTOM_TARGET_HIGH,
                4,
                p::BASE_UINT32,
            ),
        ];
        write_definition(
            &mut body,
            p::LOCAL_WORKOUT_STEP,
            p::MSG_WORKOUT_STEP,
            &step_fields,
        );
        for (i, (segment, role)) in workout.segments.iter().zip(roles).enumerate() {
            body.push(p::LOCAL_WORKOUT_STEP);
            put_u16(&mut body, count_u16(i)?);
            put_string(
                &mut body,
                &step_name(segment, *role),
                p::WKT_STEP_NAME_SIZE as usize,
            );
            body.push(p::WKT_STEP_DURATION_TIME);
            put_u32(&mut body, ms_u32(u64::from(segment.duration_s()) * 1000)?);
            match step_power_range(segment, h.ftp_w) {
                Some((low, high)) => {
                    body.push(p::WKT_STEP_TARGET_POWER);
                    put_u32(&mut body, p::WKT_STEP_TARGET_CUSTOM);
                    put_u32(&mut body, low);
                    put_u32(&mut body, high);
                }
                None => {
                    body.push(p::WKT_STEP_TARGET_OPEN);
                    put_u32(&mut body, p::INVALID_UINT32);
                    put_u32(&mut body, p::INVALID_UINT32);
                    put_u32(&mut body, p::INVALID_UINT32);
                }
            }
            body.push(step_intensity(*role));
            match step_cadence_rpm(segment) {
                Some(rpm) => {
                    body.push(p::WKT_STEP_TARGET_CADENCE);
                    put_u32(&mut body, p::WKT_STEP_TARGET_CUSTOM);
                    put_u32(&mut body, u32::from(rpm));
                    put_u32(&mut body, u32::from(rpm));
                }
                None => {
                    body.push(p::INVALID_ENUM);
                    put_u32(&mut body, p::INVALID_UINT32);
                    put_u32(&mut body, p::INVALID_UINT32);
                    put_u32(&mut body, p::INVALID_UINT32);
                }
            }
        }
    }

    // -- 3. event: timer start ----------------------------------------------
    let event_fields = [
        f(p::EVENT_TIMESTAMP, 4, p::BASE_UINT32),
        f(p::EVENT_EVENT, 1, p::BASE_ENUM),
        f(p::EVENT_EVENT_TYPE, 1, p::BASE_ENUM),
    ];
    write_definition(&mut body, p::LOCAL_EVENT, p::MSG_EVENT, &event_fields);
    let start_t_ms = activity
        .events
        .iter()
        .find(|e| e.kind == SessionEventKind::Start)
        .map(|e| e.t_ms)
        .unwrap_or(0);
    write_timer_event(
        &mut body,
        fit_ts(start_ms + start_t_ms)?,
        p::EVENT_TYPE_START,
    );

    // -- 4. records (1 Hz), pause stop/start pairs interleaved --------------
    let mut record_fields = vec![
        f(p::RECORD_TIMESTAMP, 4, p::BASE_UINT32),
        f(p::RECORD_HEART_RATE, 1, p::BASE_UINT8),
        f(p::RECORD_CADENCE, 1, p::BASE_UINT8),
        f(p::RECORD_POWER, 2, p::BASE_UINT16),
    ];
    if activity.motion.is_some() {
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
    let mut items: Vec<(u64, u8, Item)> = activity
        .samples
        .iter()
        .enumerate()
        .map(|(i, s)| (s.t_ms, 1u8, Item::Sample(i)))
        .collect();
    for e in activity.events {
        match e.kind {
            SessionEventKind::Pause => items.push((e.t_ms, 2, Item::Pause(e.t_ms))),
            SessionEventKind::Resume => items.push((e.t_ms, 0, Item::Resume(e.t_ms))),
            _ => {}
        }
    }
    items.sort_by_key(|(t, rank, _)| (*t, *rank));

    for (_, _, item) in &items {
        match item {
            Item::Sample(i) => {
                let s = &activity.samples[*i];
                body.push(p::LOCAL_RECORD);
                put_u32(&mut body, fit_ts(start_ms + s.t_ms)?);
                body.push(opt_u8(s.heart_rate_bpm));
                body.push(opt_u8(s.cadence_rpm));
                put_u16(&mut body, s.power_w.unwrap_or(p::INVALID_UINT16));
                if let Some(motion) = activity.motion {
                    let point = motion.records[*i];
                    put_u16(&mut body, scale_u16(point.speed_m_s, p::SPEED_SCALE));
                    put_u32(&mut body, scale_u32(point.distance_m, p::DISTANCE_SCALE));
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
    let mut lap_fields = vec![
        f(p::LAP_MESSAGE_INDEX, 2, p::BASE_UINT16),
        f(p::LAP_TIMESTAMP, 4, p::BASE_UINT32),
        f(p::LAP_START_TIME, 4, p::BASE_UINT32),
        f(p::LAP_TOTAL_ELAPSED_TIME, 4, p::BASE_UINT32),
        f(p::LAP_TOTAL_TIMER_TIME, 4, p::BASE_UINT32),
        f(p::LAP_TOTAL_CALORIES, 2, p::BASE_UINT16),
        f(p::LAP_AVG_HEART_RATE, 1, p::BASE_UINT8),
        f(p::LAP_MAX_HEART_RATE, 1, p::BASE_UINT8),
        f(p::LAP_AVG_CADENCE, 1, p::BASE_UINT8),
        f(p::LAP_MAX_CADENCE, 1, p::BASE_UINT8),
        f(p::LAP_AVG_POWER, 2, p::BASE_UINT16),
        f(p::LAP_MAX_POWER, 2, p::BASE_UINT16),
        f(p::LAP_NORMALIZED_POWER, 2, p::BASE_UINT16),
        f(p::LAP_TOTAL_WORK, 4, p::BASE_UINT32),
        f(p::LAP_SPORT, 1, p::BASE_ENUM),
        f(p::LAP_SUB_SPORT, 1, p::BASE_ENUM),
        f(p::LAP_LAP_TRIGGER, 1, p::BASE_ENUM),
        f(p::LAP_WKT_STEP_INDEX, 2, p::BASE_UINT16),
        f(p::LAP_INTENSITY, 1, p::BASE_ENUM),
    ];
    if activity.motion.is_some() {
        lap_fields.extend([
            f(p::LAP_TOTAL_DISTANCE, 4, p::BASE_UINT32),
            f(p::LAP_AVG_SPEED, 2, p::BASE_UINT16),
            f(p::LAP_MAX_SPEED, 2, p::BASE_UINT16),
        ]);
    }
    write_definition(&mut body, p::LOCAL_LAP, p::MSG_LAP, &lap_fields);
    let mut lap_distance_m = 0.0;
    let mut encoded_lap_distance = 0;
    for (i, lap) in activity.laps.iter().enumerate() {
        body.push(p::LOCAL_LAP);
        put_u16(&mut body, i as u16);
        put_u32(&mut body, fit_ts(start_ms + lap.end_ms)?);
        put_u32(&mut body, fit_ts(start_ms + lap.start_ms)?);
        put_u32(&mut body, ms_u32(lap.end_ms - lap.start_ms)?); // scale 1000 = ms
        put_u32(&mut body, ms_u32(lap.timer_ms)?);
        put_u16(&mut body, lap.calories_kcal);
        body.push(opt_u8(lap.average_heart_rate_bpm));
        body.push(opt_u8(lap.max_heart_rate_bpm));
        body.push(opt_u8(lap.average_cadence_rpm));
        body.push(opt_u8(lap.max_cadence_rpm));
        put_u16(&mut body, lap.average_power_w.unwrap_or(p::INVALID_UINT16));
        put_u16(&mut body, lap.max_power_w.unwrap_or(p::INVALID_UINT16));
        put_u16(
            &mut body,
            lap.normalized_power_w.unwrap_or(p::INVALID_UINT16),
        );
        put_u32(&mut body, lap.work_j);
        body.push(p::SPORT_CYCLING);
        body.push(p::SUB_SPORT_INDOOR_CYCLING);
        body.push(if i + 1 == activity.laps.len() {
            p::LAP_TRIGGER_SESSION_END
        } else {
            p::LAP_TRIGGER_FITNESS_EQUIPMENT
        });
        // Step links only point into steps this file describes.
        let step_role = step_roles
            .as_ref()
            .zip(lap.workout_segment_index)
            .and_then(|(roles, index)| roles.get(index).map(|role| (index, *role)));
        match step_role {
            Some((index, role)) => {
                put_u16(&mut body, count_u16(index)?);
                body.push(step_intensity(role));
            }
            None => {
                put_u16(&mut body, p::INVALID_UINT16);
                body.push(p::INVALID_ENUM);
            }
        }
        if let Some(motion) = activity.motion {
            let summary = motion.activity_segments[i];
            lap_distance_m += summary.distance_m;
            let cumulative = scale_u32(lap_distance_m, p::DISTANCE_SCALE);
            // Difference rounded boundaries, so all lap distances sum exactly.
            put_u32(&mut body, cumulative.saturating_sub(encoded_lap_distance));
            encoded_lap_distance = cumulative;
            put_u16(
                &mut body,
                summary
                    .average_speed_m_s(lap.timer_ms)
                    .map(|speed| scale_u16(speed, p::SPEED_SCALE))
                    .unwrap_or(p::INVALID_UINT16),
            );
            put_u16(&mut body, scale_u16(summary.max_speed_m_s, p::SPEED_SCALE));
        }
    }

    // -- 6. session ---------------------------------------------------------
    let t = activity.totals;
    let elapsed_ms = activity
        .laps
        .last()
        .map(|lap| lap.end_ms)
        .unwrap_or(u64::from(t.elapsed_s) * 1000);
    let timer_ms: u64 = activity.laps.iter().map(|lap| lap.timer_ms).sum();
    let ts_end = fit_ts(start_ms + elapsed_ms)?;
    let mut session_fields = vec![
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
        f(p::SESSION_MAX_CADENCE, 1, p::BASE_UINT8),
        f(p::SESSION_AVG_POWER, 2, p::BASE_UINT16),
        f(p::SESSION_MAX_POWER, 2, p::BASE_UINT16),
        f(p::SESSION_TOTAL_WORK, 4, p::BASE_UINT32),
        f(p::SESSION_FIRST_LAP_INDEX, 2, p::BASE_UINT16),
        f(p::SESSION_NUM_LAPS, 2, p::BASE_UINT16),
        f(p::SESSION_NORMALIZED_POWER, 2, p::BASE_UINT16),
        f(p::SESSION_TRAINING_STRESS_SCORE, 2, p::BASE_UINT16),
        f(p::SESSION_INTENSITY_FACTOR, 2, p::BASE_UINT16),
        f(p::SESSION_THRESHOLD_POWER, 2, p::BASE_UINT16),
    ];
    if activity.motion.is_some() {
        session_fields.extend([
            f(p::SESSION_TOTAL_DISTANCE, 4, p::BASE_UINT32),
            f(p::SESSION_AVG_SPEED, 2, p::BASE_UINT16),
            f(p::SESSION_MAX_SPEED, 2, p::BASE_UINT16),
        ]);
    }
    write_definition(&mut body, p::LOCAL_SESSION, p::MSG_SESSION, &session_fields);
    body.push(p::LOCAL_SESSION);
    put_u32(&mut body, ts_end);
    put_u32(&mut body, ts_start);
    body.push(p::SPORT_CYCLING);
    body.push(p::SUB_SPORT_INDOOR_CYCLING);
    put_u32(&mut body, ms_u32(elapsed_ms)?);
    put_u32(&mut body, ms_u32(timer_ms)?);
    put_u16(&mut body, t.work_kj.min(u32::from(u16::MAX - 1)) as u16); // kJ ≈ kcal
    body.push(opt_u8(t.average_heart_rate_bpm));
    body.push(opt_u8(t.max_heart_rate_bpm));
    body.push(opt_u8(t.average_cadence_rpm));
    body.push(opt_u8(t.max_cadence_rpm));
    put_u16(&mut body, t.average_power_w.unwrap_or(p::INVALID_UINT16));
    put_u16(&mut body, t.max_power_w.unwrap_or(p::INVALID_UINT16));
    put_u32(&mut body, joules_u32(t.work_j));
    put_u16(&mut body, 0); // first_lap_index
    put_u16(&mut body, activity.laps.len() as u16);
    put_u16(&mut body, t.normalized_power_w.unwrap_or(p::INVALID_UINT16));
    put_u16(
        &mut body,
        t.training_stress_score
            .map(|v| scale_u16(v, p::TSS_SCALE))
            .unwrap_or(p::INVALID_UINT16),
    );
    put_u16(
        &mut body,
        t.intensity_factor
            .map(|v| scale_u16(v, p::IF_SCALE))
            .unwrap_or(p::INVALID_UINT16),
    );
    put_u16(&mut body, h.ftp_w);
    if let Some(motion) = activity.motion {
        put_u32(
            &mut body,
            scale_u32(motion.session.distance_m, p::DISTANCE_SCALE),
        );
        put_u16(
            &mut body,
            motion
                .session
                .average_speed_m_s(timer_ms)
                .map(|speed| scale_u16(speed, p::SPEED_SCALE))
                .unwrap_or(p::INVALID_UINT16),
        );
        put_u16(
            &mut body,
            scale_u16(motion.session.max_speed_m_s, p::SPEED_SCALE),
        );
    }

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
    write_definition(
        &mut body,
        p::LOCAL_ACTIVITY,
        p::MSG_ACTIVITY,
        &activity_fields,
    );
    body.push(p::LOCAL_ACTIVITY);
    put_u32(&mut body, ts_end);
    put_u32(&mut body, ms_u32(timer_ms)?);
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

/// Joules into a uint32 total_work field; a ride past the field's range
/// pins to the largest valid value rather than wrapping.
fn joules_u32(joules: u64) -> u32 {
    joules.min(u64::from(u32::MAX - 1)) as u32
}

/// A step or lap index into a uint16 message_index field.
fn count_u16(index: usize) -> Result<u16, FitError> {
    u16::try_from(index)
        .ok()
        .filter(|v| *v != p::INVALID_UINT16)
        .ok_or_else(|| FitError::Encode("workout step index exceeds uint16".into()))
}

/// Step title for the plan: role, duration and target, e.g.
/// "Interval · 5:00 @ 105%" or "Warm-up · 10:00 @ 40→60%".
fn step_name(segment: &WorkoutSegment, role: StepRole) -> String {
    let duration = fmt_duration(segment.duration_s());
    let target = match segment {
        WorkoutSegment::Steady { power, .. } => format!(" @ {}", power_text(power)),
        WorkoutSegment::Ramp { start, end, .. } => {
            format!(" @ {}→{}", power_text(start), power_text(end))
        }
        WorkoutSegment::FreeRide { .. } => String::new(),
    };
    format!("{} · {duration}{target}", role.label())
}

fn power_text(target: &PowerTarget) -> String {
    match target {
        PowerTarget::PercentFtp(frac) => format!("{}%", (frac * 100.0).round()),
        PowerTarget::Watts(watts) => format!("{watts} W"),
    }
}

/// `m:ss`, or `h:mm:ss` from one hour.
fn fmt_duration(total_s: u32) -> String {
    let (h, m, s) = (total_s / 3600, (total_s / 60) % 60, total_s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// FIT workout_power for a target: % FTP as-is, watts offset by 1000.
fn workout_power(target: &PowerTarget) -> u32 {
    match target {
        PowerTarget::PercentFtp(frac) => (frac * 100.0).round() as u32,
        PowerTarget::Watts(watts) => p::WORKOUT_POWER_WATTS_OFFSET + u32::from(*watts),
    }
}

/// Custom power range (low, high) in workout_power units; `None` for an
/// open step. A ramp spans its endpoints; one mixing % FTP and watts is
/// resolved to watts at the ride's FTP so both ends share a unit.
fn step_power_range(segment: &WorkoutSegment, ftp: u16) -> Option<(u32, u32)> {
    let (start, end) = match segment {
        WorkoutSegment::Steady { power, .. } => (power, power),
        WorkoutSegment::Ramp { start, end, .. } => (start, end),
        WorkoutSegment::FreeRide { .. } => return None,
    };
    let (a, b) = match (start, end) {
        (PowerTarget::PercentFtp(_), PowerTarget::PercentFtp(_))
        | (PowerTarget::Watts(_), PowerTarget::Watts(_)) => {
            (workout_power(start), workout_power(end))
        }
        _ => (
            p::WORKOUT_POWER_WATTS_OFFSET + u32::from(start.resolve(ftp, 1.0)),
            p::WORKOUT_POWER_WATTS_OFFSET + u32::from(end.resolve(ftp, 1.0)),
        ),
    };
    Some((a.min(b), a.max(b)))
}

fn step_cadence_rpm(segment: &WorkoutSegment) -> Option<u16> {
    match segment {
        WorkoutSegment::Steady { cadence_rpm, .. }
        | WorkoutSegment::Ramp { cadence_rpm, .. }
        | WorkoutSegment::FreeRide { cadence_rpm, .. } => *cadence_rpm,
    }
}

/// FIT intensity for a step role, shared by the step and the laps riding it.
fn step_intensity(role: StepRole) -> u8 {
    match role {
        StepRole::WarmUp => p::INTENSITY_WARMUP,
        StepRole::CoolDown => p::INTENSITY_COOLDOWN,
        StepRole::Recovery => p::INTENSITY_REST,
        StepRole::Steady | StepRole::Interval | StepRole::Ramp => p::INTENSITY_ACTIVE,
        StepRole::FreeRide => p::INTENSITY_OTHER,
    }
}

fn scale_u16(v: f64, scale: f64) -> u16 {
    (v * scale).round().clamp(0.0, f64::from(u16::MAX - 1)) as u16
}

fn scale_u32(v: f64, scale: f64) -> u32 {
    (v * scale).round().clamp(0.0, f64::from(u32::MAX - 1)) as u32
}

/// Deterministic per-install serial (FNV-1a over the workout-session id;
/// uint32z, so never 0). tp-core has no persistence, so the session id stands
/// in for a true per-install random serial.
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
    let minor = if minor < 10 {
        minor * 10
    } else {
        minor.min(99)
    };
    major
        .saturating_mul(100)
        .saturating_add(minor)
        .min(u16::MAX - 1)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit::FitActivity;
    use crate::journal::{ActivitySegment, JournalHeader, Sample, SessionEvent, SessionEventKind};
    use crate::metrics::SessionTotals;
    use crate::model::ExecutableWorkout;

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
        let data_size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
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

    // -- synthetic activity -------------------------------------------------

    /// 2026-01-01T00:00:00Z.
    const START_UNIX_S: u64 = 1_767_225_600;
    const TS0: u64 = START_UNIX_S - 631_065_600; // FIT epoch conversion

    fn header() -> JournalHeader {
        JournalHeader {
            workout_session_id: "session-abc".into(),
            workout_definition_id: "definition-abc".into(),
            scheduled_workout_id: None,
            workout_definition_snapshot_json: r#"{"format":"TPW","version":1}"#.into(),
            started_unix_ms: START_UNIX_S * 1000,
            workout_name: "2x20".into(),
            ftp_w: 250,
            weight_kg: 75.0,
            record_distance: false,
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
                power_w: Some(200 + t as u16),
                cadence_rpm: Some(90),
                heart_rate_bpm: if t == 3 { None } else { Some(140) },
                target_power_w: Some(200),
                target_cadence_rpm: Some(90),
            });
        }
        for t in 15u64..20 {
            v.push(Sample {
                t_ms: t * 1000,
                power_w: Some(210),
                cadence_rpm: Some(92),
                heart_rate_bpm: Some(145),
                target_power_w: Some(210),
                target_cadence_rpm: Some(90),
            });
        }
        v
    }

    fn events() -> Vec<SessionEvent> {
        let e = |t_ms, kind, segment_index| SessionEvent {
            t_ms,
            kind,
            segment_index,
        };
        vec![
            e(0, SessionEventKind::Start, None),
            e(5_000, SessionEventKind::WorkoutSegmentEnd, Some(0)),
            e(9_500, SessionEventKind::Pause, None),
            e(14_500, SessionEventKind::Resume, None),
            e(19_500, SessionEventKind::End, None),
        ]
    }

    /// The plan the fixture rode: a 5 s steady step with a cadence target,
    /// then a 15 s interval; the ride ended inside the second step.
    fn workout() -> ExecutableWorkout {
        ExecutableWorkout {
            name: "2x20".into(),
            description: String::new(),
            segments: vec![
                WorkoutSegment::Steady {
                    duration_s: 5,
                    power: PowerTarget::PercentFtp(0.8),
                    cadence_rpm: Some(90),
                },
                WorkoutSegment::Steady {
                    duration_s: 15,
                    power: PowerTarget::PercentFtp(1.0),
                    cadence_rpm: None,
                },
            ],
            text_events: vec![],
        }
    }

    fn laps() -> Vec<ActivitySegment> {
        vec![
            ActivitySegment {
                start_ms: 0,
                end_ms: 5_000,
                timer_ms: 5_000,
                average_power_w: Some(202),
                max_power_w: Some(204),
                average_heart_rate_bpm: Some(140),
                max_heart_rate_bpm: Some(140),
                average_cadence_rpm: Some(90),
                max_cadence_rpm: Some(90),
                normalized_power_w: Some(202),
                work_j: 1_010,
                calories_kcal: 1,
                workout_segment_index: Some(0),
            },
            ActivitySegment {
                start_ms: 5_000,
                end_ms: 19_500,
                timer_ms: 9_500,
                average_power_w: Some(208),
                max_power_w: Some(210),
                average_heart_rate_bpm: Some(143),
                max_heart_rate_bpm: Some(145),
                average_cadence_rpm: Some(91),
                max_cadence_rpm: Some(92),
                normalized_power_w: Some(208),
                work_j: 2_085,
                calories_kcal: 2,
                workout_segment_index: None,
            },
        ]
    }

    fn totals() -> SessionTotals {
        SessionTotals {
            elapsed_s: 20,
            timer_s: 15,
            average_power_w: Some(205),
            max_power_w: Some(210),
            normalized_power_w: Some(207),
            intensity_factor: Some(0.828),
            training_stress_score: Some(2.86),
            average_heart_rate_bpm: Some(142),
            max_heart_rate_bpm: Some(145),
            average_cadence_rpm: Some(90),
            max_cadence_rpm: Some(92),
            work_j: 3_095,
            work_kj: 3,
        }
    }

    fn encode(record_distance: bool) -> Vec<u8> {
        let mut h = header();
        h.record_distance = record_distance;
        let s = samples();
        let e = events();
        let l = laps();
        let t = totals();
        let w = workout();
        let data = crate::journal::SessionRecording {
            header: h.clone(),
            samples: s.clone(),
            events: e.clone(),
        };
        let motion = record_distance.then(|| crate::motion::replay_motion(&data, &l));
        encode_activity(&FitActivity {
            header: &h,
            samples: &s,
            events: &e,
            laps: &l,
            totals: &t,
            motion: motion.as_ref(),
            workout: Some(&w),
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
        assert_eq!(
            data_size,
            buf.len() - 14 - 2,
            "data size excludes header+CRC"
        );
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
        assert_eq!(
            &globals[4..9],
            &[
                p::MSG_USER_PROFILE,
                p::MSG_ZONES_TARGET,
                p::MSG_WORKOUT,
                p::MSG_WORKOUT_STEP,
                p::MSG_WORKOUT_STEP
            ],
            "rider and plan before the ride"
        );
        assert_eq!(globals[9], p::MSG_EVENT, "timer start event");
        // Middle: records + pause events only.
        let tail_start = globals.len() - 4;
        for g in &globals[10..tail_start] {
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
        assert_ne!(
            fid.uint(p::FILE_ID_SERIAL_NUMBER),
            0,
            "uint32z serial nonzero"
        );
        assert_eq!(fid.uint(p::FILE_ID_TIME_CREATED), TS0);

        let devs: Vec<&DecMsg> = msgs
            .iter()
            .filter(|m| m.global == p::MSG_DEVICE_INFO)
            .collect();
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
        let recs: Vec<&DecMsg> = msgs.iter().filter(|m| m.global == p::MSG_RECORD).collect();
        assert_eq!(recs.len(), 15, "one record per sample");
        // unix → FIT epoch conversion on the very first record
        assert_eq!(recs[0].uint(p::RECORD_TIMESTAMP), TS0);
        // 1 Hz within each active span, 5 s hole at the pause
        for (i, r) in recs.iter().enumerate() {
            let expect = if i < 10 {
                TS0 + i as u64
            } else {
                TS0 + 5 + i as u64
            };
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
        let recs: Vec<&DecMsg> = msgs.iter().filter(|m| m.global == p::MSG_RECORD).collect();
        // The t=3 sample has no heart-rate value.
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
        assert_eq!(
            resume.uint(p::EVENT_TIMESTAMP),
            TS0 + 14,
            "resume at 14.5 s"
        );

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
        let laps_dec: Vec<&DecMsg> = msgs.iter().filter(|m| m.global == p::MSG_LAP).collect();
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
    fn laps_carry_work_np_cadence_and_step_links() {
        let msgs = decode(&encode(false));
        let laps_dec: Vec<&DecMsg> = msgs.iter().filter(|m| m.global == p::MSG_LAP).collect();

        let l0 = laps_dec[0];
        assert_eq!(l0.uint(p::LAP_MAX_CADENCE), 90);
        assert_eq!(l0.uint(p::LAP_NORMALIZED_POWER), 202);
        assert_eq!(l0.uint(p::LAP_TOTAL_WORK), 1_010, "joules");
        assert_eq!(l0.uint(p::LAP_SPORT), 2, "cycling");
        assert_eq!(l0.uint(p::LAP_SUB_SPORT), 6, "indoor_cycling");
        assert_eq!(l0.uint(p::LAP_LAP_TRIGGER), 8, "fitness_equipment");
        assert_eq!(l0.uint(p::LAP_WKT_STEP_INDEX), 0, "rode step 0");
        assert_eq!(l0.uint(p::LAP_INTENSITY), 0, "steady step is active");

        // Ended by stopping the ride: last lap, no step closed it.
        let l1 = laps_dec[1];
        assert_eq!(l1.uint(p::LAP_MAX_CADENCE), 92);
        assert_eq!(l1.uint(p::LAP_TOTAL_WORK), 2_085);
        assert_eq!(l1.uint(p::LAP_LAP_TRIGGER), 7, "session_end");
        assert_eq!(l1.raw(p::LAP_WKT_STEP_INDEX).unwrap(), &[0xFF, 0xFF]);
        assert_eq!(l1.raw(p::LAP_INTENSITY).unwrap(), &[0xFF]);
    }

    #[test]
    fn rider_profile_and_power_zones() {
        let msgs = decode(&encode(false));
        let profile = msgs
            .iter()
            .find(|m| m.global == p::MSG_USER_PROFILE)
            .unwrap();
        assert_eq!(profile.uint(p::USER_PROFILE_WEIGHT), 750, "75.0 kg × 10");
        let zones = msgs
            .iter()
            .find(|m| m.global == p::MSG_ZONES_TARGET)
            .unwrap();
        assert_eq!(zones.uint(p::ZONES_TARGET_FUNCTIONAL_THRESHOLD_POWER), 250);
        assert_eq!(zones.uint(p::ZONES_TARGET_PWR_CALC_TYPE), 1, "percent_ftp");
    }

    #[test]
    fn workout_and_steps_describe_the_plan() {
        let msgs = decode(&encode(false));
        let w = msgs.iter().find(|m| m.global == p::MSG_WORKOUT).unwrap();
        assert_eq!(w.string(p::WORKOUT_WKT_NAME), "2x20");
        assert_eq!(w.uint(p::WORKOUT_SPORT), 2, "cycling");
        assert_eq!(w.uint(p::WORKOUT_SUB_SPORT), 6, "indoor_cycling");
        assert_eq!(w.uint(p::WORKOUT_NUM_VALID_STEPS), 2);

        let steps: Vec<&DecMsg> = msgs
            .iter()
            .filter(|m| m.global == p::MSG_WORKOUT_STEP)
            .collect();
        assert_eq!(steps.len(), 2);

        let s0 = steps[0];
        assert_eq!(s0.uint(p::WORKOUT_STEP_MESSAGE_INDEX), 0);
        assert_eq!(s0.string(p::WORKOUT_STEP_NAME), "Steady · 0:05 @ 80%");
        assert_eq!(s0.uint(p::WORKOUT_STEP_DURATION_TYPE), 0, "time");
        assert_eq!(s0.uint(p::WORKOUT_STEP_DURATION_VALUE), 5_000, "ms");
        assert_eq!(s0.uint(p::WORKOUT_STEP_TARGET_TYPE), 4, "power");
        assert_eq!(s0.uint(p::WORKOUT_STEP_TARGET_VALUE), 0, "custom range");
        assert_eq!(s0.uint(p::WORKOUT_STEP_CUSTOM_TARGET_LOW), 80, "% FTP");
        assert_eq!(s0.uint(p::WORKOUT_STEP_CUSTOM_TARGET_HIGH), 80);
        assert_eq!(s0.uint(p::WORKOUT_STEP_INTENSITY), 0, "active");
        assert_eq!(s0.uint(p::WORKOUT_STEP_SECONDARY_TARGET_TYPE), 3, "cadence");
        assert_eq!(s0.uint(p::WORKOUT_STEP_SECONDARY_TARGET_VALUE), 0);
        assert_eq!(s0.uint(p::WORKOUT_STEP_SECONDARY_CUSTOM_TARGET_LOW), 90);
        assert_eq!(s0.uint(p::WORKOUT_STEP_SECONDARY_CUSTOM_TARGET_HIGH), 90);

        let s1 = steps[1];
        assert_eq!(s1.uint(p::WORKOUT_STEP_MESSAGE_INDEX), 1);
        assert_eq!(s1.string(p::WORKOUT_STEP_NAME), "Interval · 0:15 @ 100%");
        assert_eq!(s1.uint(p::WORKOUT_STEP_DURATION_VALUE), 15_000);
        assert_eq!(s1.uint(p::WORKOUT_STEP_CUSTOM_TARGET_LOW), 100);
        // No cadence prescription: secondary target absent.
        assert_eq!(
            s1.raw(p::WORKOUT_STEP_SECONDARY_TARGET_TYPE).unwrap(),
            &[0xFF]
        );
        assert_eq!(
            s1.raw(p::WORKOUT_STEP_SECONDARY_CUSTOM_TARGET_LOW).unwrap(),
            &[0xFF; 4]
        );
    }

    #[test]
    fn step_roles_ramps_open_steps_and_watt_targets() {
        let h = header();
        let s = samples();
        let t = totals();
        let w = ExecutableWorkout {
            name: "roles".into(),
            description: String::new(),
            segments: vec![
                WorkoutSegment::Ramp {
                    duration_s: 600,
                    start: PowerTarget::PercentFtp(0.4),
                    end: PowerTarget::PercentFtp(0.6),
                    cadence_rpm: None,
                },
                WorkoutSegment::Steady {
                    duration_s: 300,
                    power: PowerTarget::Watts(275),
                    cadence_rpm: None,
                },
                WorkoutSegment::Steady {
                    duration_s: 120,
                    power: PowerTarget::PercentFtp(0.5),
                    cadence_rpm: None,
                },
                WorkoutSegment::FreeRide {
                    duration_s: 60,
                    cadence_rpm: Some(85),
                },
                WorkoutSegment::Ramp {
                    duration_s: 3_660,
                    start: PowerTarget::PercentFtp(0.6),
                    end: PowerTarget::PercentFtp(0.4),
                    cadence_rpm: None,
                },
            ],
            text_events: vec![],
        };
        let buf = encode_activity(&FitActivity {
            header: &h,
            samples: &s,
            events: &[],
            laps: &[],
            totals: &t,
            motion: None,
            workout: Some(&w),
        })
        .unwrap();
        let msgs = decode(&buf);
        let steps: Vec<&DecMsg> = msgs
            .iter()
            .filter(|m| m.global == p::MSG_WORKOUT_STEP)
            .collect();
        assert_eq!(steps.len(), 5);

        let warmup = steps[0];
        assert_eq!(
            warmup.string(p::WORKOUT_STEP_NAME),
            "Warm-up · 10:00 @ 40%→60%"
        );
        assert_eq!(warmup.uint(p::WORKOUT_STEP_INTENSITY), 2, "warmup");
        assert_eq!(warmup.uint(p::WORKOUT_STEP_CUSTOM_TARGET_LOW), 40);
        assert_eq!(warmup.uint(p::WORKOUT_STEP_CUSTOM_TARGET_HIGH), 60);

        // Absolute watts: 275 W at FTP 250 is an interval, encoded as 1000 + W.
        let interval = steps[1];
        assert_eq!(
            interval.string(p::WORKOUT_STEP_NAME),
            "Interval · 5:00 @ 275 W"
        );
        assert_eq!(interval.uint(p::WORKOUT_STEP_INTENSITY), 0, "active");
        assert_eq!(interval.uint(p::WORKOUT_STEP_CUSTOM_TARGET_LOW), 1_275);
        assert_eq!(interval.uint(p::WORKOUT_STEP_CUSTOM_TARGET_HIGH), 1_275);

        let recovery = steps[2];
        assert_eq!(
            recovery.string(p::WORKOUT_STEP_NAME),
            "Recovery · 2:00 @ 50%"
        );
        assert_eq!(recovery.uint(p::WORKOUT_STEP_INTENSITY), 1, "rest");

        let open = steps[3];
        assert_eq!(open.string(p::WORKOUT_STEP_NAME), "Free ride · 1:00");
        assert_eq!(open.uint(p::WORKOUT_STEP_TARGET_TYPE), 2, "open");
        assert_eq!(
            open.raw(p::WORKOUT_STEP_CUSTOM_TARGET_LOW).unwrap(),
            &[0xFF; 4]
        );
        assert_eq!(open.uint(p::WORKOUT_STEP_INTENSITY), 6, "other");
        assert_eq!(open.uint(p::WORKOUT_STEP_SECONDARY_CUSTOM_TARGET_LOW), 85);

        let cooldown = steps[4];
        assert_eq!(
            cooldown.string(p::WORKOUT_STEP_NAME),
            "Cool-down · 1:01:00 @ 60%→40%"
        );
        assert_eq!(cooldown.uint(p::WORKOUT_STEP_INTENSITY), 3, "cooldown");
        assert_eq!(cooldown.uint(p::WORKOUT_STEP_DURATION_VALUE), 3_660_000);
    }

    #[test]
    fn without_a_workout_laps_carry_no_step_link() {
        let h = header();
        let s = samples();
        let e = events();
        let l = laps();
        let t = totals();
        let buf = encode_activity(&FitActivity {
            header: &h,
            samples: &s,
            events: &e,
            laps: &l,
            totals: &t,
            motion: None,
            workout: None,
        })
        .unwrap();
        let msgs = decode(&buf);
        assert!(!msgs.iter().any(|m| m.global == p::MSG_WORKOUT));
        assert!(!msgs.iter().any(|m| m.global == p::MSG_WORKOUT_STEP));
        let l0 = msgs.iter().find(|m| m.global == p::MSG_LAP).unwrap();
        assert_eq!(l0.raw(p::LAP_WKT_STEP_INDEX).unwrap(), &[0xFF, 0xFF]);
        assert_eq!(l0.raw(p::LAP_INTENSITY).unwrap(), &[0xFF]);
    }

    #[test]
    fn session_message_sport_and_scaled_totals() {
        let msgs = decode(&encode(false));
        let s = msgs.iter().find(|m| m.global == p::MSG_SESSION).unwrap();
        assert_eq!(s.uint(p::SESSION_SPORT), 2, "cycling");
        assert_eq!(s.uint(p::SESSION_SUB_SPORT), 6, "indoor_cycling");
        assert_eq!(s.uint(p::SESSION_START_TIME), TS0);
        assert_eq!(s.uint(p::SESSION_TIMESTAMP), TS0 + 19);
        assert_eq!(s.uint(p::SESSION_TOTAL_ELAPSED_TIME), 19_500);
        assert_eq!(s.uint(p::SESSION_TOTAL_TIMER_TIME), 14_500);
        assert_eq!(s.uint(p::SESSION_AVG_POWER), 205);
        assert_eq!(s.uint(p::SESSION_MAX_POWER), 210);
        assert_eq!(s.uint(p::SESSION_AVG_HEART_RATE), 142);
        assert_eq!(s.uint(p::SESSION_MAX_HEART_RATE), 145);
        assert_eq!(s.uint(p::SESSION_AVG_CADENCE), 90);
        assert_eq!(s.uint(p::SESSION_MAX_CADENCE), 92);
        assert_eq!(s.uint(p::SESSION_TOTAL_WORK), 3_095, "joules");
        assert_eq!(s.uint(p::SESSION_TOTAL_CALORIES), 3, "kJ ≈ kcal");
        assert_eq!(s.uint(p::SESSION_NUM_LAPS), 2);
        assert_eq!(s.uint(p::SESSION_FIRST_LAP_INDEX), 0);
        assert_eq!(s.uint(p::SESSION_NORMALIZED_POWER), 207);
        assert_eq!(
            s.uint(p::SESSION_TRAINING_STRESS_SCORE),
            29,
            "2.86 × 10 rounded"
        );
        assert_eq!(s.uint(p::SESSION_INTENSITY_FACTOR), 828, "0.828 × 1000");
        assert_eq!(s.uint(p::SESSION_THRESHOLD_POWER), 250, "FTP");
    }

    #[test]
    fn activity_message() {
        let msgs = decode(&encode(false));
        let a = msgs.last().unwrap();
        assert_eq!(a.global, p::MSG_ACTIVITY);
        assert_eq!(a.uint(p::ACTIVITY_TIMESTAMP), TS0 + 19);
        assert_eq!(a.uint(p::ACTIVITY_TOTAL_TIMER_TIME), 14_500);
        assert_eq!(a.uint(p::ACTIVITY_NUM_SESSIONS), 1);
        assert_eq!(a.uint(p::ACTIVITY_TYPE), 0, "manual");
        assert_eq!(a.uint(p::ACTIVITY_EVENT), 26, "activity");
        assert_eq!(a.uint(p::ACTIVITY_EVENT_TYPE), 1, "stop");
        assert_eq!(a.uint(p::ACTIVITY_LOCAL_TIMESTAMP), TS0 + 19);
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
        let recs: Vec<&DecMsg> = msgs.iter().filter(|m| m.global == p::MSG_RECORD).collect();
        assert_eq!(recs.len(), 15);
        let mut prev = 0u64;
        for r in &recs {
            let speed = r.uint(p::RECORD_SPEED);
            let dist = r.uint(p::RECORD_DISTANCE);
            assert!(speed < p::INVALID_UINT16 as u64);
            assert!(dist >= prev, "distance monotonic");
            prev = dist;
        }
        // First sample is at rest; later records accelerate.
        let v0 = recs[0].uint(p::RECORD_SPEED) as f64 / 1000.0;
        assert_eq!(v0, 0.0);
        assert!(recs.last().unwrap().uint(p::RECORD_SPEED) > 0);
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
        let err = encode_activity(&FitActivity {
            header: &h,
            samples: &s,
            events: &e,
            laps: &l,
            totals: &t,
            motion: None,
            workout: None,
        });
        assert!(err.is_err());
    }

    #[test]
    fn empty_activity_encodes_and_validates() {
        let h = JournalHeader {
            trainer: None,
            hrm: None,
            ..header()
        };
        let t = SessionTotals {
            elapsed_s: 0,
            timer_s: 0,
            average_power_w: None,
            max_power_w: None,
            normalized_power_w: None,
            intensity_factor: None,
            training_stress_score: None,
            average_heart_rate_bpm: None,
            max_heart_rate_bpm: None,
            average_cadence_rpm: None,
            max_cadence_rpm: None,
            work_j: 0,
            work_kj: 0,
        };
        let buf = encode_activity(&FitActivity {
            header: &h,
            samples: &[],
            events: &[],
            laps: &[],
            totals: &t,
            motion: None,
            workout: None,
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
                p::MSG_USER_PROFILE,
                p::MSG_ZONES_TARGET,
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
            power_w: None,
            cadence_rpm: None,
            heart_rate_bpm: None,
            target_power_w: None,
            target_cadence_rpm: None,
        }];
        let t = totals();
        let buf = encode_activity(&FitActivity {
            header: &h,
            samples: &s,
            events: &[],
            laps: &[],
            totals: &t,
            motion: None,
            workout: None,
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
        let buf = encode_activity(&FitActivity {
            header: &h,
            samples: &s,
            events: &[],
            laps: &[],
            totals: &t,
            motion: None,
            workout: None,
        })
        .unwrap();
        let msgs = decode(&buf);
        let devs: Vec<&DecMsg> = msgs
            .iter()
            .filter(|m| m.global == p::MSG_DEVICE_INFO)
            .collect();
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
        assert_eq!(def_globals.len(), 11, "11 definitions");
    }
}
