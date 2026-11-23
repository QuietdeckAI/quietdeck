import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown,
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
import { actionLine, sessionName } from "@/components/actionText";
import { RuleEditor, ruleFromCard } from "@/components/RuleEditor";
import { api } from "@/lib/api";
import { when } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { Approving } from "@/types/Approving";
import type { Card } from "@/types/Card";
import type { DecisionKind } from "@/types/DecisionKind";
import type { RuleOffer } from "@/types/RuleOffer";

export type Draft = { text: string; include: boolean };

/** What the card's state means to him, when it isn't simply waiting for a decision. */
function stateLine(
  card: Card,
): { text: string; tone: "quiet" | "danger" } | null {
  switch (card.state) {
    case "revising":
      return {
        text: `${card.posted_by.tab} is revising this after your note.`,
        tone: "quiet",
      };
    case "approved":
      return { text: "Approved. Waiting to run.", tone: "quiet" };
    case "executing":
      return { text: "Running.", tone: "quiet" };
    case "handed_off":
      return {
        text: `With ${card.posted_by.tab}${card.handoff_reason ? `: ${card.handoff_reason}` : "."}`,
        tone: "quiet",
      };
    case "failed": {
      const err = card.action_records.find((a) => a.status === "failed")?.error;
      return {
        text: err ? `Failed: ${err}` : "An action failed.",
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

export function ActionCard({
  card,
  draft,
  onDraft,
  suggestion,
  offer,
}: {
  card: Card;
  draft: Draft;
  onDraft: (d: Draft) => void;
  suggestion?: boolean;
  offer?: RuleOffer;
}) {
  const qc = useQueryClient();
  const [ruling, setRuling] = useState(false);
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
  const status = stateLine(card);
  const decidable = card.state === "open" || card.state === "handed_off";
  const error = later.error ?? down.error ?? retry.error ?? move.error;

  return (
    <article
      className={cn(
        "arrive rounded-lg border border-rule bg-paper-2 p-4 sm:p-5",
        suggestion && !draft.include && "border-dashed",
      )}
      aria-labelledby={`card-${card.id}`}
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-baseline gap-x-2 text-sm text-neutral">
            <span className="text-muted">
              {card.sender?.name ?? card.sender?.address ?? card.account}
            </span>
            <span aria-hidden>·</span>
            <span>{card.account}</span>
            <span aria-hidden>·</span>
            <time
              className="font-mono text-xs"
              dateTime={card.last_message_at ?? card.prepared_at}
            >
              {when(card.last_message_at ?? card.prepared_at)}
            </time>
          </p>
          <h3 id={`card-${card.id}`} className="mt-1 text-lg">
            {card.subject}
          </h3>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="quiet" size="icon" aria-label="More for this card">
              <MoreHorizontal />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            {card.link && (
              <DropdownMenuItem asChild>
                <a href={card.link} target="_blank" rel="noreferrer">
                  <ExternalLink /> Open in Gmail
                </a>
              </DropdownMenuItem>
            )}
            <DropdownMenuItem
              disabled={!decidable}
              onSelect={() => later.mutate()}
            >
              <Clock /> Leave for later
            </DropdownMenuItem>
            {!suggestion && (
              <DropdownMenuItem
                disabled={!decidable}
                onSelect={() => down.mutate()}
              >
                <ArrowDown /> Move to the archive list
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <p className="mt-2 max-w-[65ch] text-muted">{card.summary}</p>

      {card.facts.length > 0 && (
        <dl className="mt-3 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
          {card.facts.map((f) => (
            <div key={f.label} className="contents">
              <dt className="text-neutral">{f.label}</dt>
              <dd className="text-ink">{f.value}</dd>
            </div>
          ))}
        </dl>
      )}

      {card.actions.length > 0 && (
        <ul className="mt-4 flex flex-col gap-2 border-t border-rule pt-3">
          {card.actions.map((a, i) => {
            const line = actionLine(a);
            const record = card.action_records[i];
            return (
              <li key={i} className="text-sm">
                <span className="text-ink">{line.verb}</span>
                {record &&
                  record.status !== "pending" &&
                  record.status !== "released" && (
                    <span
                      className={cn(
                        "ml-2 text-xs",
                        record.status === "failed"
                          ? "text-danger"
                          : "text-neutral",
                      )}
                    >
                      {record.status}
                    </span>
                  )}
                {line.detail && (
                  <p className="mt-0.5 line-clamp-3 whitespace-pre-line text-neutral">
                    {line.detail.replace(/\n\s*\n/g, "\n")}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      )}

      <p className="mt-3 text-xs text-neutral">
        {card.reason}
        {card.rule_ref && <> · {card.rule_ref}</>} · from{" "}
        {sessionName(card.posted_by)}
      </p>

      {offer && decidable && (
        <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 text-sm text-muted">
          <span className="min-w-0 flex-1">
            {offer.rule_ids.length === 1
              ? "A rule"
              : `${offer.rule_ids.length} rules`}{" "}
            for {offer.to.tab} were archived when its earlier session closed.
          </span>
          <Button
            size="sm"
            loading={move.isPending}
            onClick={() => move.mutate(offer)}
          >
            Move {offer.rule_ids.length === 1 ? "it" : "them"} to this session
          </Button>
        </div>
      )}

      {!suggestion && decidable && card.actions.some((a) => a.type !== "lease") && !ruling && (
        <Button
          variant="quiet"
          size="sm"
          className="mt-2 -ml-3"
          onClick={() => setRuling(true)}
        >
          Auto-approve matches in future
        </Button>
      )}
      {ruling && (
        <div className="mt-3 rounded-md border border-rule bg-paper p-3 sm:p-4">
          <RuleEditor
            initial={ruleFromCard(card)}
            onDone={() => setRuling(false)}
          />
        </div>
      )}

      {status && (
        <p
          className={cn(
            "mt-3 text-sm",
            status.tone === "danger" ? "text-danger" : "text-muted",
          )}
        >
          {status.text}
        </p>
      )}

      {card.state === "failed" ? (
        <Button
          className="mt-3"
          size="sm"
          loading={retry.isPending}
          onClick={() => retry.mutate()}
        >
          <RotateCcw aria-hidden /> Retry
        </Button>
      ) : (
        decidable && (
          <div className="mt-4 flex flex-col gap-2">
            {suggestion && (
              <label className="flex items-center gap-2 text-sm text-muted">
                <input
                  type="checkbox"
                  className="size-4 accent-(--color-accent)"
                  checked={draft.include}
                  onChange={(e) =>
                    onDraft({ ...draft, include: e.target.checked })
                  }
                />
                Include this suggestion in Approve All
              </label>
            )}
            <Textarea
              rows={1}
              placeholder={`A note for ${card.posted_by.tab}: changes it to your instruction`}
              value={draft.text}
              maxLength={4000}
              onChange={(e) => onDraft({ ...draft, text: e.target.value })}
              aria-label={`Note for ${card.posted_by.tab} on ${card.subject}`}
            />
          </div>
        )
      )}
      {error && (
        <p role="alert" className="mt-2 text-sm text-danger">
          {error.message}
        </p>
      )}
    </article>
  );
}
