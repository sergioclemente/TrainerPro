//! TrainerPro backend: the Tauri host, application orchestration, and I/O
//! adapters. Reusable domain and device behavior lives in the workspace crates.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_error;
mod app_state;
mod commands;
mod database;
mod device_hub;
mod device_owner;
mod heart_rate_monitor;
mod player_runtime;
mod trainer;
mod whatsonzwift_source;
mod workout_planner_source;
mod workout_source_cache;
mod workout_sources;

use tauri::Manager;

use crate::app_state::AppState;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tp_ble=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let conn = database::open(&data_dir.join("trainerpro.sqlite3"))?;
            let hub = device_hub::DeviceHub::default();
            hub.start_event_forwarders(app.handle().clone());
            app.manage(AppState {
                db: std::sync::Mutex::new(conn),
                data_dir,
                hub,
                player: tokio::sync::Mutex::new(None),
                planner_cache: std::sync::Mutex::new(Vec::new()),
                planner_previews: std::sync::Mutex::new(std::collections::HashMap::new()),
                woz_collections: std::sync::Mutex::new(None),
                woz_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            });
            device_hub::spawn_startup_reconnect(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::workout::import_workout,
            commands::workout::create_workout,
            commands::workout::list_workouts,
            commands::workout::delete_workout,
            commands::workout::get_workout_detail,
            commands::device::start_scan,
            commands::device::connect_device,
            commands::device::disconnect_device,
            commands::device::forget_device,
            commands::device::get_device_state,
            commands::player::load_workout,
            commands::player::start_ride,
            commands::player::pause_ride,
            commands::player::resume_ride,
            commands::player::skip_segment,
            commands::player::set_intensity,
            commands::player::set_erg,
            commands::player::end_ride,
            commands::player::clear_ride,
            commands::player::get_player_state,
            commands::ride_history::list_rides,
            commands::ride_history::delete_ride,
            commands::ride_history::save_fit_as,
            commands::ride_history::reveal_fit,
            commands::ride_history::open_garmin_import,
            commands::settings::get_settings,
            commands::settings::update_settings,
            workout_planner_source::source_test,
            workout_planner_source::planner_cached,
            workout_planner_source::planner_list,
            workout_planner_source::planner_ride,
            workout_planner_source::planner_open_editor,
            workout_planner_source::planner_preview,
            whatsonzwift_source::woz_collections,
            whatsonzwift_source::woz_workouts,
            whatsonzwift_source::woz_ride,
            whatsonzwift_source::woz_open_page,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TrainerPro");
}
