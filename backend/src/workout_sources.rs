//! Shared pipeline for workout *sources* (the plugin layer).
//!
//! A source is any provider of ridable workouts: the local library, the
//! WorkoutPlanner server, whatsonzwift.com, or future connectors. Sources
//! normalize into WorkoutSpec, then share provenance and player-loading behavior.

use tauri::{AppHandle, Emitter, State};
use tp_core::workout_spec::WorkoutSpec;

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::commands::{player, workout};
use crate::database::workout_definitions as definition_db;
use crate::player_runtime::PlayerState;

/// ZWO boundary payload → WorkoutSpec → catalog entry → loaded player.
pub async fn ride_from_zwo(
    app: AppHandle,
    state: &State<'_, AppState>,
    zwo: &str,
    origin: &str,
    origin_ref: &str,
    origin_id: Option<i64>,
) -> Result<PlayerState, AppError> {
    let (definition, warnings) = workout::normalize_content(zwo, "zwo")?;
    save_and_load(
        app,
        state,
        &definition,
        warnings,
        origin,
        origin_ref,
        origin_id,
    )
    .await
}

/// Already-normalized WorkoutSpec → catalog entry → loaded player.
pub async fn ride_from_definition(
    app: AppHandle,
    state: &State<'_, AppState>,
    definition: &WorkoutSpec,
    origin: &str,
    origin_ref: &str,
    origin_id: Option<i64>,
) -> Result<PlayerState, AppError> {
    save_and_load(
        app,
        state,
        definition,
        Vec::new(),
        origin,
        origin_ref,
        origin_id,
    )
    .await
}

async fn save_and_load(
    app: AppHandle,
    state: &State<'_, AppState>,
    definition: &WorkoutSpec,
    warnings: Vec<String>,
    origin: &str,
    origin_ref: &str,
    origin_id: Option<i64>,
) -> Result<PlayerState, AppError> {
    // Reject unsupported execution before changing the catalog cache.
    definition.compile()?;
    let json = definition.to_json_pretty()?;
    let id = {
        let mut conn = state.db.lock().unwrap();
        definition_db::save_catalog_entry(
            &mut conn,
            &definition_db::ProviderWorkoutDefinition {
                id: &uuid::Uuid::new_v4().to_string(),
                tpw_json: &json,
                origin,
                origin_id,
                origin_ref,
                synced_at_unix_ms: crate::app_state::now_unix_ms() as i64,
            },
        )?
    };
    for warning in warnings.iter().take(3) {
        let _ = app.emit(
            "toast",
            serde_json::json!({ "level": "warn", "message": warning }),
        );
    }
    player::load_workout_into_player(app, state, &id, None).await
}
