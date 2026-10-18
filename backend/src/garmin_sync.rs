//! Complete-window Garmin reads and transactional provider-owned schedule cache.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde_json::Value;
use tp_integrations::garmin::{
    workouts::WorkoutSchedule, GarminClient, GarminError, Tokens, PROVIDER_ID,
};

use crate::{
    app_error::AppError,
    commands::providers::{require_active, PlanSyncReport, ProviderCapability},
    database::{provider_connections, scheduled_workouts},
    garmin_workouts::to_workout_spec,
};

/// All referenced definitions must be read successfully before reconciliation.
/// A repeated template is fetched once, but every scheduled occurrence survives.
pub(crate) async fn fetch_calendar_window(
    client: &GarminClient,
    tokens: &Tokens,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<(WorkoutSchedule, Value)>, GarminError> {
    let calendar = client
        .list_workout_schedules_by_date(tokens, start_date, end_date)
        .await?;
    let mut definitions = HashMap::new();
    let mut events = Vec::with_capacity(calendar.len());
    for event in calendar {
        let schedule = WorkoutSchedule::from_payload(&event)?;
        if !definitions.contains_key(&schedule.workout_id) {
            let detail = client
                .get_workout(tokens, &schedule.workout_id)
                .await?
                .ok_or(GarminError::Protocol)?;
            definitions.insert(schedule.workout_id.clone(), detail);
        }
        let detail = definitions[&schedule.workout_id].clone();
        events.push((schedule, detail));
    }
    Ok(events)
}

pub(crate) fn persist_calendar_window(
    conn: &mut Connection,
    connection_id: &str,
    start_date: &str,
    end_date: &str,
    events: &[(WorkoutSchedule, Value)],
    synced_at_unix_ms: i64,
) -> Result<PlanSyncReport, AppError> {
    let is_iso_date = |value: &str| {
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .is_ok_and(|date| date.to_string() == value)
    };
    if !is_iso_date(start_date) || !is_iso_date(end_date) || start_date > end_date {
        return Err(AppError::new(
            "garmin_date_range",
            "Invalid Garmin schedule window.",
        ));
    }
    let connection = require_active(conn, connection_id, Some(ProviderCapability::Planning))?;
    if connection.provider != PROVIDER_ID {
        return Err(AppError::new(
            "garmin_connection",
            "This connection is not a Garmin account.",
        ));
    }
    let mut report = PlanSyncReport {
        inserted: 0,
        updated: 0,
        unchanged: 0,
        removed: 0,
        unsupported: 0,
        issues: vec![],
        oldest_date_local: start_date.into(),
        newest_date_local: end_date.into(),
    };
    let mut present = HashSet::new();
    let mut prepared = Vec::new();
    for (event, workout) in events {
        if event.date_local.as_str() < start_date
            || event.date_local.as_str() > end_date
            || !is_iso_date(&event.date_local)
            || !present.insert(event.id.clone())
            || tp_integrations::garmin::workouts::workout_id(workout)
                .ok()
                .as_ref()
                != Some(&event.workout_id)
        {
            return Err(AppError::new(
                "garmin_response",
                "Garmin returned an inconsistent schedule window.",
            ));
        }
        match to_workout_spec(workout).and_then(|spec| spec.to_json_pretty().map_err(Into::into)) {
            Ok(json) => prepared.push((event, json)),
            Err(error) => {
                report.unsupported += 1;
                report
                    .issues
                    .push(format!("Schedule {}: {error}", event.id));
            }
        }
    }
    let transaction = conn.transaction()?;
    for (event, json) in prepared {
        let schedule_id = uuid::Uuid::new_v4().to_string();
        let definition_id = uuid::Uuid::new_v4().to_string();
        match scheduled_workouts::upsert_synced(
            &transaction,
            &scheduled_workouts::SyncedScheduledWorkout {
                schedule_id_for_insert: &schedule_id,
                definition_id_for_insert: &definition_id,
                provider_connection_id: connection_id,
                external_event_id: &event.id,
                external_event_id_i64: event.id.parse().ok(),
                definition_origin: PROVIDER_ID,
                external_revision: None,
                tpw_json: &json,
                scheduled_date_local: &event.date_local,
                scheduled_time_local: None,
                scheduled_time_zone: None,
                synced_at_unix_ms,
            },
        )? {
            scheduled_workouts::SyncedScheduleChange::Inserted => report.inserted += 1,
            scheduled_workouts::SyncedScheduleChange::Updated => report.updated += 1,
            scheduled_workouts::SyncedScheduleChange::Unchanged => report.unchanged += 1,
        }
    }
    report.removed = scheduled_workouts::remove_missing_in_window(
        &transaction,
        connection_id,
        start_date,
        end_date,
        &present,
        synced_at_unix_ms,
    )?;
    if report.issues.is_empty() {
        provider_connections::record_sync_success(&transaction, connection_id, synced_at_unix_ms)?;
    } else {
        provider_connections::record_sync_failure(
            &transaction,
            connection_id,
            &format!(
                "{} Garmin workout(s) could not be synchronized. {}",
                report.issues.len(),
                report.issues[0]
            ),
            synced_at_unix_ms,
        )?;
    }
    transaction.commit()?;
    Ok(report)
}

#[cfg(test)]
mod tests;
