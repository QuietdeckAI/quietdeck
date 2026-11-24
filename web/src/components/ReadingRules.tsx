import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import type { ReadingRule } from "@/types/ReadingRule";
import type { ReadingRuleInput } from "@/types/ReadingRuleInput";

const EMPTY: ReadingRuleInput = { name: "", source: null, account: null, from: null, text_regex: null, label: null };

/** What a rule matches, in a line: "Matrix · Infrastructure Alerts · text matches (?i)\(critical\)". */
function matches(r: ReadingRuleInput): string {
  return [
    r.source === "gmail" ? "Email" : r.source === "matrix" ? "Matrix" : null,
    r.account,
    r.from && `from ${r.from}`,
    r.label && `labelled ${r.label}`,
    r.text_regex && `text matches ${r.text_regex}`,
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * "While I'm reading": while you're on the Inbox, new mail and messages wait until you stop, so nothing changes
 * under you. These rules let matching items through anyway, with a toast.
 */
export function ReadingRulesSection() {
  const rules = useQuery({ queryKey: ["reading-rules"], queryFn: () => api<ReadingRule[]>("/api/reading-rules") });
  const [creating, setCreating] = useState(false);
  return (
    <section className="mt-10">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="text-lg text-muted">While I’m reading</h2>
        {!creating && (
          <Button size="sm" variant="quiet" onClick={() => setCreating(true)}>
            New rule
          </Button>
        )}
      </div>
      <p className="mt-1 max-w-[65ch] text-sm text-neutral">
        While you’re on the Inbox, new mail and messages wait until you stop, so nothing changes under you. Your own
        requests and updates to threads already on screen still come through. These rules let other items through
        too.
      </p>
      {rules.isError && <p className="mt-3 text-sm text-danger">{rules.error.message}</p>}
      {creating && (
        <div className="mt-4 rounded-lg border border-rule bg-paper-2 p-4 sm:p-5">
          <ReadingRuleEditor initial={EMPTY} onDone={() => setCreating(false)} />
        </div>
      )}
      <ul className="mt-3">
        {rules.data?.map((r) => (
          <ReadingRuleRow key={String(r.id)} rule={r} />
        ))}
        {rules.data?.length === 0 && !creating && (
          <li className="py-2 text-sm text-neutral">No rules: everything new waits while you read.</li>
        )}
      </ul>
    </section>
  );
}

function ReadingRuleRow({ rule }: { rule: ReadingRule }) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const refresh = () => void qc.invalidateQueries({ queryKey: ["reading-rules"] });
  const enabled = useMutation({
    mutationFn: (on: boolean) => api(`/api/reading-rules/${rule.id}/enabled`, { method: "POST", body: { enabled: on } }),
    onSuccess: refresh,
  });
  const remove = useMutation({
    mutationFn: () => api(`/api/reading-rules/${rule.id}`, { method: "DELETE" }),
    onSuccess: refresh,
  });
  if (editing) {
    return (
      <li className="my-3 rounded-lg border border-rule bg-paper-2 p-4 sm:p-5">
        <ReadingRuleEditor initial={rule} ruleId={Number(rule.id)} onDone={() => setEditing(false)} />
      </li>
    );
  }
  const error = enabled.error ?? remove.error;
  return (
    <li className="flex flex-wrap items-start gap-x-3 gap-y-2 border-b border-rule py-3 last:border-b-0">
      <Switch
        checked={rule.enabled}
        onCheckedChange={(on) => enabled.mutate(on)}
        aria-label={rule.enabled ? `Turn off ${rule.name}` : `Turn on ${rule.name}`}
        className="mt-0.5"
      />
      <div className="min-w-0 flex-1">
        <p className={rule.enabled ? "text-ink" : "text-neutral"}>{rule.name}</p>
        <p className="mt-0.5 text-sm text-neutral [overflow-wrap:anywhere]">
          {matches(rule)}
          {rule.last_matched_at && <> · let through {ago(rule.last_matched_at)}</>}
        </p>
        {error && <p className="mt-1 text-sm text-danger">{error.message}</p>}
      </div>
      <div className="flex gap-1">
        <Button variant="quiet" size="sm" onClick={() => setEditing(true)}>
          Edit
        </Button>
        <Button variant="quiet" size="sm" loading={remove.isPending} onClick={() => remove.mutate()}>
          Delete
        </Button>
      </div>
    </li>
  );
}

function ReadingRuleEditor({
  initial,
  ruleId,
  onDone,
}: {
  initial: ReadingRuleInput;
  ruleId?: number;
  onDone: () => void;
}) {
  const qc = useQueryClient();
  const [r, setR] = useState<ReadingRuleInput>({
    name: initial.name,
    source: initial.source,
    account: initial.account,
    from: initial.from,
    text_regex: initial.text_regex,
    label: initial.label,
  });
  const set = (k: keyof ReadingRuleInput, v: string) =>
    setR((all) => ({ ...all, [k]: k === "name" ? v : v.trim() === "" ? null : v }));
  const save = useMutation({
    mutationFn: () =>
      ruleId === undefined
        ? api("/api/reading-rules", { method: "POST", body: r })
        : api(`/api/reading-rules/${ruleId}`, { method: "PUT", body: r }),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: ["reading-rules"] });
      onDone();
    },
  });
  const field = (k: keyof ReadingRuleInput, label: string, placeholder: string, hint?: string) => (
    <label className="block">
      <span className="text-sm text-muted">{label}</span>
      <Input
        className="mt-1"
        value={r[k] ?? ""}
        placeholder={placeholder}
        onChange={(e) => set(k, e.target.value)}
        spellCheck={false}
      />
      {hint && <span className="mt-1 block text-xs text-neutral">{hint}</span>}
    </label>
  );
  return (
    <form
      className="grid gap-3 sm:grid-cols-2"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
    >
      <div className="sm:col-span-2">{field("name", "Name", "Critical alerts")}</div>
      <label className="block">
        <span className="text-sm text-muted">Source</span>
        <select
          className="mt-1 h-10 w-full rounded-md border border-rule bg-paper px-3 text-ink"
          value={r.source ?? ""}
          onChange={(e) => set("source", e.target.value)}
        >
          <option value="">Email or Matrix</option>
          <option value="gmail">Email</option>
          <option value="matrix">Matrix</option>
        </select>
      </label>
      {field("account", "Account or room", "Infrastructure Alerts")}
      {field("from", "From contains", "@alertmanager:")}
      {field("label", "Gmail label", "IMPORTANT")}
      <div className="sm:col-span-2">
        {field(
          "text_regex",
          "Subject or text matches",
          "(?i)\\(critical\\)",
          "A regular expression in RE2 syntax. (?i) ignores case.",
        )}
      </div>
      {save.error && <p className="text-sm text-danger sm:col-span-2">{save.error.message}</p>}
      <div className="flex gap-2 sm:col-span-2">
        <Button type="submit" size="sm" loading={save.isPending} disabled={!r.name.trim()}>
          {ruleId === undefined ? "Add rule" : "Save"}
        </Button>
        <Button type="button" size="sm" variant="quiet" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
