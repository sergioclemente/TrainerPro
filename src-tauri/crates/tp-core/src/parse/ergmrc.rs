//! ERG/MRC (CompuTrainer text) parser. SPEC.md §3.2.
//!
//! Contract highlights:
//! - Unit truth: `MINUTES WATTS|PERCENT` column line in [COURSE HEADER];
//!   fallback to `ext_hint` ("erg" ⇒ Watts, "mrc" ⇒ PercentFtp); warn on
//!   mismatch between the two.
//! - Data rows are decimal minutes + value; tolerate tabs/spaces, ','
//!   decimal separator, CRLF.
//! - (t1,p1)→(t2,p2): t2>t1 emits Steady (p1==p2) or Ramp (p1≠p2) of
//!   (t2−t1)·60 s; t2==t1 is a vertical step (sets next start power, emits
//!   nothing).
//! - Optional [COURSE TEXT]: `offset_s  message  duration_s` rows.
//! - Errors (with line numbers): non-monotonic time, <2 data rows,
//!   unparseable row.

use super::{ParseError, ParseWarning, Parsed};
use crate::consts::{
    MAX_TARGET_WATTS, POWER_FRACTION_MAX, POWER_FRACTION_MIN, TEXT_EVENT_DEFAULT_S,
};
use crate::model::{PowerTarget, Segment, SourceFormat, TextEvent, Workout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Units {
    Watts,
    Percent,
}

impl Units {
    fn name(self) -> &'static str {
        match self {
            Units::Watts => "WATTS",
            Units::Percent => "PERCENT",
        }
    }
}

/// Which section of the file the line cursor is currently inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionState {
    None,
    Header,
    Data,
    Text,
    /// Inside an unrecognized `[...]` section; contents ignored.
    Unknown,
}

/// Parse a number tolerating a `,` decimal separator (e.g. `10,50`).
fn parse_num(tok: &str) -> Option<f64> {
    let s = tok.replace(',', ".");
    let v: f64 = s.parse().ok()?;
    if v.is_finite() { Some(v) } else { None }
}

/// Normalize a `[...]` section header line: strip brackets, uppercase,
/// collapse internal whitespace. Returns `None` if the line is not a
/// bracketed section marker.
fn section_marker(line: &str) -> Option<String> {
    let t = line.trim();
    if t.len() >= 2 && t.starts_with('[') && t.ends_with(']') {
        let inner = &t[1..t.len() - 1];
        let norm = inner
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_uppercase();
        Some(norm)
    } else {
        None
    }
}

/// Detect the `MINUTES WATTS|PERCENT` column-spec line (case-insensitive).
fn column_spec(line: &str) -> Option<Units> {
    let toks: Vec<String> = line
        .split_whitespace()
        .map(|t| t.to_uppercase())
        .collect();
    if toks.len() == 2 && toks[0] == "MINUTES" {
        match toks[1].as_str() {
            "WATTS" => Some(Units::Watts),
            "PERCENT" => Some(Units::Percent),
            _ => None,
        }
    } else {
        None
    }
}

fn ext_hint_units(ext_hint: Option<&str>) -> Option<Units> {
    match ext_hint.map(|e| e.trim_start_matches('.').to_ascii_lowercase()) {
        Some(ref e) if e == "erg" => Some(Units::Watts),
        Some(ref e) if e == "mrc" => Some(Units::Percent),
        _ => None,
    }
}

/// Convert a raw power value into a `PowerTarget` under the resolved units,
/// clamping to sanity bounds with a warning.
fn make_target(raw: f64, units: Units, line_no: usize, warnings: &mut Vec<ParseWarning>) -> PowerTarget {
    match units {
        Units::Percent => {
            let frac = raw / 100.0;
            let clamped = frac.clamp(POWER_FRACTION_MIN, POWER_FRACTION_MAX);
            if (clamped - frac).abs() > f64::EPSILON {
                warnings.push(ParseWarning {
                    message: format!(
                        "line {line_no}: power {raw}% outside sane range, clamped to {:.0}%",
                        clamped * 100.0
                    ),
                });
            }
            PowerTarget::PercentFtp(clamped)
        }
        Units::Watts => {
            let w = raw.round();
            let clamped = w.clamp(0.0, f64::from(MAX_TARGET_WATTS));
            if (clamped - w).abs() > f64::EPSILON {
                warnings.push(ParseWarning {
                    message: format!(
                        "line {line_no}: power {raw} W outside 0..={MAX_TARGET_WATTS}, clamped"
                    ),
                });
            }
            PowerTarget::Watts(clamped as u16)
        }
    }
}

/// `ext_hint`: lowercase file extension without dot, if known.
pub fn parse_ergmrc(input: &str, ext_hint: Option<&str>) -> Result<Parsed, ParseError> {
    let mut warnings: Vec<ParseWarning> = Vec::new();

    let mut column_units: Option<Units> = None;
    let mut description: Option<String> = None;
    let mut file_name: Option<String> = None;
    // (line_no, minutes, power) in file order.
    let mut rows: Vec<(usize, f64, f64)> = Vec::new();
    // (line_no, raw tokens line) for [COURSE TEXT].
    let mut text_events: Vec<TextEvent> = Vec::new();

    let mut state = SectionState::None;

    for (idx, raw_line) in input.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim_end_matches('\r'); // CRLF tolerance (lines() strips it, but be safe)
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(marker) = section_marker(trimmed) {
            state = match marker.as_str() {
                "COURSE HEADER" => SectionState::Header,
                "COURSE DATA" => SectionState::Data,
                "COURSE TEXT" => SectionState::Text,
                m if m.starts_with("END ") || m == "END" => SectionState::None,
                other => {
                    warnings.push(ParseWarning {
                        message: format!("line {line_no}: unknown section [{other}] ignored"),
                    });
                    SectionState::Unknown
                }
            };
            continue;
        }

        match state {
            SectionState::Header => {
                if let Some(u) = column_spec(trimmed) {
                    column_units = Some(u);
                } else if let Some(eq) = trimmed.find('=') {
                    let key = trimmed[..eq]
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_uppercase();
                    let value = trimmed[eq + 1..].trim().to_string();
                    match key.as_str() {
                        "DESCRIPTION" => description = Some(value),
                        "FILE NAME" | "FILENAME" => file_name = Some(value),
                        // VERSION / UNITS (ENGLISH/METRIC) etc.: irrelevant to
                        // the model; silently accepted.
                        _ => {}
                    }
                }
                // Anything else in the header is tolerated silently.
            }
            SectionState::Data => {
                let toks: Vec<&str> = trimmed.split_whitespace().collect();
                if toks.len() < 2 {
                    return Err(ParseError::Invalid(format!(
                        "line {line_no}: expected `<minutes> <power>` data row, got `{trimmed}`"
                    )));
                }
                if toks.len() > 2 {
                    warnings.push(ParseWarning {
                        message: format!(
                            "line {line_no}: extra columns in data row ignored"
                        ),
                    });
                }
                let t = parse_num(toks[0]).ok_or_else(|| {
                    ParseError::Invalid(format!(
                        "line {line_no}: cannot parse time `{}` in data row",
                        toks[0]
                    ))
                })?;
                let p = parse_num(toks[1]).ok_or_else(|| {
                    ParseError::Invalid(format!(
                        "line {line_no}: cannot parse power `{}` in data row",
                        toks[1]
                    ))
                })?;
                if t < 0.0 {
                    return Err(ParseError::Invalid(format!(
                        "line {line_no}: negative time {t} in data row"
                    )));
                }
                rows.push((line_no, t, p));
            }
            SectionState::Text => {
                let toks: Vec<&str> = trimmed.split_whitespace().collect();
                if toks.len() < 2 {
                    warnings.push(ParseWarning {
                        message: format!(
                            "line {line_no}: course-text row needs `<offset_s> <message>`, ignored"
                        ),
                    });
                    continue;
                }
                let Some(offset) = parse_num(toks[0]).filter(|v| *v >= 0.0) else {
                    warnings.push(ParseWarning {
                        message: format!(
                            "line {line_no}: cannot parse course-text offset `{}`, row ignored",
                            toks[0]
                        ),
                    });
                    continue;
                };
                // If the last token is numeric and there is a message between,
                // it is the duration; otherwise duration defaults.
                let (message, duration_s) = if toks.len() >= 3 {
                    match parse_num(toks[toks.len() - 1]).filter(|v| *v >= 0.0) {
                        Some(d) => (toks[1..toks.len() - 1].join(" "), d.round() as u32),
                        None => (toks[1..].join(" "), TEXT_EVENT_DEFAULT_S),
                    }
                } else {
                    (toks[1..].join(" "), TEXT_EVENT_DEFAULT_S)
                };
                text_events.push(TextEvent {
                    offset_s: offset.round() as u32,
                    message,
                    duration_s,
                });
            }
            SectionState::Unknown => {}
            SectionState::None => {
                warnings.push(ParseWarning {
                    message: format!("line {line_no}: content outside any section ignored"),
                });
            }
        }
    }

    // Resolve units: column line is the source of truth; extension is the
    // fallback; mismatch between the two is a warning.
    let hint_units = ext_hint_units(ext_hint);
    let units = match (column_units, hint_units) {
        (Some(c), Some(h)) => {
            if c != h {
                warnings.push(ParseWarning {
                    message: format!(
                        "column line says {} but file extension implies {}; using {}",
                        c.name(),
                        h.name(),
                        c.name()
                    ),
                });
            }
            c
        }
        (Some(c), None) => c,
        (None, Some(h)) => h,
        (None, None) => {
            warnings.push(ParseWarning {
                message: "no MINUTES WATTS|PERCENT column line and no usable file \
                          extension; assuming PERCENT"
                    .to_string(),
            });
            Units::Percent
        }
    };

    if rows.len() < 2 {
        return Err(ParseError::Invalid(format!(
            "[COURSE DATA] needs at least 2 data rows, found {}",
            rows.len()
        )));
    }

    // Build segments from consecutive row pairs.
    let mut segments: Vec<Segment> = Vec::new();
    for pair in rows.windows(2) {
        let (_l1, t1, p1) = pair[0];
        let (l2, t2, p2) = pair[1];
        if t2 < t1 {
            return Err(ParseError::Invalid(format!(
                "line {l2}: non-monotonic time ({t2} min after {t1} min)"
            )));
        }
        let duration_s = ((t2 - t1) * 60.0).round() as u32;
        if duration_s == 0 {
            // Vertical step: emits nothing; the next pair naturally starts
            // from p2.
            continue;
        }
        let start = make_target(p1, units, l2, &mut warnings);
        if p1 == p2 {
            segments.push(Segment::Steady {
                duration_s,
                power: start,
                cadence_rpm: None,
            });
        } else {
            let end = make_target(p2, units, l2, &mut warnings);
            segments.push(Segment::Ramp {
                duration_s,
                start,
                end,
                cadence_rpm: None,
            });
        }
    }

    if segments.is_empty() {
        return Err(ParseError::Empty);
    }

    text_events.sort_by_key(|e| e.offset_s);

    let source_format = match units {
        Units::Watts => SourceFormat::Erg,
        Units::Percent => SourceFormat::Mrc,
    };
    let description = description.unwrap_or_default();
    let name = if !description.is_empty() {
        description.clone()
    } else {
        file_name.unwrap_or_default()
    };

    Ok(Parsed {
        workout: Workout {
            name,
            description,
            source_format,
            segments,
            text_events,
        },
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(input: &str, ext: Option<&str>) -> Parsed {
        parse_ergmrc(input, ext).expect("expected successful parse")
    }

    fn err(input: &str, ext: Option<&str>) -> ParseError {
        parse_ergmrc(input, ext).expect_err("expected parse error")
    }

    const MRC_SS: &str = "\
[COURSE HEADER]
VERSION = 2
UNITS = ENGLISH
DESCRIPTION = Sweet Spot 3x12
FILE NAME = ss3x12.mrc
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00   45
10.00  45
10.00  88
22.00  88
[END COURSE DATA]
";

    #[test]
    fn mrc_percent_happy_path() {
        let p = ok(MRC_SS, Some("mrc"));
        assert_eq!(p.workout.source_format, SourceFormat::Mrc);
        assert_eq!(p.workout.name, "Sweet Spot 3x12");
        assert_eq!(p.workout.description, "Sweet Spot 3x12");
        assert_eq!(
            p.workout.segments,
            vec![
                Segment::Steady {
                    duration_s: 600,
                    power: PowerTarget::PercentFtp(0.45),
                    cadence_rpm: None,
                },
                Segment::Steady {
                    duration_s: 720,
                    power: PowerTarget::PercentFtp(0.88),
                    cadence_rpm: None,
                },
            ]
        );
        assert!(p.warnings.is_empty(), "warnings: {:?}", p.warnings);
        assert_eq!(p.workout.duration_s(), 1320);
    }

    #[test]
    fn erg_watts_happy_path() {
        let input = "\
[COURSE HEADER]
DESCRIPTION = Erg Test
MINUTES WATTS
[END COURSE HEADER]
[COURSE DATA]
0.00   100
5.00   100
5.00   250
15.00  250
[END COURSE DATA]
";
        let p = ok(input, Some("erg"));
        assert_eq!(p.workout.source_format, SourceFormat::Erg);
        assert_eq!(
            p.workout.segments,
            vec![
                Segment::Steady {
                    duration_s: 300,
                    power: PowerTarget::Watts(100),
                    cadence_rpm: None,
                },
                Segment::Steady {
                    duration_s: 600,
                    power: PowerTarget::Watts(250),
                    cadence_rpm: None,
                },
            ]
        );
        assert!(p.warnings.is_empty(), "warnings: {:?}", p.warnings);
    }

    #[test]
    fn column_line_is_source_of_truth_and_mismatch_warns() {
        // Column says WATTS, extension says mrc (PERCENT): column wins + warn.
        let input = "\
[COURSE HEADER]
MINUTES WATTS
[END COURSE HEADER]
[COURSE DATA]
0.00  150
1.00  150
[END COURSE DATA]
";
        let p = ok(input, Some("mrc"));
        assert_eq!(p.workout.source_format, SourceFormat::Erg);
        assert_eq!(
            p.workout.segments[0],
            Segment::Steady {
                duration_s: 60,
                power: PowerTarget::Watts(150),
                cadence_rpm: None,
            }
        );
        assert!(
            p.warnings.iter().any(|w| w.message.contains("WATTS")
                && w.message.contains("PERCENT")),
            "expected mismatch warning, got {:?}",
            p.warnings
        );
    }

    #[test]
    fn ext_hint_fallback_when_no_column_line() {
        let input = "\
[COURSE HEADER]
DESCRIPTION = No Columns
[END COURSE HEADER]
[COURSE DATA]
0.00  200
2.00  200
[END COURSE DATA]
";
        let p = ok(input, Some("erg"));
        assert_eq!(p.workout.source_format, SourceFormat::Erg);
        assert_eq!(
            p.workout.segments[0],
            Segment::Steady {
                duration_s: 120,
                power: PowerTarget::Watts(200),
                cadence_rpm: None,
            }
        );
        // Fallback per spec, so no mismatch warning.
        assert!(p.warnings.is_empty(), "warnings: {:?}", p.warnings);

        let p = ok(input, Some("mrc"));
        assert_eq!(p.workout.source_format, SourceFormat::Mrc);
        assert_eq!(
            p.workout.segments[0],
            Segment::Steady {
                duration_s: 120,
                power: PowerTarget::PercentFtp(2.0),
                cadence_rpm: None,
            }
        );
    }

    #[test]
    fn no_units_anywhere_defaults_to_percent_with_warning() {
        let input = "\
[COURSE DATA]
0.00  50
1.00  50
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(p.workout.source_format, SourceFormat::Mrc);
        assert!(
            p.warnings.iter().any(|w| w.message.contains("assuming PERCENT")),
            "warnings: {:?}",
            p.warnings
        );
    }

    #[test]
    fn steady_vs_ramp_emission() {
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00   50
5.00   75
5.00   75
10.00  75
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(
            p.workout.segments,
            vec![
                Segment::Ramp {
                    duration_s: 300,
                    start: PowerTarget::PercentFtp(0.50),
                    end: PowerTarget::PercentFtp(0.75),
                    cadence_rpm: None,
                },
                Segment::Steady {
                    duration_s: 300,
                    power: PowerTarget::PercentFtp(0.75),
                    cadence_rpm: None,
                },
            ]
        );
    }

    #[test]
    fn vertical_step_emits_nothing_and_sets_next_start_power() {
        // 0→10 @45 steady, vertical step to 88, 10→22 @88 steady.
        let p = ok(MRC_SS, Some("mrc"));
        assert_eq!(p.workout.segments.len(), 2);
        // Vertical step in a ramp context: next segment starts at stepped power.
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  40
0.00  60
5.00  90
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(
            p.workout.segments,
            vec![Segment::Ramp {
                duration_s: 300,
                start: PowerTarget::PercentFtp(0.60),
                end: PowerTarget::PercentFtp(0.90),
                cadence_rpm: None,
            }]
        );
    }

    #[test]
    fn decimal_comma_and_crlf_tolerated() {
        let input = "[COURSE HEADER]\r\nMINUTES PERCENT\r\n[END COURSE HEADER]\r\n\
                     [COURSE DATA]\r\n0,00\t45\r\n10,50\t45\r\n[END COURSE DATA]\r\n";
        let p = ok(input, None);
        assert_eq!(
            p.workout.segments,
            vec![Segment::Steady {
                duration_s: 630,
                power: PowerTarget::PercentFtp(0.45),
                cadence_rpm: None,
            }]
        );
    }

    #[test]
    fn course_text_parsing() {
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00   45
10.00  45
[END COURSE DATA]
[COURSE TEXT]
300  Halfway there  15
120  Find a rhythm  10
30   Settle in
[END COURSE TEXT]
";
        let p = ok(input, None);
        assert_eq!(
            p.workout.text_events,
            vec![
                TextEvent {
                    offset_s: 30,
                    message: "Settle in".into(),
                    duration_s: TEXT_EVENT_DEFAULT_S,
                },
                TextEvent {
                    offset_s: 120,
                    message: "Find a rhythm".into(),
                    duration_s: 10,
                },
                TextEvent {
                    offset_s: 300,
                    message: "Halfway there".into(),
                    duration_s: 15,
                },
            ]
        );
    }

    #[test]
    fn bad_course_text_rows_warn_but_do_not_fail() {
        let input = "\
[COURSE DATA]
0.00  45
1.00  45
[END COURSE DATA]
[COURSE TEXT]
notanumber hello 10
[END COURSE TEXT]
";
        let p = ok(input, Some("mrc"));
        assert!(p.workout.text_events.is_empty());
        assert!(
            p.warnings.iter().any(|w| w.message.contains("course-text")),
            "warnings: {:?}",
            p.warnings
        );
    }

    #[test]
    fn non_monotonic_time_errors_with_line_number() {
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00   45
10.00  45
5.00   88
[END COURSE DATA]
";
        let e = err(input, None);
        let msg = e.to_string();
        assert!(msg.contains("line 7"), "message: {msg}");
        assert!(msg.contains("non-monotonic"), "message: {msg}");
    }

    #[test]
    fn fewer_than_two_data_rows_errors() {
        let one_row = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  45
[END COURSE DATA]
";
        let msg = err(one_row, None).to_string();
        assert!(msg.contains("at least 2 data rows"), "message: {msg}");

        let no_data_section = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
";
        let msg = err(no_data_section, None).to_string();
        assert!(msg.contains("at least 2 data rows"), "message: {msg}");
    }

    #[test]
    fn unparseable_row_errors_with_line_number() {
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  45
abc   88
[END COURSE DATA]
";
        let msg = err(input, None).to_string();
        assert!(msg.contains("line 6"), "message: {msg}");

        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  45
5.00
[END COURSE DATA]
";
        let msg = err(input, None).to_string();
        assert!(msg.contains("line 6"), "message: {msg}");

        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  45
5.00  watts
[END COURSE DATA]
";
        let msg = err(input, None).to_string();
        assert!(msg.contains("line 6"), "message: {msg}");
    }

    #[test]
    fn negative_time_rejected() {
        let input = "\
[COURSE DATA]
-1.00  45
5.00   45
[END COURSE DATA]
";
        let msg = err(input, Some("mrc")).to_string();
        assert!(msg.contains("line 2"), "message: {msg}");
    }

    #[test]
    fn case_insensitive_sections_and_column_line() {
        let input = "\
[course header]
minutes percent
[end course header]
[Course Data]
0.00  45
1.00  45
[End Course Data]
";
        let p = ok(input, None);
        assert_eq!(p.workout.source_format, SourceFormat::Mrc);
        assert_eq!(p.workout.segments.len(), 1);
    }

    #[test]
    fn tolerant_whitespace_in_section_markers_and_rows() {
        let input = "[  COURSE   HEADER ]\n  MINUTES\t WATTS  \n[ END COURSE HEADER ]\n\
                     [ COURSE  DATA ]\n  0.00 \t 100 \n  2.00 \t 100 \n[ END COURSE DATA ]\n";
        let p = ok(input, None);
        assert_eq!(
            p.workout.segments,
            vec![Segment::Steady {
                duration_s: 120,
                power: PowerTarget::Watts(100),
                cadence_rpm: None,
            }]
        );
    }

    #[test]
    fn all_vertical_steps_yields_empty_error() {
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  45
0.00  88
[END COURSE DATA]
";
        let e = err(input, None);
        assert!(matches!(e, ParseError::Empty), "got {e:?}");
    }

    #[test]
    fn watts_clamped_with_warning() {
        let input = "\
[COURSE HEADER]
MINUTES WATTS
[END COURSE HEADER]
[COURSE DATA]
0.00  2500
1.00  2500
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(
            p.workout.segments[0],
            Segment::Steady {
                duration_s: 60,
                power: PowerTarget::Watts(MAX_TARGET_WATTS),
                cadence_rpm: None,
            }
        );
        assert!(
            p.warnings.iter().any(|w| w.message.contains("clamped")),
            "warnings: {:?}",
            p.warnings
        );
    }

    #[test]
    fn percent_clamped_with_warning() {
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  0
1.00  0
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(
            p.workout.segments[0],
            Segment::Steady {
                duration_s: 60,
                power: PowerTarget::PercentFtp(POWER_FRACTION_MIN),
                cadence_rpm: None,
            }
        );
        assert!(
            p.warnings.iter().any(|w| w.message.contains("clamped")),
            "warnings: {:?}",
            p.warnings
        );
    }

    #[test]
    fn unknown_section_ignored_with_warning() {
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE WIND]
0.00 5
[END COURSE WIND]
[COURSE DATA]
0.00  45
1.00  45
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(p.workout.segments.len(), 1);
        assert!(
            p.warnings.iter().any(|w| w.message.contains("unknown section")),
            "warnings: {:?}",
            p.warnings
        );
    }

    #[test]
    fn name_falls_back_to_file_name() {
        let input = "\
[COURSE HEADER]
FILE NAME = ss3x12.mrc
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  45
1.00  45
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(p.workout.name, "ss3x12.mrc");
        assert_eq!(p.workout.description, "");
    }

    #[test]
    fn fractional_minutes_round_to_seconds() {
        let input = "\
[COURSE HEADER]
MINUTES PERCENT
[END COURSE HEADER]
[COURSE DATA]
0.00  45
0.25  45
0.75  60
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(p.workout.segments[0].duration_s(), 15);
        assert_eq!(p.workout.segments[1].duration_s(), 30);
    }

    #[test]
    fn ramp_direction_preserved() {
        let input = "\
[COURSE HEADER]
MINUTES WATTS
[END COURSE HEADER]
[COURSE DATA]
0.00   250
5.00   100
[END COURSE DATA]
";
        let p = ok(input, None);
        assert_eq!(
            p.workout.segments,
            vec![Segment::Ramp {
                duration_s: 300,
                start: PowerTarget::Watts(250),
                end: PowerTarget::Watts(100),
                cadence_rpm: None,
            }]
        );
    }
}
