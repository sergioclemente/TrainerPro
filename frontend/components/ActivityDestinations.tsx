import { useState } from "react";
import { AppError, ProviderConnection, ipc } from "../ipc";
import { useStore } from "../state";

function DestinationButton({ connection, activityId }: { connection: ProviderConnection; activityId: string }) {
  const { activityTransfer, pushToast } = useStore();
  const [lastError, setLastError] = useState<AppError | null>(null);
  const receipt = connection.activity_destination?.uploads.find((upload) => upload.activity_id === activityId);
  const uploading = activityTransfer?.connectionId === connection.connection_id && activityTransfer?.activityId === activityId;
  const name = connection.provider.name;

  async function upload() {
    if (useStore.getState().activityTransfer) return;
    const connectionId = connection.connection_id;
    if (connection.state !== "connected" || !connectionId) {
      useStore.setState({ screen: "settings", settingsTab: "connections" });
      return;
    }
    if (lastError && ["activity_upload_uncertain", "activity_duplicate", "activity_receipt_failed"].includes(lastError.code)
      && !confirm(`${name} may already have this activity. Check its activity history first. Attempt another upload?`)) return;
    useStore.setState({ activityTransfer: { connectionId, activityId } });
    setLastError(null);
    try {
      const uploaded = await ipc.uploadActivity(connectionId, activityId);
      useStore.setState((state) => ({
        providerConnections: state.providerConnections?.map((item) => item.connection_id === connectionId && item.activity_destination
          ? { ...item, activity_destination: { uploads: [...item.activity_destination.uploads.filter((receipt) => receipt.activity_id !== activityId), uploaded] } }
          : item) ?? null,
      }));
      pushToast("info", `Activity uploaded to ${name}`);
    } catch (error) {
      const failure = error as AppError;
      setLastError(failure);
      pushToast("error", failure.message ?? String(error));
      await useStore.getState().refreshProviderConnections().catch(() => {});
    } finally {
      useStore.setState({ activityTransfer: null });
    }
  }

  if (receipt) {
    return <span className="lib-badge on" title={`Uploaded ${new Date(receipt.uploaded_at_unix_ms).toLocaleString()}`}>Uploaded to {name}</span>;
  }
  return (
    <button className="ghost" disabled={activityTransfer !== null} title={lastError?.message} onClick={() => void upload()}>
      {uploading ? "Uploading…" : connection.state === "needs_sign_in" ? `Sign in to ${name}`
        : connection.state === "disconnected" ? `Connect ${name}`
        : connection.state === "unavailable" ? `Review ${name} connection`
        : lastError ? `Retry ${name} upload` : `Upload to ${name}`}
    </button>
  );
}

export default function ActivityDestinations({ activityId }: { activityId: string }) {
  const { providerConnections, providerConnectionsError, refreshProviderConnections } = useStore();
  if (!providerConnections) {
    return providerConnectionsError
      ? <button className="ghost" title={providerConnectionsError} onClick={() => void refreshProviderConnections().catch(() => {})}>Reload connections</button>
      : <button className="ghost" disabled>Loading destinations…</button>;
  }
  return <>{providerConnections.filter((connection) => connection.provider.capabilities.includes("activities"))
    .map((connection) => <DestinationButton key={`${connection.provider.id}:${connection.connection_id}`} connection={connection} activityId={activityId} />)}</>;
}
