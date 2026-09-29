//! Device IPC commands.

use crate::controller::ControllerSource;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tp_ble::Role;
#[cfg(not(feature = "simulator"))]
use tp_ble::{SimHrm, SimTrainer};

use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};
use crate::database::devices as device_db;

type R<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize)]
pub struct DeviceSlot {
    pub role: Role,
    pub saved_name: Option<String>,
    pub saved_platform_id: Option<String>,
    pub connected: bool,
    pub controller_source: Option<ControllerSource>,
    pub controller_profile: Option<tp_ble::ControllerProfile>,
    pub error: Option<String>,
}

fn role_from(s: &str) -> R<Role> {
    match s {
        "trainer" => Ok(Role::Trainer),
        "hrm" => Ok(Role::Hrm),
        "controller" => Ok(Role::Controller),
        _ => Err(AppError::new("bad_role", format!("unknown role {s}"))),
    }
}

#[tauri::command]
pub async fn start_scan(app: AppHandle, state: State<'_, AppState>) -> R<()> {
    let app2 = app.clone();
    tokio::spawn(async move {
        let state = app2.state::<AppState>();
        if let Err(e) = state.hub.scan(app2.clone()).await {
            let _ = app2.emit(
                "toast",
                serde_json::json!({
                    "level": "error", "message": format!("Scan failed: {e}"),
                }),
            );
        }
        let _ = app2.emit("scan_done", ());
    });
    let _ = state; // scan runs detached; command returns immediately
    Ok(())
}

#[tauri::command]
pub async fn connect_device(
    state: State<'_, AppState>,
    role: String,
    platform_id: String,
    name: Option<String>,
) -> R<()> {
    let role_e = role_from(&role)?;
    let already_saved = {
        let conn = state.db.lock().unwrap();
        device_db::platform_id_for_role(&conn, &role)?.is_some_and(|saved| saved == platform_id)
    };
    if already_saved {
        state.hub.maintain(role_e, &platform_id).await?;
        if role_e == Role::Controller {
            device_db::save_controller_source(
                &state.db.lock().unwrap(),
                ControllerSource::PairedController,
            )?;
        }
        return Ok(());
    }
    let dev_name = state.hub.connect(role_e, &platform_id).await?;
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction()?;
    if role_e == Role::Controller {
        device_db::save_controller_source(&transaction, ControllerSource::PairedController)?;
    }
    device_db::upsert(
        &transaction,
        &role,
        &platform_id,
        &name.unwrap_or(dev_name),
        now_unix_ms() as i64,
    )?;
    transaction.commit()?;
    Ok(())
}

#[tauri::command]
pub async fn disconnect_device(state: State<'_, AppState>, role: String) -> R<()> {
    if role_from(&role)? == Role::Controller {
        state
            .hub
            .controller()
            .select_source(ControllerSource::Disabled)
            .await?;
        device_db::save_controller_source(&state.db.lock().unwrap(), ControllerSource::Disabled)?;
    } else {
        state.hub.disconnect(role_from(&role)?).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn forget_device(state: State<'_, AppState>, role: String) -> R<()> {
    let role_e = role_from(&role)?;
    let disable_selected_controller = role_e == Role::Controller
        && state.hub.controller().state().controller_source
            == Some(ControllerSource::PairedController);
    if disable_selected_controller {
        state
            .hub
            .controller()
            .select_source(ControllerSource::Disabled)
            .await?;
    } else if role_e != Role::Controller {
        state.hub.disconnect(role_e).await?;
    }
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction()?;
    device_db::delete(&transaction, &role)?;
    if disable_selected_controller {
        device_db::save_controller_source(&transaction, ControllerSource::Disabled)?;
    }
    transaction.commit()?;
    Ok(())
}

#[tauri::command]
pub async fn get_device_state(state: State<'_, AppState>) -> R<Vec<DeviceSlot>> {
    let saved = {
        let conn = state.db.lock().unwrap();
        device_db::list(&conn)?
    };
    #[cfg(not(feature = "simulator"))]
    let saved: Vec<_> = saved
        .into_iter()
        .filter(|device| {
            device.platform_id != SimTrainer::ID
                && device.platform_id != SimHrm::ID
                && device.platform_id != tp_ble::SimController::ID
        })
        .collect();
    let find = |role: &str| saved.iter().find(|device| device.role == role);
    let trainer_connected = state.hub.trainer_connected();
    let hrm_connected = state.hub.heart_rate_monitor_connected();
    let controller = state.hub.controller().state();
    Ok(vec![
        DeviceSlot {
            role: Role::Trainer,
            saved_name: find("trainer").map(|device| device.name.clone()),
            saved_platform_id: find("trainer").map(|device| device.platform_id.clone()),
            connected: trainer_connected,
            controller_source: None,
            controller_profile: None,
            error: None,
        },
        DeviceSlot {
            role: Role::Hrm,
            saved_name: find("hrm").map(|device| device.name.clone()),
            saved_platform_id: find("hrm").map(|device| device.platform_id.clone()),
            connected: hrm_connected,
            controller_source: None,
            controller_profile: None,
            error: None,
        },
        DeviceSlot {
            role: Role::Controller,
            saved_name: find("controller").map(|d| d.name.clone()),
            saved_platform_id: find("controller").map(|d| d.platform_id.clone()),
            connected: controller.is_connected(),
            controller_source: controller.controller_source,
            controller_profile: controller.controller_profile,
            error: controller.error,
        },
    ])
}

#[tauri::command]
pub async fn set_controller_source(state: State<'_, AppState>, source: ControllerSource) -> R<()> {
    state.hub.controller().select_source(source).await?;
    device_db::save_controller_source(&state.db.lock().unwrap(), source)?;
    Ok(())
}
