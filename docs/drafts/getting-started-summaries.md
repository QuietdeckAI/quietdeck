<!-- Draft from the claude summarizer tab for the getting-started guide's summaries section. The quietdeck tab edits
it in and deletes this file. Every limit below is the module's default and matches crates/quietdeck-node/src/summaries. Not yet run: `quietdeck-node import-summaries` (the node core adds the subcommand) and the launchctl lines (run when claude-summarizer is retired on the author's Mac). -->

## Session summaries

Each node keeps a short, current summary of every Claude Code and Codex session on its machine, and sends it to the
deck. The Sessions screen, search, `quietdeck sessions` and `quietdeck who-has` all read these summaries. You don't
have to set anything up: summaries start when the node starts.

### What a summary is

Every session has three sizes of summary, rewritten together:

- **Sentence:** one line saying what the session is working on and where it stands. The Sessions screen shows it on
  each row.
- **Paragraph:** one or two short paragraphs: what the session is for, where it stands, and what's open.
- **Page:** up to about a page of Markdown: the session's purpose, what happened recently (with dates), and what it's
  waiting on. It keeps the identifiers you'd search for, such as ticket IDs, pull request numbers and branch names.

Each summary is also a point on the session's timeline, one per hour of activity. A later summary in the same hour
replaces the earlier one. On the Sessions screen, **Page** opens the latest page, with Prev and Next through the
earlier ones and a timeline of their sentences.

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
  working again within the minute, it waits for its next idle. Idle includes a finished turn you haven't looked at
  yet.
- **Never while it's waiting on you.** A session at a permission prompt or a question isn't summarised until you've
  answered and it has gone idle.
- **At most once every 30 minutes per session.** If a session goes idle again within 30 minutes of its last summary,
  the node schedules one summary for when the 30 minutes are up. Nothing is dropped.
- **When a session ends.** Closing its tab, or its process exiting, counts as idle.
- **The daily sweep.** When the node starts, and once a day after that, it summarises any session that isn't running
  and has activity it hasn't summarised yet. That catches sessions that ended without an idle moment, such as a
  crash or a machine that went to sleep mid-turn.

Not every idle moment costs a model call. A summary is written only when the new activity holds a message from you or
from another session, 300 or more characters from the agent, or a file edit. A check that found nothing, or a
session waiting on CI, moves on for free. When there is something to summarise but nothing changed (a background
check that found nothing, say), the model answers "no change" and the summary stays as it was.

### What it costs

Each summary is one headless Claude Code call on Sonnet, on that machine's Claude subscription. A call takes 10 to 15
seconds and uses 4,000 to 25,000 input tokens. About 3,000 of those are the fixed prompt; the rest is the current page
plus the new activity, which is capped at 48,000 characters (roughly 12,000 tokens). Over the cap, tool lines are left
out first, then older agent replies, then your older messages. Your newest messages always stay.

As a rough guide, a session that's active all day gets 6 to 10 summaries. Only sessions that did something that day
cost anything.

The node makes at most two summary calls at once, so a dozen tabs finishing together don't all hit your account in
the same second.

### Which account it uses

With several Claude accounts on the machine, each call uses the one whose week resets soonest and still has room
(the "soonest reset" rule from the account usage guide). With one account, calls use Claude Code's own login. If
every account has run out for the week, summaries wait, and the next idle session or the daily sweep tries again.

The calls run with no settings, hooks, plugins, tools, MCP servers or CLAUDE.md, so your configuration doesn't add
to their cost or change what they do.

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
your deck private: it's served only on your tailnet, behind your passkey.

### Coming from claude-summarizer

If you ran claude-summarizer, import its history once on each machine that ran it. Do this before the node's first
sweep: until the import has run, the node holds the sweep, so it doesn't summarise six months of history again.

```sh
quietdeck-node import-summaries <claude-summarizer vault>
```

The vault is the `vault_dir` in `~/.config/claude-summarizer/config.json`, or `~/.local/share/claude-summarizer` if
you never set one. The import sends every update to the deck as a point on its session's timeline, with its original
hour, so each session's history and timeline carry over. It also copies claude-summarizer's byte offsets from
`~/.local/state/claude-summarizer/state.json`, so the node starts where claude-summarizer stopped.

Then stop claude-summarizer. On a Mac:

```sh
launchctl bootout gui/$(id -u)/com.<you>.claude-summarizer
launchctl bootout gui/$(id -u)/com.<you>.claude-summarizer-web
```

(use the labels from its `launchd/` plists). The Markdown files stay where they are; the node doesn't write them.
