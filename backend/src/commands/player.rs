//! Player IPC commands and the shared load-into-player flow.

use tauri::{AppHandle, State};

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::player_runtime::{self as runtime, Cmd, PlayerState, RideSummary};
use crate::commands::workout::load_workout_model;

type R<T> = Result<T, AppError>;

#[tauri::command]
pub async fn load_workout(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> R<PlayerState> {
    do_load_workout(app, &state, &id).await
}

/// Shared load-into-player flow (also used by planner_ride).
pub async fn do_load_workout(
    app: AppHandle,
    state: &State<'_, AppState>,
    id: &str,
) -> R<PlayerState> {
    let workout = load_workout_model(state, id)?;
    let mut player = state.player.lock().await;
    if let Some(h) = player.as_ref() {
        let phase = h.state_rx.borrow().phase.clone();
        if phase == "ready" || phase == "finished" {
            // Never-started (or already-finalized) ride: nothing worth
            // keeping — drop it and load the new workout. Dropping the
            // handle closes the cmd channel and the runtime task exits.
            *player = None;
        } else {
            return Err(AppError::new(
                "ride_active",
                "a ride is in progress — end it first",
            ));
        }
    }
    let handle = runtime::spawn(app, id.to_string(), workout).await?;
    let ps = handle.state_rx.borrow().clone();
    *player = Some(handle);
    Ok(ps)
}

async fn send_cmd(state: &State<'_, AppState>, cmd: Cmd) -> R<()> {
    let player = state.player.lock().await;
    let handle = player.as_ref().ok_or_else(|| AppError::new("no_ride", "no ride loaded"))?;
    handle
        .cmd_tx
        .send(cmd)
        .await
        .map_err(|_| AppError::new("no_ride", "player is gone"))
}

#[tauri::command]
pub async fn start_ride(state: State<'_, AppState>) -> R<()> {
    send_cmd(&state, Cmd::Start).await
}
#[tauri::command]
pub async fn pause_ride(state: State<'_, AppState>) -> R<()> {
    send_cmd(&state, Cmd::Pause).await
}
#[tauri::command]
pub async fn resume_ride(state: State<'_, AppState>) -> R<()> {
    send_cmd(&state, Cmd::Resume).await
}
#[tauri::command]
pub async fn skip_segment(state: State<'_, AppState>) -> R<()> {
    send_cmd(&state, Cmd::Skip).await
}
#[tauri::command]
pub async fn set_intensity(state: State<'_, AppState>, pct: f64) -> R<()> {
    send_cmd(&state, Cmd::SetIntensity(pct)).await
}

#[tauri::command]
pub async fn set_erg(state: State<'_, AppState>, enabled: bool) -> R<()> {
    send_cmd(&state, Cmd::SetErg(enabled)).await
}

#[tauri::command]
pub async fn end_ride(state: State<'_, AppState>) -> R<RideSummary> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_cmd(&state, Cmd::End(tx)).await?;
    let summary = rx
        .await
        .map_err(|_| AppError::new("no_ride", "player exited before summary"))??;
    *state.player.lock().await = None;
    Ok(summary)
}

/// Drop the player slot after a naturally-completed ride (runtime already
/// finalized and emitted `ride_finished`).
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
