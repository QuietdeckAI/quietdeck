import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink } from "lucide-react";
import { useEffect, useRef } from "react";
import { useInfo } from "@/components/AccountTabs";
import { api } from "@/lib/api";
import { when } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { AccountLabel } from "@/types/AccountLabel";
import type { MailBadges } from "@/types/MailBadges";
import type { MailKind } from "@/types/MailKind";
import type { MailRow } from "@/types/MailRow";

export function useMailBadges() {
  return useQuery({ queryKey: ["mail-badges"], queryFn: () => api<MailBadges>("/api/mail/badges") });
}

/** Opens the draft (to edit or send) or the sent thread in Gmail, as the account's own address. */
function gmailLink(row: MailRow, accounts: AccountLabel[]): string | null {
  const address = accounts.find((a) => a.id === row.account)?.address;
  if (!address) return null;
  const base = `https://mail.google.com/mail/?authuser=${encodeURIComponent(address)}`;
  if (row.kind === "draft") return `${base}#drafts?compose=${encodeURIComponent(row.message_id)}`;
  return row.thread_id ? `${base}#all/${encodeURIComponent(row.thread_id)}` : `${base}#sent`;
}

/**
 * Gmail's snippets come HTML-escaped ("don&#39;t", "&amp;"): the text as it reads. DOMParser builds an inert
 * document, so nothing in the snippet runs.
 */
function unescapeHtml(text: string): string {
  if (!text.includes("&")) return text;
  return new DOMParser().parseFromString(text, "text/html").documentElement.textContent ?? text;
}

/** The address part of "Name <address>", or the whole header. */
function who(header: string | null): string {
  if (!header) return "";
  const first = header.split(",")[0].trim();
  const name = /^"?([^"<]+?)"?\s*</.exec(first)?.[1];
  const more = header.includes(",") ? ` +${header.split(",").length - 1}` : "";
  return (name ?? first.replace(/[<>]/g, "")) + more;
}

/** Every draft (or every sent email) on the person's accounts, newest first. Opening Sent clears its count of new mail
 * and dots what was new; Drafts just lists the drafts there are. */
export function MailScreen({ kind }: { kind: MailKind }) {
  const qc = useQueryClient();
  const rows = useQuery({ queryKey: ["mail", kind], queryFn: () => api<MailRow[]>(`/api/mail?kind=${kind}`) });
  const accounts = useInfo().data?.accounts ?? [];
  const seen = useMutation({
    mutationFn: () => api("/api/mail/seen", { method: "POST", body: { kind } }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["mail-badges"] }),
  });
  // Seen on opening, and again on leaving, so what arrived while it was open doesn't count as new.
  // biome-ignore lint/correctness/useExhaustiveDependencies: once per tab
  useEffect(() => {
    if (kind !== "sent") return;
    seen.mutate();
    return () => {
      void api("/api/mail/seen", { method: "POST", body: { kind } }).then(() =>
        qc.invalidateQueries({ queryKey: ["mail-badges"] }),
      );
    };
  }, [kind]);
  const label = (id: string) => accounts.find((a) => a.id === id)?.label ?? id;
  // The dots stay steady for the visit: what was new when the tab opened, and anything that arrives while it's open.
  // (Marking the tab seen refetches the list with every row not new, which would make them blink away.)
  const firstIds = useRef<Set<number> | null>(null);
  const newIds = useRef<Set<number>>(new Set());
  if (rows.data && firstIds.current === null) {
    firstIds.current = new Set(rows.data.map((r) => Number(r.id)));
    for (const r of rows.data) if (r.new) newIds.current.add(Number(r.id));
  }
  if (rows.data && firstIds.current) {
    for (const r of rows.data) if (!firstIds.current.has(Number(r.id))) newIds.current.add(Number(r.id));
  }
  return (
    <main className="mx-auto max-w-3xl px-4 pt-6 pb-10">
      {rows.isError && <p className="mt-6 text-danger">{rows.error.message}</p>}
      {rows.data?.length === 0 && (
        <p className="mt-16 text-center font-display text-xl text-neutral">
          {kind === "draft" ? "No drafts." : "Nothing sent in the last 30 days."}
        </p>
      )}
      <ul>
        {rows.data?.map((r) => {
          const href = gmailLink(r, accounts);
          return (
            <li key={String(r.id)} className="group flex items-start gap-3 border-b border-rule py-3 last:border-b-0">
              {/* Sent only: a dot on what's new since the last visit. Drafts have none. */}
              {kind === "sent" && (
                <span
                  className={cn(
                    "mt-2 size-1.5 shrink-0 rounded-full",
                    newIds.current.has(Number(r.id)) ? "bg-accent" : "bg-transparent",
                  )}
                  aria-label={newIds.current.has(Number(r.id)) ? "New" : undefined}
                />
              )}
              <div className="min-w-0 flex-1">
                {/* The whole first line opens it in Gmail: a draft to finish and send, a sent email's thread. */}
                {href ? (
                  <a
                    href={href}
                    target="_blank"
                    rel="noreferrer noopener"
                    className="flex items-baseline gap-2 hover:[&>span:first-child]:underline hover:[&>span:first-child]:underline-offset-4"
                  >
                    <span className="truncate text-ink">{r.subject || "(no subject)"}</span>
                    <span className="ml-auto shrink-0 text-xs tabular-nums text-neutral">{when(r.at)}</span>
                  </a>
                ) : (
                  <p className="flex items-baseline gap-2">
                    <span className="truncate text-ink">{r.subject || "(no subject)"}</span>
                    <span className="ml-auto shrink-0 text-xs tabular-nums text-neutral">{when(r.at)}</span>
                  </p>
                )}
                <p className="mt-0.5 truncate text-sm text-muted">
                  To {who(r.to)}
                  <span className="text-neutral"> · {label(r.account)}</span>
                  {r.posted_by && (
                    <span className="text-neutral">
                      {" "}
                      · by <span className="font-mono text-xs">{r.posted_by.tab}</span>
                      {r.card_id !== null && <> (c_{String(r.card_id)})</>}
                    </span>
                  )}
                </p>
                {r.snippet && <p className="mt-0.5 line-clamp-1 text-sm text-neutral">{unescapeHtml(r.snippet)}</p>}
              </div>
              {href && (
                <a
                  href={href}
                  target="_blank"
                  rel="noreferrer noopener"
                  aria-label={`Open “${r.subject ?? ""}” in Gmail`}
                  title="Open in Gmail"
                  className="mt-0.5 rounded p-1 text-neutral hover:text-ink"
                >
                  <ExternalLink className="size-4" aria-hidden />
                </a>
              )}
            </li>
          );
        })}
      </ul>
    </main>
  );
}
