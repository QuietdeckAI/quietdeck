import { useMutation } from "@tanstack/react-query";
import { X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { agoShort, stampWithZone } from "@/lib/format";
import type { Presence } from "@/types/Presence";
import type { Report } from "@/types/Report";
import type { ReportChange } from "@/types/ReportChange";

/** Re-renders every `ms`, for times that count. */
function useNow(ms: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(t);
  }, [ms]);
  return now;
}

/** "Updated 2 min ago" and the exact time in the person's zone ("Wed 25 Nov, 12:07pm NZDT"), ticking. */
export function UpdatedAt({ at, stacked }: { at: string; stacked?: boolean }) {
  const now = useNow(15_000);
  return (
    <p className="text-xs leading-snug text-neutral">
      Updated {agoShort(at, now)}
      {stacked ? <br /> : " "}
      <span className="text-neutral/80">{stacked ? stampWithZone(at) : `(${stampWithZone(at)})`}</span>
    </p>
  );
}

/** On screens too narrow for the bar's corner, the "Updated" line sits above the report instead. */
export function ReportMeta({ report }: { report: Report }) {
  return (
    <div className="xl:hidden">
      <UpdatedAt at={report.updated_at} />
    </div>
  );
}

/**
 * While the person reads: a toast that stays in the top right until what's held comes in, saying what waits, with
 * Show now.
 */
function HeldToast({ presence }: { presence: Presence }) {
  const release = useMutation({ mutationFn: () => api("/api/presence/release", { method: "POST" }) });
  return (
    <div
      role="status"
      className="toast-in pointer-events-auto rounded-lg border border-rule-strong bg-paper-3 py-2.5 pr-2.5 pl-3.5 text-sm shadow-lg shadow-black/30"
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-ink">
            {presence.held_count === 1 ? "1 new item is waiting" : `${presence.held_count} new items are waiting`}
          </p>
          <p className="mt-0.5 text-xs text-neutral">
            {presence.held ? `${presence.held}. ` : ""}They come in when you stop reading.
          </p>
        </div>
        <Button
          size="sm"
          variant="secondary"
          className="shrink-0"
          loading={release.isPending}
          disabled={release.isSuccess}
          onClick={() => release.mutate()}
        >
          {release.isSuccess ? "Coming in" : "Show now"}
        </Button>
      </div>
    </div>
  );
}

type Toast = ReportChange & { shownAt: number };

// Long enough to notice and read while working elsewhere on the page; the ✕ dismisses one sooner.
const TOAST_MS = 60_000;

/** Scroll to the first default naming one of these cards, and ring it briefly. */
function pointAt(cards: number[]) {
  for (const id of cards) {
    const el = document.querySelector<HTMLElement>(`[data-card-ids~="${id}"]`);
    if (!el) continue;
    el.scrollIntoView({ behavior: "smooth", block: "center" });
    el.classList.remove("flash");
    void el.offsetWidth;
    el.classList.add("flash");
    window.setTimeout(() => el.classList.remove("flash"), 2600);
    return;
  }
}

/**
 * Every change to the report while the page is open, as a toast in the bottom right saying what changed. Pressing
 * one scrolls to the default it's about. Changes from before the page opened aren't toasted.
 */
export function ReportToasts({ report, presence }: { report: Report; presence: Presence }) {
  const seen = useRef<Set<number> | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  useEffect(() => {
    const ids = report.changes.map((c) => Number(c.id));
    if (seen.current === null) {
      seen.current = new Set(ids);
      return;
    }
    const fresh = report.changes.filter((c) => !seen.current?.has(Number(c.id)));
    if (fresh.length === 0) return;
    for (const c of fresh) seen.current.add(Number(c.id));
    const at = Date.now();
    setToasts((all) => [...all, ...fresh.map((c) => ({ ...c, shownAt: at }))].slice(-4));
  }, [report.changes]);
  useEffect(() => {
    if (toasts.length === 0) return;
    const next = Math.min(...toasts.map((t) => t.shownAt + TOAST_MS)) - Date.now();
    const timer = window.setTimeout(
      () => setToasts((all) => all.filter((t) => t.shownAt + TOAST_MS > Date.now())),
      Math.max(0, next),
    );
    return () => window.clearTimeout(timer);
  }, [toasts]);
  const dismiss = (id: number) => setToasts((all) => all.filter((t) => Number(t.id) !== id));
  const held = presence.held_count > 0;
  return (
    <>
      {held && (
        <div className="pointer-events-none fixed top-16 right-4 z-30 w-[min(24rem,calc(100vw-2rem))]">
          <HeldToast presence={presence} />
        </div>
      )}
      {toasts.length > 0 && (
        <ol
          aria-live="polite"
          className={
            // Above the chat bar on a wide screen; under the header on a phone (below the waiting toast), where the bar is
            // taller.
            "pointer-events-none fixed right-4 z-30 flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2 " +
            (held ? "top-40 " : "top-16 ") +
            "sm:top-auto sm:bottom-24"
          }
        >
          {toasts.map((t) => (
            <li
              key={String(t.id)}
              className="toast-in pointer-events-auto flex items-start gap-2 rounded-lg border border-rule-strong bg-paper-3 py-2.5 pr-2 pl-3.5 text-sm shadow-lg shadow-black/30"
            >
              <button
                type="button"
                className="min-w-0 flex-1 text-left"
                onClick={() => {
                  pointAt(t.cards.map(Number));
                  dismiss(Number(t.id));
                }}
              >
                <span className="block text-xs text-neutral">Inbox changed · {agoShort(t.at)}</span>
                <span className="mt-0.5 block text-ink">{t.summary}</span>
              </button>
              <button
                type="button"
                aria-label="Dismiss"
                onClick={() => dismiss(Number(t.id))}
                className="rounded p-1 text-neutral hover:text-ink"
              >
                <X className="size-4" aria-hidden />
              </button>
            </li>
          ))}
        </ol>
      )}
    </>
  );
}

/**
 * When Approve All may go: 10 seconds after the newest change to anything it would approve (a card's new version, or
 * a change to its default on the report). The deck checks the same.
 */
export function settleUntil(report: Report, cards: { id: number; updated_at: string }[]): number {
  const ids = new Set(cards.map((c) => Number(c.id)));
  const times = cards.map((c) => new Date(c.updated_at).getTime());
  for (const ch of report.changes) {
    if (ch.cards.some((id) => ids.has(Number(id)))) times.push(new Date(ch.at).getTime());
  }
  return times.length ? Math.max(...times) + SETTLE_MS : 0;
}

export const SETTLE_MS = 10_000;
