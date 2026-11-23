import { when } from "@/lib/format";
import type { TimelineEntry } from "@/types/TimelineEntry";

const SOURCE: Record<TimelineEntry["source"], string> = { runner: "runner", agent: "triage", deck: "deck" };

export function TimelineRow({ entry }: { entry: TimelineEntry }) {
  return (
    <li className="arrive flex items-baseline gap-3 py-1.5 text-sm">
      <time className="w-24 shrink-0 font-mono text-xs text-neutral" dateTime={entry.created_at}>
        {when(entry.created_at)}
      </time>
      <span className="min-w-0 flex-1 text-muted">
        {entry.text}
        {entry.count > 1 && <span className="text-neutral"> ×{entry.count}</span>}
        <span className="text-neutral"> · {SOURCE[entry.source]}</span>
      </span>
    </li>
  );
}
