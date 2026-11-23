import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { AccountTabs, useAccount } from "@/components/AccountTabs";
import { ActionCard, type Draft } from "@/components/ActionCard";
import { ApproveAllBar } from "@/components/ApproveAllBar";
import { ArchiveLine } from "@/components/ArchiveLine";
import { Link } from "@/components/Header";
import { SentSection } from "@/components/SentSection";
import { TimelineRow } from "@/components/TimelineRow";
import { api, query } from "@/lib/api";
import { ago } from "@/lib/format";
import type { ApproveAll } from "@/types/ApproveAll";
import type { Card } from "@/types/Card";
import type { CardDecision } from "@/types/CardDecision";
import type { Inbox } from "@/types/Inbox";

const EMPTY: Draft = { text: "", include: false };
const key = (c: Card) => `${c.id}:${c.version}`;

/** Unmentioned means yes: every card he didn't change goes in as approved, and a typed note makes it an adjustment. */
function approveAllRequest(inbox: Inbox, drafts: Record<string, Draft>): ApproveAll {
  const cards: CardDecision[] = [];
  const add = (c: Card, includeIfUntouched: boolean) => {
    if (c.state !== "open" && c.state !== "handed_off" && c.state !== "handled_elsewhere") return;
    const d = drafts[key(c)] ?? EMPTY;
    const text = d.text.trim();
    if (!text && !includeIfUntouched) return;
    if (!text && c.section === "actions" && c.actions.length === 0) return;
    cards.push(
      text
        ? { card_id: c.id, version: c.version, kind: "adjust", fields: null, text }
        : c.section === "suggestion"
          ? { card_id: c.id, version: c.version, kind: "edit", fields: c.actions, text: null }
          : { card_id: c.id, version: c.version, kind: "approve", fields: null, text: null },
    );
  };
  for (const c of inbox.actions) add(c, true);
  for (const c of inbox.suggestions) add(c, (drafts[key(c)] ?? EMPTY).include);
  for (const c of inbox.archive_list) add(c, true);
  for (const c of inbox.sent) add(c, true);
  return { cards, timeline: inbox.timeline.map((e) => e.id) };
}

export function InboxScreen() {
  const account = useAccount();
  const inbox = useQuery({
    queryKey: ["inbox", account ?? null],
    queryFn: () => api<Inbox>(`/api/inbox${query({ account })}`),
  });
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});

  return (
    <>
      <main className="mx-auto max-w-3xl px-4 pt-6 pb-4">
        <AccountTabs />
        {inbox.isError && (
          <p role="alert" className="mt-8 text-danger">
            {inbox.error.message}
          </p>
        )}
        {inbox.data && <InboxBody inbox={inbox.data} drafts={drafts} setDrafts={setDrafts} />}
      </main>
      {inbox.data && <ApproveAllBar inbox={inbox.data} request={approveAllRequest(inbox.data, drafts)} />}
    </>
  );
}

function InboxBody({
  inbox,
  drafts,
  setDrafts,
}: {
  inbox: Inbox;
  drafts: Record<string, Draft>;
  setDrafts: React.Dispatch<React.SetStateAction<Record<string, Draft>>>;
}) {
  const draftFor = (c: Card) => drafts[key(c)] ?? EMPTY;
  const setDraft = (c: Card) => (d: Draft) => setDrafts((all) => ({ ...all, [key(c)]: d }));
  const quietSince =
    inbox.last_agent_at && Date.now() - new Date(inbox.last_agent_at).getTime() > 3_600_000
      ? inbox.last_agent_at
      : null;
  const empty =
    inbox.actions.length + inbox.suggestions.length + inbox.archive_list.length + inbox.timeline.length === 0;

  return (
    <>
      {quietSince && <p className="mt-6 text-sm text-neutral">The triage agent last checked in {ago(quietSince)}.</p>}

      {empty && <p className="mt-16 text-center font-display text-xl text-neutral">Nothing waiting.</p>}

      {inbox.actions.length > 0 && (
        <section className="mt-6 flex flex-col gap-4" aria-label="Actions">
          {inbox.actions.map((c) => (
            <ActionCard key={key(c)} card={c} draft={draftFor(c)} onDraft={setDraft(c)} />
          ))}
        </section>
      )}

      {inbox.suggestions.length > 0 && (
        <section className="mt-10">
          <h2 className="mb-3 text-lg text-muted">Suggestions</h2>
          <div className="flex flex-col gap-4">
            {inbox.suggestions.map((c) => (
              <ActionCard key={key(c)} card={c} draft={draftFor(c)} onDraft={setDraft(c)} suggestion />
            ))}
          </div>
        </section>
      )}

      {inbox.archive_list.length > 0 && (
        <section className="mt-10">
          <h2 className="mb-1 text-lg text-muted">Archive list</h2>
          <ul>
            {inbox.archive_list.map((c) => (
              <ArchiveLine key={key(c)} card={c} />
            ))}
          </ul>
        </section>
      )}

      {inbox.timeline.length > 0 && (
        <section className="mt-10">
          <h2 className="mb-1 text-lg text-muted">Automatic actions</h2>
          <ul>
            {inbox.timeline.map((e) => (
              <TimelineRow key={e.id} entry={e} />
            ))}
          </ul>
        </section>
      )}

      {inbox.has_later && (
        <p className="mt-10">
          <Link to="/later" className="text-sm text-neutral underline-offset-4 hover:text-ink hover:underline">
            Later
          </Link>
        </p>
      )}

      <SentSection cards={inbox.sent} />
    </>
  );
}
