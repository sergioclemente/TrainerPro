import { AppError, Role, ipc } from "../ipc";
import { useStore } from "../state";

const ROLE_LABEL: Record<Role, string> = { trainer: "Smart Trainer", hrm: "Heart Rate" };

export default function Devices() {
  const {
    devices,
    scanResults,
    scanning,
    deviceStatus,
    deviceReading,
    pushToast,
    refreshDevices,
  } = useStore();

  async function scan(role: Role) {
    useStore.setState({ scanning: role, scanResults: [] });
    try {
      await ipc.startScan(role);
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
      useStore.setState({ scanning: null });
    }
  }

  async function connect(role: Role, platformId: string, name: string) {
    try {
      await ipc.connectDevice(role, platformId, name);
      useStore.setState({ scanning: null, scanResults: [] });
      await refreshDevices();
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  return (
    <div className="screen">
      <header className="screen-head">
        <h1>Devices</h1>
      </header>
      <div className="device-slots">
        {(["trainer", "hrm"] as Role[]).map((role) => {
          const slot = devices.find((d) => d.role === role);
          const status = deviceStatus[role];
          const connected = slot?.connected ?? false;
          const connecting = status?.status === "connecting";
          const reading = deviceReading[role];
          const live =
            connected && reading
              ? role === "trainer"
                ? reading.power != null
                  ? `${reading.power} W${reading.cadence != null ? ` · ${reading.cadence} rpm` : ""}`
                  : null
                : reading.hr != null
                  ? `${reading.hr} bpm`
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
                ) : status?.status === "reconnecting" ? (
                  <span className="status warn">
                    ● reconnecting… (attempt {status.attempt})
                  </span>
                ) : slot?.saved_name ? (
                  <span className="status muted">○ {slot.saved_name} (saved)</span>
                ) : (
                  <span className="status muted">○ not paired</span>
                )}
              </div>
              <div className="row gap">
                {!connected && slot?.saved_platform_id && (
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
                {connected && (
                  <button onClick={() => ipc.disconnectDevice(role).then(refreshDevices)}>
                    Disconnect
                  </button>
                )}
                <button onClick={() => scan(role)} disabled={scanning !== null || connecting}>
                  {scanning === role ? "Scanning…" : "Scan"}
                </button>
                {slot?.saved_platform_id && (
                  <button
                    className="danger"
                    onClick={() => ipc.forgetDevice(role).then(refreshDevices)}
                  >
                    Forget
                  </button>
                )}
              </div>
              {scanning === role || scanResults.some((r) => r.role === role) ? (
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
                  {scanning === role && <li className="muted">searching…</li>}
                </ul>
              ) : null}
            </div>
          );
        })}
      </div>
      <p className="muted footnote">
        The Simulated KICKR / HRM entries always appear in scans — use them to try
        TrainerPro without hardware.
      </p>
    </div>
  );
}
