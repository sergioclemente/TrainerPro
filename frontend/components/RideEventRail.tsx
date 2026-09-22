import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { SegmentResult, SegmentRow } from "../ipc";
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

function segmentSummary(result: SegmentResult): string {
  const duration = result.skipped
    ? `${formatTimelineDuration(result.ridden_duration_s)} / ${formatTimelineDuration(result.planned_duration_s)}`
    : formatTimelineDuration(result.planned_duration_s);
  return result.average_power_w === null
    ? duration
    : `${duration} @ ${result.average_power_w} W`;
}

function SegmentCard({ result, segment }: { result: SegmentResult; segment?: SegmentRow }) {
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
        <span>{segment?.label || `Interval ${result.segment_index + 1}`}</span>
        {result.skipped && <span className="timeline-segment-skipped">Skipped</span>}
      </div>
      <div className="timeline-segment-result">{segmentSummary(result)}</div>
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
  segments: SegmentRow[];
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

export default function RideEventRail({ segments }: { segments: SegmentRow[] }) {
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
