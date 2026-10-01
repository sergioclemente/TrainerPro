import { useEffect, useState } from "react";
import { AppError, ProviderConnection, ipc } from "../ipc";
import { useStore } from "../state";
import { refreshPlanConnections } from "../providers";
import { SIGN_IN_FORMS } from "../components/provider-auth";

function ConnectionCard({ connection }: { connection: ProviderConnection }) {
  const { refreshProviderConnections, refreshNextUp, refreshWorkouts, activityTransfer, pushToast } = useStore();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const { provider, plan_source: plan } = connection;
  const disabled = busy || activityTransfer?.connectionId === connection.connection_id;
  const SignIn = SIGN_IN_FORMS[provider.id];

  async function synchronize(selected: ProviderConnection) {
    try {
      const [outcome] = await refreshPlanConnections([selected], ipc.refreshProviderPlans);
      if (!outcome) return;
      if (outcome.error) throw outcome.error;
      const report = outcome.report;
      pushToast(report.issues.length ? "warn" : "info", report.issues.length
        ? `${provider.name} synced with ${report.issues.length} workout issue(s)`
        : `${provider.name} synced: ${report.inserted} added, ${report.updated} updated, ${report.removed} removed`);
    } finally {
      await Promise.all([refreshProviderConnections(), refreshNextUp(), refreshWorkouts()]);
    }
  }

  async function connected() {
    setBusy(true);
    try {
      const connections = await refreshProviderConnections();
      pushToast("info", `Connected to ${provider.name}`);
      const updated = connections.find((item) => item.provider.id === provider.id);
      if (updated?.connection_id && updated.plan_source) await synchronize(updated);
    } catch (failure) { setError((failure as AppError).message ?? String(failure)); }
    finally { setBusy(false); }
  }

  async function refresh() {
    if (!connection.connection_id) return;
    setBusy(true);
    setError("");
    try { await synchronize(connection); }
    catch (failure) { setError((failure as AppError).message ?? String(failure)); }
    finally { setBusy(false); }
  }

  async function disconnect() {
    if (!connection.connection_id) return;
    if (plan && !confirm(`Disconnect ${provider.name}? Synced workouts will leave Next Up, but completed Activity history is preserved.`)) return;
    setBusy(true);
    setError("");
    try {
      await ipc.disconnectProvider(connection.connection_id);
      await Promise.all([refreshProviderConnections(), refreshNextUp(), refreshWorkouts()]);
      pushToast("info", `${provider.name} disconnected`);
    } catch (failure) { setError((failure as AppError).message ?? String(failure)); }
    finally { setBusy(false); }
  }

  return <section className="connection-card">
    <div className="connection-head">
      <div><h2>{provider.name}</h2><p className="muted">{provider.description}</p></div>
      <span className={`lib-badge ${connection.state === "connected" ? "on" : "off"}`}>
        {connection.state === "connected" ? "connected" : connection.state === "needs_sign_in" ? "sign in required"
          : connection.state === "unavailable" ? "unavailable" : "not connected"}
      </span>
    </div>
    <div className="connection-body">
      {provider.notice && <p className="muted footnote">{provider.notice}</p>}
      {connection.external_account_id && <p>Account: {connection.display_name || connection.external_account_id}</p>}
      {plan?.time_zone && <p>Time zone: {plan.time_zone}</p>}
      {(error || connection.error) && <p className="connection-error" role="alert">{error || connection.error?.message}</p>}
      {plan && connection.connection_id && <>
        <p className="muted footnote">{plan.last_sync_succeeded_at_unix_ms
          ? `Last synchronized ${new Date(plan.last_sync_succeeded_at_unix_ms).toLocaleString()}` : "Not synchronized yet"}</p>
        {plan.last_sync_error && <p className="connection-error">Last sync: {plan.last_sync_error}</p>}
      </>}
      {(connection.state === "disconnected" || connection.state === "needs_sign_in") && SignIn && <SignIn disabled={disabled} onConnected={connected} />}
      <div className="row gap">
        {plan && connection.state === "connected" && <button className="primary" disabled={disabled} onClick={() => void refresh()}>{busy ? "Synchronizing…" : "Sync now"}</button>}
        {connection.state === "unavailable" && <button disabled={disabled} onClick={() => void refreshProviderConnections().catch((failure) => setError(failure.message))}>Reload connection</button>}
        {connection.connection_id && <button className="danger" disabled={disabled} onClick={() => void disconnect()}>Disconnect</button>}
      </div>
      {connection.activity_destination && <p className="muted footnote">Disconnecting removes local sign-in credentials. Your activities and upload history are preserved.</p>}
    </div>
  </section>;
}

export default function ConnectionsPanel() {
  const { providerConnections, providerConnectionsError, refreshProviderConnections } = useStore();
  useEffect(() => { void refreshProviderConnections().catch(() => {}); }, [refreshProviderConnections]);
  return <div className="connection-list">
    {providerConnectionsError && <p className="connection-error" role="alert">{providerConnectionsError} <button onClick={() => void refreshProviderConnections().catch(() => {})}>Reload connections</button></p>}
    {!providerConnections && !providerConnectionsError && <p className="muted">Loading connections…</p>}
    {providerConnections?.map((connection) => <ConnectionCard key={connection.provider.id} connection={connection} />)}
  </div>;
}
