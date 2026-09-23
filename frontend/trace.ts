import { ipc } from "./ipc";

type TraceValue = string | number | boolean | null | undefined;
export type TraceFields = Record<string, TraceValue>;

function logfmtValue(value: string | number | boolean): string {
  if (typeof value !== "string") return String(value);
  return /^[A-Za-z0-9_.:/-]+$/.test(value) ? value : JSON.stringify(value);
}

/** Emit one conventional logfmt event into the backend tracing stream. */
export function traceEvent(name: string, fields: TraceFields = {}): void {
  const entries = Object.entries(fields)
    .filter((entry): entry is [string, string | number | boolean] =>
      entry[1] !== null && entry[1] !== undefined &&
      (typeof entry[1] !== "number" || Number.isFinite(entry[1])))
    .map(([key, value]) => `${key}=${logfmtValue(value)}`);
  const message = [
    `event=${logfmtValue(name)}`,
    `at_unix_ms=${Date.now()}`,
    ...entries,
  ].join(" ");
  void ipc.traceFrontend(message).catch(() => undefined);
}
