import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { Document } from "@/types/Document";
import type { SessionMark } from "@/types/SessionMark";

/** The documents agents write, in order. The Inbox is always there. */
export function useDocuments() {
  return useQuery({ queryKey: ["documents"], queryFn: () => api<Document[]>("/api/documents"), staleTime: 60_000 });
}

/** The person's marks on sessions (ignored, parked). */
export function useSessionMarks() {
  return useQuery({ queryKey: ["session-marks"], queryFn: () => api<SessionMark[]>("/api/session-marks") });
}

export type TopTab = { name: string; path: string; docs: Document[] };

/** Paths the app's own screens use: a tab named like one gets `/tab/<name>` instead. */
const TAKEN = new Set(["drafts", "sent", "threads", "cards", "later", "sessions", "history", "rules", "settings", "tab", "api"]);

function pathFor(name: string): string {
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "tab";
  return TAKEN.has(slug) ? `/tab/${slug}` : `/${slug}`;
}

/**
 * The top-level tabs: one per distinct `tab`, in the order of their first document. The one holding the Inbox document
 * (id 1) is at `/`.
 */
export function topTabs(docs: Document[]): TopTab[] {
  const tabs: TopTab[] = [];
  for (const d of docs) {
    let t = tabs.find((x) => x.name === d.tab);
    if (!t) {
      t = { name: d.tab, path: "", docs: [] };
      tabs.push(t);
    }
    t.docs.push(d);
  }
  for (const t of tabs) t.path = t.docs.some((d) => Number(d.id) === 1) ? "/" : pathFor(t.name);
  return tabs;
}

/** The top-level tab a path shows, other than the Inbox's own `/`. */
export function tabAt(docs: Document[] | undefined, path: string): TopTab | undefined {
  if (!docs || path === "/") return undefined;
  const clean = path.replace(/\/+$/, "");
  return topTabs(docs).find((t) => t.path !== "/" && t.path === clean);
}

/** `tab@node` for a session mark or an entry's `session` attribute. */
export const sessionAddress = (node: string, tab: string) => `${tab}@${node}`;

/** `tab@node` → [node, tab]; the node is after the last @. */
export function splitAddress(address: string): { node: string; tab: string } | null {
  const at = address.lastIndexOf("@");
  if (at <= 0 || at === address.length - 1) return null;
  return { tab: address.slice(0, at), node: address.slice(at + 1) };
}
