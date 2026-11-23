import { useMutation } from "@tanstack/react-query";
import { ArrowUp } from "lucide-react";
import { decide } from "@/components/ActionCard";
import { Button } from "@/components/ui/button";
import type { Card } from "@/types/Card";

/** One line: who, what, any amount, and Move up. "Handled elsewhere" lines carry the session's note. */
export function ArchiveLine({ card }: { card: Card }) {
  const up = useMutation({ mutationFn: () => decide(card, "move_up") });
  const amount = card.facts.find((f) => /amount|total|price/i.test(f.label))?.value;
  const elsewhere = card.state === "handled_elsewhere";
  return (
    <li className="arrive flex min-h-11 items-center gap-3 border-b border-rule py-2 last:border-b-0">
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm">
          <span className="text-muted">{card.sender?.name ?? card.sender?.address ?? card.account}</span>
          <span className="text-neutral"> · </span>
          <span className="text-ink">{card.subject}</span>
          {amount && <span className="ml-2 font-mono text-xs text-muted">{amount}</span>}
        </p>
        {elsewhere && card.recheck_note && (
          <p className="truncate text-xs text-neutral">Handled elsewhere: {card.recheck_note}</p>
        )}
        {up.error && <p className="text-xs text-danger">{up.error.message}</p>}
      </div>
      {!elsewhere && card.state === "open" && (
        <Button variant="quiet" size="sm" loading={up.isPending} onClick={() => up.mutate()}>
          <ArrowUp aria-hidden /> Move up
        </Button>
      )}
    </li>
  );
}
