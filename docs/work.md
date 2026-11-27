# Work: what your agents are doing

The Inbox is one document: what came in today, what needs you at the top, the rest under Archive all, and a thread
under every item. **Work** is the same kind of document for what your agents are doing. It lists what's waiting on
you, what's finished and what's still running. An agent called the **overseer** keeps it up to date.

<!-- DRAFT: names and settings to confirm with the quietdeck work session before this ships. -->

## Documents and tabs

Every page you read in Quietdeck is a **document**. A document has a name, the tab it sits under, and the agent that
writes it. A deck starts with two tabs:

- **Inbox:** today's mail, written by the triage agent ([How Quietdeck works](concepts.md)).
- **Work:** your agents' work, written by the overseer.

You name tabs and documents yourself, and nest documents under a tab: Work could hold "Business", "Personal" and
"Side projects", each kept separate by the overseer's instructions. A tab with one document shows no sub-tabs. Each
document has its own **Approve All**.

[Quietdeck solo](solo.md) has one document, the Inbox, unless you add Work.

## Question panels

Some entries ask you something instead of proposing an action. A **question panel** has no approve check, just a
question and its own thread. Some offer answers as buttons, with a box for anything else. Others take a written
answer. You answer in the thread, and the agent updates the entry. The Inbox has them too.

## What's at the top of Work

Everything that needs you, newest first:

- **A blocked prompt:** a session stopped on one of its own menus ("Allow this command?"). It shows as a small form,
  and your choice is typed into the session.
- **A question** an agent is asking you.
- **A finished session** waiting to be wrapped up.
- **Work waiting for your review:** a pull request, a draft.
- **Your tasks** due today, or already started.
- **Things only you can do:** paste a key into your password manager, touch a security key. These have a **Done** in
  their thread, or the overseer marks them done when it sees the result.

## The FYI list

Under what needs you comes an **FYI** list: progress and completions ("The release session shipped 0.4.2"), under its
own **Archive all**. Approve All clears the FYI lines.

Work is never wiped after Approve All. It's an ongoing document. Entries stay until they're done or no longer matter,
and the overseer prunes it as it goes.

## Each entry is a session

Most entries are about one agent session. Where an Inbox item has a mail icon, a Work entry has a session icon. Its
menu shows the session's name (`tab@machine`), a summary of what it's doing, and **Open transcript**.

What you can approve under an entry:

- **Wrap up session:** asks the session to run its wrap-up: list loose ends, propose stories and comments. The
  overseer reads the result and puts those proposals under the entry, with **Close session** last.
- **Close session.**
- **Message session:** passes your words on.
- **Create a story, or comment on one.**

The overseer never starts or closes a session without your approval.

## Parking

Not everything needs doing this week. **Park** a session or an entry to put it away indefinitely, or **snooze** it until
a date. Parked entries sit in a collapsed **Parked** section at the bottom of Work, each with **Unpark**.

## The overseer

The overseer is an agent like the triage agent, with a different job. Instead of mail it reads your other sessions: a
summary first, and only the new part of a transcript when it needs detail. It looks at a session when it goes idle,
and skips one that's still waiting on CI or a background task. It also reads your task list for today.

It leaves out the triage agent and any session you mark **ignored**, unless one of them is stuck on a prompt.

You chat with it in Work's threads. It answers from what it's read, or passes your message on to the session.

The overseer shares the runner with the triage agent. The runner carries out what you approve and hands anything it
can't do back to the agent that posted the entry. [Set up an overseer](guides/overseer.md) walks through it.
