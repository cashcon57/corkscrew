/**
 * Format a date-ish value as a relative time ("3 days ago", "just now").
 * Accepts ISO strings, epoch millis, or Date. Returns "" on parse failure.
 */
export function relativeTime(input: string | number | Date | null | undefined, nowMs?: number): string {
  if (input === null || input === undefined) return "";
  let ms: number;
  if (input instanceof Date) {
    ms = input.getTime();
  } else if (typeof input === "number") {
    ms = input;
  } else {
    const parsed = Date.parse(input);
    if (Number.isNaN(parsed)) return "";
    ms = parsed;
  }
  const now = nowMs ?? Date.now();
  const diff = Math.max(0, now - ms);
  const sec = Math.floor(diff / 1000);
  if (sec < 45) return "just now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `${day}d ago`;
  const mon = Math.floor(day / 30);
  if (day < 365) return `${mon}mo ago`;
  const yr = Math.floor(day / 365);
  return `${yr}y ago`;
}

/** Format as absolute short date (YYYY-MM-DD) for tooltips. */
export function absoluteDate(input: string | number | Date | null | undefined): string {
  if (input === null || input === undefined) return "";
  let d: Date;
  if (input instanceof Date) d = input;
  else if (typeof input === "number") d = new Date(input);
  else {
    const t = Date.parse(input);
    if (Number.isNaN(t)) return "";
    d = new Date(t);
  }
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const da = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${da}`;
}
