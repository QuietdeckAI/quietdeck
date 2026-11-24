import { useMutation, useQuery } from "@tanstack/react-query";
import { decide } from "@/components/ActionCard";
import { useAccount } from "@/components/AccountTabs";
import { Button } from "@/components/ui/button";
import { Link } from "@/components/Header";
import { api, query } from "@/lib/api";
import { when } from "@/lib/format";
import type { Card } from "@/types/Card";

/** Threads left for later. A newer message in one brings it back as a new card. */
export function LaterScreen() {
  const account = useAccount();
  const later = useQuery({
    queryKey: ["later", account ?? null],
    queryFn: () => api<Card[]>(`/api/later${query({ account })}`),
  });
  return (
    <main className="mx-auto max-w-3xl px-4 py-6">
      <Link to="/threads" className="text-sm text-neutral hover:text-ink">
        ← Threads
      </Link>
      <h1 className="mt-3 text-2xl">Later</h1>
      {later.isError && <p className="mt-4 text-danger">{later.error.message}</p>}
      {later.data?.length === 0 && <p className="mt-6 text-neutral">Nothing is left for later.</p>}
      <ul className="mt-4">
        {later.data?.map((c) => (
          <LaterRow key={c.id} card={c} />
        ))}
      </ul>
    </main>
  );
}

function LaterRow({ card }: { card: Card }) {
  const back = useMutation({ mutationFn: () => decide(card, "move_up") });
  return (
    <li className="flex items-center gap-3 border-b border-rule py-2.5 last:border-b-0">
      <div className="min-w-0 flex-1">
        <p className="truncate">
          <span className="text-muted">{card.sender?.name ?? card.sender?.address ?? card.account}</span>
          <span className="text-neutral"> · </span>
          {card.subject}
        </p>
        <p className="truncate text-sm text-neutral">
          Left {when(card.updated_at)} · {card.summary}
        </p>
        {back.error && <p className="text-sm text-danger">{back.error.message}</p>}
      </div>
      <Button size="sm" loading={back.isPending} onClick={() => back.mutate()}>
        Bring back
      </Button>
    </li>
  );
}
