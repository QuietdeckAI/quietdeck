# Read and change Gmail with gog

An agent system that works on your mail needs something that can read Gmail and change it. Quietdeck's runner can do
that through an API gateway such as [Executor](../executor.md), or through [gog](https://github.com/steipete/gogcli), a
command-line tool for Google's services that you sign in to once. gog is what [Quietdeck solo](../solo.md) uses, and
what `runner.mail: gog` selects in the full deck.

A CLI is simpler to set up than a gateway: nothing to host, no tokens to issue. But gog isn't Gmail's API, and a few of
the things the runner needs from Gmail aren't there or come in another shape. This guide is for anyone who wants their
own agent system to use Gmail through gog: what it gives, what it doesn't, and how to call it safely with values that
came out of strangers' mail. The commands and flags here are the ones the runner runs, checked against real output
from gog 0.46.0.

## gog or Executor

| | `runner.mail: gog` | Executor |
|---|---|---|
| What you run | gog on the runner's machine, signed in as the runner's user | An Executor server, and a token for the runner |
| Sign-ins | One per machine and Unix user | Held once, for every agent and machine |
| Many agents on the same account | Each one signing in to Google refreshes its own token; fine for one runner, not for thirty sessions | The reason to use it: one refresh, no races ([why](../executor.md#why-it-helps-with-many-agents)) |
| Config | `mail: gog` and the accounts; every mail tool is filled in | A tool path and input for each tool |
| What it can carry out | Archive, mark read, label, save a draft | The same, plus stories, tasks and calendar events, through other apps' tools |
| Mail you archive in Gmail | Found by comparing inbox listings every 5 minutes; missed while the runner is stopped | Read from Gmail's history, which the runner keeps its place in across restarts |

Pick gog for one machine and one person's mail, as solo does. Pick Executor once several agents or machines need the
same accounts, or the runner should create stories and calendar events too. They mix: with `mail: gog`, any tool you
set by hand in `node.yaml` wins over the one gog would fill in, so mail can go through gog while stories go through
Executor.

## Sign gog in

Sign gog in to each account, with Gmail access, as the Unix user the runner runs as:

```sh
gog auth add you@example.com --services gmail
```

gog's [README](https://github.com/steipete/gogcli) covers installing it and setting up its Google sign-in. Check it
can read the mailbox:

```sh
gog --account you@example.com gmail search 'in:inbox' --max 3
```

In the full deck, the runner's section of `node.yaml` then needs only the agent and the accounts:

```yaml
runner:
  enabled: true
  agent: { tab: email triage }
  mail: gog
  gog: { bin: /opt/homebrew/bin/gog }      # optional: gog on PATH
  accounts:
    personal: { address: you@example.com, label: Personal }
```

Solo writes the same thing for you from `quietdeck-solo init`.

## How the runner calls gog

Every call has the same shape:

```sh
gog --account=you@example.com --json --no-input --readonly gmail …
```

- `--account` always comes from `node.yaml`, never from an email.
- `--json` makes gog print JSON, which the runner turns into the shapes of Gmail's own API (`historyId`, `labelIds`,
  `messagesAdded`), so the rest of the runner doesn't care which backend it has.
- `--no-input` makes gog fail rather than wait for an answer nobody will give.
- `--readonly` goes on every read, so a read can't change the mailbox even by mistake. Writes (archive, label, a draft)
  leave it off.

Each call has a 60-second limit, and the full inbox listing 180.

## What `gmail history` gives, and what it doesn't

Gmail's history is how a watcher learns what changed since it last looked: give it a history ID and it lists the
changes after it. gog wraps it:

```sh
gog … gmail history --since=28605522 --max=100
```

```json
{
  "historyId": "28643311",
  "messages": ["1a1fdeb3931f11b9", "1a1fdeb3f0eb86ba", "1a1fe18c6d392682"],
  "nextPageToken": "10454226333937819819"
}
```

That's less than Gmail's API returns:

- **New messages only.** gog asks Gmail for added messages. It can't tell you a label came off a message (an archive)
  or a message was deleted.
- **Bare IDs.** No thread ID, no labels. A new message in your inbox, a message you sent and one that went straight to
  spam look the same.
- **No getProfile.** Gmail's API tells you the mailbox's current history ID, which is where a new watcher starts. gog
  has no command for it.

The runner fills each gap with another gog command.

### Labels and thread: `gmail raw --format=minimal`

For each new ID, the runner reads the message's labels and thread:

```sh
gog … gmail raw 1a1fdeb3931f11b9 --format=minimal
```

```json
{
  "historyId": "28605522",
  "id": "1a1fdeb3931f11b9",
  "internalDate": "1795228887000",
  "labelIds": ["SENT"],
  "sizeEstimate": 865,
  "snippet": "…",
  "threadId": "1a1f37b62f43d035"
}
```

That's one call per new message, so the runner asks for history 100 entries a page. A message that's already gone
(exit code 5, below) is skipped: Gmail sometimes adds a message and takes it back again, and that isn't new mail.

### Where to start: the newest message's historyId

With no getProfile, the runner takes the history ID of the newest message in the mailbox:

```sh
gog … gmail messages search --max=1 -- in:anywhere
gog … gmail raw <that message's id> --format=minimal
```

It starts watching from there. It never backfills from an older ID: a watcher that started from last month would wake
the agent for a month of mail. (Solo hands the agent the inbox's 25 newest messages on its first run as a separate
step, so the first report isn't empty: see [Give an agent untrusted mail and one
command](headless-agent-one-cli.md#handing-the-agent-the-mail).)

Gmail forgets history after a while. When a saved history ID is too old (after a machine has slept for a long time),
gog exits with 5, and the runner starts again from the newest message and tells the agent mail came while it couldn't
follow.

### Mail that leaves the inbox

Since history can't show archives or deletions, the runner lists every thread in the inbox every 5 minutes and compares
two complete listings. [Archive in Gmail and the report keeps up](archive-in-gmail.md#through-gog) explains how, and
why a listing that fails or stops part way must change nothing.

## Exit codes, not error text

gog's exit codes are stable, so the runner tells errors apart by code and never by matching what gog printed:

| Exit code | Meaning | What the runner does |
|---|---|---|
| 0 | It worked | Reads the JSON |
| 4 | The account needs signing in again, or lacks Gmail access | Reports "gog isn't signed in to … any more: run `gog auth add … --services gmail`" |
| 5 | Not found: an expired history ID, a message or thread that's gone | Treats it as Gmail's 404 |
| 7 | Rate-limited | Reports "Gmail is rate-limiting"; the mail watch tries again on its next check, and a failed action waits for Retry |
| Anything else | Some other failure | Reports the exit code |

With `--json`, gog prints the error itself as JSON on stdout, not stderr:

```json
{"error": {"class": "not_found", "code": 5, "message": "Google API error (404 notFound): Requested entity was not found."}}
```

A gog that isn't installed is a spawn error, not an exit code; the runner says to install it or set `runner.gog.bin`
(`gog.bin` in solo) to its full path.

## Passing values from mail safely

The runner calls gog with values that came out of email: thread IDs, message IDs, label names, a draft's subject and
body, recipients. Plain code can't be talked into anything, but a value can still become something it shouldn't if it
lands in the wrong place on a command line. Three rules keep that from happening:

- **An ID goes in as a positional argument only if it looks like a Gmail ID**: letters, digits, `-` and `_`, never
  starting with `-`, at most 200 characters. Anything else is refused before gog runs, so `--account=evil@example.com`
  can't arrive where a thread ID should be. A history ID must be all digits.
- **Every other value goes as `--flag=value`**, in one argument. `--subject=--help` is a subject, never a flag, and a
  value can't take the next argument with it. Label names are also refused if they hold control characters.
- **A draft's body goes on stdin**, with `--body-file=-`. It never appears on the command line, where it could be read
  as a flag or seen in `ps`.

An archive, for example:

```sh
gog … gmail thread modify 1a1f37b62f43d035 --remove-label=INBOX
```

and a draft reply, with the body piped in:

```sh
gog … gmail drafts create --to=a@example.com,b@example.com '--subject=Re: Saturday' --body-file=- \
  --from=you@example.com --reply-to-message-id=1a1fdeb3931f11b9
```

Search queries go after `--` (`gmail search … -- in:inbox`) for the same reason, and the runner's own queries are
fixed strings. Before a draft is saved, the runner checks its From address against
`gog gmail settings sendas list`, read hourly.

## "Text file busy"

Now and then, starting gog fails with `Text file busy` (ETXTBSY). Linux refuses to run a file that some process has
open for writing. Two ways to hit it:

- **gog is being upgraded in place** at the moment the runner starts it.
- **A test writes a fake gog and runs it at once.** Under a parallel test run, another thread may fork while the
  script's file is still open for writing. The child inherits that open file until it calls exec, and for that moment
  the kernel won't run the script.

The runner retries the spawn when it gets that error: up to 20 times, 50 milliseconds apart, so at most a second. Any
other error fails at once. If your own tests write a script and run it, either do the same, or close the file and make
sure no fork can hold it before you run it.

## The commands, all together

| What | Command (after `gog --account=… --json --no-input`) |
|---|---|
| Where to start watching | `--readonly gmail messages search --max=1 -- in:anywhere`, then `gmail raw <id> --format=minimal` |
| New messages since | `--readonly gmail history --since=<id> --max=100 [--page=<token>]` |
| A message's labels and thread | `--readonly gmail raw <id> --format=minimal` |
| A message's headers | `--readonly gmail raw <id> --format=metadata` |
| A message's text, without HTML or links | `--readonly gmail get <id> --format=full --sanitize-content` |
| The inbox's newest messages (first run) | `--readonly gmail messages search --max=25 -- in:inbox` |
| Every thread in the inbox | `--readonly gmail search --ids-only --all --max=500 -- in:inbox` |
| A thread's messages and labels | `--readonly gmail thread get <id>` |
| Drafts | `--readonly gmail drafts list --all --max=500` |
| Send-as addresses | `--readonly gmail settings sendas list` |
| Labels | `--readonly gmail labels list` |
| Archive | `gmail thread modify <id> --remove-label=INBOX` |
| Mark read | `gmail thread modify <id> --remove-label=UNREAD` |
| Label | `gmail thread modify <id> --add-label=<name> --remove-label=<name>` |
| Save a draft | `gmail drafts create --to=… --subject=… --body-file=- [--cc=…] [--from=…] [--reply-to-message-id=<id>]` |

## Where this lives in Quietdeck

- The gog backend: every command, the ID checks, the exit codes and the spawn retry:
  [crates/quietdeck-node/src/runner/gog.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/gog.rs)
- Real gog 0.46.0 output the tests read:
  [crates/quietdeck-node/src/runner/fixtures/gog/](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/fixtures/gog/)
- `runner.mail: gog` filling in the mail tools:
  [crates/quietdeck-node/src/runner/config.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/config.rs)
- The mail watch, starting again after an expired history ID, and the inbox listings:
  [crates/quietdeck-node/src/runner/mail.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/mail.rs)
- The example runner config, with `mail: gog`:
  [deploy/node.example.yaml](https://github.com/QuietdeckAI/quietdeck/blob/main/deploy/node.example.yaml)
