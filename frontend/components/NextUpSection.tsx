import { useEffect } from "react";
import { AppError, NextUpItem, SchedulePlacement, fmtDuration, ipc } from "../ipc";
import WorkoutGraph from "./WorkoutGraph";
import { useStore } from "../state";

function localDateKey(now: Date): string {
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function dateKeyInZone(now: Date, timeZone: string | null): string {
  if (!timeZone) return localDateKey(now);
  try {
    const parts = new Intl.DateTimeFormat("en-CA", {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    }).formatToParts(now);
    const part = (type: Intl.DateTimeFormatPartTypes) =>
      parts.find((candidate) => candidate.type === type)?.value;
    const year = part("year");
    const month = part("month");
    const day = part("day");
    if (year && month && day) return `${year}-${month}-${day}`;
  } catch {
    // An invalid provider time zone should not make Next Up unusable.
  }
  return localDateKey(now);
}

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
  const { nextUp, nextUpStatus, nextUpError, refreshNextUp, pushToast } = useStore();

  useEffect(() => {
    void refreshNextUp();
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
        <button className="ghost" onClick={() => void refreshNextUp()}>
          Refresh
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
