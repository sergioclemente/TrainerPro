import { AppError, fmtDuration, ipc, revealLabel } from "../ipc";
import { useStore } from "../state";

export default function History() {
  const { rides, refreshRides, pushToast } = useStore();

  async function del(id: string, name: string, date: string) {
    if (
      !confirm(
        `Delete the ride “${name}” from ${date}?\n\n` +
          `Removes TrainerPro's .fit and journal files for this ride. ` +
          `Any copy you exported elsewhere is not affected.`,
      )
    )
      return;
    try {
      await ipc.deleteRide(id);
      await refreshRides();
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  return (
    <div className="screen">
      <header className="screen-head">
        <h1>History</h1>
      </header>
      {rides.length === 0 && <p className="empty">No rides yet.</p>}
      {rides.length > 0 && (
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
            {rides.map((r) => (
              <tr key={r.id}>
                <td>{new Date(r.started_at).toLocaleDateString()}</td>
                <td>
                  {r.workout_name}
                  {r.completed_pct < 99 && (
                    <span className="muted"> ({Math.round(r.completed_pct)}%)</span>
                  )}
                </td>
                <td>{fmtDuration(r.timer_s)}</td>
                <td>{r.avg_power ?? "–"}</td>
                <td>{r.np ?? "–"}</td>
                <td>{r.tss != null ? Math.round(r.tss) : "–"}</td>
                <td>{r.avg_hr ?? "–"}</td>
                <td className="row gap">
                  <button className="ghost" onClick={() => ipc.revealFit(r.id)}>
                    {revealLabel}
                  </button>
                  <button
                    className="ghost danger"
                    title="Delete this ride"
                    onClick={() =>
                      del(
                        r.id,
                        r.workout_name,
                        new Date(r.started_at).toLocaleDateString(),
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
