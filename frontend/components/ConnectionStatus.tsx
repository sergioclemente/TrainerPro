import type { DeviceStatusEvent } from "../ipc";
import { useStore } from "../state";
import RailStatusCard, {
  HeartRateStatusIcon,
  ControllerStatusIcon,
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
      {(["trainer", "hrm", "controller"] as const).map((role) => {
        const device = deviceStatus[role];
        const trainer = role === "trainer";
        const saved = devices.find((slot) => slot.role === role);
        const controllerSource = role === "controller" ? saved?.controller_source : null;
        const presentation = controllerSource === "disabled"
          ? connectionPresentation(undefined)
          : connectionPresentation(device?.status ?? (saved?.connected ? "connected" : undefined));
        const name = controllerSource === "disabled" ? "Off"
          : controllerSource === "trainer_controls"
            ? (device?.status === "connected" ? device.name : null) ?? "Trainer controls"
            : device?.name ?? saved?.saved_name ?? "Not configured";
        const roleLabel = trainer ? "Trainer" : role === "hrm" ? "Heart-rate monitor" : "Controller";
        return (
          <RailStatusCard
            key={role}
            icon={trainer ? <TrainerStatusIcon /> : role === "hrm" ? <HeartRateStatusIcon /> : <ControllerStatusIcon />}
            primary={name}
            tone={presentation.tone}
            ariaLabel={`${roleLabel}: ${name}, ${presentation.state}`}
          />
        );
      })}
    </div>
  );
}
