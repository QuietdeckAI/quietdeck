# Configuration reference

Every setting Quietdeck reads, in one place. The pages linked from each row explain what the setting is for.

| File | Where | Read by |
|---|---|---|
| `solo.yaml` | `~/.config/quietdeck/solo.yaml` | `quietdeck-solo`. See [solo.yaml and the quietdeck-solo command](solo-reference.md) |
| `node.yaml` | `~/.config/quietdeck/node.yaml` on each machine, for each Unix user that runs agents | `quietdeck-node`, when it starts |
| `deck.yaml` | `/data/deck.yaml` on the deck's data volume | `quietdeck-server`, when it starts |
| Environment variables | The deck's container, and a few on each node | `quietdeck-server` and `quietdeck-node` |

None of the files hold a secret. Where a token or password is needed, the file names another file (mode 600) or an
environment variable, and your secrets manager fills it ([Secrets for agents](secrets-management.md)).

Both YAML files refuse a key they don't know, so a typo stops the program with an error instead of being ignored. The
`summaries:` and `usage:` sections of `node.yaml` are the exception: they ignore unknown keys. Change a file, then
restart what reads it: `quietdeck-node install` restarts a node, and the deck reads `deck.yaml` again when its
container restarts.

Durations are written `30s`, `2m`, `1h`, or a plain number of seconds.

## node.yaml

`quietdeck node enroll` writes the first version, with the `node:` section filled in
([Enrol a machine](getting-started.md#enrol-a-machine)). Everything else is optional. A working example is in
[`deploy/node.example.yaml`](https://github.com/QuietdeckAI/quietdeck/blob/main/deploy/node.example.yaml).

### node: the deck connection

| Key | Type | Default | What it does |
|---|---|---|---|
| `node.name` | string | required | This machine's name on the deck. Sessions show as `<tab>@<name>`. |
| `node.deck` | URL | required | The deck's address, the same as its `QUIETDECK_PUBLIC_URL`. |
| `node.token_file` | path | `~/.config/quietdeck/node-token` | The file holding the node token. The node refuses a file other users can read. |
| `node.token_env` | string | | An environment variable holding the token instead, for a node started under your secrets manager. Set this or `token_file`, not both. See [How the node reads its token](secrets-management.md#how-the-node-reads-its-token). |

### sessions

Arguments for the sessions the deck starts or resumes on this machine. Permission flags come only from here, never
from the deck or a card ([Starting, resuming and closing sessions](getting-started.md#starting-resuming-and-closing-sessions)).

| Key | Type | Default | What it does |
|---|---|---|---|
| `sessions.claude_args` | list of strings | `[]` | Added to every Claude Code session, such as `--dangerously-skip-permissions`. |
| `sessions.codex_args` | list of strings | `[--no-daemon]` | Added to every Codex session. `--no-daemon` stops a Codex daemon from another pane reporting this session as its own. |

### programs

| Key | Type | Default | What it does |
|---|---|---|---|
| `programs.herdr` | string | `herdr` | The herdr binary, by name on `PATH` or by full path. |
| `programs.claude` | string | `claude` | The Claude Code binary. |
| `programs.codex` | string | `codex` | The Codex binary. |

### summaries

Session summaries, written by headless Claude Code on this machine ([Session
summaries](getting-started.md#session-summaries), [the module's interface](summaries-interface.md)).

| Key | Type | Default | What it does |
|---|---|---|---|
| `summaries.enabled` | bool | `true` | Summarise this machine's sessions. |
| `summaries.idle_seconds` | number | `60` | How long a session must stay idle before it's summarised. |
| `summaries.min_gap_seconds` | number | `1800` | At most one summary per session in this window. Raise it to spend less. |
| `summaries.max_concurrent` | number | `2` | Summary calls at once on this machine. |
| `summaries.input_cap_chars` | number | `48000` | The most new activity, in characters, one call reads. |
| `summaries.daily_chunk_days` | number | `30` | Older history is summarised by day for this many days, then by week. |
| `summaries.sweep` | bool | `true` | Run a sweep when the node starts and once a day, which catches sessions that crashed. |
| `summaries.model` | string | `sonnet` | The model for summary and catch-up calls. |
| `summaries.user_name` | string | `""` | Your name, as the prompts use it ("since Sam's last message"). Empty means "the user". |

### usage

Account usage for this machine's Codex sign-in. No token leaves the machine ([the usage module's
interface](usage-interface.md)).

| Key | Type | Default | What it does |
|---|---|---|---|
| `usage.enabled` | bool | `true` | Read usage on this machine. |
| `usage.poll_minutes` | number | `5` | Poll each account's usage at most this often, and less while sessions report readings. |
| `usage.codex` | bool | `true` | Read Codex sign-ins. |
| `usage.claude` | | | Legacy: accepted and ignored. Claude sign-ins belong to Fleetlayer's claude-auth-manager plugin now. |
| `usage.serve_coordinator_api` | | | Legacy: accepted and ignored, for the same reason. |

### runner

The runner, only on the machine where the triage agent runs. It watches mail, wakes the agent, and carries out
approved actions that need no thought ([Executor, and the runner that uses it](executor.md), [Read and change Gmail
with gog](guides/mail-through-gog.md)). The node checks this section when it starts, but only while `enabled` is true.

| Key | Type | Default | What it does |
|---|---|---|---|
| `runner.enabled` | bool | `false` | Turn the runner on. |
| `runner.agent.tab` | string | required | The triage agent's tab (or its name, when headless). It gets wake-ups, hand-offs and error reports. |
| `runner.agent.headless` | map | | Run the agent as one `claude -p` turn per message instead of in a tab. Keys below. |
| `runner.executor` | map | | Your Executor, for tools given by `path`. Keys below. Required as soon as any tool has a `path`. |
| `runner.mail` | `gog` | | Fill every mail tool you haven't set with the user's own gog CLI, and the mail watch too when there are accounts. A tool you set by hand still wins. |
| `runner.gog.bin` | string | `gog` | Where gog is, for `gog:` tools. |
| `runner.accounts.<id>.address` | string | required per account | The mailbox address. `<id>` is the account ID from `deck.yaml`. |
| `runner.accounts.<id>.label` | string | the ID, capitalised | How wake-ups and reports name the account. |
| `runner.accounts.<id>.vars` | map of strings | `{}` | Anything else the tool inputs need, as `${account.<name>}`: an Executor account ID, a gog account name. |
| `runner.mail_watch` | map | | Watch the accounts for new mail. Keys in the next table. Needs `runner.accounts`. |
| `runner.thread` | tool | | Reads a thread's message IDs, labels and dates (never its content), for the freshness check before an action. |
| `runner.send_as` | tool | | Lists the addresses an account can send as, read hourly. A draft from any other address goes to the agent. |
| `runner.labels` | tool | | Lists an account's labels, so a label that doesn't exist yet goes to the agent, and a lease finds its label. |
| `runner.boards.<name>` | map | | The IDs your story tool needs for each board named in `deck.yaml`, as `${board.<field>}` (a workflow state, a team, a list). |
| `runner.gmail_filters` | map | | The tools behind gmail-filter leases. Keys below. Without it, each holder makes its own filters ([Leases](leases.md#the-runners-filter-tools)). |
| `runner.eligible` | list of action types | every type except `reply` | The action types the runner takes on. A type left out always goes to the agent. `reply` is refused: a reply always goes through the agent. |
| `runner.tools.<name>` | tool | | The tool that carries out each action. Names: `archive`, `mark_read`, `label`, `draft`, `calendar`, `story.create`, `story.comment`, `task.create`, `task.update`, `task.done`. Any other name is refused. Forwards, resumes, new sessions, leases and watches need no tool. An action with no tool goes to the agent. |

The default `eligible` list is `archive`, `mark_read`, `label`, `draft`, `forward`, `watch`, `lease`, `story`, `task`,
`calendar`, `resume` and `new_session`.

#### A tool

Every key above marked "tool" takes the same shape. Set exactly one of `path`, `command` or `gog`.

| Key | Type | What it does |
|---|---|---|
| `path` | string | An Executor app tool, such as `tools.google.gmail.modify`. Needs `runner.executor`. |
| `command` | list of strings | A program and its arguments. It gets the input as JSON on stdin and prints JSON. |
| `gog` | string | A gog operation: `profile`, `history`, `message`, `message_text`, `recent`, `drafts`, `inbox`, `thread`, `send_as`, `labels`, `archive`, `mark_read`, `label` or `draft`. |
| `input` | any | What the tool gets. In it, `${...}` names a value: `account.address`, `account.<var>`, `action.<field>`, `card.<field>`, `board.<field>`; for the mail watch `history_id`, `page_token` and `message_id`; for leases `lease.*` and `thread_ids`. A string that is only a placeholder keeps the value's type, and a placeholder with no value leaves its key out. The full list is in [Add the runner to node.yaml](executor.md#3-add-the-runner-to-nodeyaml). |

#### runner.executor

| Key | Type | Default | What it does |
|---|---|---|---|
| `runner.executor.url` | URL | required | Executor's MCP endpoint, `https://executor.example.com/org/<org>/mcp`. |
| `runner.executor.token_file` | path | | The runner's own Executor token, in a file only you can read. |
| `runner.executor.token_env` | string | | An environment variable holding it instead. Set this or `token_file`, not both. |
| `runner.executor.call_tool` | string | `call` | The MCP tool that runs one app tool by path. |

#### runner.agent.headless

The agent reads untrusted mail, so it gets one command and nothing else ([Give an agent untrusted mail and one
command](guides/headless-agent-one-cli.md)).

| Key | Type | Default | What it does |
|---|---|---|---|
| `runner.agent.headless.bin` | string | `claude` | The Claude Code CLI. |
| `runner.agent.headless.cwd` | path | `~/.local/share/quietdeck/agent` | Where each turn runs. Made on first use. |
| `runner.agent.headless.instructions_file` | path | | The triage instructions, added to Claude Code's system prompt. |
| `runner.agent.headless.model` | string | Claude Code's default | Passed as `--model`. |
| `runner.agent.headless.allowed_tools` | list of strings | `quietdeck` with `whoami`, `card`, `cards`, `report`, `round`, `chat`, `decisions`, `action`, `actions`, `history` | Claude Code permission rules, as `Bash(quietdeck card:*)`. `Bash`, `Bash(*)`, `Bash(quietdeck:*)` and `Bash(quietdeck *)` are refused. |
| `runner.agent.headless.turn_timeout` | duration | `15m` | The longest one turn may take before it's stopped. |

#### runner.mail_watch

With `runner.mail: gog`, the whole section is filled in for you. With Executor, you set `profile` and `history` at
least ([Archive in Gmail and the report keeps up](guides/archive-in-gmail.md)).

| Key | Type | Default | What it does |
|---|---|---|---|
| `runner.mail_watch.every` | duration | `30s` | How often each account is checked. |
| `runner.mail_watch.profile` | tool | required | Reads the account's current history ID (Gmail's `users.getProfile`). |
| `runner.mail_watch.history` | tool | required | Lists changes since a history ID (Gmail's `users.history.list`): new messages, and labels removed or messages deleted, so mail you archive in Gmail leaves the report. |
| `runner.mail_watch.message` | tool | | Reads one new message's From, To, Subject and snippet, never its body, for [while-you-read](guides/while-you-read.md) rules. A message that's already gone then doesn't wake the agent. |
| `runner.mail_watch.drafts` | tool | | Lists the account's drafts every 5 minutes, for the Drafts tab. |
| `runner.mail_watch.inbox` | tool | | Lists every thread ID in the inbox every `inbox_every`, to notice threads that left it, for a history tool that can't say so itself (gog's). |
| `runner.mail_watch.inbox_every` | duration | `5m` | How often `inbox` runs. |
| `runner.mail_watch.message_text` | tool | | Reads one new message's headers and sanitised text, so each wake-up carries the mail itself, for an agent with no mail tools of its own. |
| `runner.mail_watch.recent` | tool | | Lists the inbox's newest messages once, on the first check, so such an agent starts with what's already there. |

#### runner.gmail_filters

In each input, `${lease.query}`, `${lease.label}`, `${lease.label_id}`, `${lease.filter_id}` and `${thread_ids}` name
the lease's values ([Leases](leases.md#the-runners-filter-tools)).

| Key | Type | Default | What it does |
|---|---|---|---|
| `runner.gmail_filters.create` | tool | required | Creates the filter: the query, add the lease's label, take it out of the inbox and mark it read. Returns `{id}`. |
| `runner.gmail_filters.delete` | tool | required | Deletes the filter. |
| `runner.gmail_filters.create_label` | tool | required | Creates the lease's label the first time. Returns `{id}`. |
| `runner.gmail_filters.inbox` | tool | | Finds matching threads already in the inbox. Set it together with `move`. |
| `runner.gmail_filters.move` | tool | | Moves those threads out of the inbox, read, with the lease's label. |

### matrix

The Matrix reader, only on the machine that holds the Matrix user's password ([Bring Matrix messages to the triage
agent](guides/matrix-source.md)). The node checks this section whenever it's present.

| Key | Type | Default | What it does |
|---|---|---|---|
| `matrix.homeserver` | URL | required | The homeserver, `https://matrix.example.com`. |
| `matrix.user` | string | required | The node's own Matrix user, a full ID such as `'@quietdeck:example.com'`. Quote it. |
| `matrix.password_file` | path | required | A file only you can read, holding the password and nothing else. |
| `matrix.rooms` | list of strings | required | The rooms to read, by ID (`'!abc123:example.com'`), not alias. The user must already be a member. |
| `matrix.store` | path | `~/.local/state/quietdeck/matrix` | The device, its token and its crypto store. Keep it on one node. |
| `matrix.device_name` | string | `Quietdeck (<node name>)` | The device name other members see. |
| `matrix.heartbeats[].room` | string | | A dead man's switch room, by ID. A message there is passed on as a ping and goes nowhere else. It can't also be in `rooms`. |
| `matrix.heartbeats[].ping_url_file` | path | | A file only you can read, holding the ping URL (a Healthchecks.io check, say). At most one ping a minute per room. |

## deck.yaml

The deck reads it when it starts. A missing file means every default. An example is in
[`deploy/deck.example.yaml`](https://github.com/QuietdeckAI/quietdeck/blob/main/deploy/deck.example.yaml).

| Key | Type | Default | What it does |
|---|---|---|---|
| `accounts[].id` | string | | The account's ID on cards, timeline entries and pings. Each ID once. With no accounts listed, any ID is accepted. |
| `accounts[].label` | string | | What the Inbox's tab for it says. |
| `accounts[].address` | string | | The mailbox address, so "View email" can open a card's thread in Gmail. Optional. |
| `boards` | list of strings | `[]` | The board names stories can go to. The runner's `boards` maps each to your tracker's IDs. |
| `tasks.label` | string | | Your own task board's label. |
| `pings.weekly_budget` | number | `10` | How many pings a week before the Rules screen shows you're over. Nothing is ever held back ([Pings on your phone](getting-started.md#pings-on-your-phone)). |
| `leases.fallback.node` | string | | With `tab`: who cleans up a lease when no runner is connected and the holder doesn't answer, and who gets your comments on the deck's own timeline entries. Usually your triage tab. |
| `leases.fallback.tab` | string | | That session's tab. |
| `leases.heavy_build_slots.<node>` | number | `2` | Heavy builds at once on that machine. At least 1. A machine not listed gets 2 ([Leases](leases.md#heavy-builds)). |
| `projects.<name>.dirs` | list of paths | `[]` | The working directories in this project. `~` works. A session belongs to the project with the longest matching directory. |
| `projects.<name>.accounts` | list of strings | `[]` | Accounts in this project, for actions on no session. |
| `projects.<name>.automatic` | list of action types | `[]` | The action types the project's policy lets run automatically, under a rule you granted. Adding `reply` works but logs a warning ([Automatic actions](getting-started.md#automatic-actions-rules-and-policies)). |

Action types are `archive`, `mark_read`, `label`, `forward`, `draft`, `reply`, `story`, `task`, `calendar`, `resume`,
`new_session`, `lease` and `watch`.

## Environment variables

### The deck

Set on the deck's container. [`deploy/README.md`](https://github.com/QuietdeckAI/quietdeck/blob/main/deploy/README.md)
has the ports, volume and ingress around them.

| Variable | Required | Default | What it does |
|---|---|---|---|
| `QUIETDECK_PUBLIC_URL` | yes | | The deck's one stable HTTPS origin, a host name rather than an IP address: the passkey relying party, the origin push links open, and the address nodes connect to. Changing it means registering passkeys again. `http://localhost:<port>` works for a deck on your own machine. |
| `QUIETDECK_HOOKS_PUBLIC_URL` | for webhooks | | The public origin of the hooks port, such as `https://hooks.example.com`, for printing endpoint URLs ([Send alerts to the agent that owns them](guides/alerts-to-owners.md)). |
| `QUIETDECK_FLEETLAYER_URL` | no | | Fleetlayer's control plane. With the token, the deck pulls the summaries Fleetlayer's summaries plugin writes. |
| `QUIETDECK_FLEETLAYER_TOKEN` | with the URL | | A read-scoped Fleetlayer token. A secret. Set both or neither. |
| `QUIETDECK_FLEETLAYER_SEND_TOKEN` | no | | A send-scoped Fleetlayer token. With the URL, the deck's own words to sessions (the chat, report answers, handoffs) go through Fleetlayer as `service:quietdeck`. A secret. |
| `QUIETDECK_VAPID_PUBLIC_KEY` | for push | | Web Push public key, base64url. `quietdeck-server vapid-keys` makes a pair. |
| `QUIETDECK_VAPID_PRIVATE_KEY` | for push | | The matching private key. A secret. Changing the pair drops every device's push subscription. |
| `QUIETDECK_VAPID_SUBJECT` | no | `QUIETDECK_PUBLIC_URL` | A contact for push services: `mailto:you@example.com` or an `https://` URL. Needed when the public URL isn't HTTPS. |
| `QUIETDECK_DATA_DIR` | no | `/data` | Where the database lives. |
| `QUIETDECK_CONFIG` | no | `$QUIETDECK_DATA_DIR/deck.yaml` | Where `deck.yaml` is. |
| `QUIETDECK_LISTEN` | no | `0.0.0.0:8080` | The app's address: web app, API, node WebSocket and MCP. Private network only. |
| `QUIETDECK_HOOKS_LISTEN` | no | `0.0.0.0:8081` | The webhook receiver's address. |
| `RUST_LOG` | no | `info` | Log level. |
| `QUIETDECK_DEV_TRUST_LOCAL` | no | off | Development only: every browser request counts as the signed-in owner. Never on a deck anyone else can reach. |
| `QUIETDECK_LOCAL_ONLY` | no | off | Quietdeck solo's mode: loopback only, no passkeys, no webhook receiver. `quietdeck-solo` turns it on itself; don't set it on a deck. |

`QUIETDECK_COMMIT` and `QUIETDECK_BUILT_AT` are read when the image is built, not when it runs: they're what Settings'
Release panel shows.

### A node and the CLI

All optional. The defaults suit almost every machine.

| Variable | Default | What it does |
|---|---|---|
| `QUIETDECK_CONFIG_DIR` | `~/.config/quietdeck` | Where `node.yaml` and the node token are. |
| `QUIETDECK_STATE_DIR` | `~/.local/state/quietdeck` | The node's state: offsets, the runner's history IDs, summaries state, the Matrix store. |
| `QUIETDECK_NODE_SOCKET` | `$XDG_RUNTIME_DIR/quietdeck/node.sock` on Linux, `~/Library/Application Support/quietdeck/node.sock` on macOS | The local socket the CLI and the menu bar app use. |
| `QUIETDECK_FLEETLAYER_SOCKET` | `fleetlayer-plugin.sock` beside the node socket | The socket of the node's Fleetlayer plugin, for messages sent through Fleetlayer. |
| `QUIETDECK_SESSION_ADDRESS` | herdr's `HERDR_PANE_ID` | Says which session a CLI call comes from, for an orchestrator other than herdr ([Orchestrators](orchestrators.md)). |
| `CLAUDE_ACCOUNT_ROTATOR_DIR` | `~/.claude/account-rotator` | Where summaries look for the Claude account rotator's `coordinator.sock`, to pick the account for their calls. |

`QUIETDECK_SOLO_CONFIG` and `QUIETDECK_SOLO_DATA` belong to Quietdeck solo ([solo.yaml and the quietdeck-solo
command](solo-reference.md)).

## Minimal examples

### node.yaml, any machine

```yaml
node:
  name: laptop
  deck: https://deck.example.com
  token_file: ~/.config/quietdeck/node-token
```

### node.yaml, the triage agent's machine, mail through gog

```yaml
node:
  name: server
  deck: https://deck.example.com
  token_file: ~/.config/quietdeck/node-token

sessions:
  claude_args: [--dangerously-skip-permissions]

runner:
  enabled: true
  agent: { tab: email triage }
  mail: gog
  accounts:
    personal: { address: you@example.com, label: Personal }
```

For the same runner through Executor, with stories and leases, copy the `runner:` section of
[`deploy/node.example.yaml`](https://github.com/QuietdeckAI/quietdeck/blob/main/deploy/node.example.yaml).

### deck.yaml

```yaml
accounts:
  - { id: personal, label: Personal, address: you@example.com }
boards: [engineering]
pings: { weekly_budget: 10 }
leases:
  fallback: { node: server, tab: email triage }
projects:
  app:      { dirs: [~/code/app], automatic: [forward, archive, story, task] }
  personal: { accounts: [personal], automatic: [forward, archive] }
```

### The deck's environment

```sh
QUIETDECK_PUBLIC_URL=https://deck.example.com
QUIETDECK_HOOKS_PUBLIC_URL=https://hooks.example.com
QUIETDECK_VAPID_PUBLIC_KEY=<from quietdeck-server vapid-keys>
QUIETDECK_VAPID_PRIVATE_KEY=<from your secrets manager>
QUIETDECK_VAPID_SUBJECT=mailto:you@example.com
```
