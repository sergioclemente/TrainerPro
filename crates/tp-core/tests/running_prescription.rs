use serde_json::{json, Value};
use tp_core::{
    model::{PowerTarget, WorkoutSegment},
    workout_definition::{
        RunningStep, RunningTarget, WorkoutDefinition, WorkoutDefinitionError, WorkoutPrescription,
        TPW_VERSION,
    },
};

const CYCLING: &str = include_str!("../../../testdata/tpw/cycling-cadence.json");
const TIMED: &str = include_str!("../../../testdata/tpw/running-time.json");
const DISTANCE: &str = include_str!("../../../testdata/tpw/running-distance.json");

fn run(step: Value) -> String {
    json!({"format":"TPW","version":1,"title":"Synthetic validation case",
        "prescription":{"sport":"running","steps":[step]}})
    .to_string()
}

fn timed_step() -> Value {
    json!({"type":"step","end":{"type":"time","duration_seconds":60},"target":{"type":"open"}})
}

#[test]
fn private_format_keeps_version_one_and_existing_cycling_execution() {
    assert_eq!(TPW_VERSION, 1);
    let workout = WorkoutDefinition::from_json(CYCLING).unwrap();
    let executable = workout.compile().unwrap();
    assert_eq!(executable.duration_s(), 900);
    assert_eq!(executable.segments.len(), 6);
    assert!(matches!(
        executable.segments[0],
        WorkoutSegment::Steady {
            duration_s: 180,
            power: PowerTarget::PercentFtp(1.06),
            cadence_rpm: Some(95)
        }
    ));
    assert_eq!(
        WorkoutDefinition::from_json(&workout.to_json_pretty().unwrap()).unwrap(),
        workout
    );
}

#[test]
fn running_prescriptions_round_trip_without_losing_intent() {
    for fixture in [TIMED, DISTANCE] {
        let definition = WorkoutDefinition::from_json(fixture).unwrap();
        definition.validate().unwrap();
        let restored = WorkoutDefinition::from_json(&definition.to_json_pretty().unwrap()).unwrap();
        assert_eq!(definition, restored);
        assert!(matches!(
            definition.prescription,
            WorkoutPrescription::Running { .. }
        ));
    }
}

#[test]
fn timed_running_duration_includes_repetitions_and_recoveries() {
    assert_eq!(
        WorkoutDefinition::from_json(TIMED)
            .unwrap()
            .duration_seconds()
            .unwrap(),
        1800
    );
}

#[test]
fn distance_target_does_not_turn_prescribed_pace_into_known_duration() {
    assert!(matches!(
        WorkoutDefinition::from_json(DISTANCE)
            .unwrap()
            .duration_seconds(),
        Err(WorkoutDefinitionError::UnknownDuration)
    ));
}

#[test]
fn valid_running_is_not_executable_by_the_cycling_player() {
    for fixture in [TIMED, DISTANCE] {
        assert!(
            matches!(WorkoutDefinition::from_json(fixture).unwrap().compile(),
            Err(WorkoutDefinitionError::UnsupportedExecutionSport { sport }) if sport == "running")
        );
    }
}

#[test]
fn step_endings_must_be_positive_and_have_one_explicit_unit() {
    for end in [
        json!({"type":"time","duration_seconds":0}),
        json!({"type":"distance","distance_meters":0}),
        json!({"type":"time","duration_seconds":60,"distance_meters":400}),
        json!({"type":"distance","distance_meters":1.5}),
    ] {
        let mut step = timed_step();
        step["end"] = end;
        assert!(WorkoutDefinition::from_json(&run(step)).is_err());
    }
}

#[test]
fn target_units_and_ranges_are_validated_without_silent_conversion() {
    for target in [
        json!({"type":"pace","seconds_per_km":0}),
        json!({"type":"pace","seconds_per_km":-300}),
        json!({"type":"pace_range","fast_seconds_per_km":360,"slow_seconds_per_km":300}),
        json!({"type":"heart_rate","bpm":0}),
        json!({"type":"heart_rate","bpm":301}),
        json!({"type":"heart_rate_range","min_bpm":160,"max_bpm":140}),
        json!({"type":"pace","seconds_per_km":300,"watts":200}),
        json!({"type":"percent_ftp","percent":100}),
    ] {
        let mut step = timed_step();
        step["target"] = target;
        assert!(WorkoutDefinition::from_json(&run(step)).is_err());
    }
}

#[test]
fn direct_rust_construction_cannot_bypass_finite_pace_validation() {
    for pace in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut definition = WorkoutDefinition::from_json(&run(timed_step())).unwrap();
        let WorkoutPrescription::Running { steps } = &mut definition.prescription else {
            unreachable!()
        };
        let RunningStep::Step { target, .. } = &mut steps[0] else {
            unreachable!()
        };
        *target = RunningTarget::Pace {
            seconds_per_km: pace,
        };
        assert!(definition.validate().is_err());
    }
}

#[test]
fn running_cadence_is_both_feet_steps_per_minute_not_cycling_rpm() {
    for cadence in [
        json!({"type":"exact","rpm":90}),
        json!({"type":"exact","steps_per_minute":0}),
        json!({"type":"range","min_steps_per_minute":180,"max_steps_per_minute":170}),
        json!({"type":"exact","steps_per_minute":401}),
    ] {
        let mut step = timed_step();
        step["cadence"] = cadence;
        assert!(WorkoutDefinition::from_json(&run(step)).is_err());
    }
}

#[test]
fn distance_cues_are_at_step_start_and_time_cues_stay_within_the_step() {
    let mut step = timed_step();
    step["cues"] = json!([{"offset_seconds":59,"message":"Almost done"}]);
    assert!(WorkoutDefinition::from_json(&run(step.clone())).is_ok());
    step["cues"][0]["offset_seconds"] = json!(60);
    assert!(WorkoutDefinition::from_json(&run(step.clone())).is_err());
    step["end"] = json!({"type":"distance","distance_meters":1000});
    assert!(WorkoutDefinition::from_json(&run(step.clone())).is_err());
    step["cues"][0]["offset_seconds"] = json!(0);
    assert!(WorkoutDefinition::from_json(&run(step)).is_ok());
}

#[test]
fn running_resource_limits_apply_before_any_execution_expansion() {
    let mut nested = timed_step();
    for _ in 0..8 {
        nested = json!({"type":"repeat","count":1,"steps":[nested]});
    }
    assert!(WorkoutDefinition::from_json(&run(nested.clone())).is_ok());
    assert!(WorkoutDefinition::from_json(&run(
        json!({"type":"repeat","count":1,"steps":[nested]})
    ))
    .is_err());
    for count in [0, 101] {
        assert!(WorkoutDefinition::from_json(&run(
            json!({"type":"repeat","count":count,"steps":[timed_step()]})
        ))
        .is_err());
    }
    let explosion = json!({"type":"repeat","count":100,"steps":[{"type":"repeat","count":100,"steps":[{"type":"repeat","count":2,"steps":[timed_step()]}]}]});
    assert!(WorkoutDefinition::from_json(&run(explosion)).is_err());
    for end in [
        json!({"type":"time","duration_seconds":u32::MAX}),
        json!({"type":"distance","distance_meters":u32::MAX}),
    ] {
        let mut step = timed_step();
        step["end"] = end;
        assert!(WorkoutDefinition::from_json(&run(
            json!({"type":"repeat","count":2,"steps":[step]})
        ))
        .is_err());
    }
}
