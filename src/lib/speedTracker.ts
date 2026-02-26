/**
 * Rolling-window speed calculator for tracking throughput.
 * Used by both collection and WJ install progress tracking.
 */
export class SpeedTracker {
  private samples: { time: number; bytes: number }[] = [];
  private windowMs: number;

  constructor(windowMs = 5000) {
    this.windowMs = windowMs;
  }

  /** Feed current cumulative bytes, returns speed in bytes/sec */
  update(currentBytes: number): number {
    const now = Date.now();
    this.samples.push({ time: now, bytes: currentBytes });
    this.samples = this.samples.filter((s) => now - s.time <= this.windowMs);
    if (this.samples.length < 2) return 0;
    const oldest = this.samples[0];
    const elapsed = (now - oldest.time) / 1000;
    if (elapsed <= 0) return 0;
    return (currentBytes - oldest.bytes) / elapsed;
  }

  reset(): void {
    this.samples = [];
  }

  static formatSpeed(bytesPerSec: number): string {
    if (bytesPerSec <= 0) return "";
    if (bytesPerSec < 1024) return `${bytesPerSec.toFixed(0)} B/s`;
    if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
    if (bytesPerSec < 1024 * 1024 * 1024) return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`;
    return `${(bytesPerSec / (1024 * 1024 * 1024)).toFixed(2)} GB/s`;
  }

  static formatEta(remainingBytes: number, speed: number): string {
    if (speed <= 0) return "";
    const secs = remainingBytes / speed;
    if (secs < 60) return "< 1 min";
    if (secs < 3600) return `~${Math.ceil(secs / 60)} min`;
    const hrs = Math.floor(secs / 3600);
    const mins = Math.ceil((secs % 3600) / 60);
    return `~${hrs}h ${mins}m`;
  }

  static formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }

  static formatElapsed(startTime: number): string {
    const secs = Math.floor((Date.now() - startTime) / 1000);
    if (secs < 60) return `${secs}s`;
    return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  }
}
