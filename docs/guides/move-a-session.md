# Move an agent session to another machine

Sooner or later a long-running agent is on the wrong machine. It started on your laptop and should run all the time on
a server, or the machine it's on is being retired. A session can't be copied: its context is its transcript, which
stays on the machine where it ran. Starting a fresh session somewhere else loses everything it knew, and the cards it
posted stay tied to the old tab.

Quietdeck moves a session with a **handoff**. The old session writes down what a new one needs to know and what it
needs to reach. The deck keeps that, starts the new session from it, links the two, and gives the new one the old
one's open cards and report. Then the old one closes.

This guide assumes a deck and a node on both machines, as in [Getting started](../getting-started.md). For setting up
machines that run agents in general, see Fleetlayer's
[node services guide](https://fleetlayer.ai/guides/node-services/). The examples move a `billing` tab from a machine
called `laptop` to one called `build-server`.

## How it works

1. The old session writes two Markdown files, the handoff and the access manifest, and stores them with
   `quietdeck handoff add`. That prints an ID like `h_7`.
2. Someone prepares the destination against the manifest.
3. `quietdeck session start --handoff h_7` starts the new session. The deck records that it continues the old one and
   moves the old one's open cards and open report to it.
4. The new session reads the handoff, checks the manifest and says whether it's ready.
5. The old session closes.

The steps are also written as a skill for agents,
[skills/move-session/SKILL.md](https://github.com/QuietdeckAI/quietdeck/blob/main/skills/move-session/SKILL.md), so an
agent asked to move itself knows what to do. For Claude Code, copy it to `~/.claude/skills/move-session/SKILL.md` on
each machine.

## 1. The old session writes its handoff

The handoff is written by the session being moved, about itself, for a capable stranger who has read none of the
transcript:

- what the session is for, who it works for, and where the project's spec lives
- what's done, what's half-done (in which files or branches), and what's waiting on whom
- open items, each with its next step
- decisions the person made and why, so nobody asks again
- rules learned the hard way, and commands that need particular flags
- pointers: specs, pull requests, stories, other sessions it works with

It leaves out what the repository's own instructions already say, and never includes a secret's value.

## 2. And its access manifest

The manifest lists everything the new session must be able to reach, one line each, with where it comes from:
repositories (with any uncommitted or unpushed work), CLIs and their versions, credentials by name and source (never
values), SSH hosts and networks, MCP servers and how each is signed in, and running things (watchers, dev servers,
timers, leases), with which of them move and which stay. It ends with what stays behind on the old machine and what
the new session does instead.

The old session stores both:

```sh
quietdeck handoff add --handoff handoff.md --manifest manifest.md
```

```text
h_7 written. Start the session that continues you with `quietdeck session start --on <machine> --tab billing --cwd <dir> --handoff h_7`.
```

Run it from inside the session being moved: the deck records the calling session as the handoff's author. Either file
can be `-` to read it from standard input. The handoff can be up to 100,000 bytes, the manifest up to 50,000.

## 3. Prepare the destination

Go through the manifest on `build-server` line by line: clone the repositories, install the CLIs, provision each
credential through its usual channel (never by pasting a value into a session), and check each host answers. The
machine needs a node connected to the deck and the agent installed and signed in.

Do the agent's first-run screens now too. On a machine where Claude Code has never run, its theme picker and other
one-off screens stop a session from starting. Dismiss them by running it once by hand.

Tell the old session about any line that can't be met. It decides whether that blocks the move.

## 4. Start the new session

From the old session, or anywhere with the CLI:

```sh
quietdeck session start --on build-server --tab billing --cwd "~/code/billing" --handoff h_7
```

```text
Started billing@build-server (session 5c1e9a40-…).
```

- **Keep the tab's name**, so messages addressed to `billing` reach the new session once the old one has closed.
  While both run, a bare `billing` matches two sessions, so senders have to add `@build-server`.
- `--cwd` is a directory on the destination; `~/` is that machine's home.
- Match the old session with `--agent codex`, `--model` or `--effort` where it used them. Permission flags come from
  the destination's `node.yaml`, never from the command.

Without `--prompt`, the new session's first prompt points it at the handoff:

```text
You continue the session billing@laptop (session ID 81d0b7f2-…), which was moved to this machine. Read its handoff
and access manifest first: `quietdeck handoff show h_7`. Then check you can reach what the manifest lists, and reply
with one line saying you're ready or what's missing.
```

When the start succeeds, the deck links the two sessions and moves the old one's open cards and open Inbox report to
the new one. That matters for a triage agent: the runner carries out approvals only on its own agent's cards, and the
Inbox sends your answers to the report's tab. Cards that are done, handled elsewhere or automatic stay where they were.

A handoff starts one session. A second start from `h_7` is refused with `h_7 was already continued by
billing@build-server`.

### When the start stops at a prompt

If the new tab stops on a screen before the agent is ready, the start fails and the handoff isn't continued:

```text
… Handoff h_7 isn't continued yet. If the new tab is open, deal with what it stopped at, send it its first prompt
(`quietdeck msg send`), and have it run `quietdeck handoff claim h_7`.
```

Don't start again. Open the tab on `build-server`, deal with what it stopped at, send it the first prompt with
`quietdeck msg send --to "billing@build-server" "…"`, and have the new session run:

```sh
quietdeck handoff claim h_7
```

```text
You continue billing@laptop from h_7: its open cards and its report are yours now.
```

Claiming does what a successful start would have done. Claiming again from the same tab is harmless. The old session
can't claim its own handoff, and no session can claim one that another already continues. Until the handoff is
claimed, the old session keeps its cards and report.

## 5. The new session reads and checks

The new session runs `quietdeck handoff show h_7`, which prints who wrote it, who continues it, the handoff and the
access manifest. It checks every manifest line it can: runs each CLI, opens each repository, makes a harmless call
with each credential, reaches each host. Then it replies with one line: ready, or exactly what's missing. It starts
the running things the manifest says move with it.

## 6. Close the old session

Once the new session is ready, the old one stops the running things that move, releases its leases (the new session
takes its own), tells the person and the sessions it works with where it is now, and closes:

```sh
quietdeck session close "billing@laptop"
```

A session can close itself, or a session that has moved (one that a session started from its handoff now continues),
so the new session can close the old one too. You can close any session. A session that hasn't moved can't be closed
by another session.

If the new session reports something missing that can't be fixed soon, don't close the old one. Keep it working and
close the new one instead.

## After the move

The old transcript stays on `laptop`. The deck remembers the link both ways:

```sh
quietdeck sessions show billing@build-server
```

prints `continues billing@laptop (session 81d0b7f2-…), moved here from handoff h_7`, and the old session's entry says
`moved to billing@build-server on <time> (handoff h_7)`. `quietdeck who-has` marks a hit in the old session's transcript with
`(moved to billing@build-server)`, so someone looking for earlier work finds where it went. The timeline records the
start as continuing `billing@laptop` from `h_7`.

## Where this lives in Quietdeck

- The steps, for agents:
  [skills/move-session/SKILL.md](https://github.com/QuietdeckAI/quietdeck/blob/main/skills/move-session/SKILL.md)
- `handoff add`, `handoff show`, `handoff claim`, `session start` and `session close`:
  [crates/quietdeck-cli/src/main.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-cli/src/main.rs)
- Storing handoffs, the first prompt, claiming, and moving cards and the report:
  [crates/quietdeck-server/src/handoffs.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/handoffs.rs)
- Starting from a handoff and who may close a session:
  [crates/quietdeck-server/src/control.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/control.rs)
- The handoff's fields and limits:
  [crates/quietdeck-types/src/handoff.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-types/src/handoff.rs)
