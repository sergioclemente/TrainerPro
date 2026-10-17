use tp_core::consts::{TEXT_EVENT_DEFAULT_S, WORKOUT_REPEAT_COUNT_MAX, WORKOUT_REPEAT_DEPTH_MAX};
use tp_core::model::{ExecutableWorkout, PowerTarget, TextEvent, WorkoutSegment};
use tp_core::workout_spec::{
    CyclingCadenceTarget, CyclingPowerTarget, CyclingStep, WorkoutCue, WorkoutFormat,
    WorkoutPrescription, WorkoutSpec, WorkoutSpecError, WORKOUT_SPEC_VERSION,
};

fn definition(steps: Vec<CyclingStep>) -> WorkoutSpec {
    WorkoutSpec {
        format: WorkoutFormat::WorkoutSpec,
        version: WORKOUT_SPEC_VERSION,
        title: "Aerobic Builder".into(),
        description: "Smooth, controlled work".into(),
        training_focus: Some("Endurance Base".into()),
        prescription: WorkoutPrescription::Cycling { steps },
    }
}

fn steady(duration_seconds: u32, percent: f64) -> CyclingStep {
    CyclingStep::Steady {
        duration_seconds,
        power: CyclingPowerTarget::PercentFtp { percent },
        cadence: None,
        cues: vec![],
    }
}

#[test]
fn json_is_explicit_versioned_and_round_trips() {
    let workout = definition(vec![CyclingStep::Steady {
        duration_seconds: 600,
        power: CyclingPowerTarget::PercentFtpRange {
            min_percent: 65.0,
            max_percent: 75.0,
        },
        cadence: Some(CyclingCadenceTarget::Range {
            min_rpm: 85,
            max_rpm: 95,
        }),
        cues: vec![WorkoutCue {
            offset_seconds: 0,
            message: "Relax your shoulders".into(),
            display_seconds: 10,
        }],
    }]);

    let json = workout.to_json_pretty().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        serde_json::json!({
            "format": "WorkoutSpec",
            "version": 1,
            "title": "Aerobic Builder",
            "description": "Smooth, controlled work",
            "training_focus": "Endurance Base",
            "prescription": {
                "sport": "cycling",
                "steps": [{
                    "type": "steady",
                    "duration_seconds": 600,
                    "power": {
                        "type": "percent_ftp_range",
                        "min_percent": 65.0,
                        "max_percent": 75.0
                    },
                    "cadence": {
                        "type": "range",
                        "min_rpm": 85,
                        "max_rpm": 95
                    },
                    "cues": [{
                        "offset_seconds": 0,
                        "message": "Relax your shoulders",
                        "display_seconds": 10
                    }]
                }]
            }
        })
    );
    assert_eq!(WorkoutSpec::from_json(&json).unwrap(), workout);
}

#[test]
fn json_defaults_optional_metadata_collections_and_cue_duration() {
    let json = r#"{
      "format": "WorkoutSpec",
      "version": 1,
      "title": "Endurance",
      "prescription": {
        "sport": "cycling",
        "steps": [{
          "type": "steady",
          "duration_seconds": 600,
          "power": {"type": "percent_ftp", "percent": 70},
          "cues": [{"offset_seconds": 0, "message": "Settle in"}]
        }]
      }
    }"#;

    let workout = WorkoutSpec::from_json(json).unwrap();
    assert_eq!(workout.description, "");
    assert_eq!(workout.training_focus, None);
    let WorkoutPrescription::Cycling { steps } = &workout.prescription else {
        panic!("expected cycling prescription");
    };
    let CyclingStep::Steady { cadence, cues, .. } = &steps[0] else {
        panic!("expected steady step");
    };
    assert_eq!(*cadence, None);
    assert_eq!(cues[0].display_seconds, TEXT_EVENT_DEFAULT_S);
}

#[test]
fn compiles_nested_repeats_ranges_and_step_local_cues() {
    let workout = definition(vec![
        steady(300, 55.0),
        CyclingStep::Repeat {
            count: 2,
            steps: vec![
                CyclingStep::Steady {
                    duration_seconds: 60,
                    power: CyclingPowerTarget::WattsRange {
                        min_watts: 299,
                        max_watts: 300,
                    },
                    cadence: Some(CyclingCadenceTarget::Range {
                        min_rpm: 89,
                        max_rpm: 90,
                    }),
                    cues: vec![WorkoutCue {
                        offset_seconds: 10,
                        message: "Strong and smooth".into(),
                        display_seconds: 8,
                    }],
                },
                steady(120, 50.0),
            ],
        },
    ]);

    let executable = workout.compile().unwrap();
    assert_eq!(executable.name, "Aerobic Builder");
    assert_eq!(executable.duration_s(), 660);
    assert_eq!(executable.segments.len(), 5);
    assert_eq!(
        executable.segments[1],
        WorkoutSegment::Steady {
            duration_s: 60,
            power: PowerTarget::Watts(300),
            cadence_rpm: Some(90),
        }
    );
    assert_eq!(
        executable
            .text_events
            .iter()
            .map(|cue| cue.offset_s)
            .collect::<Vec<_>>(),
        vec![310, 490]
    );
}

#[test]
fn compiled_definition_drives_the_existing_engine() {
    let workout = definition(vec![CyclingStep::Steady {
        duration_seconds: 60,
        power: CyclingPowerTarget::PercentFtpRange {
            min_percent: 65.0,
            max_percent: 75.0,
        },
        cadence: None,
        cues: vec![],
    }]);
    let mut engine = tp_core::engine::Engine::new(workout.compile().unwrap(), 200, 1.0);

    assert_eq!(
        engine.step(tp_core::engine::EngineEvent::Start),
        vec![
            tp_core::engine::EngineAction::StartTrainer,
            tp_core::engine::EngineAction::SetTargetPower { watts: 140 },
        ]
    );
}

#[test]
fn normalizes_flat_executable_workout_without_file_provenance() {
    let executable = ExecutableWorkout {
        name: "Imported workout".into(),
        description: "From a boundary parser".into(),
        segments: vec![
            WorkoutSegment::Steady {
                duration_s: 60,
                power: PowerTarget::PercentFtp(0.75),
                cadence_rpm: Some(90),
            },
            WorkoutSegment::FreeRide {
                duration_s: 30,
                cadence_rpm: None,
            },
        ],
        text_events: vec![TextEvent {
            offset_s: 65,
            message: "Choose your effort".into(),
            duration_s: 8,
        }],
    };

    let definition = WorkoutSpec::from_executable(executable.clone()).unwrap();
    assert_eq!(definition.title, "Imported workout");
    assert_eq!(definition.compile().unwrap(), executable);
    let WorkoutPrescription::Cycling { steps } = definition.prescription else {
        panic!("expected cycling prescription");
    };
    let CyclingStep::FreeRide { cues, .. } = &steps[1] else {
        panic!("expected free-ride step");
    };
    assert_eq!(cues[0].offset_seconds, 5);
}

#[test]
fn rejects_unsupported_versions_invalid_ranges_and_unsafe_repeats() {
    let mut workout = definition(vec![steady(60, 75.0)]);
    workout.version += 1;
    assert!(matches!(
        workout.validate(),
        Err(WorkoutSpecError::UnsupportedVersion { .. })
    ));

    let bad_range = definition(vec![CyclingStep::Steady {
        duration_seconds: 60,
        power: CyclingPowerTarget::PercentFtpRange {
            min_percent: 90.0,
            max_percent: 80.0,
        },
        cadence: None,
        cues: vec![],
    }]);
    assert!(matches!(
        bad_range.validate(),
        Err(WorkoutSpecError::Invalid { path, .. })
            if path == "prescription.steps[0].power"
    ));

    let explosive = definition(vec![CyclingStep::Repeat {
        count: WORKOUT_REPEAT_COUNT_MAX,
        steps: vec![CyclingStep::Repeat {
            count: WORKOUT_REPEAT_COUNT_MAX,
            steps: vec![CyclingStep::Repeat {
                count: 2,
                steps: vec![steady(1, 75.0)],
            }],
        }],
    }]);
    assert!(matches!(
        explosive.validate(),
        Err(WorkoutSpecError::Invalid { path, .. })
            if path == "prescription.steps"
    ));

    let mut deeply_nested = steady(1, 75.0);
    for _ in 0..=WORKOUT_REPEAT_DEPTH_MAX {
        deeply_nested = CyclingStep::Repeat {
            count: 1,
            steps: vec![deeply_nested],
        };
    }
    assert!(matches!(
        definition(vec![deeply_nested]).validate(),
        Err(WorkoutSpecError::Invalid { message, .. })
            if message.contains("nesting")
    ));
}

#[test]
fn rejects_unknown_json_fields_instead_of_silently_dropping_them() {
    let json = r#"{
      "format": "WorkoutSpec",
      "version": 1,
      "title": "Endurance",
      "prescription": {
        "sport": "cycling",
        "steps": [{
          "type": "steady",
          "duration_seconds": 600,
          "power": {"type": "percent_ftp", "percent": 70},
          "mystery_target": 42
        }]
      }
    }"#;
    assert!(matches!(
        WorkoutSpec::from_json(json),
        Err(WorkoutSpecError::InvalidJson { .. })
    ));
}

#[test]
fn future_json_versions_are_reported_before_their_fields_are_parsed() {
    let json = r#"{
      "format": "WorkoutSpec",
      "version": 2,
      "brand_new_v2_field": true
    }"#;
    assert!(matches!(
        WorkoutSpec::from_json(json),
        Err(WorkoutSpecError::UnsupportedVersion {
            found: 2,
            supported: WORKOUT_SPEC_VERSION
        })
    ));
}

#[test]
fn other_formats_are_reported_before_their_fields_are_parsed() {
    let json = r#"{
      "format": "ZWO",
      "version": 1,
      "zwo_specific_field": true
    }"#;
    assert!(matches!(
        WorkoutSpec::from_json(json),
        Err(WorkoutSpecError::UnsupportedFormat { found }) if found == "ZWO"
    ));
}

#[test]
fn legacy_tpw_reads_as_workout_spec_and_writes_the_canonical_name() {
    for fixture in [
        include_str!("../../../testdata/workout-spec/cycling-cadence.json"),
        include_str!("../../../testdata/workout-spec/running-time.json"),
        include_str!("../../../testdata/workout-spec/running-distance.json"),
    ] {
        let mut legacy: serde_json::Value = serde_json::from_str(fixture).unwrap();
        legacy["format"] = serde_json::json!("TPW");
        legacy["description"] = serde_json::json!("Keep TPW in authored text");
        let legacy_json = serde_json::to_string(&legacy).unwrap();
        let spec = WorkoutSpec::from_json(&legacy_json).unwrap();
        let direct: WorkoutSpec = serde_json::from_str(&legacy_json).unwrap();
        assert_eq!(spec, direct);
        direct.validate().unwrap();
        let canonical: serde_json::Value =
            serde_json::from_str(&spec.to_json_pretty().unwrap()).unwrap();
        legacy["format"] = serde_json::json!("WorkoutSpec");
        // Normalize optional defaults through the same public reader.
        let expected = WorkoutSpec::from_json(&legacy.to_string()).unwrap();
        assert_eq!(spec, expected);
        assert_eq!(canonical["format"], "WorkoutSpec");
        assert_eq!(canonical["version"], 1);
        assert_eq!(canonical["description"], "Keep TPW in authored text");
        assert_eq!(
            serde_json::to_value(&direct).unwrap()["format"],
            "WorkoutSpec"
        );
    }
}

#[test]
fn both_format_names_enforce_version_and_prescription_validation() {
    for format in ["TPW", "WorkoutSpec"] {
        let future = serde_json::json!({"format":format,"version":2,"future_field":true});
        assert!(matches!(
            WorkoutSpec::from_json(&future.to_string()),
            Err(WorkoutSpecError::UnsupportedVersion {
                found: 2,
                supported: 1
            })
        ));
        let mut invalid: serde_json::Value = serde_json::from_str(include_str!(
            "../../../testdata/workout-spec/cycling-cadence.json"
        ))
        .unwrap();
        invalid["format"] = serde_json::json!(format);
        invalid["prescription"]["steps"] = serde_json::json!([]);
        assert!(matches!(
            WorkoutSpec::from_json(&invalid.to_string()),
            Err(WorkoutSpecError::Invalid { .. })
        ));
        invalid["unexpected"] = serde_json::json!(true);
        assert!(matches!(
            WorkoutSpec::from_json(&invalid.to_string()),
            Err(WorkoutSpecError::InvalidJson { .. })
        ));
    }
    for format in ["tpw", "workoutspec", "Other"] {
        let json = serde_json::json!({"format":format,"version":1});
        assert!(matches!(
            WorkoutSpec::from_json(&json.to_string()),
            Err(WorkoutSpecError::UnsupportedFormat { .. })
        ));
    }
}
