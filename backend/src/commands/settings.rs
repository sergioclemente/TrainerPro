//! Profile and settings IPC commands.

use tauri::State;

use crate::app_error::AppError;
use crate::app_state::{AppState, Settings};

type R<T> = Result<T, AppError>;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> R<Settings> {
    Ok(state.settings())
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, settings: Settings) -> R<Settings> {
    state.save_settings(&settings)?;
    Ok(state.settings())
}
