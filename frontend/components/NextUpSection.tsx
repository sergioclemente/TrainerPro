import { useEffect, useState } from "react";
import { AppError, NextUpItem, SchedulePlacement, fmtDuration, ipc } from "../ipc";
import WorkoutGraph from "./WorkoutGraph";
import { useStore } from "../state";
import { dateKeyInZone } from "../date";

let automaticProviderSyncStarted = false;

function shiftDateKey(key: string, days: number): string {
  const [year, month, day] = key.split("-").map(Number);
  if (!year || !month || !day) return key;
  const shifted = new Date(Date.UTC(year, month - 1, day + days));
  return shifted.toISOString().slice(0, 10);
}

function absoluteDateLabel(key: string, now: Date): string {
  const [year, month, day] = key.split("-").map(Number);
  if (!year || !month || !day) return key;
  return new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    year: year === now.getFullYear() ? undefined : "numeric",
    timeZone: "UTC",
  }).format(new Date(Date.UTC(year, month - 1, day)));
}

function scheduleLabel(placement: SchedulePlacement, now = new Date()): string {
  const today = dateKeyInZone(now, placement.time_zone);
  const date =
    placement.date_local === today
      ? "Today"
      : placement.date_local === shiftDateKey(today, 1)
        ? "Tomorrow"
        : placement.date_local === shiftDateKey(today, -1)
          ? "Yesterday"
          : absoluteDateLabel(placement.date_local, now);
  const time = placement.time_local?.slice(0, 5);
  return time ? `${date} · ${time}` : date;
}

function itemKey(item: NextUpItem): string {
  return item.kind === "scheduled"
    ? `scheduled:${item.scheduled_workout_id}`
    : `recommendation:${item.recommender}:${item.workout.id}`;
}

export default function NextUpSection() {
  const {
    nextUp,
    nextUpStatus,
    nextUpError,
    refreshNextUp,
    refreshWorkouts,
    pushToast,
  } = useStore();
  const [syncing, setSyncing] = useState(false);

  async function refreshProvider(showSuccess: boolean) {
    setSyncing(true);
    try {
      const connection = await ipc.getIntervalsIcuConnection();
      if (!connection.connected) {
        await refreshNextUp();
        return;
      }
      const report = await ipc.refreshIntervalsIcu(
        dateKeyInZone(new Date(), connection.time_zone),
      );
      await Promise.all([refreshNextUp(), refreshWorkouts()]);
      if (showSuccess) {
        pushToast(
          report.issues.length > 0 ? "warn" : "info",
          report.issues.length > 0
            ? `Synced with ${report.issues.length} workout issue(s)`
            : "Intervals.icu is up to date",
        );
      }
    } catch (error) {
      const message = (error as AppError).message ?? String(error);
      pushToast("warn", `Intervals.icu sync failed: ${message}`);
    } finally {
      setSyncing(false);
    }
  }

  useEffect(() => {
    void refreshNextUp();
    if (!automaticProviderSyncStarted) {
      automaticProviderSyncStarted = true;
      void refreshProvider(false);
    }
    // The automatic attempt is intentionally once per app run; manual refresh
    // remains available and cached Next Up renders before the network returns.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshNextUp]);

  async function openDetail(item: NextUpItem) {
    try {
      const detail = await ipc.getWorkoutDetail(item.workout.id);
      useStore.setState({
        detail: {
          source: "library",
          id: detail.summary.id,
          scheduledWorkoutId:
            item.kind === "scheduled" ? item.scheduled_workout_id : undefined,
          name: detail.summary.name,
          description: detail.summary.description,
          duration_s: detail.summary.duration_s,
          est_if: detail.summary.est_if,
          est_tss: detail.summary.est_tss,
          tags: detail.summary.training_focus ?? "",
          origin: detail.summary.origin,
          graph: detail.summary.graph,
          segments: detail.segments,
        },
        screen: "workout",
      });
    } catch (e) {
      const err = e as AppError;
      pushToast("error", err.message ?? String(e));
    }
  }

  return (
    <section className="next-up-section">
      <header className="next-up-head">
        <div>
          <h2>Next Up</h2>
          <p className="muted next-up-subtitle">Choose what you want to ride.</p>
        </div>
        <button className="ghost" onClick={() => void refreshProvider(true)} disabled={syncing}>
          {syncing ? "Syncing…" : "Refresh"}
        </button>
      </header>

      {nextUpStatus === "loading" && nextUp.length === 0 && (
        <p className="empty">Loading your next workouts…</p>
      )}

      {nextUpStatus === "error" && (
        <div className="next-up-error">
          <span>Next Up could not be loaded: {nextUpError}</span>
          <button onClick={() => void refreshNextUp()}>Retry</button>
        </div>
      )}

      {nextUpStatus === "ready" && nextUp.length === 0 && (
        <div className="next-up-empty">
          <p className="muted">
            Nothing lined up yet. Ride something from the Library and recent favorites will
            appear here.
          </p>
        </div>
      )}

      {nextUp.length > 0 && (
        <div className="next-up-carousel">
          {nextUp.map((item) => {
            const workout = item.workout;
            const focus =
              item.kind === "recommendation" ? item.training_focus : workout.training_focus;
            return (
              <button
                key={itemKey(item)}
                className={`next-up-card ${item.kind}`}
                onClick={() => void openDetail(item)}
              >
                <div className="next-up-thumb">
                  <WorkoutGraph graph={workout.graph} durationS={workout.duration_s} height={76} />
                </div>
                <div className="next-up-info">
                  <div className="next-up-labels">
                    <span className="next-up-kind">
                      {item.kind === "scheduled"
                        ? scheduleLabel(item.placement)
                        : "Recommended"}
                    </span>
                    {focus && <span className="next-up-focus">{focus}</span>}
                    {workout.origin && (
                      <span className="next-up-source">{workout.origin}</span>
                    )}
                  </div>
                  <strong className="next-up-title">{workout.name}</strong>
                  <span className="muted next-up-meta">
                    {fmtDuration(workout.duration_s)} · IF {workout.est_if.toFixed(2)} · TSS{" "}
                    {Math.round(workout.est_tss)}
                  </span>
                  {workout.description && (
                    <span className="muted next-up-description">{workout.description}</span>
                  )}
                </div>
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
}
