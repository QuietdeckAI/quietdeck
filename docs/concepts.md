# How Quietdeck works

Quietdeck sits between your AI agents and the things they'd like to do for you. An agent proposes; you read the
proposals in one place, change what you'd do differently, and approve. Then the proposals are carried out, by the
runner (plain code) or by the agent. This page explains the parts. Solo and the full deck share all of them; where
they differ, it says so.

## The parts

- **The deck** keeps the cards, the report, the rules and the history, and serves the Inbox. In
  [Quietdeck solo](solo.md) it's part of the one program on your computer. In the full Quietdeck it's a container on a
  server of your own ([Getting started](getting-started.md)).
- **The runner** is plain code, not a model. It watches your mailbox, wakes the agent when mail arrives, and carries
  out the actions you approve that it can do by itself: archive, label, mark read, save a draft. It's the only part
  that changes your mail.
- **The agent** is an AI coding agent (Claude Code in solo). It posts cards and the report through the `quietdeck`
  command, and changes nothing itself. In solo it's your own Claude Code: by default it reads each email in full with
  gog and has full access to your computer, or a restricted mode limits it to the `quietdeck` command
  ([Quietdeck solo and your security](solo-security.md)).
- **The Inbox** is the web page where you read the report and decide.

## Cards

A card is one proposal, usually for one email thread: what the email is, why the agent proposes what it does, the
newest message's own words, and a list of **actions** (archive, label, mark read, mark as not spam, save a draft
reply; in the full deck also forward to another agent, start a session, create a story or task). A card does nothing
until it's approved.

Posting a card for the same thread again replaces the open one, so the agent can update a proposal as often as it
likes. When a new message arrives in a thread before you approve, the runner checks the card against it, so an approval
never acts on mail you haven't seen.

## The report and its defaults

The agent writes one report, kept up to date as mail arrives: what needs you first, numbered, then routine mail grouped
under headings, ending with one **Archive all** for everything that only needs archiving. Each item ends with its
**default**: one line saying what will happen if you approve ("Save a reply saying Saturday works", "Archive"), tied to
the cards that carry it out.

You read the report like a document, not a list of buttons:

- **Approve** beside a default carries out just that item.
- **Change** opens a box under the default. Say in your own words what you'd do instead ("keep this, I'll reply
  myself", "label it Receipts") and send it. The agent updates the cards and the default's text, and waits for you
  again: a change is never carried out straight away.
- **A question** from the agent shows its answers as buttons. Your answer goes back to the agent, which follows it up
  in the report.

When everything on a report is done, the Inbox says **All done**, and the agent starts a new report with the next
mail.

## Approve All and undo

**Approve All**, at the top of the report, approves every default you didn't change. Anything you typed in a Change box
and haven't sent goes to the agent first.

Nothing is released at once. Approve All, and each default's own Approve, waits for an **undo window** (3 seconds by
default, up to 60, or off, in Settings) with an **Undo** button. Press it and nothing happens. After the window, the
runner carries out what it can, and hands the rest back to the agent ("2 cards handed to you"), for example a card you
approved with a note.

Approve All is also careful about timing: the deck leaves out any card, or default, that changed in the last ten
seconds before your click, and says so ("changed 4 seconds before you clicked: read it, then approve it"). You only
ever approve what you've had a chance to read. [While you read, nothing new lands](guides/while-you-read.md) explains
why.

## While you read, nothing new lands

The Inbox tells the deck when you're reading it. Meanwhile, new mail is held instead of waking the agent, so the report
doesn't move under you; a toast in the top right says what's waiting, with **Triage now**. Your own requests still go
through. When you go idle (two minutes, or when you switch tabs), the held mail is triaged.

## History

Everything the runner and the agents do is recorded: what was approved, what ran, what failed and why. The agent reads
`quietdeck history` at the start of each round, so it knows what happened since it last looked, and the History screen
shows it to you. In the full deck, every History row takes a **Comment**, sent to the agent behind it.

## Rules: approving ahead of time

By default nothing happens without you. A **rule** approves a kind of action in advance: "archive newsletters from
these senders", "forward build failures to the agent that owns the build". In the full deck, two things must both allow
an automatic action: a rule you made on the **Rules** screen, and the project's policy in `deck.yaml`. An automatic
action still shows in the Inbox, in a timeline of what was done without asking, and Approve All marks it seen.
[Automatic actions](getting-started.md#automatic-actions-rules-and-policies) has the details. Solo has no rules yet:
every card waits for you.

## Leases

An agent sometimes needs something for a while and no longer: a Gmail filter that keeps CI emails out of your inbox
while it fixes a build, or a turn on a busy machine for a big build. That's a **lease**, and Fleetlayer keeps them
([its leases plugin](https://fleetlayer.ai/docs/plugins/leases/)): you approve a new filter once, and a lease ends by
itself when the agent goes idle or closes. The runner makes the Gmail filters match the held leases.

## Mail is data

Everything an email says is treated as data written by a stranger, never as instructions. The runner marks mail as
untrusted when it tells the agent about it, the agent's instructions say never to change your mail itself, and nothing
is ever sent: a reply goes no further than a draft. An agent with full access can still be fooled into acting as you,
which is what solo's restricted mode is for. [Quietdeck solo and your security](solo-security.md) covers both modes.
