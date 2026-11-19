use tp_integrations::garmin::workouts::*;
#[test]
fn cycling_authoring_preserves_relative_ranges_repeats_and_cadence() {
    let w = CyclingWorkout {
        name: "Threshold".into(),
        description: "Synthetic".into(),
        steps: vec![CyclingStep::Repeat {
            count: 2,
            steps: vec![CyclingStep::Timed {
                seconds: 60,
                power: Some(CyclingPower::PercentFtp {
                    low: 90.0,
                    high: 100.0,
                }),
                cadence: Some((85, 95)),
                notes: Some("Smooth circles".into()),
            }],
        }],
    };
    let mut body = w.payload().unwrap();
    assert_eq!(body["sportType"]["sportTypeId"], 2);
    let step = &body["workoutSegments"][0]["workoutSteps"][0]["workoutSteps"][0];
    assert_eq!(step["targetValueOne"], 90.0);
    assert_eq!(step["targetValueUnit"]["unitId"], 253);
    assert_eq!(step["secondaryTargetValueTwo"], 95);
    assert_eq!(step["description"], "Smooth circles");
    assert_eq!(body["estimatedDurationInSecs"], 120);
    assert!(w.matches(&body).unwrap());
    body["workoutSegments"][0]["workoutSteps"][0]["workoutSteps"][0]["description"] =
        serde_json::json!("Smooth circles, edited");
    assert!(!w.matches(&body).unwrap());
    body["workoutSegments"][0]["workoutSteps"][0]["workoutSteps"][0]["description"] =
        serde_json::json!("Smooth circles");
    assert!(w.matches(&body).unwrap());
    body["workoutSegments"][0]["workoutSteps"][0]["workoutSteps"][0]["targetValueUnit"]["factor"] =
        serde_json::json!(0.1);
    assert!(!w.matches(&body).unwrap());
    body["workoutSegments"][0]["workoutSteps"][0]["workoutSteps"][0]["targetValueUnit"] =
        serde_json::Value::Null;
    assert!(!w.matches(&body).unwrap());
    let invalid = CyclingWorkout {
        steps: vec![CyclingStep::Timed {
            seconds: 60,
            power: Some(CyclingPower::PercentFtp {
                low: f64::NAN,
                high: 100.0,
            }),
            cadence: None,
            notes: None,
        }],
        ..w
    };
    assert!(invalid.payload().is_err());
}

#[test]
fn cycling_steps_without_notes_accept_absent_or_empty_remote_descriptions() {
    let w = CyclingWorkout {
        name: "Endurance".into(),
        description: "Synthetic".into(),
        steps: vec![CyclingStep::Timed {
            seconds: 600,
            power: None,
            cadence: None,
            notes: None,
        }],
    };
    let mut body = w.payload().unwrap();
    assert!(body["workoutSegments"][0]["workoutSteps"][0]
        .get("description")
        .is_none());
    body["workoutSegments"][0]["workoutSteps"][0]["description"] = serde_json::json!("");
    assert!(w.matches(&body).unwrap());
    body["workoutSegments"][0]["workoutSteps"][0]["description"] =
        serde_json::json!("added remotely");
    assert!(!w.matches(&body).unwrap());
    let stored: CyclingWorkout = serde_json::from_str(
        r#"{"name":"legacy","description":"","steps":[{"type":"timed","seconds":60,"power":null,"cadence":null}]}"#,
    )
    .unwrap();
    assert!(matches!(
        stored.steps[0],
        CyclingStep::Timed { notes: None, .. }
    ));
}
