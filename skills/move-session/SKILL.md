---
name: move-session
description: Move a running Claude Code or Codex session to another machine that runs a Quietdeck node, so a new session there carries on its work. The old session writes a handoff (what the new one needs to know) and an access manifest (what it needs to reach), the destination is checked against the manifest, the new session starts from the handoff with `quietdeck session start --handoff`, and the old one closes. Use when someone asks to move a session or an agent to another machine, server or VM, or when you're told to move yourself.
---

# Moving a session to another machine

A session can't be copied between machines: its context lives in its transcript, which stays where it ran. Moving
one means a new session on the destination that knows what the old one knew and can reach what it reached. The deck
keeps the link: `quietdeck sessions show` on either one names the other, the new session takes over the old one's open
cards and open Inbox report, and the old transcript stays on its machine and in the owner's archive.

Two roles: the **old session** (the one being moved) does steps 1, 2, 4 and 6. Whoever prepares the destination (the
person, or the session that runs that machine's provisioning) does step 3. The **new session** does step 5.

## 1. Write the handoff

A Markdown file for the new session, written by the old one about itself. Write it for a capable stranger who has
read none of the transcript:

- **What you're for**: your tab's job, who you work for, the project and where its spec lives.
- **Where you're up to**: what's done, what's half-done (and in which files or branches), what's waiting on whom.
- **Open items**: each with its next step, and who decides it when it isn't yours to decide.
- **Decisions and why**: what the person settled, in their words where it matters, so nobody asks again.
- **Conventions and gotchas**: the rules you learned the hard way, commands that need particular flags, things not to
  touch.
- **Pointers**: specs, docs, PRs, stories, dashboards, other sessions you work with and how to reach them.

Leave out what the repo and its AGENTS.md already say, and never include a secret's value.

## 2. Write the access manifest

A second Markdown file: everything the new session must be able to reach, one line each, with where it comes from.

- **Repositories**: path, remote, branch, and uncommitted or unpushed work (commit or push it first, or say how it
  travels).
- **CLIs and toolchains**: each command you use, with its version where it matters (`gh`, `cargo`, `pnpm`, `herdr`,
  `quietdeck`, `rack-gateway`...).
- **Credentials**: each one by name and where it comes from (1Password item and vault, a file path, a keychain entry,
  an OAuth sign-in). Names only, never values.
- **Hosts and networks**: SSH hosts and the keys they need, Tailscale or LAN names, registries, the deck's URL.
- **MCP servers and tools**: each server and how it's signed in (Executor apps, Chrome, Talira...), and which ones
  won't exist on the destination.
- **Running things**: background processes, dev servers, watchers, timers, launchd or systemd jobs, and leases you hold.
  Say which move, which stop, and which stay where they are.
- **What stays behind**: anything tied to the old machine (a browser profile, local data that mustn't leave it), and
  what the new session does instead.

Then store both:

```bash
quietdeck handoff add --handoff handoff.md --manifest manifest.md   # prints h_N
```

## 3. Prepare the destination

Go through the manifest on the destination machine, line by line: clone the repositories, install the CLIs, provision
the credentials through their normal channels (never by pasting values into a session), and check the hosts answer.
The destination needs a Quietdeck node connected to the deck, and the agent installed and signed in. Note each line
that can't be met and tell the old session; it decides whether that blocks the move.

## 4. Start the new session

From the old session (or anywhere with the CLI):

```bash
quietdeck session start --on <node> --tab "<tab>" --cwd "~/code/<repo>" --handoff h_N
```

Keep the tab's name, so messages to it still arrive. Add `--agent codex`, `--model` or `--effort` to match the old
session. Without `--prompt`, the new session's first prompt tells it to read the handoff and check the manifest.

## 5. As the new session: read, check, report

Run `quietdeck handoff show h_N` and read both parts. Check every manifest line you can (run each CLI, open each
repo, make a harmless call with each credential, reach each host). Then reply with one line: ready, or exactly what's
missing. Start or re-arm the running things the manifest says move with the session (watchers get their timeouts).

## 6. Close the old session

When the new session says it's ready: stop the running things that move, release leases you hold (the new session
takes its own), tell the person and the sessions you work with where you are now, and close:

```bash
quietdeck session close "<tab>@<old node>"
```

If the new session reports something missing that can't be fixed soon, don't close: keep working, and close the new
one instead.
