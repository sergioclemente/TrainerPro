use tp_core::model::{ExecutableWorkout, PowerTarget, StepRole, WorkoutSegment};

fn steady(pct: f64) -> WorkoutSegment {
    WorkoutSegment::Steady {
        duration_s: 60,
        power: PowerTarget::PercentFtp(pct / 100.0),
        cadence_rpm: None,
    }
}

fn ramp(from_pct: f64, to_pct: f64) -> WorkoutSegment {
    WorkoutSegment::Ramp {
        duration_s: 60,
        start: PowerTarget::PercentFtp(from_pct / 100.0),
        end: PowerTarget::PercentFtp(to_pct / 100.0),
        cadence_rpm: None,
    }
}

fn workout(segments: Vec<WorkoutSegment>) -> ExecutableWorkout {
    ExecutableWorkout {
        name: "roles".into(),
        description: String::new(),
        segments,
        text_events: vec![],
    }
}

#[test]
fn step_roles_follow_position_and_effort() {
    let w = workout(vec![
        ramp(40.0, 60.0),
        steady(105.0),
        steady(50.0),
        steady(50.0),
        ramp(60.0, 80.0),
        WorkoutSegment::FreeRide {
            duration_s: 60,
            cadence_rpm: None,
        },
        ramp(70.0, 40.0),
    ]);
    assert_eq!(
        w.step_roles(250),
        vec![
            StepRole::WarmUp,
            StepRole::Interval,
            // Easy spinning right after harder work.
            StepRole::Recovery,
            // Easy spinning after easy spinning is just steady.
            StepRole::Steady,
            StepRole::Ramp,
            StepRole::FreeRide,
            StepRole::CoolDown,
        ]
    );
    assert_eq!(StepRole::WarmUp.label(), "Warm-up");
    assert_eq!(StepRole::FreeRide.label(), "Free ride");
}

#[test]
fn rising_ramp_is_a_warm_up_only_at_the_start() {
    let w = workout(vec![steady(70.0), ramp(40.0, 60.0)]);
    assert_eq!(w.step_roles(250), vec![StepRole::Steady, StepRole::Ramp]);
    // A single falling ramp is first and last: not a warm-up, so a cool-down.
    let w = workout(vec![ramp(60.0, 40.0)]);
    assert_eq!(w.step_roles(250), vec![StepRole::CoolDown]);
}

#[test]
fn watt_targets_classify_at_the_riders_ftp() {
    let w = workout(vec![WorkoutSegment::Steady {
        duration_s: 60,
        power: PowerTarget::Watts(220),
        cadence_rpm: None,
    }]);
    assert_eq!(w.step_roles(250), vec![StepRole::Interval], "88 % of 250");
    assert_eq!(w.step_roles(300), vec![StepRole::Steady], "73 % of 300");
}
