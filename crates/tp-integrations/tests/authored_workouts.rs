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
            }],
        }],
    };
    let mut body = w.payload().unwrap();
    assert_eq!(body["sportType"]["sportTypeId"], 2);
    let step = &body["workoutSegments"][0]["workoutSteps"][0]["workoutSteps"][0];
    assert_eq!(step["targetValueOne"], 90.0);
    assert_eq!(step["targetValueUnit"]["unitId"], 253);
    assert_eq!(step["secondaryTargetValueTwo"], 95);
    assert_eq!(body["estimatedDurationInSecs"], 120);
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
        }],
        ..w
    };
    assert!(invalid.payload().is_err());
}
