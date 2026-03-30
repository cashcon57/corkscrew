import * as Sentry from "@sentry/browser";
import { getConfig } from "./api";

const SENTRY_DSN =
  "https://de71b88287dbb157e219aff7e1ba2d9c@o4511134300045312.ingest.us.sentry.io/4511134367940608";

let initialized = false;

/**
 * Initialize Sentry if the user has opted in to telemetry.
 * Safe to call multiple times — subsequent calls are no-ops.
 */
export async function initSentry(): Promise<void> {
  if (initialized) return;
  try {
    const cfg = await getConfig();
    const consent = (cfg as Record<string, unknown>).telemetry_consent;
    if (consent !== "granted") return;

    Sentry.init({
      dsn: SENTRY_DSN,
      sendDefaultPii: false,
      sampleRate: 1.0,
      debug: false,
      beforeSend(event) {
        // Strip any PII that might leak through
        delete event.server_name;
        if (event.user) delete event.user;
        console.log("[sentry] sending event:", event.event_id, event.message || event.exception?.values?.[0]?.value);
        return event;
      },
    });
    initialized = true;
    console.log("[sentry] initialized successfully");
  } catch (err) {
    console.error("Sentry init failed:", err);
  }
}

/**
 * Capture an error or message to Sentry (no-op if not initialized).
 */
export function captureError(
  error: Error | string,
  context?: Record<string, string>
): void {
  if (!initialized) {
    console.log("[sentry] captureError called but not initialized — skipping");
    return;
  }
  console.log("[sentry] capturing:", typeof error === "string" ? error : error.message);
  if (typeof error === "string") {
    Sentry.captureMessage(error, { extra: context });
  } else {
    Sentry.captureException(error, { extra: context });
  }
}

/**
 * Tear down Sentry (e.g. when user disables telemetry).
 */
export function teardownSentry(): void {
  if (initialized) {
    Sentry.close();
    initialized = false;
  }
}
