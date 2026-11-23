import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { api } from "@/lib/api";
import { when } from "@/lib/format";
import { useSearch } from "@/lib/route";
import { cn } from "@/lib/utils";
import type { Ping } from "@/types/Ping";
import type { PingsView } from "@/types/PingsView";
import type { ThreadWatch } from "@/types/ThreadWatch";

/** Watched threads: a reply on one may ping. */
export function WatchesSection() {
  const qc = useQueryClient();
  const watches = useQuery({ queryKey: ["watches"], queryFn: () => api<ThreadWatch[]>("/api/watches") });
  const stop = useMutation({
    mutationFn: (id: number) => api(`/api/watches/${id}/stop`, { method: "POST" }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["watches"] }),
  });
  return (
    <section className="mt-10" aria-labelledby="watches">
      <h2 id="watches" className="mb-1 text-lg text-muted">
        Watched threads
      </h2>
      <p className="mb-2 text-sm text-neutral">A reply on one of these may ping your phone.</p>
      {watches.isError && <p className="text-sm text-danger">{watches.error.message}</p>}
      <ul>
        {watches.data?.map((w) => (
          <li key={w.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-rule py-2 last:border-b-0">
            <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">
              <span className="text-ink">{w.subject ?? w.thread_key}</span>
              <span className="text-sm text-neutral">
                {" "}
                · {w.account} · since {when(w.created_at)}
              </span>
            </span>
            <Button
              variant="quiet"
              size="sm"
              loading={stop.isPending && stop.variables === w.id}
              onClick={() => stop.mutate(w.id)}
            >
              Stop watching
            </Button>
          </li>
        ))}
        {watches.data?.length === 0 && (
          <li className="py-2 text-sm text-neutral">No watched threads. Watch one from a card’s menu.</li>
        )}
      </ul>
      {stop.isError && <p className="text-sm text-danger">{stop.error.message}</p>}
    </section>
  );
}

/** Pings against the weekly budget, which holds nothing back, each with "Don't ping for this again". */
export function PingsSection() {
  const highlight = useSearch().get("ping");
  const view = useQuery({ queryKey: ["pings"], queryFn: () => api<PingsView>("/api/pings") });
  const loaded = view.isSuccess;
  useEffect(() => {
    if (loaded && highlight) document.getElementById(`ping-${highlight}`)?.scrollIntoView({ block: "center" });
  }, [loaded, highlight]);
  const v = view.data;
  return (
    <section className="mt-10" aria-labelledby="pings">
      <h2 id="pings" className="mb-1 text-lg text-muted">
        Pings
      </h2>
      {v && (
        <p className="mb-2 text-sm text-neutral">
          {v.last_week} in the last 7 days, against a budget of {v.weekly_budget}.
          {v.last_week > v.weekly_budget && " Over the budget: every ping was still sent."}
        </p>
      )}
      {view.isError && <p className="text-sm text-danger">{view.error.message}</p>}
      <ul>
        {v?.pings.map((p) => (
          <PingRow key={p.id} ping={p} highlighted={String(p.id) === highlight} />
        ))}
        {v?.pings.length === 0 && (
          <li className="py-2 text-sm text-neutral">
            No pings yet. An agent pings only for what can’t wait a few hours.
          </li>
        )}
      </ul>
    </section>
  );
}

function PingRow({ ping: p, highlighted }: { ping: Ping; highlighted: boolean }) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [note, setNote] = useState(p.feedback ?? "");
  const save = useMutation({
    mutationFn: () => api<Ping>(`/api/pings/${p.id}/feedback`, { method: "POST", body: { note: note.trim() || null } }),
    onSuccess: () => {
      setEditing(false);
      void qc.invalidateQueries({ queryKey: ["pings"] });
    },
  });
  const link = p.link && /^https?:\/\//.test(p.link) ? p.link : null;
  return (
    <li
      id={`ping-${p.id}`}
      className={cn(
        "flex flex-col gap-1.5 border-b border-rule py-3 last:border-b-0",
        highlighted && "-mx-3 rounded-md border-b-0 bg-paper-2 px-3",
      )}
    >
      <p className="flex flex-wrap items-baseline justify-between gap-x-3">
        <span className="text-ink">{p.title}</span>
        <span className="text-sm text-neutral">
          {p.sent_by.tab} · {when(p.sent_at)}
        </span>
      </p>
      <p className="text-muted [overflow-wrap:anywhere]">{p.body}</p>
      <p className="text-sm text-neutral [overflow-wrap:anywhere]">Why now: {p.reason}</p>
      {p.link && (
        <p className="text-sm [overflow-wrap:anywhere]">
          {link ? (
            <a href={link} target="_blank" rel="noreferrer noopener" className="text-accent hover:underline">
              {link}
            </a>
          ) : (
            <span className="text-muted">{p.link}</span>
          )}
        </p>
      )}
      {p.result && <p className="text-sm text-neutral">{p.result}</p>}
      {editing ? (
        <form
          className="flex flex-col gap-2 pt-1"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate();
          }}
        >
          <label className="flex flex-col gap-1 text-sm text-muted">
            Anything {p.sent_by.tab} should know? (optional)
            <Textarea value={note} onChange={(e) => setNote(e.target.value)} rows={2} maxLength={2000} />
          </label>
          <div className="flex gap-2">
            <Button type="submit" size="sm" loading={save.isPending}>
              Don’t ping for this again
            </Button>
            <Button type="button" variant="quiet" size="sm" onClick={() => setEditing(false)}>
              Cancel
            </Button>
          </div>
          {save.isError && <p className="text-sm text-danger">{save.error.message}</p>}
        </form>
      ) : p.feedback_at ? (
        <div className="flex flex-wrap items-baseline gap-x-3 text-sm">
          <span className="text-muted">
            You said don’t ping for this again{p.feedback ? `: “${p.feedback}”` : "."}{" "}
            {p.feedback_picked_up_at
              ? `${p.sent_by.tab} picked it up ${when(p.feedback_picked_up_at)}.`
              : `${p.sent_by.tab} hasn’t picked it up yet.`}
          </span>
          <Button variant="quiet" size="sm" onClick={() => setEditing(true)}>
            Change
          </Button>
        </div>
      ) : (
        <div>
          <Button variant="secondary" size="sm" onClick={() => setEditing(true)}>
            Don’t ping for this again
          </Button>
        </div>
      )}
    </li>
  );
}
