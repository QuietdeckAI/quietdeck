import { ChevronRight } from "lucide-react";
import { useState } from "react";
import { actionsShort, type Draft } from "@/components/ActionCard";
import { Textarea } from "@/components/ui/input";
import { when } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { Card } from "@/types/Card";

/**
 * One line in the archive list: who and what. Approve All archives it unless he opens it and writes what to do
 * instead. Opened, it shows the time, the facts and that note box; nothing else on the line until then.
 */
export function ArchiveLine({
  card,
  draft,
  onDraft,
}: {
  card: Card;
  draft: Draft;
  onDraft: (d: Draft) => void;
}) {
  const [open, setOpen] = useState(false);
  const elsewhere = card.state === "handled_elsewhere";
  const sender = card.sender?.name ?? card.sender?.address ?? card.account;
  const noted = draft.text.trim().length > 0;
  // A plain archive goes without saying here; anything more (an unsubscribe reply, a label) is said in a few words.
  const plain = card.actions.every((a) => a.type === "archive");
  return (
    <li className={cn("arrive border-b border-rule last:border-b-0", open && "bg-paper-2")}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="flex w-full min-w-0 items-start gap-1 rounded-sm px-1 py-1 text-left text-sm leading-5 hover:bg-paper-3"
      >
        <ChevronRight
          aria-hidden
          className={cn(
            "mt-0.5 size-4 shrink-0 text-neutral transition-transform duration-(--dur-quick)",
            open && "rotate-90",
          )}
        />
        <span className={cn("min-w-0 flex-1", !open && "truncate")}>
          {sender !== card.account && (
            <>
              <span className="text-ink">{sender}</span>
              <span className="text-neutral"> · </span>
            </>
          )}
          <span className="text-muted">{card.summary || card.subject}</span>
          {noted ? (
            <span className="text-accent"> → your note</span>
          ) : (
            !plain && <span className="text-accent"> → {actionsShort(card)}</span>
          )}
          {elsewhere && <span className="text-neutral"> · handled elsewhere</span>}
        </span>
      </button>
      {open && (
        <div className="flex flex-col gap-2 pb-2 pl-6 pr-2 text-sm">
          <time className="font-mono text-xs text-neutral" dateTime={card.last_message_at ?? card.prepared_at}>
            {when(card.last_message_at ?? card.prepared_at)}
          </time>
          {elsewhere && card.recheck_note && <p className="text-muted">{card.recheck_note}</p>}
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
          {card.state === "open" && (
            <Textarea
              rows={1}
              placeholder="Archived unless you say what to do instead"
              value={draft.text}
              maxLength={4000}
              onChange={(e) => onDraft({ ...draft, text: e.target.value })}
              aria-label={`Instead of archiving ${card.subject}`}
              className="min-h-9 py-1.5 text-sm"
            />
          )}
        </div>
      )}
    </li>
  );
}
