import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown,
  ChevronRight,
  Eye,
  EyeOff,
  MessageSquare,
  Info,
  Clock,
  ExternalLink,
  MoreHorizontal,
  RotateCcw,
} from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Textarea } from "@/components/ui/input";
import { useInfo } from "@/components/AccountTabs";
import { actionLine, sessionName } from "@/components/actionText";
import { api } from "@/lib/api";
import { emailLink } from "@/lib/emailLink";
import { when } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { Action } from "@/types/Action";
import type { Approving } from "@/types/Approving";
import type { Card } from "@/types/Card";
import type { DecisionKind } from "@/types/DecisionKind";
import type { RuleOffer } from "@/types/RuleOffer";
import type { ThreadWatch } from "@/types/ThreadWatch";

export type Draft = { text: string; include: boolean };

/** What the card's state means to him, when it isn't simply waiting for a decision. */
function stateLine(
  card: Card,
): { text: string; short: string; tone: "quiet" | "danger" } | null {
  switch (card.state) {
    case "revising":
      return {
        text: `${card.posted_by.tab} is revising this after your note.`,
        short: "revising",
        tone: "quiet",
      };
    case "approved":
      return { text: "Approved. Waiting to run.", short: "approved", tone: "quiet" };
    case "executing":
      return { text: "Running.", short: "running", tone: "quiet" };
    case "handed_off":
      return {
        text: `With ${card.posted_by.tab}${card.handoff_reason ? `: ${card.handoff_reason}` : "."}`,
        short: `with ${card.posted_by.tab}`,
        tone: "quiet",
      };
    case "failed": {
      const err = card.action_records.find((a) => a.status === "failed")?.error;
      return {
        text: err ? `Failed: ${err}` : "An action failed.",
        short: "failed",
        tone: "danger",
      };
    }
    default:
      return null;
  }
}

export function decide(card: Card, kind: DecisionKind) {
  return api<Card>(`/api/cards/${card.id}/decide`, {
    method: "POST",
    body: { version: card.version, kind },
  });
}

/** The proposed actions in a few words, as the triage skill wrote them: `reply “Stop, thanks.” + archive`. */
export function actionsShort(card: Card): string {
  return card.actions.map(actionShort).join(" + ");
}

function quoted(text: string, max = 40): string {
  const one = text.replace(/\s+/g, " ").trim();
  return `“${one.length > max ? `${one.slice(0, max - 1)}…` : one}”`;
}

function actionShort(a: Action): string {
  switch (a.type) {
    case "archive":
      return "archive";
    case "mark_read":
      return "mark read";
    case "not_spam":
      return "not spam";
    case "close_conversation":
      return "close";
    case "label":
      return `label ${[...a.add.map((l) => `+${l}`), ...a.remove.map((l) => `−${l}`)].join(" ")}`;
    case "forward":
      return `forward to ${a.to.tab}`;
    case "draft":
      return `draft ${quoted(a.body)}`;
    case "reply":
      return `reply ${quoted(a.body)}`;
    case "story":
      return a.op === "create" ? `new ${a.story_type}: ${a.title}` : `comment on ${a.story_id}`;
    case "task":
      return a.op === "create" ? `task: ${a.title ?? ""}` : a.op === "done" ? "finish task" : "update task";
    case "calendar":
      return `calendar: ${a.title}`;
    case "resume":
      return `resume ${a.tab}`;
    case "new_session":
      return `start ${a.tab}`;
    case "lease":
      return `lease ${a.kind}`;
    case "watch":
      return a.on ? "watch" : "stop watching";
    case "session":
      return a.op === "message"
        ? `message ${a.to.tab}`
        : a.op === "wrap_up"
          ? `wrap up ${a.to.tab}`
          : a.op === "compact"
            ? `compact ${a.to.tab}`
            : `close ${a.to.tab}`;
  }
}

/**
 * One email as one line, like the triage skill's list: who, what it is, and what's proposed. Approve All takes it
 * unless he changes it. A note (the speech bubble) turns it into an instruction for the agent; the menu has the rest.
 * Clicking the line opens the details: the full drafts, facts, the rule the agent used, auto-approve.
 */
export function ActionCard({
  card,
  draft,
  onDraft,
  suggestion,
  offer,
  showAccount,
}: {
  card: Card;
  draft: Draft;
  onDraft: (d: Draft) => void;
  suggestion?: boolean;
  offer?: RuleOffer;
  showAccount?: boolean;
}) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [noting, setNoting] = useState(false);
  const [why, setWhy] = useState(false);
  const move = useMutation({
    mutationFn: async (o: RuleOffer) => {
      for (const id of o.rule_ids) {
        await api(`/api/rules/${id}/status`, {
          method: "POST",
          body: { status: "active", move_to: o.to },
        });
      }
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["rules"] });
      void qc.invalidateQueries({ queryKey: ["inbox"] });
    },
  });
  const later = useMutation({ mutationFn: () => decide(card, "later") });
  const down = useMutation({ mutationFn: () => decide(card, "move_down") });
  const retry = useMutation({
    mutationFn: () =>
      api<Approving>(`/api/cards/${card.id}/retry`, { method: "POST" }),
  });
  const gmail = emailLink(card, useInfo().data?.accounts ?? []);
  const watches = useQuery({
    queryKey: ["watches"],
    queryFn: () => api<ThreadWatch[]>("/api/watches"),
  });
  const watching = watches.data?.find(
    (w) => w.account === card.account && w.thread_key === card.thread_key,
  );
  const watch = useMutation({
    mutationFn: () =>
      watching
        ? api(`/api/watches/${watching.id}/stop`, { method: "POST" })
        : api<ThreadWatch>("/api/watches", {
            method: "POST",
            body: {
              account: card.account,
              thread_key: card.thread_key,
              subject: card.subject,
            },
          }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["watches"] }),
  });
  const status = stateLine(card);
  const decidable = card.state === "open" || card.state === "handed_off";
  const error =
    later.error ?? down.error ?? retry.error ?? move.error ?? watch.error;
  const sender = card.sender?.name ?? card.sender?.address ?? card.account;
  const proposed = actionsShort(card);
  const left = suggestion && !draft.include;
  const noteShown = noting || draft.text.length > 0;

  return (
    <li
      className={cn(
        "arrive border-b border-rule last:border-b-0",
        open && "bg-paper-2",
      )}
    >
      <div className="flex items-start gap-1 py-0.5">
        {suggestion && decidable ? (
          <input
            type="checkbox"
            className="mt-1 size-4 shrink-0 accent-(--color-accent)"
            checked={draft.include}
            onChange={(e) => onDraft({ ...draft, include: e.target.checked })}
            aria-label={`Include “${card.subject}” in Approve All`}
          />
        ) : null}
        <button
          type="button"
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className="flex min-w-0 flex-1 items-start gap-1 rounded-sm px-1 py-0.5 text-left text-sm leading-5 hover:bg-paper-3"
        >
          <ChevronRight
            aria-hidden
            className={cn(
              "mt-0.5 size-4 shrink-0 text-neutral transition-transform duration-(--dur-quick)",
              open && "rotate-90",
            )}
          />
          <span
            className={cn(
              "flex min-w-0 flex-1 flex-col lg:flex-row lg:gap-1.5",
              left && "opacity-60",
            )}
          >
            <span className={cn("min-w-0", !open && "truncate")}>
              {showAccount && <span className="text-neutral">{card.account} · </span>}
              {sender !== card.account && (
                <>
                  <span className="text-ink">{sender}</span>
                  <span className="text-neutral"> · </span>
                </>
              )}
              <span className="text-muted">{card.summary || card.subject}</span>
            </span>
            {(proposed || status) && (
              <span className={cn("min-w-0 lg:max-w-[45%] lg:shrink-0", !open && "truncate")}>
                {proposed && (
                  <span className="text-accent">→ {draft.text.trim() ? "your note" : proposed}</span>
                )}
                {status && (
                  <span className={status.tone === "danger" ? "text-danger" : "text-neutral"}>
                    {proposed ? " · " : ""}
                    {status.short}
                  </span>
                )}
              </span>
            )}
          </span>
        </button>
        <div className="flex shrink-0 items-center">
          {card.state === "failed" && (
            <Button variant="quiet" size="sm" loading={retry.isPending} onClick={() => retry.mutate()}>
              <RotateCcw aria-hidden /> Retry
            </Button>
          )}
          {decidable && (
            <Button
              variant="quiet"
              size="icon"
              aria-label={`Note for ${card.posted_by.tab} on ${card.subject}`}
              aria-pressed={noteShown}
              className={cn("size-7", noteShown && "text-accent")}
              onClick={() => setNoting(!noting)}
            >
              <MessageSquare />
            </Button>
          )}
          {/* Not modal: a modal menu locks the page's scrolling, which hides the sticky bar at the bottom. */}
          <DropdownMenu modal={false}>
            <DropdownMenuTrigger asChild>
              <Button variant="quiet" size="icon" className="size-7" aria-label="More for this card">
                <MoreHorizontal />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent>
              {gmail && (
                <DropdownMenuItem asChild>
                  <a href={gmail} target="_blank" rel="noreferrer">
                    <ExternalLink /> Open in Gmail
                  </a>
                </DropdownMenuItem>
              )}
              <DropdownMenuItem disabled={!watches.isSuccess} onSelect={() => watch.mutate()}>
                {watching ? (
                  <>
                    <EyeOff /> Stop watching the thread
                  </>
                ) : (
                  <>
                    <Eye /> Watch thread
                  </>
                )}
              </DropdownMenuItem>
              <DropdownMenuItem disabled={!decidable} onSelect={() => later.mutate()}>
                <Clock /> Leave for later
              </DropdownMenuItem>
              {!suggestion && (
                <DropdownMenuItem disabled={!decidable} onSelect={() => down.mutate()}>
                  <ArrowDown /> Move to the archive list
                </DropdownMenuItem>
              )}
              <DropdownMenuItem
                onSelect={() => {
                  setOpen(true);
                  setWhy(true);
                }}
              >
                <Info /> Why this
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {decidable && noteShown && (
        <div className="pb-2 pl-6">
          <Textarea
            rows={1}
            autoFocus={noting && !draft.text}
            placeholder={`A note for ${card.posted_by.tab}: changes it to your instruction`}
            value={draft.text}
            maxLength={4000}
            onChange={(e) => onDraft({ ...draft, text: e.target.value })}
            aria-label={`Note for ${card.posted_by.tab} on ${card.subject}`}
            className="min-h-9 py-1.5 text-sm"
          />
        </div>
      )}

      {open && (
        <div className="flex flex-col gap-2 pb-2 pl-6 pr-2 text-sm">
          <time className="font-mono text-xs text-neutral" dateTime={card.last_message_at ?? card.prepared_at}>
            {when(card.last_message_at ?? card.prepared_at)}
          </time>
          {card.facts.length > 0 && (
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-0.5">
              {card.facts.map((f) => (
                <div key={f.label} className="contents">
                  <dt className="text-neutral">{f.label}</dt>
                  <dd className="text-ink">{f.value}</dd>
                </div>
              ))}
            </dl>
          )}
          {card.actions.length > 0 && (
            <ul className="flex flex-col gap-2">
              {card.actions.map((a, i) => {
                const line = actionLine(a);
                const record = card.action_records[i];
                return (
                  <li key={i}>
                    <span className="text-ink">{line.verb}</span>
                    {record && record.status !== "pending" && record.status !== "released" && (
                      <span
                        className={cn("ml-2 text-xs", record.status === "failed" ? "text-danger" : "text-neutral")}
                      >
                        {record.status}
                      </span>
                    )}
                    {line.detail && (
                      <p className="mt-0.5 whitespace-pre-line text-muted">{line.detail.replace(/\n\s*\n/g, "\n")}</p>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
          {why && (
            <p className="text-xs text-neutral">
              {card.reason}
              {card.rule_ref && <> · {card.rule_ref}</>} · {card.subject} · {sessionName(card.posted_by)}
            </p>
          )}
          {status && (
            <p className={status.tone === "danger" ? "text-danger" : "text-muted"}>{status.text}</p>
          )}
          {offer && decidable && (
            <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-muted">
              <span className="min-w-0 flex-1">
                {offer.rule_ids.length === 1 ? "A rule" : `${offer.rule_ids.length} rules`} for {offer.to.tab} were
                archived when its earlier session closed.
              </span>
              <Button size="sm" loading={move.isPending} onClick={() => move.mutate(offer)}>
                Move {offer.rule_ids.length === 1 ? "it" : "them"} to this session
              </Button>
            </div>
          )}
        </div>
      )}
      {error && (
        <p role="alert" className="pb-2 pl-6 text-sm text-danger">
          {error.message}
        </p>
      )}
    </li>
  );
}
