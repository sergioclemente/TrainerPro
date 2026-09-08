//! Managed application state + profile/settings access. SPEC.md §8.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::database as db;
use crate::device_hub::DeviceHub;
use crate::player_runtime::PlayerHandle;

pub struct AppState {
    pub db: Mutex<Connection>,
    pub data_dir: PathBuf,
    pub hub: DeviceHub,
    pub player: tokio::sync::Mutex<Option<PlayerHandle>>,
    /// Last planner_list result; used for edit-URL construction.
    pub planner_cache: Mutex<Vec<crate::workout_planner_source::PlannerWorkout>>,
    /// Preview cache: wid → (zwo sha256, preview). Invalidated by hash.
    pub planner_previews:
        Mutex<std::collections::HashMap<i64, (String, crate::workout_planner_source::PlannerPreview)>>,
    /// whatsonzwift caches (per app run).
    pub woz_collections: Mutex<Option<Vec<crate::whatsonzwift_source::WozCollection>>>,
    pub woz_cache: Mutex<
        std::collections::HashMap<String, Vec<(tp_core::model::Workout, crate::whatsonzwift_source::WozWorkout)>>,
    >,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub ftp: u16,
    pub weight_kg: f64,
}

impl Default for Profile {
    fn default() -> Self {
        Profile { name: String::new(), ftp: 200, weight_kg: 75.0 }
    }
}

/// One workout-library provider's persisted state — the plugin surface. A
/// provider has a string id (`planner`, `woz`, …), an `enabled` toggle, and a
/// stringly-typed `values` bag whose keys are declared per-provider by the
/// frontend descriptor (`frontend/sources.ts`). Adding a provider needs no schema
/// change here, just a new id + its fetch code. Reused by trainer-coach.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceConfig {
    pub enabled: bool,
    #[serde(default)]
    pub values: HashMap<String, String>,
}

impl SourceConfig {
    pub fn get(&self, key: &str) -> String {
        self.values.get(key).cloned().unwrap_or_default()
    }
}

/// Typed view of the `planner` provider's config, so the WorkoutPlanner source's fetch code
/// keeps concrete fields instead of reaching into the values bag by hand.
#[derive(Debug, Clone)]
pub struct PlannerSettings {
    pub url: String,
    pub user: String,
    pub pass: String,
    pub enabled: bool,
}

impl PlannerSettings {
    pub fn from_source(c: &SourceConfig) -> Self {
        PlannerSettings {
            url: c.get("url"),
            user: c.get("user"),
            pass: c.get("pass"),
            enabled: c.enabled,
        }
    }
}

/// Built-in provider defaults, so the Libraries list is always complete even
/// before anything is saved. Both remote providers are opt-in: `woz` fetches
/// from a third-party site, `planner` needs credentials — the user enables
/// them in Settings → Libraries.
fn default_sources() -> HashMap<String, SourceConfig> {
    let mut m = HashMap::new();
    m.insert("woz".to_string(), SourceConfig { enabled: false, values: HashMap::new() });
    m.insert("planner".to_string(), SourceConfig { enabled: false, values: HashMap::new() });
    m
}

/// Resolve the providers map from persisted settings, starting from the
/// built-in defaults so both known providers are always present. Prefer the
/// new `sources` blob; else migrate the legacy typed `planner` key so existing
/// installs keep their credentials. Pure (no DB) so the migration is testable.
fn load_sources(saved_sources: Option<&str>, legacy_planner: Option<&str>) -> HashMap<String, SourceConfig> {
    let mut sources = default_sources();
    if let Some(v) = saved_sources {
        if let Ok(m) = serde_json::from_str::<HashMap<String, SourceConfig>>(v) {
            sources.extend(m);
        }
    } else if let Some(v) = legacy_planner {
        #[derive(Deserialize)]
        struct LegacyPlanner {
            url: String,
            user: String,
            pass: String,
            enabled: bool,
        }
        if let Ok(p) = serde_json::from_str::<LegacyPlanner>(v) {
            sources.insert(
                "planner".to_string(),
                SourceConfig {
                    enabled: p.enabled,
                    values: HashMap::from([
                        ("url".to_string(), p.url),
                        ("user".to_string(), p.user),
                        ("pass".to_string(), p.pass),
                    ]),
                },
            );
        }
    }
    sources
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub profile: Profile,
    pub record_distance: bool,
    pub intensity_default: f64,
    /// Extra folder FIT files are copied to at ride end (friendly names).
    /// None = app data dir only.
    #[serde(default)]
    pub export_dir: Option<String>,
    /// Workout-library providers, keyed by id (see `SourceConfig`). Replaces
    /// the old typed `planner` object; `woz` is now a first-class entry.
    #[serde(default)]
    pub sources: HashMap<String, SourceConfig>,
}

impl Settings {
    /// A provider's config by id (default = disabled, empty).
    pub fn source(&self, id: &str) -> SourceConfig {
        self.sources.get(id).cloned().unwrap_or_default()
    }
    /// Typed WorkoutPlanner view over `sources["planner"]`.
    pub fn planner(&self) -> PlannerSettings {
        PlannerSettings::from_source(&self.source("planner"))
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            profile: Profile::default(),
            record_distance: false,
            intensity_default: 1.0,
            export_dir: None,
            sources: default_sources(),
        }
    }
}

impl AppState {
    pub fn settings(&self) -> Settings {
        let conn = self.db.lock().unwrap();
        let mut s = Settings::default();
        if let Some(v) = db::get_setting(&conn, "profile") {
            if let Ok(p) = serde_json::from_str(&v) {
                s.profile = p;
            }
        }
        if let Some(v) = db::get_setting(&conn, "record_distance") {
            s.record_distance = v == "true";
        }
        if let Some(v) = db::get_setting(&conn, "intensity_default") {
            if let Ok(i) = v.parse() {
                s.intensity_default = i;
            }
        }
        if let Some(v) = db::get_setting(&conn, "export_dir") {
            if !v.is_empty() {
                s.export_dir = Some(v);
            }
        }
        s.sources = load_sources(
            db::get_setting(&conn, "sources").as_deref(),
            db::get_setting(&conn, "planner").as_deref(),
        );
        s
    }

    pub fn save_settings(&self, s: &Settings) -> Result<(), rusqlite::Error> {
        let conn = self.db.lock().unwrap();
        db::set_setting(&conn, "profile", &serde_json::to_string(&s.profile).unwrap())?;
        db::set_setting(&conn, "record_distance", if s.record_distance { "true" } else { "false" })?;
        db::set_setting(&conn, "intensity_default", &s.intensity_default.to_string())?;
        db::set_setting(&conn, "export_dir", s.export_dir.as_deref().unwrap_or(""))?;
        db::set_setting(&conn, "sources", &serde_json::to_string(&s.sources).unwrap())?;
        Ok(())
    }

    pub fn workouts_dir(&self) -> PathBuf {
        self.data_dir.join("workouts")
    }
    pub fn rides_dir(&self) -> PathBuf {
        self.data_dir.join("rides")
    }
}

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_install_gets_default_providers() {
        let s = load_sources(None, None);
        assert!(!s["woz"].enabled, "Zwift off until enabled");
        assert!(!s["planner"].enabled, "planner off until configured");
        assert_eq!(s["planner"].get("url"), "");
    }

    #[test]
    fn legacy_planner_key_migrates_with_creds() {
        let legacy = r#"{"url":"https://x.example.com","user":"user","pass":"pw","enabled":true}"#;
        let s = load_sources(None, Some(legacy));
        let p = &s["planner"];
        assert!(p.enabled);
        assert_eq!(p.get("url"), "https://x.example.com");
        assert_eq!(p.get("user"), "user");
        assert_eq!(p.get("pass"), "pw");
        // Defaults for the other provider still present.
        assert!(!s["woz"].enabled);
    }

    #[test]
    fn saved_sources_win_over_legacy_and_defaults() {
        let saved = r#"{"planner":{"enabled":true,"values":{"url":"u"}},"woz":{"enabled":false,"values":{}}}"#;
        let legacy = r#"{"url":"ignored","user":"","pass":"","enabled":false}"#;
        let s = load_sources(Some(saved), Some(legacy));
        assert!(s["planner"].enabled);
        assert_eq!(s["planner"].get("url"), "u");
        assert!(!s["woz"].enabled, "explicit disable respected");
    }

    #[test]
    fn planner_view_reads_from_values_bag() {
        let mut sources = default_sources();
        sources.insert(
            "planner".into(),
            SourceConfig {
                enabled: true,
                values: HashMap::from([
                    ("url".into(), "https://x".into()),
                    ("user".into(), "u".into()),
                    ("pass".into(), "p".into()),
                ]),
            },
        );
        let s = Settings { sources, ..Settings::default() };
        let p = s.planner();
        assert!(p.enabled);
        assert_eq!(p.url, "https://x");
        assert_eq!(p.user, "u");
        assert_eq!(p.pass, "p");
    }
}
