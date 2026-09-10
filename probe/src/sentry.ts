// Thin slice of paritytech/bulletin-deploy/src/telemetry.ts.
//
// Mirrors the PII-scrub + graceful-flush behaviour from the upstream owner of
// the deploy.* span schema. When bulletin-deploy adds "./telemetry" to its
// package exports, replace this file with direct imports from
// "bulletin-deploy/telemetry".

import * as Sentry from "@sentry/node";

const USER_PATH_RE  = /\/Users\/[^/\s"'`]+/g;
const LINUX_HOME_RE = /\/home\/[^/\s"'`]+/g;

export function scrubPaths(msg: string): string {
  if (!msg) return msg;
  return msg
    .replace(USER_PATH_RE, "/Users/<redacted>")
    .replace(LINUX_HOME_RE, "/home/<redacted>");
}

export interface InitOptions {
  dsn: string;
  release: string;
  environment: string;
}

export function initSentry(opts: InitOptions): void {
  Sentry.init({
    dsn: opts.dsn,
    release: opts.release,
    environment: opts.environment,
    tracesSampleRate: 1.0,
    // Anonymise the default os.hostname() server_name so personal machine
    // names don't surface in events.
    serverName: process.env.CI ? (process.env.RUNNER_NAME ?? "ci") : "local",
    beforeSend(event) {
      if (event.server_name) event.server_name = process.env.CI ? (process.env.RUNNER_NAME ?? "ci") : "local";
      if (event.message) event.message = scrubPaths(event.message);
      for (const ex of event.exception?.values ?? []) {
        if (ex.value) ex.value = scrubPaths(ex.value);
      }
      for (const bc of event.breadcrumbs ?? []) {
        if (bc.message) bc.message = scrubPaths(bc.message);
      }
      return event;
    },
    beforeSendTransaction(event) {
      for (const span of event.spans ?? []) {
        const attrs = span.data;
        if (!attrs) continue;
        for (const k of Object.keys(attrs)) {
          const v = attrs[k];
          if (typeof v === "string") attrs[k] = scrubPaths(v);
        }
      }
      return event;
    },
  });
}

export async function closeSentry(timeoutMs: number): Promise<void> {
  try {
    await Sentry.close(timeoutMs);
  } catch {
    // Best-effort: about to exit anyway.
  }
}
