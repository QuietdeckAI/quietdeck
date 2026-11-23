import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { PingsSection, WatchesSection } from "@/components/Pings";
import { ACTION_WORD, RuleEditor, ruleInput } from "@/components/RuleEditor";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import type { Rule } from "@/types/Rule";
import type { RuleInput } from "@/types/RuleInput";
import type { RulesView } from "@/types/RulesView";
import type { RuleStatusChange } from "@/types/RuleStatusChange";

const NEW_RULE: RuleInput = {
  mode: "fuzzy",
  instruction: "",
  criteria: {
    account: null,
    from_address: null,
    from_domain: null,
    subject_contains: null,
    list_id: null,
  },
  action_template: { types: [], target: null },
  session_ref: null,
  project: null,
  topic: null,
  known_sender_domains: [],
  created_from_card_id: null,
};

/** What a rule matches, in a line. */
export function ruleMatches(r: Rule): string {
  if (r.mode === "fuzzy") {
    const from = r.known_sender_domains.length
      ? ` (from ${r.known_sender_domains.join(", ")})`
      : "";
    return `${r.instruction ?? ""}${from}`;
  }
  const c = r.criteria;
  const parts = [
    c.from_address && `From ${c.from_address}`,
    c.from_domain && `From anyone at ${c.from_domain}`,
    c.subject_contains && `subject contains “${c.subject_contains}”`,
    c.list_id && `list ${c.list_id}`,
    c.account && `on ${c.account}`,
  ].filter(Boolean);
  return parts.join(", ");
}

/** What a rule allows, in a line. */
export function ruleAllows(r: Rule): string {
  const words = r.action_template.types
    .map((t) => ACTION_WORD[t].toLowerCase())
    .join(", ");
  const to = r.action_template.target
    ? ` to ${r.action_template.target.tab}`
    : "";
  return `${words}${to}`;
}

/** Auto-approve rules, active and archived, and each project's policy from deck.yaml. */
export function RulesScreen() {
  const view = useQuery({
    queryKey: ["rules"],
    queryFn: () => api<RulesView>("/api/rules"),
  });
  const [creating, setCreating] = useState(false);
  const active = view.data?.rules.filter((r) => r.status === "active") ?? [];
  const inactive = view.data?.rules.filter((r) => r.status !== "active") ?? [];

  return (
    <main className="mx-auto max-w-3xl px-4 py-6">
      <div className="flex items-baseline justify-between gap-4">
        <h1 className="text-2xl">Rules</h1>
        {!creating && (
          <Button size="sm" onClick={() => setCreating(true)}>
            New rule
          </Button>
        )}
      </div>
      <p className="mt-2 max-w-[65ch] text-sm text-neutral">
        What may happen without asking you. Every automatic action still shows
        in the Inbox’s timeline.
      </p>
      {view.isError && <p className="mt-4 text-danger">{view.error.message}</p>}

      {creating && (
        <section className="mt-6 rounded-lg border border-rule bg-paper-2 p-4 sm:p-5">
          <RuleEditor initial={NEW_RULE} onDone={() => setCreating(false)} />
        </section>
      )}

      {view.data && active.length === 0 && !creating && (
        <p className="mt-8 text-neutral">
          No rules yet. Make one here, or from a card with “Auto-approve matches
          in future”.
        </p>
      )}
      <ul className="mt-6">
        {active.map((r) => (
          <RuleRow key={r.id} rule={r} />
        ))}
      </ul>

      {inactive.length > 0 && (
        <section className="mt-10">
          <h2 className="mb-1 text-lg text-muted">Archived</h2>
          <ul>
            {inactive.map((r) => (
              <RuleRow key={r.id} rule={r} />
            ))}
          </ul>
        </section>
      )}

      {view.data && (
        <section className="mt-10">
          <h2 className="mb-1 text-lg text-muted">Project policies</h2>
          <p className="mb-2 text-sm text-neutral">
            From deck.yaml. A rule can allow only what its project’s policy
            allows.
          </p>
          <ul>
            {view.data.policies.map((p) => (
              <li
                key={p.name}
                className="flex flex-wrap items-baseline gap-x-3 border-b border-rule py-2 text-sm last:border-b-0"
              >
                <span className="text-ink">{p.name}</span>
                <span className="min-w-0 flex-1 text-neutral [overflow-wrap:anywhere]">
                  {[...p.dirs, ...p.accounts.map((a) => `account ${a}`)].join(
                    ", ",
                  )}
                </span>
                <span className="text-muted">
                  {p.automatic.length
                    ? p.automatic
                        .map((t) => ACTION_WORD[t].toLowerCase())
                        .join(", ")
                    : "nothing automatic"}
                </span>
              </li>
            ))}
            {view.data.policies.length === 0 && (
              <li className="py-2 text-sm text-neutral">
                No projects in deck.yaml, so nothing can run automatically.
              </li>
            )}
          </ul>
          <p className="mt-3 text-sm text-neutral">
            Automatic resumes and new sessions: at most{" "}
            {view.data.session_starts_per_day} a day.
          </p>
        </section>
      )}

      <WatchesSection />
      <PingsSection />
    </main>
  );
}

function RuleRow({ rule }: { rule: Rule }) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const status = useMutation({
    mutationFn: (change: RuleStatusChange) =>
      api<Rule>(`/api/rules/${rule.id}/status`, {
        method: "POST",
        body: change,
      }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["rules"] }),
  });

  if (editing) {
    return (
      <li className="my-3 rounded-lg border border-rule bg-paper-2 p-4 sm:p-5">
        <RuleEditor
          initial={ruleInput(rule)}
          ruleId={rule.id}
          onDone={() => setEditing(false)}
        />
      </li>
    );
  }
  return (
    <li className="border-b border-rule py-3 last:border-b-0">
      <div className="flex flex-wrap items-start gap-x-3 gap-y-2">
        <div className="min-w-0 flex-1">
          <p className="[overflow-wrap:anywhere]">{ruleMatches(rule)}</p>
          <p className="mt-0.5 text-sm text-neutral">
            {rule.mode === "fuzzy" ? "The agent judges" : "Exact match"} ·{" "}
            {ruleAllows(rule)}
            {rule.last_matched_at && (
              <> · last used {ago(rule.last_matched_at)}</>
            )}
          </p>
          {rule.status !== "active" && rule.archived_reason && (
            <p className="mt-0.5 text-sm text-neutral">
              Archived: {rule.archived_reason}
            </p>
          )}
          {status.error && (
            <p className="mt-1 text-sm text-danger">{status.error.message}</p>
          )}
        </div>
        <div className="flex gap-1">
          {rule.status === "active" ? (
            <>
              <Button
                variant="quiet"
                size="sm"
                onClick={() => setEditing(true)}
              >
                Edit
              </Button>
              <Button
                variant="quiet"
                size="sm"
                loading={status.isPending}
                onClick={() =>
                  status.mutate({ status: "archived", move_to: null })
                }
              >
                Archive
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              loading={status.isPending}
              onClick={() => status.mutate({ status: "active", move_to: null })}
            >
              Restore
            </Button>
          )}
        </div>
      </div>
    </li>
  );
}
