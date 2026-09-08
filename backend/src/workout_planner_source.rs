//! WorkoutPlanner integration client + IPC commands.
//! spec-workoutplanner.md Part B. ZWO is the interchange: this module never
//! parses the planner's DSL — it fetches ZWO from /workout_file and feeds
//! TrainerPro's existing import path.

use serde::Serialize;
use sha2::Digest;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use std::collections::HashMap;

use crate::app_error::AppError;
use crate::player_runtime::PlayerState;
use crate::app_state::{AppState, PlannerSettings, SourceConfig};

const TIMEOUT_S: u64 = 30;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct PlannerWorkout {
    pub wid: i64,
    pub title: String,
    pub duration_s: u32,
    pub tss: Option<f64>,
    pub tags: String,
    /// creation_ts from the server (ISO-ish string), when provided.
    pub created: Option<String>,
    /// Kept only for constructing the web-editor URL; never parsed here.
    pub dsl: String,
}

fn client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_S))
        .build()
        .map_err(|e| AppError::new("planner_unreachable", e.to_string()))
}

fn base_url(cfg: &PlannerSettings) -> String {
    cfg.url.trim_end_matches('/').to_string()
}

fn apply_auth(req: reqwest::RequestBuilder, cfg: &PlannerSettings) -> reqwest::RequestBuilder {
    if cfg.user.is_empty() {
        req
    } else {
        req.basic_auth(&cfg.user, Some(&cfg.pass))
    }
}

async fn get_with_retry(
    cfg: &PlannerSettings,
    path_and_query: &str,
) -> Result<reqwest::Response, AppError> {
    let url = format!("{}{}", base_url(cfg), path_and_query);
    let client = client()?;
    let mut last_err = None;
    // One retry: Fly machines auto-stop; first hit may be a cold start.
    for _ in 0..2 {
        match apply_auth(client.get(&url), cfg).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.as_u16() == 401 {
                    return Err(AppError::new(
                        "planner_auth",
                        "WorkoutPlanner rejected the credentials (check Settings)",
                    ));
                }
                if status.as_u16() == 404 || status.as_u16() == 422 {
                    return Err(AppError::new(
                        "planner_bad_workout",
                        format!("planner returned {status}"),
                    ));
                }
                if !status.is_success() {
                    return Err(AppError::new(
                        "planner_unreachable",
                        format!("planner returned {status}"),
                    ));
                }
                return Ok(resp);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(AppError::new(
        "planner_unreachable",
        format!(
            "could not reach WorkoutPlanner: {}",
            last_err.map(|e| e.to_string()).unwrap_or_default()
        ),
    ))
}

/// GET /workouts, filtered to Bike (sport_type 1). Tolerant of response
/// shape drift: accepts a bare array or a `{data: [...]}` wrapper (the
/// planner's /compute_workout style), and numeric fields as numbers or
/// strings. On failure the error carries a body snippet for diagnosis.
pub async fn fetch_workouts(cfg: &PlannerSettings) -> Result<Vec<PlannerWorkout>, AppError> {
    let resp = get_with_retry(cfg, "/workouts").await?;
    let body = resp
        .text()
        .await
        .map_err(|e| AppError::new("planner_unreachable", e.to_string()))?;
    parse_workouts_body(&body)
}

fn snippet(body: &str) -> String {
    let s: String = body.chars().take(200).collect();
    if body.len() > 200 { format!("{s}…") } else { s }
}

fn parse_workouts_body(body: &str) -> Result<Vec<PlannerWorkout>, AppError> {
    let bad = |what: &str| {
        AppError::new(
            "planner_bad_response",
            format!("/workouts: {what} — body starts: {}", snippet(body)),
        )
    };
    let value: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|_| bad("response is not JSON"))?;
    let arr = match &value {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::Object(o) => match o.get("data").or_else(|| o.get("workouts")) {
            Some(serde_json::Value::Array(a)) => a.clone(),
            // Some servers double-encode: {"data": "[...]"}.
            Some(serde_json::Value::String(s)) => match serde_json::from_str(s) {
                Ok(serde_json::Value::Array(a)) => a,
                _ => return Err(bad("object without a data/workouts array")),
            },
            _ => return Err(bad("object without a data/workouts array")),
        },
        _ => return Err(bad("expected a JSON array")),
    };

    fn as_i64(v: &serde_json::Value) -> Option<i64> {
        v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }
    fn as_f64(v: &serde_json::Value) -> Option<f64> {
        v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }

    let mut out = Vec::new();
    for w in &arr {
        let Some(wid) = w.get("id").and_then(as_i64) else { continue };
        let sport = w.get("sport_type").and_then(as_i64).unwrap_or(1);
        if sport != 1 {
            continue;
        }
        out.push(PlannerWorkout {
            wid,
            created: w.get("creation_ts").and_then(|v| v.as_str()).map(String::from),
            title: w
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("(untitled)")
                .to_string(),
            duration_s: w.get("duration_sec").and_then(as_i64).unwrap_or(0).max(0) as u32,
            tss: w.get("tss").and_then(as_f64),
            tags: w
                .get("tags")
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .to_string(),
            dsl: w
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        });
    }
    if out.is_empty() && !arr.is_empty() {
        return Err(bad("rows lack usable id/sport_type fields"));
    }
    Ok(out)
}

/// GET /workout_file → ZWO text (spec Part A1).
pub async fn fetch_zwo(cfg: &PlannerSettings, wid: i64, ftp: u16) -> Result<String, AppError> {
    let resp = get_with_retry(cfg, &format!("/workout_file?wid={wid}&format=zwo&ftp={ftp}")).await?;
    resp.text()
        .await
        .map_err(|e| AppError::new("planner_unreachable", e.to_string()))
}

fn cfg_checked(state: &State<'_, AppState>) -> Result<PlannerSettings, AppError> {
    let cfg = state.settings().planner();
    if !cfg.enabled || cfg.url.is_empty() {
        return Err(AppError::new("planner_disabled", "WorkoutPlanner is not configured"));
    }
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// IPC commands
// ---------------------------------------------------------------------------

/// Generic provider connection test (the plugin dispatch point). Uses the
/// *submitted* values, not yet saved, so Test works before Save. `detail` is a
/// human line for the toast. Sources with no test return `not_testable`.
#[derive(Debug, Clone, Serialize)]
pub struct SourceTestResult {
    pub ok: bool,
    pub detail: String,
}

#[tauri::command]
pub async fn source_test(
    id: String,
    values: HashMap<String, String>,
) -> Result<SourceTestResult, AppError> {
    match id.as_str() {
        "planner" => {
            let cfg = PlannerSettings::from_source(&SourceConfig { enabled: true, values });
            let list = fetch_workouts(&cfg).await?;
            Ok(SourceTestResult { ok: true, detail: format!("{} workouts found", list.len()) })
        }
        other => Err(AppError::new(
            "not_testable",
            format!("'{other}' has no connection test"),
        )),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannerListResult {
    pub rows: Vec<PlannerWorkout>,
    pub from_cache: bool,
    pub fetched_at_ms: i64,
}

fn read_cached_list(state: &State<'_, AppState>) -> Option<PlannerListResult> {
    let conn = state.db.lock().unwrap();
    let c = crate::workout_source_cache::get(&conn, "planner", "list")?;
    let rows: Vec<PlannerWorkout> = serde_json::from_str(&c.value).ok()?;
    Some(PlannerListResult { rows, from_cache: true, fetched_at_ms: c.fetched_at_ms })
}

/// Startup hydration: cached list only, no network.
#[tauri::command]
pub async fn planner_cached(
    state: State<'_, AppState>,
) -> Result<Option<PlannerListResult>, AppError> {
    let cached = read_cached_list(&state);
    if let Some(r) = &cached {
        *state.planner_cache.lock().unwrap() = r.rows.clone();
    }
    Ok(cached)
}

/// Network fetch, write-through to the cache; on failure fall back to the
/// cached copy (offline mode) before surfacing an error.
#[tauri::command]
pub async fn planner_list(state: State<'_, AppState>) -> Result<PlannerListResult, AppError> {
    let cfg = cfg_checked(&state)?;
    match fetch_workouts(&cfg).await {
        Ok(list) => {
            *state.planner_cache.lock().unwrap() = list.clone();
            let now = crate::app_state::now_unix_ms() as i64;
            let conn = state.db.lock().unwrap();
            crate::workout_source_cache::put(
                &conn,
                "planner",
                "list",
                None,
                &serde_json::to_string(&list).unwrap_or_default(),
                now,
            );
            Ok(PlannerListResult { rows: list, from_cache: false, fetched_at_ms: now })
        }
        Err(e) => match read_cached_list(&state) {
            Some(r) => {
                *state.planner_cache.lock().unwrap() = r.rows.clone();
                Ok(r)
            }
            None => Err(e),
        },
    }
}

/// Fetch ZWO → existing import pipeline (sha256 dedup) → tag origin →
/// load into the player. spec §B2.
#[tauri::command]
pub async fn planner_ride(
    app: AppHandle,
    state: State<'_, AppState>,
    wid: i64,
) -> Result<PlayerState, AppError> {
    let cfg = cfg_checked(&state)?;
    let ftp = state.settings().profile.ftp;
    let zwo = match fetch_zwo(&cfg, wid, ftp).await {
        Ok(z) => z,
        Err(e) => {
            // Offline fallback: cached ZWO from the last preview of this
            // workout — possibly stale, so say so.
            let cached = {
                let conn = state.db.lock().unwrap();
                crate::workout_source_cache::get(&conn, "planner", &format!("preview:{wid}"))
            }
            .and_then(|c| serde_json::from_str::<StoredPreview>(&c.value).ok());
            match cached {
                Some(s) if !s.zwo.is_empty() => {
                    let _ = tauri::Emitter::emit(&app, "toast", serde_json::json!({
                        "level": "warn",
                        "message": "WorkoutPlanner unreachable — riding the cached copy",
                    }));
                    s.zwo
                }
                _ => return Err(e),
            }
        }
    };
    crate::workout_sources::ride_from_zwo(app, &state, &zwo, "planner", &wid.to_string(), Some(wid))
        .await
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct PlannerPreview {
    pub wid: i64,
    pub graph: Vec<(u32, f64)>,
    pub duration_s: u32,
    pub est_if: f64,
    pub est_tss: f64,
    pub segments: Vec<crate::commands::workout::SegmentRow>,
}

/// Lazy per-workout preview: fetch ZWO → existing parser → graph polyline
/// (same shape the library thumbnails use). Cached in AppState keyed by
/// wid + ZWO content hash, so each workout costs one fetch until it changes.
#[tauri::command]
pub async fn planner_preview(
    state: State<'_, AppState>,
    wid: i64,
) -> Result<PlannerPreview, AppError> {
    let cfg = cfg_checked(&state)?;
    let ftp = state.settings().profile.ftp;

    // Cache check BEFORE any network: key on the DSL text from the last
    // /workouts sync (changes when the workout changes). Without this, every
    // preview render cost a fetch even on hit.
    let dsl_key = state
        .planner_cache
        .lock()
        .unwrap()
        .iter()
        .find(|w| w.wid == wid)
        .map(|w| format!("{:x}", sha2::Sha256::digest(w.dsl.as_bytes())));
    if let Some(key) = &dsl_key {
        if let Some((cached, p)) = state.planner_previews.lock().unwrap().get(&wid) {
            if cached == key {
                return Ok(p.clone());
            }
        }
        // L2: persistent cache (survives restarts; enables offline).
        let db_hit = {
            let conn = state.db.lock().unwrap();
            crate::workout_source_cache::get(&conn, "planner", &format!("preview:{wid}"))
        };
        if let Some(c) = db_hit {
            if c.content_hash.as_deref() == Some(key.as_str()) {
                if let Ok(stored) = serde_json::from_str::<StoredPreview>(&c.value) {
                    state
                        .planner_previews
                        .lock()
                        .unwrap()
                        .insert(wid, (key.clone(), stored.preview.clone()));
                    return Ok(stored.preview);
                }
            }
        }
    }

    let zwo = fetch_zwo(&cfg, wid, ftp).await?;
    let sha = dsl_key
        .unwrap_or_else(|| format!("{:x}", sha2::Sha256::digest(zwo.as_bytes())));
    if let Some((cached_sha, p)) = state.planner_previews.lock().unwrap().get(&wid) {
        if *cached_sha == sha {
            return Ok(p.clone());
        }
    }

    let parsed = tp_core::parse::parse_zwo(&zwo)
        .map_err(|e| AppError::new("planner_bad_workout", format!("planner ZWO: {e}")))?;
    let w = &parsed.workout;
    let (est_if, est_tss) = tp_core::metrics::estimate_if_tss(w, ftp);
    let preview = PlannerPreview {
        wid,
        graph: crate::commands::workout::graph_points(w, ftp),
        duration_s: w.duration_s(),
        est_if,
        est_tss,
        segments: crate::commands::workout::segment_rows(w, ftp),
    };
    state.planner_previews.lock().unwrap().insert(wid, (sha.clone(), preview.clone()));
    {
        let conn = state.db.lock().unwrap();
        crate::workout_source_cache::put(
            &conn,
            "planner",
            &format!("preview:{wid}"),
            Some(&sha),
            &serde_json::to_string(&StoredPreview { preview: preview.clone(), zwo })
                .unwrap_or_default(),
            crate::app_state::now_unix_ms() as i64,
        );
    }
    Ok(preview)
}

/// Persistent preview payload: the computed preview plus the ZWO text, so
/// rides can fall back to the cached copy offline.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct StoredPreview {
    preview: PlannerPreview,
    zwo: String,
}

/// Open the planner's web editor for a cached workout. Until A4 wid-links
/// ship, the URL carries title/sport/DSL query params (spec §B3).
#[tauri::command]
pub async fn planner_open_editor(
    app: AppHandle,
    state: State<'_, AppState>,
    wid: i64,
) -> Result<(), AppError> {
    let cfg = cfg_checked(&state)?;
    let cached = state
        .planner_cache
        .lock()
        .unwrap()
        .iter()
        .find(|w| w.wid == wid)
        .cloned();
    let w = match cached {
        Some(w) => w,
        None => fetch_workouts(&cfg)
            .await?
            .into_iter()
            .find(|w| w.wid == wid)
            .ok_or_else(|| AppError::new("planner_bad_workout", "workout not found"))?,
    };
    let url = format!(
        "{}/?t={}&st=1&w={}",
        base_url(&cfg),
        urlencode(&w.title),
        urlencode(&w.dsl),
    );
    app.opener()
        .open_url(url, None::<String>)
        .map_err(|e| AppError::new("io", e.to_string()))
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One-shot HTTP stub: accepts a single connection, returns `response`,
    /// records the request head. Dependency-free.
    fn stub(response: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap();
            sock.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (addr, handle)
    }

    fn http(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn cfg(url: &str, user: &str) -> PlannerSettings {
        PlannerSettings { url: url.into(), user: user.into(), pass: "pw".into(), enabled: true }
    }

    #[tokio::test]
    async fn fetch_workouts_parses_and_filters_bike() {
        let body = r#"[
          {"id":1,"title":"SS 3x12","value":"3[(12min,88),(4min,50)]","tags":"","duration_sec":3720,"tss":74,"sport_type":1},
          {"id":2,"title":"Swim","value":"(30min,70)","tags":null,"duration_sec":1800,"tss":40,"sport_type":0}
        ]"#;
        let (url, h) = stub(http("200 OK", body));
        let out = fetch_workouts(&cfg(&url, "")).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].wid, 1);
        assert_eq!(out[0].duration_s, 3720);
        let req = h.join().unwrap();
        assert!(req.starts_with("GET /workouts"));
        assert!(!req.contains("Authorization"), "no auth header when user empty");
    }

    #[tokio::test]
    async fn basic_auth_header_sent_and_401_maps() {
        let (url, h) = stub(http("401 Unauthorized", "Unauthorized"));
        let err = fetch_workouts(&cfg(&url, "user")).await.unwrap_err();
        assert_eq!(err.code, "planner_auth");
        let req = h.join().unwrap();
        assert!(req.contains("authorization: Basic") || req.contains("Authorization: Basic"));
    }

    #[tokio::test]
    async fn fetch_zwo_roundtrips_through_parser() {
        let zwo = r#"<workout_file><name>SS</name><description>d</description>
            <sportType>bike</sportType><workout>
            <SteadyState Duration="60" Power="0.8"/></workout></workout_file>"#;
        let (url, h) = stub(http("200 OK", zwo));
        let text = fetch_zwo(&cfg(&url, ""), 42, 250).await.unwrap();
        // The interchange contract, verified from the consuming side:
        let parsed = tp_core::parse::parse_zwo(&text).expect("planner ZWO parses");
        assert_eq!(parsed.workout.duration_s(), 60);
        let req = h.join().unwrap();
        assert!(req.starts_with("GET /workout_file?wid=42&format=zwo&ftp=250"));
    }

    #[tokio::test]
    async fn unreachable_maps_to_planner_unreachable() {
        // Nothing listening on this port.
        let err = fetch_workouts(&cfg("http://127.0.0.1:1", "")).await.unwrap_err();
        assert_eq!(err.code, "planner_unreachable");
    }

    #[test]
    fn parse_workouts_tolerates_wrapper_and_string_numbers() {
        let wrapped = r#"{"data":[
          {"id":"7","title":"T","value":"(1min,50)","tags":null,"duration_sec":"60","tss":"4.5","sport_type":"1"},
          {"id":8,"title":"Run","value":"x","sport_type":2}
        ],"error":""}"#;
        let out = parse_workouts_body(wrapped).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].wid, 7);
        assert_eq!(out[0].duration_s, 60);
        assert_eq!(out[0].tss, Some(4.5));
    }

    #[test]
    fn parse_workouts_error_carries_snippet() {
        let err = parse_workouts_body("<html>login page</html>").unwrap_err();
        assert_eq!(err.code, "planner_bad_response");
        assert!(err.message.contains("<html>login"), "{}", err.message);
    }

    #[test]
    fn urlencode_dsl() {
        assert_eq!(urlencode("3[(12min, 88)]"), "3%5B%2812min%2C%2088%29%5D");
    }
}
