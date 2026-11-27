# solo.yaml and the quietdeck-solo command

The settings file and commands of [Quietdeck solo](solo.md).

## Commands

| Command | What it does |
|---|---|
| `quietdeck-solo init` | Writes `solo.yaml` and the agent's instructions. `--account id=address` (repeatable; default: every account gog is signed in to), `--port`, `--model`, `--actions` and `--restricted` (below), `--force` to replace existing files. |
| `quietdeck-solo install` | Runs solo as a service that starts at login (launchd on macOS, systemd `--user` on Linux), after checking the settings. `--no-start` writes the service file only. Run it again after changing settings, moving the program or changing PATH. |
| `quietdeck-solo uninstall` | Stops the service and removes its file. Settings and data stay. |
| `quietdeck-solo run` | Runs solo in the terminal: what the service runs. |
| `quietdeck-solo open` | Opens the Inbox in your browser with a one-time sign-in link (5 minutes, used up on first visit). `--print` prints the link instead. |
| `quietdeck-solo status` | Says whether the runner and the agent are keeping up, and what's wrong when they aren't. Exits 1 when something is wrong. |

Every command takes `--config <file>` (or `QUIETDECK_SOLO_CONFIG`) to use another settings file.

## solo.yaml

```yaml
port: 7350                       # the Inbox's port, on 127.0.0.1 only
accounts:                        # each one signed in to gog
  personal: { address: you@gmail.com }
  work: { address: you@example.com, label: Acme }
agent:
  name: email triage             # how the agent is named on its cards
  headless:
    instructions_file: /Users/you/.config/quietdeck/solo-agent.md
    model: sonnet                # optional: claude --model
    bin: claude                  # optional: where Claude Code is
    turn_timeout: 15m            # optional: the longest one round may take
    cwd: ~/somewhere             # optional: where its rounds run (default: <data folder>/agent)
    restricted: false            # optional: true limits it to the quietdeck command (security page)
    allowed_tools:               # restricted mode only: what it may run (the default below)
      - Bash(quietdeck whoami:*)
      - Bash(quietdeck card:*)
      # … one rule per quietdeck subcommand
gog: { bin: /opt/homebrew/bin/gog }   # optional: where gog is, when it isn't on PATH
actions: [archive, mark_read, label, draft]   # optional: what may run once you approve it (all four by default)
```

- **`accounts`**: the key is the account's ID on its cards (letters, digits, `-` and `_`); `label` is what the Inbox
  calls it (default: the ID with a capital letter).
- **`actions`**: what the runner carries out once you approve it. The agent can't propose anything else: a card with
  another action is refused when it's posted, and the agent posts it with no actions instead, saying what you can do
  in Gmail (approving it just closes it). `[draft]` is a safe way to try solo: nothing in your mailbox changes except
  new drafts.
- **`agent.headless.restricted`**: off by default, so the agent runs as your own Claude Code with full access
  (permissions skipped, your settings and MCP servers). On, it may run only `allowed_tools`, with no MCP servers. See
  [Quietdeck solo and your security](solo-security.md).
- **`agent.headless.allowed_tools`** (restricted mode only): Claude Code permission rules. The default allows
  `quietdeck` with `whoami`, `card`, `cards`, `report`, `round`, `chat`, `decisions`, `action`, `actions` and
  `history`. A rule that allows any command, or any quietdeck subcommand, is refused.

## Files

| Where | What |
|---|---|
| `~/.config/quietdeck/solo.yaml` | Your settings (mode 600). |
| `~/.config/quietdeck/solo-agent.md` | The agent's instructions, added to Claude Code's system prompt each round. |
| `~/.local/share/quietdeck/solo/` | The Inbox's database, solo's own node token and the browser sign-in sessions (mode 700). `QUIETDECK_SOLO_DATA` moves it. |
| `~/.local/state/quietdeck/runner-mail.json` | Where the mail watch got to in each mailbox, so a restart carries on from there. |
| `~/.local/state/quietdeck/headless-agent.json` | The agent's Claude Code conversation, resumed each round. |
| `~/.local/share/quietdeck/solo/agent/` | The folder the agent's rounds run in (follows `QUIETDECK_SOLO_DATA`; `agent.headless.cwd` moves it). |
| `~/Library/Logs/quietdeck-solo.log` (macOS) | The service's log. On Linux: `journalctl --user -u quietdeck-solo`. |

## How it checks mail

Every 30 seconds the runner asks gog for what's new in each mailbox since its last look (Gmail's history), reads each
new message, and sends the agent one message with all of them. Every 5 minutes it lists the inbox's threads, to notice
mail you archived or deleted in Gmail yourself. A message's text is cut at 3,000 characters, and one round carries at
most 25 messages; the rest come in the next.

## Health

`http://127.0.0.1:<port>/api/solo/health` answers `{"ok": true, "problems": []}`, or HTTP 503 with what's wrong:

- the mail check is failing (with gog's error), or no mailbox was read in the last 10 minutes;
- the agent's last round failed (with the error);
- the runner hasn't reported for 10 minutes, or since solo started.

`quietdeck-solo status` and the Inbox's warning read the same thing.
