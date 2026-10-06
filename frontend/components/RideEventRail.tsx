import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { WorkoutSegmentResult, WorkoutSegmentRow } from "../ipc";
import type { RideTimelineItem } from "../rideTimeline";
import { useStore } from "../state";
import { zoneColor } from "./WorkoutGraph";
import ConnectionStatus from "./ConnectionStatus";
import VoiceComposer from "./VoiceComposer";

const TIMELINE_BOTTOM_TOLERANCE_PX = 32;

function formatTimelineDuration(durationS: number): string {
  const seconds = Math.max(0, Math.round(durationS));
  if (seconds < 60) return `${seconds} sec`;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return remainder === 0 ? `${minutes} min` : `${minutes} min ${remainder} sec`;
}

/** "2nd", "3rd", … for a re-ridden interval's attempt number. */
function ordinal(n: number): string {
  const tens = n % 100;
  if (tens >= 11 && tens <= 13) return `${n}th`;
  switch (n % 10) {
    case 1:
      return `${n}st`;
    case 2:
      return `${n}nd`;
    case 3:
      return `${n}rd`;
    default:
      return `${n}th`;
  }
}

function segmentSummary(result: WorkoutSegmentResult, segment?: WorkoutSegmentRow): string {
  const duration = result.skipped
    ? `${formatTimelineDuration(result.ridden_duration_s)} / ${formatTimelineDuration(result.planned_duration_s)}`
    : formatTimelineDuration(result.planned_duration_s);
  if (result.average_power_w === null) return duration;
  // An open interval had no target, so "@" would suggest one it never had.
  return segment?.kind === "freeride"
    ? `${duration} · rode ${result.average_power_w} W avg`
    : `${duration} @ ${result.average_power_w} W`;
}

function SegmentCard({ result, segment }: { result: WorkoutSegmentResult; segment?: WorkoutSegmentRow }) {
  const startColor = segment ? zoneColor(segment.start_pct) : "#30363d";
  const endColor = segment ? zoneColor(segment.end_pct) : startColor;
  const style = {
    "--timeline-zone-start": startColor,
    "--timeline-zone-end": endColor,
  } as CSSProperties;

  return (
    <div
      className={`timeline-bubble timeline-segment ${result.skipped ? "skipped" : ""}`}
      style={style}
    >
      <div className="timeline-segment-head">
        <span>
          {segment?.label || `Interval ${result.segment_index + 1}`}
          {result.attempt > 1 && ` (${ordinal(result.attempt)})`}
        </span>
        {result.skipped && <span className="timeline-segment-skipped">Skipped</span>}
      </div>
      <div className="timeline-segment-result">{segmentSummary(result, segment)}</div>
      {result.average_cadence_rpm !== null && (
        <div className="timeline-segment-cadence">
          {result.average_cadence_rpm} rpm
        </div>
      )}
    </div>
  );
}

function TimelineEntry({ item, segments }: {
  item: RideTimelineItem;
  segments: WorkoutSegmentRow[];
}) {
  if (item.kind === "user-action") {
    return (
      <div className="timeline-row timeline-row-user">
        <div className="timeline-bubble timeline-action">
          <span className="timeline-action-label">{item.label}</span>
        </div>
      </div>
    );
  }
  if (item.kind === "segment") {
    return (
      <div className="timeline-row timeline-row-app">
        <SegmentCard result={item.result} segment={segments[item.result.segment_index]} />
      </div>
    );
  }
  return (
    <div className="timeline-row timeline-row-app">
      <div className={`timeline-bubble timeline-message ${item.tone}`}>
        {item.message}
      </div>
    </div>
  );
}

export default function RideEventRail({ segments }: { segments: WorkoutSegmentRow[] }) {
  const items = useStore((state) => state.rideTimeline.items);
  const feedRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const [showLatest, setShowLatest] = useState(false);

  const scrollToLatest = (behavior: ScrollBehavior = "smooth") => {
    const feed = feedRef.current;
    if (!feed) return;
    feed.scrollTo({ top: feed.scrollHeight, behavior });
    pinnedRef.current = true;
    setShowLatest(false);
  };

  useEffect(() => {
    if (pinnedRef.current) scrollToLatest(items.length <= 1 ? "auto" : "smooth");
  }, [items.length]);

  return (
    <aside className="ride-event-rail" aria-label="Ride timeline and status">
      <ConnectionStatus className="ride-connections" />
      <div className="ride-event-feed-wrap">
        <div
          ref={feedRef}
          className="ride-event-feed"
          role="log"
          aria-live="polite"
          onScroll={(event) => {
            const feed = event.currentTarget;
            const distance = feed.scrollHeight - feed.scrollTop - feed.clientHeight;
            const pinned = distance <= TIMELINE_BOTTOM_TOLERANCE_PX;
            pinnedRef.current = pinned;
            setShowLatest(!pinned);
          }}
        >
          <div className="ride-event-feed-spacer" />
          {items.map((item) => (
            <TimelineEntry key={item.id} item={item} segments={segments} />
          ))}
        </div>
        {showLatest && (
          <button className="timeline-latest" onClick={() => scrollToLatest()}>
            Latest ↓
          </button>
        )}
      </div>
      <VoiceComposer />
    </aside>
  );
}
