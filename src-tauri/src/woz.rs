//! whatsonzwift.com workout source. Read-only scrape of public pages:
//! /workouts lists ~150 collections; each collection page carries every
//! workout's full interval structure inline as `textbar` lines
//! ("3x 2min @ 105% FTP, 1min @ 90% FTP"), which we parse into the normal
//! Workout model. Riding serializes via tp_core's ZWO writer into the shared
//! source pipeline. Results are cached in-memory per collection.

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use tp_core::model::{PowerTarget, Segment, SourceFormat, Workout};

use crate::err::AppError;
use crate::runtime::PlayerState;
use crate::sources;
use crate::state::AppState;

const BASE: &str = "https://whatsonzwift.com";
const UA: &str = "TrainerPro/0.1 (+https://github.com/sergioclemente/TrainerPro)";

const TTL_MS: i64 = 3_600_000; // 1 h — Sync forces a refresh

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct WozCollection {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct WozWorkout {
    /// Index within its collection (stable per fetch; rides reference it).
    pub idx: usize,
    pub title: String,
    pub duration_s: u32,
    pub est_if: f64,
    pub est_tss: f64,
    pub graph: Vec<(u32, f64)>,
    pub segments: Vec<crate::cmd::SegmentRow>,
}

async fn fetch(path: &str) -> Result<String, AppError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(UA)
        .build()
        .map_err(|e| AppError::new("woz_unreachable", e.to_string()))?;
    let resp = client
        .get(format!("{BASE}{path}"))
        .send()
        .await
        .map_err(|e| AppError::new("woz_unreachable", format!("whatsonzwift: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::new(
            "woz_unreachable",
            format!("whatsonzwift returned {}", resp.status()),
        ));
    }
    resp.text().await.map_err(|e| AppError::new("woz_unreachable", e.to_string()))
}

// ---------------------------------------------------------------------------
// HTML/text parsing (pure; unit-tested)
// ---------------------------------------------------------------------------

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&#039;", "'")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Collections on the index are cards: an empty overlay `<a>` carries the
/// href; the visible title sits in a preceding `<p class="m-0 self-center…">`
/// with a sport glyph (`flaticon-bike`). We slice the html between anchors,
/// read the last title-p in each slice, and filter to cards mentioning the
/// bike glyph. Fallback: humanized slug, so a site redesign degrades to
/// usable names instead of an empty list.
pub fn parse_collections(html: &str) -> Vec<WozCollection> {
    let a_re = regex::Regex::new(
        r#"<a[^>]*href="https://whatsonzwift\.com/workouts/([a-z0-9-]+)""#,
    )
    .unwrap();
    let title_re =
        regex::Regex::new(r#"(?s)<p class="m-0 self-center[^"]*">(.*?)</p>"#).unwrap();

    let anchors: Vec<(usize, String)> = a_re
        .captures_iter(html)
        .map(|c| (c.get(0).unwrap().start(), c[1].to_string()))
        .collect();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut prev_end = 0usize;
    for (pos, slug) in anchors {
        let slice = &html[prev_end..pos];
        prev_end = pos;
        if !seen.insert(slug.clone()) {
            continue;
        }
        // Bike-only cards (skip run/swim-only collections when detectable).
        if slice.contains("flaticon-") && !slice.contains("flaticon-bike") {
            continue;
        }
        let title = title_re
            .captures_iter(slice)
            .last()
            .map(|c| strip_tags(&c[1]))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| humanize_slug(&slug));
        out.push(WozCollection { slug, title });
    }
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    out
}

fn humanize_slug(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One duration token sequence: "1hr 5min 30sec" → seconds.
fn parse_duration(s: &str) -> Option<u32> {
    let re = regex::Regex::new(r"(\d+)\s*(hr|min|sec)").unwrap();
    let mut total = 0u32;
    let mut any = false;
    for cap in re.captures_iter(s) {
        let n: u32 = cap[1].parse().ok()?;
        total += match &cap[2] {
            "hr" => n * 3600,
            "min" => n * 60,
            _ => n,
        };
        any = true;
    }
    if any { Some(total) } else { None }
}

/// Split a textbar body into steps: commas separate steps, but a comma can
/// also separate cadence from power within one step ("30sec @ 105rpm, 95%
/// FTP") — pieces that don't begin with a duration merge into the previous.
fn split_steps(body: &str) -> Vec<String> {
    let starts_with_duration = |p: &str| {
        regex::Regex::new(r"^\d+\s*(hr|min|sec)").unwrap().is_match(p.trim_start())
    };
    let mut steps: Vec<String> = Vec::new();
    for piece in body.split(',') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        if steps.is_empty() || starts_with_duration(piece) {
            steps.push(piece.to_string());
        } else {
            let last = steps.last_mut().unwrap();
            last.push_str(", ");
            last.push_str(piece);
        }
    }
    steps
}

/// Parse one step ("2min @ 105rpm, 95% FTP" / "5min from 40 to 105% FTP" /
/// "10min free ride" / "30sec MAX") into a Segment.
fn parse_step(step: &str) -> Option<Segment> {
    let dur = parse_duration(step)?;
    if dur == 0 {
        return None;
    }
    let lower = step.to_lowercase();
    let cadence = regex::Regex::new(r"(\d+)\s*rpm")
        .unwrap()
        .captures(&lower)
        .and_then(|c| c[1].parse::<u16>().ok());

    if lower.contains("free ride") || lower.contains("freeride") {
        return Some(Segment::FreeRide { duration_s: dur });
    }
    if let Some(cap) = regex::Regex::new(r"from\s+(\d+(?:\.\d+)?)\s*(?:%\s*)?to\s+(\d+(?:\.\d+)?)\s*%\s*ftp")
        .unwrap()
        .captures(&lower)
    {
        let a: f64 = cap[1].parse().ok()?;
        let b: f64 = cap[2].parse().ok()?;
        return Some(Segment::Ramp {
            duration_s: dur,
            start: PowerTarget::PercentFtp(a / 100.0),
            end: PowerTarget::PercentFtp(b / 100.0),
            cadence_rpm: cadence,
        });
    }
    if lower.contains("max") {
        return Some(Segment::Steady {
            duration_s: dur,
            power: PowerTarget::PercentFtp(1.5),
            cadence_rpm: cadence,
        });
    }
    if let Some(cap) = regex::Regex::new(r"(\d+(?:\.\d+)?)\s*%\s*ftp")
        .unwrap()
        .captures(&lower)
    {
        let p: f64 = cap[1].parse().ok()?;
        return Some(Segment::Steady {
            duration_s: dur,
            power: PowerTarget::PercentFtp(p / 100.0),
            cadence_rpm: cadence,
        });
    }
    None // running pace lines etc. — signals "not a bike workout"
}

/// Parse one textbar line, expanding an optional "Nx " repeat prefix.
pub fn parse_textbar(line: &str) -> Option<Vec<Segment>> {
    let line = line.trim();
    let (reps, body) = match regex::Regex::new(r"^(\d+)x\s+(.*)$").unwrap().captures(line) {
        Some(cap) => (cap[1].parse::<u32>().ok()?.clamp(1, 100), cap[2].to_string()),
        None => (1, line.to_string()),
    };
    let steps: Option<Vec<Segment>> = split_steps(&body).iter().map(|s| parse_step(s)).collect();
    let steps = steps?;
    if steps.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(steps.len() * reps as usize);
    for _ in 0..reps {
        out.extend(steps.iter().cloned());
    }
    Some(out)
}

/// Parse a collection page: h3-titled workout sections, each with textbars.
/// Workouts with any unparseable line (e.g. running pace) are skipped.
pub fn parse_collection_page(html: &str) -> Vec<(String, Workout)> {
    let h3 = regex::Regex::new(r"<h3[^>]*>(.*?)</h3>").unwrap();
    let bar = regex::Regex::new(r#"<div class="textbar"[^>]*>(.*?)</div>"#).unwrap();

    // Section boundaries: h3 positions.
    let mut sections: Vec<(String, usize, usize)> = Vec::new();
    let matches: Vec<_> = h3.captures_iter(html).collect();
    for (i, cap) in matches.iter().enumerate() {
        let title = strip_tags(&cap[1]);
        let start = cap.get(0).unwrap().end();
        let end = matches
            .get(i + 1)
            .map(|c| c.get(0).unwrap().start())
            .unwrap_or(html.len());
        if !title.is_empty() {
            sections.push((title, start, end));
        }
    }

    let mut out = Vec::new();
    for (title, start, end) in sections {
        let slice = &html[start..end];
        let mut segments: Vec<Segment> = Vec::new();
        let mut ok = true;
        let mut any = false;
        for cap in bar.captures_iter(slice) {
            any = true;
            match parse_textbar(&strip_tags(&cap[1])) {
                Some(mut segs) => segments.append(&mut segs),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && any && !segments.is_empty() {
            let w = Workout {
                name: title.clone(),
                description: String::new(),
                source_format: SourceFormat::Zwo,
                segments,
                text_events: vec![],
            };
            out.push((title, w));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn db_get(state: &State<'_, AppState>, key: &str) -> Option<crate::cache::Cached> {
    let conn = state.db.lock().unwrap();
    crate::cache::get(&conn, "woz", key)
}

fn db_put(state: &State<'_, AppState>, key: &str, value: &str) {
    let conn = state.db.lock().unwrap();
    crate::cache::put(&conn, "woz", key, None, value, crate::state::now_unix_ms() as i64);
}

fn fresh(c: &crate::cache::Cached) -> bool {
    (crate::state::now_unix_ms() as i64) - c.fetched_at_ms < TTL_MS
}

/// Cached-first with 1 h TTL; `force` (the Sync button) always refetches.
/// Network failure falls back to the cache at any age.
#[tauri::command]
pub async fn woz_collections(
    state: State<'_, AppState>,
    force: Option<bool>,
) -> Result<Vec<WozCollection>, AppError> {
    let force = force.unwrap_or(false);
    if !force {
        if let Some(cached) = state.woz_collections.lock().unwrap().clone() {
            return Ok(cached);
        }
        if let Some(c) = db_get(&state, "collections").filter(fresh) {
            if let Ok(list) = serde_json::from_str::<Vec<WozCollection>>(&c.value) {
                *state.woz_collections.lock().unwrap() = Some(list.clone());
                return Ok(list);
            }
        }
    }
    let fetched = async {
        let html = fetch("/workouts").await?;
        let list = parse_collections(&html);
        if list.is_empty() {
            return Err(AppError::new(
                "woz_parse",
                "no collections found — whatsonzwift.com layout may have changed",
            ));
        }
        Ok(list)
    }
    .await;
    match fetched {
        Ok(list) => {
            db_put(&state, "collections", &serde_json::to_string(&list).unwrap_or_default());
            *state.woz_collections.lock().unwrap() = Some(list.clone());
            Ok(list)
        }
        Err(e) => match db_get(&state, "collections")
            .and_then(|c| serde_json::from_str::<Vec<WozCollection>>(&c.value).ok())
        {
            Some(list) => {
                *state.woz_collections.lock().unwrap() = Some(list.clone());
                Ok(list)
            }
            None => Err(e),
        },
    }
}

type CollectionEntries = Vec<(Workout, WozWorkout)>;

fn load_collection_from_db(state: &State<'_, AppState>, collection: &str) -> Option<CollectionEntries> {
    db_get(state, &format!("collection:{collection}"))
        .and_then(|c| serde_json::from_str::<CollectionEntries>(&c.value).ok())
}

#[tauri::command]
pub async fn woz_workouts(
    state: State<'_, AppState>,
    collection: String,
    force: Option<bool>,
) -> Result<Vec<WozWorkout>, AppError> {
    let force = force.unwrap_or(false);
    let ftp = state.settings().profile.ftp;
    if !force {
        if let Some(cached) = state.woz_cache.lock().unwrap().get(&collection) {
            return Ok(cached.iter().map(|(_, meta)| meta.clone()).collect());
        }
        if let Some(c) = db_get(&state, &format!("collection:{collection}")).filter(fresh) {
            if let Ok(entries) = serde_json::from_str::<CollectionEntries>(&c.value) {
                let metas = entries.iter().map(|(_, m)| m.clone()).collect();
                state.woz_cache.lock().unwrap().insert(collection, entries);
                return Ok(metas);
            }
        }
    }
    let html = match fetch(&format!("/workouts/{collection}")).await {
        Ok(h) => h,
        Err(e) => {
            // Offline fallback: cached copy at any age.
            if let Some(entries) = load_collection_from_db(&state, &collection) {
                let metas = entries.iter().map(|(_, m)| m.clone()).collect();
                state.woz_cache.lock().unwrap().insert(collection, entries);
                return Ok(metas);
            }
            return Err(e);
        }
    };
    let parsed = parse_collection_page(&html);
    if parsed.is_empty() {
        return Err(AppError::new(
            "woz_parse",
            "no ridable bike workouts found in this collection",
        ));
    }
    let entries: Vec<(Workout, WozWorkout)> = parsed
        .into_iter()
        .enumerate()
        .map(|(idx, (title, w))| {
            let (est_if, est_tss) = tp_core::metrics::estimate_if_tss(&w, ftp);
            let meta = WozWorkout {
                idx,
                title,
                duration_s: w.duration_s(),
                est_if,
                est_tss,
                graph: crate::cmd::graph_points(&w, ftp),
                segments: crate::cmd::segment_rows(&w, ftp),
            };
            (w, meta)
        })
        .collect();
    let metas = entries.iter().map(|(_, m)| m.clone()).collect();
    db_put(
        &state,
        &format!("collection:{collection}"),
        &serde_json::to_string(&entries).unwrap_or_default(),
    );
    state.woz_cache.lock().unwrap().insert(collection, entries);
    Ok(metas)
}

#[tauri::command]
pub async fn woz_ride(
    app: AppHandle,
    state: State<'_, AppState>,
    collection: String,
    idx: usize,
) -> Result<PlayerState, AppError> {
    let workout = {
        let hit = state
            .woz_cache
            .lock()
            .unwrap()
            .get(&collection)
            .and_then(|v| v.get(idx))
            .map(|(w, _)| w.clone());
        match hit {
            Some(w) => w,
            None => {
                let entries = load_collection_from_db(&state, &collection).ok_or_else(|| {
                    AppError::new("woz_stale", "workout not in cache — re-open the collection")
                })?;
                let w = entries
                    .get(idx)
                    .map(|(w, _)| w.clone())
                    .ok_or_else(|| AppError::new("woz_stale", "workout not in cache"))?;
                state.woz_cache.lock().unwrap().insert(collection.clone(), entries);
                w
            }
        }
    };
    let zwo = tp_core::parse::zwo::to_zwo(&workout);
    let origin_ref = format!("{collection}#{idx}");
    sources::ride_from_zwo(app, &state, &zwo, "whatsonzwift", &origin_ref, None).await
}

/// Attribution / browse-out: open the collection on whatsonzwift.com.
#[tauri::command]
pub async fn woz_open_page(app: AppHandle, collection: String) -> Result<(), AppError> {
    app.opener()
        .open_url(format!("{BASE}/workouts/{collection}"), None::<String>)
        .map_err(|e| AppError::new("io", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steady_ramp_freeride_max() {
        let segs = parse_textbar("5min from 40 to 105% FTP").unwrap();
        assert!(matches!(segs[0], Segment::Ramp { duration_s: 300, .. }));
        let segs = parse_textbar("2min @ 50% FTP").unwrap();
        match &segs[0] {
            Segment::Steady { duration_s, power, .. } => {
                assert_eq!(*duration_s, 120);
                assert_eq!(power.resolve(200, 1.0), 100);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse_textbar("10min free ride").unwrap()[0],
            Segment::FreeRide { duration_s: 600 }
        ));
        let segs = parse_textbar("30sec MAX").unwrap();
        match &segs[0] {
            Segment::Steady { power, .. } => assert_eq!(power.resolve(200, 1.0), 300),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn repeat_with_cadence_comma_inside_step() {
        // Comma separates cadence from power INSIDE a step, and steps from
        // each other — the duration-lookahead merge must distinguish them.
        let segs = parse_textbar("4x 30sec @ 105rpm, 95% FTP, 30sec @ 85rpm, 55% FTP").unwrap();
        assert_eq!(segs.len(), 8);
        match &segs[0] {
            Segment::Steady { duration_s, power, cadence_rpm } => {
                assert_eq!(*duration_s, 30);
                assert_eq!(power.resolve(100, 1.0), 95);
                assert_eq!(*cadence_rpm, Some(105));
            }
            other => panic!("{other:?}"),
        }
        match &segs[1] {
            Segment::Steady { power, cadence_rpm, .. } => {
                assert_eq!(power.resolve(100, 1.0), 55);
                assert_eq!(*cadence_rpm, Some(85));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn compound_duration_and_reject_running_pace() {
        let segs = parse_textbar("1min 30sec @ 80% FTP").unwrap();
        assert!(matches!(segs[0], Segment::Steady { duration_s: 90, .. }));
        // Running lines (no % FTP) must fail → workout skipped.
        assert!(parse_textbar("2min @ 80% 5k pace").is_none());
    }

    #[test]
    fn parses_collection_page_sections() {
        let html = r#"
          <h3 class="x"> Over-Unders</h3>
          <div class="textbar" style="a">5min from <span>40</span> to <span>105</span>% FTP</div>
          <div class="textbar">3x 2min @ <span>105</span>% FTP,<br /> 1min @ <span>90</span>% FTP</div>
          <h3> Run Section</h3>
          <div class="textbar">2min @ 80% 5k pace</div>
          <h3> Another Bike</h3>
          <div class="textbar">10min @ 55% FTP</div>
        "#;
        let out = parse_collection_page(html);
        assert_eq!(out.len(), 2, "run section skipped");
        assert_eq!(out[0].0, "Over-Unders");
        assert_eq!(out[0].1.segments.len(), 1 + 6);
        assert_eq!(out[0].1.duration_s(), 300 + 3 * 180);
        assert_eq!(out[1].0, "Another Bike");
    }

    #[test]
    fn parses_collection_index_card_layout() {
        // Real layout: empty overlay anchor; title in a preceding p tag.
        let html = r#"
          <li><p class="m-0 self-center max-h-16"> <i class="glyph-icon flaticon-bike"></i> 10-12wk FTP Builder </p>
          <a href="https://whatsonzwift.com/workouts/10-12wk-ftp-builder"><span class="absolute inset-0"></span></a></li>
          <li><p class="m-0 self-center max-h-16"> <i class="glyph-icon flaticon-run"></i> Run Plan </p>
          <a href="https://whatsonzwift.com/workouts/3run-13-1"><span class="absolute inset-0"></span></a></li>
          <li><a href="https://whatsonzwift.com/workouts/mystery"><span class="absolute inset-0"></span></a></li>
        "#;
        let out = parse_collections(html);
        assert_eq!(out.len(), 2, "run-only collection skipped: {out:?}");
        assert_eq!(out[0].slug, "10-12wk-ftp-builder");
        assert_eq!(out[0].title, "10-12wk FTP Builder");
        // No title found anywhere -> humanized slug fallback.
        assert_eq!(out[1].title, "Mystery");
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test]
    #[ignore] // network; run explicitly for diagnosis
    async fn live_fetch_collections_and_one_page() {
        let html = fetch("/workouts").await.expect("index fetch");
        let cols = parse_collections(&html);
        println!("collections: {}", cols.len());
        assert!(!cols.is_empty(), "no collections parsed; html len {}", html.len());
        let html = fetch("/workouts/threshold").await.expect("collection fetch");
        let parsed = parse_collection_page(&html);
        println!("threshold workouts: {}", parsed.len());
        for (t, w) in parsed.iter().take(3) {
            println!("  {t}: {} segs, {}s", w.segments.len(), w.duration_s());
        }
        assert!(!parsed.is_empty(), "no workouts parsed; textbars in html: {}",
            html.matches("textbar").count());
    }
}
