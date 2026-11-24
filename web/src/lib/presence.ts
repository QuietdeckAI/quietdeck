import { useEffect } from "react";

/** How often activity is told to the deck at most. The deck counts the person idle two minutes after the last one. */
const BEAT_MS = 15_000;

/** One ID per tab, so one tab going quiet doesn't end another's reading. */
function deviceId(): string {
  const key = "quietdeck.presence.device";
  let id = sessionStorage.getItem(key);
  if (!id) {
    // Not crypto.randomUUID: it exists only on HTTPS pages, and the dev server is plain HTTP.
    id = Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, "0")).join("");
    sessionStorage.setItem(key, id);
  }
  return id;
}

function send(device: string, active: boolean) {
  // keepalive: the "gone" beat must survive the tab closing.
  void fetch("/api/presence", {
    method: "POST",
    credentials: "same-origin",
    keepalive: true,
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ device, active }),
  }).catch(() => undefined);
}

// Kept across mounts: React mounts the Inbox twice in development, and a quick remount (the Inbox's own re-render,
// or navigating back to it) mustn't look like leaving and coming back.
let lastBeat = 0;
let goneTimer: number | undefined;

/**
 * While this page is open and the person is doing something on it (moving the mouse, scrolling, typing, touching),
 * tell the deck at most every 15 seconds, so the triage agent holds new mail instead of changing the page under them.
 * Hiding the tab or leaving the page says so: hiding at once, leaving after a moment in case the page comes straight
 * back.
 */
export function usePresence() {
  useEffect(() => {
    const device = deviceId();
    window.clearTimeout(goneTimer);
    const activity = () => {
      if (document.visibilityState !== "visible") return;
      const now = Date.now();
      if (now - lastBeat < BEAT_MS) return;
      lastBeat = now;
      send(device, true);
    };
    const gone = () => {
      if (lastBeat === 0) return;
      lastBeat = 0;
      send(device, false);
    };
    const visibility = () => (document.visibilityState === "visible" ? activity() : gone());
    const events = ["pointermove", "pointerdown", "keydown", "wheel", "touchstart"] as const;
    for (const e of events) window.addEventListener(e, activity, { passive: true });
    window.addEventListener("scroll", activity, { passive: true, capture: true });
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pagehide", gone);
    activity();
    return () => {
      for (const e of events) window.removeEventListener(e, activity);
      window.removeEventListener("scroll", activity, { capture: true });
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("pagehide", gone);
      goneTimer = window.setTimeout(gone, 1000);
    };
  }, []);
}
