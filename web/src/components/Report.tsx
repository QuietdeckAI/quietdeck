import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Markdown as Render } from "markdown-to-jsx/react";
import { Check, CircleHelp, Loader2, MessageSquare, MessageSquareText, Square, Undo2 } from "lucide-react";
import { Children, createContext, Fragment, isValidElement, type ReactNode, use, useEffect, useRef, useState } from "react";
import { useInfo } from "@/components/AccountTabs";
import { EmailMenu } from "@/components/EmailMenu";
import { SessionMenu, useMarkSession } from "@/components/SessionMenu";
import { useSessionMarks } from "@/lib/documents";
import type { Document } from "@/types/Document";
import type { SessionMark } from "@/types/SessionMark";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import type { Settings } from "@/types/Settings";
import { Textarea } from "@/components/ui/input";
import { api } from "@/lib/api";
import { clock } from "@/lib/format";
import { cn, enterSends } from "@/lib/utils";
import type { Approving } from "@/types/Approving";
import type { Card } from "@/types/Card";
import type { Delivered } from "@/types/Delivered";
import { dismissNote, notify, replaceNote } from "@/lib/notes";
import { sessionKey, sessionPath } from "@/screens/Sessions";
import type { SessionRow } from "@/types/SessionRow";
import type { Report } from "@/types/Report";
import type { SessionRef } from "@/types/SessionRef";
import type { ReportAnswer } from "@/types/ReportAnswer";
import type { ReportAnswerAdd } from "@/types/ReportAnswerAdd";

/** A `<Default>`'s identity: its id, or else its cards. Changes and Approve All key on it. */
export const defaultKey = (id: string | undefined, cards: string | undefined) => id ?? `cards:${cards ?? ""}`;

/** The cards each `<Default>` in the text names, keyed by defaultKey. */
export function reportDefaults(markdown: string): { key: string; about: string; cards: number[] }[] {
  const out: { key: string; about: string; cards: number[] }[] = [];
  const tag = /<Default\b([^>]*)>([^<]*)<\/Default>/g;
  for (const m of markdown.matchAll(tag)) {
    const attrs = m[1] ?? "";
    const cards = /\bcards="([^"]*)"/.exec(attrs)?.[1];
    const id = /\bid="([^"]*)"/.exec(attrs)?.[1];
    out.push({
      key: defaultKey(id, cards),
      about: (m[2] ?? "").trim(),
      cards: (cards ?? "")
        .split(/[\s,]+/)
        .filter(Boolean)
        .map((c) => Number(c.replace(/^c_/, "")))
        .filter((n) => Number.isInteger(n)),
    });
  }
  return out;
}

/** The defaults the person changed on this version of the report: Approve All leaves their cards to the agent. */
export function changedKeys(report: Report): Set<string> {
  return new Set(
    report.answers.filter((a) => a.kind === "change" && a.report_version === report.version).map((a) => a.control),
  );
}

/** The sessions running now. The deck's event stream keeps them current. */
function useRunningSessions() {
  return useQuery({ queryKey: ["sessions", "running"], queryFn: () => api<SessionRow[]>("/api/sessions") });
}

/** The session that writes the report, if it's running. */
export function useReportAgent(by: SessionRef): SessionRow | undefined {
  const sessions = useRunningSessions().data ?? [];
  return sessions.find(
    (s) => s.node === by.node && s.tab === by.tab && (by.session_id === null || s.session_id === by.session_id),
  );
}

/**
 * The deck's undo window for what the person sends: `start` runs the send after it (Settings, 3 seconds by default),
 * and `cancel` takes it back. A send still waiting when the page goes is sent then.
 */
function useUndoWindow() {
  const seconds = useInfo().data?.settings.undo_seconds ?? 3;
  const timer = useRef<number | undefined>(undefined);
  const due = useRef<(() => void) | null>(null);
  const [pending, setPending] = useState(false);
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
      due.current?.();
    },
    [],
  );
  const start = (fire: () => void) => {
    window.clearTimeout(timer.current);
    if (seconds <= 0) return fire();
    due.current = fire;
    setPending(true);
    timer.current = window.setTimeout(() => {
      due.current = null;
      setPending(false);
      fire();
    }, seconds * 1000);
  };
  const cancel = () => {
    window.clearTimeout(timer.current);
    due.current = null;
    setPending(false);
  };
  return { pending, start, cancel };
}

function UndoButton({ onClick, label }: { onClick: () => void; label: string }) {
  return (
    <button type="button" onClick={onClick} aria-label={label} className={smallButton}>
      <Undo2 aria-hidden />
      Undo
    </button>
  );
}

// The panel's buttons, in the Threads list's quiet style: no outline, a wash on hover, a 1px dip when pressed.
const smallButton =
  "inline-flex h-7 shrink-0 items-center justify-center gap-1 rounded-md px-1.5 text-xs text-muted " +
  "transition-[background-color,color,transform] duration-(--dur-quick) ease-(--ease-out) hover:bg-paper-3 " +
  "hover:text-ink active:translate-y-px disabled:opacity-45 aria-expanded:bg-paper-3 aria-expanded:text-ink " +
  "[&_svg]:size-4 [&_svg]:shrink-0";

type AgentState = "working" | "idle" | "blocked" | "offline";

function agentState(agent: SessionRow | undefined): AgentState {
  if (!agent?.running || !agent.node_online) return "offline";
  if (agent.status === "working") return "working";
  if (agent.status === "blocked") return "blocked";
  return "idle";
}

/** Bottom left of the chat bar: the agent's dot and what it's doing. Green idle, yellow working. */
export function AgentStatus({ by }: { by: SessionRef }) {
  const state = agentState(useReportAgent(by));
  const words = { working: "Working", idle: "Idle", blocked: "Waiting on you", offline: "Offline" }[state];
  return (
    <span
      role="status"
      title={`${by.tab} on ${by.node}: ${words.toLowerCase()}`}
      className="inline-flex h-10 shrink-0 items-center gap-1.5 rounded-md border border-rule px-2.5 text-xs text-muted"
    >
      <span
        aria-hidden
        className={cn(
          "size-2 rounded-full",
          state === "idle" && "bg-ok",
          state === "working" && "bg-busy motion-safe:animate-pulse",
          state === "blocked" && "bg-busy",
          state === "offline" && "bg-neutral",
        )}
      />
      <span className="max-sm:sr-only">{words}</span>
    </span>
  );
}

/**
 * Whether the agent is on this answer now: it's working, and started after the answer reached it. Work it began
 * earlier is something else; it reads the answer after that turn, which starts working again.
 */
function useWorkingOn(answer: ReportAnswer | undefined): boolean {
  const { report } = useReport();
  const agent = useReportAgent(report.posted_by);
  if (!answer?.picked_up || agentState(agent) !== "working" || !agent) return false;
  return Date.parse(agent.status_changed_at) >= Date.parse(answer.answered_at) - 5000;
}

/** Under a change or an answer: whether it reached the agent, and whether the agent is on it. */
function SentStatus({ answer }: { answer: ReportAnswer }) {
  const { report } = useReport();
  const working = useWorkingOn(answer);
  const tab = report.posted_by.tab;
  if (!answer.picked_up)
    return <span className="block text-sm text-neutral">Not sent to {tab} yet: it gets this when it's next online.</span>;
  return (
    <span className="block text-sm text-neutral">
      {working ? (
        <>
          {tab} is working on it
          <InlineSpinner />
        </>
      ) : (
        `Sent to ${tab}.`
      )}
    </span>
  );
}

/** Right after "… is working on it": the agent is on it now. */
function InlineSpinner() {
  return <Loader2 aria-hidden className="ml-1.5 inline-block size-3 animate-spin align-[-1px]" />;
}

type Ctx = {
  report: Report;
  /** The cards the report's defaults name, by ID. */
  cards: Map<number, Card>;
  /** The sessions running now, for `tab` names in the text. */
  sessions: SessionRow[];
  drafts: Record<string, string>;
  setDraft: (key: string, text: string) => void;
  /**
   * What the person has open and is typing in. The agent rewrites the report as mail arrives, which rebuilds its
   * controls, so this lives with the report, not in each control: a box he was typing in stays open with his
   * cursor where it was.
   */
  kept: KeptFields;
  /** The cards the last Approve All sent: their defaults show a spinner, then a check (or a failure). */
  approved?: Set<number>;
  /** Cards the deck says were carried out after an approval: their defaults keep the check after a reload, a
   * report edit, or on another device. */
  finished?: Set<number>;
  /**
   * Approve one default's cards on their own, as Approve All would (same undo window), or undefined where nothing
   * can be approved (History's snapshot). Resolves once the deck has the batch.
   */
  approveOne?: (cards: number[]) => Promise<Approving | null>;
  /** Undoes a batch still in its undo window. */
  undoBatch?: (batch: string) => Promise<void>;
  /** Defaults whose cards the person changed (a change sent or typed): those wait for the agent, not an approval. */
  changed?: Set<string>;
  /** The document it's on, when not the Inbox: Work entries have session menus and an FYI part. */
  doc?: Document;
};

type Caret = { key: string; start: number; end: number };

type KeptFields = {
  open: Set<string>;
  setOpen: (key: string, on: boolean) => void;
  /** An `<Ask>`'s Other text, by control. */
  notes: Record<string, string>;
  setNote: (key: string, text: string) => void;
  /** The box with the cursor, and where the cursor is. */
  caret: React.RefObject<Caret | null>;
};

function useKeptFields(): KeptFields {
  const [open, setOpenKeys] = useState<Set<string>>(() => new Set());
  const [notes, setNotes] = useState<Record<string, string>>({});
  const caret = useRef<Caret | null>(null);
  return {
    open,
    setOpen: (key, on) =>
      setOpenKeys((all) => {
        if (all.has(key) === on) return all;
        const next = new Set(all);
        if (on) next.add(key);
        else next.delete(key);
        return next;
      }),
    notes,
    setNote: (key, text) => setNotes((all) => ({ ...all, [key]: text })),
    caret,
  };
}

/**
 * Props for a text box that keeps the cursor through a rebuild: it records where the cursor is as he types, and a
 * box rebuilt while he was in it takes the focus back with the cursor in the same place.
 */
function useKeptCaret(key: string) {
  const { kept } = useReport();
  const record = (el: HTMLTextAreaElement) => {
    kept.caret.current = { key, start: el.selectionStart, end: el.selectionEnd };
  };
  return {
    /** Opening the box: the cursor goes to the end of what's in it. */
    focusAtEnd: (length: number) => {
      kept.caret.current = { key, start: length, end: length };
    },
    /** For the textarea. */
    props: {
      ref: (el: HTMLTextAreaElement | null) => {
        const c = kept.caret.current;
        if (el && c?.key === key && document.activeElement !== el) {
          el.focus();
          el.setSelectionRange(c.start, c.end);
        }
      },
      onSelect: (e: React.SyntheticEvent<HTMLTextAreaElement>) => record(e.currentTarget),
      onInput: (e: React.FormEvent<HTMLTextAreaElement>) => record(e.currentTarget),
      onKeyUp: (e: React.KeyboardEvent<HTMLTextAreaElement>) => record(e.currentTarget),
      onMouseUp: (e: React.MouseEvent<HTMLTextAreaElement>) => record(e.currentTarget),
      onFocus: (e: React.FocusEvent<HTMLTextAreaElement>) => record(e.currentTarget),
      onBlur: (e: React.FocusEvent<HTMLTextAreaElement>) => {
        const el = e.currentTarget;
        // A rebuild takes the box out of the page: keep the cursor for the new one. A click elsewhere: forget it.
        window.setTimeout(() => {
          if (el.isConnected && kept.caret.current?.key === key) kept.caret.current = null;
        });
      },
    },
  };
}

const ReportCtx = createContext<Ctx | null>(null);

function useReport(): Ctx {
  const ctx = use(ReportCtx);
  if (!ctx) throw new Error("a report control outside the report");
  return ctx;
}

function text(children: ReactNode): string {
  return Children.toArray(children)
    .map((c) => (typeof c === "string" || typeof c === "number" ? String(c) : isValidElement(c) ? text((c.props as { children?: ReactNode }).children) : ""))
    .join("");
}

function useAnswer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (a: ReportAnswerAdd) => api<Report>("/api/report/answers", { method: "POST", body: a }),
    // Returned, so the send counts as finished only once the refreshed report (with the message in it) is back.
    onSuccess: () => qc.invalidateQueries({ queryKey: ["inbox"] }),
  });
}

function latest(answers: ReportAnswer[], control: string): ReportAnswer | undefined {
  return answers.filter((a) => a.control === control).at(-1);
}

/** The card IDs in a `cards` value (`c_1 c_2`, `1,2`). */
function cardIds(cards: string | undefined): number[] {
  return (cards ?? "")
    .split(/[\s,]+/)
    .map((c) => Number(c.replace(/^c_/, "")))
    .filter((n) => Number.isInteger(n) && n > 0);
}

/**
 * A default's conversation, oldest first: what the person said about it and the agent's replies. A message belongs
 * to it by the default's key, or by a card it names, so the conversation follows an email the agent moves to another
 * group.
 */
function threadOf(report: Report, key: string, cards: string | undefined): ReportAnswer[] {
  const mine = new Set(cardIds(cards));
  return report.answers.filter(
    (a) =>
      (a.kind === "choice"
        ? cardIds(a.cards).some((id) => mine.has(id))
        : a.control === key || (a.control.startsWith("cards:") && cardIds(a.control.slice(6)).some((id) => mine.has(id)))),
  );
}

/** One message in a default's thread: his on the right, the agent's on the left. */
function Bubble({ answer, pending }: { answer?: ReportAnswer; pending?: string }) {
  const { report } = useReport();
  const mine = pending !== undefined || answer?.kind !== "reply";
  return (
    <span className={cn("flex flex-col", mine ? "items-end pl-8" : "items-start pr-8")}>
      <span
        className={cn(
          "rounded-lg px-2.5 py-1.5 text-sm whitespace-pre-line text-ink",
          mine ? "rounded-br-sm bg-accent/15" : "rounded-bl-sm bg-paper-3 [&_code]:bg-paper",
        )}
      >
        {answer?.kind === "choice" && answer.cards && <span className="block text-xs text-muted">{answer.about}</span>}
        {mine ? (pending ?? answer?.value) : <Render options={CHAT_OPTIONS}>{answer?.value ?? ""}</Render>}
      </span>
      <span className="mt-0.5 px-1 text-xs text-neutral">
        {pending !== undefined
          ? `Sending to ${report.posted_by.tab}…`
          : `${mine ? "You" : report.posted_by.tab} · ${clock(answer?.answered_at ?? "")}`}
      </span>
    </span>
  );
}

/** A paragraph of the agent's reply: a span, since the thread sits inside the report's own paragraphs. */
function ChatParagraph({ children }: { children?: ReactNode }) {
  return <span className="block [&+&]:mt-1.5">{children}</span>;
}

/** Which default a message was written on: its control, or for a pick, the cards its question names. */
function originOf(a: ReportAnswer): string {
  return a.kind === "choice" && a.cards ? `cards:${a.cards}` : a.control;
}

/** A heading for messages written on another default: the emails it named ("Threshold Brands: Sales pitch"). */
function originLabel(origin: string, first: ReportAnswer, byId: Map<number, Card>): string {
  const named = origin.startsWith("cards:") ? namedCards(origin.slice(6), byId) : [];
  if (named.length > 0) {
    return named.map((c) => `${c.sender?.name || c.sender?.address || c.subject}: ${c.summary}`).join(" · ");
  }
  return first.kind === "choice" ? first.about : first.about.replace(/\s+/g, " ").trim() || origin;
}

/**
 * A thread's messages, with a heading above each run written on another default: a card that moved into this
 * default brings its conversation, and the heading says which email it was about. A thread written only here has no
 * headings.
 */
function ThreadItems({ thread, ownKey, ownLabel }: { thread: ReportAnswer[]; ownKey: string; ownLabel: string }) {
  const { cards: byId } = useReport();
  const mixed = thread.some((a) => originOf(a) !== ownKey);
  return thread.map((a, i) => {
    const origin = originOf(a);
    const heading = mixed && (i === 0 || originOf(thread[i - 1]) !== origin);
    return (
      <Fragment key={a.id}>
        {heading && (
          <span className={cn("flex items-center gap-2 text-xs font-medium text-muted", i > 0 && "mt-1")}>
            <span className="h-px min-w-4 flex-1 bg-rule" />
            <span className="max-w-[85%] truncate">{origin === ownKey ? ownLabel : originLabel(origin, a, byId)}</span>
            <span className="h-px min-w-4 flex-1 bg-rule" />
          </span>
        )}
        <Bubble answer={a} />
      </Fragment>
    );
  });
}

/** The thread's messages, scrolled to the newest. */
function ThreadMessages({
  thread,
  ownKey,
  ownLabel,
  pending,
}: {
  thread: ReportAnswer[];
  ownKey: string;
  ownLabel: string;
  pending?: string | null;
}) {
  const end = useRef<HTMLSpanElement>(null);
  const [scrolled, setScrolled] = useState(false);
  // Once the deck's copy of the pending message is in the thread, the pending copy goes, so it never shows twice.
  if (pending && thread.some((a) => a.kind !== "reply" && a.value === pending)) pending = null;
  const count = thread.length + (pending ? 1 : 0);
  useEffect(() => {
    end.current?.scrollIntoView({ block: "nearest" });
  }, [count]);
  if (count === 0) return null;
  return (
    <span className="mt-2 block border-t border-rule pt-2">
      <span
        onScroll={(e) => setScrolled(e.currentTarget.scrollTop > 0)}
        className={cn(
          "flex max-h-80 flex-col gap-2 overflow-y-auto pr-1",
          scrolled && "[mask-image:linear-gradient(to_bottom,transparent,black_2rem)]",
        )}
      >
        <ThreadItems thread={thread} ownKey={ownKey} ownLabel={ownLabel} />
        {pending && <Bubble pending={pending} />}
        <span ref={end} />
      </span>
    </span>
  );
}

/**
 * `<Default cards="c_1 c_2">forward, then archive</Default>`: the text in a compact box of its own, which marks it as
 * the default without a label. The chat button at its right edge opens the default's conversation inside the box: his
 * messages on the right, the agent's replies on the left, and a box to say more. Closed, the box shows the newest
 * message. The item's headline stays the only bold text in it.
 */
/**
 * Left of a default's text, for the cards the last Approve All sent: a spinner while they're carried out, a green
 * check once all are done, red if one failed. Above the busy darkening, so it stays bright.
 */
/**
 * Approve this default's cards on their own, at the right end of its panel; Approve All still takes everything
 * that's left. Gone once nothing in it waits for approval; greyed out while a change to it is typed or with the
 * agent, since then the agent rewrites it first.
 */
function ApproveOne({
  about,
  ids,
  blocked,
  approveOne,
  undoBatch,
}: {
  about: string;
  ids: number[];
  blocked: boolean;
  approveOne: (cards: number[]) => Promise<Approving | null>;
  undoBatch?: (batch: string) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  // In the undo window the check is an undo: pressing it cancels this approval, like Undo in the bar.
  const [pending, setPending] = useState<Approving | null>(null);
  useEffect(() => {
    if (!pending) return;
    const t = window.setTimeout(() => setPending(null), Math.max(0, new Date(pending.release_at).getTime() - Date.now()));
    return () => window.clearTimeout(t);
  }, [pending]);
  if (pending && undoBatch)
    return (
      <button
        type="button"
        onClick={() => {
          const batch = pending.batch;
          setPending(null);
          undoBatch(batch);
        }}
        aria-label={`Undo approving: ${about}`}
        title="Undo"
        className={cn(smallButton, "w-7 px-0")}
      >
        <Undo2 aria-hidden />
      </button>
    );
  if (ids.length === 0) return null;
  return (
    <button
      type="button"
      disabled={blocked || busy}
      onClick={() => {
        setBusy(true);
        approveOne(ids)
          .then((a) => setPending(a))
          .finally(() => setBusy(false));
      }}
      aria-label={`Approve: ${about}`}
      title={blocked ? "Waiting for the agent to take your change" : "Approve"}
      className={cn(smallButton, "w-7 px-0")}
    >
      <Check aria-hidden />
    </button>
  );
}

function ApprovedMark({ cards }: { cards: string | undefined }) {
  const { cards: byId, approved, finished } = useReport();
  const ids = cardIds(cards);
  const base = "relative z-10 mt-1 mr-1.5 inline-flex size-4 shrink-0 items-center justify-center rounded-full";
  // A failed action stays marked, whether or not it came from the last Approve All, until it's retried or handled.
  if (ids.some((id) => byId.get(id)?.state === "failed"))
    return (
      <span
        className={cn(base, "bg-[oklch(48%_0.12_28)] text-[10px] leading-none font-bold text-white")}
        title="An action failed: open the card to retry it"
        role="img"
        aria-label="Failed"
      >
        !
      </span>
    );
  // Carried out after an approval, as the deck records it: the check stays, whatever this browser remembers.
  if (ids.length > 0 && ids.every((id) => finished?.has(id)))
    return (
      <span className={cn(base, "bg-[oklch(52%_0.1_150)] text-white")} title="Done" role="img" aria-label="Done">
        <Check className="size-2.5" strokeWidth={4} aria-hidden />
      </span>
    );
  const mine = ids.filter((id) => approved?.has(id));
  if (mine.length === 0) return null;
  // During the undo window nothing has started (the cards are still open): no mark until it's really being done.
  if (mine.every((id) => byId.get(id)?.state === "open")) return null;
  // A finished card leaves the Inbox's list, so one that's gone is done.
  const done = mine.every((id) => {
    const state = byId.get(id)?.state ?? "done";
    return state === "done" || state === "automatic" || state === "handled_elsewhere";
  });
  if (done)
    return (
      <span className={cn(base, "check-in bg-[oklch(52%_0.1_150)] text-white")} title="Done" role="img" aria-label="Done">
        <Check className="size-2.5" strokeWidth={4} aria-hidden />
      </span>
    );
  // A spinner only while something is still being carried out. Stopped partway (Stop after the undo window), the
  // cards that didn't run are open again: no mark, and the next Approve All takes them.
  const inProgress = mine.some((id) => {
    const state = byId.get(id)?.state;
    return state === "approved" || state === "executing" || state === "handed_off" || state === "revising";
  });
  if (!inProgress) return null;
  return (
    <span className={cn(base, "text-accent")} title="Being carried out" role="img" aria-label="Being carried out">
      <Loader2 className="size-3.5 animate-spin" aria-hidden />
    </span>
  );
}

/** The chat box's shortcuts (Settings → Chat shortcuts): each sends its text as an ordinary message. */
function ChatShortcuts({ about, disabled, onPick }: { about: string; disabled: boolean; onPick: (text: string) => void }) {
  const settings = useQuery({ queryKey: ["settings"], queryFn: () => api<Settings>("/api/settings") });
  const shortcuts = settings.data?.chat_shortcuts ?? [];
  if (shortcuts.length === 0) return null;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          size="sm"
          variant="secondary"
          disabled={disabled}
          aria-label={`Shortcut replies about: ${about}`}
          title="Shortcut replies"
          className="h-10 w-10 shrink-0 px-0"
        >
          <MessageSquareText aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-w-[min(22rem,calc(100vw-2rem))]">
        <DropdownMenuLabel>Send a shortcut</DropdownMenuLabel>
        {shortcuts.map((s) => (
          <DropdownMenuItem key={s} onSelect={() => onPick(s)}>
            {s}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function Default({
  id,
  cards,
  session,
  children,
}: {
  id?: string;
  cards?: string;
  session?: string;
  children?: ReactNode;
}) {
  const { report, cards: byId, drafts, setDraft, kept, approveOne, undoBatch, changed } = useReport();
  const key = defaultKey(id, cards);
  const open = kept.open.has(key);
  const setOpen = (on: boolean) => kept.setOpen(key, on);
  const caret = useKeptCaret(key);
  const send = useAnswer();
  const thread = threadOf(report, key, cards);
  const lastMine = thread.filter((a) => a.kind !== "reply").at(-1);
  const newest = thread.at(-1);
  const draft = drafts[key] ?? "";
  const about = text(children).trim();
  const tab = report.posted_by.tab;
  // A sent message waits out the undo window as a draft: Approve All keeps its cards out meanwhile, and sends it first.
  const undoWindow = useUndoWindow();
  const [sending, setSending] = useState<string | null>(null);
  const latestDraft = useRef(draft);
  latestDraft.current = draft;
  // A shortcut sends its text as if he'd typed it: into the draft, then the same undo window.
  const submit = (typed?: string) => {
    const value = (typed ?? draft).trim();
    if (!value) return;
    if (typed !== undefined) setDraft(key, typed);
    setSending(value);
    undoWindow.start(() => {
      // Approve All sent it already (and cleared the draft), or it was edited since.
      if (latestDraft.current.trim() !== value) {
        setSending(null);
        return;
      }
      // The pending message stays in the conversation, and the box stays empty, until the deck's copy is on screen:
      // clearing it earlier puts the text back in the box for a moment.
      send.mutate(
        { report_id: report.id, report_version: report.version, control: key, kind: "change", about, value },
        {
          onSuccess: () => {
            if (latestDraft.current.trim() === value) setDraft(key, "");
            setSending(null);
          },
          onError: () => setSending(null),
        },
      );
    });
  };
  const undo = () => {
    undoWindow.cancel();
    setSending(null);
    caret.focusAtEnd(draft.length);
  };
  // In the undo window the chat button is Undo. While the agent works on his message it stays the chat button, so the
  // conversation can still be closed: the spinner shows the work, and the bar at the bottom has the one Stop.
  const chatButton = undoWindow.pending ? (
    <UndoButton onClick={undo} label={`Undo the message about: ${about}`} />
  ) : (
    <button
      type="button"
      onClick={() => {
        if (!open) caret.focusAtEnd(draft.length);
        setOpen(!open);
      }}
      aria-expanded={open}
      aria-label={`${thread.length > 0 ? "Conversation" : "Change"}: ${about}`}
      title={thread.length > 0 ? "Conversation" : "Change"}
      className={cn(smallButton, thread.length > 0 ? "px-1.5" : "w-7 px-0")}
    >
      <MessageSquare aria-hidden />
      {thread.length > 0 && <span className="tabular-nums">{thread.length}</span>}
    </button>
  );
  return (
    <span
      className="report-default relative rounded-md border border-rule bg-paper-2 py-1 pr-1 pl-3"
      data-card-ids={cardIds(cards).join(" ")}
    >
      <span className="-my-0.5 flex items-start gap-0.5">
        <ApprovedMark cards={cards} />
        <span className="min-w-0 flex-1 py-0.5 text-ink">{children}</span>
        {chatButton}
        {session ? <SessionMenu session={session} about={about} /> : <EmailMenu about={about} cards={namedCards(cards, byId)} />}
        {approveOne && (
          <ApproveOne
            about={about}
            ids={cardIds(cards).filter((cid) => {
              const state = byId.get(cid)?.state;
              return state === "open" || state === "handed_off";
            })}
            blocked={(changed?.has(key) ?? false) || draft.trim() !== "" || sending !== null}
            approveOne={approveOne}
            undoBatch={undoBatch}
          />
        )}
      </span>
      {open ? (
        <>
          <ThreadMessages thread={thread} ownKey={key} ownLabel={about} pending={sending} />
          {lastMine && !sending && newest?.kind !== "reply" && <ChatStatus answer={lastMine} />}
          <span className="mt-2 mb-1 flex flex-col gap-2 sm:flex-row sm:items-end">
            <Textarea
              rows={1}
              {...caret.props}
              // Sent text leaves the box at once and shows in the conversation. It stays a draft behind the scenes
              // until the undo window ends, so Approve All keeps its cards out, and Undo puts it back here.
              value={sending !== null ? "" : draft}
              maxLength={4000}
              placeholder={thread.length > 0 ? `Reply to ${tab}` : "What to change"}
              aria-label={`Message about: ${about}`}
              disabled={undoWindow.pending}
              onChange={(e) => setDraft(key, e.target.value)}
              onKeyDown={enterSends(() => draft.trim() && !undoWindow.pending && submit())}
              className="bg-paper py-[9px] text-sm leading-5"
            />
            <span className="flex shrink-0 gap-2">
              <ChatShortcuts about={about} disabled={undoWindow.pending} onPick={(t) => submit(t)} />
              <Button
                size="sm"
                loading={send.isPending}
                disabled={!draft.trim() || undoWindow.pending}
                onClick={() => submit()}
                className="h-10 flex-1 sm:flex-none"
              >
                Send
              </Button>
            </span>
          </span>
        </>
      ) : (
        (sending !== null || newest) && (
          <span className="mt-0.5 block truncate text-sm text-muted">
            {sending !== null ? (
              <>You: {sending} <span className="text-neutral">· Sending to {tab}…</span></>
            ) : (
              <>
                {newest?.kind === "reply" ? tab : "You"}: {newest?.value.replace(/`/g, "")}
              </>
            )}
          </span>
        )
      )}
      {send.error && <span className="block text-sm text-danger">{send.error.message}</span>}
    </span>
  );
}

/** Under his newest message while the agent hasn't replied: whether it reached the agent, and whether it's on it. */
function ChatStatus({ answer }: { answer: ReportAnswer }) {
  const { report } = useReport();
  const working = useWorkingOn(answer);
  const tab = report.posted_by.tab;
  const words = !answer.picked_up
    ? `Not sent to ${tab} yet: it gets this when it's next online.`
    : working
      ? `${tab} is working on it…`
      : `Sent to ${tab}.`;
  return (
    <span className="mt-1 block pr-1 text-right text-xs text-neutral">
      {words}
      {answer.picked_up && working && <InlineSpinner />}
    </span>
  );
}

/** The cards a `<Default cards="c_1 c_2">` names, among those the report has. */
function namedCards(cards: string | undefined, byId: Map<number, Card>): Card[] {
  return cardIds(cards)
    .map((id) => byId.get(id))
    .filter((c): c is Card => c !== undefined);
}

/** A default in a snapshot: the text, the emails, and its conversation, with nothing to press but the email menu. */
function SnapshotDefault({ id, cards, children }: { id?: string; cards?: string; children?: ReactNode }) {
  const { report, cards: byId } = useReport();
  const thread = threadOf(report, defaultKey(id, cards), cards);
  return (
    <span className="report-default relative rounded-md border border-rule bg-paper-2 py-1 pr-1 pl-3">
      <span className="-my-0.5 flex items-start gap-0.5">
        <span className="min-w-0 flex-1 py-0.5 text-ink">{children}</span>
        <EmailMenu about={text(children).trim()} cards={namedCards(cards, byId)} />
      </span>
      {thread.length > 0 && (
        <span className="mt-1.5 mb-1 flex flex-col gap-2 pr-1">
          <ThreadItems thread={thread} ownKey={defaultKey(id, cards)} ownLabel={text(children).trim()} />
        </span>
      )}
    </span>
  );
}

/** A question in a snapshot, with his answer. */
function SnapshotAsk({ id, children }: { id?: string; children?: ReactNode }) {
  const { report } = useReport();
  const key = id ?? text(children).trim();
  const answered = latest(report.answers.filter((a) => a.kind === "choice"), key);
  const thread = report.answers.filter((a) => a.control === key && (a.kind === "choice" || a.kind === "reply"));
  if (thread.length > 1 || thread.some((a) => a.kind === "reply")) {
    return (
      <span className="report-default report-question rounded-md border border-rule bg-paper-2 py-1 pr-1 pl-3">
        <span className="text-ink">{children}</span>
        <span className="mt-1.5 mb-1 flex flex-col gap-2 pr-1">
          <ThreadItems thread={thread} ownKey={key} ownLabel={key} />
        </span>
      </span>
    );
  }
  return (
    <span className="report-ask">
      <span>{children}</span>{" "}
      {answered ? (
        <span className="text-accent">{answered.value}</span>
      ) : (
        <span className="text-sm text-neutral">Not answered</span>
      )}
    </span>
  );
}

/**
 * `<Ask id="signin" options="Yes|No" other>Was this you?</Ask>`: the question, a button per option, Other. With
 * `cards="c_12"`, the answer also shows in the thread of the default that names those cards, as his message.
 */
function Ask({
  id,
  options,
  other,
  cards,
  session,
  children,
}: {
  id?: string;
  options?: string;
  other?: unknown;
  cards?: string;
  session?: string;
  children?: ReactNode;
}) {
  const choices = (options ?? "").split("|").map((o) => o.trim()).filter(Boolean);
  if (choices.length === 0) return <QuestionPanel id={id} session={session}>{children}</QuestionPanel>;
  const ask = <ChoiceAsk id={id} choices={choices} other={other} cards={cards}>{children}</ChoiceAsk>;
  if (!session) return ask;
  // On Work, a choice about a session (a Done for something only he can do) sits in a panel with the session's menu.
  return (
    <span className="report-default relative rounded-md border border-rule bg-paper-2 py-1 pr-1 pl-3">
      <span className="flex items-start gap-1">
        <span className="min-w-0 flex-1 py-0.5 [&_.report-ask]:block">{ask}</span>
        <SessionMenu session={session} about={text(children).trim()} />
      </span>
    </span>
  );
}

/** An `<Ask>` with options: a button per option, Other, and the agent's replies to his answer under it. */
function ChoiceAsk({
  id,
  choices,
  other,
  cards,
  children,
}: {
  id?: string;
  choices: string[];
  other?: unknown;
  cards?: string;
  children?: ReactNode;
}) {
  const { report, kept } = useReport();
  const control = id ?? text(children).trim();
  const about = text(children).trim();
  const answered = latest(report.answers.filter((a) => a.kind === "choice"), control);
  const keptKey = `ask:${control}`;
  const otherOpen = kept.open.has(keptKey);
  const setOtherOpen = (on: boolean) => kept.setOpen(keptKey, on);
  const otherText = kept.notes[keptKey] ?? "";
  const setOtherText = (t: string) => kept.setNote(keptKey, t);
  const caret = useKeptCaret(keptKey);
  const send = useAnswer();
  const undoWindow = useUndoWindow();
  const [sending, setSending] = useState<string | null>(null);
  const answer = (value: string) => {
    setSending(value);
    setOtherOpen(false);
    undoWindow.start(() => {
      setSending(null);
      send.mutate(
        { report_id: report.id, report_version: report.version, control, kind: "choice", about, value, cards },
        { onSuccess: () => setOtherText("") },
      );
    });
  };
  const undo = () => {
    undoWindow.cancel();
    // An Other answer goes back into its box.
    if (sending !== null && !choices.includes(sending)) {
      caret.focusAtEnd(otherText.length);
      setOtherOpen(true);
    }
    setSending(null);
  };
  const shown = sending ?? answered?.value;
  const otherOn = other !== undefined && other !== false && other !== "false";
  const chat = useAskChat(control, about);
  return (
    <span className="report-ask relative">
      <span>{children}</span>{" "}
      <span className="mt-1.5 ml-[5px] inline-flex flex-wrap items-center gap-1.5 align-baseline">
        {choices.map((c) => (
          <Button
            key={c}
            size="sm"
            variant={shown === c ? "primary" : "secondary"}
            aria-pressed={shown === c}
            disabled={send.isPending}
            onClick={() => answer(c)}
            className="h-7"
          >
            {c}
          </Button>
        ))}
        {otherOn && (
          <Button
            size="sm"
            variant={answered && !choices.includes(answered.value) ? "primary" : "secondary"}
            aria-expanded={otherOpen}
            onClick={() => {
              if (!otherOpen) caret.focusAtEnd(otherText.length);
              setOtherOpen(!otherOpen);
            }}
            className="h-7"
          >
            Other
          </Button>
        )}
        {undoWindow.pending && <UndoButton onClick={undo} label={`Undo the answer to: ${about}`} />}
        {chat.button}
      </span>
      {shown !== undefined && !choices.includes(shown) && <span className="block text-accent">You said: {shown}</span>}
      {sending !== null ? (
        <span className="block text-sm text-neutral">Sending to {report.posted_by.tab}…</span>
      ) : (
        answered && <SentStatus answer={answered} />
      )}
      {otherOpen && (
        <span className="mt-1.5 flex flex-col gap-2 sm:flex-row sm:items-end">
          <Textarea
            rows={1}
            {...caret.props}
            value={otherText}
            maxLength={4000}
            placeholder="Your answer"
            aria-label={`Answer to: ${about}`}
            onChange={(e) => setOtherText(e.target.value)}
            onKeyDown={enterSends(() => otherText.trim() && answer(otherText.trim()))}
            className="py-[9px] text-sm leading-5"
          />
          <Button
            size="sm"
            loading={send.isPending}
            disabled={!otherText.trim()}
            onClick={() => answer(otherText.trim())}
            className="h-10 shrink-0"
          >
            Send
          </Button>
        </span>
      )}
      {chat.body}
      {send.error && <span className="block text-sm text-danger">{send.error.message}</span>}
    </span>
  );
}

/**
 * The chat on an `<Ask>` with options, for when none of them fits and he'd rather talk it over: a button beside the
 * options opens the question's thread (his picks, his messages, the agent's replies) with a box to write in. What he
 * writes goes to the agent as a message about the question, not as his answer, so the question stays open. Closed,
 * the thread's messages show under the question as before.
 */
function useAskChat(control: string, about: string): { button: ReactNode; body: ReactNode } {
  const { report, kept } = useReport();
  const key = `chat:${control}`;
  const open = kept.open.has(key);
  const draft = kept.notes[key] ?? "";
  const setDraft = (t: string) => kept.setNote(key, t);
  const caret = useKeptCaret(key);
  const send = useAnswer();
  const undoWindow = useUndoWindow();
  const [sending, setSending] = useState<string | null>(null);
  const tab = report.posted_by.tab;
  const thread = report.answers.filter((a) => a.control === control);
  const talk = thread.filter((a) => a.kind !== "choice");
  const lastMine = talk.filter((a) => a.kind !== "reply").at(-1);
  const newest = talk.at(-1);
  const submit = () => {
    const value = draft.trim();
    if (!value) return;
    setSending(value);
    setDraft("");
    undoWindow.start(() =>
      send.mutate(
        { report_id: report.id, report_version: report.version, control, kind: "change", about, value },
        { onSettled: () => setSending(null), onError: () => setDraft(value) },
      ),
    );
  };
  const undo = () => {
    undoWindow.cancel();
    if (sending !== null) {
      setDraft(sending);
      caret.focusAtEnd(sending.length);
    }
    setSending(null);
  };
  const button = (
    <button
      type="button"
      onClick={() => {
        if (!open) caret.focusAtEnd(draft.length);
        kept.setOpen(key, !open);
      }}
      aria-expanded={open}
      aria-label={`${talk.length > 0 ? "Conversation" : "Chat"}: ${about}`}
      title={talk.length > 0 ? "Conversation" : "Chat about it instead"}
      className={cn(smallButton, talk.length > 0 ? "px-1.5" : "w-7 px-0")}
    >
      <MessageSquare aria-hidden />
      {talk.length > 0 && <span className="tabular-nums">{talk.length}</span>}
    </button>
  );
  const body = open ? (
    <span className="block">
      <ThreadMessages thread={thread} ownKey={control} ownLabel={about} pending={sending} />
      {lastMine && !sending && newest?.kind !== "reply" && <ChatStatus answer={lastMine} />}
      <span className="mt-2 mb-1 flex flex-col gap-2 sm:flex-row sm:items-end">
        <Textarea
          rows={1}
          {...caret.props}
          value={draft}
          maxLength={4000}
          placeholder={talk.length > 0 ? `Reply to ${tab}` : `Talk it over with ${tab}`}
          aria-label={`Message about: ${about}`}
          disabled={undoWindow.pending}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={enterSends(() => draft.trim() && !undoWindow.pending && submit())}
          className="bg-paper py-[9px] text-sm leading-5"
        />
        {undoWindow.pending ? (
          <UndoButton onClick={undo} label={`Undo the message about: ${about}`} />
        ) : (
          <Button size="sm" loading={send.isPending} disabled={!draft.trim()} onClick={submit} className="h-10 shrink-0">
            Send
          </Button>
        )}
      </span>
      {send.error && <span className="block text-sm text-danger">{send.error.message}</span>}
    </span>
  ) : (
    (talk.length > 0 || sending !== null) && (
      <span className="mt-1.5 flex flex-col gap-2">
        {talk.map((a) => (
          <Bubble key={a.id} answer={a} />
        ))}
        {sending !== null && <Bubble pending={sending} />}
      </span>
    )
  );
  return { button, body };
}

/** Whether an `<Ask>` still waits for him: it has no answer from him yet. */
export function unansweredAsks(report: Report): string[] {
  const out: string[] = [];
  for (const m of report.markdown.matchAll(/<Ask\b([^>]*)>([\s\S]*?)<\/Ask>/g)) {
    const key = /\bid="([^"]*)"/.exec(m[1] ?? "")?.[1] ?? (m[2] ?? "").trim();
    if (!report.answers.some((a) => a.control === key && a.kind === "choice")) out.push(key);
  }
  return out;
}

/**
 * `<Ask id="budget">How much can I spend on the venue?</Ask>` with no options: a question panel. The question with a
 * thread of its own, always open, where he answers in words and the agent replies. No approve check: Approve All
 * passes it by, and the agent takes it out or rewrites it once it's settled.
 */
function QuestionPanel({ id, session, children }: { id?: string; session?: string; children?: ReactNode }) {
  const { report, kept } = useReport();
  const about = text(children).trim();
  const key = id ?? about;
  const keptKey = `ask:${key}`;
  const draft = kept.notes[keptKey] ?? "";
  const setDraft = (t: string) => kept.setNote(keptKey, t);
  const caret = useKeptCaret(keptKey);
  const send = useAnswer();
  const undoWindow = useUndoWindow();
  const [sending, setSending] = useState<string | null>(null);
  const thread = report.answers.filter((a) => a.control === key && (a.kind === "choice" || a.kind === "reply"));
  const lastMine = thread.filter((a) => a.kind !== "reply").at(-1);
  const newest = thread.at(-1);
  const tab = report.posted_by.tab;
  const submit = () => {
    const value = draft.trim();
    if (!value) return;
    setSending(value);
    setDraft("");
    undoWindow.start(() =>
      send.mutate(
        { report_id: report.id, report_version: report.version, control: key, kind: "choice", about, value },
        { onSettled: () => setSending(null), onError: () => setDraft(value) },
      ),
    );
  };
  const undo = () => {
    undoWindow.cancel();
    if (sending !== null) {
      setDraft(sending);
      caret.focusAtEnd(sending.length);
    }
    setSending(null);
  };
  return (
    <span className="report-default report-question relative rounded-md border border-accent/40 bg-paper-2 py-1.5 pr-1.5 pl-3">
      <span className="flex items-start gap-2">
        <CircleHelp aria-hidden className="mt-[3px] size-4 shrink-0 text-accent" />
        <span className="min-w-0 flex-1 text-ink">{children}</span>
        {lastMine && <span className="mt-0.5 shrink-0 text-xs text-neutral">Answered</span>}
        {session && <span className="-my-1"><SessionMenu session={session} about={about} /></span>}
      </span>
      <ThreadMessages thread={thread} ownKey={key} ownLabel={about} pending={sending} />
      {lastMine && !sending && newest?.kind !== "reply" && <ChatStatus answer={lastMine} />}
      <span className="mt-2 mb-0.5 flex flex-col gap-2 sm:flex-row sm:items-end">
        <Textarea
          rows={1}
          {...caret.props}
          value={draft}
          maxLength={4000}
          placeholder={thread.length > 0 ? `Reply to ${tab}` : "Your answer"}
          aria-label={`Answer to: ${about}`}
          disabled={undoWindow.pending}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={enterSends(() => draft.trim() && !undoWindow.pending && submit())}
          className="bg-paper py-[9px] text-sm leading-5"
        />
        {undoWindow.pending ? (
          <UndoButton onClick={undo} label={`Undo the answer to: ${about}`} />
        ) : (
          <Button size="sm" loading={send.isPending} disabled={!draft.trim()} onClick={submit} className="h-10 shrink-0">
            Send
          </Button>
        )}
      </span>
      {send.error && <span className="block text-sm text-danger">{send.error.message}</span>}
    </span>
  );
}

/**
 * The session a `tab` name in the text means: a session with that tab, on the machine the report's cards forward to
 * it on when they say, and otherwise the one active most recently.
 */
function sessionNamed(name: string, sessions: SessionRow[], cards: Map<number, Card>): SessionRow | undefined {
  const nodes = new Set<string>();
  for (const c of cards.values())
    for (const a of c.actions) {
      if (a.type === "forward" && a.to.tab === name) nodes.add(a.to.node);
      if ((a.type === "resume" || a.type === "new_session") && a.tab === name) nodes.add(a.node);
    }
  const recent = (s: SessionRow) => s.last_activity ?? s.status_changed_at;
  return sessions
    .filter((s) => s.tab === name)
    .sort((a, b) => Number(nodes.has(b.node)) - Number(nodes.has(a.node)) || recent(b).localeCompare(recent(a)))[0];
}

/**
 * Under a Work document's FYI list: one panel, as the Inbox's archive part has, whose check clears the list. Approve
 * All clears it too. The deck puts it there (fyiPanel); the agent doesn't write it.
 */
function FyiClear() {
  const { doc } = useReport();
  const qc = useQueryClient();
  const clear = useMutation({
    mutationFn: () => api(`/api/report/fyi/clear?doc=${encodeURIComponent(doc?.slug ?? "")}`, { method: "POST" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["inbox"] }),
    onError: (e) => notify(`Couldn't clear the list: ${e.message}`, { error: true, ms: 30_000 }),
  });
  return (
    <span className="report-default report-fyi relative rounded-md border border-rule bg-paper-2 py-1 pr-1 pl-3">
      <span className="-my-0.5 flex items-center gap-0.5">
        <span className="min-w-0 flex-1 py-0.5 text-ink">Archive all</span>
        <button
          type="button"
          onClick={() => clear.mutate()}
          disabled={clear.isPending}
          aria-label="Archive all: clear the FYI list"
          title="Clear the FYI list"
          className={smallButton + " w-7 px-0"}
        >
          {clear.isPending ? <Loader2 className="animate-spin" aria-hidden /> : <Check aria-hidden />}
        </button>
      </span>
    </span>
  );
}

/**
 * Inline code that names a running session's tab (`dep updates`) links to that session, so the text can say "forward
 * to `dep updates`" and the look shows it's a session. It opens in a new tab, keeping the report where it was. Any
 * other code stays code.
 */
function Code({ className, children }: { className?: string; children?: ReactNode }) {
  const { sessions, cards } = useReport();
  const target = typeof children === "string" && !className ? sessionNamed(children, sessions, cards) : undefined;
  if (!target) return <code className={className}>{children}</code>;
  return (
    <a
      href={sessionPath(target.node, target.session_id)}
      target="_blank"
      rel="noreferrer noopener"
      className="session-ref"
      title={`Session ${children} on ${target.node}`}
    >
      {children}
    </a>
  );
}

function Link(props: React.ComponentProps<"a">) {
  return <a {...props} target="_blank" rel="noreferrer noopener" />;
}

const OPTIONS = {
  forceBlock: true,
  overrides: { Default, Ask, FyiClear, a: Link, code: Code },
};

const SNAPSHOT_OPTIONS = {
  forceBlock: true,
  overrides: { Default: SnapshotDefault, Ask: SnapshotAsk, FyiClear: () => null, a: Link, code: Code },
};

/** An agent's reply in a thread: inline Markdown, with no headings or blocks of the report's size. */
const CHAT_OPTIONS = {
  wrapper: "span",
  forceWrapper: true,
  overrides: { a: Link, code: Code, p: ChatParagraph },
} as const;

/**
 * Only the report's own two tags are HTML: every other `<` is escaped first, so a tag an email slipped into the
 * agent's text shows as text and never reaches the page as markup.
 */
/**
 * Leaves out the part above the report's first `---` when nothing is in it but headings (the agent's "# Inbox" over an
 * empty Needs-you part), so a report with only an archive part starts at "Archive" rather than "Inbox", a rule, then
 * "Archive". Any item, question or line of text there keeps it.
 */
export function withoutEmptyTop(markdown: string): string {
  const rule = /^(?:-{3,}|\*{3,}|_{3,})\s*$/m.exec(markdown);
  if (!rule) return markdown;
  const top = markdown.slice(0, rule.index);
  const rest = top
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
  if (rest.length > 0) return markdown;
  // The rule stays, hidden as the report's first element (app.css): the archive part's spacing keys off it.
  return markdown.slice(rule.index);
}

export function onlyReportTags(markdown: string): string {
  return markdown.replace(/<(?!\/?(?:Default|Ask|FyiClear)\b)/g, "&lt;");
}

/** A Work document's FYI part ends with the deck's Archive all panel, while it has items. */
export function withFyiPanel(markdown: string): string {
  const lines = markdown.split("\n");
  const start = lines.findIndex((l) => /^#{1,2}\s+FYI\b/i.test(l.trim()));
  if (start < 0) return markdown;
  const level = (lines[start].trim().match(/^#+/)?.[0] ?? "#").length;
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    const t = lines[i].trim();
    const h = t.match(/^(#+)\s/);
    if ((h && h[1].length <= level) || /^-{3,}$/.test(t)) {
      end = i;
      break;
    }
  }
  const items = lines.slice(start + 1, end).some((l) => /^(?:[-*]|\d{1,3}\.)\s/.test(l));
  if (!items) return markdown;
  return [...lines.slice(0, end), "", "<FyiClear></FyiClear>", "", ...lines.slice(end)].join("\n");
}

/**
 * Splits out the entries about parked sessions: each top-level list item (with its indented lines) whose panel names
 * a parked session (`session="tab@node"`). The rest stays in place.
 */
export function splitParked(markdown: string, parked: Set<string>): { main: string; parked: Map<string, string[]> } {
  const out = new Map<string, string[]>();
  if (parked.size === 0) return { main: markdown, parked: out };
  const lines = markdown.split("\n");
  const kept: string[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (/^(?:[-*]|\d{1,3}\.)\s/.test(line)) {
      // The item and its continuation: indented lines, and blank lines followed by more indented ones.
      let j = i + 1;
      while (j < lines.length && (/^\s+\S/.test(lines[j]) || (lines[j].trim() === "" && /^\s+\S/.test(lines[j + 1] ?? "")))) j++;
      const item = lines.slice(i, j).join("\n");
      const who = [...item.matchAll(/\bsession="([^"]+)"/g)].map((m) => m[1]).find((a) => parked.has(a));
      if (who) {
        out.set(who, [...(out.get(who) ?? []), item.replace(/^\d{1,3}\.\s/, "- ")]);
        i = j;
        continue;
      }
      kept.push(...lines.slice(i, j));
      i = j;
      continue;
    }
    kept.push(line);
    i++;
  }
  return { main: kept.join("\n"), parked: out };
}

const parkedDay = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short", hour: "numeric" });

/** The collapsed Parked part at the bottom of a Work document: each parked session, its entries, and Unpark. */
function ParkedPart({ marks, entries }: { marks: SessionMark[]; entries: Map<string, string[]> }) {
  const [open, setOpen] = useState(false);
  const { remove } = useMarkSession();
  if (marks.length === 0) return null;
  return (
    <section className="mt-10 border-t border-rule pt-4" aria-label="Parked">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="font-display text-xl text-muted hover:text-ink"
      >
        Parked <span className="text-base text-neutral tabular-nums">({marks.length})</span>
      </button>
      {open && (
        <ul className="mt-3 flex flex-col gap-3">
          {marks.map((m) => {
            const address = `${m.tab}@${m.node}`;
            return (
              <li key={address} className="rounded-md border border-rule bg-paper-2 px-3 py-2">
                <div className="flex items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <p className="font-mono text-sm text-ink">{address}</p>
                    <p className="text-xs text-neutral">
                      {m.until ? `Snoozed until ${parkedDay.format(new Date(m.until))}` : "Parked"}
                      {m.note ? ` · ${m.note}` : ""}
                    </p>
                  </div>
                  <Button size="sm" variant="secondary" className="h-7" onClick={() => remove.mutate({ node: m.node, tab: m.tab, mark: "parked" })}>
                    Unpark
                  </Button>
                </div>
                {(entries.get(address) ?? []).length > 0 && (
                  <div className="md report mt-2 text-sm opacity-80">
                    <Render options={SNAPSHOT_OPTIONS}>{onlyReportTags((entries.get(address) ?? []).join("\n"))}</Render>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/**
 * The report's `# Triage Notes` section (what the agent's access rules stopped it doing), without its heading, or null
 * when there is none. It runs to the next top-level heading or rule. Shown above "All done." too, so a note never
 * vanishes when everything else is finished: Nathan wants them kept until he's dealt with them.
 */
export function triageNotes(markdown: string): string | null {
  const start = /^#\s+Triage Notes\s*$/im.exec(markdown);
  if (!start) return null;
  const after = markdown.slice(start.index + start[0].length);
  const end = /^(?:#\s|-{3,}\s*$|\*{3,}\s*$|_{3,}\s*$)/m.exec(after);
  const body = (end ? after.slice(0, end.index) : after).trim();
  return body === "" ? null : body;
}

/** The Triage Notes on their own, read-only: above "All done." when the rest of the report has rolled away. */
export function TriageNotes({ markdown }: { markdown: string }) {
  return (
    <section className="report md mb-8" aria-label="Triage Notes">
      <h1>Triage Notes</h1>
      <Render options={{ ...SNAPSHOT_OPTIONS, wrapper: null }}>{onlyReportTags(markdown)}</Render>
    </section>
  );
}

/** The triage agent's report, as Markdown with its controls. */
export function ReportBody({
  report,
  cards,
  drafts,
  setDraft,
  approved,
  rolled,
  approveOne,
  undoBatch,
  changed,
  finished,
  doc,
}: {
  report: Report;
  cards: Map<number, Card>;
  drafts: Record<string, string>;
  setDraft: (key: string, text: string) => void;
  approved?: Set<number>;
  /** Everything on it is done: all but the heading rolls up and away ("rolling"), then is removed ("gone"). */
  rolled?: "no" | "rolling" | "gone";
  approveOne?: (cards: number[]) => Promise<Approving | null>;
  undoBatch?: (batch: string) => Promise<void>;
  changed?: Set<string>;
  finished?: Set<number>;
  doc?: Document;
}) {
  const running = useRunningSessions();
  const kept = useKeptFields();
  const work = doc?.kind === "work";
  const marks = useSessionMarks().data ?? [];
  const parkedMarks = work ? marks.filter((m) => m.mark === "parked") : [];
  const { main, parked } = splitParked(report.markdown, new Set(parkedMarks.map((m) => `${m.tab}@${m.node}`)));
  const markdown = work ? withFyiPanel(main) : report.markdown;
  return (
    <ReportCtx
      value={{ report, cards, sessions: running.data ?? [], drafts, setDraft, kept, approved, approveOne, undoBatch, changed, finished, doc }}
    >
      <Render
        className={cn("md report", (rolled === "rolling" || rolled === "gone") && "rolled", rolled === "gone" && "gone")}
        options={OPTIONS}
      >
        {onlyReportTags(withoutEmptyTop(markdown))}
      </Render>
      {work && <ParkedPart marks={parkedMarks} entries={parked} />}
    </ReportCtx>
  );
}

const noDraft = () => undefined;

/** The report as it was at an Approve All (History): his changes and answers in place, nothing to press but the
 * email menus. */
export function ReportSnapshotBody({ report, cards }: { report: Report; cards: Map<number, Card> }) {
  const running = useRunningSessions();
  const kept = useKeptFields();
  return (
    <ReportCtx value={{ report, cards, sessions: running.data ?? [], drafts: {}, setDraft: noDraft, kept }}>
      <Render className="md report" options={SNAPSHOT_OPTIONS}>
        {onlyReportTags(report.markdown)}
      </Render>
    </ReportCtx>
  );
}

/** The chat box under the report: plain words straight to the agent that writes it. Nothing is kept. Approve All
 * sits at its right end (children), the agent's status at its left, and one Stop for the agent and the run. */
export function ReportChatBox({
  doc,
  by,
  running,
  children,
  corner,
}: {
  /** Another document's slug: its chat goes to its own agent. */
  doc?: string;
  by: SessionRef;
  running: boolean;
  children?: ReactNode;
  /** In the bar's bottom-right corner, outside the column, on screens wide enough to have room ("Updated 2 min ago"). */
  corner?: ReactNode;
}) {
  const agent = by.tab;
  const session = useReportAgent(by);
  const working = agentState(session) === "working";
  const qc = useQueryClient();
  // The one Stop: Esc in the agent's session while it works, and the end of a run of approved actions while one runs.
  const stop = useMutation({
    mutationFn: async () => {
      if (running) await api("/api/decisions/stop", { method: "POST" });
      if (working && session)
        await api("/api/sessions/interrupt", {
          method: "POST",
          body: { session: sessionKey(session.node, session.session_id) },
        });
    },
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["inbox"] }),
    onError: (e) => void notify(`Couldn't stop: ${e.message}`, { error: true, ms: 60_000 }),
  });
  const [textValue, setText] = useState("");
  // "Sending to …" as a toast while the undo window runs and the message goes; it goes once delivered, or turns into
  // the error.
  const sendingNote = useRef<number | null>(null);
  const send = useMutation({
    mutationFn: (t: string) =>
      api<Delivered>(`/api/report/chat${doc ? `?doc=${encodeURIComponent(doc)}` : ""}`, { method: "POST", body: { text: t } }),
    // Delivered: the "Sending" toast just goes (no "Sent" toast).
    onSuccess: () => {
      if (sendingNote.current !== null) dismissNote(sendingNote.current);
      sendingNote.current = null;
    },
    onError: (e) => {
      const text = `Not sent to ${agent}: ${e.message}`;
      if (sendingNote.current !== null) replaceNote(sendingNote.current, text, { error: true, ms: 60_000 });
      else notify(text, { error: true, ms: 60_000 });
      sendingNote.current = null;
    },
  });
  // Sent after the undo window; Send is Undo meanwhile, which puts the text back.
  const undoWindow = useUndoWindow();
  const waiting = useRef("");
  const submit = () => {
    const t = textValue.trim();
    if (!t) return;
    waiting.current = t;
    setText("");
    if (sendingNote.current !== null) dismissNote(sendingNote.current);
    sendingNote.current = notify(`Sending to ${agent}…`, { ms: null });
    undoWindow.start(() => send.mutate(t));
  };
  const undo = () => {
    undoWindow.cancel();
    setText(waiting.current);
    if (sendingNote.current !== null) dismissNote(sendingNote.current);
    sendingNote.current = null;
  };
  return (
    <div className="sticky bottom-0 z-10 mt-10 border-t border-rule bg-paper/95 pb-[env(safe-area-inset-bottom)] backdrop-blur-sm">
      {corner && <div className="absolute right-4 bottom-3 hidden text-right xl:block">{corner}</div>}
      {/* On a phone the message box takes its own row above the buttons, which would otherwise squeeze it. */}
      <div className="relative mx-auto flex max-w-3xl flex-wrap items-center gap-2 px-4 py-3 sm:flex-nowrap">
        <AgentStatus by={by} />
        <Textarea
          rows={1}
          value={textValue}
          maxLength={8000}
          placeholder={`Message ${agent}`}
          aria-label={`Message ${agent}`}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={enterSends(submit)}
          // Padding that centres one line in the 40px box, level with the buttons beside it (its line height
          // isn't a whole number of pixels); more lines grow the box from there.
          className="order-first min-h-10 basis-full py-[calc((2.5rem-2px-1lh)/2)] text-sm sm:order-none sm:basis-auto"
        />
        {/* Pushes the buttons to the right on a phone's second row. */}
        <span className="flex-1 sm:hidden" aria-hidden />
        {undoWindow.pending ? (
          <Button size="sm" variant="secondary" onClick={undo} aria-label="Undo the message" className="h-10 min-w-20">
            <Undo2 aria-hidden /> Undo
          </Button>
        ) : (
          <Button
            size="sm"
            loading={send.isPending}
            disabled={!textValue.trim()}
            onClick={submit}
            className="h-10 min-w-20"
          >
            Send
          </Button>
        )}
        <Button
          size="sm"
          variant="secondary"
          aria-label={running ? "Stop the approved actions" : `Stop ${agent}`}
          title={running ? "Stop carrying out the approved actions" : `Stop ${agent}: Esc in its session`}
          loading={stop.isPending}
          disabled={!working && !running}
          onClick={() => stop.mutate()}
          className="h-10"
        >
          {!stop.isPending && <Square aria-hidden className="size-3 fill-current" />}
          <span className="max-sm:sr-only">Stop</span>
        </Button>
        {children}
      </div>
    </div>
  );
}
