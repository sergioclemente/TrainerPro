import { save } from "@tauri-apps/plugin-dialog";
import { fmtDuration, ipc, revealLabel } from "../ipc";
import { useStore } from "../state";

export default function Summary() {
  const { summary, go, pushToast } = useStore();

  if (!summary) {
    return (
      <div className="screen">
        <p className="empty">No ride summary.</p>
        <button onClick={() => go("history")}>History</button>
      </div>
    );
  }

  async function saveFit() {
    const date = new Date(summary!.started_at).toISOString().slice(0, 10);
    const dest = await save({
      defaultPath: `TrainerPro_${summary!.workout_name.replace(/\W+/g, "_")}_${date}.fit`,
      filters: [{ name: "FIT activity", extensions: ["fit"] }],
    });
    if (!dest) return;
    await ipc.saveFitAs(summary!.ride_id, dest);
    pushToast("info", "FIT file saved");
  }

  const stat = (label: string, value: string) => (
    <div className="stat">
      <span className="stat-value">{value}</span>
      <span className="stat-label">{label}</span>
    </div>
  );

  return (
    <div className="screen">
      <header className="screen-head">
        <h1>{summary.workout_name}</h1>
        <span className="muted">
          {new Date(summary.started_at).toLocaleString()} ·{" "}
          {Math.round(summary.completed_pct)}% completed
        </span>
      </header>

      <div className="stat-row">
        {stat("moving time", fmtDuration(summary.timer_s))}
        {stat("avg power", summary.avg_power != null ? `${summary.avg_power} W` : "–")}
        {stat("NP", summary.np != null ? `${summary.np} W` : "–")}
        {stat("IF", summary.if_ != null ? summary.if_.toFixed(2) : "–")}
        {stat("TSS", summary.tss != null ? String(Math.round(summary.tss)) : "–")}
        {stat("avg HR", summary.avg_hr != null ? `${summary.avg_hr}` : "–")}
        {stat("work", `${summary.kj} kJ`)}
      </div>

      <h2>Laps</h2>
      <table className="table">
        <thead>
          <tr>
            <th>#</th>
            <th>start</th>
            <th>duration</th>
            <th>avg W</th>
            <th>max W</th>
            <th>avg HR</th>
          </tr>
        </thead>
        <tbody>
          {summary.laps.map((l, i) => (
            <tr key={i}>
              <td>{i + 1}</td>
              <td>{fmtDuration(l.start_s)}</td>
              <td>{fmtDuration(l.duration_s)}</td>
              <td>{l.avg_power ?? "–"}</td>
              <td>{l.max_power ?? "–"}</td>
              <td>{l.avg_hr ?? "–"}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <div className="row gap export-row">
        <button className="primary" onClick={saveFit}>
          Save .FIT…
        </button>
        <button onClick={() => ipc.revealFit(summary.ride_id)}>{revealLabel}</button>
        <button onClick={() => ipc.openGarminImport()}>Open Garmin Connect</button>
        <button className="ghost" onClick={() => go("library")}>
          Done
        </button>
      </div>
      <p className="muted footnote">
        Upload to Garmin: drag the saved .FIT file into the Garmin Connect import
        page that opens.
      </p>
    </div>
  );
}
