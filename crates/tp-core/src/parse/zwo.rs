//! ZWO (Zwift workout XML) parser. SPEC.md §3.1. Uses `roxmltree`.
//!
//! Contract highlights:
//! - Power attributes are FTP fractions → `PowerTarget::PercentFtp`.
//! - `IntervalsT` expands to Repeat × (Steady on, Steady off).
//! - Warmup/Ramp ramp PowerLow→PowerHigh; Cooldown ramps PowerHigh→PowerLow
//!   (known ambiguity — see spec; keep the rule in one place).
//! - `<textevent>` offsets are relative to the parent segment's start;
//!   convert to workout-absolute. Missing duration → TEXT_EVENT_DEFAULT_S.
//! - Unknown elements/attributes → ParseWarning, never an error. Elements
//!   with a Duration but unknown semantics map to Steady @ 100 % FTP + warn.
//! - Power fractions outside POWER_FRACTION_MIN..=MAX clamp with a warning.
//! - Errors: zero/negative durations, empty <workout>, malformed XML.

use roxmltree::Node;

use super::{ParseError, ParseWarning, Parsed};
use crate::consts::{POWER_FRACTION_MAX, POWER_FRACTION_MIN, TEXT_EVENT_DEFAULT_S};
use crate::model::{PowerTarget, Segment, SourceFormat, TextEvent, Workout};

pub fn parse_zwo(input: &str) -> Result<Parsed, ParseError> {
    // Real-world exporters emit unescaped '&' in titles/messages
    // ("Bridge & Surge"), which strict XML parsing rejects. Escape bare
    // ampersands (ones not starting a valid entity) before parsing.
    let (input, escaped_amps) = escape_bare_ampersands(input);
    let doc = roxmltree::Document::parse(&input)
        .map_err(|e| ParseError::Invalid(format!("malformed XML: {e}")))?;

    let root = doc.root_element();
    if root.tag_name().name() != "workout_file" {
        return Err(ParseError::Invalid(format!(
            "expected <workout_file> root element, found <{}>",
            root.tag_name().name()
        )));
    }

    let mut warnings: Vec<ParseWarning> = Vec::new();
    if escaped_amps > 0 {
        warnings.push(warning(format!(
            "{escaped_amps} unescaped '&' in the file were auto-escaped (exporter bug)"
        )));
    }

    let name = child_text(root, "name").unwrap_or_default();
    let description = child_text(root, "description").unwrap_or_default();
    // <author>, <sportType>, <tags>… are metadata we have no model fields for;
    // they are read past silently (they are well-known, not "unknown").

    let workout_el = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "workout")
        .ok_or_else(|| ParseError::Invalid("missing <workout> element".into()))?;

    let mut segments: Vec<Segment> = Vec::new();
    let mut text_events: Vec<TextEvent> = Vec::new();
    let mut elapsed_s: u32 = 0;

    for el in workout_el.children().filter(|n| n.is_element()) {
        let seg_start_s = elapsed_s;
        let tag = el.tag_name().name();
        match tag {
            "SteadyState" => {
                let Some(duration_s) = duration_or_skip(el, "Duration", &mut warnings)? else {
                    continue;
                };
                let power = required_power(el, "Power", &mut warnings)?;
                let cadence_rpm = optional_cadence(el, "Cadence", &mut warnings);
                warn_unknown_attrs(el, &["Duration", "Power", "Cadence"], &mut warnings);
                segments.push(Segment::Steady {
                    duration_s,
                    power,
                    cadence_rpm,
                });
                elapsed_s = elapsed_s.saturating_add(duration_s);
                collect_textevents(el, seg_start_s, &mut text_events, &mut warnings);
            }
            "IntervalsT" => {
                let repeat = required_repeat(el)?;
                let (Some(on_s), Some(off_s)) = (
                    duration_or_skip(el, "OnDuration", &mut warnings)?,
                    duration_or_skip(el, "OffDuration", &mut warnings)?,
                ) else {
                    continue;
                };
                let on_power = required_power(el, "OnPower", &mut warnings)?;
                let off_power = required_power(el, "OffPower", &mut warnings)?;
                let on_cadence = optional_cadence(el, "Cadence", &mut warnings);
                let off_cadence = optional_cadence(el, "CadenceResting", &mut warnings);
                warn_unknown_attrs(
                    el,
                    &[
                        "Repeat",
                        "OnDuration",
                        "OffDuration",
                        "OnPower",
                        "OffPower",
                        "Cadence",
                        "CadenceResting",
                    ],
                    &mut warnings,
                );
                for _ in 0..repeat {
                    segments.push(Segment::Steady {
                        duration_s: on_s,
                        power: on_power,
                        cadence_rpm: on_cadence,
                    });
                    segments.push(Segment::Steady {
                        duration_s: off_s,
                        power: off_power,
                        cadence_rpm: off_cadence,
                    });
                }
                elapsed_s =
                    elapsed_s.saturating_add(repeat.saturating_mul(on_s.saturating_add(off_s)));
                collect_textevents(el, seg_start_s, &mut text_events, &mut warnings);
            }
            "Warmup" | "Ramp" | "Cooldown" => {
                let Some(duration_s) = duration_or_skip(el, "Duration", &mut warnings)? else {
                    continue;
                };
                let low = required_power(el, "PowerLow", &mut warnings)?;
                let high = required_power(el, "PowerHigh", &mut warnings)?;
                let cadence_rpm = optional_cadence(el, "Cadence", &mut warnings);
                warn_unknown_attrs(
                    el,
                    &["Duration", "PowerLow", "PowerHigh", "Cadence"],
                    &mut warnings,
                );
                // Spec §3.1: Warmup/Ramp go low→high; Cooldown goes high→low.
                let (start, end) = if tag == "Cooldown" {
                    (high, low)
                } else {
                    (low, high)
                };
                segments.push(Segment::Ramp {
                    duration_s,
                    start,
                    end,
                    cadence_rpm,
                });
                elapsed_s = elapsed_s.saturating_add(duration_s);
                collect_textevents(el, seg_start_s, &mut text_events, &mut warnings);
            }
            "FreeRide" => {
                let Some(duration_s) = duration_or_skip(el, "Duration", &mut warnings)? else {
                    continue;
                };
                warn_unknown_attrs(el, &["Duration"], &mut warnings);
                segments.push(Segment::FreeRide { duration_s });
                elapsed_s = elapsed_s.saturating_add(duration_s);
                collect_textevents(el, seg_start_s, &mut text_events, &mut warnings);
            }
            other => {
                // Unknown element: Steady @ 100 % FTP when a usable Duration
                // exists, otherwise skipped. Warning either way, never an error.
                match parse_positive_seconds(el.attribute("Duration")) {
                    Some(duration_s) => {
                        warnings.push(warning(format!(
                            "unknown element <{other}> at {}: mapped to Steady at 100 % FTP for {duration_s} s",
                            pos(el)
                        )));
                        segments.push(Segment::Steady {
                            duration_s,
                            power: PowerTarget::PercentFtp(1.0),
                            cadence_rpm: None,
                        });
                        elapsed_s = elapsed_s.saturating_add(duration_s);
                        collect_textevents(el, seg_start_s, &mut text_events, &mut warnings);
                    }
                    None => {
                        warnings.push(warning(format!(
                            "unknown element <{other}> at {}: no usable Duration, skipped",
                            pos(el)
                        )));
                    }
                }
            }
        }
    }

    if segments.is_empty() {
        return Err(ParseError::Empty);
    }

    // Model contract: text events sorted by workout-absolute offset.
    text_events.sort_by_key(|e| e.offset_s);

    Ok(Parsed {
        workout: Workout {
            name,
            description,
            source_format: SourceFormat::Zwo,
            segments,
            text_events,
        },
        warnings,
    })
}

// ---------------------------------------------------------------------------
// helpers

fn warning(message: String) -> ParseWarning {
    ParseWarning { message }
}

/// "line N, column M" of an element's start tag, for error/warning messages.
fn pos(el: Node) -> String {
    let p = el.document().text_pos_at(el.range().start);
    format!("line {}, column {}", p.row, p.col)
}

/// Trimmed text content of the first child element named `name`.
fn child_text(parent: Node, name: &str) -> Option<String> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
}

/// Lenient positive-seconds parse used for unknown-element fallback:
/// `None` unless the attribute exists, parses, and rounds to ≥ 1 s.
fn parse_positive_seconds(raw: Option<&str>) -> Option<u32> {
    let v = raw?.trim().parse::<f64>().ok()?;
    if !v.is_finite() || v.round() < 1.0 {
        return None;
    }
    Some(v.round() as u32)
}

/// Duration for known elements: missing attribute is a structural error,
/// but zero/unparseable values skip the element with a warning instead of
/// failing the workout — real-world exporters emit Duration="0" for steps
/// they couldn't convert (observed: WorkoutPlanner with "4min30sec").
fn duration_or_skip(
    el: Node,
    attr: &str,
    warnings: &mut Vec<ParseWarning>,
) -> Result<Option<u32>, ParseError> {
    let tag = el.tag_name().name();
    let raw = el.attribute(attr).ok_or_else(|| {
        ParseError::Invalid(format!("<{tag}> at {}: missing {attr} attribute", pos(el)))
    })?;
    match raw.trim().parse::<f64>() {
        Ok(v) if v.is_finite() && v.round() >= 1.0 => Ok(Some(v.round() as u32)),
        _ => {
            warnings.push(warning(format!(
                "<{tag}> at {}: unusable {attr}=\"{raw}\" — segment skipped",
                pos(el)
            )));
            Ok(None)
        }
    }
}

/// Required FTP-fraction attribute; values outside the sanity bounds are
/// clamped with a warning (spec §3.1), unparseable values are errors.
fn required_power(
    el: Node,
    attr: &str,
    warnings: &mut Vec<ParseWarning>,
) -> Result<PowerTarget, ParseError> {
    let tag = el.tag_name().name();
    let raw = el.attribute(attr).ok_or_else(|| {
        ParseError::Invalid(format!("<{tag}> at {}: missing {attr} attribute", pos(el)))
    })?;
    let v = raw.trim().parse::<f64>().map_err(|_| {
        ParseError::Invalid(format!(
            "<{tag}> at {}: unparseable {attr} \"{raw}\"",
            pos(el)
        ))
    })?;
    if !v.is_finite() {
        return Err(ParseError::Invalid(format!(
            "<{tag}> at {}: non-finite {attr} \"{raw}\"",
            pos(el)
        )));
    }
    let clamped = v.clamp(POWER_FRACTION_MIN, POWER_FRACTION_MAX);
    if clamped != v {
        warnings.push(warning(format!(
            "<{tag}> at {}: {attr}={raw} outside {POWER_FRACTION_MIN}–{POWER_FRACTION_MAX}, clamped to {clamped}",
            pos(el)
        )));
    }
    Ok(PowerTarget::PercentFtp(clamped))
}

/// Optional cadence attribute; unusable values warn and yield `None`.
fn optional_cadence(el: Node, attr: &str, warnings: &mut Vec<ParseWarning>) -> Option<u16> {
    let raw = el.attribute(attr)?;
    match raw.trim().parse::<f64>() {
        Ok(v) if v.is_finite() && v.round() >= 1.0 && v.round() <= f64::from(u16::MAX) => {
            Some(v.round() as u16)
        }
        _ => {
            warnings.push(warning(format!(
                "<{}> at {}: ignoring invalid {attr} \"{raw}\"",
                el.tag_name().name(),
                pos(el)
            )));
            None
        }
    }
}

/// Required positive integer Repeat for IntervalsT.
fn required_repeat(el: Node) -> Result<u32, ParseError> {
    let tag = el.tag_name().name();
    let raw = el.attribute("Repeat").ok_or_else(|| {
        ParseError::Invalid(format!("<{tag}> at {}: missing Repeat attribute", pos(el)))
    })?;
    let v = raw.trim().parse::<f64>().map_err(|_| {
        ParseError::Invalid(format!(
            "<{tag}> at {}: unparseable Repeat \"{raw}\"",
            pos(el)
        ))
    })?;
    if !v.is_finite() || v.round() < 1.0 {
        return Err(ParseError::Invalid(format!(
            "<{tag}> at {}: Repeat must be ≥ 1, got \"{raw}\"",
            pos(el)
        )));
    }
    Ok(v.round() as u32)
}

/// Any attribute not in `known` is collected as a warning (spec §3 general
/// contract: unknown elements/attributes warn, never error).
fn warn_unknown_attrs(el: Node, known: &[&str], warnings: &mut Vec<ParseWarning>) {
    for a in el.attributes() {
        if !known.contains(&a.name()) {
            warnings.push(warning(format!(
                "<{}> at {}: unknown attribute {}=\"{}\" ignored",
                el.tag_name().name(),
                pos(el),
                a.name(),
                a.value()
            )));
        }
    }
}

/// Parse `<textevent>` children of a segment element. `timeoffset` is
/// relative to the parent segment's start; stored offsets are
/// workout-absolute. Missing/invalid `duration` → TEXT_EVENT_DEFAULT_S.
fn collect_textevents(
    el: Node,
    seg_start_s: u32,
    out: &mut Vec<TextEvent>,
    warnings: &mut Vec<ParseWarning>,
) {
    for child in el.children().filter(|n| n.is_element()) {
        let tag = child.tag_name().name();
        if tag != "textevent" {
            warnings.push(warning(format!(
                "unknown element <{tag}> at {} inside <{}> ignored",
                pos(child),
                el.tag_name().name()
            )));
            continue;
        }
        let Some(off_raw) = child.attribute("timeoffset") else {
            warnings.push(warning(format!(
                "<textevent> at {}: missing timeoffset, skipped",
                pos(child)
            )));
            continue;
        };
        let offset_rel_s = match off_raw.trim().parse::<f64>() {
            Ok(v) if v.is_finite() && v >= 0.0 => v.round() as u32,
            _ => {
                warnings.push(warning(format!(
                    "<textevent> at {}: invalid timeoffset \"{off_raw}\", skipped",
                    pos(child)
                )));
                continue;
            }
        };
        let Some(message) = child.attribute("message") else {
            warnings.push(warning(format!(
                "<textevent> at {}: missing message, skipped",
                pos(child)
            )));
            continue;
        };
        let duration_s = match child.attribute("duration") {
            None => TEXT_EVENT_DEFAULT_S,
            Some(d) => match d.trim().parse::<f64>() {
                Ok(v) if v.is_finite() && v.round() >= 1.0 => v.round() as u32,
                _ => {
                    warnings.push(warning(format!(
                        "<textevent> at {}: invalid duration \"{d}\", using default {TEXT_EVENT_DEFAULT_S} s",
                        pos(child)
                    )));
                    TEXT_EVENT_DEFAULT_S
                }
            },
        };
        warn_unknown_attrs(child, &["timeoffset", "message", "duration"], warnings);
        out.push(TextEvent {
            offset_s: seg_start_s.saturating_add(offset_rel_s),
            message: message.to_string(),
            duration_s,
        });
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pf(v: f64) -> PowerTarget {
        PowerTarget::PercentFtp(v)
    }

    /// Realistic file modeled on Zwift exports: full metadata, every element
    /// from the §3.1 table, sportType present-but-ignored.
    const FULL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workout_file>
    <author>TrainerPro Tests</author>
    <name>Sweet Spot Builder</name>
    <description>Warmup, sweet spot, VO2 bursts, free ride, cooldown.</description>
    <sportType>bike</sportType>
    <tags/>
    <workout>
        <Warmup Duration="600" PowerLow="0.40" PowerHigh="0.75" Cadence="90"/>
        <SteadyState Duration="300" Power="0.88" Cadence="95"/>
        <IntervalsT Repeat="3" OnDuration="60" OffDuration="120" OnPower="1.15" OffPower="0.55" Cadence="100" CadenceResting="85"/>
        <Ramp Duration="120" PowerLow="0.50" PowerHigh="0.90"/>
        <FreeRide Duration="300"/>
        <Cooldown Duration="600" PowerLow="0.35" PowerHigh="0.70"/>
    </workout>
</workout_file>"#;

    #[test]
    fn full_file_metadata_and_segment_layout() {
        let parsed = parse_zwo(FULL).unwrap();
        let w = &parsed.workout;
        assert_eq!(w.name, "Sweet Spot Builder");
        assert_eq!(
            w.description,
            "Warmup, sweet spot, VO2 bursts, free ride, cooldown."
        );
        assert_eq!(w.source_format, SourceFormat::Zwo);
        assert!(parsed.warnings.is_empty(), "unexpected: {:?}", parsed.warnings);

        // 1 warmup + 1 steady + 3×(on,off) + 1 ramp + 1 freeride + 1 cooldown
        assert_eq!(w.segments.len(), 11);
        assert_eq!(w.duration_s(), 600 + 300 + 3 * (60 + 120) + 120 + 300 + 600);
    }

    #[test]
    fn steadystate_maps_to_steady_with_cadence() {
        let parsed = parse_zwo(FULL).unwrap();
        assert_eq!(
            parsed.workout.segments[1],
            Segment::Steady {
                duration_s: 300,
                power: pf(0.88),
                cadence_rpm: Some(95),
            }
        );
    }

    #[test]
    fn warmup_ramps_low_to_high() {
        let parsed = parse_zwo(FULL).unwrap();
        assert_eq!(
            parsed.workout.segments[0],
            Segment::Ramp {
                duration_s: 600,
                start: pf(0.40),
                end: pf(0.75),
                cadence_rpm: Some(90),
            }
        );
    }

    #[test]
    fn ramp_element_ramps_low_to_high() {
        let parsed = parse_zwo(FULL).unwrap();
        assert_eq!(
            parsed.workout.segments[8],
            Segment::Ramp {
                duration_s: 120,
                start: pf(0.50),
                end: pf(0.90),
                cadence_rpm: None,
            }
        );
    }

    #[test]
    fn cooldown_ramps_high_to_low() {
        let parsed = parse_zwo(FULL).unwrap();
        assert_eq!(
            parsed.workout.segments[10],
            Segment::Ramp {
                duration_s: 600,
                start: pf(0.70),
                end: pf(0.35),
                cadence_rpm: None,
            }
        );
    }

    #[test]
    fn freeride_maps_to_freeride() {
        let parsed = parse_zwo(FULL).unwrap();
        assert_eq!(
            parsed.workout.segments[9],
            Segment::FreeRide { duration_s: 300 }
        );
    }

    #[test]
    fn intervalst_expands_repeat_with_cadences() {
        let parsed = parse_zwo(FULL).unwrap();
        let on = Segment::Steady {
            duration_s: 60,
            power: pf(1.15),
            cadence_rpm: Some(100),
        };
        let off = Segment::Steady {
            duration_s: 120,
            power: pf(0.55),
            cadence_rpm: Some(85),
        };
        // segments[2..8] = on,off × 3, in order
        assert_eq!(
            &parsed.workout.segments[2..8],
            &[
                on.clone(),
                off.clone(),
                on.clone(),
                off.clone(),
                on,
                off
            ]
        );
    }

    #[test]
    fn textevent_offsets_become_workout_absolute_and_sorted() {
        let xml = r#"<workout_file>
            <name>Text events</name>
            <workout>
                <Warmup Duration="300" PowerLow="0.4" PowerHigh="0.7">
                    <textevent timeoffset="10" message="Welcome!"/>
                </Warmup>
                <SteadyState Duration="600" Power="0.8">
                    <textevent timeoffset="30" message="Settle in" duration="15"/>
                    <textevent timeoffset="0" message="Here we go"/>
                </SteadyState>
            </workout>
        </workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(
            parsed.workout.text_events,
            vec![
                TextEvent {
                    offset_s: 10,
                    message: "Welcome!".into(),
                    duration_s: 10, // default
                },
                TextEvent {
                    offset_s: 300, // second segment starts at 300 s
                    message: "Here we go".into(),
                    duration_s: 10,
                },
                TextEvent {
                    offset_s: 330,
                    message: "Settle in".into(),
                    duration_s: 15,
                },
            ]
        );
    }

    #[test]
    fn textevent_inside_intervalst_is_relative_to_block_start() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="100" Power="0.5"/>
            <IntervalsT Repeat="2" OnDuration="30" OffDuration="60" OnPower="1.2" OffPower="0.5">
                <textevent timeoffset="5" message="Go!"/>
            </IntervalsT>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(parsed.workout.text_events.len(), 1);
        assert_eq!(parsed.workout.text_events[0].offset_s, 105);
    }

    #[test]
    fn textevent_invalid_fields_warn_and_skip_or_default() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60" Power="0.5">
                <textevent timeoffset="-3" message="never shown"/>
                <textevent message="no offset"/>
                <textevent timeoffset="5"/>
                <textevent timeoffset="10" message="bad duration" duration="oops"/>
            </SteadyState>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(parsed.workout.text_events.len(), 1);
        assert_eq!(parsed.workout.text_events[0].duration_s, 10);
        assert_eq!(parsed.warnings.len(), 4);
    }

    #[test]
    fn unknown_element_with_duration_becomes_steady_at_100pct() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60" Power="0.5"/>
            <MaxEffort Duration="30"/>
            <SteadyState Duration="60" Power="0.6"/>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(parsed.workout.segments.len(), 3);
        assert_eq!(
            parsed.workout.segments[1],
            Segment::Steady {
                duration_s: 30,
                power: pf(1.0),
                cadence_rpm: None,
            }
        );
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].message.contains("MaxEffort"));
        assert_eq!(parsed.workout.duration_s(), 150);
    }

    #[test]
    fn unknown_element_without_duration_is_skipped_with_warning() {
        let xml = r#"<workout_file><workout>
            <SolidState Power="0.8"/>
            <SteadyState Duration="120" Power="0.6">
                <textevent timeoffset="0" message="start"/>
            </SteadyState>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(parsed.workout.segments.len(), 1);
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].message.contains("SolidState"));
        assert!(parsed.warnings[0].message.contains("skipped"));
        // skipped element contributes no time: textevent lands at 0
        assert_eq!(parsed.workout.text_events[0].offset_s, 0);
    }

    #[test]
    fn power_fraction_clamps_high_and_low_with_warnings() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60" Power="3.5"/>
            <SteadyState Duration="60" Power="0.01"/>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(
            parsed.workout.segments[0],
            Segment::Steady {
                duration_s: 60,
                power: pf(3.0),
                cadence_rpm: None,
            }
        );
        assert_eq!(
            parsed.workout.segments[1],
            Segment::Steady {
                duration_s: 60,
                power: pf(0.05),
                cadence_rpm: None,
            }
        );
        assert_eq!(parsed.warnings.len(), 2);
        assert!(parsed.warnings[0].message.contains("clamped"));
        assert!(parsed.warnings[1].message.contains("clamped"));
    }

    #[test]
    fn negative_power_fraction_clamps_to_min() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60" Power="-0.5"/>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(
            parsed.workout.segments[0],
            Segment::Steady {
                duration_s: 60,
                power: pf(0.05),
                cadence_rpm: None,
            }
        );
        assert_eq!(parsed.warnings.len(), 1);
    }

    #[test]
    fn unknown_attribute_warns_but_parses() {
        // Real Zwift exports carry pace="0" on many elements.
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60" Power="0.5" pace="0"/>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(parsed.workout.segments.len(), 1);
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].message.contains("pace"));
    }

    #[test]
    fn invalid_cadence_warns_and_is_dropped() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60" Power="0.5" Cadence="abc"/>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(
            parsed.workout.segments[0],
            Segment::Steady {
                duration_s: 60,
                power: pf(0.5),
                cadence_rpm: None,
            }
        );
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].message.contains("Cadence"));
    }

    // -- error cases --------------------------------------------------------

    #[test]
    fn malformed_xml_is_invalid() {
        for bad in ["not xml at all", "<workout_file><workout>", "<a></b>"] {
            match parse_zwo(bad) {
                Err(ParseError::Invalid(msg)) => {
                    assert!(msg.contains("malformed XML"), "{msg}")
                }
                other => panic!("expected Invalid, got {other:?}"),
            }
        }
    }

    #[test]
    fn wrong_root_element_is_invalid() {
        let err = parse_zwo("<ride><workout/></ride>").unwrap_err();
        assert!(matches!(err, ParseError::Invalid(_)));
        assert!(err.to_string().contains("workout_file"));
    }

    #[test]
    fn missing_workout_element_is_invalid() {
        let err = parse_zwo("<workout_file><name>x</name></workout_file>").unwrap_err();
        assert!(err.to_string().contains("<workout>"));
    }

    #[test]
    fn empty_workout_is_empty_error() {
        for xml in [
            "<workout_file><workout/></workout_file>",
            "<workout_file><workout>   </workout></workout_file>",
        ] {
            assert!(matches!(parse_zwo(xml), Err(ParseError::Empty)));
        }
    }

    #[test]
    fn workout_with_only_skipped_unknowns_is_empty_error() {
        let xml = r#"<workout_file><workout>
            <SolidState Power="0.8"/>
        </workout></workout_file>"#;
        assert!(matches!(parse_zwo(xml), Err(ParseError::Empty)));
    }

    #[test]
    fn zero_duration_segment_skipped_with_warning() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60" Power="0.8"/>
            <SteadyState Duration="0" Power="0.9"/>
        </workout></workout_file>"#;
        let p = parse_zwo(xml).unwrap();
        assert_eq!(p.workout.segments.len(), 1);
        assert!(p.warnings.iter().any(|w| w.message.contains("skipped")), "{:?}", p.warnings);
    }

    #[test]
    fn negative_duration_segment_skipped_with_warning() {
        let xml = r#"<workout_file><workout>
            <FreeRide Duration="-5"/>
            <SteadyState Duration="60" Power="0.8"/>
        </workout></workout_file>"#;
        let p = parse_zwo(xml).unwrap();
        assert_eq!(p.workout.segments.len(), 1);
        assert!(p.warnings.iter().any(|w| w.message.contains("skipped")));
    }

    #[test]
    fn bad_interval_off_duration_skips_block_with_warning() {
        let xml = r#"<workout_file><workout>
            <IntervalsT Repeat="3" OnDuration="60" OffDuration="-1" OnPower="1.0" OffPower="0.5"/>
            <SteadyState Duration="60" Power="0.8"/>
        </workout></workout_file>"#;
        let p = parse_zwo(xml).unwrap();
        assert_eq!(p.workout.segments.len(), 1);
        assert!(p.warnings.iter().any(|w| w.message.contains("OffDuration")));
    }

    #[test]
    fn unparseable_duration_skips_segment_with_warning() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="soon" Power="0.5"/>
            <SteadyState Duration="60" Power="0.5"/>
        </workout></workout_file>"#;
        let p = parse_zwo(xml).unwrap();
        assert_eq!(p.workout.segments.len(), 1);
        assert!(p.warnings.iter().any(|w| w.message.contains("skipped")));
    }

    #[test]
    fn missing_power_is_invalid() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60"/>
        </workout></workout_file>"#;
        let err = parse_zwo(xml).unwrap_err();
        assert!(err.to_string().contains("Power"), "{err}");
    }

    #[test]
    fn zero_repeat_is_invalid() {
        let xml = r#"<workout_file><workout>
            <IntervalsT Repeat="0" OnDuration="30" OffDuration="60" OnPower="1.2" OffPower="0.5"/>
        </workout></workout_file>"#;
        let err = parse_zwo(xml).unwrap_err();
        assert!(err.to_string().contains("Repeat"), "{err}");
    }

    // -- misc leniency ------------------------------------------------------

    #[test]
    fn fractional_duration_rounds_to_whole_seconds() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="59.6" Power="0.5"/>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(parsed.workout.segments[0].duration_s(), 60);
    }

    #[test]
    fn missing_metadata_defaults_to_empty_strings() {
        let xml = r#"<workout_file><workout>
            <SteadyState Duration="60" Power="0.5"/>
        </workout></workout_file>"#;
        let parsed = parse_zwo(xml).unwrap();
        assert_eq!(parsed.workout.name, "");
        assert_eq!(parsed.workout.description, "");
    }

    #[test]
    fn error_messages_name_the_line() {
        let xml = "<workout_file>\n<workout>\n<SteadyState Duration=\"60\"/>\n</workout>\n</workout_file>";
        let err = parse_zwo(xml).unwrap_err();
        assert!(err.to_string().contains("line 3"), "{err}");
    }
}

#[cfg(test)]
mod writer_tests {
    use super::*;
    use crate::model::{PowerTarget, Segment, SourceFormat, TextEvent, Workout};

    #[test]
    fn to_zwo_roundtrips_through_parse_zwo() {
        let w = Workout {
            name: "RT <&> test".into(),
            description: "desc".into(),
            source_format: SourceFormat::Zwo,
            segments: vec![
                Segment::Ramp {
                    duration_s: 300,
                    start: PowerTarget::PercentFtp(0.40),
                    end: PowerTarget::PercentFtp(1.05),
                    cadence_rpm: None,
                },
                Segment::Steady {
                    duration_s: 120,
                    power: PowerTarget::PercentFtp(0.50),
                    cadence_rpm: Some(90),
                },
                Segment::FreeRide { duration_s: 60 },
                Segment::Ramp {
                    duration_s: 300,
                    start: PowerTarget::PercentFtp(0.70),
                    end: PowerTarget::PercentFtp(0.40),
                    cadence_rpm: None,
                },
            ],
            text_events: vec![TextEvent { offset_s: 310, message: "go!".into(), duration_s: 10 }],
        };
        let xml = to_zwo(&w);
        let parsed = parse_zwo(&xml).expect("roundtrip parses");
        assert_eq!(parsed.workout.name, w.name);
        assert_eq!(parsed.workout.duration_s(), w.duration_s());
        assert_eq!(parsed.workout.segments.len(), w.segments.len());
        // Descending ramp keeps its direction.
        match &parsed.workout.segments[3] {
            Segment::Ramp { start, end, .. } => {
                assert_eq!(start.resolve(200, 1.0), 140);
                assert_eq!(end.resolve(200, 1.0), 80);
            }
            other => panic!("expected ramp, got {other:?}"),
        }
        assert_eq!(parsed.workout.text_events.len(), 1);
        assert_eq!(parsed.workout.text_events[0].offset_s, 310);
    }
}

/// Serialize a Workout back to ZWO XML (flat segments — repeats stay
/// expanded). Lets any source's parsed model enter the files-are-truth
/// import pipeline. Round-trips through `parse_zwo`.
pub fn to_zwo(w: &Workout) -> String {
    fn frac(p: &PowerTarget) -> String {
        match p {
            PowerTarget::PercentFtp(f) => format!("{f:.3}"),
            // Sources feeding this writer produce PercentFtp; absolute watts
            // are emitted relative to a nominal 100 W (pre-convert upstream).
            PowerTarget::Watts(watts) => format!("{:.3}", f64::from(*watts) / 100.0),
        }
    }
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }
    let mut out = String::new();
    out.push_str("<workout_file>\n");
    out.push_str(&format!("    <name>{}</name>\n", esc(&w.name)));
    out.push_str(&format!("    <description>{}</description>\n", esc(&w.description)));
    out.push_str("    <sportType>bike</sportType>\n    <workout>\n");
    let mut t = 0u32;
    for seg in &w.segments {
        let texts: String = w
            .text_events
            .iter()
            .filter(|e| e.offset_s >= t && e.offset_s < t + seg.duration_s())
            .map(|e| {
                format!(
                    "\n            <textevent timeoffset=\"{}\" message=\"{}\" duration=\"{}\"/>",
                    e.offset_s - t,
                    esc(&e.message),
                    e.duration_s
                )
            })
            .collect();
        let close = |tag: &str, inner: &str| {
            if inner.is_empty() { "/>".to_string() } else { format!(">{inner}\n        </{tag}>") }
        };
        match seg {
            Segment::Steady { duration_s, power, cadence_rpm } => {
                let cad = cadence_rpm.map(|c| format!(" Cadence=\"{c}\"")).unwrap_or_default();
                out.push_str(&format!(
                    "        <SteadyState Duration=\"{duration_s}\" Power=\"{}\"{cad}{}\n",
                    frac(power),
                    close("SteadyState", &texts)
                ));
            }
            Segment::Ramp { duration_s, start, end, cadence_rpm } => {
                let cad = cadence_rpm.map(|c| format!(" Cadence=\"{c}\"")).unwrap_or_default();
                out.push_str(&format!(
                    "        <Ramp Duration=\"{duration_s}\" PowerLow=\"{}\" PowerHigh=\"{}\"{cad}{}\n",
                    frac(start),
                    frac(end),
                    close("Ramp", &texts)
                ));
            }
            Segment::FreeRide { duration_s } => {
                out.push_str(&format!(
                    "        <FreeRide Duration=\"{duration_s}\"{}\n",
                    close("FreeRide", &texts)
                ));
            }
        }
        t += seg.duration_s();
    }
    out.push_str("    </workout>\n</workout_file>\n");
    out
}

/// Escape `&` characters that don't begin a valid XML entity reference.
/// Returns the (possibly fixed) text and how many were escaped.
fn escape_bare_ampersands(input: &str) -> (std::borrow::Cow<'_, str>, usize) {
    fn valid_entity_after_amp(rest: &str) -> bool {
        for known in ["amp;", "lt;", "gt;", "quot;", "apos;"] {
            if rest.starts_with(known) {
                return true;
            }
        }
        // Numeric refs: &#123; or &#x1F3; (bounded length)
        if let Some(num) = rest.strip_prefix('#') {
            let digits = num.strip_prefix(['x', 'X']).unwrap_or(num);
            if let Some(semi) = digits.find(';') {
                return semi > 0 && semi <= 6 && digits[..semi].chars().all(|c| c.is_ascii_hexdigit());
            }
        }
        false
    }
    let mut count = 0usize;
    let mut out = String::new();
    let mut last = 0usize;
    for (i, b) in input.bytes().enumerate() {
        if b == b'&' && !valid_entity_after_amp(&input[i + 1..]) {
            out.push_str(&input[last..i]);
            out.push_str("&amp;");
            last = i + 1;
            count += 1;
        }
    }
    if count == 0 {
        (std::borrow::Cow::Borrowed(input), 0)
    } else {
        out.push_str(&input[last..]);
        (std::borrow::Cow::Owned(out), count)
    }
}

#[cfg(test)]
mod entity_tests {
    use super::*;

    #[test]
    fn bare_ampersands_escaped_with_warning() {
        let xml = r#"<workout_file><name>Bridge & Surge</name><workout>
            <SteadyState Duration="60" Power="0.8">
                <textevent timeoffset="0" message="surge & recover"/>
            </SteadyState>
        </workout></workout_file>"#;
        let p = parse_zwo(xml).expect("parses after escaping");
        assert_eq!(p.workout.name, "Bridge & Surge");
        assert_eq!(p.workout.text_events[0].message, "surge & recover");
        assert!(p.warnings.iter().any(|w| w.message.contains("auto-escaped")));
    }

    #[test]
    fn valid_entities_left_alone() {
        let xml = r#"<workout_file><name>A &amp; B &#38; C</name><workout>
            <SteadyState Duration="60" Power="0.8"/>
        </workout></workout_file>"#;
        let p = parse_zwo(xml).unwrap();
        assert_eq!(p.workout.name, "A & B & C");
        assert!(!p.warnings.iter().any(|w| w.message.contains("auto-escaped")));
    }
}
