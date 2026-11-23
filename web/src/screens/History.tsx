import { useQuery } from "@tanstack/react-query";
import { AccountTabs, useAccount } from "@/components/AccountTabs";
import { actionLine, sessionName } from "@/components/actionText";
import { Button } from "@/components/ui/button";
import { api, query } from "@/lib/api";
import { dayHeading, dayKey, when } from "@/lib/format";
import { setParam, useSearch } from "@/lib/route";
import { cn } from "@/lib/utils";
import type { Card } from "@/types/Card";
import type { History } from "@/types/History";
import type { TimelineEntry } from "@/types/TimelineEntry";

type Row = { at: string; card?: Card; entry?: TimelineEntry };

/** By day: finished cards with each action's outcome and who ran it, and the automatic actions he's seen. */
export function HistoryScreen() {
  const account = useAccount();
  const until = useSearch().get("until") ?? undefined;
  const history = useQuery({
    queryKey: ["history", account ?? null, until ?? null],
    queryFn: () => api<History>(`/api/history${query({ account, until })}`),
  });

  const rows: Row[] = [
    ...(history.data?.cards ?? []).map((c) => ({ at: c.updated_at, card: c })),
    ...(history.data?.entries ?? []).map((e) => ({ at: e.created_at, entry: e })),
  ].sort((a, b) => b.at.localeCompare(a.at));
  const days: { key: string; heading: string; rows: Row[] }[] = [];
  for (const r of rows) {
    const k = dayKey(r.at);
    if (days.at(-1)?.key !== k) days.push({ key: k, heading: dayHeading(r.at), rows: [] });
    days.at(-1)?.rows.push(r);
  }

  return (
    <main className="mx-auto max-w-3xl px-4 py-6">
      <AccountTabs />
      {history.isError && <p className="mt-6 text-danger">{history.error.message}</p>}
      {history.data && days.length === 0 && <p className="mt-10 text-neutral">Nothing happened in this week.</p>}
      {days.map((d) => (
        <section key={d.key} className="mt-8">
          <h2 className="mb-2 text-lg text-muted">{d.heading}</h2>
          <ul>
            {d.rows.map((r) => (r.card ? <CardRow key={`c${r.card.id}`} card={r.card} /> : r.entry && <EntryRow key={`e${r.entry.id}`} entry={r.entry} />))}
          </ul>
        </section>
      ))}
      {history.data && (
        <div className="mt-10 flex justify-center gap-2">
          {until && (
            <Button variant="quiet" onClick={() => setParam("until", undefined)}>
              This week
            </Button>
          )}
          <Button variant="quiet" onClick={() => setParam("until", history.data.since)}>
            The week before
          </Button>
        </div>
      )}
    </main>
  );
}

const STATE_WORD: Partial<Record<Card["state"], string>> = {
  done: "Done",
  failed: "Failed",
  handled_elsewhere: "Handled elsewhere",
  automatic: "Automatic",
};

function CardRow({ card }: { card: Card }) {
  return (
    <li className="border-b border-rule py-3 last:border-b-0">
      <div className="flex items-baseline gap-3">
        <time className="w-14 shrink-0 font-mono text-xs text-neutral">{when(card.updated_at).split(" ").at(-1)}</time>
        <div className="min-w-0 flex-1">
          <p className="truncate">
            <span className="text-muted">{card.sender?.name ?? card.sender?.address ?? card.account}</span>
            <span className="text-neutral"> · </span>
            {card.subject}
          </p>
          <ul className="mt-1 flex flex-col gap-0.5 text-sm">
            {card.action_records.map((a) => (
              <li key={a.id} className={cn(a.status === "failed" ? "text-danger" : "text-neutral")}>
                {actionLine(a.action).verb}
                <span> · {a.status}</span>
                {a.run_by && <span> by {a.run_by === "runner" ? "runner" : card.posted_by.tab}</span>}
                {a.error && <span>: {a.error}</span>}
              </li>
            ))}
            {card.recheck_note && <li className="text-neutral">{card.recheck_note}</li>}
          </ul>
        </div>
        <span className="shrink-0 text-xs text-neutral">{STATE_WORD[card.state] ?? card.state}</span>
      </div>
      <p className="mt-1 pl-17 text-xs text-neutral">from {sessionName(card.posted_by)}</p>
    </li>
  );
}

function EntryRow({ entry }: { entry: TimelineEntry }) {
  return (
    <li className="flex items-baseline gap-3 border-b border-rule py-2 text-sm last:border-b-0">
      <time className="w-14 shrink-0 font-mono text-xs text-neutral">{when(entry.created_at).split(" ").at(-1)}</time>
      <span className="min-w-0 flex-1 text-muted">
        {entry.text}
        {entry.count > 1 && <span className="text-neutral"> ×{entry.count}</span>}
      </span>
      <span className="shrink-0 text-xs text-neutral">{entry.source === "agent" ? "triage" : entry.source}</span>
    </li>
  );
}
