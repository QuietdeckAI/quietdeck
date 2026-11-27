import type { Action } from "@/types/Action";
import type { SessionRef } from "@/types/SessionRef";

export const sessionName = (s: SessionRef) => `${s.tab}@${s.node}`;

/** One plain line per proposed action: what will happen if he approves. */
export function actionLine(a: Action): { verb: string; detail?: string } {
  switch (a.type) {
    case "archive":
      return { verb: "Archive" };
    case "mark_read":
      return { verb: "Mark read" };
    case "not_spam":
      return { verb: "Mark as not spam" };
    case "label": {
      const parts = [...a.add.map((l) => `+${l}`), ...a.remove.map((l) => `−${l}`)];
      return { verb: "Label", detail: parts.join("  ") };
    }
    case "forward":
      return { verb: `Forward to ${sessionName(a.to)}`, detail: a.text };
    case "draft":
      return { verb: `Draft a reply to ${a.to.join(", ")}`, detail: a.body };
    case "reply":
      return { verb: `Send a reply to ${a.to.join(", ")}`, detail: a.body };
    case "story":
      return a.op === "create"
        ? { verb: `New ${a.story_type} on ${a.board}`, detail: a.title }
        : { verb: `Comment on ${a.story_id}`, detail: a.text };
    case "task":
      return { verb: a.op === "create" ? "New task" : a.op === "done" ? "Finish task" : "Update task", detail: a.title ?? a.story_id ?? undefined };
    case "calendar":
      return { verb: `Add to ${a.calendar}`, detail: `${a.title} · ${new Date(a.start).toLocaleString()}` };
    case "resume":
      return { verb: `Resume ${a.tab}@${a.node}` };
    case "new_session":
      return { verb: `Start ${a.tab}@${a.node}`, detail: a.prompt };
    case "lease": {
      const query = typeof a.params.query === "string" ? a.params.query : null;
      if (a.kind === "gmail-filter" && query)
        return {
          verb: `Keep mail matching ${query} out of the inbox while ${sessionName(a.holder)} works`,
          detail: `${a.resource} · ${a.reason}`,
        };
      return { verb: `Lease ${a.kind} for ${sessionName(a.holder)}`, detail: `${a.resource} · ${a.reason}` };
    }
    case "watch":
      return { verb: a.on ? "Watch this thread" : "Stop watching this thread" };
    case "session":
      return a.op === "message"
        ? { verb: `Message ${sessionName(a.to)}`, detail: a.text ?? undefined }
        : a.op === "wrap_up"
          ? { verb: `Wrap up ${sessionName(a.to)}`, detail: a.text ?? undefined }
          : { verb: `Close ${sessionName(a.to)}` };
  }
}
