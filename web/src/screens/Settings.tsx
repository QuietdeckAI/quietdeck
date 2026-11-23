import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, KeyRound, Pencil, Trash2 } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { fieldClass, Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { when } from "@/lib/format";
import { cn } from "@/lib/utils";
import { createPasskey, guessDeviceName } from "@/lib/webauthn";
import type { AuthState } from "@/types/AuthState";
import type { Ceremony } from "@/types/Ceremony";
import type { EnrolmentCode } from "@/types/EnrolmentCode";
import type { NodeRow } from "@/types/NodeRow";
import type { PasskeyInfo } from "@/types/PasskeyInfo";
import type { Settings } from "@/types/Settings";

export function SettingsScreen() {
  return (
    <main className="mx-auto flex max-w-3xl flex-col gap-12 px-4 py-8">
      <DeckSettings />
      <Passkeys />
      <Nodes />
      <SignOut />
    </main>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <h2 className="mb-4 text-xl">{title}</h2>
      {children}
    </section>
  );
}

const UNDO_CHOICES = [0, 1, 2, 3, 5, 10, 15, 30, 60];

function DeckSettings() {
  const qc = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: () => api<Settings>("/api/settings") });
  const save = useMutation({
    mutationFn: (s: Settings) => api<Settings>("/api/settings", { method: "PUT", body: s }),
    onMutate: (s) => qc.setQueryData(["settings"], s),
    onSettled: () => qc.invalidateQueries({ queryKey: ["settings"] }),
  });
  const s = settings.data;
  return (
    <Section title="Approving">
      {s && (
        <div className="flex flex-col divide-y divide-rule rounded-lg border border-rule bg-paper-2">
          <label className="flex items-center justify-between gap-4 p-4">
            <span>
              <span className="block">Undo window</span>
              <span className="block text-sm text-neutral">How long Approve All waits before anything runs.</span>
            </span>
            <select
              className={cn(fieldClass, "h-9 w-36 shrink-0")}
              value={s.undo_seconds}
              onChange={(e) => save.mutate({ ...s, undo_seconds: Number(e.target.value) })}
            >
              {UNDO_CHOICES.map((n) => (
                <option key={n} value={n}>
                  {n === 0 ? "Off" : `${n} second${n === 1 ? "" : "s"}`}
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center justify-between gap-4 p-4">
            <span>
              <span className="block">Require a proposal for every new filter</span>
              <span className="block text-sm text-neutral">
                A Gmail filter that wasn’t approved before needs a card first.
              </span>
            </span>
            <Switch
              checked={s.require_filter_proposal}
              onCheckedChange={(on) => save.mutate({ ...s, require_filter_proposal: on })}
            />
          </label>
        </div>
      )}
      {(settings.error ?? save.error) && (
        <p role="alert" className="mt-2 text-sm text-danger">
          {(settings.error ?? save.error)?.message}
        </p>
      )}
    </Section>
  );
}

function Passkeys() {
  const qc = useQueryClient();
  const passkeys = useQuery({ queryKey: ["passkeys"], queryFn: () => api<PasskeyInfo[]>("/api/passkeys") });
  const refresh = () => void qc.invalidateQueries({ queryKey: ["passkeys"] });
  const add = useMutation({
    mutationFn: async () => {
      const c = await api<Ceremony>("/api/auth/register/start", {
        method: "POST",
        body: { setup_code: null, device_name: guessDeviceName() },
      });
      const credential = await createPasskey(c.options);
      return api<AuthState>("/api/auth/register/finish", { method: "POST", body: { ceremony: c.ceremony, credential } });
    },
    onSuccess: refresh,
  });
  return (
    <Section title="Passkeys">
      {passkeys.data?.length === 0 && <p className="text-neutral">No passkey yet.</p>}
      <ul className="flex flex-col divide-y divide-rule rounded-lg border border-rule bg-paper-2 empty:hidden">
        {passkeys.data?.map((p) => (
          <PasskeyRow key={p.id} passkey={p} onlyOne={passkeys.data.length === 1} onChange={refresh} />
        ))}
      </ul>
      <Button className="mt-3" loading={add.isPending} onClick={() => add.mutate()}>
        <KeyRound aria-hidden /> Add a passkey on this device
      </Button>
      {(passkeys.error ?? add.error) && (
        <p role="alert" className="mt-2 text-sm text-danger">
          {(passkeys.error ?? add.error)?.message}
        </p>
      )}
    </Section>
  );
}

function PasskeyRow({ passkey, onlyOne, onChange }: { passkey: PasskeyInfo; onlyOne: boolean; onChange: () => void }) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(passkey.device_name);
  const rename = useMutation({
    mutationFn: () =>
      api(`/api/passkeys/${passkey.id}`, { method: "PATCH", body: { device_name: name.trim() } }),
    onSuccess: () => {
      setEditing(false);
      onChange();
    },
  });
  const remove = useMutation({
    mutationFn: () => api(`/api/passkeys/${passkey.id}`, { method: "DELETE" }),
    onSuccess: onChange,
  });
  return (
    <li className="flex flex-wrap items-center gap-3 p-4">
      {editing ? (
        <form
          className="flex min-w-0 flex-1 gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            rename.mutate();
          }}
        >
          <Input value={name} onChange={(e) => setName(e.target.value)} maxLength={60} autoFocus required />
          <Button type="submit" size="md" loading={rename.isPending} aria-label="Save the name">
            <Check aria-hidden />
          </Button>
        </form>
      ) : (
        <div className="min-w-0 flex-1">
          <p className="truncate">
            {passkey.device_name}
            {passkey.this_device && <span className="ml-2 text-sm text-accent">this device</span>}
          </p>
          <p className="text-sm text-neutral">
            Added {when(passkey.created_at)}
            {passkey.last_used_at && <> · last used {when(passkey.last_used_at)}</>}
          </p>
        </div>
      )}
      {!editing && (
        <div className="flex gap-1">
          <Button variant="quiet" size="icon" aria-label={`Rename ${passkey.device_name}`} onClick={() => setEditing(true)}>
            <Pencil />
          </Button>
          <Button
            variant="quiet"
            size="icon"
            aria-label={`Remove ${passkey.device_name}`}
            disabled={onlyOne}
            title={onlyOne ? "Add another passkey before removing this one" : undefined}
            loading={remove.isPending}
            onClick={() => remove.mutate()}
          >
            {!remove.isPending && <Trash2 />}
          </Button>
        </div>
      )}
      {(rename.error ?? remove.error) && (
        <p role="alert" className="w-full text-sm text-danger">
          {(rename.error ?? remove.error)?.message}
        </p>
      )}
    </li>
  );
}

function Nodes() {
  const qc = useQueryClient();
  const nodes = useQuery({ queryKey: ["nodes"], queryFn: () => api<NodeRow[]>("/api/nodes") });
  const [code, setCode] = useState<EnrolmentCode | null>(null);
  const [copied, setCopied] = useState(false);
  const newCode = useMutation({
    mutationFn: () => api<EnrolmentCode>("/api/nodes/enrolment-codes", { method: "POST" }),
    onSuccess: (c) => {
      setCode(c);
      setCopied(false);
    },
  });
  const revoke = useMutation({
    mutationFn: (name: string) => api(`/api/nodes/${encodeURIComponent(name)}`, { method: "DELETE" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["nodes"] }),
  });
  const command = code ? `quietdeck node enroll ${code.code} --deck ${window.location.origin}` : "";
  return (
    <Section title="Machines">
      {nodes.data?.length === 0 && <p className="mb-3 text-neutral">No machine is enrolled yet.</p>}
      {nodes.data && nodes.data.length > 0 && (
        <ul className="flex flex-col divide-y divide-rule rounded-lg border border-rule bg-paper-2">
          {nodes.data.map((n) => (
            <li key={n.name} className="flex flex-wrap items-center gap-3 p-4">
              <span
                className={cn("size-2 shrink-0 rounded-full", n.online ? "bg-accent" : "bg-rule-strong")}
                aria-hidden
              />
              <div className="min-w-0 flex-1">
                <p className="truncate">{n.name}</p>
                <p className="text-sm text-neutral">
                  {n.online ? "Connected" : n.last_seen_at ? `Last seen ${when(n.last_seen_at)}` : "Never connected"}
                  {n.version && <> · {n.version}</>}
                  {n.runner && <> · runner: {n.runner_types.join(", ") || "no action types"}</>}
                </p>
              </div>
              <Button variant="danger" size="sm" loading={revoke.isPending && revoke.variables === n.name} onClick={() => revoke.mutate(n.name)}>
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      )}
      <Button className="mt-3" loading={newCode.isPending} onClick={() => newCode.mutate()}>
        Enrol a machine
      </Button>
      {code && (
        <div className="mt-3 rounded-lg border border-rule bg-paper-2 p-4">
          <p className="text-sm text-neutral">
            On the machine, within 15 minutes (the code works once):
          </p>
          <div className="mt-2 flex items-center gap-2">
            <code className="min-w-0 flex-1 overflow-x-auto rounded-md bg-paper px-3 py-2 font-mono text-sm whitespace-nowrap text-ink">
              {command}
            </code>
            <Button
              variant="quiet"
              size="icon"
              aria-label="Copy the command"
              onClick={() => void navigator.clipboard.writeText(command).then(() => setCopied(true))}
            >
              {copied ? <Check /> : <Copy />}
            </Button>
          </div>
        </div>
      )}
      {(nodes.error ?? newCode.error ?? revoke.error) && (
        <p role="alert" className="mt-2 text-sm text-danger">
          {(nodes.error ?? newCode.error ?? revoke.error)?.message}
        </p>
      )}
    </Section>
  );
}

function SignOut() {
  const qc = useQueryClient();
  const out = useMutation({
    mutationFn: () => api("/api/auth/logout", { method: "POST" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["auth"] }),
  });
  return (
    <div>
      <Button variant="quiet" loading={out.isPending} onClick={() => out.mutate()}>
        Sign out on this device
      </Button>
    </div>
  );
}
