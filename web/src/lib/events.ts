import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type { Changed } from "@/types/Changed";

/** Which queries each kind of change makes stale. */
const STALE: Record<Changed, string[][]> = {
  cards: [["inbox"], ["later"], ["history"], ["mail-badges"]],
  timeline: [["inbox"], ["history"]],
  sessions: [["sessions"]],
  settings: [["settings"], ["info"]],
  nodes: [["nodes"], ["hooks"]],
  rules: [["rules"], ["reading-rules"], ["inbox"]],
  leases: [["leases"]],
  pings: [["pings"], ["watches"]],
  push: [["push"]],
  mail: [["mail"], ["mail-badges"]],
  all: [[]],
};

/**
 * Keeps every open screen current from the deck's event stream. EventSource reconnects by itself after a dropped
 * connection, but not after an HTTP error, which is what a proxy answers while the deck restarts for a deploy: then it
 * closes for good and the page stops updating. So a closed stream is opened again, after 1, 2, 4… up to 30 seconds,
 * and everything is refetched once it's back.
 */
export function useDeckEvents(enabled: boolean) {
  const qc = useQueryClient();
  useEffect(() => {
    if (!enabled) return;
    let source: EventSource | null = null;
    let retry: number | undefined;
    let delay = 1000;
    let stopped = false;
    const connect = () => {
      source = new EventSource("/api/events");
      source.addEventListener("changed", (e) => {
        const what = JSON.parse((e as MessageEvent<string>).data) as Changed;
        for (const key of STALE[what] ?? [[]]) void qc.invalidateQueries({ queryKey: key });
      });
      // After a reconnect, anything may have changed while the stream was down.
      source.addEventListener("open", () => {
        delay = 1000;
        void qc.invalidateQueries();
      });
      source.addEventListener("error", () => {
        if (stopped || source?.readyState !== EventSource.CLOSED) return;
        source = null;
        retry = window.setTimeout(connect, delay);
        delay = Math.min(delay * 2, 30_000);
      });
    };
    connect();
    return () => {
      stopped = true;
      window.clearTimeout(retry);
      source?.close();
    };
  }, [enabled, qc]);
}
