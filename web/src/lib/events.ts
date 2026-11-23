import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type { Changed } from "@/types/Changed";

/** Which queries each kind of change makes stale. */
const STALE: Record<Changed, string[][]> = {
  cards: [["inbox"], ["later"], ["history"]],
  timeline: [["inbox"], ["history"]],
  sessions: [["sessions"]],
  settings: [["settings"], ["info"]],
  nodes: [["nodes"]],
  rules: [["rules"], ["inbox"]],
  leases: [["leases"]],
  pings: [["pings"], ["watches"]],
  push: [["push"]],
  all: [[]],
};

/** Keeps every open screen current from the deck's event stream. EventSource reconnects on its own. */
export function useDeckEvents(enabled: boolean) {
  const qc = useQueryClient();
  useEffect(() => {
    if (!enabled) return;
    const source = new EventSource("/api/events");
    source.addEventListener("changed", (e) => {
      const what = JSON.parse((e as MessageEvent<string>).data) as Changed;
      for (const key of STALE[what] ?? [[]])
        void qc.invalidateQueries({ queryKey: key });
    });
    // After a reconnect, anything may have changed while the stream was down.
    source.addEventListener("open", () => void qc.invalidateQueries());
    return () => source.close();
  }, [enabled, qc]);
}
