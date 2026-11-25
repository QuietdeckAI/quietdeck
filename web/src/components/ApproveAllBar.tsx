import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Square, Undo2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import type { ApproveAll } from "@/types/ApproveAll";
import type { Approving } from "@/types/Approving";
import type { Inbox } from "@/types/Inbox";
import type { Skipped } from "@/types/Skipped";
import type { StopResult } from "@/types/StopResult";

/** The one button. Then, for the undo window, Undo; while anything runs, Stop. It never counts what's waiting. */
export function ApproveAllBar({
  inbox,
  request,
  beforeApprove,
  top,
  inline,
  onPending,
}: {
  inbox: Inbox;
  request: ApproveAll;
  /** Runs first: the report sends any change typed but not yet sent. */
  beforeApprove?: () => Promise<void>;
  /** Above the report, rather than pinned to the bottom of the screen. */
  top?: boolean;
  /** Just the buttons, compact, for another bar (the report's chat box): the status floats above that bar. */
  inline?: boolean;
  /** Told when an Approve All starts and finishes being sent, so the page can show it's busy from the click. */
  onPending?: (pending: boolean) => void;
}) {
  const qc = useQueryClient();
  const [skipped, setSkipped] = useState<Skipped[]>([]);
  // Awaited in onSuccess, so a button stays busy until the Inbox it changed is back: otherwise Approve All shows
  // again for a moment between the answer and the refreshed Inbox, then turns into Undo.
  const refresh = () => qc.invalidateQueries({ queryKey: ["inbox"] });

  // The batch being undone (or "pending": Undo pressed before the deck answered), so the bar shows Approve All at once.
  const [undoing, setUndoing] = useState<string | null>(null);
  // Undo pressed before the deck answered: undo as soon as the answer names the batch.
  const undoWhenAnswered = useRef(false);
  const approve = useMutation({
    onMutate: () => setUndoing(null),
    mutationFn: async () => {
      undoWhenAnswered.current = false;
      await beforeApprove?.();
      return api<Approving>("/api/decisions/approve-all", { method: "POST", body: request });
    },
    onSuccess: async (a) => {
      setSkipped(a.skipped);
      if (undoWhenAnswered.current && a.cards.length > 0) {
        undoWhenAnswered.current = false;
        setUndoing(a.batch);
        await api("/api/decisions/undo", { method: "POST", body: { batch: a.batch } });
      }
      await refresh();
    },
  });
  useEffect(() => onPending?.(approve.isPending), [approve.isPending, onPending]);
  const undo = useMutation({
    mutationFn: (batch: string) => api("/api/decisions/undo", { method: "POST", body: { batch } }),
    onSuccess: refresh,
  });
  const stop = useMutation({
    mutationFn: () => api<StopResult>("/api/decisions/stop", { method: "POST" }),
    onSuccess: refresh,
  });

  const approving = inbox.approving;
  // Undo shows the moment Approve All is pressed, not after the deck answers, so the button never flashes through a
  // loading state on its way there.
  // And it goes back to Approve All the moment Undo is pressed, for the same reason.
  const undoingThis = undoing !== null && (approving?.batch === undoing || (approving === null && undoing === "pending"));
  const showUndo = (approving !== null || approve.isPending) && !undoingThis;
  const pressUndo = () => {
    undone.current = true;
    if (approving) {
      setUndoing(approving.batch);
      undo.mutate(approving.batch);
    } else {
      setUndoing("pending");
      undoWhenAnswered.current = true;
    }
  };
  // Undo and Approve All share the bar's corner: a tap meant for Undo that lands just after the window ends must
  // not approve whatever is on screen now. So Approve All waits a moment after each window.
  const [settling, setSettling] = useState(false);
  const undone = useRef(false);
  const lastBatch = useRef<string | null>(null);
  useEffect(() => {
    if (approving) {
      lastBatch.current = approving.batch;
      return;
    }
    if (lastBatch.current === null) return;
    lastBatch.current = null;
    // Undone on purpose: no stray tap to guard against.
    if (undone.current) {
      undone.current = false;
      return;
    }
    setSettling(true);
    const t = window.setTimeout(() => setSettling(false), 1500);
    return () => window.clearTimeout(t);
  }, [approving]);
  const nothing = request.cards.length === 0 && request.timeline.length === 0;
  // In the chat bar the button keeps its place, greyed out, so the row doesn't shift when there's nothing to approve.
  if (!inline && nothing && !approving && !inbox.running && skipped.length === 0) return null;
  const error = approve.error ?? undo.error ?? stop.error;

  if (inline) {
    const status = approving && !undoingThis
      ? `Approving ${approving.cards.length === 1 ? "1 card" : `${approving.cards.length} cards`}`
      : skipped.length > 0
        ? `Left alone: ${skipped.map((s) => s.reason).filter((r, i, all) => all.indexOf(r) === i).join("; ")}`
        : null;
    return (
      <>
        {approving && !undoingThis && (
          <div
            key={approving.batch}
            className="drain absolute inset-x-0 top-0 h-px bg-accent"
            style={{ animationDuration: `${Math.max(0, new Date(approving.release_at).getTime() - Date.now())}ms` }}
            aria-hidden
          />
        )}
        {(status || error) && (
          <p
            role={error ? "alert" : "status"}
            className={
              "absolute right-4 bottom-full mb-2 max-w-[calc(100%-2rem)] truncate rounded-md border border-rule " +
              "bg-paper-2 px-2.5 py-1 text-xs " +
              (error ? "text-danger" : "text-muted")
            }
          >
            {error ? error.message : status}
          </p>
        )}
        {showUndo ? (
          <Button
            size="sm"
            variant="secondary"
            className="h-10 min-w-32"
            onClick={pressUndo}
          >
            <Undo2 aria-hidden /> Undo
          </Button>
        ) : (
          // The chat bar's one Stop stops the run too. Undo and Approve All share one width, so swapping them
          // doesn't move the row.
          <Button
            size="sm"
            variant="primary"
            className="h-10 min-w-32"
            disabled={settling || nothing}
            onClick={() => approve.mutate()}
          >
            Approve All
          </Button>
        )}
      </>
    );
  }

  return (
    <div
      className={
        top
          ? "border-b border-rule"
          : "sticky bottom-0 z-10 mt-10 border-t border-rule bg-paper/95 pb-[env(safe-area-inset-bottom)] backdrop-blur-sm"
      }
    >
      {approving && !undoingThis && (
        <div
          key={approving.batch}
          className="drain h-px bg-accent"
          style={{ animationDuration: `${Math.max(0, new Date(approving.release_at).getTime() - Date.now())}ms` }}
          aria-hidden
        />
      )}
      <div className={top ? "flex flex-wrap items-center gap-3 py-3" : "mx-auto flex max-w-5xl flex-wrap items-center gap-3 px-4 py-3"}>
        {showUndo ? (
          <>
            <p className="flex-1 text-sm text-muted" role="status">
              {approving
                ? `Approving ${approving.cards.length === 1 ? "1 card" : `${approving.cards.length} cards`}`
                : "Approving"}
            </p>
            <Button variant="secondary" onClick={pressUndo}>
              <Undo2 aria-hidden /> Undo
            </Button>
          </>
        ) : (
          <>
            <div className="min-w-0 flex-1 text-sm text-neutral" role="status">
              {skipped.length > 0 && (
                <p className="truncate">
                  Left alone: {skipped.map((s) => s.reason).filter((r, i, all) => all.indexOf(r) === i).join("; ")}
                </p>
              )}
            </div>
            {inbox.running && (
              <Button variant="danger" loading={stop.isPending} onClick={() => stop.mutate()}>
                <Square aria-hidden /> Stop
              </Button>
            )}
            {!nothing && (
              <Button
                variant="primary"
                    disabled={settling}
                onClick={() => approve.mutate()}
              >
                Approve All
              </Button>
            )}
          </>
        )}
      </div>
      {error && (
        <p role="alert" className={top ? "pb-3 text-sm text-danger" : "mx-auto max-w-5xl px-4 pb-3 text-sm text-danger"}>
          {error.message}
        </p>
      )}
    </div>
  );
}
