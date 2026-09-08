//! Authoring: an editor tree → ZWO text. The inverse of `parse::zwo`.
//!
//! The builder screen edits a shallow tree (Simple / Ramp / Repeat, one level
//! of nesting) and saves by emitting ZWO, which then goes through the ordinary
//! import pipeline. That keeps one ingestion path for every workout in the app
//! — the player, FIT export and dedup never learn that a workout was authored
//! rather than imported.
//!
//! Emission targets *our own parser first*: every element this module writes
//! reads back to the same segments, which the round-trip tests below assert.
//! Zwift compatibility follows from using the same vocabulary, not from
//! guessing at its quirks.
//!
//! Structure preservation: a Repeat of exactly two Simple children maps to
//! `IntervalsT`, which keeps the repeat visible in the file. Anything else is
//! expanded inline — valid, rides identically, but the file no longer records
//! that it came from a repeat. See `to_zwo`'s return value.

use serde::{Deserialize, Serialize};

use crate::consts::{POWER_FRACTION_MAX, POWER_FRACTION_MIN};

/// Longest single interval the builder accepts (4 h). Guards against a stray
/// keystroke turning 60 into 60000 rather than expressing a real limit.
pub const MAX_SEGMENT_S: u32 = 4 * 3600;
/// Most repetitions one Repeat may carry.
pub const MAX_REPEAT_COUNT: u32 = 100;

/// Percent-of-FTP bounds, mirroring the model's fraction bounds so we never
/// emit a value the parser would clamp and warn about.
pub const MIN_POWER_PCT: f64 = POWER_FRACTION_MIN * 100.0;
pub const MAX_POWER_PCT: f64 = POWER_FRACTION_MAX * 100.0;

/// One node of the authoring tree. Intensities are percent of FTP (75.0 = 75 %)
/// because that is what the editor's fields hold; ZWO fractions are an emission
/// detail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BuildNode {
    Simple {
        duration_s: u32,
        power_pct: f64,
        #[serde(default)]
        cadence_rpm: Option<u16>,
    },
    Ramp {
        duration_s: u32,
        start_pct: f64,
        end_pct: f64,
        #[serde(default)]
        cadence_rpm: Option<u16>,
    },
    /// Container. `children` may not contain another Repeat — the builder's
    /// drop rules prevent it and `validate` enforces it.
    Repeat { count: u32, children: Vec<BuildNode> },
}

impl BuildNode {
    /// Ridden length in seconds, repeats expanded.
    pub fn duration_s(&self) -> u32 {
        match self {
            BuildNode::Simple { duration_s, .. } | BuildNode::Ramp { duration_s, .. } => *duration_s,
            BuildNode::Repeat { count, children } => {
                let inner: u32 = children.iter().map(BuildNode::duration_s).sum();
                inner.saturating_mul(*count)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkoutDraft {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub nodes: Vec<BuildNode>,
}

impl WorkoutDraft {
    pub fn duration_s(&self) -> u32 {
        self.nodes.iter().map(BuildNode::duration_s).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildError {
    pub message: String,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for BuildError {}

fn err<T>(message: impl Into<String>) -> Result<T, BuildError> {
    Err(BuildError { message: message.into() })
}

/// What emission had to give up, if anything. Surfaced to the user as an
/// import warning rather than swallowed.
#[derive(Debug, Clone, PartialEq)]
pub struct Emitted {
    pub xml: String,
    /// Repeats that could not be written as `IntervalsT` and were expanded.
    pub expanded_repeats: usize,
}

/// Reject a draft that would emit nonsense. Deliberately thin: the editor's
/// drop rules are what keep the *structure* legal, so this checks values plus
/// the one structural rule (no nested Repeat) that a bug could still violate.
pub fn validate(draft: &WorkoutDraft) -> Result<(), BuildError> {
    if draft.name.trim().is_empty() {
        return err("workout needs a name");
    }
    if draft.nodes.is_empty() {
        return err("workout needs at least one interval");
    }
    for node in &draft.nodes {
        validate_node(node, /*inside_repeat=*/ false)?;
    }
    Ok(())
}

fn validate_node(node: &BuildNode, inside_repeat: bool) -> Result<(), BuildError> {
    match node {
        BuildNode::Simple { duration_s, power_pct, .. } => {
            check_duration(*duration_s)?;
            check_power(*power_pct)
        }
        BuildNode::Ramp { duration_s, start_pct, end_pct, .. } => {
            check_duration(*duration_s)?;
            check_power(*start_pct)?;
            check_power(*end_pct)
        }
        BuildNode::Repeat { count, children } => {
            if inside_repeat {
                return err("a repeat cannot contain another repeat");
            }
            if *count < 1 || *count > MAX_REPEAT_COUNT {
                return err(format!("repeat count must be 1..={MAX_REPEAT_COUNT}, got {count}"));
            }
            if children.is_empty() {
                return err("a repeat needs at least one interval inside it");
            }
            for c in children {
                validate_node(c, /*inside_repeat=*/ true)?;
            }
            Ok(())
        }
    }
}

fn check_duration(duration_s: u32) -> Result<(), BuildError> {
    if duration_s == 0 {
        return err("interval duration must be at least 1 s");
    }
    if duration_s > MAX_SEGMENT_S {
        return err(format!("interval duration must be at most {MAX_SEGMENT_S} s"));
    }
    Ok(())
}

fn check_power(pct: f64) -> Result<(), BuildError> {
    if !pct.is_finite() || pct < MIN_POWER_PCT || pct > MAX_POWER_PCT {
        return err(format!(
            "intensity must be {MIN_POWER_PCT:.0}–{MAX_POWER_PCT:.0} % FTP, got {pct}"
        ));
    }
    Ok(())
}

/// Percent of FTP → the fraction ZWO stores, as short decimal text.
/// 75 → "0.75", 100 → "1", 102.5 → "1.025".
fn frac(pct: f64) -> String {
    let v = pct / 100.0;
    let mut s = format!("{v:.3}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s.is_empty() || s == "-0" {
        s = "0".into();
    }
    s
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn cadence_attr(cadence_rpm: Option<u16>, attr: &str) -> String {
    match cadence_rpm {
        Some(c) if c > 0 => format!(" {attr}=\"{c}\""),
        _ => String::new(),
    }
}

/// One leaf element. Ramps always use `<Ramp>` with PowerLow = start and
/// PowerHigh = end: the parser reads those positionally for `Ramp`, so a
/// descending ramp round-trips without needing `<Cooldown>`'s swap rule.
fn emit_leaf(node: &BuildNode, out: &mut String) {
    match node {
        BuildNode::Simple { duration_s, power_pct, cadence_rpm } => {
            out.push_str(&format!(
                "    <SteadyState Duration=\"{}\" Power=\"{}\"{}/>\n",
                duration_s,
                frac(*power_pct),
                cadence_attr(*cadence_rpm, "Cadence"),
            ));
        }
        BuildNode::Ramp { duration_s, start_pct, end_pct, cadence_rpm } => {
            out.push_str(&format!(
                "    <Ramp Duration=\"{}\" PowerLow=\"{}\" PowerHigh=\"{}\"{}/>\n",
                duration_s,
                frac(*start_pct),
                frac(*end_pct),
                cadence_attr(*cadence_rpm, "Cadence"),
            ));
        }
        // Callers never pass a Repeat here; validate() has already rejected
        // nesting, so the only Repeat is the top-level one emit_node handles.
        BuildNode::Repeat { .. } => {}
    }
}

/// A Repeat writes as `IntervalsT` only in the shape that element can express:
/// exactly two Simple children, read as (work, recovery).
fn as_intervals_t(count: u32, children: &[BuildNode]) -> Option<String> {
    let [BuildNode::Simple { duration_s: on_s, power_pct: on_p, cadence_rpm: on_c }, BuildNode::Simple { duration_s: off_s, power_pct: off_p, cadence_rpm: off_c }] =
        children
    else {
        return None;
    };
    Some(format!(
        "    <IntervalsT Repeat=\"{}\" OnDuration=\"{}\" OffDuration=\"{}\" OnPower=\"{}\" OffPower=\"{}\"{}{}/>\n",
        count,
        on_s,
        off_s,
        frac(*on_p),
        frac(*off_p),
        cadence_attr(*on_c, "Cadence"),
        cadence_attr(*off_c, "CadenceResting"),
    ))
}

fn emit_node(node: &BuildNode, out: &mut String, expanded: &mut usize) {
    match node {
        BuildNode::Repeat { count, children } => {
            if let Some(xml) = as_intervals_t(*count, children) {
                out.push_str(&xml);
            } else {
                *expanded += 1;
                for _ in 0..*count {
                    for c in children {
                        emit_leaf(c, out);
                    }
                }
            }
        }
        leaf => emit_leaf(leaf, out),
    }
}

/// Draft → ZWO document. Validates first; the returned `Emitted` reports how
/// many repeats had to be flattened to fit the format.
pub fn to_zwo(draft: &WorkoutDraft) -> Result<Emitted, BuildError> {
    validate(draft)?;

    let mut body = String::new();
    let mut expanded_repeats = 0usize;
    for node in &draft.nodes {
        emit_node(node, &mut body, &mut expanded_repeats);
    }

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <workout_file>\n\
         \x20 <author>TrainerPro</author>\n\
         \x20 <name>{}</name>\n\
         \x20 <description>{}</description>\n\
         \x20 <sportType>bike</sportType>\n\
         \x20 <tags/>\n\
         \x20 <workout>\n\
         {}\
         \x20 </workout>\n\
         </workout_file>\n",
        esc(draft.name.trim()),
        esc(&draft.description),
        body,
    );

    Ok(Emitted { xml, expanded_repeats })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PowerTarget, Segment};
    use crate::parse::parse_zwo;

    fn simple(duration_s: u32, power_pct: f64) -> BuildNode {
        BuildNode::Simple { duration_s, power_pct, cadence_rpm: None }
    }

    fn draft(nodes: Vec<BuildNode>) -> WorkoutDraft {
        WorkoutDraft { name: "Test".into(), description: String::new(), nodes }
    }

    fn round_trip(nodes: Vec<BuildNode>) -> Vec<Segment> {
        let out = to_zwo(&draft(nodes)).expect("emit");
        parse_zwo(&out.xml).expect("parse back").workout.segments
    }

    #[test]
    fn steady_round_trips() {
        let segs = round_trip(vec![simple(600, 75.0)]);
        assert_eq!(
            segs,
            vec![Segment::Steady {
                duration_s: 600,
                power: PowerTarget::PercentFtp(0.75),
                cadence_rpm: None,
            }]
        );
    }

    #[test]
    fn cadence_round_trips() {
        let segs = round_trip(vec![BuildNode::Simple {
            duration_s: 60,
            power_pct: 100.0,
            cadence_rpm: Some(95),
        }]);
        assert_eq!(
            segs,
            vec![Segment::Steady {
                duration_s: 60,
                power: PowerTarget::PercentFtp(1.0),
                cadence_rpm: Some(95),
            }]
        );
    }

    #[test]
    fn ascending_and_descending_ramps_keep_their_direction() {
        let segs = round_trip(vec![
            BuildNode::Ramp { duration_s: 600, start_pct: 55.0, end_pct: 75.0, cadence_rpm: None },
            BuildNode::Ramp { duration_s: 300, start_pct: 75.0, end_pct: 55.0, cadence_rpm: None },
        ]);
        assert_eq!(
            segs,
            vec![
                Segment::Ramp {
                    duration_s: 600,
                    start: PowerTarget::PercentFtp(0.55),
                    end: PowerTarget::PercentFtp(0.75),
                    cadence_rpm: None,
                },
                Segment::Ramp {
                    duration_s: 300,
                    start: PowerTarget::PercentFtp(0.75),
                    end: PowerTarget::PercentFtp(0.55),
                    cadence_rpm: None,
                },
            ]
        );
    }

    #[test]
    fn two_child_repeat_writes_intervals_t_and_expands_on_read() {
        let out = to_zwo(&draft(vec![BuildNode::Repeat {
            count: 3,
            children: vec![simple(60, 100.0), simple(30, 55.0)],
        }]))
        .unwrap();
        assert!(out.xml.contains("<IntervalsT Repeat=\"3\""), "{}", out.xml);
        assert_eq!(out.expanded_repeats, 0);

        let segs = parse_zwo(&out.xml).unwrap().workout.segments;
        assert_eq!(segs.len(), 6);
        assert_eq!(segs[0].duration_s(), 60);
        assert_eq!(segs[1].duration_s(), 30);
    }

    #[test]
    fn three_child_repeat_is_expanded_and_reported() {
        let out = to_zwo(&draft(vec![BuildNode::Repeat {
            count: 2,
            children: vec![simple(60, 100.0), simple(30, 55.0), simple(15, 40.0)],
        }]))
        .unwrap();
        assert!(!out.xml.contains("IntervalsT"));
        assert_eq!(out.expanded_repeats, 1);

        let segs = parse_zwo(&out.xml).unwrap().workout.segments;
        assert_eq!(segs.len(), 6);
        assert_eq!(
            segs.iter().map(|s| s.duration_s()).collect::<Vec<_>>(),
            vec![60, 30, 15, 60, 30, 15]
        );
    }

    #[test]
    fn repeat_holding_a_ramp_is_expanded() {
        let out = to_zwo(&draft(vec![BuildNode::Repeat {
            count: 2,
            children: vec![
                BuildNode::Ramp {
                    duration_s: 60,
                    start_pct: 80.0,
                    end_pct: 110.0,
                    cadence_rpm: None,
                },
                simple(60, 50.0),
            ],
        }]))
        .unwrap();
        assert_eq!(out.expanded_repeats, 1);
        assert_eq!(parse_zwo(&out.xml).unwrap().workout.segments.len(), 4);
    }

    #[test]
    fn the_worked_example_round_trips_whole() {
        // 10 min ramp 55→75, 3 × (1:00 @ 100 %, 0:30 @ 55 %), 5 min @ 55 %.
        let nodes = vec![
            BuildNode::Ramp { duration_s: 600, start_pct: 55.0, end_pct: 75.0, cadence_rpm: None },
            BuildNode::Repeat { count: 3, children: vec![simple(60, 100.0), simple(30, 55.0)] },
            simple(300, 55.0),
        ];
        let d = draft(nodes.clone());
        assert_eq!(d.duration_s(), 1170);

        let segs = round_trip(nodes);
        assert_eq!(segs.len(), 8);
        assert_eq!(segs.iter().map(|s| s.duration_s()).sum::<u32>(), 1170);
    }

    #[test]
    fn name_and_description_are_escaped() {
        let out = to_zwo(&WorkoutDraft {
            name: "Bridge & Surge".into(),
            description: "<hard>".into(),
            nodes: vec![simple(60, 100.0)],
        })
        .unwrap();
        assert!(out.xml.contains("Bridge &amp; Surge"));
        let parsed = parse_zwo(&out.xml).unwrap().workout;
        assert_eq!(parsed.name, "Bridge & Surge");
        assert_eq!(parsed.description, "<hard>");
    }

    #[test]
    fn fractions_are_written_short() {
        assert_eq!(frac(75.0), "0.75");
        assert_eq!(frac(100.0), "1");
        assert_eq!(frac(102.5), "1.025");
        assert_eq!(frac(55.0), "0.55");
    }

    #[test]
    fn rejects_empty_name_and_empty_workout() {
        assert!(to_zwo(&WorkoutDraft {
            name: "  ".into(),
            description: String::new(),
            nodes: vec![simple(60, 100.0)],
        })
        .is_err());
        assert!(to_zwo(&draft(vec![])).is_err());
    }

    #[test]
    fn rejects_bad_durations_and_intensities() {
        assert!(to_zwo(&draft(vec![simple(0, 100.0)])).is_err());
        assert!(to_zwo(&draft(vec![simple(MAX_SEGMENT_S + 1, 100.0)])).is_err());
        assert!(to_zwo(&draft(vec![simple(60, 0.0)])).is_err());
        assert!(to_zwo(&draft(vec![simple(60, 400.0)])).is_err());
        assert!(to_zwo(&draft(vec![simple(60, f64::NAN)])).is_err());
    }

    #[test]
    fn rejects_nested_repeats_and_degenerate_repeats() {
        let nested = BuildNode::Repeat {
            count: 2,
            children: vec![BuildNode::Repeat { count: 2, children: vec![simple(60, 100.0)] }],
        };
        assert!(to_zwo(&draft(vec![nested])).is_err());
        assert!(to_zwo(&draft(vec![BuildNode::Repeat { count: 0, children: vec![simple(60, 100.0)] }])).is_err());
        assert!(to_zwo(&draft(vec![BuildNode::Repeat { count: 2, children: vec![] }])).is_err());
    }

    #[test]
    fn duration_of_a_repeat_counts_every_lap() {
        let n = BuildNode::Repeat { count: 4, children: vec![simple(60, 100.0), simple(30, 55.0)] };
        assert_eq!(n.duration_s(), 360);
    }
}
