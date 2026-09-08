import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { AppError, fmtDuration, ipc } from "../ipc";
import { useStore } from "../state";
import WorkoutGraph from "../components/WorkoutGraph";
import { SOURCES } from "../sources";

export default function Library() {
  const { workouts, refreshWorkouts, pushToast, settings } = useStore();
  const [tab, setTab] = useState<string>("local");

  // Tabs = the built-in local grid + every enabled provider. A source disabled
  // (or never configured) in the Libraries screen contributes no tab.
  const tabs = SOURCES.filter((s) => s.builtin || settings?.sources[s.id]?.enabled);
  // If the active tab's provider was just disabled, fall back to local.
  const activeTab = tabs.some((s) => s.id === tab) ? tab : "local";

  async function importFiles() {
    const picked = await open({
      multiple: true,
      filters: [{ name: "Workouts", extensions: ["zwo", "erg", "mrc"] }],
    });
    if (!picked) return;
    for (const path of Array.isArray(picked) ? picked : [picked]) {
      try {
        const r = await ipc.importWorkout(path as string);
        if (r.already_existed) {
          pushToast("info", `${r.summary.name}: already in library`);
        } else {
          pushToast("info", `Imported ${r.summary.name}`);
          r.warnings.slice(0, 3).forEach((w) => pushToast("warn", w));
        }
      } catch (e) {
        pushToast("error", (e as AppError).message ?? String(e));
      }
    }
    await refreshWorkouts();
  }

  async function openDetail(id: string) {
    try {
      const d = await ipc.getWorkoutDetail(id);
      useStore.setState({
        detail: {
          source: "library",
          id: d.summary.id,
          name: d.summary.name,
          description: d.summary.description,
          duration_s: d.summary.duration_s,
          est_if: d.summary.est_if,
          est_tss: d.summary.est_tss,
          tags: "",
          origin: d.summary.origin,
          graph: d.summary.graph,
          segments: d.segments,
        },
        screen: "workout",
      });
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  async function del(id: string, name: string) {
    if (!confirm(`Delete workout “${name}”?`)) return;
    await ipc.deleteWorkout(id);
    await refreshWorkouts();
  }

  return (
    <div className="screen">
      <header className="screen-head">
        <h1>Workouts</h1>
        <div className="tabs">
          {tabs.map((s) => (
            <button
              key={s.id}
              className={`tab ${activeTab === s.id ? "active" : ""}`}
              onClick={() => setTab(s.id)}
            >
              {s.label}
            </button>
          ))}
        </div>
        {activeTab === "local" && (
          <button className="primary" onClick={importFiles}>
            Import…
          </button>
        )}
      </header>
      {tabs.map(
        (s) =>
          s.Component &&
          activeTab === s.id && (
            <s.Component key={s.id} />
          ),
      )}
      {activeTab === "local" && workouts.length === 0 && (
        <p className="empty">
          No workouts yet. Import a .zwo, .erg, or .mrc file to get started.
        </p>
      )}
      {activeTab === "local" && (
      <div className="card-grid">
        {workouts.map((w) => (
          <div key={w.id} className="card workout-card" onClick={() => openDetail(w.id)}>
            <div className="thumb">
              <WorkoutGraph graph={w.graph} durationS={w.duration_s} height={80} />
            </div>
            <div className="card-body">
              <strong>
                {w.name}
                {w.origin === "planner" && <span className="badge">planner</span>}
                {w.origin === "whatsonzwift" && <span className="badge">zwift</span>}
              </strong>
              <span className="muted">
                {fmtDuration(w.duration_s)} · IF {w.est_if.toFixed(2)} · TSS{" "}
                {Math.round(w.est_tss)}
              </span>
            </div>
            <button
              className="ghost danger"
              onClick={(e) => {
                e.stopPropagation();
                void del(w.id, w.name);
              }}
            >
              ✕
            </button>
          </div>
        ))}
      </div>
      )}
    </div>
  );
}
