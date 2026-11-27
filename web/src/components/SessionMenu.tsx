import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BellOff, Clock, EyeOff, PauseCircle, ScrollText, SquareTerminal } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { api } from "@/lib/api";
import { splitAddress, useSessionMarks } from "@/lib/documents";
import { notify } from "@/lib/notes";
import { navigate } from "@/lib/route";
import { sessionPath } from "@/screens/Sessions";
import type { MarkKind } from "@/types/MarkKind";
import type { SessionMark } from "@/types/SessionMark";
import type { SessionRow } from "@/types/SessionRow";

/** 9am on the next day, or on the next weekday given (1 = Monday), in the browser's time zone. */
function nineAm(daysAhead: number | "monday"): Date {
  const d = new Date();
  if (daysAhead === "monday") {
    const add = ((8 - d.getDay()) % 7) || 7;
    d.setDate(d.getDate() + add);
  } else d.setDate(d.getDate() + daysAhead);
  d.setHours(9, 0, 0, 0);
  return d;
}

const dayFormat = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" });

/** Park, snooze, unpark, ignore: the person's marks on a session. */
export function useMarkSession() {
  const qc = useQueryClient();
  const set = useMutation({
    mutationFn: (m: { node: string; tab: string; mark: MarkKind; until?: Date | null; note?: string | null }) =>
      api<SessionMark[]>("/api/session-marks", {
        method: "POST",
        body: { node: m.node, tab: m.tab, mark: m.mark, until: m.until?.toISOString() ?? null, note: m.note ?? null },
      }),
    onSuccess: (marks) => qc.setQueryData(["session-marks"], marks),
    onError: (e) => notify(`Couldn't do that: ${e.message}`, { error: true, ms: 30_000 }),
  });
  const remove = useMutation({
    mutationFn: (m: { node: string; tab: string; mark: MarkKind }) =>
      api<SessionMark[]>("/api/session-marks/remove", { method: "POST", body: m }),
    onSuccess: (marks) => qc.setQueryData(["session-marks"], marks),
    onError: (e) => notify(`Couldn't do that: ${e.message}`, { error: true, ms: 30_000 }),
  });
  return { set, remove };
}

/** The session behind a `tab@node`: the running one with that tab on that node, else the latest seen. */
export function useSessionAt(address: string): SessionRow | undefined {
  const all = useQuery({ queryKey: ["sessions", "running"], queryFn: () => api<SessionRow[]>("/api/sessions") });
  const at = splitAddress(address);
  if (!at) return undefined;
  const recent = (s: SessionRow) => s.last_activity ?? s.status_changed_at;
  return (all.data ?? [])
    .filter((s) => s.node === at.node && s.tab === at.tab)
    .sort((a, b) => Number(b.running) - Number(a.running) || recent(b).localeCompare(recent(a)))[0];
}

/**
 * The terminal icon on a Work entry's panel, where an Inbox entry has the envelope: the session it's about by name
 * (tab@node), Fleetlayer's summary of it, Open transcript, and the person's marks: park it, snooze it, ignore it.
 */
export function SessionMenu({ session: address, about }: { session: string; about: string }) {
  const at = splitAddress(address);
  const session = useSessionAt(address);
  const marks = useSessionMarks().data ?? [];
  const { set, remove } = useMarkSession();
  if (!at) return null;
  const mine = marks.filter((m) => m.node === at.node && m.tab === at.tab);
  const parked = mine.find((m) => m.mark === "parked");
  const ignored = mine.some((m) => m.mark === "ignored");
  const park = (until: Date | null) => set.mutate({ ...at, mark: "parked", until, note: about });
  const summary = session?.paragraph ?? session?.sentence;
  return (
    // Not modal: a modal menu locks the page's scrolling, which hides the sticky chat bar while it's open.
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={`The session ${address}: ${about}`}
          title={address}
          className={
            "inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted " +
            "transition-[background-color,color,transform] duration-(--dur-quick) ease-(--ease-out) " +
            "hover:bg-paper-3 hover:text-ink active:translate-y-px " +
            "data-[state=open]:bg-paper-3 data-[state=open]:text-ink [&_svg]:size-4"
          }
        >
          <SquareTerminal />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="bottom" className="max-h-[70vh] w-80 max-w-[calc(100vw-2rem)] overflow-y-auto">
        <div className="px-2 pt-1 pb-1.5">
          <p className="font-mono text-sm text-ink [overflow-wrap:anywhere]">{address}</p>
          <p className="mt-0.5 text-xs text-neutral">
            {session
              ? `${session.running ? session.status : "not running"}${session.project ? ` · ${session.project}` : ""}`
              : "Not seen by the deck"}
            {parked && ` · parked${parked.until ? ` until ${dayFormat.format(new Date(parked.until))}` : ""}`}
            {ignored && " · ignored"}
          </p>
          {summary && (
            <p className="mt-2 line-clamp-10 border-t border-rule pt-2 text-xs whitespace-pre-line text-muted">{summary}</p>
          )}
        </div>
        <DropdownMenuItem disabled={!session} onSelect={() => session && navigate(sessionPath(session.node, session.session_id))}>
          <ScrollText /> Open transcript
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {parked ? (
          <DropdownMenuItem onSelect={() => remove.mutate({ ...at, mark: "parked" })}>
            <PauseCircle /> Unpark
          </DropdownMenuItem>
        ) : (
          <>
            <DropdownMenuItem onSelect={() => park(null)}>
              <PauseCircle /> Park
            </DropdownMenuItem>
            <DropdownMenuLabel className="pt-2 text-xs font-normal text-neutral">Snooze until</DropdownMenuLabel>
            <DropdownMenuItem onSelect={() => park(nineAm(1))}>
              <Clock /> Tomorrow, 9am
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => park(nineAm("monday"))}>
              <Clock /> {dayFormat.format(nineAm("monday"))}, 9am
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => park(nineAm(7))}>
              <Clock /> In a week
            </DropdownMenuItem>
          </>
        )}
        <DropdownMenuSeparator />
        {ignored ? (
          <DropdownMenuItem onSelect={() => remove.mutate({ ...at, mark: "ignored" })}>
            <BellOff /> Stop ignoring this session
          </DropdownMenuItem>
        ) : (
          <DropdownMenuItem onSelect={() => set.mutate({ ...at, mark: "ignored", note: about })}>
            <EyeOff /> Ignore this session
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
