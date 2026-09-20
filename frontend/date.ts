function localDateKey(now: Date): string {
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/** ISO local date for an instant in an IANA time zone, with the device's local
 * date as a defensive fallback for missing or invalid provider metadata. */
export function dateKeyInZone(now: Date, timeZone: string | null): string {
  if (!timeZone) return localDateKey(now);
  try {
    const parts = new Intl.DateTimeFormat("en-CA", {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    }).formatToParts(now);
    const part = (type: Intl.DateTimeFormatPartTypes) =>
      parts.find((candidate) => candidate.type === type)?.value;
    const year = part("year");
    const month = part("month");
    const day = part("day");
    if (year && month && day) return `${year}-${month}-${day}`;
  } catch {
    // The provider boundary validates this value. Keep the UI usable if an
    // older stored account contains a time zone this runtime does not know.
  }
  return localDateKey(now);
}
