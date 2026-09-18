import { useVoice } from "../voice/VoiceController";
import type { VoicePhase } from "../voice/voiceMachine";
import RailStatusCard, {
  MicrophoneStatusIcon,
  type RailStatusTone,
} from "./RailStatusCard";

const STATUS: Record<VoicePhase, { state: string; tone: RailStatusTone }> = {
  off: { state: "Off", tone: "neutral" },
  "permission-required": { state: "Permission needed", tone: "warn" },
  preparing: { state: "Preparing…", tone: "active" },
  listening: { state: "Listening", tone: "ok" },
  speech: { state: "Hearing command", tone: "active" },
  interpreting: { state: "Interpreting…", tone: "active" },
  executing: { state: "Executing…", tone: "active" },
  suspended: { state: "Suspended", tone: "neutral" },
  unavailable: { state: "Unavailable", tone: "danger" },
  error: { state: "Error", tone: "danger" },
};

export default function VoiceStatus() {
  const { state, progress, retry } = useVoice();
  const commandsPaused = state.commandsSuspended && (
    state.phase === "listening" || state.phase === "speech" ||
    state.phase === "interpreting" || state.phase === "executing"
  );
  const status = commandsPaused
    ? { state: "Commands paused", tone: "warn" as const }
    : STATUS[state.phase];
  const canRetry = state.phase === "unavailable" || state.phase === "error";
  const detail = state.error ?? (
    state.phase === "unavailable"
      ? "Allow microphone access, then retry."
      : commandsPaused
        ? "Say “resume listening” to reactivate."
        : null
  );

  return (
    <RailStatusCard
      className="voice-status"
      icon={<MicrophoneStatusIcon />}
      label="Voice"
      state={status.state}
      tone={status.tone}
      announce
      detail={state.phase === "preparing" && progress
        ? `Loading ${progress.source === "speech" ? "speech" : "commands"}…`
        : detail}
      action={canRetry
        ? <button className="voice-retry" onClick={retry}>Retry</button>
        : undefined}
    />
  );
}
