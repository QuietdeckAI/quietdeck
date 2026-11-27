# Let agents message each other (and never paste into a terminal)

Agents that work for you soon need to talk to each other. One finishes a fix and tells the session that asked for it,
another hands a job on to the agent that owns that code. On one machine Claude Code already does this with its own
`SendMessage`. Across machines it can't, and an agent that can't reach a session will look for another way in.

This guide is about the way in that goes wrong, and the two commands that replace it.

It assumes a deck and a node on each machine, as in [Getting started](../getting-started.md). The examples use a
`billing` tab on a machine called `laptop` and an `infra` tab on `build-server`.

## What went wrong

An agent finished a job and tried to report back with `SendMessage`. The session it wanted was on another machine, so
`SendMessage` failed. The agent didn't give up: it found the other session's tab with herdr and typed its report in
with `herdr agent prompt`.

The person was typing a message into that tab at the time. The report landed in the middle of it, and both were
mangled.

`herdr agent prompt` types text into a terminal as if a person had typed it. That has three costs:

- **It mixes with whatever is in the prompt box.** A person half way through a sentence gets someone else's text
  spliced into it.
- **The receiving agent can't tell who wrote it.** Claude Code records the text as the person's own message. A
  message that arrives through `SendMessage` is labelled as coming from another session, with Claude Code's warning that
  a peer isn't the user and can't grant permissions. Typed text gets none of that.
- **It's refused when the session is blocked** on a question or a permission prompt, so it fails exactly when the
  receiver is waiting on someone.

[How Claude Code passes messages between sessions](../claude-code-messaging.md) has the details and the tests.

## The fix: two commands, and never the terminal

**On the same machine, use `SendMessage`.** It goes through the session's own messaging socket, into the same queue
that holds what the person types while Claude is busy. An idle session starts a turn with it; a working one reads it at
its next tool round; a blocked one reads it once the person answers. `SendMessage` reaches only sessions of the same
Unix user on the same machine.

**Anywhere else, use `fleetlayer msg send`** (or `quietdeck msg send`, which does the same). That covers a session on
another machine, a session under another Unix user, or any time `SendMessage` says a session isn't reachable.
Messages between sessions are [Fleetlayer](with-fleetlayer.md)'s job, so the machine needs a Fleetlayer node; Quietdeck
alone (solo, on a laptop) has no other sessions to message.

```sh
fleetlayer msg send --to "infra@build-server" "The migration is done. Rebase on main."
```

`--to` takes `<tab>@<machine>`, or a bare `<tab>` when only one machine has a tab of that name. Fleetlayer delivers it
the way `SendMessage` does, through the session's socket, so it's queued and read at the next turn, and its access
rules decide who may message whom. It arrives with who sent it and how to answer:

```text
Message from billing (laptop) via Fleetlayer:

The migration is done. Rebase on main.

Reply with: fleetlayer msg send --to "billing@laptop" "…"
```

So the receiver can answer straight back without looking anything up.

**`quietdeck sessions` lists every session on every machine**, as `<tab>@<machine>`, with its status and a line on
what it's doing. An agent that doesn't know where a session lives reads that, not herdr's tab list.

```sh
quietdeck sessions
quietdeck sessions --node build-server
```

### When a send fails

Nothing is queued. A message to a session that isn't running fails at once, and the agent should do the work another
way or try later. The errors say what's wrong:

- `infra@build-server is offline`: the session, or its machine's node, isn't running.
- `no session named 'triage'; did you mean 'email triage@build-server'?`: a wrong name, with running tabs whose names
  contain what was typed.
- `several sessions have that tab name: …; add @<node>`: two machines have that tab. Add the machine.
- `not confirmed within 5 seconds`: the deck didn't confirm delivery in time. The message may not have arrived; send
  it again.

None of these is a reason to reach for the terminal.

### Messages held in the receiving tab

A Claude Code session started with `--dangerously-skip-permissions` holds messages from outside Claude Code for the
person to approve, unless `~/.claude/settings.json` on that machine has `"crossSessionInbound": "accept"`. Until then,
every Quietdeck message waits in the tab behind a "Deliver / Deny" dialog. herdr shows that session as idle, so it's
easy to miss. That setting also accepts messages from any process of the same user that can read the session's key
file; decide whether that's right for your machine. [Getting started](../getting-started.md) covers it.

## Quietdeck's own herdr fallback

The node does use `herdr agent prompt` in one place: for a session it can't reach any other way. That's a Codex
session, which has no messaging socket, or a running Claude Code session the node can't reach through one (no socket,
or no entry in Claude Code's session registry). Those sessions would otherwise get no messages at all. A Claude Code
session whose process has ended gets nothing typed into its pane: the send fails as offline.

It's never a fallback for a Claude Code session that has a socket. Agents shouldn't copy it either: when the node
chooses herdr, it knows the session can't be reached a better way. An agent that pastes into a tab doesn't.

## Put it in your agents' instructions

Agents follow what their instructions file says. Paste this into the file every session loads (for Claude Code,
`~/.claude/CLAUDE.md`) on every machine:

```markdown
# Messaging other sessions

- On the same machine, message another Claude session with `SendMessage`.
- On another machine, or when `SendMessage` says a session isn't reachable, use
  `quietdeck msg send --to "<tab>@<machine>" "<text>"`. `quietdeck sessions` lists every session on every machine.
- Never type into another session's terminal (`herdr agent prompt`, tmux `send-keys` or anything like it). It lands
  in the middle of whatever the person is typing there, and the receiver can't tell it from the person's own words.
- A send to a session that isn't running fails at once and isn't kept. Do the work yourself or try again later.
```

[Instructions for your agents](../agent-instructions.md) has the full Quietdeck section, which includes these lines.

## Where this lives in Quietdeck

- The `msg send` and `sessions` commands:
  [crates/quietdeck-cli/src/main.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-cli/src/main.rs)
- Finding the target and the errors:
  [crates/quietdeck-server/src/relay.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/relay.rs)
  and
  [crates/quietdeck-types/src/relay.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-types/src/relay.rs)
- Delivery on the receiving machine, socket first and herdr only without one:
  [crates/quietdeck-node/src/deliver.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/deliver.rs)
- How Claude Code's own messaging works:
  [docs/claude-code-messaging.md](https://github.com/QuietdeckAI/quietdeck/blob/main/docs/claude-code-messaging.md)
