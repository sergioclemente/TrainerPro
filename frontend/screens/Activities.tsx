import { AppError, fmtDuration, ipc, revealLabel } from "../ipc";
import { useStore } from "../state";

export default function Activities() {
  const { activities, refreshActivities, pushToast } = useStore();

  async function del(id: string, name: string, date: string) {
    if (
      !confirm(
        `Delete the activity “${name}” from ${date}?\n\n` +
          `Removes TrainerPro's .fit and journal files for this activity. ` +
          `Any copy you exported elsewhere is not affected.`,
      )
    )
      return;
    try {
      await ipc.deleteActivity(id);
      await refreshActivities();
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  return (
    <div className="screen">
      <header className="screen-head">
        <h1>Activities</h1>
      </header>
      {activities.length === 0 && <p className="empty">No activities yet.</p>}
      {activities.length > 0 && (
        <table className="table">
          <thead>
            <tr>
              <th>date</th>
              <th>workout</th>
              <th>time</th>
              <th>avg W</th>
              <th>NP</th>
              <th>TSS</th>
              <th>HR</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {activities.map((r) => (
              <tr key={r.id}>
                <td>{new Date(r.started_at_unix_ms).toLocaleDateString()}</td>
                <td>
                  {r.workout_name}
                  {r.completed_pct < 99 && (
                    <span className="muted"> ({Math.round(r.completed_pct)}%)</span>
                  )}
                </td>
                <td>{fmtDuration(r.timer_s)}</td>
                <td>{r.average_power_w ?? "–"}</td>
                <td>{r.normalized_power_w ?? "–"}</td>
                <td>
                  {r.training_stress_score != null
                    ? Math.round(r.training_stress_score)
                    : "–"}
                </td>
                <td>{r.average_heart_rate_bpm ?? "–"}</td>
                <td className="row gap">
                  <button className="ghost" onClick={() => ipc.revealFit(r.id)}>
                    {revealLabel}
                  </button>
                  <button
                    className="ghost danger"
                    title="Delete this activity"
                    onClick={() =>
                      del(
                        r.id,
                        r.workout_name,
                        new Date(r.started_at_unix_ms).toLocaleDateString(),
                      )
                    }
                  >
                    ✕
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
