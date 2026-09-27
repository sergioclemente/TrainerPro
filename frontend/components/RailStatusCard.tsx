import type { ReactNode } from "react";

export type RailStatusTone = "neutral" | "ok" | "active" | "warn" | "danger";

interface RailStatusCardProps {
  icon: ReactNode;
  primary: string;
  tone: RailStatusTone;
  ariaLabel: string;
  className?: string;
}

export default function RailStatusCard({
  icon,
  primary,
  tone,
  ariaLabel,
  className = "",
}: RailStatusCardProps) {
  return (
    <div
      className={`rail-status-card rail-status-${tone} ${className}`.trim()}
      aria-label={ariaLabel}
      title={ariaLabel}
    >
      <span className="rail-status-card-icon" aria-hidden="true">{icon}</span>
      <span className="rail-status-card-primary">{primary}</span>
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

export function ControllerStatusIcon() {
  return (
    <svg {...iconProps}>
      <path d="M8 7h8a4 4 0 0 1 4 3l1 7a2 2 0 0 1-3 2l-3-3H9l-3 3a2 2 0 0 1-3-2l1-7a4 4 0 0 1 4-3Z" />
      <path d="M7 10v4M5 12h4M16 11h.01M18 13h.01" />
    </svg>
  );
}
