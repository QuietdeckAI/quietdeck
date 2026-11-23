import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useAccount } from "@/components/AccountTabs";
import { ApproveAllBar } from "@/components/ApproveAllBar";
import { Link } from "@/components/Header";
import { changedKeys, ReportBody, ReportChatBox, reportDefaults } from "@/components/Report";
import { TimelineRow } from "@/components/TimelineRow";
import { api, query } from "@/lib/api";
import type { ApproveAll } from "@/types/ApproveAll";
import type { Card } from "@/types/Card";
import type { CardDecision } from "@/types/CardDecision";
import type { Inbox } from "@/types/Inbox";
import type { Report } from "@/types/Report";

// The Inbox is the triage agent's report and nothing else. The cards under it have their own screen (Cards).

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

/** Whether open cards wait that the report doesn't name: the Inbox then points at the Cards screen. */
function hasUnnamed(inbox: Inbox, report: Report): boolean {
  const named = new Set(reportDefaults(report.markdown).flatMap((d) => d.cards));
  return [...inbox.actions, ...inbox.suggestions, ...inbox.archive_list].some(
    (c) => !named.has(Number(c.id)) && (c.state === "open" || c.state === "handed_off" || c.state === "failed"),
  );
}

export function InboxScreen() {
  const account = useAccount();
  const inbox = useQuery({
    queryKey: ["inbox", account ?? null],
    queryFn: () => api<Inbox>(`/api/inbox${query({ account })}`),
  });
  const [changes, setChanges] = useState<Record<string, string>>({});
  const report = inbox.data?.report ?? null;

  if (inbox.data && report) {
    return <ReportScreen inbox={inbox.data} report={report} changes={changes} setChanges={setChanges} />;
  }
  return (
    <main className="mx-auto max-w-3xl px-4 pt-6 pb-4">
      {inbox.isError && (
        <p role="alert" className="mt-8 text-danger">
          {inbox.error.message}
        </p>
      )}
      {inbox.data && (
        <>
          <p className="mt-16 text-center font-display text-xl text-neutral">No report yet.</p>
          <p className="mt-3 text-center text-sm text-neutral">
            The triage agent writes one on its next round. Every card is on{" "}
            <Link to="/cards" className="text-ink underline underline-offset-4 hover:text-accent">
              Cards
            </Link>
            .
          </p>
          {inbox.data.timeline.length > 0 && <Automatic inbox={inbox.data} />}
        </>
      )}
    </main>
  );
}

function ReportScreen({
  inbox,
  report,
  changes,
  setChanges,
}: {
  inbox: Inbox;
  report: Report;
  changes: Record<string, string>;
  setChanges: React.Dispatch<React.SetStateAction<Record<string, string>>>;
}) {
  const setChange = (k: string, t: string) => setChanges((all) => ({ ...all, [k]: t }));
  const others = hasUnnamed(inbox, report);
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
        {others && (
          <p className="mt-10 text-sm text-neutral">
            Some cards aren't in the report yet:{" "}
            <Link to="/cards" className="text-ink underline underline-offset-4 hover:text-accent">
              Cards
            </Link>
          </p>
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
