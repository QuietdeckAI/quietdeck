import { useQuery } from "@tanstack/react-query";
import { createContext, useContext } from "react";
import { AccountTabs, useAccount } from "@/components/AccountTabs";
import { actionLine, sessionName } from "@/components/actionText";
import { CommentBox } from "@/components/CommentBox";
import { Link } from "@/components/Header";
import { Button } from "@/components/ui/button";
import { api, query } from "@/lib/api";
import { clock, dayHeading, dayKey } from "@/lib/format";
import { setParam, useSearch } from "@/lib/route";
import { cn } from "@/lib/utils";
import type { Card } from "@/types/Card";
import type { History } from "@/types/History";
import type { ReportSnapshotSummary } from "@/types/ReportSnapshotSummary";
import type { Comment as CommentT } from "@/types/Comment";
import type { CommentTarget } from "@/types/CommentTarget";
import type { TimelineEntry } from "@/types/TimelineEntry";

type Row = { at: string; card?: Card; entry?: TimelineEntry; snapshot?: ReportSnapshotSummary };

const CommentsContext = createContext<CommentT[]>([]);

/** His comments on a row, under it. */
function Comments({ on }: { on: CommentTarget }) {
  const all = useContext(CommentsContext).filter(
    (c) => c.target.type === on.type && c.target.id === on.id,
  );
  if (all.length === 0) return null;
  return (
    <ul className="mt-1 flex flex-col gap-0.5 text-sm">
      {all.map((c) => (
        <li key={c.id} className="text-muted">
          You: “{c.text}”
          {c.to && <span className="text-neutral"> → {c.to.tab}</span>}
        </li>
      ))}
    </ul>
  );
}

/** History's two views: what was done (cards and automatic actions), and the Inbox as it was at each Approve All. */
function ViewTabs({ view }: { view: "actions" | "inbox" }) {
  const tabs = [
    { id: "actions", label: "Actions" },
    { id: "inbox", label: "Inbox" },
  ] as const;
  return (
    <div role="tablist" aria-label="History" className="mb-4 flex gap-6 border-b border-rule">
      {tabs.map((t) => {
        const selected = t.id === view;
        return (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={selected}
            onClick={() => setParam("view", t.id === "actions" ? undefined : t.id)}
            className={cn(
              "-mb-px border-b-2 pb-2 text-base transition-colors duration-(--dur-quick)",
              selected ? "border-accent text-ink" : "border-transparent text-neutral hover:text-ink",
            )}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

/** By day: finished cards with each action's outcome and who ran it, and the automatic actions he's seen; or, on the
 * Inbox tab, the Inbox document as it was at each Approve All. */
export function HistoryScreen() {
  const search = useSearch();
  const view = search.get("view") === "inbox" ? "inbox" : "actions";
  // The Inbox document spans every account, so its tab has no account filter.
  const accountParam = useAccount();
  const account = view === "inbox" ? undefined : accountParam;
  const until = search.get("until") ?? undefined;
  const history = useQuery({
    queryKey: ["history", account ?? null, until ?? null],
    queryFn: () => api<History>(`/api/history${query({ account, until })}`),
  });

  const rows: Row[] = (
    view === "inbox"
      ? (history.data?.snapshots ?? []).map((s) => ({ at: s.taken_at, snapshot: s }))
      : [
          ...(history.data?.cards ?? []).map((c) => ({ at: c.updated_at, card: c })),
          ...(history.data?.entries ?? []).map((e) => ({ at: e.created_at, entry: e })),
        ]
  ).sort((a, b) => b.at.localeCompare(a.at));
  const days: { key: string; heading: string; rows: Row[] }[] = [];
  for (const r of rows) {
    const k = dayKey(r.at);
    if (days.at(-1)?.key !== k)
      days.push({ key: k, heading: dayHeading(r.at), rows: [] });
    days.at(-1)?.rows.push(r);
  }

  return (
    <CommentsContext.Provider value={history.data?.comments ?? []}>
      <main className="mx-auto max-w-3xl px-4 py-6">
        <ViewTabs view={view} />
        {view === "actions" && <AccountTabs />}
        {history.isError && (
          <p className="mt-6 text-danger">{history.error.message}</p>
        )}
        {history.data && days.length === 0 && (
          <p className="mt-10 text-neutral">
            {view === "inbox" ? "No Approve All this week." : "Nothing happened in this week."}
          </p>
        )}
        {days.map((d) => (
          <section key={d.key} className="mt-8">
            <h2 className="mb-2 text-lg text-muted">{d.heading}</h2>
            <ul>
              {d.rows.map((r) =>
                r.card ? (
                  <CardRow key={`c${r.card.id}`} card={r.card} />
                ) : r.snapshot ? (
                  <SnapshotRow key={`s${r.snapshot.id}`} snapshot={r.snapshot} />
                ) : (
                  r.entry && <EntryRow key={`e${r.entry.id}`} entry={r.entry} />
                ),
              )}
            </ul>
          </section>
        ))}
        {history.data && (
          <div className="mt-10 flex justify-center gap-2">
            {until && (
              <Button
                variant="quiet"
                onClick={() => setParam("until", undefined)}
              >
                This week
              </Button>
            )}
            <Button
              variant="quiet"
              onClick={() => setParam("until", history.data.since)}
            >
              The week before
            </Button>
          </div>
        )}
      </main>
    </CommentsContext.Provider>
  );
}

const STATE_WORD: Partial<Record<Card["state"], string>> = {
  done: "Done",
  failed: "Failed",
  handled_elsewhere: "Handled elsewhere",
  automatic: "Automatic",
};

/** The card's action records, identical ones (same action, status and who ran it, no error) merged with a count. */
function groupRecords(card: Card): { a: Card["action_records"][number]; n: number }[] {
  const groups: { key: string; a: Card["action_records"][number]; n: number }[] = [];
  for (const a of card.action_records) {
    const key = a.error ? `error:${a.id}` : `${actionLine(a.action).verb}\u0000${a.status}\u0000${a.run_by ?? ""}`;
    const g = groups.find((x) => x.key === key);
    if (g) g.n += 1;
    else groups.push({ key, a, n: 1 });
  }
  return groups;
}

function CardRow({ card }: { card: Card }) {
  return (
    <li className="border-b border-rule py-3 last:border-b-0">
      <div className="flex items-baseline gap-3">
        <time className="w-16 shrink-0 font-mono text-xs text-neutral">
          {clock(card.updated_at)}
        </time>
        <div className="min-w-0 flex-1">
          <p className="truncate">
            <span className="text-muted">
              {card.sender?.name ?? card.sender?.address ?? card.account}
            </span>
            <span className="text-neutral"> · </span>
            {card.subject}
          </p>
          <ul className="mt-1 flex flex-col gap-0.5 text-sm">
            {/* One line per kind of action, with how many: a card that archives 12 emails says so once, not 12
                times. Failures keep a line each, with their error. */}
            {groupRecords(card).map(({ a, n }) => (
              <li key={a.id} className={cn(a.status === "failed" ? "text-danger" : "text-neutral")}>
                {actionLine(a.action).verb}
                {n > 1 && <span> · {n} emails</span>}
                <span> · {a.status}</span>
                {a.run_by && <span> by {a.run_by === "runner" ? "runner" : card.posted_by.tab}</span>}
                {a.error && <span>: {a.error}</span>}
              </li>
            ))}
            {card.recheck_note && (
              <li className="text-neutral">{card.recheck_note}</li>
            )}
          </ul>
        </div>
        <span className="shrink-0 text-xs text-neutral">
          {STATE_WORD[card.state] ?? card.state}
        </span>
      </div>
      <p className="mt-1 pl-17 text-xs text-neutral">
        from {sessionName(card.posted_by)} ·{" "}
        <CommentBox
          target={{ type: "card", id: card.id }}
          label={card.subject}
        />
      </p>
      <Comments on={{ type: "card", id: card.id }} />
    </li>
  );
}

/** An Approve All: a link to the Inbox document as it was then. */
function SnapshotRow({ snapshot }: { snapshot: ReportSnapshotSummary }) {
  const changes = snapshot.answers === 0 ? "" : ` · ${snapshot.answers} ${snapshot.answers === 1 ? "change or answer" : "changes and answers"}`;
  return (
    <li className="flex items-baseline gap-3 border-b border-rule py-2 text-sm last:border-b-0">
      <time className="w-16 shrink-0 font-mono text-xs text-neutral">{clock(snapshot.taken_at)}</time>
      <span className="min-w-0 flex-1">
        <Link to={`/history/inbox/${snapshot.id}`} className="text-ink underline underline-offset-4 hover:text-accent">
          The Inbox at Approve All
        </Link>
        <span className="text-neutral">
          {" "}
          · {snapshot.approved} {snapshot.approved === 1 ? "card" : "cards"}
          {changes}
        </span>
      </span>
    </li>
  );
}

function EntryRow({ entry }: { entry: TimelineEntry }) {
  return (
    <li className="flex items-baseline gap-3 border-b border-rule py-2 text-sm last:border-b-0">
      <time className="w-16 shrink-0 font-mono text-xs text-neutral">
        {clock(entry.created_at)}
      </time>
      <span className="min-w-0 flex-1 text-muted">
        {entry.text}
        {entry.count > 1 && (
          <span className="text-neutral"> ×{entry.count}</span>
        )}
        <span className="text-xs text-neutral"> · </span>
        <CommentBox
          target={{ type: "timeline", id: entry.id }}
          label={entry.text}
        />
        <Comments on={{ type: "timeline", id: entry.id }} />
      </span>
      <span className="shrink-0 text-xs text-neutral">
        {entry.source === "agent" ? "triage" : entry.source}
      </span>
    </li>
  );
}
