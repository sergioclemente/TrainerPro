use super::*;
use serde_json::json;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../testdata/providers/garmin/scheduled-cycling.json"
    ))
    .unwrap()
}

#[test]
fn imports_repeats_units_cadence_cues_and_freeride_without_trusting_estimates() {
    let spec = to_workout_spec(&fixture()).unwrap();
    assert_eq!(spec.duration_seconds().unwrap(), 285);
    let compiled = spec.compile().unwrap();
    assert_eq!(compiled.segments.len(), 6);
    assert_eq!(compiled.target_power_w_at(0, 200, 1.0), None);
    assert_eq!(compiled.target_power_w_at(60, 200, 1.0), Some(190));
    assert_eq!(compiled.target_power_w_at(60, 300, 1.0), Some(285));
    assert_eq!(compiled.target_power_w_at(120, 300, 1.0), Some(110));
    assert_eq!(compiled.target_cadence_rpm_at(60), Some(90));
    assert_eq!(compiled.text_events.len(), 2);
    assert_eq!(compiled.text_events[0].message, "Hold a smooth cadence");
    assert_eq!(
        WorkoutSpec::from_json(&spec.to_json_pretty().unwrap()).unwrap(),
        spec
    );
}

#[test]
fn distinguishes_low_watts_from_identical_percent_values() {
    let mut workout = fixture();
    let relative = to_workout_spec(&workout).unwrap().compile().unwrap();
    workout["workoutSegments"][0]["workoutSteps"][1]["workoutSteps"][0]["targetValueUnit"] =
        Value::Null;
    let absolute = to_workout_spec(&workout).unwrap().compile().unwrap();
    assert_eq!(relative.target_power_w_at(60, 200, 1.0), Some(190));
    assert_eq!(absolute.target_power_w_at(60, 200, 1.0), Some(95));
}

#[test]
fn rejects_unsupported_prescriptions_without_silently_dropping_them() {
    for (pointer, value) in [
        ("/sportType/sportTypeKey", json!("running")),
        (
            "/workoutSegments/0/workoutSteps/0/endCondition/conditionTypeKey",
            json!("lap.button"),
        ),
        (
            "/workoutSegments/0/workoutSteps/0/endConditionValue",
            json!(1.5),
        ),
        (
            "/workoutSegments/0/workoutSteps/0/endConditionValue",
            json!(0),
        ),
        ("/workoutSegments/0/workoutSteps/1/smartRepeat", json!(true)),
        (
            "/workoutSegments/0/workoutSteps/1/skipLastRestStep",
            json!(true),
        ),
        (
            "/workoutSegments/0/workoutSteps/1/numberOfIterations",
            json!(3),
        ),
        (
            "/workoutSegments/0/workoutSteps/1/workoutSteps/0/stepOrder",
            json!(1),
        ),
        (
            "/workoutSegments/0/workoutSteps/1/workoutSteps/0/targetValueOne",
            json!(101),
        ),
        (
            "/workoutSegments/0/workoutSteps/1/workoutSteps/0/targetValueUnit/unitKey",
            json!("unknown"),
        ),
        (
            "/workoutSegments/0/workoutSteps/1/workoutSteps/0/targetValueUnit/factor",
            json!(100),
        ),
        (
            "/workoutSegments/0/workoutSteps/1/workoutSteps/0/targetType/workoutTargetTypeKey",
            json!("heart.rate.zone"),
        ),
    ] {
        let mut workout = fixture();
        *workout.pointer_mut(pointer).unwrap() = value;
        assert!(to_workout_spec(&workout).is_err(), "accepted {pointer}");
    }
    for (field, value) in [
        ("zoneNumber", json!(3)),
        ("futureControlTarget", json!({"value":1})),
        ("targetValueUnit", json!({"unitKey":"watts","factor":100})),
    ] {
        let mut workout = fixture();
        workout["workoutSegments"][0]["workoutSteps"][1]["workoutSteps"][0][field] = value;
        assert!(to_workout_spec(&workout).is_err(), "accepted {field}");
    }
}

#[test]
fn cadence_only_remains_freeride_and_unknown_secondary_targets_are_rejected() {
    let mut workout = fixture();
    let step = &mut workout["workoutSegments"][0]["workoutSteps"][1]["workoutSteps"][0];
    step["targetType"] = json!({"workoutTargetTypeId":1,"workoutTargetTypeKey":"no.target"});
    step["targetValueOne"] = Value::Null;
    step["targetValueTwo"] = Value::Null;
    step["targetValueUnit"] = Value::Null;
    let compiled = to_workout_spec(&workout).unwrap().compile().unwrap();
    assert_eq!(compiled.target_power_w_at(60, 200, 1.0), None);
    assert_eq!(compiled.target_cadence_rpm_at(60), Some(90));
    workout["workoutSegments"][0]["workoutSteps"][1]["workoutSteps"][0]["secondaryTargetType"] =
        json!({"workoutTargetTypeId":4,"workoutTargetTypeKey":"heart.rate.zone"});
    assert!(to_workout_spec(&workout).is_err());
}

#[test]
fn nested_fixed_repeats_compile_in_global_step_order() {
    let mut workout = fixture();
    let mut leaf = workout["workoutSegments"][0]["workoutSteps"][2].clone();
    leaf["stepOrder"] = json!(3);
    let mut inner = workout["workoutSegments"][0]["workoutSteps"][1].clone();
    inner["workoutSteps"] = json!([leaf]);
    let mut outer = inner.clone();
    outer["stepOrder"] = json!(1);
    outer["workoutSteps"] = json!([inner]);
    workout["workoutSegments"][0]["workoutSteps"] = json!([outer]);
    let compiled = to_workout_spec(&workout).unwrap().compile().unwrap();
    assert_eq!(compiled.duration_s(), 180);
    assert_eq!(compiled.segments.len(), 4);
}

#[test]
fn garmin_editor_neutral_equipment_and_stroke_descriptors_do_not_change_execution() {
    let mut workout = fixture();
    let expected = to_workout_spec(&workout).unwrap();
    let step = &mut workout["workoutSegments"][0]["workoutSteps"][0];
    step["equipmentType"] = json!({"equipmentTypeId":0,"displayOrder":0});
    step["strokeType"] = json!({"strokeTypeId":0,"displayOrder":0});
    assert_eq!(to_workout_spec(&workout).unwrap(), expected);
    for (field, id_field) in [
        ("equipmentType", "equipmentTypeId"),
        ("strokeType", "strokeTypeId"),
    ] {
        let mut prescribed = workout.clone();
        prescribed["workoutSegments"][0]["workoutSteps"][0][field][id_field] = json!(1);
        assert!(to_workout_spec(&prescribed).is_err());
        prescribed["workoutSegments"][0]["workoutSteps"][0][field] = json!({"displayOrder":0});
        assert!(to_workout_spec(&prescribed).is_err());
    }
}
