//! IPC command surface. SPEC.md §9 — names and shapes match the spec table;
//! `ipc.ts` on the frontend mirrors these.

use std::path::PathBuf;

use serde::Serialize;
use sha2::Digest;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use tp_ble::Role;
use tp_core::model::{PowerTarget, Segment, SourceFormat, Workout};
use tp_core::parse::{parse_ergmrc, parse_zwo, Parsed};

use crate::err::AppError;
use crate::runtime::{self, Cmd, PlayerState, RideSummary};
use crate::state::{now_unix_ms, AppState, Settings};

type R<T> = Result<T, AppError>;

// ---------------------------------------------------------------------------
// Workouts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct WorkoutSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_format: String,
    pub duration_s: u32,
    pub est_if: f64,
    pub est_tss: f64,
    pub graph: Vec<(u32, f64)>,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub summary: WorkoutSummary,
    pub warnings: Vec<String>,
    pub already_existed: bool,
}

fn fmt_str(f: SourceFormat) -> &'static str {
    match f {
        SourceFormat::Zwo => "zwo",
        SourceFormat::Erg => "erg",
        SourceFormat::Mrc => "mrc",
    }
}

/// Graph polyline for thumbnails/player: (t_s, %FTP) breakpoints. SPEC §3.3.
pub fn graph_points(w: &Workout, ftp: u16) -> Vec<(u32, f64)> {
    let pct = |p: &PowerTarget| match p {
        PowerTarget::PercentFtp(f) => f * 100.0,
        PowerTarget::Watts(watts) => f64::from(*watts) / f64::from(ftp.max(1)) * 100.0,
    };
    let mut out = Vec::with_capacity(w.segments.len() * 2);
    let mut t = 0u32;
    for seg in &w.segments {
        match seg {
            Segment::Steady { duration_s, power, .. } => {
                out.push((t, pct(power)));
                out.push((t + duration_s, pct(power)));
            }
            Segment::Ramp { duration_s, start, end, .. } => {
                out.push((t, pct(start)));
                out.push((t + duration_s, pct(end)));
            }
            Segment::FreeRide { duration_s } => {
                out.push((t, 0.0));
                out.push((t + duration_s, 0.0));
            }
        }
        t += seg.duration_s();
    }
    out
}

fn parse_by_ext(ext: &str, content: &str) -> R<Parsed> {
    match ext {
        "zwo" => Ok(parse_zwo(content)?),
        "erg" | "mrc" => Ok(parse_ergmrc(content, Some(ext))?),
        other => Err(AppError::new(
            "parse_failed",
            format!("unsupported file type .{other} (expected .zwo, .erg, .mrc)"),
        )),
    }
}

#[tauri::command]
pub async fn import_workout(state: State<'_, AppState>, path: String) -> R<ImportResult> {
    import_from_path(&state, &PathBuf::from(path))
}

/// Shared import pipeline: used by the file-import command and the
/// WorkoutPlanner ride path (spec-workoutplanner.md §B2).
pub fn import_from_path(state: &State<'_, AppState>, src: &std::path::Path) -> R<ImportResult> {
    let content = std::fs::read_to_string(src)?;
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    import_content(state, &content, &ext, Vec::new())
}

/// The import pipeline proper, over content already in memory. Authored
/// workouts (the builder) join here rather than round-tripping through a temp
/// file, so there is still exactly one path into the library: hash → dedup →
/// parse → store file → row.
pub fn import_content(
    state: &State<'_, AppState>,
    content: &str,
    ext: &str,
    extra_warnings: Vec<String>,
) -> R<ImportResult> {
    let sha = format!("{:x}", sha2::Sha256::digest(content.as_bytes()));
    let ftp = state.settings().profile.ftp;

    // Duplicate: same bytes → return existing entry. SPEC §3.3.
    let existing: Option<String> = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT id FROM workouts WHERE sha256 = ?1", [&sha], |r| r.get(0))
            .ok()
    };
    if let Some(id) = existing {
        let summary = get_workout_summary(state, &id)?;
        return Ok(ImportResult { summary, warnings: vec![], already_existed: true });
    }

    let parsed = parse_by_ext(ext, content)?;
    let w = &parsed.workout;
    let (est_if, est_tss) = tp_core::metrics::estimate_if_tss(w, ftp);
    let id = uuid::Uuid::new_v4().to_string();

    std::fs::create_dir_all(state.workouts_dir())?;
    let dest = state.workouts_dir().join(format!("{id}.{ext}"));
    std::fs::write(&dest, content)?;

    let graph = graph_points(w, ftp);
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO workouts (id, name, description, source_format, file_path, sha256,
               duration_s, est_if, est_tss, graph_json, imported_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![
                id,
                w.name,
                w.description,
                fmt_str(w.source_format),
                dest.to_string_lossy(),
                sha,
                w.duration_s(),
                est_if,
                est_tss,
                serde_json::to_string(&graph).unwrap(),
                now_unix_ms() as i64,
            ],
        )?;
    }
    Ok(ImportResult {
        summary: WorkoutSummary {
            id,
            name: w.name.clone(),
            description: w.description.clone(),
            source_format: fmt_str(w.source_format).into(),
            duration_s: w.duration_s(),
            est_if,
            est_tss,
            graph,
            origin: None,
        },
        warnings: extra_warnings
            .into_iter()
            .chain(parsed.warnings.iter().map(|w| w.message.clone()))
            .collect(),
        already_existed: false,
    })
}

/// Save a workout authored in the builder. Emits ZWO and hands it to the
/// ordinary import pipeline, so an authored workout is indistinguishable from
/// an imported one everywhere downstream.
#[tauri::command]
pub async fn create_workout(
    state: State<'_, AppState>,
    draft: tp_core::build::WorkoutDraft,
) -> R<ImportResult> {
    let emitted = tp_core::build::to_zwo(&draft)
        .map_err(|e| AppError::new("invalid_workout", e.message))?;

    // ZWO can only express a repeat as IntervalsT (one work + one recovery).
    // Anything else was written out lap by lap: it rides identically, but the
    // file no longer records that it was a repeat. Say so rather than hide it.
    let mut warnings = Vec::new();
    if emitted.expanded_repeats > 0 {
        let n = emitted.expanded_repeats;
        warnings.push(format!(
            "{n} repeat{} could not be stored as a repeat in ZWO and {} written out lap by lap. \
             The workout rides exactly the same.",
            if n == 1 { "" } else { "s" },
            if n == 1 { "was" } else { "were" },
        ));
    }
    import_content(&state, &emitted.xml, "zwo", warnings)
}

fn get_workout_summary(state: &State<'_, AppState>, id: &str) -> R<WorkoutSummary> {
    let conn = state.db.lock().unwrap();
    conn.query_row(
        "SELECT id, name, description, source_format, duration_s, est_if, est_tss, graph_json, origin
         FROM workouts WHERE id = ?1",
        [id],
        |r| {
            Ok(WorkoutSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                source_format: r.get(3)?,
                duration_s: r.get(4)?,
                est_if: r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                est_tss: r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                graph: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
                origin: r.get(8)?,
            })
        },
    )
    .map_err(|_| AppError::new("not_found", format!("workout {id} not found")))
}

#[tauri::command]
pub async fn list_workouts(state: State<'_, AppState>) -> R<Vec<WorkoutSummary>> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, name, description, source_format, duration_s, est_if, est_tss, graph_json, origin
         FROM workouts ORDER BY imported_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(WorkoutSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                source_format: r.get(3)?,
                duration_s: r.get(4)?,
                est_if: r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                est_tss: r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                graph: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
                origin: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Removes the workout from the library: drops the SQLite row and deletes
/// the app's *copy* under `<appdata>/workouts/`. The file the user imported
/// from is never touched (import copies it in).
#[tauri::command]
pub async fn delete_workout(state: State<'_, AppState>, id: String) -> R<()> {
    let file: Option<String> = {
        let conn = state.db.lock().unwrap();
        let f = conn
            .query_row("SELECT file_path FROM workouts WHERE id = ?1", [&id], |r| r.get(0))
            .ok();
        conn.execute("DELETE FROM workouts WHERE id = ?1", [&id])?;
        f
    };
    if let Some(f) = file {
        let _ = std::fs::remove_file(f);
    }
    Ok(())
}

/// Structured segment description for the workout detail view. Percentages
/// are of FTP (absolute-watt targets converted at the caller's FTP).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct SegmentRow {
    pub kind: String, // "steady" | "ramp" | "freeride"
    /// Derived step title: Warm-up / Cool-down / Ramp / Steady / Interval /
    /// Recovery / Free ride.
    pub label: String,
    /// Workout text event(s) starting inside this segment (the planner's
    /// per-step comments travel through ZWO as textevents).
    pub note: Option<String>,
    pub duration_s: u32,
    pub start_pct: f64,
    pub end_pct: f64,
    pub cadence_rpm: Option<u16>,
}

pub fn segment_rows(w: &Workout, ftp: u16) -> Vec<SegmentRow> {
    let pct = |p: &PowerTarget| match p {
        PowerTarget::PercentFtp(f) => f * 100.0,
        PowerTarget::Watts(watts) => f64::from(*watts) / f64::from(ftp.max(1)) * 100.0,
    };
    let n = w.segments.len();
    let mut start_t = 0u32;
    let mut rows: Vec<SegmentRow> = Vec::with_capacity(n);
    for (i, s) in w.segments.iter().enumerate() {
        let dur = s.duration_s();
        let end_t = start_t + dur;
        let note = {
            let texts: Vec<&str> = w
                .text_events
                .iter()
                .filter(|t| t.offset_s >= start_t && t.offset_s < end_t)
                .map(|t| t.message.as_str())
                .collect();
            if texts.is_empty() { None } else { Some(texts.join(" · ")) }
        };
        let row = match s {
            Segment::Steady { duration_s, power, cadence_rpm } => {
                let p = pct(power);
                // Recovery = easy spinning right after harder work.
                let label = if p < 62.0
                    && rows
                        .last()
                        .map(|prev: &SegmentRow| prev.end_pct > p + 12.0)
                        .unwrap_or(false)
                {
                    "Recovery"
                } else if p >= 82.0 {
                    "Interval"
                } else {
                    "Steady"
                };
                SegmentRow {
                    kind: "steady".into(),
                    label: label.into(),
                    note,
                    duration_s: *duration_s,
                    start_pct: p,
                    end_pct: p,
                    cadence_rpm: *cadence_rpm,
                }
            }
            Segment::Ramp { duration_s, start, end, cadence_rpm } => {
                let (a, b) = (pct(start), pct(end));
                let label = if i == 0 && b > a {
                    "Warm-up"
                } else if i + 1 == n && b < a {
                    "Cool-down"
                } else {
                    "Ramp"
                };
                SegmentRow {
                    kind: "ramp".into(),
                    label: label.into(),
                    note,
                    duration_s: *duration_s,
                    start_pct: a,
                    end_pct: b,
                    cadence_rpm: *cadence_rpm,
                }
            }
            Segment::FreeRide { duration_s } => SegmentRow {
                kind: "freeride".into(),
                label: "Free ride".into(),
                note,
                duration_s: *duration_s,
                start_pct: 0.0,
                end_pct: 0.0,
                cadence_rpm: None,
            },
        };
        rows.push(row);
        start_t = end_t;
    }
    rows
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkoutDetail {
    pub summary: WorkoutSummary,
    pub segments: Vec<SegmentRow>,
}

#[tauri::command]
pub async fn get_workout_detail(state: State<'_, AppState>, id: String) -> R<WorkoutDetail> {
    let summary = get_workout_summary(&state, &id)?;
    let workout = load_workout_model(&state, &id)?;
    let ftp = state.settings().profile.ftp;
    Ok(WorkoutDetail { summary, segments: segment_rows(&workout, ftp) })
}

/// Load the full workout model (re-parsed from the stored file — files are
/// truth, SPEC §8).
pub fn load_workout_model(state: &State<'_, AppState>, id: &str) -> R<Workout> {
    let path: String = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT file_path FROM workouts WHERE id = ?1", [id], |r| r.get(0))
            .map_err(|_| AppError::new("not_found", format!("workout {id} not found")))?
    };
    let path = PathBuf::from(path);
    let content = std::fs::read_to_string(&path)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    Ok(parse_by_ext(&ext, &content)?.workout)
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

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
pub async fn start_scan(app: AppHandle, state: State<'_, AppState>, role: String) -> R<()> {
    let role = role_from(&role)?;
    let app2 = app.clone();
    tokio::spawn(async move {
        let state = app2.state::<AppState>();
        if let Err(e) = state.hub.scan(app2.clone(), role).await {
            let _ = app2.emit("toast", serde_json::json!({
                "level": "error", "message": format!("Scan failed: {e}"),
            }));
        }
        let _ = app2.emit("scan_done", serde_json::json!({ "role": role }));
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

// ---------------------------------------------------------------------------
// Player
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RideRow {
    pub id: String,
    pub workout_name: String,
    pub started_at: i64,
    pub timer_s: u32,
    pub avg_power: Option<u16>,
    pub np: Option<u16>,
    pub tss: Option<f64>,
    pub avg_hr: Option<u16>,
    pub completed_pct: f64,
    pub fit_path: String,
}

#[tauri::command]
pub async fn list_rides(state: State<'_, AppState>) -> R<Vec<RideRow>> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, workout_name, started_at, timer_s, avg_power, np, tss, avg_hr,
                completed_pct, fit_path
         FROM rides ORDER BY started_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RideRow {
                id: r.get(0)?,
                workout_name: r.get(1)?,
                started_at: r.get(2)?,
                timer_s: r.get(3)?,
                avg_power: r.get(4)?,
                np: r.get(5)?,
                tss: r.get(6)?,
                avg_hr: r.get(7)?,
                completed_pct: r.get(8)?,
                fit_path: r.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Deletes a ride from history: the SQLite row plus the app's own .fit and
/// journal files. Any copy you exported elsewhere (export folder, Save
/// As…) is left alone.
#[tauri::command]
pub async fn delete_ride(state: State<'_, AppState>, id: String) -> R<()> {
    let paths: Option<(String, String)> = {
        let conn = state.db.lock().unwrap();
        let p = conn
            .query_row(
                "SELECT fit_path, journal_path FROM rides WHERE id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        conn.execute("DELETE FROM rides WHERE id = ?1", [&id])?;
        p
    };
    if let Some((fit, journal)) = paths {
        let _ = std::fs::remove_file(fit);
        let _ = std::fs::remove_file(journal);
    }
    Ok(())
}

#[tauri::command]
pub async fn save_fit_as(state: State<'_, AppState>, id: String, dest_path: String) -> R<()> {
    let fit: String = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT fit_path FROM rides WHERE id = ?1", [&id], |r| r.get(0))
            .map_err(|_| AppError::new("not_found", "ride not found"))?
    };
    std::fs::copy(fit, dest_path)?;
    Ok(())
}

#[tauri::command]
pub async fn reveal_fit(app: AppHandle, state: State<'_, AppState>, id: String) -> R<()> {
    let fit: String = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT fit_path FROM rides WHERE id = ?1", [&id], |r| r.get(0))
            .map_err(|_| AppError::new("not_found", "ride not found"))?
    };
    app.opener()
        .reveal_item_in_dir(&fit)
        .map_err(|e| AppError::new("io", e.to_string()))
}

#[tauri::command]
pub async fn open_garmin_import(app: AppHandle) -> R<()> {
    app.opener()
        .open_url("https://connect.garmin.com/modern/import-data", None::<String>)
        .map_err(|e| AppError::new("io", e.to_string()))
}

// ---------------------------------------------------------------------------
// Profile / settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> R<Settings> {
    Ok(state.settings())
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, settings: Settings) -> R<Settings> {
    state.save_settings(&settings)?;
    Ok(state.settings())
}
