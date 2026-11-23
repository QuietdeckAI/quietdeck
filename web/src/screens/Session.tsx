import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, ChevronRight, ExternalLink } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Markdown } from "@/components/Markdown";
import { Button } from "@/components/ui/button";
import { api, query } from "@/lib/api";
import { when } from "@/lib/format";
import { navigate, setParam, useSearch } from "@/lib/route";
import { cn } from "@/lib/utils";
import { LinkChips, sessionKey, sessionPath, sessionTitle, stateWords } from "@/screens/Sessions";
import type { Catchup } from "@/types/Catchup";
import type { SessionDetail } from "@/types/SessionDetail";
import type { SessionRow } from "@/types/SessionRow";
import type { Subagent } from "@/types/Subagent";
import type { TranscriptItem } from "@/types/TranscriptItem";
import type { TranscriptPage } from "@/types/TranscriptPage";

/** How often an open viewer asks the session's machine for what's new. */
const POLL_MS = 4000;

const itemKey = (i: TranscriptItem) => `${i.offset}:${i.part}`;
const time = (iso: string | null) => (iso ? when(iso) : "");

type Tool = Extract<TranscriptItem, { kind: "tool" }>;

function ToolLine({ item, result, onSubagent }: { item: Tool; result?: { result: string; error: boolean }; onSubagent: (id: string) => void }) {
  const out = item.result ?? result?.result;
  const error = item.error || (result?.error ?? false);
  return (
    <details className="group">
      <summary
        className={cn(
          "flex cursor-pointer list-none items-start gap-1.5 font-mono text-xs text-neutral hover:text-muted",
          error && "text-danger hover:text-danger",
        )}
      >
        <ChevronRight aria-hidden className="mt-0.5 size-3 shrink-0 transition-transform group-open:rotate-90" />
        <span className="min-w-0 [overflow-wrap:anywhere]">{item.line}</span>
      </summary>
      <div className="mt-1.5 ml-4.5 flex flex-col gap-1.5">
        <pre className="max-h-64 overflow-auto rounded-md border border-rule bg-paper-2 p-2 font-mono text-xs whitespace-pre-wrap text-muted">
          {item.input}
        </pre>
        {out !== undefined && out !== null && (
          <pre
            className={cn(
              "max-h-64 overflow-auto rounded-md border border-rule p-2 font-mono text-xs whitespace-pre-wrap text-muted",
              error && "border-danger/50",
            )}
          >
            {out || "(no output)"}
          </pre>
        )}
        {item.subagent && (
          <Button variant="quiet" size="sm" className="self-start" onClick={() => onSubagent(item.subagent as string)}>
            Read the subagent’s transcript
          </Button>
        )}
      </div>
    </details>
  );
}

function Item({
  item,
  results,
  onSubagent,
}: {
  item: TranscriptItem;
  results: Map<string, { result: string; error: boolean }>;
  onSubagent: (id: string) => void;
}) {
  switch (item.kind) {
    case "person":
      return (
        <div className="rounded-lg bg-paper-2 px-3 py-2">
          <p className="mb-0.5 text-xs text-neutral">
            You <span className="font-mono">{time(item.at)}</span>
          </p>
          <p className="whitespace-pre-wrap [overflow-wrap:anywhere]">{item.text}</p>
        </div>
      );
    case "peer":
      return (
        <div className="border-l-2 border-accent/60 pl-3">
          <p className="mb-0.5 text-xs text-neutral">
            From {item.from ?? "another session"} <span className="font-mono">{time(item.at)}</span>
          </p>
          <p className="whitespace-pre-wrap text-muted [overflow-wrap:anywhere]">{item.text}</p>
        </div>
      );
    case "agent":
      return <Markdown text={item.text} />;
    case "thinking":
      return (
        <details className="group text-sm text-neutral">
          <summary className="flex cursor-pointer list-none items-center gap-1.5 hover:text-muted">
            <ChevronRight aria-hidden className="size-3 transition-transform group-open:rotate-90" />
            Thinking
          </summary>
          <p className="mt-1 ml-4.5 whitespace-pre-wrap [overflow-wrap:anywhere]">{item.text}</p>
        </details>
      );
    case "tool":
      return <ToolLine item={item} result={item.id ? results.get(item.id) : undefined} onSubagent={onSubagent} />;
    case "wake_up":
      return <p className="text-xs whitespace-pre-wrap text-neutral/70 [overflow-wrap:anywhere]">{item.text}</p>;
    case "compaction":
      return (
        <details className="group border-y border-rule py-1.5 text-center text-xs text-neutral">
          <summary className="cursor-pointer list-none hover:text-muted">The conversation was compacted here</summary>
          {item.summary && (
            <p className="mt-2 text-left text-sm whitespace-pre-wrap text-muted [overflow-wrap:anywhere]">{item.summary}</p>
          )}
        </details>
      );
    case "interrupted":
      return <p className="text-xs text-neutral">You interrupted it.</p>;
    case "tool_result":
      return null;
  }
}

/** "Since your last message": the session's machine writes it, and it fills in when ready. */
function CatchupBox({ session }: { session: SessionRow }) {
  const catchup = useQuery({
    queryKey: ["catchup", session.node, session.session_id],
    queryFn: () => api<Catchup>(`/api/sessions/catchup${query({ session: sessionKey(session.node, session.session_id) })}`),
    enabled: session.node_online && session.agent === "claude",
    retry: false,
    refetchInterval: (q) => (q.state.data?.stale ? 10_000 : false),
  });
  if (!session.node_online || session.agent !== "claude") return null;
  return (
    <section aria-label="Since your last message" className="rounded-lg border border-rule px-4 py-3">
      <h2 className="mb-1 text-base">Since your last message</h2>
      {catchup.isPending && <p className="text-sm text-neutral">Writing the catch-up…</p>}
      {catchup.isError && <p className="text-sm text-neutral">No catch-up: {catchup.error.message}</p>}
      {catchup.data &&
        (catchup.data.anchor ? (
          <>
            <Markdown text={catchup.data.text} className="text-muted" />
            {catchup.data.stale && <p className="mt-1 text-xs text-neutral">More has happened since; updating.</p>}
          </>
        ) : (
          <p className="text-sm text-neutral">You haven’t written in this session.</p>
        ))}
    </section>
  );
}

function SubagentList({ subagents, current, onPick }: { subagents: Subagent[]; current: string | null; onPick: (id: string | null) => void }) {
  return (
    <ul className="flex flex-col gap-0.5 text-sm">
      <li>
        <button
          type="button"
          onClick={() => onPick(null)}
          aria-current={current === null ? "true" : undefined}
          className={cn("w-full rounded-md px-2 py-1 text-left text-muted hover:bg-paper-3 hover:text-ink", current === null && "bg-paper-3 text-ink")}
        >
          The session
        </button>
      </li>
      {subagents.map((s) => (
        <li key={s.id} style={{ paddingLeft: `${(s.depth - 1) * 0.75}rem` }}>
          <button
            type="button"
            onClick={() => onPick(s.id)}
            aria-current={current === s.id ? "true" : undefined}
            className={cn(
              "w-full rounded-md px-2 py-1 text-left text-muted hover:bg-paper-3 hover:text-ink",
              current === s.id && "bg-paper-3 text-ink",
            )}
          >
            {s.description ?? s.id}
            {s.agent_type && <span className="block text-xs text-neutral">{s.agent_type}</span>}
          </button>
        </li>
      ))}
    </ul>
  );
}

function nearBottom() {
  return window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 160;
}

/** The transcript: opens at the person's last message, loads earlier turns going up, and follows new ones. */
function Transcript({
  session,
  subagent,
  onSubagents,
  onSubagent,
}: {
  session: SessionRow;
  subagent: string | null;
  onSubagents: (s: Subagent[]) => void;
  onSubagent: (id: string) => void;
}) {
  const key = sessionKey(session.node, session.session_id);
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [earlier, setEarlier] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const end = useRef(0);
  const busy = useRef(false);
  const top = useRef<HTMLDivElement>(null);
  // Set before a change to the list, read after it renders: how to keep the reader's place.
  const scroll = useRef<{ kind: "anchor" } | { kind: "keep"; height: number } | { kind: "bottom" } | null>(null);

  const fetchPage = useCallback(
    (params: { before?: number; after?: number }) =>
      api<TranscriptPage>(
        `/api/sessions/transcript${query({
          session: key,
          subagent: subagent ?? undefined,
          before: params.before?.toString(),
          after: params.after?.toString(),
        })}`,
      ),
    [key, subagent],
  );

  useEffect(() => {
    let live = true;
    setItems([]);
    setLoaded(false);
    setError(null);
    busy.current = true;
    fetchPage({})
      .then((p) => {
        if (!live) return;
        scroll.current = { kind: "anchor" };
        setItems(p.items);
        setEarlier(p.earlier);
        end.current = p.end;
        onSubagents(p.subagents);
        setLoaded(true);
      })
      .catch((e: Error) => live && setError(e.message))
      .finally(() => {
        busy.current = false;
      });
    return () => {
      live = false;
    };
  }, [fetchPage, onSubagents]);

  const loadEarlier = useCallback(() => {
    if (busy.current || earlier === null) return;
    busy.current = true;
    fetchPage({ before: earlier })
      .then((p) => {
        scroll.current = { kind: "keep", height: document.documentElement.scrollHeight };
        setItems((old) => [...p.items, ...old]);
        setEarlier(p.earlier);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => {
        busy.current = false;
      });
  }, [earlier, fetchPage]);

  // New turns while the page is open and the machine is connected.
  useEffect(() => {
    if (!loaded || !session.node_online) return;
    const id = window.setInterval(() => {
      if (busy.current || document.visibilityState !== "visible") return;
      busy.current = true;
      fetchPage({ after: end.current })
        .then((p) => {
          end.current = p.end;
          if (p.items.length === 0) return;
          if (nearBottom()) scroll.current = { kind: "bottom" };
          setItems((old) => [...old, ...p.items]);
        })
        .catch(() => {
          // The machine dropped off or the session closed: the next tick tries again, and the page keeps what it has.
        })
        .finally(() => {
          busy.current = false;
        });
    }, POLL_MS);
    return () => window.clearInterval(id);
  }, [loaded, session.node_online, fetchPage]);

  useLayoutEffect(() => {
    const s = scroll.current;
    scroll.current = null;
    if (!s) return;
    if (s.kind === "keep") {
      window.scrollBy(0, document.documentElement.scrollHeight - s.height);
    } else if (s.kind === "bottom") {
      window.scrollTo({ top: document.documentElement.scrollHeight });
    } else {
      const anchor = document.querySelectorAll<HTMLElement>("[data-person]");
      const last = anchor[anchor.length - 1];
      if (last) last.scrollIntoView({ block: "start" });
      else window.scrollTo({ top: document.documentElement.scrollHeight });
    }
  }, [items]);

  // Scrolling up to the top of what's loaded loads the page before it.
  useEffect(() => {
    const el = top.current;
    if (!el || !loaded) return;
    const io = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && loadEarlier(), {
      rootMargin: "200px 0px 0px 0px",
    });
    io.observe(el);
    return () => io.disconnect();
  }, [loaded, loadEarlier]);

  const results = useMemo(() => {
    const m = new Map<string, { result: string; error: boolean }>();
    for (const i of items) if (i.kind === "tool_result") m.set(i.id, { result: i.result, error: i.error });
    return m;
  }, [items]);

  if (error && items.length === 0) return <p className="py-6 text-muted">{error}</p>;
  if (!loaded) return <p className="py-6 text-sm text-neutral">Reading the transcript on {session.node}…</p>;
  return (
    <div className="flex flex-col gap-4">
      <div ref={top} />
      {earlier !== null ? (
        <Button variant="quiet" size="sm" className="self-center" onClick={loadEarlier}>
          Load earlier
        </Button>
      ) : (
        <p className="text-center text-xs text-neutral">The start of the {subagent ? "subagent’s " : ""}transcript</p>
      )}
      {items.length === 0 && <p className="text-muted">Nothing in this transcript yet.</p>}
      {items.map((i) => (
        <div key={itemKey(i)} data-person={i.kind === "person" ? "" : undefined} className="scroll-mt-4">
          <Item item={i} results={results} onSubagent={onSubagent} />
        </div>
      ))}
      {error && <p className="text-sm text-danger">{error}</p>}
    </div>
  );
}

/** One session: its header and catch-up, and its transcript read from its machine. */
export function SessionScreen({ node, sessionId }: { node: string; sessionId: string }) {
  const subagent = useSearch().get("subagent");
  const [subagents, setSubagents] = useState<Subagent[]>([]);
  const detail = useQuery({
    queryKey: ["sessions", "show", node, sessionId],
    queryFn: () => api<SessionDetail>(`/api/sessions/show${query({ session: sessionKey(node, sessionId) })}`),
  });
  const pick = useCallback((id: string | null) => {
    setParam("subagent", id ?? undefined);
    window.scrollTo({ top: 0 });
  }, []);
  const s = detail.data?.session;
  const path = sessionPath(node, sessionId);
  const current = subagents.find((a) => a.id === subagent);
  return (
    <main className="mx-auto flex max-w-5xl flex-col gap-4 px-4 py-6">
      <a
        href="/sessions"
        onClick={(e) => {
          e.preventDefault();
          navigate("/sessions");
        }}
        className="flex items-center gap-1 self-start text-sm text-neutral hover:text-ink"
      >
        <ArrowLeft aria-hidden className="size-4" /> Sessions
      </a>
      {detail.isError && <p className="text-muted">{detail.error.message}</p>}
      {s && (
        <>
          <header className="flex flex-col gap-2">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <h1 className="text-2xl">{sessionTitle(s)}</h1>
                <p className="text-sm text-neutral">
                  {s.node}
                  {s.project && <> · {s.project}</>} · {stateWords(s)}
                </p>
              </div>
              <div className="flex gap-2">
                {s.page && (
                  <Button variant="secondary" size="sm" onClick={() => navigate(`${path}/page`)}>
                    Page
                  </Button>
                )}
                {s.running && s.open_url && (
                  <Button variant="secondary" size="sm" asChild className="max-md:hidden">
                    <a href={s.open_url}>
                      <ExternalLink aria-hidden />
                      Open in {s.opens_in ?? "the terminal"}
                    </a>
                  </Button>
                )}
              </div>
            </div>
            <LinkChips links={s.links} />
          </header>
          {!subagent && <CatchupBox session={s} />}
          <div className={cn("grid gap-6", subagents.length > 0 && "lg:grid-cols-[minmax(0,1fr)_15rem]")}>
            <div className="min-w-0">
              {subagent && (
                <p className="mb-4 text-sm text-neutral">
                  Subagent: <span className="text-ink">{current?.description ?? subagent}</span> ·{" "}
                  <button type="button" className="underline hover:text-ink" onClick={() => pick(null)}>
                    back to the session
                  </button>
                </p>
              )}
              {!s.node_online ? (
                <p className="py-6 text-muted">
                  {s.node} isn’t connected, so its transcripts can’t be read now. The summary is on its Page.
                </p>
              ) : (
                <Transcript session={s} subagent={subagent} onSubagents={setSubagents} onSubagent={pick} />
              )}
            </div>
            {subagents.length > 0 && (
              <aside className="order-first lg:order-none">
                <details className="lg:hidden">
                  <summary className="cursor-pointer text-sm text-neutral hover:text-ink">Subagents</summary>
                  <div className="mt-2">
                    <SubagentList subagents={subagents} current={subagent} onPick={pick} />
                  </div>
                </details>
                <div className="sticky top-4 max-lg:hidden">
                  <h2 className="mb-2 text-base">Subagents</h2>
                  <SubagentList subagents={subagents} current={subagent} onPick={pick} />
                </div>
              </aside>
            )}
          </div>
        </>
      )}
    </main>
  );
}
