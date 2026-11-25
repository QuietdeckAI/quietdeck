import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, KeyRound, Pencil, Trash2 } from "lucide-react";
import { useState } from "react";
import { useNow } from "@/components/ReportMeta";
import { Button } from "@/components/ui/button";
import { fieldClass, Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { appleMobile, currentSubscription, installed, pushPossible, subscribe } from "@/lib/push";
import { agoShort, stampWithZone, when } from "@/lib/format";
import { cn } from "@/lib/utils";
import { createPasskey, guessDeviceName } from "@/lib/webauthn";
import type { AuthState } from "@/types/AuthState";
import type { Ceremony } from "@/types/Ceremony";
import type { DeckInfo } from "@/types/DeckInfo";
import type { EnrolmentCode } from "@/types/EnrolmentCode";
import type { Lease } from "@/types/Lease";
import type { LeasesView } from "@/types/LeasesView";
import type { LeaseState } from "@/types/LeaseState";
import type { NodeRow } from "@/types/NodeRow";
import type { PasskeyInfo } from "@/types/PasskeyInfo";
import type { PushConfig } from "@/types/PushConfig";
import type { PushDevice } from "@/types/PushDevice";
import type { Settings } from "@/types/Settings";

export function SettingsScreen() {
  return (
    <main className="mx-auto flex max-w-3xl flex-col gap-12 px-4 py-8">
      <DeckSettings />
      <Notifications />
      <Passkeys />
      <Nodes />
      <Leases />
      <SignOut />
      <Release />
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
          <label className="flex items-center justify-between gap-4 p-4">
            <span>
              <span className="block">Show the pending count on the tab icon</span>
              <span className="block text-sm text-neutral">
                How many cards are in your Inbox, on the deck’s tab icon and in its title, and on the installed
                app’s icon.
              </span>
            </span>
            <Switch
              checked={s.show_pending_count}
              onCheckedChange={(on) => save.mutate({ ...s, show_pending_count: on })}
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

/** Pings reach the phone by web push. Nothing else is pushed. */
function Notifications() {
  const qc = useQueryClient();
  const config = useQuery({ queryKey: ["push", "config"], queryFn: () => api<PushConfig>("/api/push") });
  const devices = useQuery({ queryKey: ["push", "devices"], queryFn: () => api<PushDevice[]>("/api/push/devices") });
  const mine = useQuery({
    queryKey: ["push", "this-device"],
    queryFn: async () => (await currentSubscription())?.endpoint ?? null,
  });
  const refresh = () => void qc.invalidateQueries({ queryKey: ["push"] });
  const turnOn = useMutation({
    mutationFn: async (publicKey: string) =>
      api<PushDevice[]>("/api/push/devices", { method: "POST", body: await subscribe(publicKey) }),
    onSuccess: refresh,
  });
  const remove = useMutation({
    mutationFn: async (endpoint: string) => {
      if (endpoint === mine.data) await (await currentSubscription())?.unsubscribe();
      return api<PushDevice[]>("/api/push/devices/remove", { method: "POST", body: { endpoint } });
    },
    onSuccess: refresh,
  });
  const key = config.data?.public_key;
  const thisOne = devices.data?.some((d) => d.endpoint === mine.data) ?? false;
  const error = config.error ?? devices.error ?? turnOn.error ?? remove.error;
  return (
    <Section title="Notifications">
      <p className="mb-3 max-w-[65ch] text-sm text-neutral">
        Pings only: an agent pings when something can’t wait a few hours, such as a reply on a thread you watch.
      </p>
      {config.isSuccess && !key && (
        <p className="text-muted">
          Web push is off on this deck: it needs VAPID keys. The getting started guide says how to make them.
        </p>
      )}
      {key && !pushPossible() && (
        <p className="text-muted">
          {appleMobile() && !installed()
            ? "On an iPhone or iPad, add Quietdeck to your Home Screen (Share, then Add to Home Screen) and open it from there to turn notifications on."
            : "This browser can’t take push notifications."}
        </p>
      )}
      {key && pushPossible() && mine.isSuccess && !thisOne && (
        <Button loading={turnOn.isPending} onClick={() => turnOn.mutate(key)}>
          Turn on notifications on this device
        </Button>
      )}
      {(devices.data?.length ?? 0) > 0 && (
        <ul className="mt-3 flex flex-col divide-y divide-rule rounded-lg border border-rule bg-paper-2">
          {devices.data?.map((d) => (
            <li key={d.endpoint} className="flex items-center justify-between gap-4 p-4">
              <span className="min-w-0">
                <span className="block">
                  {d.device ?? "A device"}
                  {d.endpoint === mine.data && <span className="text-neutral"> (this device)</span>}
                </span>
                <span className="block text-sm text-neutral">
                  {d.last_ok_at ? `Last reached ${when(d.last_ok_at)}` : `Added ${when(d.created_at)}, not reached yet`}
                </span>
              </span>
              <Button
                variant="quiet"
                size="sm"
                loading={remove.isPending && remove.variables === d.endpoint}
                onClick={() => remove.mutate(d.endpoint)}
              >
                Turn off
              </Button>
            </li>
          ))}
        </ul>
      )}
      {error && (
        <p role="alert" className="mt-2 text-sm text-danger">
          {error.message}
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
                {n.runner && n.no_handler.length > 0 && (
                  <p className="text-sm text-neutral">
                    Handed to the agent for lack of a handler, most often first: {n.no_handler.join(", ")}
                  </p>
                )}
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

const LEASE_STATE: Record<LeaseState, string> = {
  proposed: "Waiting for your approval",
  queued: "Queued",
  approved: "Approved, being set up",
  active: "Active",
  expiring: "Ending",
  cleanup_requested: "Ended, filter being removed",
  overdue: "Filter removal overdue",
  released: "Ended",
  rejected: "Not approved",
};

function leaseWhat(l: Lease): string {
  const query = typeof l.params.query === "string" ? l.params.query : null;
  return query ? `Filter on ${l.resource}: ${query}` : `Heavy build on ${l.resource}`;
}

function Leases() {
  const qc = useQueryClient();
  const view = useQuery({ queryKey: ["leases"], queryFn: () => api<LeasesView>("/api/leases") });
  const end = useMutation({
    mutationFn: (id: number) => api<Lease>(`/api/leases/${id}/release`, { method: "POST", body: {} }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["leases"] }),
  });
  const cleaned = useMutation({
    mutationFn: (id: number) => api<Lease>(`/api/leases/${id}/cleaned`, { method: "POST", body: {} }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["leases"] }),
  });
  const revoke = useMutation({
    mutationFn: (id: number) => api<LeasesView>(`/api/lease-patterns/${id}/revoke`, { method: "POST", body: {} }),
    onSuccess: (v) => qc.setQueryData(["leases"], v),
  });
  const leases = view.data?.leases ?? [];
  const current = leases.filter((l) => !l.ended_at);
  const ended = leases.filter((l) => l.ended_at);
  const approved = (view.data?.patterns ?? []).filter((p) => !p.revoked_at);
  const error = view.error ?? end.error ?? cleaned.error ?? revoke.error;
  return (
    <Section title="Leases">
      <p className="mb-3 text-sm text-neutral">
        Temporary Gmail filters your sessions hold while they work, and heavy-build slots on each machine. Each ends
        when its session releases it, goes idle or closes.
      </p>
      {view.data && current.length === 0 && <p className="mb-3 text-neutral">Nothing is held right now.</p>}
      {current.length > 0 && (
        <ul className="flex flex-col divide-y divide-rule rounded-lg border border-rule bg-paper-2">
          {current.map((l) => (
            <li key={l.id} className="flex flex-wrap items-center gap-3 p-4">
              <div className="min-w-0 flex-1">
                <p className="break-words [overflow-wrap:anywhere]">{leaseWhat(l)}</p>
                <p className="text-sm text-neutral">
                  {LEASE_STATE[l.state]}
                  {l.queue_position != null && <> · position {l.queue_position}</>} · {l.holder.tab}@{l.holder.node}
                  {l.expires_at && l.state === "active" && <> · until {when(l.expires_at)}</>}
                </p>
                {l.reason && <p className="text-sm text-neutral">{l.reason}</p>}
                {l.state === "overdue" && (
                  <p className="text-sm text-neutral">
                    Nobody confirmed removing it. Delete the filter in Gmail, then mark it removed.
                  </p>
                )}
              </div>
              {(l.state === "overdue" || l.state === "cleanup_requested") ? (
                <Button size="sm" loading={cleaned.isPending && cleaned.variables === l.id} onClick={() => cleaned.mutate(l.id)}>
                  Mark removed
                </Button>
              ) : (
                <Button variant="quiet" size="sm" loading={end.isPending && end.variables === l.id} onClick={() => end.mutate(l.id)}>
                  End
                </Button>
              )}
            </li>
          ))}
        </ul>
      )}
      {ended.length > 0 && (
        <>
          <h3 className="mt-6 mb-2 text-neutral">Ended today</h3>
          <ul className="flex flex-col divide-y divide-rule rounded-lg border border-rule">
            {ended.map((l) => (
              <li key={l.id} className="p-4">
                <p className="break-words text-neutral [overflow-wrap:anywhere]">{leaseWhat(l)}</p>
                <p className="text-sm text-neutral">
                  {LEASE_STATE[l.state]} {l.ended_at && when(l.ended_at)} · {l.holder.tab}@{l.holder.node}
                  {l.end_reason && <> · {l.end_reason}</>}
                </p>
              </li>
            ))}
          </ul>
        </>
      )}
      <h3 className="mt-6 mb-2">Approved filters</h3>
      <p className="mb-3 text-sm text-neutral">
        Any session may re-create one of these on the same account without asking again. Revoke one to need a new
        approval.
      </p>
      {view.data && approved.length === 0 && <p className="text-neutral">None yet.</p>}
      {approved.length > 0 && (
        <ul className="flex flex-col divide-y divide-rule rounded-lg border border-rule bg-paper-2">
          {approved.map((p) => (
            <li key={p.id} className="flex flex-wrap items-center gap-3 p-4">
              <div className="min-w-0 flex-1">
                <p className="break-words font-mono text-sm [overflow-wrap:anywhere]">{p.pattern}</p>
                <p className="text-sm text-neutral">
                  {p.resource}
                  {p.confirmed_at && <> · approved {when(p.confirmed_at)}</>}
                  {!p.confirmed_by_card_id && <> · made without a proposal</>}
                </p>
              </div>
              <Button variant="danger" size="sm" loading={revoke.isPending && revoke.variables === p.id} onClick={() => revoke.mutate(p.id)}>
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      )}
      {error && (
        <p role="alert" className="mt-2 text-sm text-danger">
          {error.message}
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
      <Button loading={out.isPending} onClick={() => out.mutate()}>
        Sign out on this device
      </Button>
    </div>
  );
}

/** The release running: its commit (linked to GitHub), when it went live, and when it was built. */
function Release() {
  const info = useQuery({ queryKey: ["info"], queryFn: () => api<DeckInfo>("/api/info") });
  const now = useNow(30_000);
  if (!info.data) return null;
  const { commit, built_at, started_at, version } = info.data;
  return (
    <Section title="Release">
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 rounded-lg border border-rule bg-paper-2 p-4 text-sm">
        <dt className="text-neutral">Commit</dt>
        <dd>
          {commit ? (
            <a
              href={`https://github.com/QuietdeckAI/quietdeck/commit/${commit}`}
              target="_blank"
              rel="noreferrer noopener"
              className="font-mono underline-offset-4 hover:underline"
            >
              {commit}
            </a>
          ) : (
            <span className="text-neutral">a local build</span>
          )}
          <span className="text-neutral"> · version {version}</span>
        </dd>
        <dt className="text-neutral">Deployed</dt>
        <dd>
          {stampWithZone(started_at)} <span className="text-neutral">({agoShort(started_at, now)})</span>
        </dd>
        {built_at && (
          <>
            <dt className="text-neutral">Built</dt>
            <dd>
              {stampWithZone(built_at)} <span className="text-neutral">({agoShort(built_at, now)})</span>
            </dd>
          </>
        )}
      </dl>
    </Section>
  );
}
