import { useEffect } from "react";
import { ipc } from "./ipc";
import { useStore } from "./state";
import Library from "./screens/Library";
import Devices from "./screens/Devices";
import Player from "./screens/Player";
import Summary from "./screens/Summary";
import History from "./screens/History";
import SettingsScreen from "./screens/SettingsScreen";
import WorkoutDetail from "./screens/WorkoutDetail";
import Builder from "./screens/Builder";

function copyText(text: string) {
  if (navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(text).catch(() => copyFallback(text));
  } else {
    copyFallback(text);
  }
}

function copyFallback(text: string) {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  document.execCommand("copy");
  ta.remove();
}

const NAV = [
  ["library", "Workouts"],
  ["builder", "Build"],
  ["devices", "Devices"],
  ["history", "History"],
  ["settings", "Settings"],
] as const;

export default function App() {
  const {
    screen,
    go,
    player,
    toasts,
    deviceStatus,
    refreshWorkouts,
    refreshDevices,
    refreshRides,
    refreshSettings,
  } = useStore();

  useEffect(() => {
    void refreshWorkouts();
    void refreshDevices();
    void refreshRides();
    void refreshSettings();
    // Rehydrate a ride the backend still has loaded (e.g. after a UI
    // reload), so the sidebar shows it and the Player screen can resume.
    void ipc.getPlayerState().then((ps) => {
      if (ps) useStore.setState({ player: ps });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const riding = player !== null && screen === "player";

  return (
    <div className={`app ${riding ? "app-riding" : ""}`}>
      {!riding && (
        <nav className="rail">
          <div className="brand">TrainerPro</div>
          {NAV.map(([key, label]) => (
            <button
              key={key}
              className={`rail-item ${screen === key ? "active" : ""}`}
              onClick={() => go(key)}
            >
              {label}
            </button>
          ))}
          {player && (
            <button className="rail-item ride-live" onClick={() => go("player")}>
              ● Ride in progress
            </button>
          )}
          <div className="rail-status">
            {(["trainer", "hrm"] as const).map((r) => {
              const s = deviceStatus[r]?.status;
              const cls =
                s === "connected" ? "ok" : s === "reconnecting" ? "warn" : "muted";
              return (
                <span key={r} className={`status ${cls}`}>
                  {r === "trainer" ? "🚴" : "❤"} {s ?? "—"}
                </span>
              );
            })}
          </div>
        </nav>
      )}
      <main className="content">
        {screen === "library" && <Library />}
        {screen === "devices" && <Devices />}
        {screen === "player" && <Player />}
        {screen === "summary" && <Summary />}
        {screen === "history" && <History />}
        {screen === "settings" && <SettingsScreen />}
        {screen === "workout" && <WorkoutDetail />}
        {screen === "builder" && <Builder />}
      </main>
      <div className="toasts">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.level}`}>
            <span className="toast-msg">{t.message}</span>
            <span className="toast-actions">
              <button
                className="toast-btn"
                title="Copy error text"
                onClick={() => copyText(t.message)}
              >
                ⧉
              </button>
              <button
                className="toast-btn"
                title="Dismiss"
                onClick={() => useStore.getState().dismissToast(t.id)}
              >
                ✕
              </button>
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
