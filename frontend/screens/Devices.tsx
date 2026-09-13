import { AppError, Role, ipc } from "../ipc";
import { useStore } from "../state";

const ROLE_LABEL: Record<Role, string> = { trainer: "Smart Trainer", hrm: "Heart Rate" };

export default function Devices() {
  const {
    devices,
    scanResults,
    scanning,
    deviceStatus,
    deviceMeasurement,
    pushToast,
    refreshDevices,
  } = useStore();

  async function scan() {
    useStore.setState({ scanning: true, scanResults: [] });
    try {
      await ipc.startScan();
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
      useStore.setState({ scanning: false });
    }
  }

  async function connect(role: Role, platformId: string, name: string) {
    try {
      await ipc.connectDevice(role, platformId, name);
      useStore.setState({ scanning: false, scanResults: [] });
      await refreshDevices();
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  return (
    <div className="screen">
      <header className="screen-head">
        <h1>Devices</h1>
        <button onClick={scan} disabled={scanning}>
          {scanning ? "Scanning…" : "Scan for devices"}
        </button>
      </header>
      <div className="device-slots">
        {(["trainer", "hrm"] as Role[]).map((role) => {
          const slot = devices.find((d) => d.role === role);
          const status = deviceStatus[role];
          const connected = status ? status.status === "connected" : (slot?.connected ?? false);
          const connecting = status?.status === "connecting";
          const reconnecting = status?.status === "reconnecting";
          const active = connecting || reconnecting;
          const reading = deviceMeasurement[role];
          const live =
            connected && reading
              ? role === "trainer"
                ? reading.power_w != null
                  ? `${reading.power_w} W${reading.cadence_rpm != null ? ` · ${reading.cadence_rpm} rpm` : ""}`
                  : null
                : reading.heart_rate_bpm != null
                  ? `${reading.heart_rate_bpm} bpm`
                  : null
              : null;
          return (
            <div key={role} className="card device-card">
              <div className="card-body">
                <strong>{ROLE_LABEL[role]}</strong>
                {connecting ? (
                  <span className="status warn">● connecting…</span>
                ) : connected ? (
                  <span className="status ok">
                    ● {status?.name ?? slot?.saved_name ?? "connected"}
                    {live && <span className="live-reading"> {live}</span>}
                  </span>
                ) : reconnecting ? (
                  <span className="status warn">
                    ● reconnecting to {status.name ?? slot?.saved_name ?? "saved device"}…
                  </span>
                ) : slot?.saved_name ? (
                  <span className="status muted">○ {slot.saved_name} (saved)</span>
                ) : (
                  <span className="status muted">○ not paired</span>
                )}
              </div>
              <div className="row gap">
                {!connected && !active && slot?.saved_platform_id && (
                  <button
                    className="primary"
                    disabled={connecting}
                    onClick={() =>
                      connect(role, slot.saved_platform_id!, slot.saved_name ?? "")
                    }
                  >
                    {connecting ? "Connecting…" : "Connect"}
                  </button>
                )}
                {(connected || active) && (
                  <button onClick={() => ipc.disconnectDevice(role).then(refreshDevices)}>
                    {connected ? "Disconnect" : "Stop trying"}
                  </button>
                )}
                {slot?.saved_platform_id && (
                  <button
                    className="danger"
                    onClick={() => ipc.forgetDevice(role).then(refreshDevices)}
                  >
                    Forget
                  </button>
                )}
              </div>
              {scanning || scanResults.some((r) => r.role === role) ? (
                <ul className="scan-list">
                  {scanResults
                    .filter((r) => r.role === role)
                    .map((r) => (
                      <li key={r.platform_id}>
                        <button
                          className="scan-item"
                          disabled={connecting}
                          onClick={() => connect(role, r.platform_id, r.name)}
                        >
                          {r.name}
                          {r.rssi !== null && (
                            <span className="muted"> {r.rssi} dBm</span>
                          )}
                        </button>
                      </li>
                    ))}
                  {scanning && <li className="muted">searching…</li>}
                </ul>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
