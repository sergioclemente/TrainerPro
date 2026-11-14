// The interval clock's direction (elapsed or remaining) is one rider
// preference shown in two windows: the Player and the mini HUD. It lives in
// localStorage, which both windows share, so it sticks across rides; a Tauri
// event tells the other window about a change while both are open.

import { useCallback, useEffect, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";

const INTERVAL_CLOCK_KEY = "trainerpro.player.intervalClock";
const INTERVAL_CLOCK_EVENT = "interval_clock_changed";
const REMAINING = "remaining";
const ELAPSED = "elapsed";

interface IntervalClockChanged {
  showElapsed: boolean;
}

/** Elapsed unless the rider chose remaining. */
function storedShowElapsed(): boolean {
  return localStorage.getItem(INTERVAL_CLOCK_KEY) !== REMAINING;
}

/** Whether the interval clock counts up, and a toggle that flips it in
    every window. */
export function useIntervalClock(): [showElapsed: boolean, toggle: () => void] {
  const [showElapsed, setShowElapsed] = useState(storedShowElapsed);

  useEffect(() => {
    let disposed = false;
    const subscription = listen<IntervalClockChanged>(INTERVAL_CLOCK_EVENT, ({ payload }) => {
      if (!disposed) setShowElapsed(payload.showElapsed);
    });
    return () => {
      disposed = true;
      void subscription.then((unlisten) => unlisten());
    };
  }, []);

  const toggle = useCallback(() => {
    const next = !storedShowElapsed();
    localStorage.setItem(INTERVAL_CLOCK_KEY, next ? ELAPSED : REMAINING);
    setShowElapsed(next);
    void emit(INTERVAL_CLOCK_EVENT, { showElapsed: next } satisfies IntervalClockChanged).catch(
      () => undefined,
    );
  }, []);

  return [showElapsed, toggle];
}
