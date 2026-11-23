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
export function ApproveAllBar({ inbox, request }: { inbox: Inbox; request: ApproveAll }) {
  const qc = useQueryClient();
  const [skipped, setSkipped] = useState<Skipped[]>([]);
  const refresh = () => void qc.invalidateQueries({ queryKey: ["inbox"] });

  const approve = useMutation({
    mutationFn: () => api<Approving>("/api/decisions/approve-all", { method: "POST", body: request }),
    onSuccess: (a) => {
      setSkipped(a.skipped);
      refresh();
    },
  });
  const undo = useMutation({
    mutationFn: (batch: string) => api("/api/decisions/undo", { method: "POST", body: { batch } }),
    onSuccess: refresh,
  });
  const stop = useMutation({
    mutationFn: () => api<StopResult>("/api/decisions/stop", { method: "POST" }),
    onSuccess: refresh,
  });

  const approving = inbox.approving;
  // Undo and Approve All share the bar's corner: a tap meant for Undo that lands just after the window ends must
  // not approve whatever is on screen now. So Approve All waits a moment after each window.
  const [settling, setSettling] = useState(false);
  const lastBatch = useRef<string | null>(null);
  useEffect(() => {
    if (approving) {
      lastBatch.current = approving.batch;
      return;
    }
    if (lastBatch.current === null) return;
    lastBatch.current = null;
    setSettling(true);
    const t = window.setTimeout(() => setSettling(false), 1500);
    return () => window.clearTimeout(t);
  }, [approving]);
  const nothing = request.cards.length === 0 && request.timeline.length === 0;
  if (nothing && !approving && !inbox.running && skipped.length === 0) return null;
  const error = approve.error ?? undo.error ?? stop.error;

  return (
    <div className="sticky bottom-0 z-10 mt-10 border-t border-rule bg-paper/95 pb-[env(safe-area-inset-bottom)] backdrop-blur-sm">
      {approving && (
        <div
          key={approving.batch}
          className="drain h-px bg-accent"
          style={{ animationDuration: `${Math.max(0, new Date(approving.release_at).getTime() - Date.now())}ms` }}
          aria-hidden
        />
      )}
      <div className="mx-auto flex max-w-5xl flex-wrap items-center gap-3 px-4 py-3">
        {approving ? (
          <>
            <p className="flex-1 text-sm text-muted" role="status">
              Approving {approving.cards.length === 1 ? "1 card" : `${approving.cards.length} cards`}
            </p>
            <Button variant="secondary" loading={undo.isPending} onClick={() => undo.mutate(approving.batch)}>
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
                loading={approve.isPending}
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
        <p role="alert" className="mx-auto max-w-5xl px-4 pb-3 text-sm text-danger">
          {error.message}
        </p>
      )}
    </div>
  );
}
