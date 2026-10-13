/**
 * Shared human-readable byte-size formatter.
 * Single source of truth — do not hand-roll copies in components.
 */
const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

/** Format a byte count as e.g. "512 B", "1.5 MB", "2.3 TB". Non-finite/negative input renders as "0 B". */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNITS.length - 1);
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${BYTE_UNITS[i]}`;
}
