//! Player IPC commands and the shared load-into-player flow.

use tauri::{AppHandle, State};

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::commands::workout::load_workout_definition;
use crate::database::scheduled_workouts as scheduled_db;
use crate::player_runtime::{self as runtime, ActivitySummary, PlayerCommand, PlayerState};

type R<T> = Result<T, AppError>;

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
