import { useSyncExternalStore } from "react";

/**
 * Short notices as toasts (never lines in the page): "Sending to quietdeck triage…", "Sent to quietdeck triage.", an
 * error. One store for the page; ReportToasts shows them in the bottom-right stack with the Inbox's change toasts.
 */
export type Note = { id: number; text: string; error: boolean; until: number | null };

let notes: Note[] = [];
let nextId = 1;
const listeners = new Set<() => void>();
const timers = new Map<number, number>();

function emit() {
  for (const l of listeners) l();
}

function schedule(n: Note) {
  const old = timers.get(n.id);
  if (old) window.clearTimeout(old);
  timers.delete(n.id);
  if (n.until !== null) timers.set(n.id, window.setTimeout(() => dismissNote(n.id), Math.max(0, n.until - Date.now())));
}

/** Shows a toast; `ms` null keeps it until it's replaced or dismissed. Returns its ID. */
export function notify(text: string, opts: { error?: boolean; ms?: number | null } = {}): number {
  const ms = opts.ms === undefined ? 4000 : opts.ms;
  const n: Note = { id: nextId++, text, error: opts.error ?? false, until: ms === null ? null : Date.now() + ms };
  notes = [...notes, n].slice(-4);
  schedule(n);
  emit();
  return n.id;
}

/** Changes a toast in place (a "Sending…" that becomes "Sent."), or shows a new one if it's gone. */
export function replaceNote(id: number, text: string, opts: { error?: boolean; ms?: number | null } = {}): number {
  if (!notes.some((n) => n.id === id)) return notify(text, opts);
  const ms = opts.ms === undefined ? 4000 : opts.ms;
  notes = notes.map((n) =>
    n.id === id ? { ...n, text, error: opts.error ?? false, until: ms === null ? null : Date.now() + ms } : n,
  );
  const n = notes.find((x) => x.id === id);
  if (n) schedule(n);
  emit();
  return id;
}

export function dismissNote(id: number) {
  const t = timers.get(id);
  if (t) window.clearTimeout(t);
  timers.delete(id);
  if (!notes.some((n) => n.id === id)) return;
  notes = notes.filter((n) => n.id !== id);
  emit();
}

export function useNotes(): Note[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => notes,
  );
}
