import { useState } from "react";
import { AppError, SourceConfig, ipc } from "../ipc";
import { useStore } from "../state";
import { CONFIGURABLE_SOURCES, WorkoutSourceDef } from "../sources";

// Workout-library providers: a master-detail list. Pick a provider on the left,
// toggle it and edit its config on the right. The form is generated from the
// provider's descriptor `fields`, so every provider — and trainer-coach's own
// sources later — renders through this one component.

/** A provider's stored config, or the descriptor default if never saved. */
function configOf(
  settings: { sources: Record<string, SourceConfig> } | null,
  def: WorkoutSourceDef,
): SourceConfig {
  const stored = settings?.sources[def.id];
  if (stored) return stored;
  const values: Record<string, string> = {};
  for (const f of def.fields) if (f.placeholder) values[f.key] = "";
  return { enabled: false, values };
}

// Embeddable panel — rendered as the "Libraries" tab inside Settings (it used to
// be its own screen). No screen/header chrome of its own.
export default function LibrariesPanel() {
  const { settings, refreshSettings, pushToast } = useStore();
  const [selected, setSelected] = useState(CONFIGURABLE_SOURCES[0]?.id ?? "");
  // Field edits are held locally until Save/Test, keyed by field.
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);

  if (!settings) return <p className="muted">Loading…</p>;

  const def = CONFIGURABLE_SOURCES.find((s) => s.id === selected);
  const cfg = def ? configOf(settings, def) : null;

  function valueOf(key: string): string {
    return draft[key] ?? cfg!.values[key] ?? "";
  }

  function draftValues(): Record<string, string> {
    const out: Record<string, string> = { ...cfg!.values };
    for (const f of def!.fields) out[f.key] = valueOf(f.key).trim();
    return out;
  }

  async function persist(next: SourceConfig) {
    await ipc.updateSettings({
      ...settings!,
      sources: { ...settings!.sources, [def!.id]: next },
    });
    await refreshSettings();
  }

  async function toggle(enabled: boolean) {
    // A source with config fields can't be enabled until it's been tested &
    // saved; the toggle only turns things off, or on for field-less sources.
    if (enabled && def!.testable) {
      await save(true);
      return;
    }
    try {
      await persist({ enabled, values: draftValues() });
      setDraft({});
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  async function save(enable: boolean) {
    setBusy(true);
    try {
      if (def!.testable) {
        const r = await ipc.sourceTest(def!.id, draftValues());
        pushToast("info", `${def!.label} connected — ${r.detail}`);
      }
      await persist({ enabled: enable, values: draftValues() });
      setDraft({});
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  function select(id: string) {
    setSelected(id);
    setDraft({});
  }

  return (
    <>
      <p className="muted footnote" style={{ marginTop: 0 }}>
        Enable the workout libraries you want as tabs in Workouts. Disabled
        libraries are hidden and never fetched.
      </p>

      <div className="libraries">
        <ul className="lib-list">
          {CONFIGURABLE_SOURCES.map((s) => {
            const on = settings.sources[s.id]?.enabled ?? false;
            return (
              <li key={s.id}>
                <button
                  className={`lib-row ${selected === s.id ? "active" : ""}`}
                  onClick={() => select(s.id)}
                >
                  <span className="lib-name">{s.label}</span>
                  <span className={`lib-badge ${on ? "on" : "off"}`}>
                    {on ? "on" : "off"}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>

        {def && cfg && (
          <div className="lib-detail form">
            <div className="lib-detail-head">
              <h2>{def.label}</h2>
              <label className="row gap">
                <input
                  type="checkbox"
                  checked={cfg.enabled}
                  onChange={(e) => toggle(e.target.checked)}
                  disabled={busy}
                />
                Enabled
              </label>
            </div>
            {def.description && (
              <p className="muted" style={{ marginTop: 0 }}>
                {def.description}
              </p>
            )}

            {def.fields.map((f) => (
              <label key={f.key}>
                {f.label}
                <input
                  type={f.kind === "password" ? "password" : "text"}
                  value={valueOf(f.key)}
                  placeholder={f.placeholder}
                  onChange={(e) => setDraft((d) => ({ ...d, [f.key]: e.target.value }))}
                />
              </label>
            ))}

            {def.fields.length > 0 ? (
              <div className="row gap">
                <button className="primary" onClick={() => save(true)} disabled={busy}>
                  {busy ? "Testing…" : def.testable ? "Test & save" : "Save"}
                </button>
                {cfg.enabled && (
                  <button onClick={() => toggle(false)} disabled={busy}>
                    Disable
                  </button>
                )}
              </div>
            ) : (
              <p className="muted footnote" style={{ marginTop: 0 }}>
                {cfg.enabled
                  ? `Enabled — the ${def.label} tab is live in Workouts.`
                  : `Disabled. Enable to show the ${def.label} tab in Workouts.`}
              </p>
            )}
          </div>
        )}
      </div>
    </>
  );
}
