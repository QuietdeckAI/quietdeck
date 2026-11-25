import { useQuery } from "@tanstack/react-query";
import { Link } from "@/components/Header";
import { ReportSnapshotBody } from "@/components/Report";
import { api } from "@/lib/api";
import { clock, dayHeading } from "@/lib/format";
import type { Card } from "@/types/Card";
import type { ReportSnapshot } from "@/types/ReportSnapshot";

/** The Inbox document as it was when he pressed Approve All, with the changes and answers he gave that round. */
export function SnapshotScreen({ id }: { id: number }) {
  const snap = useQuery({
    queryKey: ["snapshot", id],
    queryFn: () => api<ReportSnapshot>(`/api/report/snapshots/${id}`),
    staleTime: Infinity,
  });
  const s = snap.data;
  const cards = new Map<number, Card>((s?.cards ?? []).map((c) => [Number(c.id), c]));
  return (
    <main className="mx-auto max-w-3xl px-4 py-6">
      <Link to="/history?view=inbox" className="text-sm text-neutral hover:text-ink">
        ← History
      </Link>
      {snap.isError && <p className="mt-4 text-danger">{snap.error.message}</p>}
      {s && (
        <>
          <p className="mt-3 text-sm text-neutral">
            The Inbox as it was when you pressed Approve All: {dayHeading(s.taken_at)}, {clock(s.taken_at)}. {s.approved}{" "}
            {s.approved === 1 ? "card" : "cards"} approved.
          </p>
          <div className="mt-6">
            <ReportSnapshotBody report={s.report} cards={cards} />
          </div>
          {s.round_answers.length > 0 && (
            <section className="mt-10 border-t border-rule pt-6">
              <h2 className="text-lg text-muted">What you said this round</h2>
              <ul className="mt-3 flex flex-col gap-3">
                {s.round_answers.map((a) => (
                  <li key={a.id} className="flex items-baseline gap-3">
                    <time className="w-16 shrink-0 font-mono text-xs text-neutral">
                      {clock(a.answered_at)}
                    </time>
                    <div className="min-w-0 flex-1">
                      <p className="text-sm text-muted">{a.about}</p>
                      <p className="text-accent">{a.value}</p>
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </>
      )}
    </main>
  );
}
