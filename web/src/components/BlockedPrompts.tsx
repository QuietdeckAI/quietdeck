import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleAlert } from "lucide-react";
import { useState } from "react";
import { SessionMenu } from "@/components/SessionMenu";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { api } from "@/lib/api";
import { notify } from "@/lib/notes";
import { sessionKey } from "@/screens/Sessions";
import { cn } from "@/lib/utils";
import type { BlockedPrompt } from "@/types/BlockedPrompt";
import type { PromptAnswer } from "@/types/PromptAnswer";
import type { SessionRow } from "@/types/SessionRow";

/** Picks per question: option numbers, and the Other text. */
type Choice = { picks: number[]; text: string };

/**
 * Sessions blocked on one of the harness's own menus (Claude Code's question form, a permission prompt, a plan to
 * approve), at the top of the Work tab: each as a small form. The answer is typed into the session through Fleetlayer.
 * Every blocked session shows here, ignored ones included: a prompt always needs him.
 */
export function BlockedPrompts() {
  const sessions = useQuery({ queryKey: ["sessions", "running"], queryFn: () => api<SessionRow[]>("/api/sessions") });
  const blocked = (sessions.data ?? []).filter((s) => s.prompt && s.running);
  if (blocked.length === 0) return null;
  return (
    <section aria-label="Blocked on a prompt" className="mb-8">
      <h1 className="font-display text-[1.75rem] leading-tight text-ink">Blocked on a prompt</h1>
      <div className="mt-3 flex flex-col gap-3">
        {blocked.map((s) => s.prompt && <PromptForm key={`${s.node}/${s.session_id}/${s.prompt.id}`} session={s} prompt={s.prompt} />)}
      </div>
    </section>
  );
}

function PromptForm({ session, prompt }: { session: SessionRow; prompt: BlockedPrompt }) {
  const qc = useQueryClient();
  const address = `${session.tab ?? session.session_id}@${session.node}`;
  // A question form has questions; a permission or plan prompt is one question with its own options.
  const questions =
    prompt.kind === "question" && prompt.questions.length > 0
      ? prompt.questions
      : [
          {
            header: null,
            question:
              prompt.kind === "permission"
                ? `Allow ${prompt.tool ?? "this tool"}?`
                : prompt.kind === "plan"
                  ? "Approve the plan?"
                  : "Answer",
            multiSelect: false,
            options: prompt.options,
            other: false,
          },
        ];
  const [choices, setChoices] = useState<Choice[]>(() => questions.map(() => ({ picks: [], text: "" })));
  const set = (i: number, c: Choice) => setChoices((all) => all.map((x, j) => (j === i ? c : x)));
  const answers: (PromptAnswer | null)[] = questions.map((q, i) => {
    const c = choices[i];
    if (c.text.trim()) return { text: c.text.trim() };
    if (q.multiSelect) return c.picks.length > 0 ? { picks: c.picks } : null;
    return c.picks.length > 0 ? { pick: c.picks[0] } : null;
  });
  const ready = answers.every((a) => a !== null);
  const send = useMutation({
    mutationFn: () =>
      api("/api/sessions/answer", {
        method: "POST",
        body: { session: sessionKey(session.node, session.session_id), prompt: prompt.id, answers },
      }),
    onSuccess: () => {
      notify(`Answered ${address}`, { ms: 4000 });
      void qc.invalidateQueries({ queryKey: ["sessions"] });
    },
    onError: (e) => notify(`Not answered: ${e.message}`, { error: true, ms: 30_000 }),
  });
  return (
    <div className="report-default rounded-md border border-busy/50 bg-paper-2 py-2 pr-1.5 pl-3">
      <div className="flex items-start gap-2">
        <CircleAlert aria-hidden className="mt-[3px] size-4 shrink-0 text-busy" />
        <p className="min-w-0 flex-1 text-ink">
          <span className="font-mono text-sm">{session.tab ?? session.session_id}</span>{" "}
          <span className="text-sm text-neutral">on {session.node} is waiting on a {prompt.kind === "question" ? "question" : prompt.kind}</span>
        </p>
        <span className="-my-1">
          <SessionMenu session={address} about={questions[0]?.question ?? "Blocked prompt"} />
        </span>
      </div>
      {prompt.kind === "permission" && prompt.tool_input && (
        <pre className="mt-2 mr-1.5 max-h-40 overflow-auto rounded bg-paper p-2 text-xs whitespace-pre-wrap text-muted">{prompt.tool_input}</pre>
      )}
      {prompt.kind === "plan" && prompt.plan && (
        <pre className="mt-2 mr-1.5 max-h-60 overflow-auto rounded bg-paper p-2 text-xs whitespace-pre-wrap text-muted">{prompt.plan}</pre>
      )}
      {questions.map((q, i) => (
        <fieldset key={i} className="mt-2.5 mr-1.5">
          <legend className="text-sm text-ink">
            {q.header && <span className="mr-1.5 rounded bg-paper-3 px-1.5 py-0.5 text-xs text-muted">{q.header}</span>}
            {q.question}
          </legend>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {q.options.map((o) => {
              const on = choices[i].picks.includes(o.n);
              return (
                <Button
                  key={o.n}
                  size="sm"
                  variant={on ? "primary" : "secondary"}
                  aria-pressed={on}
                  title={o.description ?? undefined}
                  className="h-auto min-h-7 py-1 text-left whitespace-normal"
                  onClick={() =>
                    set(i, {
                      text: "",
                      picks: q.multiSelect ? (on ? choices[i].picks.filter((n) => n !== o.n) : [...choices[i].picks, o.n]) : [o.n],
                    })
                  }
                >
                  {o.label}
                </Button>
              );
            })}
          </div>
          {q.options.some((o) => o.description) && (
            <ul className="mt-1 text-xs text-neutral">
              {q.options.filter((o) => o.description).map((o) => (
                <li key={o.n}>
                  <span className="text-muted">{o.label}:</span> {o.description}
                </li>
              ))}
            </ul>
          )}
          {q.other && (
            <Textarea
              rows={1}
              value={choices[i].text}
              onChange={(e) => set(i, { picks: [], text: e.target.value })}
              placeholder="Other"
              aria-label={`Other answer to: ${q.question}`}
              className={cn("mt-1.5 bg-paper py-[7px] text-sm leading-5")}
            />
          )}
        </fieldset>
      ))}
      <div className="mt-2.5 mr-1.5 flex justify-end">
        <Button size="sm" loading={send.isPending} disabled={!ready} onClick={() => send.mutate()} className="h-8">
          Answer
        </Button>
      </div>
    </div>
  );
}
