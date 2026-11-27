# Archive in Gmail and the report keeps up

Quietdeck isn't the only place you deal with mail. You archive a newsletter on your phone at lunch, delete a receipt in
Gmail's web page, or clear out a thread you've already handled. Those emails are on the report, with cards proposing
what to do with them.

If the report didn't notice, it would keep asking you about mail you've already dealt with, and Approve All would try to
archive threads that are gone. So when every email in a thread leaves your inbox outside Quietdeck, the deck closes its
card, and the agent takes it off the report. When nothing on the report waits for you any more, the Inbox says
**All done.**

It works in [Quietdeck solo](../solo.md) and the full deck, through gog or through Executor. [How Quietdeck
works](../concepts.md) explains the runner, the cards and the report.

## What you see

Archive or delete a thread in Gmail, anywhere. Within a few minutes:

- **Its card closes.** History records it as done, with the note "you archived it in Gmail", and any of its actions
  that hadn't run are stopped.
- **The report changes.** The agent removes the item and the change shows as a toast like any other: "Took the
  newsletter off: archived in Gmail."
- **Or the report is finished.** If that was the last thing waiting for you, the agent marks the report done, and the
  Inbox says **All done.** The next mail starts a new report.

Only cards that are still waiting for you are closed this way: open ones, and ones whose action failed. A card you've
approved, or one the runner or the agent is carrying out, is left alone. Their own archiving shows up in Gmail the same
way, and it mustn't be mistaken for yours.

## Every message of the thread has to go

A Gmail thread can hold many messages, and the Inbox label belongs to each message, not to the thread. Archiving one
message from a conversation leaves the rest in your inbox. So a hint that a thread may have left is only a hint. Before
the deck hears of it, the runner reads the thread and checks that **no** message in it still has the Inbox label, or
that the thread has been deleted.

If that check fails (Gmail is slow, gog's sign-in has expired), the thread is treated as still in the inbox. A card is
never closed on a guess.

Some cards cover more than one thread: one "Archive all" for several newsletters, say. The deck remembers each thread
that left for a month, and closes such a card only once **every** thread it covers has gone. If new mail arrives in a
thread that had left, the deck forgets that it left.

## How the runner notices

Gmail keeps a history of what changed in a mailbox. How the runner learns from it depends on which mail tools it uses.

### Through Executor

Gmail's history API can report removed labels and deleted messages directly. The runner's `history` tool asks for
`labelRemoved` and `messageDeleted` as well as `messageAdded`:

```yaml
historyTypes: [messageAdded, labelRemoved, messageDeleted]
```

Each history entry where the Inbox label came off a message, or a message was deleted, gives a thread to check. Leave
those two types out and the runner sees new mail only, and never notices you archiving anything. The full tool is in
`deploy/node.example.yaml`.

### Through gog

[gog](https://github.com/steipete/gogcli) is what solo uses, and what `runner.mail: gog` selects in the full deck. Its
`gmail history` command asks Gmail for new messages only, and prints bare message IDs: no labels, no archives, no
deletions. So the runner finds threads that left by listing the whole inbox and comparing listings:

```sh
gog gmail search --ids-only --all --max=500 -- in:inbox
```

That prints the ID of every thread in the inbox, 500 a page, across every page. The runner lists the inbox every 5
minutes (`mail_watch.inbox_every`) and compares the list with the last one. A thread in the last listing and missing
from this one is a thread to check.

The comparison is only ever between two **complete** listings. A listing that fails, or stops part way (one that still
has a next-page token), changes nothing: the runner keeps the last complete one and tries again next time. Comparing
against half a listing would report most of your inbox as archived.

New mail that arrives between listings joins the last listing straight away, so a message that comes in and is
archived before the next listing is still noticed.

The last listing is kept in memory only. After the runner restarts, its first listing is a fresh starting point, so a
thread you archived while it was stopped isn't noticed, and its card stays on the report until you deal with it in the
Inbox. The Executor route doesn't have this gap: the runner keeps its place in Gmail's history on disk, and picks up
where it left off.

**If you're building something like this:** a diff of two snapshots is safe only when both are complete. Treat any
error, timeout or leftover page token as "no new snapshot", never as an empty one.

## What the agent does

When the deck closes cards this way, the runner tells the agent which ones, with their subjects, and what to do:

```text
Emails left the inbox outside Quietdeck (archived or deleted in Gmail, by the person or by an agent working
directly), so the deck resolved their cards: c_12 'Weekly digest', c_15 'Your receipt'. Take them off the report
(`quietdeck report show --markdown`, then `report set`). If nothing on it waits for the person any more, mark it done
with `quietdeck report done` so the Inbox says "All done."
```

The agent edits the report like any other change: it reads the current text, removes the items, and publishes it with
`quietdeck report set --base <version> --change "…"`. `quietdeck report done` marks the report finished when nothing on
it waits for you. Both are among the few commands solo's agent is allowed to run.

The deck closes the cards in code, before the agent is told, so the report's defaults never point at an action that can
still run. If the agent isn't reachable at that moment, the cards are closed anyway, and the agent sees it on its next
round.

## Where this lives in Quietdeck

- Closing the cards whose threads have all left (`threads_left`):
  [crates/quietdeck-server/src/mail.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/mail.rs)
- Reading the history and comparing inbox listings (`check`, `inbox_left`):
  [crates/quietdeck-node/src/runner/mail.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/mail.rs)
- Checking every message of a thread, and telling the agent (`threads_moved`):
  [crates/quietdeck-node/src/runner/mod.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/mod.rs)
- The gog backend, including the inbox listing:
  [crates/quietdeck-node/src/runner/gog.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/gog.rs)
- The `history` tool's history types, and `inbox_every`:
  [deploy/node.example.yaml](https://github.com/QuietdeckAI/quietdeck/blob/main/deploy/node.example.yaml) and
  [crates/quietdeck-node/src/runner/config.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/config.rs)
- `quietdeck report done`:
  [crates/quietdeck-cli/src/main.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-cli/src/main.rs)
