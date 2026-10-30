//! Opening a stored WorkoutSpec in the builder: every builder-authored
//! definition round-trips, and anything the builder cannot hold is refused
//! with a reason instead of being dropped.

use tp_core::build::{from_workout_spec, to_workout_spec, BuildNode, WorkoutDraft};
use tp_core::workout_spec::WorkoutSpec;

fn spec(steps: &str) -> WorkoutSpec {
    WorkoutSpec::from_json(&format!(
        r#"{{"format":"WorkoutSpec","version":1,"title":"Edit me","description":"Notes",
            "prescription":{{"sport":"cycling","steps":[{steps}]}}}}"#
    ))
    .unwrap()
}

#[test]
fn builder_authored_definitions_round_trip_through_the_store() {
    let draft = WorkoutDraft {
        name: "Over-unders".into(),
        description: "Three sets".into(),
        nodes: vec![
            BuildNode::Ramp {
                duration_s: 600,
                start_pct: 50.0,
                end_pct: 75.0,
                cadence_rpm: None,
            },
            BuildNode::Repeat {
                count: 3,
                children: vec![
                    BuildNode::Simple {
                        duration_s: 120,
                        power_pct: 105.0,
                        cadence_rpm: Some(95),
                    },
                    BuildNode::Simple {
                        duration_s: 60,
                        power_pct: 85.0,
                        cadence_rpm: None,
                    },
                ],
            },
            BuildNode::Simple {
                duration_s: 300,
                power_pct: 55.0,
                cadence_rpm: None,
            },
        ],
    };
    let stored = to_workout_spec(&draft).unwrap();
    let json = stored.to_json_pretty().unwrap();
    let reopened = from_workout_spec(&WorkoutSpec::from_json(&json).unwrap()).unwrap();
    assert_eq!(reopened, draft);
}

#[test]
fn imported_percent_targets_with_exact_cadence_open_in_the_builder() {
    let definition = spec(
        r#"{"type":"steady","duration_seconds":300,"power":{"type":"percent_ftp","percent":65},
            "cadence":{"type":"exact","rpm":90}}"#,
    );
    let draft = from_workout_spec(&definition).unwrap();
    assert_eq!(draft.name, "Edit me");
    assert_eq!(draft.description, "Notes");
    assert_eq!(
        draft.nodes,
        vec![BuildNode::Simple {
            duration_s: 300,
            power_pct: 65.0,
            cadence_rpm: Some(90),
        }]
    );
}

#[test]
fn constructs_the_builder_cannot_hold_are_refused_with_a_reason() {
    let cases = [
        (
            r#"{"type":"free_ride","duration_seconds":300}"#,
            "free-ride",
        ),
        (
            r#"{"type":"steady","duration_seconds":300,"power":{"type":"watts","watts":200}}"#,
            "absolute-watt",
        ),
        (
            r#"{"type":"steady","duration_seconds":300,
                "power":{"type":"percent_ftp_range","min_percent":60,"max_percent":70}}"#,
            "power range",
        ),
        (
            r#"{"type":"steady","duration_seconds":300,"power":{"type":"percent_ftp","percent":65},
                "cadence":{"type":"range","min_rpm":80,"max_rpm":90}}"#,
            "cadence range",
        ),
        (
            r#"{"type":"steady","duration_seconds":300,"power":{"type":"percent_ftp","percent":65},
                "cues":[{"offset_seconds":0,"message":"Settle in"}]}"#,
            "step cues",
        ),
        (
            r#"{"type":"repeat","count":2,"steps":[{"type":"repeat","count":2,"steps":[
                {"type":"steady","duration_seconds":60,"power":{"type":"percent_ftp","percent":100}}]}]}"#,
            "repeat inside another repeat",
        ),
    ];
    for (steps, expected) in cases {
        let error = from_workout_spec(&spec(steps)).unwrap_err();
        assert!(
            error.message.contains(expected),
            "{steps}: expected reason mentioning {expected:?}, got {:?}",
            error.message
        );
    }
}

#[test]
fn running_definitions_are_refused() {
    let definition = WorkoutSpec::from_json(include_str!(
        "../../../testdata/workout-spec/running-time.json"
    ))
    .unwrap();
    let error = from_workout_spec(&definition).unwrap_err();
    assert!(error.message.contains("cycling"));
}
