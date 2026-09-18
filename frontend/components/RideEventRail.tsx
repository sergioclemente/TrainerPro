import { useVoice } from "../voice/VoiceController";
import ConnectionStatus from "./ConnectionStatus";
import VoiceStatus from "./VoiceStatus";

export default function RideEventRail() {
  const { feedback } = useVoice();

  return (
    <aside className="ride-event-rail" aria-label="Ride status">
      <div className="ride-event-feed" aria-live="polite">
        {feedback && (
          <div className={`voice-feedback ${feedback.kind}`}>
            <span className="voice-feedback-command">{feedback.command}</span>
            <span className="voice-feedback-result">{feedback.result}</span>
          </div>
        )}
      </div>
      <div className="ride-status-cluster">
        <VoiceStatus />
        <ConnectionStatus />
      </div>
    </aside>
  );
}
