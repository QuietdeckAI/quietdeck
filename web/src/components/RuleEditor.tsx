import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useInfo } from "@/components/AccountTabs";
import { Button } from "@/components/ui/button";
import { fieldClass, Input, Textarea } from "@/components/ui/input";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { ActionType } from "@/types/ActionType";
import type { Card } from "@/types/Card";
import type { Rule } from "@/types/Rule";
import type { RuleInput } from "@/types/RuleInput";
import type { RulesView } from "@/types/RulesView";
import type { SessionRef } from "@/types/SessionRef";

export const ACTION_WORD: Record<ActionType, string> = {
  archive: "Archive",
  mark_read: "Mark read",
  not_spam: "Mark as not spam",
  label: "Label",
  forward: "Forward",
  draft: "Draft",
  reply: "Reply",
  story: "Story",
  task: "Task",
  calendar: "Calendar",
  resume: "Resume",
  new_session: "New session",
  lease: "Lease",
  watch: "Watch",
  session: "Session",
};

const TARGETED: ActionType[] = ["forward", "resume", "new_session"];

/** The session a card's forward, resume or new session goes to. */
function cardTarget(card: Card): SessionRef | null {
  for (const a of card.actions) {
    if (a.type === "forward")
      return { node: a.to.node, tab: a.to.tab, session_id: null };
    if (a.type === "resume" || a.type === "new_session")
      return { node: a.node, tab: a.tab, session_id: null };
  }
  return null;
}

/** A new rule from a card: fuzzy, with the card's sender domain known and its action types allowed. */
export function ruleFromCard(card: Card): RuleInput {
  const domain = card.sender?.address.split("@")[1]?.toLowerCase() ?? null;
  const types = [...new Set(card.actions.map((a) => a.type))];
  return {
    mode: "fuzzy",
    instruction: `Mail like "${card.subject}"${card.sender ? ` from ${card.sender.name ?? card.sender.address}` : ""}`,
    criteria: {
      account: card.account,
      from_address: null,
      from_domain: domain,
      subject_contains: null,
      list_id: card.list_id,
    },
    action_template: { types, target: cardTarget(card) },
    session_ref: null,
    project: null,
    topic: null,
    known_sender_domains: domain ? [domain] : [],
    created_from_card_id: card.id,
  };
}

export function ruleInput(r: Rule): RuleInput {
  return {
    mode: r.mode,
    instruction: r.instruction,
    criteria: r.criteria,
    action_template: r.action_template,
    session_ref: r.session_ref,
    project: r.project,
    topic: r.topic,
    known_sender_domains: r.known_sender_domains,
    created_from_card_id: r.created_from_card_id,
  };
}

const blank = (s: string) => (s.trim() === "" ? null : s.trim());

function parseTarget(s: string): SessionRef | null {
  const t = s.trim();
  if (!t) return null;
  const at = t.lastIndexOf("@");
  return at > 0
    ? { node: t.slice(at + 1), tab: t.slice(0, at), session_id: null }
    : { node: "", tab: t, session_id: null };
}

/**
 * Write or edit a grant. Fuzzy (the agent judges matches from the instruction, for known sender domains only) is the
 * default; strict matches the card's fields on the deck and needs DMARC or DKIM to pass.
 */
export function RuleEditor({
  initial,
  ruleId,
  onDone,
}: {
  initial: RuleInput;
  ruleId?: number;
  onDone: () => void;
}) {
  const qc = useQueryClient();
  const info = useInfo();
  const view = useQuery({
    queryKey: ["rules"],
    queryFn: () => api<RulesView>("/api/rules"),
  });
  const [r, setR] = useState<RuleInput>(initial);
  const [domains, setDomains] = useState(
    initial.known_sender_domains.join(", "),
  );
  const [target, setTarget] = useState(
    initial.action_template.target
      ? `${initial.action_template.target.tab}@${initial.action_template.target.node}`
      : "",
  );
  const types = r.action_template.types;
  const needsTarget = types.some((t) => TARGETED.includes(t));

  const save = useMutation({
    mutationFn: () => {
      const body: RuleInput = {
        ...r,
        instruction: blank(r.instruction ?? ""),
        known_sender_domains: domains
          .split(/[\s,]+/)
          .map((d) => d.trim())
          .filter(Boolean),
        action_template: {
          types,
          target: needsTarget ? parseTarget(target) : null,
        },
      };
      return ruleId === undefined
        ? api<Rule>("/api/rules", { method: "POST", body })
        : api<Rule>(`/api/rules/${ruleId}`, { method: "PUT", body });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["rules"] });
      onDone();
    },
  });

  const set = (patch: Partial<RuleInput>) => setR((x) => ({ ...x, ...patch }));
  const setCriteria = (patch: Partial<RuleInput["criteria"]>) =>
    setR((x) => ({ ...x, criteria: { ...x.criteria, ...patch } }));
  const toggle = (t: ActionType) =>
    setR((x) => ({
      ...x,
      action_template: {
        ...x.action_template,
        types: x.action_template.types.includes(t)
          ? x.action_template.types.filter((y) => y !== t)
          : [...x.action_template.types, t],
      },
    }));
  const offered: ActionType[] = [
    ...new Set<ActionType>([
      ...types,
      "archive",
      "mark_read",
      "label",
      "forward",
      "story",
      "task",
    ]),
  ];

  return (
    <form
      className="flex min-w-0 flex-col gap-4"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
    >
      <fieldset
        className="flex gap-1 rounded-md bg-paper-3 p-1 text-sm"
        aria-label="How it matches"
      >
        {(["fuzzy", "strict"] as const).map((m) => (
          <label
            key={m}
            className={cn(
              "flex-1 cursor-pointer rounded px-3 py-1.5 text-center text-muted",
              r.mode === m && "bg-paper-2 text-ink shadow-sm",
            )}
          >
            <input
              type="radio"
              name="mode"
              className="sr-only"
              checked={r.mode === m}
              onChange={() => set({ mode: m })}
            />
            {m === "fuzzy" ? "The agent judges" : "Exact match"}
          </label>
        ))}
      </fieldset>

      {r.mode === "fuzzy" ? (
        <>
          <label className="flex min-w-0 flex-col gap-1 text-sm text-muted">
            What counts as a match
            <Textarea
              rows={2}
              maxLength={2000}
              value={r.instruction ?? ""}
              onChange={(e) => set({ instruction: e.target.value })}
            />
          </label>
          <label className="flex min-w-0 flex-col gap-1 text-sm text-muted">
            Only from these sender domains
            <Input
              value={domains}
              onChange={(e) => setDomains(e.target.value)}
              placeholder="example.com, mail.example.com"
            />
          </label>
        </>
      ) : (
        <div className="grid grid-cols-[minmax(0,1fr)] gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
          <label className="flex min-w-0 flex-col gap-1 text-sm text-muted">
            Account
            <select
              className={cn(fieldClass, "h-10 min-w-0 truncate")}
              value={r.criteria.account ?? ""}
              onChange={(e) => setCriteria({ account: blank(e.target.value) })}
            >
              <option value="">Choose…</option>
              {info.data?.accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.label}
                </option>
              ))}
            </select>
          </label>
          <label className="flex min-w-0 flex-col gap-1 text-sm text-muted">
            From domain
            <Input
              value={r.criteria.from_domain ?? ""}
              onChange={(e) =>
                setCriteria({ from_domain: blank(e.target.value) })
              }
            />
          </label>
          <label className="flex min-w-0 flex-col gap-1 text-sm text-muted">
            Or from address
            <Input
              value={r.criteria.from_address ?? ""}
              onChange={(e) =>
                setCriteria({ from_address: blank(e.target.value) })
              }
            />
          </label>
          <label className="flex min-w-0 flex-col gap-1 text-sm text-muted">
            Subject contains (optional)
            <Input
              value={r.criteria.subject_contains ?? ""}
              onChange={(e) =>
                setCriteria({ subject_contains: blank(e.target.value) })
              }
            />
          </label>
        </div>
      )}

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 text-sm text-muted">
          Allowed without asking
        </legend>
        <div className="flex flex-wrap gap-x-4 gap-y-2">
          {offered.map((t) => (
            <label key={t} className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                className="size-4 accent-(--color-accent)"
                checked={types.includes(t)}
                onChange={() => toggle(t)}
              />
              {ACTION_WORD[t]}
            </label>
          ))}
        </div>
      </fieldset>

      {needsTarget && (
        <label className="flex min-w-0 flex-col gap-1 text-sm text-muted">
          To the session in
          <Input
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            placeholder="tab@machine"
          />
        </label>
      )}

      {view.data && view.data.policies.length > 0 && (
        <label className="flex min-w-0 flex-col gap-1 text-sm text-muted">
          Project policy
          <select
            className={cn(fieldClass, "h-10 min-w-0 truncate")}
            value={r.project ?? ""}
            onChange={(e) => set({ project: blank(e.target.value) })}
          >
            <option value="">From each action’s session or account</option>
            {view.data.policies.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
      )}

      {save.error && (
        <p role="alert" className="text-sm text-danger">
          {save.error.message}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="submit"
          variant="primary"
          size="sm"
          loading={save.isPending}
        >
          {ruleId === undefined ? "Create rule" : "Save rule"}
        </Button>
        <Button type="button" variant="quiet" size="sm" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
