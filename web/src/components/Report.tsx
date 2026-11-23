import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Markdown as Render } from "markdown-to-jsx/react";
import { Children, createContext, isValidElement, type ReactNode, use, useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { Delivered } from "@/types/Delivered";
import type { Report } from "@/types/Report";
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

type Ctx = {
  report: Report;
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

/** `<Default cards="c_1 c_2">forward, then archive</Default>`: the default, and Change this to say what to do instead. */
function Default({ id, cards, children }: { id?: string; cards?: string; children?: ReactNode }) {
  const { report, drafts, setDraft } = useReport();
  const key = defaultKey(id, cards);
  const [open, setOpen] = useState(false);
  const send = useAnswer();
  const change = report.answers.filter((a) => a.kind === "change" && a.control === key && a.report_version === report.version).at(-1);
  const draft = drafts[key] ?? "";
  const about = text(children).trim();
  const submit = () =>
    send.mutate(
      { report_id: report.id, report_version: report.version, control: key, kind: "change", about, value: draft.trim() },
      {
        onSuccess: () => {
          setDraft(key, "");
          setOpen(false);
        },
      },
    );
  return (
    <span className="report-default">
      <span className={cn(change && "text-neutral line-through decoration-rule-strong")}>Default: {children}</span>{" "}
      {!change && (
        <button
          type="button"
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className="whitespace-nowrap text-sm text-accent underline-offset-4 hover:underline"
        >
          Change this
        </button>
      )}
      {change && (
        <span className="block text-accent">
          Instead: {change.value}
          {!change.picked_up && <span className="text-neutral"> (waiting for the agent)</span>}
        </span>
      )}
      {open && !change && (
        <span className="mt-1.5 flex flex-col gap-2 sm:flex-row sm:items-start">
          <Textarea
            rows={2}
            autoFocus
            value={draft}
            maxLength={4000}
            placeholder="What to do instead"
            aria-label={`Instead of: ${about}`}
            onChange={(e) => setDraft(key, e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && draft.trim()) submit();
            }}
            className="text-sm"
          />
          <Button size="sm" loading={send.isPending} disabled={!draft.trim()} onClick={submit}>
            Send
          </Button>
        </span>
      )}
      {send.error && <span className="block text-sm text-danger">{send.error.message}</span>}
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
  const answer = (value: string) =>
    send.mutate(
      { report_id: report.id, report_version: report.version, control, kind: "choice", about, value },
      {
        onSuccess: () => {
          setOtherOpen(false);
          setOtherText("");
        },
      },
    );
  const otherOn = other !== undefined && other !== false && other !== "false";
  return (
    <span className="report-ask">
      <span>{children}</span>{" "}
      <span className="inline-flex flex-wrap items-center gap-1.5 align-baseline">
        {choices.map((c) => (
          <Button
            key={c}
            size="sm"
            variant={answered?.value === c ? "primary" : "secondary"}
            aria-pressed={answered?.value === c}
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
      </span>
      {answered && !choices.includes(answered.value) && (
        <span className="block text-accent">You said: {answered.value}</span>
      )}
      {answered && !answered.picked_up && <span className="block text-sm text-neutral">Waiting for the agent.</span>}
      {otherOpen && (
        <span className="mt-1.5 flex flex-col gap-2 sm:flex-row sm:items-start">
          <Textarea
            rows={2}
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
          >
            Send
          </Button>
        </span>
      )}
      {send.error && <span className="block text-sm text-danger">{send.error.message}</span>}
    </span>
  );
}

function Link(props: React.ComponentProps<"a">) {
  return <a {...props} target="_blank" rel="noreferrer noopener" />;
}

const OPTIONS = {
  forceBlock: true,
  overrides: { Default, Ask, a: Link },
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
  drafts,
  setDraft,
}: {
  report: Report;
  drafts: Record<string, string>;
  setDraft: (key: string, text: string) => void;
}) {
  return (
    <ReportCtx value={{ report, drafts, setDraft }}>
      <Render className="md report" options={OPTIONS}>
        {onlyReportTags(report.markdown)}
      </Render>
    </ReportCtx>
  );
}

/** The chat box under the report: plain words straight to the agent that writes it. Nothing is kept. */
export function ReportChatBox({ agent }: { agent: string }) {
  const [textValue, setText] = useState("");
  const [sent, setSent] = useState(false);
  const send = useMutation({
    mutationFn: (t: string) => api<Delivered>("/api/report/chat", { method: "POST", body: { text: t } }),
    onSuccess: () => {
      setText("");
      setSent(true);
    },
  });
  const submit = () => {
    if (textValue.trim()) send.mutate(textValue.trim());
  };
  return (
    <div className="sticky bottom-0 z-10 mt-10 border-t border-rule bg-paper/95 pb-[env(safe-area-inset-bottom)] backdrop-blur-sm">
      <div className="mx-auto flex max-w-3xl items-end gap-2 px-4 py-3">
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
        <Button loading={send.isPending} disabled={!textValue.trim()} onClick={submit}>
          Send
        </Button>
      </div>
      {(send.error || sent) && (
        <p
          role={send.error ? "alert" : "status"}
          className={cn("mx-auto max-w-3xl px-4 pb-2 text-sm", send.error ? "text-danger" : "text-neutral")}
        >
          {send.error ? send.error.message : `Sent to ${agent}.`}
        </p>
      )}
    </div>
  );
}
