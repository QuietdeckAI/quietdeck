import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { AccountTabs, useAccount } from "@/components/AccountTabs";
import { ActionCard, type Draft } from "@/components/ActionCard";
import { ApproveAllBar } from "@/components/ApproveAllBar";
import { ArchiveLine } from "@/components/ArchiveLine";
import { Link } from "@/components/Header";
import { changedKeys, ReportBody, ReportChatBox, reportDefaults } from "@/components/Report";
import { SentSection } from "@/components/SentSection";
import { TimelineRow } from "@/components/TimelineRow";
import { api, query } from "@/lib/api";
import { ago } from "@/lib/format";
import type { ApproveAll } from "@/types/ApproveAll";
import type { Card } from "@/types/Card";
import type { CardDecision } from "@/types/CardDecision";
import type { Inbox } from "@/types/Inbox";
import type { Report } from "@/types/Report";

const EMPTY: Draft = { text: "", include: false };
const key = (c: Card) => `${c.id}:${c.version}`;

/** Unmentioned means yes: every card he didn't change goes in as approved, and a typed note makes it an adjustment. */
function approveAllRequest(
  inbox: Inbox,
  drafts: Record<string, Draft>,
): ApproveAll {
  const cards: CardDecision[] = [];
  const add = (c: Card, includeIfUntouched: boolean) => {
    if (
      c.state !== "open" &&
      c.state !== "handed_off" &&
      c.state !== "handled_elsewhere"
    )
      return;
    const d = drafts[key(c)] ?? EMPTY;
    const text = d.text.trim();
    if (!text && !includeIfUntouched) return;
    if (!text && c.section === "actions" && c.actions.length === 0) return;
    cards.push(
      text
        ? {
            card_id: c.id,
            version: c.version,
            kind: "adjust",
            fields: null,
            text,
          }
        : c.section === "suggestion"
          ? {
              card_id: c.id,
              version: c.version,
              kind: "edit",
              fields: c.actions,
              text: null,
            }
          : {
              card_id: c.id,
              version: c.version,
              kind: "approve",
              fields: null,
              text: null,
            },
    );
  };
  for (const c of inbox.actions) add(c, true);
  for (const c of inbox.suggestions) add(c, (drafts[key(c)] ?? EMPTY).include);
  for (const c of inbox.archive_list) add(c, true);
  for (const c of inbox.sent) add(c, true);
  return { cards, timeline: inbox.timeline.map((e) => e.id) };
}

/** With a report: approve the cards of every default he didn't change. Cards the report doesn't name wait. */
function reportApproveRequest(inbox: Inbox, report: Report, changed: Set<string>): ApproveAll {
  const all = new Map<number, Card>();
  for (const c of [...inbox.actions, ...inbox.suggestions, ...inbox.archive_list, ...inbox.sent]) all.set(Number(c.id), c);
  const seen = new Set<number>();
  const cards: CardDecision[] = [];
  for (const d of reportDefaults(report.markdown)) {
    if (changed.has(d.key)) continue;
    for (const id of d.cards) {
      const c = all.get(id);
      if (!c || seen.has(id) || (c.state !== "open" && c.state !== "handed_off")) continue;
      seen.add(id);
      cards.push(
        c.section === "suggestion"
          ? { card_id: c.id, version: c.version, kind: "edit", fields: c.actions, text: null }
          : { card_id: c.id, version: c.version, kind: "approve", fields: null, text: null },
      );
    }
  }
  return { cards, timeline: inbox.timeline.map((e) => e.id) };
}

/** Cards the report doesn't name yet: shown under it, one line each, so nothing waits unseen. */
function unnamed(inbox: Inbox, report: Report): Card[] {
  const named = new Set(reportDefaults(report.markdown).flatMap((d) => d.cards));
  return [...inbox.actions, ...inbox.suggestions, ...inbox.archive_list].filter(
    (c) => !named.has(Number(c.id)) && (c.state === "open" || c.state === "handed_off" || c.state === "failed"),
  );
}

export function InboxScreen() {
  const account = useAccount();
  const inbox = useQuery({
    queryKey: ["inbox", account ?? null],
    queryFn: () => api<Inbox>(`/api/inbox${query({ account })}`),
  });
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [changes, setChanges] = useState<Record<string, string>>({});
  const report = inbox.data?.report ?? null;

  if (inbox.data && report) {
    return (
      <ReportScreen
        inbox={inbox.data}
        report={report}
        changes={changes}
        setChanges={setChanges}
        drafts={drafts}
        setDrafts={setDrafts}
      />
    );
  }

  return (
    <>
      <main className="mx-auto max-w-5xl px-4 pt-6 pb-4">
        <AccountTabs />
        {inbox.isError && (
          <p role="alert" className="mt-8 text-danger">
            {inbox.error.message}
          </p>
        )}
        {inbox.data && (
          <InboxBody
            inbox={inbox.data}
            drafts={drafts}
            setDrafts={setDrafts}
            showAccount={!account}
          />
        )}
      </main>
      {inbox.data && (
        <ApproveAllBar
          inbox={inbox.data}
          request={approveAllRequest(inbox.data, drafts)}
        />
      )}
    </>
  );
}

function InboxBody({
  inbox,
  drafts,
  setDrafts,
  showAccount,
}: {
  inbox: Inbox;
  drafts: Record<string, Draft>;
  setDrafts: React.Dispatch<React.SetStateAction<Record<string, Draft>>>;
  showAccount: boolean;
}) {
  const draftFor = (c: Card) => drafts[key(c)] ?? EMPTY;
  const setDraft = (c: Card) => (d: Draft) =>
    setDrafts((all) => ({ ...all, [key(c)]: d }));
  const quietSince =
    inbox.last_agent_at &&
    Date.now() - new Date(inbox.last_agent_at).getTime() > 3_600_000
      ? inbox.last_agent_at
      : null;
  const empty =
    inbox.actions.length +
      inbox.suggestions.length +
      inbox.archive_list.length +
      inbox.timeline.length ===
    0;

  return (
    <>
      {quietSince && (
        <p className="mt-6 text-sm text-neutral">
          The triage agent last checked in {ago(quietSince)}.
        </p>
      )}

      {empty && (
        <p className="mt-16 text-center font-display text-xl text-neutral">
          Nothing waiting.
        </p>
      )}

      {inbox.actions.length > 0 && (
        <section className="mt-6" aria-label="Actions">
          <ul>
            {inbox.actions.map((c) => (
              <ActionCard
                key={key(c)}
                card={c}
                draft={draftFor(c)}
                onDraft={setDraft(c)}
                offer={inbox.rule_offers.find((o) => o.card_id === c.id)}
                showAccount={showAccount}
              />
            ))}
          </ul>
        </section>
      )}

      {inbox.suggestions.length > 0 && (
        <section className="mt-8">
          <h2 className="mb-1 text-lg text-muted">Suggestions</h2>
          <ul>
            {inbox.suggestions.map((c) => (
              <ActionCard
                key={key(c)}
                card={c}
                draft={draftFor(c)}
                onDraft={setDraft(c)}
                suggestion
                showAccount={showAccount}
              />
            ))}
          </ul>
        </section>
      )}

      {inbox.archive_list.length > 0 && (
        <section className="mt-8">
          <h2 className="mb-1 text-lg text-muted">Archive list</h2>
          <ul>
            {inbox.archive_list.map((c) => (
              <ArchiveLine key={key(c)} card={c} draft={draftFor(c)} onDraft={setDraft(c)} />
            ))}
          </ul>
        </section>
      )}

      {inbox.timeline.length > 0 && (
        <section className="mt-8">
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
          <Link
            to="/later"
            className="text-sm text-neutral underline-offset-4 hover:text-ink hover:underline"
          >
            Later
          </Link>
        </p>
      )}

      <SentSection cards={inbox.sent} />
    </>
  );
}

function ReportScreen({
  inbox,
  report,
  changes,
  setChanges,
  drafts,
  setDrafts,
}: {
  inbox: Inbox;
  report: Report;
  changes: Record<string, string>;
  setChanges: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  drafts: Record<string, Draft>;
  setDrafts: React.Dispatch<React.SetStateAction<Record<string, Draft>>>;
}) {
  const setChange = (k: string, t: string) => setChanges((all) => ({ ...all, [k]: t }));
  const others = unnamed(inbox, report);
  // A change typed but not sent goes to the agent before Approve All, so its cards stay out of the approval.
  const pending = Object.entries(changes).filter(([, t]) => t.trim());
  const changed = changedKeys(report);
  for (const [k] of pending) changed.add(k);
  const request = reportApproveRequest(inbox, report, changed);
  const beforeApprove = async () => {
    const defaults = reportDefaults(report.markdown);
    for (const [k, t] of pending) {
      await api("/api/report/answers", {
        method: "POST",
        body: {
          report_id: report.id,
          report_version: report.version,
          control: k,
          kind: "change",
          about: defaults.find((d) => d.key === k)?.about ?? "",
          value: t.trim(),
        },
      });
    }
    setChanges({});
  };
  return (
    <>
      <main className="mx-auto max-w-3xl px-4 pt-4 pb-4">
        <ApproveAllBar inbox={inbox} request={request} beforeApprove={beforeApprove} top />
        <div className="mt-6">
          <ReportBody report={report} drafts={changes} setDraft={setChange} />
        </div>
        {others.length > 0 && (
          <section className="mt-10">
            <h2 className="mb-1 text-lg text-muted">Not in the report yet</h2>
            <ul>
              {others.map((c) => (
                <ActionCard
                  key={key(c)}
                  card={c}
                  draft={drafts[key(c)] ?? EMPTY}
                  onDraft={(d) => setDrafts((all) => ({ ...all, [key(c)]: d }))}
                  showAccount
                />
              ))}
            </ul>
          </section>
        )}
        {inbox.timeline.length > 0 && <Automatic inbox={inbox} />}
      </main>
      <ReportChatBox agent={report.posted_by.tab} />
    </>
  );
}

function Automatic({ inbox }: { inbox: Inbox }) {
  const [open, setOpen] = useState(false);
  return (
    <section className="mt-10">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="text-sm text-neutral underline-offset-4 hover:text-ink hover:underline"
      >
        {open ? "Hide what ran automatically" : "What ran automatically"}
      </button>
      {open && (
        <ul className="mt-2">
          {inbox.timeline.map((e) => (
            <TimelineRow key={e.id} entry={e} />
          ))}
        </ul>
      )}
    </section>
  );
}
