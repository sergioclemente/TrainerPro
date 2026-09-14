//! Workout IPC commands and shared workout presentation/import helpers.
//! `ipc.ts` on the frontend mirrors these.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;
use tp_core::model::{ExecutableWorkout, PowerTarget, Segment};
use tp_core::parse::{parse_ergmrc, parse_zwo, Parsed};
use tp_core::workout_definition::WorkoutDefinition;

use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};
use crate::database::workout_definitions as definition_db;

type R<T> = Result<T, AppError>;

// ---------------------------------------------------------------------------
// Workouts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct WorkoutSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub training_focus: Option<String>,
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

/// Graph polyline for thumbnails/player: (t_s, %FTP) breakpoints. SPEC §3.3.
pub fn graph_points(w: &ExecutableWorkout, ftp: u16) -> Vec<(u32, f64)> {
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

/// Read a user-selected boundary file and hand its contents to normalization.
pub fn import_from_path(state: &State<'_, AppState>, src: &std::path::Path) -> R<ImportResult> {
    let content = std::fs::read_to_string(src)?;
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    import_content(state, &content, &ext, Vec::new())
}

/// Parse a boundary format and normalize it into the canonical TPW store.
pub fn import_content(
    state: &State<'_, AppState>,
    content: &str,
    ext: &str,
    extra_warnings: Vec<String>,
) -> R<ImportResult> {
    let parsed = parse_by_ext(ext, content)?;
    let definition = WorkoutDefinition::from_executable(parsed.workout)?;
    let warnings = extra_warnings
        .into_iter()
        .chain(parsed.warnings.into_iter().map(|warning| warning.message))
        .collect();
    store_definition(state, &definition, warnings)
}

/// Validate and persist one TPW definition. This is the shared entry point for
/// local authoring, boundary imports, and connected workout sources.
pub fn store_definition(
    state: &State<'_, AppState>,
    definition: &WorkoutDefinition,
    warnings: Vec<String>,
) -> R<ImportResult> {
    let tpw_json = definition.to_json_pretty()?;
    let existing = {
        let conn = state.db.lock().unwrap();
        definition_db::find_id_by_tpw_json(&conn, &tpw_json)?
    };
    if let Some(id) = existing {
        return Ok(ImportResult {
            summary: get_workout_summary(state, &id)?,
            warnings,
            already_existed: true,
        });
    }

    let id = uuid::Uuid::new_v4().to_string();
    {
        let conn = state.db.lock().unwrap();
        definition_db::insert(
            &conn,
            &definition_db::NewWorkoutDefinition {
                id: &id,
                tpw_json: &tpw_json,
                created_at_ms: now_unix_ms() as i64,
            },
        )?;
    }

    Ok(ImportResult {
        summary: summary_from_definition(id, None, definition, state.settings().profile.ftp)?,
        warnings,
        already_existed: false,
    })
}

/// Save a locally authored workout directly as TPW.
#[tauri::command]
pub async fn create_workout(
    state: State<'_, AppState>,
    draft: tp_core::build::WorkoutDraft,
) -> R<ImportResult> {
    let definition = tp_core::build::to_workout_definition(&draft)
        .map_err(|e| AppError::new("invalid_workout", e.message))?;
    store_definition(&state, &definition, Vec::new())
}

fn get_workout_summary(state: &State<'_, AppState>, id: &str) -> R<WorkoutSummary> {
    let row = {
        let conn = state.db.lock().unwrap();
        definition_db::get(&conn, id)?
            .ok_or_else(|| AppError::new("not_found", format!("workout {id} not found")))?
    };
    summary_from_row(row, state.settings().profile.ftp)
}

fn summary_from_row(
    row: definition_db::WorkoutDefinitionRow,
    ftp: u16,
) -> R<WorkoutSummary> {
    let definition = WorkoutDefinition::from_json(&row.tpw_json)?;
    summary_from_definition(row.id, row.origin, &definition, ftp)
}

fn summary_from_definition(
    id: String,
    origin: Option<String>,
    definition: &WorkoutDefinition,
    ftp: u16,
) -> R<WorkoutSummary> {
    let executable = definition.compile()?;
    let (est_if, est_tss) = tp_core::metrics::estimate_if_tss(&executable, ftp);
    Ok(WorkoutSummary {
        id,
        name: definition.title.clone(),
        description: definition.description.clone(),
        training_focus: definition.training_focus.clone(),
        duration_s: executable.duration_s(),
        est_if,
        est_tss,
        graph: graph_points(&executable, ftp),
        origin,
    })
}

#[tauri::command]
pub async fn list_workouts(state: State<'_, AppState>) -> R<Vec<WorkoutSummary>> {
    let rows = {
        let conn = state.db.lock().unwrap();
        definition_db::list(&conn)?
    };
    let ftp = state.settings().profile.ftp;
    rows.into_iter()
        .map(|row| summary_from_row(row, ftp))
        .collect()
}

/// Removes a TPW definition from TrainerPro. An external source file, if any,
/// is never touched.
#[tauri::command]
pub async fn delete_workout(state: State<'_, AppState>, id: String) -> R<()> {
    let deleted = {
        let conn = state.db.lock().unwrap();
        definition_db::delete(&conn, &id)?
    };
    if !deleted {
        return Err(AppError::new(
            "not_found",
            format!("workout {id} not found"),
        ));
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

pub fn segment_rows(w: &ExecutableWorkout, ftp: u16) -> Vec<SegmentRow> {
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

/// Load and compile the canonical TPW definition from SQLite.
pub fn load_workout_model(state: &State<'_, AppState>, id: &str) -> R<ExecutableWorkout> {
    let tpw_json = {
        let conn = state.db.lock().unwrap();
        definition_db::get(&conn, id)?
            .map(|row| row.tpw_json)
            .ok_or_else(|| AppError::new("not_found", format!("workout {id} not found")))?
    };
    Ok(WorkoutDefinition::from_json(&tpw_json)?.compile()?)
}
