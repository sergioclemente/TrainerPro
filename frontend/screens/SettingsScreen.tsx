import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { useStore } from "../state";
import LibrariesPanel from "./Libraries";
import ConnectionsPanel from "./Connections";
import binaryDistributionTerms from "../../BINARY_DISTRIBUTION_TERMS.md?raw";
import thirdPartyNotices from "../../NOTICE.txt?raw";
import gemmaProhibitedUsePolicy from "../../legal/GEMMA_PROHIBITED_USE_POLICY_2024-02-21.txt?raw";
import gemmaTerms from "../../legal/GEMMA_TERMS_2026-04-01.txt?raw";

// Settings is now a tabbed screen. "Libraries" (workout sources) used to be its
// own top-level screen — it's mostly configuration, so it lives here now as a
// tab alongside Basic info and Export.
type Tab = "basic" | "export" | "libraries" | "connections" | "about";

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

  async function toggleVoice() {
    await ipc.updateSettings({ ...settings!, voice_enabled: !settings!.voice_enabled });
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
    ["connections", "Connections"],
    ["about", "About"],
  ];

  return (
    <div className="screen">
      <header className="screen-head">
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
            <div className="setting-option">
              <label className="row gap">
                <input type="checkbox" checked={settings.voice_enabled} onChange={toggleVoice} />
                Enable voice commands
              </label>
              <span className="muted setting-description">
                Processed on this device. Audio and transcripts are not stored or sent.
              </span>
            </div>
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
              Completed activities always save into the app's data folder; when an export
              folder is set, a copy with a friendly name is written there too.
            </p>
          </div>
        </>
      )}

      {tab === "libraries" && <LibrariesPanel />}
      {tab === "connections" && <ConnectionsPanel />}

      {tab === "about" && (
        <div className="about-settings">
          <section>
            <h2>Privacy</h2>
            <p>
              Voice commands are processed entirely on this device. TrainerPro does not
              store or send microphone audio or transcripts.
            </p>
          </section>
          <section>
            <h2>License</h2>
            <p>
              TrainerPro-authored source code is provided under the MIT License.
              Packaged builds containing EmbeddingGemma are also subject to the
              model-specific distribution terms below.
            </p>
            <details className="legal-disclosure">
              <summary>TrainerPro Binary Distribution Terms</summary>
              <pre className="legal-document">{binaryDistributionTerms}</pre>
            </details>
            <details className="legal-disclosure">
              <summary>Gemma Terms of Use — April 1, 2026</summary>
              <pre className="legal-document">{gemmaTerms}</pre>
            </details>
            <details className="legal-disclosure">
              <summary>Gemma Prohibited Use Policy — February 21, 2024</summary>
              <pre className="legal-document">{gemmaProhibitedUsePolicy}</pre>
            </details>
          </section>
          <section>
            <h2>Third-party notices</h2>
            <p className="muted">
              Voice models and their local runtime include the following notices and
              license references.
            </p>
            <pre className="legal-document">{thirdPartyNotices}</pre>
          </section>
        </div>
      )}
    </div>
  );
}
