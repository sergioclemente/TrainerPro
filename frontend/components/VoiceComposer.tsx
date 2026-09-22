import { useVoice } from "../voice/VoiceController";
import type { VoicePhase } from "../voice/voiceMachine";
import { MicrophoneStatusIcon } from "./RailStatusCard";

const WAVEFORM_BAR_MULTIPLIERS = [0.35, 0.7, 1, 0.55, 0.85, 0.45, 0.65];
const WAVEFORM_REST_SCALE = 0.18;

const PHASE_COPY: Record<VoicePhase, string> = {
  off: "Voice commands off",
  "permission-required": "Allow microphone access…",
  preparing: "Preparing voice commands…",
  listening: "Listening",
  speech: "Listening…",
  interpreting: "Interpreting…",
  executing: "Running command…",
  suspended: "Voice capture paused",
  unavailable: "Voice commands unavailable",
  error: "Voice commands unavailable",
};

function VoiceWaveform({ level }: { level: number }) {
  return (
    <span className="voice-waveform" aria-hidden="true">
      {WAVEFORM_BAR_MULTIPLIERS.map((multiplier, index) => (
        <span
          key={index}
          className="voice-waveform-bar"
          style={{ transform: `scaleY(${WAVEFORM_REST_SCALE + level * multiplier})` }}
        />
      ))}
    </span>
  );
}

export default function VoiceComposer() {
  const {
    state,
    progress,
    notice,
    partialTranscript,
    finalTranscript,
    inputLevel,
    retry,
  } = useVoice();
  const commandsPaused = state.commandsSuspended && (
    state.phase === "listening" || state.phase === "speech" ||
    state.phase === "interpreting" || state.phase === "executing"
  );
  const canRetry = state.phase === "unavailable" || state.phase === "error";
  const transcript = state.phase === "speech"
    ? partialTranscript
    : finalTranscript || (notice ? partialTranscript : "");
  const loading = state.phase === "preparing" && progress
    ? `Loading ${progress.source === "speech" ? "speech" : "commands"}…`
    : null;
  const statusMessage = loading || (
    commandsPaused ? "Commands paused — say “resume listening”" : null
  ) || state.error || PHASE_COPY[state.phase];
  const announcement = transcript && notice
    ? `Heard ${transcript}. ${notice.message}`
    : transcript || notice?.message || statusMessage;
  const showWaveform = !notice && !transcript && !loading &&
    !commandsPaused && state.phase === "listening";

  return (
    <div
      className={`voice-composer voice-composer-${notice?.kind ?? state.phase}`}
      role="status"
      aria-live="polite"
      aria-label={`Voice commands: ${announcement}`}
    >
      <span className="voice-composer-icon" aria-hidden="true">
        <MicrophoneStatusIcon />
      </span>
      <span className="voice-composer-content">
        {showWaveform ? (
          <VoiceWaveform level={inputLevel} />
        ) : transcript ? (
          <>
            <span className="voice-composer-transcript">“{transcript}”</span>
            {notice && (
              <span className={`voice-composer-notice ${notice.kind}`}>
                {notice.message}
              </span>
            )}
          </>
        ) : (
          notice?.message || statusMessage
        )}
      </span>
      {canRetry && (
        <button className="voice-retry" onClick={retry}>Retry</button>
      )}
    </div>
  );
}
