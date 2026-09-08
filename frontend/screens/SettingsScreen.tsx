import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { useStore } from "../state";
import LibrariesPanel from "./Libraries";

// Settings is now a tabbed screen. "Libraries" (workout sources) used to be its
// own top-level screen — it's mostly configuration, so it lives here now as a
// tab alongside Basic info and Export.
type Tab = "basic" | "export" | "libraries";

export default function SettingsScreen() {
  const { settings, refreshSettings, pushToast, settingsTab } = useStore();
  const [tab, setTab] = useState<Tab>((settingsTab as Tab) || "basic");
  const [ftp, setFtp] = useState<string | null>(null);
  const [weight, setWeight] = useState<string | null>(null);

  if (!settings) return <div className="screen">Loading…</div>;

  const ftpVal = ftp ?? String(settings.profile.ftp);
  const weightVal = weight ?? String(settings.profile.weight_kg);

  async function saveAll() {
    const next = {
      ...settings!,
      profile: {
        ...settings!.profile,
        ftp: Math.max(50, Math.min(600, parseInt(ftpVal, 10) || settings!.profile.ftp)),
        weight_kg:
          Math.max(30, Math.min(200, parseFloat(weightVal) || settings!.profile.weight_kg)),
      },
    };
    await ipc.updateSettings(next);
    await refreshSettings();
    setFtp(null);
    setWeight(null);
    pushToast("info", "Settings saved");
  }

  async function toggleDistance() {
    await ipc.updateSettings({ ...settings!, record_distance: !settings!.record_distance });
    await refreshSettings();
  }

  async function chooseExportDir() {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") {
      await ipc.updateSettings({ ...settings!, export_dir: dir });
      await refreshSettings();
    }
  }

  async function clearExportDir() {
    await ipc.updateSettings({ ...settings!, export_dir: null });
    await refreshSettings();
  }

  const TABS: [Tab, string][] = [
    ["basic", "Basic info"],
    ["export", "Export"],
    ["libraries", "Libraries"],
  ];

  return (
    <div className="screen">
      <header className="screen-head">
        <h1>Settings</h1>
        <div className="tabs">
          {TABS.map(([id, label]) => (
            <button
              key={id}
              className={`tab ${tab === id ? "active" : ""}`}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </div>
      </header>

      {tab === "basic" && (
        <>
          <div className="form">
            <label>
              FTP (watts)
              <input value={ftpVal} onChange={(e) => setFtp(e.target.value)} inputMode="numeric" />
            </label>
            <label>
              Weight (kg)
              <input
                value={weightVal}
                onChange={(e) => setWeight(e.target.value)}
                inputMode="decimal"
              />
            </label>
            <label className="row gap">
              <input type="checkbox" checked={settings.record_distance} onChange={toggleDistance} />
              Record virtual speed/distance in FIT files
            </label>
            <button className="primary" onClick={saveAll}>
              Save
            </button>
          </div>
          <p className="muted footnote">
            %FTP workout targets resolve against this FTP when a ride starts.
          </p>
        </>
      )}

      {tab === "export" && (
        <>
          <div className="form">
            <label>
              FIT export folder
              <span className="muted export-path">
                {settings.export_dir ?? "App data folder only"}
              </span>
            </label>
            <div className="row gap">
              <button onClick={chooseExportDir}>Choose folder…</button>
              {settings.export_dir && <button onClick={clearExportDir}>Reset</button>}
            </div>
            <p className="muted footnote" style={{ marginTop: 0 }}>
              Finished rides always save into the app's data folder; when an export
              folder is set, a copy with a friendly name is written there too.
            </p>
          </div>
        </>
      )}

      {tab === "libraries" && <LibrariesPanel />}
    </div>
  );
}
