# While you read, nothing new lands

You open the Inbox, read the report from the top, and reach the bottom satisfied that every default is right. You press
**Approve All**. If a new email had arrived while you read, and the agent had added it to the report a few seconds
before your click, you'd have approved something you never saw.

That's how a wrong approval happens: the page changes under the person between reading and clicking. So while you're
reading the Inbox, Quietdeck holds new mail back instead of waking the agent, tells you what's waiting, and lets you
decide when it comes in. What does get through is shown to you as it lands, and Approve All refuses anything that
changed too recently to have been read.

It works the same in [Quietdeck solo](../solo.md) and the full deck. [How Quietdeck works](../concepts.md) explains the
cards, the report and Approve All.

## How the deck knows you're reading

The Inbox page sends the deck a small heartbeat while you're doing something on it:

- **Activity** is a mouse move, a click or tap, a key, the scroll wheel, a scroll, or a touch. The page sends at most
  one beat every 15 seconds, however busy you are.
- **Idle** is two minutes after the last beat. Hiding the tab (switching tabs, minimising the window) counts as idle
  at once, and so does closing the page.
- **Each tab counts on its own.** One tab going quiet doesn't end your reading in another, and you count as reading
  while any tab is active.

The deck checks every few seconds, so a laptop closed mid-read, which sends no last beat, still goes idle on time. When
reading starts or stops, the deck tells the runner. A runner that connects later is told straight away.

Two minutes is long enough to read a report without touching anything, and short enough that held mail doesn't wait
long after you walk away.

**If you're building something like this:** count presence from real input, not from the page being open. A tab left
open overnight isn't someone reading. And send the "gone" beat with `keepalive`, so it survives the tab closing.

## New mail waits

While you're reading, the runner still checks your mailbox as usual, but new mail doesn't wake the agent. The runner
keeps count of what's waiting and tells the deck, and the Inbox shows a toast in the top right:

```text
3 new items are waiting
2 Personal, 1 Work                                    [Triage now]
```

- **Go idle** (stop for two minutes, or switch tabs) and the runner wakes the agent once, for everything it held.
- **Triage now** wakes the agent for the held mail straight away, while you stay on the page. The toast goes at once.
  It comes back only if more mail arrives.

Nothing is lost by holding: the mail is still in your inbox, and the count is the runner's own. If the deck can't be
asked about a new message (it's restarting, say), the runner holds it rather than guess.

The runner reads only a new message's From, To, Subject, labels and Gmail's snippet to decide, never its body, and
doesn't log them. A message that's already gone by the time it looks (Gmail sometimes takes back a follow-up nudge)
doesn't wake anyone. In the full deck, new Matrix messages are held the same way.

## What still gets through

Holding everything would be wrong too. Some things you want at once, and some are part of what you're already reading.

- **Your own requests.** What you type in a Change box, an answer to the agent's question, and the chat box at the
  bottom all go straight to the agent. You asked for them, so you expect the page to change.
- **Updates to a thread already on screen.** A new message in a thread that has an open card goes through: the card
  you're looking at is out of date without it, and the agent should revise it.
- **Whatever a "While I'm reading" rule lets through.** Under **Rules → While I'm reading**, a rule names the mail you
  always want straight away. It can match on the source (Email or Matrix), the account (or Matrix room), the sender
  (**From contains**, case doesn't matter), a Gmail label, or a regular expression run against the subject and the
  snippet. A rule
  needs at least one condition, and matches only when every condition it sets does. The Rules screen counts each
  rule's matches and shows when it last matched.

The regular expression uses RE2 syntax (Rust's `regex` crate): no backreferences or look-around, and matching time
grows with the text, never exponentially, so a rule can't stall the deck on a hostile subject line. `(?i)` at the start
ignores case:

```text
Name:                      Sign-in codes
From contains:             no-reply@accounts.example.com

Name:                      Outages
Source:                    Email
Subject or text matches:   (?i)\b(critical|outage)\b
```

When something gets through, the agent's wake-up says so and why, so it handles only that and leaves the rest:

```text
The person is reading the Inbox, so only these get through now: an update on c_4's thread: new mail on Personal
"Re: Saturday". Everything else new (2 Personal) waits.
```

## Every change shows as a toast

Anything that does get through changes the report, so every change is announced. Each time the agent updates the
report, a toast says what changed in a line, in the agent's own words (`quietdeck report set --change "…"`), or in
words the deck works out from the defaults that changed when the agent gave none. Press a toast and the page scrolls to
the default it's about and rings it.

Change toasts stay for a minute, up to four at a time, with **Dismiss All** when there are several. On a wide screen
they sit at the bottom right, above the chat box; the toast about held mail stays at the top right. Changes from before
you opened the page aren't toasted.

## Approve All leaves out what just changed

Holding and toasts make a late change unlikely and visible. The last guard is on the deck itself, at the moment you
approve.

Every approval names the version of each card you saw. When the deck records it:

- **A card with a newer version** than the one on your screen is left out. You approved what you read, not what's
  there now.
- **A card that changed in the last ten seconds** is left out, even when your screen had caught up. That covers both the
  card itself and its default on the report. Nobody can have read something that appeared a moment ago.

Left-out cards stay on the report with their defaults, and the bar says why:

```text
Left alone: changed 3 seconds before you clicked: read it, then approve it
```

Read it, and approve it again. The ten-second rule applies to plain approvals only. An edit you made to a card's
actions, or a Change you typed, is you acting on that card, so it goes ahead. The same check covers each default's own
**Approve**, which goes through the same path as Approve All.

The settle window is checked on the deck, not in the page, so it holds for a slow connection, a stale tab or another
client calling the API.

## The undo window

Nothing you approve runs at once. Approve All, and each default's own Approve, waits for the **undo window** first,
and the button becomes **Undo** while it lasts. Press Undo and nothing happens: the deck drops the whole batch. After
the window, the runner carries out what it can, and the button becomes **Stop** while the actions run.

The window is 3 seconds by default. Change it under **Settings → Approving → Undo window**: off, or 1 to 60 seconds
(the choices are 1, 2, 3, 5, 10, 15, 30 and 60). The deck keeps it as `undo_seconds`, 0 to 60.

Undo and Approve All share the same corner of the bar. A tap meant for Undo that lands just after the window ends
mustn't approve whatever is on screen by then, so Approve All stays greyed out for a moment after each window. Not
after an Undo, though: there's no stray tap to guard against.

## Where this lives in Quietdeck

- The page's heartbeat:
  [web/src/lib/presence.ts](https://github.com/QuietdeckAI/quietdeck/blob/main/web/src/lib/presence.ts)
- Presence, the hold check and the "While I'm reading" rules on the deck:
  [crates/quietdeck-server/src/reading.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/reading.rs)
- The idle timeout, the settle window and how a rule matches:
  [crates/quietdeck-types/src/reading.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-types/src/reading.rs)
- The runner holding new mail and waking the agent (`sort_new`, `wake_if_due`):
  [crates/quietdeck-node/src/runner/mod.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/mod.rs)
- The held-mail toast, **Triage now** and the change toasts:
  [web/src/components/ReportMeta.tsx](https://github.com/QuietdeckAI/quietdeck/blob/main/web/src/components/ReportMeta.tsx)
- The rules' editor:
  [web/src/components/ReadingRules.tsx](https://github.com/QuietdeckAI/quietdeck/blob/main/web/src/components/ReadingRules.tsx)
- Approve All, the cards it leaves out, and Undo on the deck:
  [crates/quietdeck-server/src/store/decisions.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/store/decisions.rs)
- The bar's Approve All, Undo and Stop:
  [web/src/components/ApproveAllBar.tsx](https://github.com/QuietdeckAI/quietdeck/blob/main/web/src/components/ApproveAllBar.tsx)
- Recording what changed on the report:
  [crates/quietdeck-server/src/reports.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/reports.rs)
