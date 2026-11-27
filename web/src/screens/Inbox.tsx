import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useAccount } from "@/components/AccountTabs";
import { ApproveAllBar } from "@/components/ApproveAllBar";
import { notify } from "@/lib/notes";
import { changedKeys, ReportBody, ReportChatBox, reportDefaults, TriageNotes, triageNotes } from "@/components/Report";
import { ReportMeta, ReportToasts, UpdatedAt } from "@/components/ReportMeta";
import { TimelineRow } from "@/components/TimelineRow";
import { api, query } from "@/lib/api";
import { usePresence } from "@/lib/presence";
import type { ApproveAll } from "@/types/ApproveAll";
import type { Approving } from "@/types/Approving";
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

/** The decision Approve All sends for a card: approve, or for a suggestion, its actions as they stand. */
function decisionFor(c: Card): CardDecision {
  return c.section === "suggestion"
    ? { card_id: c.id, version: c.version, kind: "edit", fields: c.actions, text: null }
    : { card_id: c.id, version: c.version, kind: "approve", fields: null, text: null };
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
      cards.push(decisionFor(c));
    }
  }
  return { cards, timeline: inbox.timeline.map((e) => e.id) };
}

const APPROVED_KEY = "quietdeck.inbox.approved";

/** The cards being carried out, as saved for this report: none for another report. */
function loadApproved(reportId: number, version: number): { cards: Set<number>; version: number } | null {
  try {
    const saved = JSON.parse(localStorage.getItem(APPROVED_KEY) ?? "null") as {
      report: number;
      version: number;
      cards: number[];
    } | null;
    // Another report, or a deck that started again (its report is at an earlier version): not ours.
    if (!saved || saved.report !== Number(reportId) || saved.version > Number(version)) return null;
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

/**
 * Still water at dusk, in thin grey lines: a sun setting on the horizon when a round is done, a crescent moon over the
 * same water while the Inbox waits for its first report. Decorative, so hidden from screen readers.
 */
function StillWater({ moon }: { moon?: boolean }) {
  return (
    <svg
      viewBox="0 0 120 64"
      className="mx-auto h-44 w-[330px] max-w-full text-neutral/45"
      fill="none"
      stroke="currentColor"
      strokeWidth="0.6"
      strokeLinecap="round"
      aria-hidden
    >
      {moon ? (
        <path d="M66 12a12 12 0 1 0 6 22.4A14 14 0 0 1 66 12Z" />
      ) : (
        <>
          <path d="M44 38a16 16 0 0 1 32 0" />
          {/* Seven rays every 22.5°, from 20 to 25 units out from the sun's centre. */}
          <path d="M41.52 30.35L36.90 28.43M45.86 23.86L42.32 20.32M52.35 19.52L50.43 14.90M60 18L60 13M67.65 19.52L69.57 14.90M74.14 23.86L77.68 20.32M78.48 30.35L83.10 28.43" />
        </>
      )}
      <path d="M14 38h92" />
      <path d="M30 46h28M66 46h22" opacity="0.8" />
      <path d="M40 53h16M64 53h14" opacity="0.6" />
      <path d="M52 59h14" opacity="0.4" />
    </svg>
  );
}

/** The Inbox's empty state: "No report yet." over a moon before the first report, "All done." over a setting sun
 * after a round is cleared. */
function NoReport({ cleared }: { cleared?: boolean }) {
  return (
    <div className="text-center">
      <p className="font-display text-2xl text-neutral">{cleared ? "All done." : "No report yet."}</p>
      <div className="mt-8">
        <StillWater moon={!cleared} />
      </div>
    </div>
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
    // Fills the screen below the header, so the empty state sits in the middle of the screen.
    <main className="mx-auto flex max-w-3xl flex-1 flex-col px-4 pt-6 pb-4">
      {inbox.isError && (
        <p role="alert" className="mt-8 text-danger">
          {inbox.error.message}
        </p>
      )}
      {inbox.data && (
        <>
          <div className="flex flex-1 items-center justify-center">
            <NoReport />
          </div>
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
  const byId = cardsById(inbox);
  // From the click until the approved actions have run: the report shows it's being dealt with.
  const [sending, setSending] = useState(false);
  // The cards being carried out: the last Approve All's, and any card approved or running now (so a reload mid-run
  // still shows them). Kept as they finish, so each default shows a spinner, then a check. Cleared when the report
  // next changes after the work is over (the agent rewrites it for the next round).
  // Kept in this browser's storage, for this report, so the checks survive a reload until the reset.
  const [approved, setApprovedState] = useState<{ cards: Set<number>; version: number } | null>(() =>
    loadApproved(report.id, report.version),
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
      .filter((c) => c.state === "approved" || c.state === "executing" || c.state === "handed_off")
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
  // Everything an Approve All set going, until it's all finished: cards waiting to be picked up, being carried out by
  // the runner, handed to the agent, or being carried out by it. One continuous state from the click to "All done.",
  // with no gap between the runner's part and the agent's. (Not the deck's "running" flag.)
  const unfinished = (id: number) => {
    const state = byId.get(id)?.state;
    return state === "approved" || state === "executing" || state === "handed_off" || state === "revising";
  };
  // Cards approved one at a time (a default's own Approve): they show their spinner and check like the rest, but the
  // report's darkening is Approve All's alone, so they don't count towards it.
  const [singles, setSingles] = useState<Set<number>>(() => new Set());
  const whole = (id: number) => !singles.has(id);
  const executing =
    [...(approved?.cards ?? [])].filter(whole).some(unfinished) ||
    inFlight.split(",").filter((id) => id !== "" && whole(Number(id))).length > 0;
  const approvingAll = inbox.approving !== null && inbox.approving.cards.map(Number).some(whole);
  const busy = sending || approvingAll || executing;
  // Carrying out the approved actions (the undo window is over): the sheen moves.
  const working = executing && !approvingAll && !sending;
  useEffect(() => {
    if (!busy && !inFlight && approved && report.version !== approved.version) setApproved(null);
  }, [busy, inFlight, approved, report.version]);
  const setChange = (k: string, t: string) => setChanges((all) => ({ ...all, [k]: t }));
  // A change typed but not sent goes to the agent before Approve All, so its cards stay out of the approval.
  const pending = Object.entries(changes).filter(([, t]) => t.trim());
  const changed = changedKeys(report);
  for (const [k] of pending) changed.add(k);
  // Everything the report names is finished and nothing more is running: a moment after the last check, all but the
  // heading rolls up and away, until the next report arrives.
  const named = reportDefaults(report.markdown).flatMap((d) => d.cards);
  const finished = (id: number) => {
    const state = byId.get(id)?.state ?? "done";
    return state === "done" || state === "automatic" || state === "handled_elsewhere";
  };
  // Nothing left to act on: every card the defaults name is finished, or there are no defaults or questions at all
  // (whatever text the agent left).
  // Or the agent said so (`quietdeck report done`), whatever text it left.
  const notes = triageNotes(report.markdown);
  const allDone =
    report.finished ||
    (named.every(finished) && !/<Ask\b/.test(report.markdown) && report.cards_not_in_report.length === 0);
  // "rolling" slides and fades the parts away; "gone" then removes them, so the space closes. A report that's already
  // done when the page opens (or when a new one arrives done) shows "All done." at once, without the roll.
  const [rolled, setRolled] = useState<"no" | "rolling" | "gone">(() => (allDone ? "gone" : "no"));
  const firstReport = useRef(true);
  useEffect(() => {
    if (firstReport.current) {
      firstReport.current = false;
      return;
    }
    setRolled(allDone ? "gone" : "no");
    // Only when another report arrives; allDone changing on this one is the roll below.
    // biome-ignore lint/correctness/useExhaustiveDependencies: keyed on the report alone
  }, [report.id]);
  const rolledNow = useRef(rolled);
  rolledNow.current = rolled;
  // After an Approve All the page stays dark through the end: the sheen fades, the report rolls up under the
  // darkening, then the darkening fades and "All done." comes in. A report that finishes any other way (the person
  // archived the last emails in Gmail) just rolls up.
  const ranApproveAll = useRef(false);
  if (busy) ranApproveAll.current = true;
  const [holdingDark, setHoldingDark] = useState(false);
  useEffect(() => {
    if (!allDone) {
      setRolled("no");
      setHoldingDark(false);
      if (!busy) ranApproveAll.current = false;
      return;
    }
    if (busy || rolledNow.current !== "no") return;
    const timers: number[] = [];
    const at = (ms: number, f: () => void) => timers.push(window.setTimeout(f, ms));
    if (ranApproveAll.current) {
      ranApproveAll.current = false;
      setHoldingDark(true);
      // The sheen's fade (400ms), the roll (its last part starts 320ms in and takes 450ms), the darkening's fade.
      at(500, () => setRolled("rolling"));
      at(500 + 800, () => setHoldingDark(false));
      at(500 + 800 + 400, () => setRolled("gone"));
    } else {
      at(1200, () => setRolled("rolling"));
      at(1200 + 800, () => setRolled("gone"));
    }
    return () => {
      for (const t of timers) window.clearTimeout(t);
    };
  }, [allDone, busy]);
  const request = reportApproveRequest(inbox, report, changed);
  const qc = useQueryClient();
  // One default's Approve: its cards go through Approve All's path (the same undo window, Undo in the bar), alone.
  const approveOne = async (ids: number[]) => {
    const cards = ids.flatMap((id) => {
      const c = byId.get(id);
      return c && (c.state === "open" || c.state === "handed_off") ? [decisionFor(c)] : [];
    });
    if (cards.length === 0) return null;
    setSingles((prev) => new Set([...prev, ...ids]));
    const a = await api<Approving>("/api/decisions/approve-all", { method: "POST", body: { cards, timeline: [], single: true } });
    // Only what the deck took counts as approved; what it left alone stays, and says why.
    const taken = a.cards.map(Number);
    setSingles((prev) => new Set([...prev].filter((id) => !ids.includes(id) || taken.includes(id))));
    setApproved((prev) => ({ cards: new Set([...(prev?.cards ?? []), ...taken]), version: report.version }));
    if (a.skipped.length > 0) {
      const reasons = a.skipped.map((s) => s.reason).filter((r, i, all) => all.indexOf(r) === i);
      notify(`Left alone: ${reasons.join("; ")}`, { ms: 8000 });
    }
    await qc.invalidateQueries({ queryKey: ["inbox"] });
    return a.cards.length > 0 ? a : null;
  };
  const undoBatch = async (batch: string) => {
    await api("/api/decisions/undo", { method: "POST", body: { batch } });
    await qc.invalidateQueries({ queryKey: ["inbox"] });
  };
  const beforeApprove = async () => {
    // Undone singles that Approve All now takes count towards its darkening again.
    const taken = new Set(request.cards.map((c) => Number(c.card_id)));
    setSingles((prev) => new Set([...prev].filter((id) => !taken.has(id))));
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
    // Fills the rest of the screen under the header (App's column), and the report's part grows to fill it: with
    // little on the page (the empty state) the bar still sits exactly at the bottom, as it does when the report is
    // long and scrolls.
    <div className="flex flex-1 flex-col">
      {/* overflow-clip: the busy darkening reaches a little past the report, inside this padding, and mustn't add
          scroll to the page. */}
      <main className="mx-auto flex w-full max-w-3xl flex-1 flex-col overflow-clip px-4 pt-6 pb-4">
        <ReportMeta report={report} />
        <div
          className={`report-busy-wrap mt-4${busy || holdingDark ? " busy" : ""}${working ? " running" : ""}`}
          aria-busy={busy || holdingDark}
        >
          <ReportBody
            report={report}
            cards={byId}
            drafts={changes}
            setDraft={setChange}
            approved={approved?.cards}
            rolled={rolled}
            approveOne={approveOne}
            undoBatch={undoBatch}
            changed={changed}
            finished={new Set(inbox.finished_cards.map(Number))}
          />
        </div>
        {rolled === "gone" && (
          // Fills the space the report left, with "All done." in its middle: the middle of the screen, near enough.
          <div className="arrive flex flex-1 flex-col items-center justify-center">
            {notes && (
              <div className="w-full self-stretch">
                <TriageNotes markdown={notes} />
              </div>
            )}
            <NoReport cleared />
          </div>
        )}
        {inbox.timeline.length > 0 && <Automatic inbox={inbox} />}
      </main>
      <ReportChatBox
        by={report.posted_by}
        running={inbox.running}
        corner={<UpdatedAt at={report.updated_at} stacked />}
      >
        <ApproveAllBar inbox={inbox} request={request} beforeApprove={beforeApprove} onPending={setSending} inline />
      </ReportChatBox>
      <ReportToasts report={report} presence={inbox.presence} />
    </div>
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
