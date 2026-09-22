import type { DeviceStatusEvent } from "../ipc";
import { useStore } from "../state";
import RailStatusCard, {
  HeartRateStatusIcon,
  TrainerStatusIcon,
  type RailStatusTone,
} from "./RailStatusCard";

type ConnectionState = DeviceStatusEvent["status"];

function connectionPresentation(
  status: ConnectionState | undefined,
): { state: string; tone: RailStatusTone } {
  if (status === "connected") return { state: "connected", tone: "ok" };
  if (status === "connecting") return { state: "connecting", tone: "active" };
  if (status === "reconnecting") return { state: "reconnecting", tone: "warn" };
  return { state: "disconnected", tone: "neutral" };
}

export default function ConnectionStatus({ className = "" }: { className?: string }) {
  const deviceStatus = useStore((state) => state.deviceStatus);
  const devices = useStore((state) => state.devices);

  return (
    <div className={`connection-status ${className}`.trim()} aria-label="Device connections">
      {(["trainer", "hrm"] as const).map((role) => {
        const device = deviceStatus[role];
        const presentation = connectionPresentation(device?.status);
        const trainer = role === "trainer";
        const saved = devices.find((slot) => slot.role === role);
        const name = device?.name ?? saved?.saved_name ?? "Not configured";
        const roleLabel = trainer ? "Trainer" : "Heart-rate monitor";
        return (
          <RailStatusCard
            key={role}
            icon={trainer ? <TrainerStatusIcon /> : <HeartRateStatusIcon />}
            primary={name}
            tone={presentation.tone}
            ariaLabel={`${roleLabel}: ${name}, ${presentation.state}`}
          />
        );
      })}
    </div>
  );
}
