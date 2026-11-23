# Executor, and the runner that uses it

Executor is a gateway that holds your sign-ins to other services (Gmail, Calendar, Shortcut,
Slack and the rest) and gives agents their tools over one MCP connection. Quietdeck doesn't need it, host it or bundle
it, and the deck never connects to it. But once you run more than a handful of agents, it's the easiest way to give
them your services, and it's what the runner's example config uses.

## Why it helps with many agents

Each MCP server that signs in with OAuth keeps its own refresh token. Give thirty Claude Code sessions their own Gmail
or Shortcut connection and they all refresh the same token at once: one wins, and the rest find theirs revoked and ask
you to sign in again. Executor holds each sign-in once and refreshes it in one place. Your sessions connect to Executor
with a token that doesn't expire on them, and Executor calls the services.

It also means one place to see what agents called (Executor's call log), and one place to switch a service off.

## How Quietdeck uses it

- **The triage agent** reads mail, drafts replies and creates stories through Executor's tools, like any of your
  sessions. Quietdeck doesn't see any of it: the agent posts cards to the deck with what it found and what it proposes.
- **The runner**, inside the node on the triage agent's machine, carries out the approved actions that need no
  thought (archive, mark read, labels, drafts, stories, calendar events) with its own Executor token, and watches
  Gmail's history to wake the triage agent when mail arrives. It reads message IDs and labels only, never a subject,
  sender or body. Forwards to sessions, resumes and new sessions go through the deck and herdr instead, and need no
  Executor tool.

Anything the runner can't do goes to the triage agent, which does it with its own tools. A missing tool costs a few
tokens and seconds, never a stuck card. The deck's **Settings** lists, under the runner, the action types it handed
over for lack of a tool, most often first: those are the ones worth configuring next.

The runner never sends a reply. A reply sends mail as you straight away, so it always goes through the triage agent,
which rechecks the thread first, and the runner refuses to start if `runner.eligible` lists `reply`.

## Set up the runner

### 1. A token for the runner

In Executor, open **Account > Tokens** and create a token for the runner, named after the machine
(`runner on <machine>`). Keep it in your secrets manager, and write it to a file only you can read on the triage
agent's machine:

```sh
(umask 077; op read "op://agents/quietdeck-runner-<machine>/token" > ~/.config/quietdeck/executor-token)
```

Or keep it out of files entirely: set `token_env` instead of `token_file` and start the node under `op run`, as
[Secrets management for agents](secrets-management.md#how-the-node-reads-its-token) describes for the node's own
token. Either way the runner sends it only to Executor, and never logs it.

### 2. Find the tool paths and account IDs

The runner calls Executor's tools by path, with the input your `node.yaml` gives. From a Claude Code session connected
to Executor, ask for the tools you need, or call `search` yourself:

```text
search({ namespace: "google.gmail" })          # modify, draft, labels
search({ query: "gmail history list sendAs threads get", namespace: "google.api" })
search({ query: "create story", namespace: "shortcut" })
```

Each result shows the tool's input. Executor's raw API tools (`tools.google.api.*`) take
`{accountId, input: {path, query}}`, and `search` lists the account IDs. Its own Gmail and Calendar tools
(`tools.google.gmail.*`, `tools.google.calendar.*`) take an `account` address instead.

The mail watch, the freshness checks and the send-as check expect Gmail API's own response shapes
(`users.history.list`, `users.threads.get`, `users.settings.sendAs.list`), which is what Executor's raw API tools
return. A CLI that prints the same JSON works as well.

### 3. Add the runner to node.yaml

Copy the `runner:` section from [`deploy/node.example.yaml`](../deploy/node.example.yaml) into
`~/.config/quietdeck/node.yaml` on the triage agent's machine, set `enabled: true`, and fill in:

- `agent.tab`: the herdr tab where the triage agent runs.
- `executor.url`: your Executor's MCP address (`https://<executor host>/org/<org>/mcp`).
- `accounts`: each mail account, by the account IDs in the deck's config, with its address and Executor's ID for it.
- `boards`: each tracker board the deck's config names, with the IDs your story tool needs.
- `tools`: one entry per action, with the tool's path and input: `archive`, `mark_read`, `label`, `draft`, `calendar`,
  and for stories and tasks one per op, `story.create`, `story.comment`, `task.create`, `task.update` and `task.done`,
  so an approved update never runs your create tool. An action with no entry goes to the triage agent.

In an input, `${...}` names a value: `account.address`, `account.<var>` from the account's `vars`, `action.<field>`
from the approved action (plus `action.story_number`, the number in a story or task's `story_id` such as `sc-1234`,
for trackers whose tools take a number, and `action.agent_labels`, `["agent:<tab>"]` when a task names the tab that
works on it, or as `action.agent_label_objects`, `[{"name": "agent:<tab>"}]`), `card.<field>` (`id`, `thread_key`,
`last_message_ref`, `subject`), `board.<field>` for a
story, and `history_id` and `page_token` for the mail watch. A string that's only a placeholder keeps the value's
type, so `addLabels: "${action.add}"` passes a list. A placeholder with no value leaves its key out, which is how the
first page of history goes without a `pageToken`.

A tool can be a command instead of an Executor path. The runner writes the input to its stdin as JSON and reads JSON
from its stdout:

```yaml
    archive: { command: [my-gmail-tool, archive], input: { account: "${account.address}", thread: "${action.thread_id}" } }
```

`eligible` lists the action types the runner takes on. Leave a type out and it always goes to the agent.

### 4. Restart the node and check it

```sh
quietdeck-node install    # or restart your own service
```

The node's log says `runner on` with the agent's tab, and the deck's **Settings** shows `runner:` with its action
types under the machine. Within a minute of new mail the triage agent gets a message such as
`New mail: 2 Personal, 1 Work.` It gets no other until it has run its round and said so:

```sh
quietdeck round done
```

(or the `round_done` MCP tool). Mail that arrives meanwhile is counted and goes in the next wake-up, so a busy agent
finds one message waiting, not twenty. The last history ID is kept on the machine
(`~/.local/state/quietdeck/runner-mail.json`), so after the machine sleeps the runner picks up where it left off.

The watch starts from the moment the runner first runs: mail already in the inbox never wakes the agent, and is its
own to sweep or leave. To see where the watch is, the agent runs `quietdeck round status` (or the `round_status` MCP
tool):

```text
Not waiting on you: the next new mail wakes you.
Last checked the mailboxes at 2026-11-24 00:44.
Personal (personal): watching since 2026-11-23 21:40, history ID 28613693
Mail from before the watch started never woke you.
```

## What the runner checks before it acts

Approved cards reach the runner first. It sorts them in code, and hands a whole card to the agent, with a reason the
agent and the card both show, when:

- any of its actions isn't in `eligible`, has no tool configured, or is a reply;
- your decision carries text (an adjustment or a comment);
- the agent posted it with `--needs-agent`;
- an action fails the runner's own schema check, or names an account or board missing from its config;
- the thread has a message newer than the one the card was prepared on, or no longer exists;
- a draft's `from` isn't an address the account can send as (the send-as list is read hourly);
- a label it would add doesn't exist yet (when `labels` is configured).

It runs each action it kept only after the deck agrees to start it, which Stop makes fail, and refuses one whose
payload differs from what was approved. A thread that's already archived, or already read, counts as done. If a call
fails, the action is marked failed with the error and its card waits on the Cards screen with Retry, and the triage agent
gets one short report for the batch:

```text
Runner: 1 of 10 failed. Archive of 'Your invoice' (Personal): Executor error 401. Card c_812 is waiting with Retry.
```

Nothing is retried on its own: a call that failed may still have happened, so you decide.

When every card in a batch was simple, the agent isn't woken at all. On its next round,
`quietdeck history --since last` shows what the runner did.
