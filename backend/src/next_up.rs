//! Next Up read model: scheduled workouts followed by local recommendations.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::State;

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::commands::workout::{self, WorkoutSummary};
use crate::database::{provider_connections, scheduled_workouts, workout_definitions};
use crate::intervals_icu::PROVIDER_ID;
use crate::local_date;

const LOCAL_FAVORITES_RECOMMENDER: &str = "local_favorites";
const LOCAL_FAVORITES_LOOKBACK_DAYS: i64 = 180;
const MILLISECONDS_PER_DAY: i64 = 86_400_000;
const RECOMMENDATION_LIMIT: usize = 3;
const SCHEDULE_OVERDUE_RETENTION_DAYS: i64 = 7;
const STRUCTURED_RIDE_TRAINING_FOCUS: &str = "Structured Ride";

#[derive(Debug, Clone, Serialize)]
pub struct SchedulePlacement {
    pub date_local: String,
    pub time_local: Option<String>,
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NextUpItem {
    Scheduled {
        scheduled_workout_id: String,
        placement: SchedulePlacement,
        workout: WorkoutSummary,
    },
    Recommendation {
        recommender: String,
        training_focus: String,
        workout: WorkoutSummary,
    },
}

fn oldest_scheduled_date_local(today_date_local: &str) -> Result<String, AppError> {
    local_date::shift(today_date_local, -SCHEDULE_OVERDUE_RETENTION_DAYS).ok_or_else(|| {
        AppError::new(
            "next_up_date",
            format!("Invalid local date {today_date_local:?}"),
        )
    })
}

pub fn list(state: &State<'_, AppState>) -> Result<Vec<NextUpItem>, AppError> {
    list_at(state, Utc::now())
}

fn list_at(
    state: &State<'_, AppState>,
    now_utc: DateTime<Utc>,
) -> Result<Vec<NextUpItem>, AppError> {
    let ftp_w = state.settings().profile.ftp;
    let favorite_cutoff_unix_ms = now_utc
        .timestamp_millis()
        .saturating_sub(LOCAL_FAVORITES_LOOKBACK_DAYS * MILLISECONDS_PER_DAY);
    let (scheduled, favorites) = {
        let conn = state.db.lock().unwrap();
        let planning_time_zone = provider_connections::get_active_by_provider(&conn, PROVIDER_ID)?
            .map(|connection| connection.time_zone);
        let today_date_local = local_date::today_at(now_utc, planning_time_zone.as_deref());
        let oldest_scheduled_date_local = oldest_scheduled_date_local(&today_date_local)?;
        (
            scheduled_workouts::list_for_next_up(&conn, &oldest_scheduled_date_local)?,
            workout_definitions::list_by_activity_frequency_since(&conn, favorite_cutoff_unix_ms)?,
        )
    };
    assemble(scheduled, favorites, ftp_w)
}

fn assemble(
    scheduled: Vec<scheduled_workouts::ScheduledWorkoutWithDefinitionRow>,
    favorites: Vec<workout_definitions::WorkoutDefinitionRow>,
    ftp_w: u16,
) -> Result<Vec<NextUpItem>, AppError> {
    let scheduled_definition_ids = scheduled
        .iter()
        .map(|row| row.schedule.workout_definition_id.clone())
        .collect::<HashSet<_>>();
    let mut items = Vec::with_capacity(scheduled.len() + RECOMMENDATION_LIMIT);

    for row in scheduled {
        let workout = workout::summary_from_row(row.definition, ftp_w)?;
        items.push(NextUpItem::Scheduled {
            scheduled_workout_id: row.schedule.id,
            placement: SchedulePlacement {
                date_local: row.schedule.scheduled_date_local,
                time_local: row.schedule.scheduled_time_local,
                time_zone: row.schedule.scheduled_time_zone,
            },
            workout,
        });
    }

    for row in favorites
        .into_iter()
        .filter(|row| !scheduled_definition_ids.contains(&row.id))
        .take(RECOMMENDATION_LIMIT)
    {
        let workout = workout::summary_from_row(row, ftp_w)?;
        let training_focus = workout
            .training_focus
            .clone()
            .unwrap_or_else(|| STRUCTURED_RIDE_TRAINING_FOCUS.into());
        items.push(NextUpItem::Recommendation {
            recommender: LOCAL_FAVORITES_RECOMMENDER.into(),
            training_focus,
            workout,
        });
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAGGED_TPW_JSON: &str = r#"{"format":"TPW","version":1,"title":"Tagged","training_focus":"Endurance Base","prescription":{"sport":"cycling","steps":[{"type":"steady","duration_seconds":3600,"power":{"type":"percent_ftp","percent":70}}]}}"#;
    const UNTAGGED_TPW_JSON: &str = r#"{"format":"TPW","version":1,"title":"Untagged","prescription":{"sport":"cycling","steps":[{"type":"steady","duration_seconds":3600,"power":{"type":"percent_ftp","percent":70}}]}}"#;

    fn definition(id: &str, tpw_json: &str) -> workout_definitions::WorkoutDefinitionRow {
        workout_definitions::WorkoutDefinitionRow {
            id: id.into(),
            tpw_json: tpw_json.into(),
            origin: None,
        }
    }

    #[test]
    fn scheduled_items_precede_recommendations_and_suppress_duplicates() {
        let scheduled = vec![scheduled_workouts::ScheduledWorkoutWithDefinitionRow {
            schedule: scheduled_workouts::ScheduledWorkoutRow {
                id: "scheduled-1".into(),
                workout_definition_id: "tagged".into(),
                scheduled_date_local: "2026-09-15".into(),
                scheduled_time_local: Some("07:00:00".into()),
                scheduled_time_zone: Some("Europe/Zurich".into()),
                removed_at_unix_ms: None,
                provider_connection_id: None,
                external_event_id: None,
                external_revision: None,
                last_synced_at_unix_ms: None,
            },
            definition: definition("tagged", TAGGED_TPW_JSON),
        }];
        let favorites = vec![
            definition("tagged", TAGGED_TPW_JSON),
            definition("untagged", UNTAGGED_TPW_JSON),
        ];

        let items = assemble(scheduled, favorites, 250).unwrap();
        assert_eq!(items.len(), 2);
        match &items[0] {
            NextUpItem::Scheduled {
                scheduled_workout_id,
                placement,
                workout,
            } => {
                assert_eq!(scheduled_workout_id, "scheduled-1");
                assert_eq!(placement.time_local.as_deref(), Some("07:00:00"));
                assert_eq!(workout.id, "tagged");
            }
            other => panic!("expected scheduled item, got {other:?}"),
        }
        match &items[1] {
            NextUpItem::Recommendation {
                recommender,
                training_focus,
                workout,
            } => {
                assert_eq!(recommender, LOCAL_FAVORITES_RECOMMENDER);
                assert_eq!(training_focus, STRUCTURED_RIDE_TRAINING_FOCUS);
                assert_eq!(workout.id, "untagged");
            }
            other => panic!("expected recommendation, got {other:?}"),
        }
    }

    #[test]
    fn overdue_window_keeps_seven_calendar_days() {
        assert_eq!(
            oldest_scheduled_date_local("2026-09-21").unwrap(),
            "2026-09-14"
        );
        assert_eq!(
            oldest_scheduled_date_local("2026-01-03").unwrap(),
            "2025-12-27"
        );
        assert_eq!(
            oldest_scheduled_date_local("2026-02-29").unwrap_err().code,
            "next_up_date"
        );
    }
}
