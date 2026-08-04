// WorkoutPlanner tab: integrated card view with search, filters, and lazy
// previews. List/preview state lives in the global store (state.ts) so
// navigating to a workout detail and back never re-syncs the backend.

import { useEffect, useState } from "react";
import { AppError, PlannerPreview, PlannerWorkout, fmtDuration, ipc } from "../ipc";
import { useStore } from "../state";
import WorkoutGraph from "../components/WorkoutGraph";

/** Re-sync automatically only when the last sync is older than this. */
const STALE_MS = 10 * 60 * 1000;

export default function PlannerTab() {
  const {
    settings,
    go,
    pushToast,
    plannerRows: rows,
    plannerPreviews: previews,
    plannerStatus: status,
    plannerError: error,
    plannerSyncedAt: syncedAt,
    plannerFromCache: fromCache,
    plannerSync,
  } = useStore();
  const [query, setQuery] = useState("");
  const [activeTag, setActiveTag] = useState<string | null>(null);
  const [bucket, setBucket] = useState<string | null>(null);
  const [sort, setSort] = useState<"newest" | "name" | "duration" | "tss">("newest");

  const cfg = settings?.sources.planner;
  const configured = !!cfg && cfg.enabled && (cfg.values.url ?? "").length > 0;
  const stale = syncedAt === null || Date.now() - syncedAt > STALE_MS;

  useEffect(() => {
    if (configured && status === "idle") void plannerSync();
    else if (configured && status === "ready" && stale) void plannerSync();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [configured]);

  async function openDetail(w: PlannerWorkout) {
    try {
      const cached = previews[w.wid];
      const p =
        cached && cached !== "error" ? (cached as PlannerPreview) : await ipc.plannerPreview(w.wid);
      useStore.setState({
        detail: {
          source: "planner",
          wid: w.wid,
          name: w.title,
          description: "",
          duration_s: p.duration_s,
          est_if: p.est_if,
          est_tss: p.est_tss,
          tags: w.tags,
          graph: p.graph,
          segments: p.segments,
          contentKey: w.dsl,
        },
        screen: "workout",
      });
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  if (!configured) {
    return (
      <div className="planner-empty">
        <p className="empty">
          Connect TrainerPro to your WorkoutPlanner server to ride its workouts
          directly.
        </p>
        <button className="primary" onClick={() => go("settings")}>
          Connect to WorkoutPlanner
        </button>
      </div>
    );
  }

  if (status === "idle" || status === "loading" || status === "waking") {
    return (
      <p className="empty">
        {status === "waking"
          ? "Waking up the planner (free tier sleeps)…"
          : "Loading workouts…"}
      </p>
    );
  }

  if (status === "error") {
    return (
      <div className="planner-empty">
        <p className="empty">Could not reach WorkoutPlanner: {error}</p>
        <div className="row gap">
          <button className="primary" onClick={() => plannerSync()}>
            Retry
          </button>
          <button onClick={() => go("settings")}>Settings</button>
        </div>
      </div>
    );
  }

  const tagsOf = (w: PlannerWorkout) =>
    w.tags.split(/[,\s]+/).map((t) => t.trim()).filter(Boolean);
  const allTags = [...new Set(rows.flatMap(tagsOf))].sort();
  const q = query.trim().toLowerCase();
  const BUCKETS: [string, (s: number) => boolean][] = [
    ["≤ 1 h", (s) => s <= 3600],
    ["≤ 90 min", (s) => s <= 5400],
    ["≤ 2 h", (s) => s <= 7200],
    ["> 2 h", (s) => s > 7200],
  ];
  const durationOf = (w: PlannerWorkout) => {
    const p = previews[w.wid];
    return p && p !== "error" ? p.duration_s : w.duration_s;
  };
  const visible = rows
    .filter((w) => {
      if (activeTag && !tagsOf(w).includes(activeTag)) return false;
      if (bucket) {
        const f = BUCKETS.find(([label]) => label === bucket);
        if (f && !f[1](durationOf(w))) return false;
      }
      if (!q) return true;
      return w.title.toLowerCase().includes(q) || w.tags.toLowerCase().includes(q);
    })
    .sort((a, b) => {
      switch (sort) {
        case "name":
          return a.title.localeCompare(b.title);
        case "duration":
          return durationOf(b) - durationOf(a);
        case "tss": {
          const ta = previews[a.wid] && previews[a.wid] !== "error"
            ? (previews[a.wid] as PlannerPreview).est_tss : (a.tss ?? 0);
          const tb = previews[b.wid] && previews[b.wid] !== "error"
            ? (previews[b.wid] as PlannerPreview).est_tss : (b.tss ?? 0);
          return tb - ta;
        }
        case "newest":
        default:
          // creation_ts sorts lexicographically; fall back to wid.
          if (a.created && b.created && a.created !== b.created)
            return b.created.localeCompare(a.created);
          return b.wid - a.wid;
      }
    });

  const host = (cfg!.values.url ?? "").replace(/^https?:\/\//, "");
  return (
    <div>
      <div className="planner-toolbar">
        <input
          className="search"
          placeholder="Search title or tags…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <select
          className="sort-select"
          value={sort}
          onChange={(e) => setSort(e.target.value as typeof sort)}
        >
          <option value="newest">Newest</option>
          <option value="name">Name A–Z</option>
          <option value="duration">Longest</option>
          <option value="tss">Hardest (TSS)</option>
        </select>
        <span className="muted planner-meta">
          {host} · {visible.length}/{rows.length}
          {syncedAt && ` · ${fromCache ? "cached " : "synced "}${new Date(syncedAt).toLocaleTimeString()}`}
        </span>
        <button onClick={() => plannerSync()}>Sync</button>
      </div>
      <div className="chip-row">
        {BUCKETS.map(([label]) => (
          <button
            key={label}
            className={`chip ${bucket === label ? "active" : ""}`}
            onClick={() => setBucket(bucket === label ? null : label)}
          >
            {label}
          </button>
        ))}
      </div>
      {allTags.length > 0 && (
        <div className="chip-row">
          {allTags.map((t) => (
            <button
              key={t}
              className={`chip ${activeTag === t ? "active" : ""}`}
              onClick={() => setActiveTag(activeTag === t ? null : t)}
            >
              {t}
            </button>
          ))}
        </div>
      )}
      {visible.length === 0 && (
        <p className="empty">
          {rows.length === 0
            ? "No bike workouts in the planner yet."
            : "Nothing matches the search."}
        </p>
      )}
      <div className="card-grid">
        {visible.map((w) => {
          const p = previews[w.wid];
          const preview = p && p !== "error" ? p : null;
          return (
            <div key={w.wid} className="card workout-card" onClick={() => openDetail(w)}>
              <div className="thumb">
                {preview ? (
                  <WorkoutGraph
                    graph={preview.graph}
                    durationS={preview.duration_s}
                    height={80}
                  />
                ) : (
                  <div className={`thumb-pending ${p === "error" ? "err" : ""}`}>
                    {p === "error" ? "no preview" : "…"}
                  </div>
                )}
              </div>
              <div className="card-body">
                <strong>{w.title}</strong>
                <span className="muted">
                  {fmtDuration(preview?.duration_s ?? w.duration_s)}
                  {preview
                    ? ` · IF ${preview.est_if.toFixed(2)} · TSS ${Math.round(preview.est_tss)}`
                    : w.tss !== null
                      ? ` · TSS ${Math.round(w.tss)}`
                      : ""}
                </span>
                {tagsOf(w).length > 0 && (
                  <span className="card-tags">
                    {tagsOf(w).map((t) => (
                      <span key={t} className="chip tiny">
                        {t}
                      </span>
                    ))}
                  </span>
                )}
              </div>
              <button
                className="ghost edit-link"
                title="Edit in WorkoutPlanner"
                onClick={(e) => {
                  e.stopPropagation();
                  void ipc.plannerOpenEditor(w.wid);
                }}
              >
                ✎↗
              </button>
            </div>
          );
        })}
      </div>
      <p className="muted footnote">
        Click a card for details and to ride it. ✎↗ opens the planner's web
        editor.
      </p>
    </div>
  );
}
