// Zwift workout browser backed by whatsonzwift.com (read-only scrape).
// Browse collections → workout cards (full structure parsed server-side, so
// graphs are immediate) → detail view → ride. State lives in the store and
// survives navigation.

import { useEffect, useState } from "react";
import { AppError, WozWorkout, fmtDuration, ipc } from "../ipc";
import { useStore } from "../state";
import WorkoutGraph from "../components/WorkoutGraph";

export default function WozTab() {
  const {
    wozCollections: collections,
    wozCollection: selected,
    wozWorkouts: workouts,
    wozStatus: status,
    wozError: error,
  } = useStore();
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (collections.length === 0) {
      ipc
        .wozCollections()
        .then((list) => useStore.setState({ wozCollections: list }))
        .catch((e) =>
          useStore.setState({
            wozStatus: "error",
            wozError: (e as AppError).message ?? String(e),
          }),
        );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function openCollection(slug: string, force = false) {
    useStore.setState({ wozCollection: slug, wozStatus: "loading", wozError: "" });
    try {
      const list = await ipc.wozWorkouts(slug, force);
      useStore.setState({ wozWorkouts: list, wozStatus: "ready" });
    } catch (e) {
      useStore.setState({
        wozStatus: "error",
        wozError: (e as AppError).message ?? String(e),
      });
    }
  }

  function openDetail(w: WozWorkout) {
    useStore.setState({
      detail: {
        source: "woz",
        wozRef: { collection: selected!, idx: w.idx },
        name: w.title,
        description: "",
        duration_s: w.duration_s,
        est_if: w.est_if,
        est_tss: w.est_tss,
        tags: "",
        graph: w.graph,
        segments: w.segments,
      },
      screen: "workout",
    });
  }

  const q = query.trim().toLowerCase();

  if (status === "error") {
    return (
      <div className="planner-empty">
        <p className="empty">whatsonzwift.com: {error}</p>
        <button
          className="primary"
          onClick={() => {
            useStore.setState({ wozStatus: "idle", wozError: "" });
            if (selected) void openCollection(selected);
            else
              ipc
                .wozCollections()
                .then((list) =>
                  useStore.setState({ wozCollections: list, wozStatus: "idle" }),
                )
                .catch(() => {});
          }}
        >
          Retry
        </button>
      </div>
    );
  }

  // Collection browser (no collection selected).
  if (!selected) {
    const visible = collections.filter((c) => !q || c.title.toLowerCase().includes(q));
    return (
      <div>
        <div className="planner-toolbar">
          <input
            className="search"
            placeholder="Search collections…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <span className="muted planner-meta">
            {collections.length === 0
              ? "loading collections…"
              : `${visible.length}/${collections.length} collections · whatsonzwift.com`}
          </span>
        </div>
        <div className="collection-grid">
          {visible.map((c) => (
            <button key={c.slug} className="collection-card" onClick={() => openCollection(c.slug)}>
              {c.title}
            </button>
          ))}
        </div>
        <p className="muted footnote">
          Workout data browsed from whatsonzwift.com — support them by visiting
          the site. Workouts are ridden with your FTP from Settings.
        </p>
      </div>
    );
  }

  const title = collections.find((c) => c.slug === selected)?.title ?? selected;
  const visible = workouts.filter((w) => !q || w.title.toLowerCase().includes(q));

  return (
    <div>
      <div className="planner-toolbar">
        <button
          className="ghost back"
          onClick={() => useStore.setState({ wozCollection: null, wozWorkouts: [], wozStatus: "idle" })}
        >
          ‹ Collections
        </button>
        <strong>{title}</strong>
        <input
          className="search"
          placeholder="Search workouts…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <span className="muted planner-meta">
          {status === "loading" ? "loading…" : `${visible.length}/${workouts.length}`}
        </span>
        <button onClick={() => openCollection(selected, true)}>Sync</button>
        <button onClick={() => ipc.wozOpenPage(selected)}>View on site ↗</button>
      </div>
      {status === "loading" && <p className="empty">Fetching collection…</p>}
      <div className="card-grid">
        {visible.map((w) => (
          <div key={w.idx} className="card workout-card" onClick={() => openDetail(w)}>
            <div className="thumb">
              <WorkoutGraph graph={w.graph} durationS={w.duration_s} height={80} />
            </div>
            <div className="card-body">
              <strong>{w.title}</strong>
              <span className="muted">
                {fmtDuration(w.duration_s)} · IF {w.est_if.toFixed(2)} · TSS{" "}
                {Math.round(w.est_tss)}
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
