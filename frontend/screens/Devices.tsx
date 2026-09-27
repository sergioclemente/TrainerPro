import { AppError, Role, ipc } from "../ipc";
import { controllerBinding } from "../controllerBindings";
import { useStore } from "../state";

const ROLE_LABEL: Record<Role, string> = { trainer: "Smart Trainer", hrm: "Heart Rate", controller: "Controller" };

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
        <button onClick={scan} disabled={scanning}>
          {scanning ? "Scanning…" : "Scan for devices"}
        </button>
      </header>
      <div className="device-slots">
        {(["trainer", "hrm", "controller"] as Role[]).map((role) => {
          const slot = devices.find((d) => d.role === role);
          const status = deviceStatus[role];
          const connected = status ? status.status === "connected" : (slot?.connected ?? false);
          const connecting = status?.status === "connecting";
          const reconnecting = status?.status === "reconnecting";
          const active = connecting || reconnecting;
          const controller = role === "controller";
          const source = slot?.controller_source;
          const followsTrainer = controller && source === "trainer_controls";
          const usesPairedController = controller && source === "paired_controller";
          const savedControllerId = controller ? slot?.saved_platform_id : null;
          const savedControllerName = controller ? slot?.saved_name : null;
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
                {controller && source === "disabled" ? (
                  <span className="status muted">○ Off</span>
                ) : connecting ? (
                  <span className="status warn">● connecting…</span>
                ) : connected ? (
                  <span className="status ok">
                    ● {status?.name ?? (followsTrainer ? "Trainer controls" : slot?.saved_name) ?? "connected"}
                    {live && <span className="live-reading"> {live}</span>}
                  </span>
                ) : reconnecting ? (
                  <span className="status warn">
                    ● reconnecting to {status.name ?? slot?.saved_name ?? "saved device"}…
                  </span>
                ) : followsTrainer ? (
                  <span className="status muted">○ Waiting for compatible trainer</span>
                ) : controller && !usesPairedController ? (
                  <span className="status muted">○ Off</span>
                ) : slot?.saved_name ? (
                  <span className="status muted">○ {slot.saved_name} (saved)</span>
                ) : (
                  <span className="status muted">○ not paired</span>
                )}
              </div>
              {controller && (
                <p className="muted">
                  {slot?.error ?? (followsTrainer && !connected ? "Controls follow a compatible connected trainer." :
                    controllerBinding(slot?.controller_profile ?? null)?.help ??
                    (usesPairedController ? "Wake the paired controller to connect." : "Choose a control source below."))}
                </p>
              )}
              {controller ? (
                <>
                  <div className="row gap controller-sources" role="group" aria-label="Controller source">
                    <button className="source-choice" aria-pressed={followsTrainer} disabled={followsTrainer}
                      onClick={() => ipc.setControllerSource("trainer_controls").then(refreshDevices).catch((e: AppError) => pushToast("error", e.message))}>
                      Trainer controls
                    </button>
                    {savedControllerId && (
                      <button className="source-choice" aria-pressed={usesPairedController} disabled={usesPairedController && (active || connected)}
                        onClick={() => connect("controller", savedControllerId, savedControllerName ?? "Paired controller")}>
                        {usesPairedController && connected ? savedControllerName ?? "Paired controller" : usesPairedController ? "Retry paired controller" : "Use paired controller"}
                      </button>
                    )}
                    <button className="source-choice" aria-pressed={source === "disabled"} disabled={source === "disabled"}
                      onClick={() => ipc.setControllerSource("disabled").then(refreshDevices).catch((e: AppError) => pushToast("error", e.message))}>
                      Off
                    </button>
                  </div>
                  {savedControllerId && (
                    <div className="row gap controller-saved-device">
                      <span className="muted">Saved controller: {savedControllerName ?? "Paired controller"}</span>
                      <button className="danger" onClick={() => ipc.forgetDevice("controller").then(refreshDevices).catch((e: AppError) => pushToast("error", e.message))}>
                        Forget controller
                      </button>
                    </div>
                  )}
                </>
              ) : (
                <div className="row gap">
                {!connected && !active && slot?.saved_platform_id && (
                  <button
                    className="primary"
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
              )}
              {scanning || scanResults.some((r) => r.role === role) ? (
                <ul className="scan-list">
                  {scanResults
                    .filter((r) => r.role === role)
                    .map((r) => (
                      <li key={r.platform_id}>
                        <button
                          className="scan-item"
                          disabled={active}
                          onClick={() => connect(role, r.platform_id, r.name)}
                        >
                          {controller ? `${r.platform_id === savedControllerId ? "Use" : "Pair"} ${r.name}` : r.name}
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
