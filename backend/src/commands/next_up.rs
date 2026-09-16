//! Next Up IPC entry point.

use tauri::State;

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::next_up::{self, NextUpItem};

#[tauri::command]
pub async fn list_next_up(state: State<'_, AppState>) -> Result<Vec<NextUpItem>, AppError> {
    next_up::list(&state)
}
