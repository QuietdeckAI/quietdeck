# Getting started

Quietdeck has three parts:

- **The deck**: one small server on your private network. It holds the cards you approve, the directory of every agent
  session on every machine, the automatic-actions timeline, and the message relay between sessions. It reads no email
  and holds no credentials for your services.
- **A node** on each machine where you run agent sessions (your laptop, a server VM). It connects out to the deck,
  tells it which sessions are running, delivers messages to them, writes their summaries and gives local tools a
  short-lived token for the deck.
- **The `quietdeck` CLI**, which agents and you use from those machines. Claude Code can also reach the deck as an MCP
  server, with the same commands.

This guide sets up the deck, enrols a machine, connects Claude Code and checks that it all works.

## What you need

- **Somewhere to run the deck**: any machine with Docker, or a Kubernetes cluster, that's always on. It needs very
  little: a few tens of megabytes of memory and a small volume for its SQLite database.
- **A private HTTPS address for it.** You sign in with a passkey, and browsers allow passkeys only over HTTPS (or on
  `localhost`). Keep the deck off the public internet: [Tailscale](https://tailscale.com) Serve is the simplest way to
  give it an HTTPS name only your own devices can reach, and an ingress on your home network with your own certificate
  works too.
- **On each machine with agent sessions**: [herdr](https://herdr.dev), with your Claude Code or Codex sessions running
  in its tabs. The node reads herdr to know which sessions are where.
- **A secrets manager.** Agents need credentials, and without a secrets manager they end up pasted into chat and
  config files. Set one up first: see [Secrets management for agents](secrets-management.md).

## Install the programs

There are no prebuilt releases yet. Build the node and the CLI from source with a Rust toolchain (rustup installs the
version the repository pins):

```sh
git clone https://github.com/QuietdeckAI/quietdeck
cd quietdeck
cargo install --locked --path crates/quietdeck-node
cargo install --locked --path crates/quietdeck-cli
```

That puts `quietdeck-node` and `quietdeck` in `~/.cargo/bin`. Do this on every machine that runs agent sessions.

## Run the deck

The deck is one container. Build its image from the repository root (it builds the web app too):

```sh
docker build -f deploy/Dockerfile -t quietdeck .
```

Then run it, with port 8080 published only on the host's loopback address, and give it an HTTPS name. With Tailscale
Serve on the same host:

```sh
docker volume create quietdeck-data
docker run -d --name quietdeck --restart unless-stopped \
  -v quietdeck-data:/data \
  -e QUIETDECK_PUBLIC_URL=https://<deck address> \
  -p 127.0.0.1:8080:8080 \
  quietdeck
tailscale serve --bg --https=443 http://127.0.0.1:8080
```

- `QUIETDECK_PUBLIC_URL` is the address you'll open the deck at: with Tailscale Serve, the HTTPS name
  `tailscale serve` prints (`https://<machine>.<tailnet>.ts.net`). Your passkeys belong to that name, so changing it
  later means adding them again. It has to be a name, not an IP address.
- On Kubernetes, see [deploy/README.md](../deploy/README.md) for the ingress and the network policy.

Check it's up:

```sh
curl -s https://<deck address>/healthz
```

It answers `ok`. The deck's own settings (your account labels, boards and the rest) go in `/data/deck.yaml`: start
from [deploy/deck.example.yaml](../deploy/deck.example.yaml). Every key is optional.

## Sign in

Nothing on the deck works until you've signed in, and you sign in only with a passkey. The first one needs a one-time
setup code from the deck's container:

```sh
docker exec quietdeck quietdeck-server setup-code
```

Open `https://<deck address>` in your browser, enter the code, name the device and create the passkey (Touch ID,
Face ID, Windows Hello or a security key). The code is good for 15 minutes, once.

To add your phone or another computer, sign in on a device you've already added, open **Settings**, and add a passkey
there. A passkey synced through iCloud Keychain, Google Password Manager or 1Password signs you in on your other
devices too. Each device stays signed in until you sign out, or until it hasn't been used for 30 days.

Keep the deck open in a pinned tab if you like: its tab icon shows how many cards are in your Inbox, and so does its
title ("(3) Quietdeck"). Switch **Show the pending count on the tab icon** off in Settings if you'd rather it didn't.

## The Inbox: the triage report

The Inbox is one report the triage agent writes and keeps up to date as mail arrives: what needs you, numbered, then
groups it handles the same way (sales pitches, mail it will archive, grouped by subject). Each item says what will
happen by default. Read it like a document:

- **Change**, beside a default, opens a box in it: say what to adjust or add (or what to do instead) and **Send**. It
  goes straight to the agent, and Approve All leaves that item to it. The **…** beside it has **View email**, which
  opens the thread in Gmail.
- **A question** shows its answers as buttons ("Was this you? Yes / No"), sometimes with **Other**. Your answer goes
  straight to the agent, which follows it up in the report.
- **Approve All**, at the top, carries out every default you didn't change. Anything typed in a Change this box and
  not sent yet goes to the agent first.
- **The box at the bottom** sends plain words to the triage agent ("archive HelloFresh from now on"). It's only passed
  on: if the agent's session isn't running, it says so and keeps your text.

The agent writes the report with `quietdeck report show` and `quietdeck report set` (`report show --markdown` prints
only the text, ready to edit and pass back; `quietdeck report --help` lists the two tags it uses for defaults and
questions). Under the report, the agent's cards carry each default's actions.

## Threads: every thread in a list

**Threads**, beside Inbox, lists the same mail one thread per row: what each email is, its facts, the actions it
proposes, and a box to say what to do instead. A new message in a thread updates its row. A session's request for a
temporary Gmail filter is a row here too. Use it when you want the whole list rather than the report. Its own **Approve All**
decides every card on screen, and **Later** is at the bottom. When the agent has posted cards its report doesn't
mention yet, the Inbox says so under the report, with a link here.

## Pings on your phone

An agent can ping your phone when something can't wait a few hours: at first, a reply on a thread you watch, or an
urgent event such as a disk running out. Nothing else is pushed: no new cards, no counts.

The deck needs a VAPID key pair to send pushes. Make one:

```sh
docker run --rm quietdeck vapid-keys
```

It prints `QUIETDECK_VAPID_PUBLIC_KEY=…` and `QUIETDECK_VAPID_PRIVATE_KEY=…`. Keep the private key in your secrets
manager ([secrets management](secrets-management.md)), and start the container again with both set, plus a contact
address push services can use:

```sh
docker rm -f quietdeck
docker run -d --name quietdeck --restart unless-stopped \
  -v quietdeck-data:/data \
  -e QUIETDECK_PUBLIC_URL=https://<deck address> \
  -e QUIETDECK_VAPID_PUBLIC_KEY -e QUIETDECK_VAPID_PRIVATE_KEY \
  -e QUIETDECK_VAPID_SUBJECT=mailto:<you@example.com> \
  -p 127.0.0.1:8080:8080 \
  quietdeck
```

(`-e NAME` with no value passes the variable from your shell, so the key never sits in your shell history.) Keep the
same pair from then on: a new one stops every device's pushes until you turn them on again.

Then, on the phone:

- **iPhone or iPad:** open the deck in Safari, tap Share, then **Add to Home Screen**. Open Quietdeck from the Home
  Screen (iOS allows push only there), sign in, open **Settings** and tap **Turn on notifications on this device**,
  then **Allow**.
- **Android, or a computer:** open the deck in Chrome, Edge or Firefox (install it from the browser's menu if you
  like), open **Settings** and turn notifications on.

Settings lists every device that takes pings, and when a ping last reached it. Tapping a ping opens it on the
**Rules** screen, with its link and the agent's reason for not waiting. **Don't ping for this again**, with a note if
you like, goes back to the agent that sent it: it picks the note up at the start of its next round and makes it a
rule of its own, and the ping shows when it did. Rules also shows how many pings came in the last seven days against
the weekly budget (`pings: { weekly_budget: 10 }` in `deck.yaml`). The budget only shows when agents ping more than
you'd like; no ping is ever held back.

**Watched threads** are listed on Rules too. Watch one from a card's menu (**Watch thread**), or approve a card that
proposes watching it.

## Enrol a machine

Each machine gets its own node token, from a one-time code. In the deck's **Settings**, press **Enrol a machine**: it
shows the whole command with a fresh code. Or get a code on the deck's host:

```sh
docker exec quietdeck quietdeck-server enrolment-code
```

The code is good for 15 minutes, once. On the machine you're enrolling:

```sh
quietdeck node enroll <code> --deck https://<deck address> --name <machine name>
```

That saves the node's token in `~/.config/quietdeck/node-token`, readable only by you, and writes
`~/.config/quietdeck/node.yaml`, which holds the deck's address and the token file's path. The node refuses to start
if other users can read the token file. To keep the token in your secrets manager instead, see
[How the node reads its token](secrets-management.md#how-the-node-reads-its-token).

Then run the node as a service, so it starts when you log in and restarts if it stops:

```sh
quietdeck-node install
```

On macOS that's a launchd agent, logging to `~/Library/Logs/quietdeck-node.log`. On Linux it's a systemd user service
(`journalctl --user -u quietdeck-node` for its logs); run `loginctl enable-linger "$USER"` too if the machine should
keep it running while you're logged out. The service runs herdr and Claude Code with the `PATH` you installed it
from, so run `quietdeck-node install` again if you move either of them. `quietdeck-node uninstall` stops and removes
it.

Check it connected:

```sh
quietdeck node status
```

It prints the machine's name, the deck's address and `connected`.

## Connect Claude Code

Agents use the `quietdeck` CLI from their shell, which works as soon as the node runs: the CLI asks the node for a
token, and the node checks which Claude Code session is asking before it gives one. So the deck always knows which
session (and which herdr tab) a card or a message came from.

To give Claude Code the same commands as MCP tools, add the deck as an MCP server, once per machine:

```sh
claude mcp add-json --scope user quietdeck \
  '{"type":"http","url":"https://<deck address>/mcp","headersHelper":"quietdeck auth headers"}'
```

`quietdeck auth headers` prints a fresh token for the session that runs it, and Claude Code runs it again when a token
expires, after an hour.

## Check that it works

From a Claude Code session on an enrolled machine (or ask the agent to run these):

```sh
quietdeck whoami
```

It names the session's herdr tab and machine, such as `You are api@laptop (session 0199…)`, and the deck's accounts.

```sh
quietdeck sessions
```

It lists every session on every enrolled machine, with its status, and its one-line summary once the first summary
is written (see below).
`--all` adds sessions that have ended, and `--since 180d` (or a date, `--since 2026-05-23`) lists every session
active since then, running or not. One session's summary, with every earlier one-line update and the command that
resumes it:

```sh
quietdeck sessions show <tab>@<machine> --timeline
```

```sh
quietdeck msg send --to <tab>@<machine> "Hello from the other machine"
```

The message appears in that session at its next turn, with a line telling it how to answer. Messages to a session
that isn't running fail at once and aren't kept; Quietdeck never queues messages between agents.

Your agents learn when to use it from your global instructions: [Instructions for your agents](agent-instructions.md)
has the section to paste in.

```sh
quietdeck who-has <some ID you've worked on>
```

It finds the sessions that mention it: in their summaries on the deck, and in the raw transcripts on every connected
machine. Transcripts never leave their machine; the deck only learns which sessions mention the ID and when they
last did. Each machine searches its newest transcripts first, for a few seconds, and says how far back it got. Give
several IDs or quoted phrases and each is looked up on its own, with its own answer.

## The Sessions screen

**Sessions** lists every session on every machine, the most recently active first, with its machine, project, what
it's doing and its summary sentence (click it for the paragraph). The search box takes words or an ID (`sc-1234`, an
invoice number): it searches every session's summaries, and the transcripts on the machines that are connected. A
machine that isn't connected is searched by summary only, and the results say so.

**Open** shows one session:

- **Since your last message:** a short catch-up of what happened after you last wrote in it, written on its machine.
  The first one for a session takes a few seconds; after that it's served at once and rewritten as the session goes
  on.
- **The transcript,** read from the session's machine as you scroll and kept on that machine. It opens at your last
  message; scrolling up loads earlier turns, compactions included, and new turns appear as they happen. Thinking and
  tool calls fold into one line each (click to see a call's input and output), and wake-ups from background tasks and
  watchers are dimmed. Claude Code sessions only for now.
- **Subagents:** the subagents the session started, beside the transcript (under **Subagents** on a phone). Pick one
  to read its transcript.
- **Open in herdr** (on a computer) brings the session's tab forward on its machine, through herdr's `herdr://` link.

**Page** shows the session's full summary, one update at a time, with **Prev** and **Next** and every update's
sentence in a timeline below it.

## Starting, resuming and closing sessions

The node follows herdr's event stream, so a session's status reaches the deck within a second of changing. A herdr
without that stream (before its client contract) still works: the node reads its agents every 3 seconds instead,
and says so once in its log.

From the deck you can start a session on any connected machine, resume one that isn't running, and close one. Each
opens or closes a herdr tab without moving anything on that machine's screen, and History lists it. From a shell or
an agent:

```sh
quietdeck session resume <tab>@<machine>
```

That resumes the session in a new tab with its old name, in its own working directory. With a tab name that has had
several sessions, it picks the one that ended last; a session ID picks exactly one.

Permission flags such as `--dangerously-skip-permissions` never come from the deck. Each machine sets the arguments
its sessions start with in the `sessions:` section of `node.yaml`:

```yaml
sessions:
  claude_args: [--dangerously-skip-permissions]
  codex_args: [--no-daemon]
```

A session that went away because herdr stopped or the machine shut down shows as not running, without a close time:
herdr sends no close events while it stops. A tab someone closed, or an agent that exited, shows how it ended.

## Automatic actions: rules and policies

Nothing happens without your approval until you grant it, rule by rule. Two things must both allow an automatic
action:

- **A rule you made**, on the **Rules** screen. A rule says what
  counts as a match and which actions it allows. **The agent judges** (the default) leaves the match to the triage
  agent, for mail from the sender domains you list and no others. **Exact match** has the deck check the account, the
  sender's address or domain and, optionally, the subject. It needs the message to have passed DMARC or DKIM, which
  the agent reports on the card, so a forged sender can't use it. A rule that forwards to, resumes or starts a session
  names that session.
- **The project's policy**, in `deck.yaml`. A project is a set of working directories, or of accounts for actions on
  no session, with the action types it allows automatically:

  ```yaml
  projects:
    app:      { dirs: [~/code/app], automatic: [forward, archive, story, task] }
    personal: { accounts: [personal], automatic: [forward, archive] }
  ```

  A forward or resume is judged by the project of the session it goes to, and anything else by its account's
  project. Replies are in no policy unless you add them. The Rules screen shows the policies; change them in
  `deck.yaml` and restart the deck.

The triage agent asks for an automatic action when it posts a card:

```sh
quietdeck rules list
quietdeck card add card.json --auto --grant g_3
```

Allowed, the card is carried out at once, by the runner or the agent, and appears in the Inbox's timeline of
automatic actions. Approve All marks those seen and moves them to History. Denied, it waits for you like any card,
with the reason. `quietdeck rules match card.json --grant g_3` gives the same answer without posting anything.
Automatic resumes and new sessions are limited to 3 in any 24 hours.

A rule tied to a session is archived when that session ends: its tab closes or the agent exits (not when herdr
restarts or a machine sleeps). The timeline says so. When a later card forwards to a new session in the same tab, it
offers to move those rules there. Archived rules can also be restored from the Rules screen.

Every timeline entry and History row takes a **Comment**. It goes to the session behind it (the agent that posted the
card, or the fallback tab in `deck.yaml` for the deck's own entries) as a message. A session that isn't running gets
it from `quietdeck history --since last` on its next round, and anything it proposes comes back as a card.

## Leases: temporary filters and build slots

A session watching CI can ask for a temporary Gmail filter that keeps those emails out of your inbox while it works,
and sessions on one machine take turns for big builds. Both end on their own when the session goes idle or closes.
How they work, what you approve, and the runner's filter tools: [Leases](leases.md).

## Session summaries

Each node keeps a short, current summary of every Claude Code and Codex session on its machine, and sends it to the
deck. The Sessions screen, search, `quietdeck sessions` and `quietdeck who-has` all read these summaries. You don't
have to set anything up: summaries start when the node starts.

### What a summary is

Every session has three sizes of summary, rewritten together:

- **Sentence:** one line saying what the session is working on and where it stands.
- **Paragraph:** one or two short paragraphs: what the session is for, where it stands, and what's open.
- **Page:** up to about a page of Markdown: the session's purpose, what happened recently (with dates), and what it's
  waiting on. It keeps the identifiers you'd search for, such as ticket IDs, pull request numbers and branch names.

Each summary is also a point on the session's timeline, one per hour of activity. A later summary in the same hour
replaces the earlier one.

### What gets summarised

Every session you run yourself in Claude Code or Codex, on every machine with a node, in any directory. The node
reads the session's transcript on that machine. Transcripts stay on the machine; only the summaries go to the deck.

Left out:

- **Subagents.** The parent session's transcript already has each subagent's task and what came back.
- **Headless runs:** `claude -p` and Agent SDK sessions, including the node's own summary calls.
- **`codex exec` runs**, such as code reviews another agent started.
- **Sessions not touched in six months.**

Within a session, the node reads only your messages, messages from your other sessions, the agent's replies and one
line per tool call. Tool output, background-task and watcher notifications, scheduled wake-ups, compaction summaries,
built-in slash commands and interruptions are dropped before anything reaches a model.

### When a session is summarised

- **When it goes idle.** A session that finishes a turn and stays idle for a minute is summarised. If it starts
  working again within the minute, it waits for its next idle.
- **Never while it's waiting on you.** A session at a permission prompt or a question isn't summarised until you've
  answered and it has gone idle.
- **At most once every 30 minutes per session.** If a session goes idle again within 30 minutes of its last summary,
  the node schedules one summary for when the 30 minutes are up. Nothing is dropped.
- **When a session ends.** Closing its tab, or its process exiting, counts as idle.
- **The daily sweep.** When the node starts, and once a day after that, it summarises any session that isn't running
  and has activity it hasn't summarised yet. That catches sessions that ended without an idle moment, such as a
  crash or a machine that went to sleep mid-turn.

Not every idle moment costs a model call. A summary is written only when the new activity holds a message from you or
from another session, 300 or more characters from the agent, or a file edit. When there is something to summarise but
nothing changed, the model answers "no change" and the summary stays as it was.

If the deck is unreachable for a while (a laptop asleep, the deck restarting), the node holds the summaries it writes
and sends them when it reconnects.

### What it costs

Each summary is one headless Claude Code call on Sonnet, on that machine's Claude subscription. A call takes 10 to 15
seconds and uses 4,000 to 25,000 input tokens. About 3,000 of those are the fixed prompt; the rest is the current page
plus the new activity, which is capped at 48,000 characters (roughly 12,000 tokens). Over the cap, tool lines are left
out first, then older agent replies, then your older messages. Your newest messages always stay.

As a rough guide, a session that's active all day gets 6 to 10 summaries. Only sessions that did something that day
cost anything. The node makes at most two summary calls at once.

### Which account it uses

With several Claude accounts on the machine, each call uses the one whose week resets soonest and still has room.
With one account, calls use Claude Code's own login. If every account has run out for the week, summaries wait, and
the next idle session or the daily sweep tries again.

The calls run with no settings, hooks, plugins, tools, MCP servers or CLAUDE.md, so your configuration doesn't add to
their cost or change what they do.

### Settings

All optional, in the `summaries:` section of `node.yaml`:

```yaml
summaries:
  enabled: true
  idle_seconds: 60          # how long a session must stay idle
  min_gap_seconds: 1800     # at most one summary per session in this window
  max_concurrent: 2         # summary calls at once on this machine
  input_cap_chars: 48000    # new activity per call
  daily_chunk_days: 30      # older history is summarised by day for this long, then by week
  sweep: true               # the start-up and daily sweeps
  model: sonnet
  user_name: <your name>    # how summaries refer to you; "the user" if empty
```

To make summaries cheaper, raise `min_gap_seconds`. Turning off `sweep` saves little and leaves crashed sessions
unsummarised.

### What summaries contain

Summaries say what each session worked on, which can include customers, money and anything else you discussed with
it. The model is told never to copy secrets (tokens, passwords, keys), bank or card numbers, or personal data about
your own customers' end users, and to refer to records by ID instead. It's a rule in the prompt, not a filter, so keep
your deck private: on your own network, behind your passkeys.

### Coming from claude-summarizer

If you ran claude-summarizer, the node picks up where it stopped. The first time the node starts on a machine that
ran claude-summarizer, it adopts claude-summarizer's byte offsets (from `~/.local/state/claude-summarizer/state.json`)
and each session's latest page, so it summarises only what's new since claude-summarizer's last run.

To bring claude-summarizer's history onto the deck, import it once on each machine that ran it, with the node
running:

```sh
quietdeck-node import-summaries <claude-summarizer vault>
```

The vault is the `vault_dir` in `~/.config/claude-summarizer/config.json`, or `~/.local/share/claude-summarizer` if
you never set one. The import sends every update to the deck as a point on its session's timeline, with its original
hour.

Then stop claude-summarizer. On a Mac, with the labels from its `launchd/` plists:

```sh
launchctl bootout gui/$(id -u)/<claude-summarizer's label>
launchctl bootout gui/$(id -u)/<claude-summarizer-web's label>
```

The Markdown files stay where they are; the node doesn't write them.

## The runner, on the triage agent's machine

The machine where your triage agent runs can also run the runner, inside its node: it wakes the agent when mail
arrives and carries out the approved actions that need no thought. It's off unless `node.yaml` turns it on, and it
calls your services through Executor or your own commands. See [Executor, and the runner that uses
it](executor.md).
