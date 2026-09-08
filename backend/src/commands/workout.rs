//! Workout IPC commands and shared workout presentation/import helpers.
//! `ipc.ts` on the frontend mirrors these.

use std::path::PathBuf;

use serde::Serialize;
use sha2::Digest;
use tauri::State;
use tp_core::model::{PowerTarget, Segment, SourceFormat, Workout};
use tp_core::parse::{parse_ergmrc, parse_zwo, Parsed};

use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};

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
