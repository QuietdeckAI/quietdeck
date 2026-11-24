import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Markdown as Render } from "markdown-to-jsx/react";
import { Loader2, MessageSquare, Square, Undo2 } from "lucide-react";
import { Children, createContext, isValidElement, type ReactNode, use, useEffect, useRef, useState } from "react";
import { useInfo } from "@/components/AccountTabs";
import { EmailMenu } from "@/components/EmailMenu";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { Card } from "@/types/Card";
import type { Delivered } from "@/types/Delivered";
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

/** Stop: Esc in the agent's session, as in Claude Code. It ends its turn and does nothing further. */
export function useStopAgent(agent: SessionRow | undefined) {
  return useMutation({
    mutationFn: () => {
      if (!agent) throw new Error("the agent isn't running");
      return api("/api/sessions/interrupt", { method: "POST", body: { session: sessionKey(agent.node, agent.session_id) } });
    },
  });
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
  return <span className="block text-sm text-neutral">{working ? `${tab} is working on it` : `Sent to ${tab}.`}</span>;
}

/** Outside the panel at its right edge, level with its first line: the agent is working on this change now. */
function WorkingSpinner({ answer }: { answer: ReportAnswer | undefined }) {
  const { report } = useReport();
  if (!useWorkingOn(answer)) return null;
  return (
    <Loader2
      role="img"
      aria-label={`${report.posted_by.tab} is working on this`}
      className="absolute top-1.5 left-full ml-2 size-3.5 animate-spin text-neutral motion-reduce:animate-none max-sm:ml-0.5"
    />
  );
}

type Ctx = {
  report: Report;
  /** The cards the report's defaults name, by ID. */
  cards: Map<number, Card>;
  /** The sessions running now, for `tab` names in the text. */
  sessions: SessionRow[];
  drafts: Record<string, string>;
  setDraft: (key: string, text: string) => void;
};
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
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["inbox"] }),
  });
}

function latest(answers: ReportAnswer[], control: string): ReportAnswer | undefined {
  return answers.filter((a) => a.control === control).at(-1);
}

/**
 * `<Default cards="c_1 c_2">forward, then archive</Default>`: the text in a compact box of its own, which marks it as
 * the default without a label, and a small Change button at its right edge that opens, inside the same box, a
 * field to say what to change: usually an adjustment or an addition. The item's headline stays the only bold text in it.
 */
function Default({ id, cards, children }: { id?: string; cards?: string; children?: ReactNode }) {
  const { report, cards: byId, drafts, setDraft } = useReport();
  const key = defaultKey(id, cards);
  const [open, setOpen] = useState(false);
  const send = useAnswer();
  const change = report.answers.filter((a) => a.kind === "change" && a.control === key && a.report_version === report.version).at(-1);
  const draft = drafts[key] ?? "";
  const about = text(children).trim();
  const working = useWorkingOn(change);
  const stop = useStopAgent(useReportAgent(report.posted_by));
  // A sent change waits out the undo window as a draft: Approve All keeps its cards out meanwhile, and sends it first.
  const undoWindow = useUndoWindow();
  const [sending, setSending] = useState<string | null>(null);
  const latestDraft = useRef(draft);
  latestDraft.current = draft;
  const submit = () => {
    const value = draft.trim();
    setOpen(false);
    setSending(value);
    undoWindow.start(() => {
      setSending(null);
      // Approve All sent it already (and cleared the draft), or it was edited since.
      if (latestDraft.current.trim() !== value) return;
      send.mutate(
        { report_id: report.id, report_version: report.version, control: key, kind: "change", about, value },
        { onSuccess: () => latestDraft.current.trim() === value && setDraft(key, "") },
      );
    });
  };
  const undo = () => {
    undoWindow.cancel();
    setSending(null);
    setOpen(true);
  };
  // In the undo window, Change is Undo; while the agent works on the change, Stop; after, Change again, to say it
  // differently.
  const changeButton = undoWindow.pending ? (
    <UndoButton onClick={undo} label={`Undo the change to: ${about}`} />
  ) : working ? (
    <button
      type="button"
      onClick={() => stop.mutate()}
      disabled={stop.isPending}
      aria-label={`Stop ${report.posted_by.tab}`}
      className={smallButton}
    >
      <Square aria-hidden className="size-3! fill-current" />
      Stop
    </button>
  ) : (
    <button
      type="button"
      onClick={() => {
        if (!open && change && !draft) setDraft(key, change.value);
        setOpen(!open);
      }}
      aria-expanded={open}
      aria-label={`Change: ${about}`}
      title="Change"
      className={cn(smallButton, "w-7 px-0")}
    >
      <MessageSquare aria-hidden />
    </button>
  );
  const named = (cards ?? "")
    .split(/[\s,]+/)
    .map((c) => byId.get(Number(c.replace(/^c_/, ""))))
    .filter((c): c is Card => c !== undefined);
  return (
    <span className="report-default relative rounded-md border border-rule bg-paper-2 py-1 pr-1 pl-3">
      <WorkingSpinner answer={change} />
      <span className="-my-0.5 flex items-start gap-0.5">
        <span className="min-w-0 flex-1 py-0.5 text-ink">
          {children}
        </span>
        {changeButton}
        <EmailMenu about={about} cards={named} />
      </span>
      {sending !== null ? (
        <span className="block text-accent">
          {sending}
          <span className="block text-sm text-neutral">Sending to {report.posted_by.tab}…</span>
        </span>
      ) : (
        change && (
          <span className="block text-accent">
            {change.value}
            <SentStatus answer={change} />
          </span>
        )
      )}
      {open && !working && !undoWindow.pending && (
        <span className="mt-1.5 mb-1 flex flex-col gap-2 sm:flex-row sm:items-end">
          <Textarea
            rows={1}
            autoFocus
            value={draft}
            maxLength={4000}
            placeholder="What to change"
            aria-label={`Change to: ${about}`}
            onChange={(e) => setDraft(key, e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && draft.trim()) submit();
            }}
            className="bg-paper text-sm"
          />
          <Button size="sm" loading={send.isPending} disabled={!draft.trim()} onClick={submit} className="h-10 shrink-0">
            Send
          </Button>
        </span>
      )}
      {(send.error ?? stop.error) && (
        <span className="block text-sm text-danger">{(send.error ?? stop.error)?.message}</span>
      )}
    </span>
  );
}

/** `<Ask id="signin" options="Yes|No" other>Was this you?</Ask>`: the question, a button per option, Other. */
function Ask({ id, options, other, children }: { id?: string; options?: string; other?: unknown; children?: ReactNode }) {
  const { report } = useReport();
  const control = id ?? text(children).trim();
  const about = text(children).trim();
  const choices = (options ?? "").split("|").map((o) => o.trim()).filter(Boolean);
  const answered = latest(report.answers, control);
  const [otherOpen, setOtherOpen] = useState(false);
  const [otherText, setOtherText] = useState("");
  const send = useAnswer();
  const undoWindow = useUndoWindow();
  const [sending, setSending] = useState<string | null>(null);
  const answer = (value: string) => {
    setSending(value);
    setOtherOpen(false);
    undoWindow.start(() => {
      setSending(null);
      send.mutate(
        { report_id: report.id, report_version: report.version, control, kind: "choice", about, value },
        { onSuccess: () => setOtherText("") },
      );
    });
  };
  const undo = () => {
    undoWindow.cancel();
    // An Other answer goes back into its box.
    if (sending !== null && !choices.includes(sending)) setOtherOpen(true);
    setSending(null);
  };
  const shown = sending ?? answered?.value;
  const otherOn = other !== undefined && other !== false && other !== "false";
  return (
    <span className="report-ask relative">
      <WorkingSpinner answer={answered} />
      <span>{children}</span>{" "}
      <span className="inline-flex flex-wrap items-center gap-1.5 align-baseline">
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
            onClick={() => setOtherOpen(!otherOpen)}
            className="h-7"
          >
            Other
          </Button>
        )}
        {undoWindow.pending && <UndoButton onClick={undo} label={`Undo the answer to: ${about}`} />}
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
            autoFocus
            value={otherText}
            maxLength={4000}
            placeholder="Your answer"
            aria-label={`Answer to: ${about}`}
            onChange={(e) => setOtherText(e.target.value)}
            className="text-sm"
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
  overrides: { Default, Ask, a: Link, code: Code },
};

/**
 * Only the report's own two tags are HTML: every other `<` is escaped first, so a tag an email slipped into the
 * agent's text shows as text and never reaches the page as markup.
 */
export function onlyReportTags(markdown: string): string {
  return markdown.replace(/<(?!\/?(?:Default|Ask)\b)/g, "&lt;");
}

/** The triage agent's report, as Markdown with its controls. */
export function ReportBody({
  report,
  cards,
  drafts,
  setDraft,
}: {
  report: Report;
  cards: Map<number, Card>;
  drafts: Record<string, string>;
  setDraft: (key: string, text: string) => void;
}) {
  const running = useRunningSessions();
  return (
    <ReportCtx value={{ report, cards, sessions: running.data ?? [], drafts, setDraft }}>
      <Render className="md report" options={OPTIONS}>
        {onlyReportTags(report.markdown)}
      </Render>
    </ReportCtx>
  );
}

/** The chat box under the report: plain words straight to the agent that writes it. Nothing is kept. Approve All
 * sits at its right end (children), the agent's status at its left, and one Stop for the agent and the run. */
export function ReportChatBox({ by, running, children }: { by: SessionRef; running: boolean; children?: ReactNode }) {
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
  });
  const [textValue, setText] = useState("");
  const [sent, setSent] = useState(false);
  const send = useMutation({
    mutationFn: (t: string) => api<Delivered>("/api/report/chat", { method: "POST", body: { text: t } }),
    onSuccess: () => setSent(true),
  });
  // Sent after the undo window; Send is Undo meanwhile, which puts the text back.
  const undoWindow = useUndoWindow();
  const waiting = useRef("");
  const submit = () => {
    const t = textValue.trim();
    if (!t) return;
    waiting.current = t;
    setText("");
    setSent(false);
    undoWindow.start(() => send.mutate(t));
  };
  const undo = () => {
    undoWindow.cancel();
    setText(waiting.current);
  };
  return (
    <div className="sticky bottom-0 z-10 mt-10 border-t border-rule bg-paper/95 pb-[env(safe-area-inset-bottom)] backdrop-blur-sm">
      <div className="relative mx-auto flex max-w-3xl items-center gap-2 px-4 py-3">
        <AgentStatus by={by} />
        <Textarea
          rows={1}
          value={textValue}
          maxLength={8000}
          placeholder={`Message ${agent}`}
          aria-label={`Message ${agent}`}
          onChange={(e) => {
            setText(e.target.value);
            setSent(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
          className="min-h-10 text-sm"
        />
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
      {(send.error || stop.error || sent || undoWindow.pending) && (
        <p
          role={send.error || stop.error ? "alert" : "status"}
          className={cn("mx-auto max-w-3xl px-4 pb-2 text-sm", send.error || stop.error ? "text-danger" : "text-neutral")}
        >
          {(send.error ?? stop.error)?.message ?? (undoWindow.pending ? `Sending to ${agent}…` : `Sent to ${agent}.`)}
        </p>
      )}
    </div>
  );
}
