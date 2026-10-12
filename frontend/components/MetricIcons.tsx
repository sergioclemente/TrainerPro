// Matched metric glyphs. Inline SVG (not unicode/emoji) so all three render
// at identical size and weight regardless of platform font fallback.

const props = {
  width: 22,
  height: 22,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  className: "metric-icon",
};

export function BoltIcon() {
  return (
    <svg {...props}>
      <path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z" />
    </svg>
  );
}

export function CadenceIcon() {
  return (
    <svg {...props}>
      <path d="M21 12a9 9 0 1 1-3.5-7.1" />
      <polyline points="21 3 21 9 15 9" />
      <circle cx="12" cy="12" r="2.5" />
    </svg>
  );
}

export function HeartIcon() {
  return (
    <svg {...props} className="metric-icon heart">
      <path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z" />
    </svg>
  );
}

const small = { ...props, width: 15, height: 15, strokeWidth: 2.2, className: "metric-icon small" };

/** Arrows pulling inward: shrink the Player to the mini HUD. */
export function MiniIcon() {
  return (
    <svg {...small}>
      <path d="M4 14v6h6" />
      <path d="M10 14l-6 6" />
      <path d="M20 10V4h-6" />
      <path d="M14 10l6-6" />
    </svg>
  );
}

/** Arrows pushing outward: the HUD's way back to the full Player. */
export function ExpandIcon() {
  return (
    <svg {...small}>
      <path d="M14 4h6v6" />
      <path d="M20 4l-7 7" />
      <path d="M10 20H4v-6" />
      <path d="M4 20l7-7" />
    </svg>
  );
}
