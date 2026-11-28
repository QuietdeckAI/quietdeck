import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, ShieldCheck, ShieldQuestion } from "lucide-react";
import { useEffect } from "react";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { agoShort, clock } from "@/lib/format";
import { notify } from "@/lib/notes";
import { setParam } from "@/lib/route";
import type { FleetApproval } from "@/types/FleetApproval";
import type { FleetApprovals } from "@/types/FleetApprovals";
import type { FleetDecided } from "@/types/FleetDecided";
import type { FleetWindow } from "@/types/FleetWindow";

const KEY = ["fleetlayer", "approvals"];
/** The step-up page sends the browser back to the Work tab with this, to approve the window again. */
const RETURN_PARAM = "fleet_approve";

const iso = (ms: number) => new Date(ms).toISOString();

/** "in 42 min", "at 14:30". */
function closesIn(ms: number, now = Date.now()): string {
  const mins = Math.round((ms - now) / 60_000);
  if (mins <= 0) return "any moment";
  if (mins < 60) return `in ${mins} min`;
  return `at ${clock(iso(ms))}`;
}

function decide(id: string, approve: boolean) {
  return api<FleetDecided>(`/api/fleetlayer/approvals/${encodeURIComponent(id)}`, { method: "POST", body: { approve } });
}

/**
 * Fleetlayer's requests waiting on the person (a session asking to start, to use a shared tool, to open an admin
 * window) and its open admin windows, at the top of the Work tab. Approving is one tap; opening a window first goes to
 * the sign-in service's passkey page, which sends the browser back here with `?fleet_approve=<id>` to approve it again.
 * Shows nothing when the deck has no Fleetlayer approve token, or for anyone but the owner.
 */
export function FleetRequests() {
  const qc = useQueryClient();
  const fleet = useQuery({
    queryKey: KEY,
    queryFn: () => api<FleetApprovals>("/api/fleetlayer/approvals"),
    // No event for these yet: poll while the Work tab is open.
    refetchInterval: 30_000,
    retry: false,
  });
  useStepUpReturn();
  const data = fleet.data;
  if (!data?.configured) return null;
  if (data.error) {
    return (
      <p role="alert" className="mb-6 text-sm text-danger">
        Fleetlayer's requests couldn't be read: {data.error}
      </p>
    );
  }
  if (data.approvals.length === 0 && data.windows.length === 0) return null;
  const refresh = () => void qc.invalidateQueries({ queryKey: KEY });
  return (
    <section aria-label="Fleetlayer" className="mb-8">
      <h1 className="font-display text-[1.75rem] leading-tight text-ink">Fleetlayer</h1>
      <div className="mt-3 flex flex-col gap-3">
        {data.windows.map((w) => (
          <WindowCard key={w.id} window={w} onDone={refresh} />
        ))}
        {data.approvals.map((a) => (
          <RequestCard key={a.id} request={a} onDone={refresh} />
        ))}
      </div>
    </section>
  );
}

/** Approved once per page load: the Work tab remounts this while its report loads. */
let returned = false;

/** Back from the passkey page: approve that window again (the deck accepts it for five minutes), then drop the param. */
function useStepUpReturn() {
  const qc = useQueryClient();
  useEffect(() => {
    const id = new URLSearchParams(window.location.search).get(RETURN_PARAM);
    if (!id || returned) return;
    returned = true;
    setParam(RETURN_PARAM, undefined);
    decide(id, true)
      .then((r) => {
        if (r.decided) notify("Window opened", { ms: 4000 });
        else notify("The passkey wasn't confirmed, so the window isn't open. Approve it again to retry.", { error: true, ms: 30_000 });
      })
      .catch((e: Error) => notify(`Window not opened: ${e.message}`, { error: true, ms: 30_000 }))
      // "all": the card that asked may have been remounted meanwhile, and a fetch started before the approval must not win.
      .finally(() => void qc.invalidateQueries({ queryKey: KEY, refetchType: "all" }));
  }, [qc]);
}

function RequestCard({ request: a, onDone }: { request: FleetApproval; onDone: () => void }) {
  const send = useMutation({
    mutationFn: (approve: boolean) => decide(a.id, approve),
    onSuccess: (r, approve) => {
      if (r.stepup_url) {
        window.location.assign(r.stepup_url);
        return;
      }
      notify(approve ? `Approved: ${a.from} ${a.verb} ${a.to}` : `Refused: ${a.from} ${a.verb} ${a.to}`, { ms: 4000 });
      onDone();
    },
    onError: (e) => notify(`Not decided: ${e.message}`, { error: true, ms: 30_000 }),
  });
  const Icon = a.window ? KeyRound : ShieldQuestion;
  const meta = [
    agoShort(iso(a.created_at)),
    a.once ? "one use" : null,
    a.expires_at ? `lapses ${closesIn(a.expires_at)}` : null,
  ].filter(Boolean);
  return (
    <div className="rounded-md border border-busy/50 bg-paper-2 py-2 pr-1.5 pl-3">
      <div className="flex items-start gap-2">
        <Icon aria-hidden className="mt-[3px] size-4 shrink-0 text-busy" />
        <div className="min-w-0 flex-1">
          <p className="text-ink">
            <span className="font-mono text-sm">{a.from}</span> <span className="text-sm text-neutral">asks to</span>{" "}
            <span className="font-mono text-sm">{a.verb}</span> <span className="text-sm text-neutral">on</span>{" "}
            <span className="font-mono text-sm [overflow-wrap:anywhere]">{a.to}</span>
          </p>
          <p className="text-xs text-neutral">{meta.join(" · ")}</p>
        </div>
      </div>
      {a.detail && (
        <pre className="mt-2 mr-1.5 max-h-40 overflow-auto rounded bg-paper p-2 text-xs whitespace-pre-wrap text-muted">{a.detail}</pre>
      )}
      <div className="mt-2.5 mr-1.5 flex flex-wrap items-center justify-end gap-2">
        {a.window && <span className="mr-auto text-xs text-neutral">Opening a window asks for your passkey.</span>}
        <Button
          size="sm"
          variant="secondary"
          loading={send.isPending && send.variables === false}
          disabled={send.isPending}
          onClick={() => send.mutate(false)}
        >
          Refuse
        </Button>
        <Button
          size="sm"
          loading={send.isPending && send.variables === true}
          disabled={send.isPending}
          onClick={() => send.mutate(true)}
        >
          {a.window ? "Open window" : "Approve"}
        </Button>
      </div>
    </div>
  );
}

function WindowCard({ window: w, onDone }: { window: FleetWindow; onDone: () => void }) {
  const close = useMutation({
    mutationFn: () => api(`/api/fleetlayer/windows/${encodeURIComponent(w.role)}/close`, { method: "POST" }),
    onSuccess: () => {
      notify(`Closed the ${w.role} window`, { ms: 4000 });
      onDone();
    },
    onError: (e) => notify(`Not closed: ${e.message}`, { error: true, ms: 30_000 }),
  });
  const meta = [
    w.requested_by ? `for ${w.requested_by}` : null,
    w.opened_at ? `opened ${agoShort(iso(w.opened_at))}` : null,
    w.ends_at ? `closes ${closesIn(w.ends_at)}` : null,
  ].filter(Boolean);
  return (
    <div className="rounded-md border border-rule-strong bg-paper-2 py-2 pr-1.5 pl-3">
      <div className="flex items-start gap-2">
        <ShieldCheck aria-hidden className="mt-[3px] size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <p className="text-ink">
            <span className="text-sm text-neutral">The</span> <span className="font-mono text-sm">{w.role}</span>{" "}
            <span className="text-sm text-neutral">window is open</span>
          </p>
          {meta.length > 0 && <p className="text-xs text-neutral">{meta.join(" · ")}</p>}
          {w.problem && <p className="mt-1 text-xs text-danger">{w.problem}</p>}
        </div>
        <Button size="sm" variant="danger" loading={close.isPending} onClick={() => close.mutate()}>
          Close
        </Button>
      </div>
    </div>
  );
}
