//! TrainerPro Tauri shell. SPEC.md §1 (layout note: tp-app lives at the
//! workspace root because the Tauri CLI expects tauri.conf.json next to the
//! app crate; tp-core/tp-ble stay under crates/).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cache;
mod cmd;
mod db;
mod device;
mod err;
mod heart_rate_monitor;
mod hub;
mod planner;
mod runtime;
mod sources;
mod woz;
mod state;
mod trainer;

use tauri::Manager;

use crate::state::AppState;

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
            let conn = db::open(&data_dir.join("trainerpro.sqlite3"))?;
            let hub = hub::DeviceHub::default();
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
            hub::spawn_startup_reconnect(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cmd::import_workout,
            cmd::create_workout,
            cmd::list_workouts,
            cmd::delete_workout,
            cmd::get_workout_detail,
            cmd::start_scan,
            cmd::connect_device,
            cmd::disconnect_device,
            cmd::forget_device,
            cmd::get_device_state,
            cmd::load_workout,
            cmd::start_ride,
            cmd::pause_ride,
            cmd::resume_ride,
            cmd::skip_segment,
            cmd::set_intensity,
            cmd::set_erg,
            cmd::end_ride,
            cmd::clear_ride,
            cmd::get_player_state,
            cmd::list_rides,
            cmd::delete_ride,
            cmd::save_fit_as,
            cmd::reveal_fit,
            cmd::open_garmin_import,
            cmd::get_settings,
            cmd::update_settings,
            planner::source_test,
            planner::planner_cached,
            planner::planner_list,
            planner::planner_ride,
            planner::planner_open_editor,
            planner::planner_preview,
            woz::woz_collections,
            woz::woz_workouts,
            woz::woz_ride,
            woz::woz_open_page,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TrainerPro");
}
