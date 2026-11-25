import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useAccount } from "@/components/AccountTabs";
import { ApproveAllBar } from "@/components/ApproveAllBar";
import { Link } from "@/components/Header";
import { changedKeys, ReportBody, ReportChatBox, reportDefaults } from "@/components/Report";
import { ReportMeta, ReportToasts, settleUntil, UpdatedAt } from "@/components/ReportMeta";
import { TimelineRow } from "@/components/TimelineRow";
import { api, query } from "@/lib/api";
import { usePresence } from "@/lib/presence";
import type { ApproveAll } from "@/types/ApproveAll";
import type { Card } from "@/types/Card";
import type { CardDecision } from "@/types/CardDecision";
import type { Inbox } from "@/types/Inbox";
import type { Report } from "@/types/Report";

// The Inbox is the triage agent's report and nothing else. The cards under it have their own screen (Threads).

/** Every card the Inbox holds, by ID: the report's defaults name them. */
function cardsById(inbox: Inbox): Map<number, Card> {
  const all = new Map<number, Card>();
  for (const c of [...inbox.actions, ...inbox.suggestions, ...inbox.archive_list, ...inbox.sent]) all.set(Number(c.id), c);
  return all;
}

/** With a report: approve the cards of every default he didn't change. Cards the report doesn't name wait. */
function reportApproveRequest(inbox: Inbox, report: Report, changed: Set<string>): ApproveAll {
  const all = cardsById(inbox);
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

const APPROVED_KEY = "quietdeck.inbox.approved";

/** The cards being carried out, as saved for this report: none for another report. */
function loadApproved(reportId: number): { cards: Set<number>; version: number } | null {
  try {
    const saved = JSON.parse(localStorage.getItem(APPROVED_KEY) ?? "null") as {
      report: number;
      version: number;
      cards: number[];
    } | null;
    if (!saved || saved.report !== Number(reportId)) return null;
    return { cards: new Set(saved.cards), version: saved.version };
  } catch {
    return null;
  }
}

function saveApproved(reportId: number, value: { cards: Set<number>; version: number } | null) {
  if (value === null) localStorage.removeItem(APPROVED_KEY);
  else
    localStorage.setItem(
      APPROVED_KEY,
      JSON.stringify({ report: Number(reportId), version: value.version, cards: [...value.cards] }),
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
            The triage agent writes one on its next round. Every thread is on{" "}
            <Link to="/threads" className="text-ink underline underline-offset-4 hover:text-accent">
              Threads
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
  usePresence();
  // From the click until the approved actions have run: the report shows it's being dealt with.
  const [sending, setSending] = useState(false);
  const busy = sending || inbox.approving !== null || inbox.running;
  // The cards being carried out: the last Approve All's, and any card approved or running now (so a reload mid-run
  // still shows them). Kept as they finish, so each default shows a spinner, then a check. Cleared when the report
  // next changes after the work is over (the agent rewrites it for the next round).
  // Kept in this browser's storage, for this report, so the checks survive a reload until the reset.
  const [approved, setApprovedState] = useState<{ cards: Set<number>; version: number } | null>(() =>
    loadApproved(report.id),
  );
  const setApproved = (
    next:
      | { cards: Set<number>; version: number }
      | null
      | ((prev: { cards: Set<number>; version: number } | null) => { cards: Set<number>; version: number } | null),
  ) =>
    setApprovedState((prev) => {
      const value = typeof next === "function" ? next(prev) : next;
      saveApproved(report.id, value);
      return value;
    });
  const inFlight = [
    ...(inbox.approving?.cards.map(Number) ?? []),
    ...[...cardsById(inbox).values()]
      .filter((c) => c.state === "approved" || c.state === "executing")
      .map((c) => Number(c.id)),
  ]
    .sort((a, b) => a - b)
    .join(",");
  useEffect(() => {
    if (!inFlight) return;
    setApproved((prev) => {
      const cards = new Set(prev?.cards);
      for (const id of inFlight.split(",")) cards.add(Number(id));
      return { cards, version: report.version };
    });
  }, [inFlight, report.version]);
  useEffect(() => {
    if (!busy && !inFlight && approved && report.version !== approved.version) setApproved(null);
  }, [busy, inFlight, approved, report.version]);
  const setChange = (k: string, t: string) => setChanges((all) => ({ ...all, [k]: t }));
  const others = report.cards_not_in_report.length > 0;
  // A change typed but not sent goes to the agent before Approve All, so its cards stay out of the approval.
  const pending = Object.entries(changes).filter(([, t]) => t.trim());
  const changed = changedKeys(report);
  for (const [k] of pending) changed.add(k);
  const request = reportApproveRequest(inbox, report, changed);
  const byId = cardsById(inbox);
  const settle = settleUntil(
    report,
    request.cards.flatMap((d) => {
      const c = byId.get(Number(d.card_id));
      return c ? [c] : [];
    }),
  );
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
      <main className="mx-auto max-w-3xl px-4 pt-6 pb-4">
        <ReportMeta report={report} />
        <div className={`report-busy-wrap mt-4${busy ? " busy" : ""}`} aria-busy={busy}>
          <ReportBody report={report} cards={byId} drafts={changes} setDraft={setChange} approved={approved?.cards} />
        </div>
        {others && (
          <p className="mt-10 text-sm text-neutral">
            Some threads aren't in the report yet:{" "}
            <Link to="/threads" className="text-ink underline underline-offset-4 hover:text-accent">
              Threads
            </Link>
          </p>
        )}
        {inbox.timeline.length > 0 && <Automatic inbox={inbox} />}
      </main>
      <ReportChatBox
        by={report.posted_by}
        running={inbox.running}
        corner={<UpdatedAt at={report.updated_at} stacked />}
      >
        <ApproveAllBar inbox={inbox} request={request} beforeApprove={beforeApprove} settleUntil={settle} onPending={setSending} inline />
      </ReportChatBox>
      <ReportToasts report={report} presence={inbox.presence} />
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
