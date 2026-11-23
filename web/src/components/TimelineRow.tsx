import { CommentBox } from "@/components/CommentBox";
import { when } from "@/lib/format";
import type { TimelineEntry } from "@/types/TimelineEntry";

const SOURCE: Record<TimelineEntry["source"], string> = {
  runner: "runner",
  agent: "triage",
  deck: "deck",
};

export function TimelineRow({ entry }: { entry: TimelineEntry }) {
  return (
    <li className="arrive flex flex-wrap items-baseline gap-x-3 py-0.5 text-sm leading-6">
      <time
        className="shrink-0 whitespace-nowrap font-mono text-xs text-neutral"
        dateTime={entry.created_at}
      >
        {when(entry.created_at)}
      </time>
      <span className="min-w-0 flex-1 truncate text-muted" title={entry.text}>
        {entry.text}
        {entry.count > 1 && (
          <span className="text-neutral"> ×{entry.count}</span>
        )}
        <span className="text-neutral"> · {SOURCE[entry.source]}</span>
      </span>
      <CommentBox
        target={{ type: "timeline", id: entry.id }}
        label={entry.text}
      />
    </li>
  );
}
