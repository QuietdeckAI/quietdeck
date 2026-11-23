# Getting started

Quietdeck has three parts:

- **The deck**: one small server, on your tailnet. It holds the cards you approve, the directory of every agent
  session on every machine, the automatic-actions timeline, and the message relay between sessions. It reads no email
  and holds no credentials for your services.
- **A node** on each machine where you run agent sessions (your laptop, a server VM). It connects out to the deck,
  tells it which sessions are running, delivers messages to them, writes their summaries and gives local tools a
  short-lived token for the deck.
- **The `quietdeck` CLI**, which agents and you use from those machines. Claude Code can also reach the deck as an MCP
  server, with the same commands.

This guide sets up the deck, enrols a machine, connects Claude Code and checks that it all works.

## What you need

- **A tailnet.** The deck is served only on your [Tailscale](https://tailscale.com) network, and it knows it's you
  from the identity Tailscale Serve attaches to each request. Nothing about the deck is public.
- **Somewhere to run the deck**: any machine with Docker, or a Kubernetes cluster, that's always on and on your
  tailnet. It needs very little: a few tens of megabytes of memory and a small volume for its SQLite database.
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

The deck is one container. Build its image from the repository root:

```sh
docker build -f deploy/Dockerfile -t quietdeck-deck .
```

Then run it, with port 8080 published only on the host's loopback address, and put it on your tailnet with
Tailscale Serve:

```sh
docker volume create quietdeck-data
docker run -d --name quietdeck --restart unless-stopped \
  -v quietdeck-data:/data \
  -e QUIETDECK_PUBLIC_URL=https://<deck host>.<tailnet>.ts.net \
  -e QUIETDECK_OWNER_LOGIN=<your Tailscale login> \
  -p 127.0.0.1:8080:8080 \
  quietdeck-deck
tailscale serve --bg --https=443 http://127.0.0.1:8080
```

- `QUIETDECK_PUBLIC_URL` is the deck's address on your tailnet: the HTTPS name `tailscale serve` prints.
- `QUIETDECK_OWNER_LOGIN` is the login Tailscale shows for you (`tailscale whois` on your own IP prints it), such as
  `you@example.com` or `you@github`. If you reach the deck from two tailnets (its machine shared into another one),
  give both logins, separated by commas.
- **Keep port 8080 off every other network.** The deck believes the identity header Tailscale Serve adds, so anything
  that can reach 8080 directly could claim to be you. Publishing on `127.0.0.1` and serving it through Tailscale on
  the same host keeps that from happening. On Kubernetes, see [deploy/README.md](../deploy/README.md) for the
  ingress and the network policy it needs.

Check it's up:

```sh
curl -s https://<deck host>.<tailnet>.ts.net/healthz
```

It answers `ok`. The deck's own settings (your account labels, boards and the rest) go in `/data/deck.yaml`: start
from [deploy/deck.example.yaml](../deploy/deck.example.yaml). Every key is optional.

## Enrol a machine

Each machine gets its own node token, from a one-time code the deck gives you. Get a code on the deck's host:

```sh
docker exec quietdeck /quietdeck-deck enrolment-code
```

The code is good for 15 minutes, once. On the machine you're enrolling:

```sh
quietdeck node enroll <code> --deck https://<deck host>.<tailnet>.ts.net --name <machine name>
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
  '{"type":"http","url":"https://<deck host>.<tailnet>.ts.net/mcp","headersHelper":"quietdeck auth headers"}'
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

```sh
quietdeck msg send --to <tab>@<machine> "Hello from the other machine"
```

The message appears in that session at its next turn, with a line telling it how to answer. Messages to a session
that isn't running fail at once and aren't kept; Quietdeck never queues messages between agents.

```sh
quietdeck who-has <some ID you've worked on>
```

It finds the sessions that mention it: in their summaries on the deck, and in the raw transcripts on every connected
machine. Transcripts never leave their machine; the deck only learns which sessions mention the ID and when they
last did. Each machine searches its newest transcripts first, for a few seconds, and says how far back it got.

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
your deck private: it's served only on your tailnet.

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
