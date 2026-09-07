// Workout profile graph: zone-colored segment polygons + progress cursor.
// Data is the (t_s, %FTP) breakpoint polyline stored at import (SPEC §3.3) —
// exactly two points per segment, so polygon i ↔ segment i. When `segments`
// is provided (detail view), bars are hoverable with a step tooltip.

import { useState } from "react";
import { SegmentRow, fmtDuration } from "../ipc";

interface Props {
  graph: [number, number][];
  durationS: number;
  progressS?: number;
  height?: number;
  segments?: SegmentRow[];
  /** FTP in watts. Supplied to draw a watt scale over the profile. */
  ftp?: number;
  /** Segment being ridden right now. Selected — and described — unless the
      pointer picks another one. */
  activeIndex?: number | null;
  /** Inclusive segment span to highlight, for callers whose selection covers
      more than one segment (a builder Repeat expands to many). Takes
      precedence over `activeIndex`; the pointer still overrides both. */
  highlightRange?: [number, number] | null;
}

/** Watt spacing of the scale labels. */
const GRID_STEP_W = 100;

export function zoneColor(pct: number): string {
  if (pct < 55) return "#6e7681"; // Z1 recovery
  if (pct <= 75) return "#388bfd"; // Z2 endurance
  if (pct <= 90) return "#3fb950"; // Z3 tempo
  if (pct <= 105) return "#d29922"; // Z4 threshold
  if (pct <= 120) return "#f0883e"; // Z5 vo2
  if (pct <= 150) return "#f85149"; // Z6 anaerobic
  return "#bc8cff"; // Z7 neuromuscular
}

/** Resolve a displayed %FTP value exactly as the Rust power model does for
    positive workout targets: nearest whole watt. */
export function wattsFromPct(pct: number, ftp: number): number {
  return Math.round((pct / 100) * ftp);
}

export function segmentText(s: SegmentRow, ftp?: number): string {
  if (s.kind === "freeride") return `${s.label} — ${fmtDuration(s.duration_s)}`;
  const pct =
    s.kind === "ramp"
      ? `${Math.round(s.start_pct)}% → ${Math.round(s.end_pct)}%`
      : `${Math.round(s.start_pct)}%`;
  const watts =
    ftp && ftp > 0
      ? s.kind === "ramp"
        ? ` (${wattsFromPct(s.start_pct, ftp)}\u00a0W → ${wattsFromPct(s.end_pct, ftp)}\u00a0W)`
        : ` (${wattsFromPct(s.start_pct, ftp)}\u00a0W)`
      : "";
  const cad = s.cadence_rpm != null ? ` · ${s.cadence_rpm} rpm` : "";
  return `${s.label} — ${fmtDuration(s.duration_s)} @ ${pct} FTP${watts}${cad}`;
}

export default function WorkoutGraph({
  graph,
  durationS,
  progressS,
  height = 120,
  segments,
  ftp,
  activeIndex = null,
  highlightRange = null,
}: Props) {
  const W = 1000;
  const [hover, setHover] = useState<number | null>(null);
  const [mouse, setMouse] = useState({ x: 0, y: 0, w: 1, h: 1 });
  const maxPct = Math.max(130, ...graph.map(([, p]) => p)) * 1.05;
  const x = (t: number) => (t / Math.max(durationS, 1)) * W;
  const y = (p: number) => height - (p / maxPct) * height;

  const polys: { points: string; color: string; x0: number; x1: number }[] = [];
  for (let i = 0; i + 1 < graph.length; i += 2) {
    const [t0, p0] = graph[i];
    const [t1, p1] = graph[i + 1];
    polys.push({
      points: `${x(t0)},${height} ${x(t0)},${y(p0)} ${x(t1)},${y(p1)} ${x(t1)},${height}`,
      color: zoneColor((p0 + p1) / 2),
      x0: x(t0),
      x1: x(t1),
    });
  }

  /** Midpoint of a bar as a % of the width, kept off the edges so a tooltip
      centred there is not clipped by the (overflow-hidden) graph frame. */
  const anchorPct = (p: { x0: number; x1: number }) =>
    Math.min(85, Math.max(15, (((p.x0 + p.x1) / 2) / W) * 100));

  const interactive = segments && segments.length === polys.length;
  // Default selection is whatever the caller marks — one segment being ridden,
  // or a span the editor has selected. The pointer overrides either.
  const clamp = (i: number) => Math.min(polys.length - 1, Math.max(0, i));
  const marked: [number, number] | null =
    highlightRange && polys.length > 0
      ? [clamp(highlightRange[0]), clamp(highlightRange[1])]
      : activeIndex !== null && activeIndex < polys.length
        ? [activeIndex, activeIndex]
        : null;
  const range: [number, number] | null = hover !== null ? [hover, hover] : marked;
  // The single index the tooltip describes: a span is named by its first bar.
  const selected = range ? range[0] : null;
  const hovered = interactive && selected !== null ? segments![selected] : null;

  // Watt scale: labels only, every 100 W. Rules across the whole width read as
  // clutter over the profile, so the numbers sit alone at the left edge. They
  // live in HTML, not SVG — the chart is stretched with
  // preserveAspectRatio="none", which would distort text.
  const gridW: number[] = [];
  if (ftp && ftp > 0) {
    for (let w = GRID_STEP_W; w < (maxPct / 100) * ftp; w += GRID_STEP_W) gridW.push(w);
  }
  const wattY = (w: number) => y((w / ftp!) * 100);

  const svg = (
    <svg
      viewBox={`0 0 ${W} ${height}`}
      preserveAspectRatio="none"
      style={{ width: "100%", height: "100%", display: "block" }}
    >
      <line x1={0} x2={W} y1={y(100)} y2={y(100)} stroke="#30363d" strokeDasharray="6 6" />
      {polys.map((p, i) => (
        <polygon
          key={i}
          points={p.points}
          fill={p.color}
          opacity={range === null || (i >= range[0] && i <= range[1]) ? 0.95 : 0.5}
        />
      ))}
      {interactive &&
        polys.map((p, i) => (
          <rect
            key={`h${i}`}
            x={p.x0}
            y={0}
            width={Math.max(p.x1 - p.x0, 1)}
            height={height}
            fill="transparent"
            onMouseEnter={() => setHover(i)}
            onMouseLeave={() => setHover(null)}
          />
        ))}
      {progressS !== undefined && (
        // Drawn over the hit rects, so it must not swallow the hover.
        <>
          <rect
            x={0}
            y={0}
            width={x(progressS)}
            height={height}
            fill="#000"
            opacity={0.35}
            pointerEvents="none"
          />
          <line x1={x(progressS)} x2={x(progressS)} y1={0} y2={height} stroke="#e6edf3" strokeWidth={2} pointerEvents="none" />
        </>
      )}
      {/* Over the progress shading, so the selected interval stays legible once
          it has been partly ridden. */}
      {range !== null && (
        <rect
          x={polys[range[0]].x0}
          y={0}
          width={Math.max(polys[range[1]].x1 - polys[range[0]].x0, 1)}
          height={height}
          fill="#e6edf3"
          fillOpacity={0.1}
          stroke="#e6edf3"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
          pointerEvents="none"
        />
      )}
    </svg>
  );

  const scaleLabels = gridW.map((w) => (
    <span key={w} className="graph-scale-label" style={{ top: `${(wattY(w) / height) * 100}%` }}>
      {w} W
    </span>
  ));

  if (!interactive) {
    return scaleLabels.length === 0 ? (
      svg
    ) : (
      <div className="graph-wrap">
        {svg}
        {scaleLabels}
      </div>
    );
  }

  return (
    <div
      className="graph-wrap"
      onMouseMove={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        setMouse({ x: e.clientX - r.left, y: e.clientY - r.top, w: r.width, h: r.height });
      }}
    >
      {svg}
      {scaleLabels}
      {hovered && (
        <div
          className="graph-tooltip"
          style={
            hover !== null
              ? {
                  left: mouse.x > mouse.w * 0.6 ? mouse.x - 250 : mouse.x + 14,
                  top: mouse.y > mouse.h * 0.55 ? mouse.y - 58 : mouse.y + 14,
                }
              : // Not hovering: the label belongs to the interval being ridden,
                // so park it over that interval rather than at the pointer.
                {
                  left: `${anchorPct(polys[selected!])}%`,
                  bottom: 8,
                  transform: "translateX(-50%)",
                }
          }
        >
          <div className="graph-tooltip-title">{segmentText(hovered, ftp)}</div>
          {hovered.note && <div className="graph-tooltip-note">{hovered.note}</div>}
        </div>
      )}
    </div>
  );
}
