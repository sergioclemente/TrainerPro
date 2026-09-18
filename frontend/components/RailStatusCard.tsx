import type { ReactNode } from "react";

export type RailStatusTone = "neutral" | "ok" | "active" | "warn" | "danger";

interface RailStatusCardProps {
  icon: ReactNode;
  label: string;
  state: string;
  tone: RailStatusTone;
  detail?: ReactNode;
  action?: ReactNode;
  className?: string;
  announce?: boolean;
}

export default function RailStatusCard({
  icon,
  label,
  state,
  tone,
  detail,
  action,
  className = "",
  announce = false,
}: RailStatusCardProps) {
  return (
    <div
      className={`rail-status-card rail-status-${tone} ${className}`.trim()}
      role={announce ? "status" : undefined}
      aria-live={announce ? "polite" : undefined}
      aria-atomic={announce ? "true" : undefined}
    >
      <div className="rail-status-card-main">
        <span className="rail-status-card-icon" aria-hidden="true">{icon}</span>
        <span className="rail-status-card-copy">
          <span className="rail-status-card-label">{label}</span>
          <span className="rail-status-card-state">{state}</span>
        </span>
      </div>
      {detail && <span className="rail-status-card-detail">{detail}</span>}
      {action && <span className="rail-status-card-action">{action}</span>}
    </div>
  );
}

const iconProps = {
  width: 20,
  height: 20,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

export function MicrophoneStatusIcon() {
  return (
    <svg {...iconProps}>
      <rect x="9" y="2" width="6" height="12" rx="3" />
      <path d="M5 10a7 7 0 0 0 14 0M12 17v5M8 22h8" />
    </svg>
  );
}

export function TrainerStatusIcon() {
  return (
    <svg {...iconProps}>
      <circle cx="6" cy="17" r="4" />
      <circle cx="18" cy="17" r="4" />
      <path d="m6 17 4-8h4l4 8M8 13h8M10 9 8 6h3M14 6h3" />
    </svg>
  );
}

export function HeartRateStatusIcon() {
  return (
    <svg {...iconProps}>
      <path d="M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1.1-1.1a5.5 5.5 0 0 0-7.8 7.8L12 21l8.8-8.6a5.5 5.5 0 0 0 0-7.8Z" />
      <path d="M3.5 13h4l1.5-3 3 6 1.5-3h7" />
    </svg>
  );
}
