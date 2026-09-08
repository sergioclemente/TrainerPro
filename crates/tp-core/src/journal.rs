//! Ride journal: JSONL line types, writer/reader over caller-supplied
//! `io::Write`/`io::Read` (tp-core stays I/O-agnostic; the backend opens the
//! file and fsyncs). SPEC.md §6.
//!
//! Line format (exactly one JSON object per line, short keys):
//!   {"h":{...header...}}
//!   {"s":{"t":1234567,"p":215,"c":92,"hr":148,"tgt":220}}   // absent key = no data
//!   {"e":{"t":...,"k":"start","seg":4}}                      // seg only for "lap"
//!
//! Samples are NOT written while paused; pause/resume events bracket gaps.
//! Replay tolerates a truncated final line (crash mid-write).

use serde::{Deserialize, Serialize};
use std::io;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalHeader {
    pub ride_id: String,
    pub started_unix_ms: u64,
    pub workout_name: String,
    pub ftp: u16,
    pub weight_kg: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trainer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hrm: Option<String>,
    pub app_ver: String,
}

/// One 1 Hz sample. `t_ms` = ms since ride start (wall clock, includes
/// pauses in the timeline but no samples are emitted during pause).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    #[serde(rename = "t")]
    pub t_ms: u64,
    #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
    pub power: Option<u16>,
    #[serde(rename = "c", skip_serializing_if = "Option::is_none")]
    pub cadence: Option<u16>,
    #[serde(rename = "hr", skip_serializing_if = "Option::is_none")]
    pub hr: Option<u16>,
    #[serde(rename = "tgt", skip_serializing_if = "Option::is_none")]
    pub target: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RideEventKind {
    Start,
    Pause,
    Resume,
    Lap,
    FreerideEnter,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RideEvent {
    #[serde(rename = "t")]
    pub t_ms: u64,
    #[serde(rename = "k")]
    pub kind: RideEventKind,
    /// Segment index just finished; present for `Lap` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seg: Option<usize>,
}

/// Fully replayed journal.
#[derive(Debug, Clone, PartialEq)]
pub struct RideData {
    pub header: JournalHeader,
    pub samples: Vec<Sample>,
    pub events: Vec<RideEvent>,
}

/// Per-lap aggregates computed from samples between lap boundaries
/// (boundaries: Start → each Lap event → End/last sample; pauses excluded
/// from `timer_ms`). Calories: kJ ≈ kcal convention for cycling work.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lap {
    pub start_ms: u64,
    pub end_ms: u64,
    pub timer_ms: u64,
    pub avg_power: Option<u16>,
    pub max_power: Option<u16>,
    pub avg_hr: Option<u16>,
    pub max_hr: Option<u16>,
    pub avg_cadence: Option<u16>,
    pub calories_kcal: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("malformed journal: {0}")]
    Malformed(String),
    #[error("journal has no header line")]
    MissingHeader,
}

/// One journal line, externally tagged so serialization yields exactly
/// `{"h":{...}}` / `{"s":{...}}` / `{"e":{...}}`.
#[derive(Debug, Deserialize)]
enum Line {
    #[serde(rename = "h")]
    Header(JournalHeader),
    #[serde(rename = "s")]
    Sample(Sample),
    #[serde(rename = "e")]
    Event(RideEvent),
}

/// Borrowing twin of [`Line`] for serialization without cloning.
#[derive(Serialize)]
enum LineRef<'a> {
    #[serde(rename = "h")]
    Header(&'a JournalHeader),
    #[serde(rename = "s")]
    Sample(&'a Sample),
    #[serde(rename = "e")]
    Event(&'a RideEvent),
}

/// Appends lines to a caller-supplied writer. The caller is responsible for
/// opening the file and calling `flush`/fsync policy (the backend fsyncs per
/// line).
pub struct JournalWriter<W: io::Write> {
    inner: W,
}

impl<W: io::Write> JournalWriter<W> {
    /// Writes the header line immediately.
    pub fn new(inner: W, header: &JournalHeader) -> Result<Self, JournalError> {
        let mut w = JournalWriter { inner };
        w.write_line(&LineRef::Header(header))?;
        Ok(w)
    }

    pub fn write_sample(&mut self, s: &Sample) -> Result<(), JournalError> {
        self.write_line(&LineRef::Sample(s))
    }

    pub fn write_event(&mut self, e: &RideEvent) -> Result<(), JournalError> {
        self.write_line(&LineRef::Event(e))
    }

    pub fn into_inner(self) -> W {
        self.inner
    }

    fn write_line(&mut self, line: &LineRef<'_>) -> Result<(), JournalError> {
        // Serialization of these plain structs cannot fail in practice; map
        // any surprise through Malformed rather than panicking.
        let json = serde_json::to_string(line)
            .map_err(|e| JournalError::Malformed(format!("serialize: {e}")))?;
        self.inner.write_all(json.as_bytes())?;
        self.inner.write_all(b"\n")?;
        Ok(())
    }
}

/// Replay a journal. Tolerates a truncated/corrupt final line (dropped with
/// no error); any earlier malformed line is `JournalError::Malformed`.
pub fn replay<R: io::BufRead>(mut reader: R) -> Result<RideData, JournalError> {
    // Read raw byte lines ourselves (not `BufRead::lines`) so a final line
    // truncated mid-UTF-8-sequence is tolerated instead of erroring.
    let mut raw_lines: Vec<Vec<u8>> = Vec::new();
    loop {
        let mut buf = Vec::new();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        raw_lines.push(buf);
    }

    let mut header: Option<JournalHeader> = None;
    let mut samples: Vec<Sample> = Vec::new();
    let mut events: Vec<RideEvent> = Vec::new();
    let last_idx = raw_lines.len().saturating_sub(1);

    for (i, mut buf) in raw_lines.into_iter().enumerate() {
        while matches!(buf.last(), Some(b'\n' | b'\r')) {
            buf.pop();
        }
        if buf.is_empty() {
            continue;
        }
        let parsed = std::str::from_utf8(&buf)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Line>(s).map_err(|e| e.to_string()));
        let line = match parsed {
            Ok(line) => line,
            // Truncated/corrupt final line (crash mid-write): drop silently.
            Err(_) if i == last_idx => continue,
            Err(e) => {
                return Err(JournalError::Malformed(format!("line {}: {e}", i + 1)));
            }
        };
        match line {
            Line::Header(h) => {
                if header.is_some() {
                    return Err(JournalError::Malformed(format!(
                        "line {}: duplicate header",
                        i + 1
                    )));
                }
                header = Some(h);
            }
            Line::Sample(s) => {
                if header.is_none() {
                    return Err(JournalError::MissingHeader);
                }
                samples.push(s);
            }
            Line::Event(e) => {
                if header.is_none() {
                    return Err(JournalError::MissingHeader);
                }
                events.push(e);
            }
        }
    }

    let header = header.ok_or(JournalError::MissingHeader)?;
    Ok(RideData {
        header,
        samples,
        events,
    })
}

/// Compute laps from replayed data (see `Lap` doc for boundary rules).
pub fn compute_laps(data: &RideData) -> Vec<Lap> {
    let start = data
        .events
        .iter()
        .find(|e| e.kind == RideEventKind::Start)
        .map(|e| e.t_ms)
        .unwrap_or(0);
    let end_event = data
        .events
        .iter()
        .find(|e| e.kind == RideEventKind::End)
        .map(|e| e.t_ms);
    // Crash case (no End event): the ride effectively ends at the last thing
    // we ever heard — final sample or final event, whichever is later.
    let ride_end = end_event.unwrap_or_else(|| {
        data.samples
            .last()
            .map(|s| s.t_ms)
            .into_iter()
            .chain(data.events.last().map(|e| e.t_ms))
            .max()
            .unwrap_or(start)
            .max(start)
    });

    // Boundaries: start, each interior Lap event, ride end.
    let mut boundaries = vec![start];
    boundaries.extend(
        data.events
            .iter()
            .filter(|e| e.kind == RideEventKind::Lap && e.t_ms > start && e.t_ms < ride_end)
            .map(|e| e.t_ms),
    );
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries.push(ride_end);

    // Pause→resume intervals; an unpaired Pause extends to ride end.
    let mut pauses: Vec<(u64, u64)> = Vec::new();
    let mut open_pause: Option<u64> = None;
    for e in &data.events {
        match e.kind {
            RideEventKind::Pause => {
                if open_pause.is_none() {
                    open_pause = Some(e.t_ms);
                }
            }
            RideEventKind::Resume => {
                if let Some(p) = open_pause.take() {
                    pauses.push((p, e.t_ms.max(p)));
                }
            }
            _ => {}
        }
    }
    if let Some(p) = open_pause {
        pauses.push((p, ride_end.max(p)));
    }

    let lap_count = boundaries.len() - 1;
    let mut laps = Vec::new();
    for i in 0..lap_count {
        let (s, e) = (boundaries[i], boundaries[i + 1]);
        if e <= s {
            continue;
        }
        // Samples: [s, e) for interior laps; the final lap is end-inclusive so
        // a sample coinciding with the ride end (crash case) is not lost.
        let is_final = i == lap_count - 1;
        let lap_samples: Vec<&Sample> = data
            .samples
            .iter()
            .filter(|smp| smp.t_ms >= s && (smp.t_ms < e || (is_final && smp.t_ms == e)))
            .collect();

        let paused_ms: u64 = pauses
            .iter()
            .map(|&(ps, pe)| pe.min(e).saturating_sub(ps.max(s)))
            .sum();

        let powers: Vec<u16> = lap_samples.iter().filter_map(|smp| smp.power).collect();
        let hrs: Vec<u16> = lap_samples.iter().filter_map(|smp| smp.hr).collect();
        let cadences: Vec<u16> = lap_samples.iter().filter_map(|smp| smp.cadence).collect();

        // kJ of the lap (1 Hz samples ⇒ each sample is 1/SAMPLE_HZ joule-seconds
        // per watt); calories = kJ, rounded (kJ ≈ kcal cycling convention).
        let kj: f64 = powers.iter().map(|&p| f64::from(p)).sum::<f64>()
            / f64::from(crate::consts::SAMPLE_HZ)
            / 1000.0;

        laps.push(Lap {
            start_ms: s,
            end_ms: e,
            timer_ms: (e - s).saturating_sub(paused_ms),
            avg_power: mean_u16(&powers),
            max_power: powers.iter().copied().max(),
            avg_hr: mean_u16(&hrs),
            max_hr: hrs.iter().copied().max(),
            avg_cadence: mean_u16(&cadences),
            calories_kcal: kj.round() as u16,
        });
    }
    laps
}

fn mean_u16(vals: &[u16]) -> Option<u16> {
    if vals.is_empty() {
        return None;
    }
    let sum: f64 = vals.iter().map(|&v| f64::from(v)).sum();
    Some((sum / vals.len() as f64).round() as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn header() -> JournalHeader {
        JournalHeader {
            ride_id: "abc-123".into(),
            started_unix_ms: 1_700_000_000_000,
            workout_name: "Sweet Spot".into(),
            ftp: 250,
            weight_kg: 72.0,
            trainer: Some("KICKR CORE 1234".into()),
            hrm: Some("TICKR 5678".into()),
            app_ver: "0.1.0".into(),
        }
    }

    fn sample(t_ms: u64, p: Option<u16>, c: Option<u16>, hr: Option<u16>, tgt: Option<u16>) -> Sample {
        Sample {
            t_ms,
            power: p,
            cadence: c,
            hr,
            target: tgt,
        }
    }

    fn event(t_ms: u64, kind: RideEventKind) -> RideEvent {
        RideEvent {
            t_ms,
            kind,
            seg: None,
        }
    }

    fn lap_event(t_ms: u64, seg: usize) -> RideEvent {
        RideEvent {
            t_ms,
            kind: RideEventKind::Lap,
            seg: Some(seg),
        }
    }

    fn written(header: &JournalHeader, samples: &[Sample], events: &[RideEvent]) -> Vec<u8> {
        let mut w = JournalWriter::new(Vec::new(), header).unwrap();
        for s in samples {
            w.write_sample(s).unwrap();
        }
        for e in events {
            w.write_event(e).unwrap();
        }
        w.into_inner()
    }

    // ---- exact line formats -------------------------------------------------

    #[test]
    fn header_line_exact_format() {
        let bytes = written(&header(), &[], &[]);
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(
            text,
            concat!(
                r#"{"h":{"ride_id":"abc-123","started_unix_ms":1700000000000,"#,
                r#""workout_name":"Sweet Spot","ftp":250,"weight_kg":72.0,"#,
                r#""trainer":"KICKR CORE 1234","hrm":"TICKR 5678","app_ver":"0.1.0"}}"#,
                "\n"
            )
        );
    }

    #[test]
    fn header_line_omits_absent_optionals() {
        let mut h = header();
        h.trainer = None;
        h.hrm = None;
        let text = String::from_utf8(written(&h, &[], &[])).unwrap();
        assert!(!text.contains("trainer"), "trainer key must be absent: {text}");
        assert!(!text.contains("hrm"), "hrm key must be absent: {text}");
        assert_eq!(
            text,
            concat!(
                r#"{"h":{"ride_id":"abc-123","started_unix_ms":1700000000000,"#,
                r#""workout_name":"Sweet Spot","ftp":250,"weight_kg":72.0,"app_ver":"0.1.0"}}"#,
                "\n"
            )
        );
    }

    #[test]
    fn sample_line_exact_format_full() {
        let bytes = written(
            &header(),
            &[sample(1_234_567, Some(215), Some(92), Some(148), Some(220))],
            &[],
        );
        let text = String::from_utf8(bytes).unwrap();
        let line = text.lines().nth(1).unwrap();
        assert_eq!(line, r#"{"s":{"t":1234567,"p":215,"c":92,"hr":148,"tgt":220}}"#);
    }

    #[test]
    fn sample_line_absent_keys_truly_absent() {
        let bytes = written(&header(), &[sample(5000, None, None, None, None)], &[]);
        let text = String::from_utf8(bytes).unwrap();
        let line = text.lines().nth(1).unwrap();
        assert_eq!(line, r#"{"s":{"t":5000}}"#);
    }

    #[test]
    fn sample_line_partial_keys() {
        let bytes = written(&header(), &[sample(2000, Some(180), None, Some(140), None)], &[]);
        let text = String::from_utf8(bytes).unwrap();
        let line = text.lines().nth(1).unwrap();
        assert_eq!(line, r#"{"s":{"t":2000,"p":180,"hr":140}}"#);
    }

    #[test]
    fn event_line_exact_formats() {
        let bytes = written(
            &header(),
            &[],
            &[
                event(0, RideEventKind::Start),
                event(60_000, RideEventKind::Pause),
                event(65_000, RideEventKind::Resume),
                lap_event(600_000, 4),
                event(700_000, RideEventKind::FreerideEnter),
                event(900_000, RideEventKind::End),
            ],
        );
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(lines[0], r#"{"e":{"t":0,"k":"start"}}"#);
        assert_eq!(lines[1], r#"{"e":{"t":60000,"k":"pause"}}"#);
        assert_eq!(lines[2], r#"{"e":{"t":65000,"k":"resume"}}"#);
        assert_eq!(lines[3], r#"{"e":{"t":600000,"k":"lap","seg":4}}"#);
        assert_eq!(lines[4], r#"{"e":{"t":700000,"k":"freeride_enter"}}"#);
        assert_eq!(lines[5], r#"{"e":{"t":900000,"k":"end"}}"#);
    }

    // ---- roundtrip ----------------------------------------------------------

    #[test]
    fn write_replay_roundtrip() {
        let h = header();
        let samples = vec![
            sample(0, Some(100), Some(90), Some(120), Some(100)),
            sample(1000, Some(150), None, Some(130), Some(150)),
            sample(2000, None, Some(85), None, None),
        ];
        let events = vec![
            event(0, RideEventKind::Start),
            lap_event(1500, 0),
            event(3000, RideEventKind::End),
        ];
        let mut w = JournalWriter::new(Vec::new(), &h).unwrap();
        w.write_event(&events[0]).unwrap();
        for s in &samples {
            w.write_sample(s).unwrap();
        }
        w.write_event(&events[1]).unwrap();
        w.write_event(&events[2]).unwrap();
        let bytes = w.into_inner();

        let data = replay(Cursor::new(bytes)).unwrap();
        assert_eq!(data.header, h);
        assert_eq!(data.samples, samples);
        assert_eq!(data.events, events);
    }

    #[test]
    fn roundtrip_header_without_optionals() {
        let mut h = header();
        h.trainer = None;
        h.hrm = None;
        let bytes = written(&h, &[], &[]);
        let data = replay(Cursor::new(bytes)).unwrap();
        assert_eq!(data.header, h);
        assert_eq!(data.header.trainer, None);
        assert_eq!(data.header.hrm, None);
    }

    // ---- replay error handling ---------------------------------------------

    #[test]
    fn truncated_final_line_tolerated() {
        let mut bytes = written(
            &header(),
            &[sample(0, Some(100), None, None, None), sample(1000, Some(110), None, None, None)],
            &[],
        );
        bytes.extend_from_slice(br#"{"s":{"t":2000,"p":1"#); // crash mid-write, no newline
        let data = replay(Cursor::new(bytes)).unwrap();
        assert_eq!(data.samples.len(), 2);
        assert_eq!(data.samples[1].t_ms, 1000);
    }

    #[test]
    fn truncated_final_line_mid_utf8_tolerated() {
        let mut bytes = written(&header(), &[sample(0, Some(100), None, None, None)], &[]);
        // Cut a multi-byte UTF-8 char in half: "…" is E2 80 A6.
        bytes.extend_from_slice(b"{\"s\":{\"t\":2000,\"x\":\"\xE2\x80");
        let data = replay(Cursor::new(bytes)).unwrap();
        assert_eq!(data.samples.len(), 1);
    }

    #[test]
    fn corrupt_final_line_with_newline_tolerated() {
        let mut bytes = written(&header(), &[sample(0, Some(100), None, None, None)], &[]);
        bytes.extend_from_slice(b"garbage not json\n");
        let data = replay(Cursor::new(bytes)).unwrap();
        assert_eq!(data.samples.len(), 1);
    }

    #[test]
    fn malformed_earlier_line_errors_with_context() {
        let mut bytes = written(&header(), &[sample(0, Some(100), None, None, None)], &[]);
        bytes.extend_from_slice(b"garbage not json\n");
        bytes.extend_from_slice(br#"{"s":{"t":2000}}"#);
        bytes.push(b'\n');
        let err = replay(Cursor::new(bytes)).unwrap_err();
        match err {
            JournalError::Malformed(msg) => {
                assert!(msg.contains("line 3"), "expected line number in: {msg}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn missing_header_empty_input() {
        let err = replay(Cursor::new(Vec::<u8>::new())).unwrap_err();
        assert!(matches!(err, JournalError::MissingHeader));
    }

    #[test]
    fn missing_header_samples_only() {
        let bytes = b"{\"s\":{\"t\":0,\"p\":100}}\n{\"s\":{\"t\":1000,\"p\":110}}\n".to_vec();
        let err = replay(Cursor::new(bytes)).unwrap_err();
        assert!(matches!(err, JournalError::MissingHeader));
    }

    #[test]
    fn missing_header_truncated_header_line() {
        // Crash while writing the very first (header) line.
        let bytes = br#"{"h":{"ride_id":"abc","start"#.to_vec();
        let err = replay(Cursor::new(bytes)).unwrap_err();
        assert!(matches!(err, JournalError::MissingHeader));
    }

    #[test]
    fn duplicate_header_is_malformed() {
        let mut bytes = written(&header(), &[], &[]);
        let again = written(&header(), &[sample(0, None, None, None, None)], &[]);
        bytes.extend_from_slice(&again);
        let err = replay(Cursor::new(bytes)).unwrap_err();
        assert!(matches!(err, JournalError::Malformed(_)));
    }

    #[test]
    fn blank_lines_skipped() {
        let mut bytes = written(&header(), &[sample(0, Some(100), None, None, None)], &[]);
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(br#"{"s":{"t":1000,"p":110}}"#);
        bytes.push(b'\n');
        let data = replay(Cursor::new(bytes)).unwrap();
        assert_eq!(data.samples.len(), 2);
    }

    // ---- compute_laps -------------------------------------------------------

    fn ride(samples: Vec<Sample>, events: Vec<RideEvent>) -> RideData {
        RideData {
            header: header(),
            samples,
            events,
        }
    }

    #[test]
    fn laps_from_start_lap_end_boundaries() {
        // 10 samples at 1 Hz, powers 100..=190; lap boundary at 5000.
        let samples: Vec<Sample> = (0..10)
            .map(|k| sample(k * 1000, Some(100 + (k as u16) * 10), None, None, None))
            .collect();
        let events = vec![
            event(0, RideEventKind::Start),
            lap_event(5000, 0),
            event(10_000, RideEventKind::End),
        ];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 2);

        assert_eq!(laps[0].start_ms, 0);
        assert_eq!(laps[0].end_ms, 5000);
        assert_eq!(laps[0].timer_ms, 5000);
        assert_eq!(laps[0].avg_power, Some(120)); // 100..140
        assert_eq!(laps[0].max_power, Some(140));

        assert_eq!(laps[1].start_ms, 5000);
        assert_eq!(laps[1].end_ms, 10_000);
        assert_eq!(laps[1].timer_ms, 5000);
        assert_eq!(laps[1].avg_power, Some(170)); // 150..190
        assert_eq!(laps[1].max_power, Some(190));
    }

    #[test]
    fn lap_boundary_sample_belongs_to_next_lap() {
        let samples = vec![
            sample(0, Some(100), None, None, None),
            sample(5000, Some(500), None, None, None), // exactly on boundary
            sample(6000, Some(100), None, None, None),
        ];
        let events = vec![
            event(0, RideEventKind::Start),
            lap_event(5000, 0),
            event(7000, RideEventKind::End),
        ];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 2);
        assert_eq!(laps[0].max_power, Some(100));
        assert_eq!(laps[1].max_power, Some(500));
    }

    #[test]
    fn timer_excludes_pause_gap() {
        let samples = vec![
            sample(0, Some(100), None, None, None),
            sample(1000, Some(100), None, None, None),
            sample(2000, Some(100), None, None, None),
            // paused 3000..7000, no samples
            sample(7000, Some(100), None, None, None),
            sample(8000, Some(100), None, None, None),
            sample(9000, Some(100), None, None, None),
        ];
        let events = vec![
            event(0, RideEventKind::Start),
            event(3000, RideEventKind::Pause),
            event(7000, RideEventKind::Resume),
            event(10_000, RideEventKind::End),
        ];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 1);
        assert_eq!(laps[0].end_ms - laps[0].start_ms, 10_000);
        assert_eq!(laps[0].timer_ms, 6000); // 10 s elapsed − 4 s paused
    }

    #[test]
    fn pause_gap_split_across_laps() {
        // Pause 4000..8000 straddles the lap boundary at 6000.
        let events = vec![
            event(0, RideEventKind::Start),
            event(4000, RideEventKind::Pause),
            lap_event(6000, 0),
            event(8000, RideEventKind::Resume),
            event(12_000, RideEventKind::End),
        ];
        let laps = compute_laps(&ride(vec![], events));
        assert_eq!(laps.len(), 2);
        assert_eq!(laps[0].timer_ms, 4000); // 6 s − 2 s paused
        assert_eq!(laps[1].timer_ms, 4000); // 6 s − 2 s paused
    }

    #[test]
    fn unpaired_pause_extends_to_ride_end() {
        // Crash while paused: Pause never resumed, no End event.
        let samples = vec![
            sample(0, Some(100), None, None, None),
            sample(1000, Some(100), None, None, None),
            sample(2000, Some(100), None, None, None),
        ];
        let events = vec![event(0, RideEventKind::Start), event(3000, RideEventKind::Pause)];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 1);
        assert_eq!(laps[0].end_ms, 3000); // last event is the latest timestamp
        assert_eq!(laps[0].timer_ms, 3000); // zero-length open pause at the very end
    }

    #[test]
    fn aggregates_with_partially_missing_fields() {
        let samples = vec![
            sample(0, Some(100), Some(90), Some(150), None),
            sample(1000, None, None, Some(160), None),
            sample(2000, Some(201), Some(80), None, None),
        ];
        let events = vec![event(0, RideEventKind::Start), event(3000, RideEventKind::End)];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 1);
        let lap = laps[0];
        assert_eq!(lap.avg_power, Some(151)); // (100+201)/2 = 150.5 → 151
        assert_eq!(lap.max_power, Some(201));
        assert_eq!(lap.avg_hr, Some(155));
        assert_eq!(lap.max_hr, Some(160));
        assert_eq!(lap.avg_cadence, Some(85));
    }

    #[test]
    fn aggregates_none_when_field_never_present() {
        let samples = vec![
            sample(0, None, None, None, None),
            sample(1000, None, None, None, None),
        ];
        let events = vec![event(0, RideEventKind::Start), event(2000, RideEventKind::End)];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 1);
        let lap = laps[0];
        assert_eq!(lap.avg_power, None);
        assert_eq!(lap.max_power, None);
        assert_eq!(lap.avg_hr, None);
        assert_eq!(lap.max_hr, None);
        assert_eq!(lap.avg_cadence, None);
        assert_eq!(lap.calories_kcal, 0);
    }

    #[test]
    fn calories_are_lap_kilojoules_rounded() {
        // 300 s at 250 W = 75 000 J = 75 kJ → 75 kcal.
        let samples: Vec<Sample> = (0..300)
            .map(|k| sample(k * 1000, Some(250), None, None, None))
            .collect();
        let events = vec![event(0, RideEventKind::Start), event(300_000, RideEventKind::End)];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps[0].calories_kcal, 75);

        // Rounding: 3 s at 250 W = 0.75 kJ → 1 kcal; 2 s at 250 W = 0.5 kJ → 1 (half up).
        let samples: Vec<Sample> = (0..3)
            .map(|k| sample(k * 1000, Some(250), None, None, None))
            .collect();
        let events = vec![event(0, RideEventKind::Start), event(3000, RideEventKind::End)];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps[0].calories_kcal, 1);

        // 1 s at 250 W = 0.25 kJ → 0 kcal.
        let samples = vec![sample(0, Some(250), None, None, None)];
        let events = vec![event(0, RideEventKind::Start), event(1000, RideEventKind::End)];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps[0].calories_kcal, 0);
    }

    #[test]
    fn crash_case_final_partial_lap_without_end_event() {
        let samples: Vec<Sample> = (0..9)
            .map(|k| sample(k * 1000, Some(100), None, None, None))
            .collect();
        let events = vec![event(0, RideEventKind::Start), lap_event(5000, 0)];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 2);
        assert_eq!(laps[0].start_ms, 0);
        assert_eq!(laps[0].end_ms, 5000);
        // Final partial lap runs to the last sample, inclusive.
        assert_eq!(laps[1].start_ms, 5000);
        assert_eq!(laps[1].end_ms, 8000);
        assert_eq!(laps[1].timer_ms, 3000);
        // Sample at t == end (8000) is included in the final lap.
        assert_eq!(laps[1].calories_kcal, (4.0f64 * 100.0 / 1000.0).round() as u16);
        assert_eq!(laps[1].avg_power, Some(100));
    }

    #[test]
    fn no_events_single_lap_over_all_samples() {
        let samples: Vec<Sample> = (0..5)
            .map(|k| sample(k * 1000, Some(200), None, None, None))
            .collect();
        let laps = compute_laps(&ride(samples, vec![]));
        assert_eq!(laps.len(), 1);
        assert_eq!(laps[0].start_ms, 0);
        assert_eq!(laps[0].end_ms, 4000);
        assert_eq!(laps[0].timer_ms, 4000);
        assert_eq!(laps[0].avg_power, Some(200));
    }

    #[test]
    fn empty_ride_yields_no_laps() {
        let laps = compute_laps(&ride(vec![], vec![]));
        assert!(laps.is_empty());
    }

    #[test]
    fn lap_event_at_boundary_times_ignored() {
        // Lap events coinciding with start or end must not create zero-length laps.
        let samples = vec![sample(0, Some(100), None, None, None)];
        let events = vec![
            event(0, RideEventKind::Start),
            lap_event(0, 0),
            lap_event(5000, 1),
            event(5000, RideEventKind::End),
        ];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 1);
        assert_eq!((laps[0].start_ms, laps[0].end_ms), (0, 5000));
    }

    #[test]
    fn start_event_nonzero_offset() {
        // Boundaries come from the Start event, not t=0.
        let samples = vec![
            sample(500, Some(100), None, None, None),
            sample(1500, Some(200), None, None, None),
        ];
        let events = vec![event(500, RideEventKind::Start), event(2000, RideEventKind::End)];
        let laps = compute_laps(&ride(samples, events));
        assert_eq!(laps.len(), 1);
        assert_eq!(laps[0].start_ms, 500);
        assert_eq!(laps[0].end_ms, 2000);
        assert_eq!(laps[0].timer_ms, 1500);
    }
}
