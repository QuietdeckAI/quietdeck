import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { api } from "@/lib/api";
import { enterSends } from "@/lib/utils";
import type { Comment } from "@/types/Comment";
import type { CommentTarget } from "@/types/CommentTarget";

/** A comment on a timeline entry or a History row. It goes to the session behind it, which proposes any change as a card. */
export function CommentBox({
  target,
  label,
}: {
  target: CommentTarget;
  label: string;
}) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const send = useMutation({
    mutationFn: () =>
      api<Comment>("/api/comments", {
        method: "POST",
        body: { target, text: text.trim() },
      }),
    onSuccess: () => {
      setText("");
      setOpen(false);
    },
  });

  if (!open) {
    return (
      <span className="inline-flex items-baseline gap-2">
        <button
          type="button"
          className="text-xs text-neutral underline-offset-4 hover:text-ink hover:underline"
          onClick={() => setOpen(true)}
          aria-label={`Comment on ${label}`}
        >
          Comment
        </button>
        {send.data && (
          <span className="text-xs text-neutral">
            {send.data.to
              ? send.data.picked_up_at
                ? `Sent to ${send.data.to.tab}.`
                : `${send.data.to.tab} gets it on its next round.`
              : "Saved; no session to send it to."}
          </span>
        )}
      </span>
    );
  }
  return (
    <form
      className="mt-2 flex w-full flex-col gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (text.trim()) send.mutate();
      }}
    >
      <Textarea
        rows={1}
        autoFocus
        maxLength={4000}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={enterSends(() => text.trim() && !send.isPending && send.mutate())}
        aria-label={`Comment on ${label}`}
      />
      {send.error && (
        <p className="text-sm text-danger">{send.error.message}</p>
      )}
      <div className="flex gap-2">
        <Button
          type="submit"
          size="sm"
          variant="primary"
          loading={send.isPending}
          disabled={!text.trim()}
        >
          Send
        </Button>
        <Button
          type="button"
          size="sm"
          variant="quiet"
          onClick={() => setOpen(false)}
        >
          Cancel
        </Button>
      </div>
    </form>
  );
}
