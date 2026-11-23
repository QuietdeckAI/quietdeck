import { useQuery } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { Markdown } from "@/components/Markdown";
import { Button } from "@/components/ui/button";
import { api, query } from "@/lib/api";
import { when } from "@/lib/format";
import { navigate, setParam, useSearch } from "@/lib/route";
import { cn } from "@/lib/utils";
import { sessionKey, sessionPath, sessionTitle } from "@/screens/Sessions";
import type { SessionDetail } from "@/types/SessionDetail";
import type { SummaryRecord } from "@/types/SummaryRecord";

function span(r: SummaryRecord): string {
  const from = when(r.activity_from);
  const to = when(r.activity_to);
  return from === to ? from : `${from} to ${to}`;
}

/** A session's full summary, one update at a time, with Prev and Next and every update's sentence. */
export function SessionPageScreen({ node, sessionId }: { node: string; sessionId: string }) {
  const at = useSearch().get("at");
  const detail = useQuery({
    queryKey: ["sessions", "show", node, sessionId, "timeline"],
    queryFn: () =>
      api<SessionDetail>(`/api/sessions/show${query({ session: sessionKey(node, sessionId), timeline: "true" })}`),
  });
  const s = detail.data?.session;
  const records = [...(detail.data?.timeline ?? [])].sort((a, b) => a.activity_from.localeCompare(b.activity_from));
  const index = at === null ? records.length - 1 : Math.min(Math.max(Number(at) || 0, 0), records.length - 1);
  const record = records[index];
  const path = sessionPath(node, sessionId);
  const go = (i: number) => {
    setParam("at", i === records.length - 1 ? undefined : String(i));
    window.scrollTo({ top: 0 });
  };
  return (
    <main className="mx-auto flex max-w-3xl flex-col gap-5 px-4 py-6">
      <a
        href={path}
        onClick={(e) => {
          e.preventDefault();
          navigate(path);
        }}
        className="flex items-center gap-1 self-start text-sm text-neutral hover:text-ink"
      >
        <ArrowLeft aria-hidden className="size-4" /> {s ? sessionTitle(s) : "The session"}
      </a>
      {detail.isError && <p className="text-muted">{detail.error.message}</p>}
      {detail.isSuccess && !record && (
        <>
          <h1 className="text-2xl">{s ? sessionTitle(s) : "Untitled session"}</h1>
          {s?.page ? <Markdown text={s.page} /> : <p className="text-muted">This session has no summary yet.</p>}
        </>
      )}
      {record && (
        <>
          <header className="flex flex-col gap-1">
            <h1 className="text-2xl">{s ? sessionTitle(s) : (record.tab ?? "Untitled session")}</h1>
            <p className="text-sm text-neutral">
              {s?.node} · {span(record)}
            </p>
          </header>
          <nav aria-label="Updates" className="flex items-center justify-between gap-2">
            <Button variant="secondary" size="sm" disabled={index <= 0} onClick={() => go(index - 1)}>
              Prev
            </Button>
            <Button variant="secondary" size="sm" disabled={index >= records.length - 1} onClick={() => go(index + 1)}>
              Next
            </Button>
          </nav>
          <article>
            <Markdown text={record.page || record.paragraph} />
          </article>
          <section aria-label="Timeline" className="border-t border-rule pt-4">
            <h2 className="mb-2 text-lg">Timeline</h2>
            <ol className="flex flex-col">
              {records
                .map((r, i) => ({ r, i }))
                .reverse()
                .map(({ r, i }) => (
                  <li key={`${r.period.kind}:${r.period.key}`}>
                    <button
                      type="button"
                      onClick={() => go(i)}
                      aria-current={i === index ? "true" : undefined}
                      className={cn(
                        "grid w-full grid-cols-[6.5rem_minmax(0,1fr)] gap-3 rounded-md px-2 py-1.5 text-left text-sm hover:bg-paper-3 sm:grid-cols-[9rem_minmax(0,1fr)]",
                        i === index && "bg-paper-3",
                      )}
                    >
                      <span className="font-mono text-xs leading-5 text-neutral">{when(r.activity_to)}</span>
                      <span className={cn("text-muted", i === index && "text-ink")}>{r.sentence}</span>
                    </button>
                  </li>
                ))}
            </ol>
          </section>
        </>
      )}
    </main>
  );
}
