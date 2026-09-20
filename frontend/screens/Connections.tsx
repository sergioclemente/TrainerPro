import { useEffect, useState } from "react";
import {
  AppError,
  IntervalsConnectionStatus,
  IntervalsSyncReport,
  ipc,
} from "../ipc";
import { useStore } from "../state";
import { dateKeyInZone } from "../date";

function syncSummary(report: IntervalsSyncReport): string {
  if (report.issues.length > 0) {
    return `Intervals.icu sync completed with ${report.issues.length} workout issue(s)`;
  }
  const changed = report.inserted + report.updated + report.removed;
  if (changed === 0) return "Intervals.icu is up to date";
  return `Intervals.icu synced: ${report.inserted} added, ${report.updated} updated, ${report.removed} removed`;
}

function syncTime(unixMs: number | null): string {
  if (unixMs == null) return "Not synchronized yet";
  return `Last synchronized ${new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(unixMs))}`;
}

export default function ConnectionsPanel() {
  const { pushToast, refreshNextUp, refreshWorkouts } = useStore();
  const [status, setStatus] = useState<IntervalsConnectionStatus | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);

  async function reloadStatus() {
    setStatus(await ipc.getIntervalsIcuConnection());
  }

  useEffect(() => {
    void reloadStatus().catch((error) =>
      pushToast("error", (error as AppError).message ?? String(error)),
    );
    // Zustand actions are stable; this panel only needs its initial status.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function synchronize(connection: IntervalsConnectionStatus) {
    const report = await ipc.refreshIntervalsIcu(
      dateKeyInZone(new Date(), connection.time_zone),
    );
    await Promise.all([reloadStatus(), refreshNextUp(), refreshWorkouts()]);
    pushToast(report.issues.length > 0 ? "warn" : "info", syncSummary(report));
  }

  async function connect() {
    if (!apiKey.trim()) return;
    setBusy(true);
    try {
      const connected = await ipc.connectIntervalsIcu(apiKey);
      setApiKey("");
      setStatus(connected);
      await synchronize(connected);
      pushToast("info", `Connected to Intervals.icu${connected.display_name ? ` as ${connected.display_name}` : ""}`);
    } catch (error) {
      await reloadStatus().catch(() => {});
      pushToast("error", (error as AppError).message ?? String(error));
    } finally {
      setBusy(false);
    }
  }

  async function refresh() {
    if (!status?.connected) return;
    setBusy(true);
    try {
      await synchronize(status);
    } catch (error) {
      await reloadStatus().catch(() => {});
      pushToast("error", (error as AppError).message ?? String(error));
    } finally {
      setBusy(false);
    }
  }

  async function disconnect() {
    if (!confirm("Disconnect Intervals.icu? Synced workouts will leave Next Up, but completed Activity history is preserved.")) {
      return;
    }
    setBusy(true);
    try {
      setStatus(await ipc.disconnectIntervalsIcu());
      await Promise.all([refreshNextUp(), refreshWorkouts()]);
      pushToast("info", "Intervals.icu disconnected");
    } catch (error) {
      pushToast("error", (error as AppError).message ?? String(error));
    } finally {
      setBusy(false);
    }
  }

  if (!status) return <p className="muted">Loading connection…</p>;

  return (
    <div className="connection-list">
      <section className="connection-card">
        <div className="connection-head">
          <div>
            <h2>Intervals.icu</h2>
            <p className="muted">Scheduled cycling workouts appear in Next Up and remain available offline.</p>
          </div>
          <span className={`lib-badge ${status.connected ? "on" : "off"}`}>
            {status.connected ? "connected" : "not connected"}
          </span>
        </div>

        {status.connected ? (
          <div className="connection-body">
            <dl className="connection-facts">
              <div>
                <dt>Account</dt>
                <dd>{status.display_name || status.external_account_id}</dd>
              </div>
              <div>
                <dt>Time zone</dt>
                <dd>{status.time_zone}</dd>
              </div>
            </dl>
            <p className="muted footnote">{syncTime(status.last_sync_succeeded_at_unix_ms)}</p>
            {status.last_sync_error && (
              <p className="connection-error">Last sync: {status.last_sync_error}</p>
            )}
            <div className="row gap">
              <button className="primary" onClick={() => void refresh()} disabled={busy}>
                {busy ? "Synchronizing…" : "Sync now"}
              </button>
              <button className="danger" onClick={() => void disconnect()} disabled={busy}>
                Disconnect
              </button>
            </div>
          </div>
        ) : (
          <div className="connection-body form">
            <p className="muted" style={{ margin: 0 }}>
              Create a personal API key in Intervals.icu under Settings → Developer Settings.
              TrainerPro stores it in your system credential manager, not in SQLite.
            </p>
            <label>
              Personal API key
              <input
                type="password"
                value={apiKey}
                autoComplete="off"
                spellCheck={false}
                onChange={(event) => setApiKey(event.target.value)}
              />
            </label>
            <button
              className="primary"
              onClick={() => void connect()}
              disabled={busy || !apiKey.trim()}
            >
              {busy ? "Connecting…" : "Connect and sync"}
            </button>
          </div>
        )}
      </section>
    </div>
  );
}
