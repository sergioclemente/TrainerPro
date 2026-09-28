//! Player IPC commands and the shared load-into-player flow.

use serde::Serialize;
use tauri::{AppHandle, State};
use tp_core::model::ExecutableWorkout;

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::commands::workout::{graph_points, load_workout_definition, segment_rows, SegmentRow};
use crate::database::scheduled_workouts as scheduled_db;
use crate::player_runtime::{self as runtime, ActivitySummary, PlayerCommand, PlayerState};

type R<T> = Result<T, AppError>;

#[derive(Serialize)]
pub struct PlayerWorkoutProfile {
    pub workout_session_id: String,
    pub graph: Vec<(u32, f64)>,
    pub segments: Vec<SegmentRow>,
    pub ftp_w: u16,
}

fn profile_for_session(
    captured_session_id: &str,
    requested_session_id: &str,
    workout: &ExecutableWorkout,
    ftp_w: u16,
) -> Option<PlayerWorkoutProfile> {
    (captured_session_id == requested_session_id).then(|| PlayerWorkoutProfile {
        workout_session_id: captured_session_id.to_owned(),
        graph: graph_points(workout, ftp_w),
        segments: segment_rows(workout, ftp_w),
        ftp_w,
    })
}

#[tauri::command]
pub async fn load_workout(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    scheduled_workout_id: Option<String>,
) -> R<PlayerState> {
    load_workout_into_player(app, &state, &id, scheduled_workout_id.as_deref()).await
}

/// Shared definition-to-player flow, also used after Planner and WOZ import.
pub async fn load_workout_into_player(
    app: AppHandle,
    state: &State<'_, AppState>,
    id: &str,
    scheduled_workout_id: Option<&str>,
) -> R<PlayerState> {
    if let Some(scheduled_workout_id) = scheduled_workout_id {
        let scheduled = {
            let conn = state.db.lock().unwrap();
            scheduled_db::get(&conn, scheduled_workout_id)?
        }
        .ok_or_else(|| {
            AppError::new(
                "schedule_not_found",
                format!("scheduled workout {scheduled_workout_id} not found"),
            )
        })?;
        if scheduled.removed_at_unix_ms.is_some() {
            return Err(AppError::new(
                "schedule_removed",
                "this workout is no longer scheduled",
            ));
        }
        if scheduled.workout_definition_id != id {
            return Err(AppError::new(
                "schedule_mismatch",
                "scheduled workout does not reference this workout definition",
            ));
        }
    }
    let definition = load_workout_definition(state, id)?;
    let mut player = state.player.lock().await;
    if let Some(h) = player.as_ref() {
        let phase = h.state_rx.borrow().phase.clone();
        if phase == "ready" || phase == "finished" {
            // Never-started (or already-finalized) ride: nothing worth
            // keeping — drop it and load the new workout. Dropping the
            // handle closes the command channel and the runtime task exits.
            *player = None;
        } else {
            return Err(AppError::new(
                "ride_active",
                "a ride is in progress — end it first",
            ));
        }
    }
    let handle = runtime::spawn(
        app,
        id.to_string(),
        scheduled_workout_id.map(str::to_owned),
        definition,
    )
    .await?;
    let ps = handle.state_rx.borrow().clone();
    *player = Some(handle);
    Ok(ps)
}

async fn send_player_command(state: &State<'_, AppState>, command: PlayerCommand) -> R<()> {
    let player = state.player.lock().await;
    let handle = player
        .as_ref()
        .ok_or_else(|| AppError::new("no_ride", "no ride loaded"))?;
    handle
        .command_tx
        .send(command)
        .await
        .map_err(|_| AppError::new("no_ride", "player is gone"))
}

#[tauri::command]
pub async fn start_ride(state: State<'_, AppState>) -> R<()> {
    send_player_command(&state, PlayerCommand::Start).await
}
#[tauri::command]
pub async fn pause_ride(state: State<'_, AppState>) -> R<()> {
    send_player_command(&state, PlayerCommand::Pause).await
}
#[tauri::command]
pub async fn resume_ride(state: State<'_, AppState>) -> R<()> {
    send_player_command(&state, PlayerCommand::Resume).await
}
#[tauri::command]
pub async fn skip_segment(state: State<'_, AppState>) -> R<()> {
    send_player_command(&state, PlayerCommand::SkipSegment).await
}
#[tauri::command]
pub async fn set_intensity(state: State<'_, AppState>, pct: f64) -> R<()> {
    send_player_command(&state, PlayerCommand::SetIntensity(pct)).await
}

#[tauri::command]
pub async fn set_erg(state: State<'_, AppState>, enabled: bool) -> R<()> {
    send_player_command(&state, PlayerCommand::SetErg(enabled)).await
}

#[tauri::command]
pub async fn end_ride(state: State<'_, AppState>) -> R<ActivitySummary> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_player_command(&state, PlayerCommand::End(tx)).await?;
    let summary = rx
        .await
        .map_err(|_| AppError::new("no_ride", "player exited before summary"))??;
    *state.player.lock().await = None;
    Ok(summary)
}

/// Drop the player slot after a naturally-completed ride (runtime already
/// finalized and emitted `activity_recorded`).
#[tauri::command]
pub async fn clear_ride(state: State<'_, AppState>) -> R<()> {
    *state.player.lock().await = None;
    Ok(())
}

#[tauri::command]
pub async fn get_player_state(state: State<'_, AppState>) -> R<Option<PlayerState>> {
    let player = state.player.lock().await;
    Ok(player.as_ref().map(|h| h.state_rx.borrow().clone()))
}

/// One-time view of the workout captured by the active player, not the
/// mutable library row identified by `workout_definition_id`.
#[tauri::command]
pub async fn get_player_workout_profile(
    state: State<'_, AppState>,
    workout_session_id: String,
) -> R<Option<PlayerWorkoutProfile>> {
    let player = state.player.lock().await;
    Ok(player.as_ref().and_then(|handle| {
        profile_for_session(
            &handle.state_rx.borrow().workout_session_id,
            &workout_session_id,
            &handle.workout,
            handle.ftp_w,
        )
    }))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tp_core::model::{ExecutableWorkout, PowerTarget, Segment};
    use tp_core::workout_definition::WorkoutDefinition;

    use super::profile_for_session;
    use crate::database::workout_definitions::{self, ProviderWorkoutDefinition};

    #[test]
    fn player_profile_stays_on_loaded_workout_after_provider_update() {
        let conn = crate::database::open(Path::new(":memory:")).unwrap();
        let definition_id = "provider-workout";
        let old = WorkoutDefinition::from_executable(ExecutableWorkout {
            name: "Original".into(),
            description: String::new(),
            segments: vec![Segment::Steady {
                duration_s: 60,
                power: PowerTarget::Watts(100),
                cadence_rpm: None,
            }],
            text_events: vec![],
        })
        .unwrap();
        let updated = WorkoutDefinition::from_executable(ExecutableWorkout {
            name: "Updated".into(),
            description: String::new(),
            segments: vec![Segment::Steady {
                duration_s: 120,
                power: PowerTarget::Watts(200),
                cadence_rpm: None,
            }],
            text_events: vec![],
        })
        .unwrap();
        let old_json = old.to_json_pretty().unwrap();
        workout_definitions::insert_provider_copy(
            &conn,
            &ProviderWorkoutDefinition {
                id: definition_id,
                tpw_json: &old_json,
                origin: "intervals_icu",
                origin_id: Some(42),
                origin_ref: "event-42",
                synced_at_unix_ms: 1,
            },
        )
        .unwrap();

        // Loading the ride captures the compiled workout, while the stable
        // library ID may later point to a newer provider definition.
        let loaded_row = workout_definitions::get(&conn, definition_id)
            .unwrap()
            .unwrap();
        let captured = WorkoutDefinition::from_json(&loaded_row.tpw_json)
            .unwrap()
            .compile()
            .unwrap();
        let updated_json = updated.to_json_pretty().unwrap();
        assert!(workout_definitions::update_provider_copy(
            &conn,
            &ProviderWorkoutDefinition {
                id: definition_id,
                tpw_json: &updated_json,
                origin: "intervals_icu",
                origin_id: Some(42),
                origin_ref: "event-42",
                synced_at_unix_ms: 2,
            }
        )
        .unwrap());
        let current_row = workout_definitions::get(&conn, definition_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            WorkoutDefinition::from_json(&current_row.tpw_json)
                .unwrap()
                .compile()
                .unwrap()
                .duration_s(),
            120
        );

        let profile = profile_for_session("session-1", "session-1", &captured, 250).unwrap();
        assert_eq!(profile.graph, vec![(0, 40.0), (60, 40.0)]);
        assert_eq!(profile.segments[0].duration_s, 60);
        assert_eq!(profile.ftp_w, 250);
        assert!(profile_for_session("session-1", "session-2", &captured, 250).is_none());
    }
}
