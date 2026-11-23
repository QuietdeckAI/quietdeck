import { useQuery } from "@tanstack/react-query";
import { Search } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, query } from "@/lib/api";
import { when } from "@/lib/format";
import { navigate, setParam, useSearch } from "@/lib/route";
import { cn } from "@/lib/utils";
import type { SessionLink } from "@/types/SessionLink";
import type { SessionRow } from "@/types/SessionRow";
import type { WhoHas } from "@/types/WhoHas";

/** The deck's address for one session: its ID on its machine. */
export function sessionKey(node: string, sessionId: string) {
  return `${sessionId}@${node}`;
}

export function sessionPath(node: string, sessionId: string) {
  return `/sessions/${encodeURIComponent(node)}/${encodeURIComponent(sessionId)}`;
}

/** The name to show: its tab, or for a session without one (imported history), its project or directory. */
export function sessionTitle(s: SessionRow): string {
  return s.tab ?? s.project ?? s.cwd?.split("/").filter(Boolean).at(-1) ?? "Untitled session";
}

/** What the session is doing, in words: running sessions by their status, the rest by how they ended. */
export function stateWords(s: SessionRow): string {
  if (!s.node_online) return `as of ${when(s.pushed_at)}`;
  if (!s.running) return s.closed_at ? `closed ${when(s.closed_at)}` : "not running";
  switch (s.status) {
    case "working":
      return "working";
    case "blocked":
      return "waiting on you";
    case "done":
      return "finished a turn";
    case "idle":
      return "idle";
    default:
      return "running";
  }
}

export function LinkChips({ links }: { links: SessionLink[] }) {
  if (links.length === 0) return null;
  return (
    <ul className="flex flex-wrap gap-1.5">
      {links.map((l) => (
        <li key={l.url}>
          <a
            href={l.url}
            target="_blank"
            rel="noreferrer noopener"
            className="inline-block rounded-sm border border-rule px-1.5 py-0.5 font-mono text-xs text-muted hover:border-rule-strong hover:text-ink"
            title={l.title ?? undefined}
          >
            {l.ref ?? l.title ?? l.url}
            {l.state && <span className="text-neutral"> · {l.state}</span>}
          </a>
        </li>
      ))}
    </ul>
  );
}

function SessionItem({ s }: { s: SessionRow }) {
  const [open, setOpen] = useState(false);
  const path = sessionPath(s.node, s.session_id);
  return (
    <li className="flex flex-col gap-1.5 border-b border-rule py-4">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <a
          href={path}
          onClick={(e) => {
            if (e.metaKey || e.ctrlKey) return;
            e.preventDefault();
            navigate(path);
          }}
          className="font-display text-lg text-ink hover:underline"
        >
          {sessionTitle(s)}
        </a>
        <span className="text-sm text-neutral">
          {s.node}
          {s.project && <> · {s.project}</>} · {stateWords(s)}
        </span>
      </div>
      {s.sentence ? (
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          className="text-left text-muted hover:text-ink"
        >
          {open && s.paragraph ? s.paragraph : s.sentence}
        </button>
      ) : (
        <p className="text-sm text-neutral">No summary yet.</p>
      )}
      <LinkChips links={s.links} />
      <div className="flex gap-2 pt-1">
        <Button variant="secondary" size="sm" onClick={() => navigate(path)}>
          Open
        </Button>
        {s.page && (
          <Button variant="quiet" size="sm" onClick={() => navigate(`${path}/page`)}>
            Page
          </Button>
        )}
      </div>
    </li>
  );
}

function SearchResults({ q, all }: { q: string; all: SessionRow[] }) {
  const words = useQuery({
    queryKey: ["sessions", "search", q],
    queryFn: () => api<SessionRow[]>(`/api/sessions/search${query({ q })}`),
  });
  const who = useQuery({
    queryKey: ["sessions", "who-has", q],
    queryFn: () => api<WhoHas>(`/api/who-has${query({ q })}`),
  });
  const found = words.data ?? [];
  const seen = new Set(found.map((s) => sessionKey(s.node, s.session_id)));
  const mentions = (who.data?.hits ?? []).filter((h) => !seen.has(sessionKey(h.node, h.session_id)));
  const byKey = new Map(all.map((s) => [sessionKey(s.node, s.session_id), s]));
  const busy = words.isPending || who.isPending;
  const nothing = !busy && found.length === 0 && mentions.length === 0;
  return (
    <section aria-label="Search results" className="flex flex-col">
      {words.isError && <p className="py-3 text-danger">{words.error.message}</p>}
      <ul>
        {found.map((s) => (
          <SessionItem key={sessionKey(s.node, s.session_id)} s={s} />
        ))}
        {mentions.map((h) => {
          const row = byKey.get(sessionKey(h.node, h.session_id));
          if (row) return <SessionItem key={sessionKey(h.node, h.session_id)} s={row} />;
          const path = sessionPath(h.node, h.session_id);
          return (
            <li key={sessionKey(h.node, h.session_id)} className="border-b border-rule py-4">
              <button type="button" onClick={() => navigate(path)} className="font-display text-lg text-ink hover:underline">
                {h.tab ?? h.session_id}
              </button>
              <p className="text-sm text-neutral">
                {h.node} · mentioned {when(h.last_mentioned_at)}
              </p>
            </li>
          );
        })}
      </ul>
      {who.isPending && <p className="py-3 text-sm text-neutral">Searching transcripts on the machines that are online…</p>}
      {nothing && <p className="py-6 text-muted">No session mentions “{q}”.</p>}
      {(who.data?.offline_nodes ?? []).map((n) => (
        <p key={n} className="pt-3 text-sm text-neutral">
          {n} isn’t connected: its sessions were searched by summary only.
        </p>
      ))}
      {(who.data?.partial ?? []).map((p) => (
        <p key={p.node} className="pt-3 text-sm text-neutral">
          {p.node} searched its transcripts back to {when(p.searched_back_to)}.
        </p>
      ))}
    </section>
  );
}

/** Every session on every machine, most recent first, with a search over summaries and transcripts. */
export function SessionsScreen() {
  const q = useSearch().get("q") ?? "";
  const [draft, setDraft] = useState(q);
  const sessions = useQuery({
    queryKey: ["sessions", "list"],
    queryFn: () => api<SessionRow[]>(`/api/sessions${query({ all: "true", limit: "100" })}`),
  });
  const rows = [...(sessions.data ?? [])].sort((a, b) =>
    (b.last_activity ?? b.status_changed_at).localeCompare(a.last_activity ?? a.status_changed_at),
  );
  return (
    <main className="mx-auto flex max-w-3xl flex-col gap-4 px-4 py-6">
      <h1 className="text-2xl">Sessions</h1>
      <form
        role="search"
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          setParam("q", draft.trim() || undefined);
        }}
      >
        <label className="relative min-w-0 flex-1">
          <span className="sr-only">Search sessions</span>
          <Search aria-hidden className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-neutral" />
          <Input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="Words or an ID (sc-1234)"
            className="pl-8"
          />
        </label>
        <Button type="submit" variant="secondary">
          Search
        </Button>
      </form>
      {q ? (
        <>
          <button
            type="button"
            className={cn("self-start text-sm text-neutral hover:text-ink")}
            onClick={() => {
              setDraft("");
              setParam("q", undefined);
            }}
          >
            Clear the search
          </button>
          <SearchResults q={q} all={rows} />
        </>
      ) : (
        <section aria-label="Recent sessions">
          {sessions.isError && <p className="text-danger">{sessions.error.message}</p>}
          {sessions.isSuccess && rows.length === 0 && (
            <p className="py-6 text-muted">No sessions yet. They appear here once a machine’s node connects.</p>
          )}
          <ul>
            {rows.map((s) => (
              <SessionItem key={sessionKey(s.node, s.session_id)} s={s} />
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}
