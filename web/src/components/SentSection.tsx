import { ChevronRight } from "lucide-react";
import { useState } from "react";
import { when } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { Card } from "@/types/Card";

/** Sent mail nobody needs to hear about: one collapsed line, one row each when opened. */
export function SentSection({ cards }: { cards: Card[] }) {
  const [open, setOpen] = useState(false);
  if (cards.length === 0) return null;
  return (
    <section className="mt-10">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="flex items-center gap-1 rounded-sm text-sm text-neutral transition-colors hover:text-ink"
      >
        <ChevronRight
          aria-hidden
          className={cn("size-4 transition-transform duration-(--dur-quick)", open && "rotate-90")}
        />
        Sent ({cards.length} ignored)
      </button>
      {open && (
        <ul className="mt-2">
          {cards.map((c) => (
            <li key={c.id} className="flex items-baseline gap-3 border-b border-rule py-2 text-sm last:border-b-0">
              <span className="w-20 shrink-0 text-neutral">{c.account}</span>
              <span className="min-w-0 flex-1 truncate">
                <span className="text-muted">{c.sender?.address ?? ""}</span>
                <span className="text-neutral"> · </span>
                {c.subject}
              </span>
              <time className="shrink-0 font-mono text-xs text-neutral">{when(c.last_message_at ?? c.prepared_at)}</time>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
