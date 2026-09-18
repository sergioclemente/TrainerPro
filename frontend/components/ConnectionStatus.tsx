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
  if (status === "connected") return { state: "Connected", tone: "ok" };
  if (status === "connecting") return { state: "Connecting…", tone: "active" };
  if (status === "reconnecting") return { state: "Reconnecting…", tone: "warn" };
  return { state: "Disconnected", tone: "neutral" };
}

export default function ConnectionStatus({ className = "" }: { className?: string }) {
  const deviceStatus = useStore((state) => state.deviceStatus);

  return (
    <div className={`connection-status ${className}`.trim()} aria-label="Device connections">
      {(["trainer", "hrm"] as const).map((role) => {
        const device = deviceStatus[role];
        const presentation = connectionPresentation(device?.status);
        const trainer = role === "trainer";
        return (
          <RailStatusCard
            key={role}
            icon={trainer ? <TrainerStatusIcon /> : <HeartRateStatusIcon />}
            label={trainer ? "Trainer" : "Heart rate"}
            state={presentation.state}
            tone={presentation.tone}
            detail={device?.name ?? undefined}
          />
        );
      })}
    </div>
  );
}
