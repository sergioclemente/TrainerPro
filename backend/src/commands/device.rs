//! Device IPC commands.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tp_ble::Role;
#[cfg(not(feature = "simulator"))]
use tp_ble::{SimHrm, SimTrainer};

use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};

type R<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize)]
pub struct DeviceSlot {
    pub role: Role,
    pub saved_name: Option<String>,
    pub saved_platform_id: Option<String>,
    pub connected: bool,
}

fn role_from(s: &str) -> R<Role> {
    match s {
        "trainer" => Ok(Role::Trainer),
        "hrm" => Ok(Role::Hrm),
        _ => Err(AppError::new("bad_role", format!("unknown role {s}"))),
    }
}

#[tauri::command]
pub async fn start_scan(app: AppHandle, state: State<'_, AppState>) -> R<()> {
    let app2 = app.clone();
    tokio::spawn(async move {
        let state = app2.state::<AppState>();
        if let Err(e) = state.hub.scan(app2.clone()).await {
            let _ = app2.emit("toast", serde_json::json!({
                "level": "error", "message": format!("Scan failed: {e}"),
            }));
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
        conn.query_row(
            "SELECT platform_id FROM devices WHERE role = ?1",
            [&role],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .is_some_and(|saved| saved == platform_id)
    };
    if already_saved {
        state.hub.maintain(role_e, &platform_id).await?;
        return Ok(());
    }
    let dev_name = state.hub.connect(role_e, &platform_id).await?;
    let conn = state.db.lock().unwrap();
    conn.execute(
        "INSERT INTO devices(role, platform_id, name, last_connected_at)
         VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(role) DO UPDATE SET platform_id = excluded.platform_id,
           name = excluded.name, last_connected_at = excluded.last_connected_at",
        rusqlite::params![role, platform_id, name.unwrap_or(dev_name), now_unix_ms() as i64],
    )?;
    Ok(())
}

#[tauri::command]
pub async fn disconnect_device(state: State<'_, AppState>, role: String) -> R<()> {
    state.hub.disconnect(role_from(&role)?).await?;
    Ok(())
}

#[tauri::command]
pub async fn forget_device(state: State<'_, AppState>, role: String) -> R<()> {
    state.hub.disconnect(role_from(&role)?).await?;
    let conn = state.db.lock().unwrap();
    conn.execute("DELETE FROM devices WHERE role = ?1", [&role])?;
    Ok(())
}

#[tauri::command]
pub async fn get_device_state(state: State<'_, AppState>) -> R<Vec<DeviceSlot>> {
    let saved: Vec<(String, String, String)> = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare("SELECT role, platform_id, name FROM devices")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    #[cfg(not(feature = "simulator"))]
    let saved: Vec<_> = saved
        .into_iter()
        .filter(|(_, platform_id, _)| platform_id != SimTrainer::ID && platform_id != SimHrm::ID)
        .collect();
    let find = |role: &str| saved.iter().find(|(r, _, _)| r == role);
    let trainer_connected = state.hub.trainer_connected();
    let hrm_connected = state.hub.heart_rate_monitor_connected();
    Ok(vec![
        DeviceSlot {
            role: Role::Trainer,
            saved_name: find("trainer").map(|(_, _, n)| n.clone()),
            saved_platform_id: find("trainer").map(|(_, p, _)| p.clone()),
            connected: trainer_connected,
        },
        DeviceSlot {
            role: Role::Hrm,
            saved_name: find("hrm").map(|(_, _, n)| n.clone()),
            saved_platform_id: find("hrm").map(|(_, p, _)| p.clone()),
            connected: hrm_connected,
        },
    ])
}
