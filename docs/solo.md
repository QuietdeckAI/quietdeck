# Quietdeck solo

Quietdeck solo runs on your own computer and helps with your email: an AI agent reads what arrives, writes a short
report of what needs you, drafts replies and proposes what to archive. You read the report in your browser, change
what you'd do differently and approve. Nothing happens to your mail until you do.

It's one program, `quietdeck-solo`, running in the background on a Mac or a Linux machine. It needs no server, no
public address and no account with anyone: your mail stays between your computer and Google.

## How it works

- **The runner** watches your Gmail with [gog](https://github.com/steipete/gogcli), a command-line tool you sign in to
  once. When mail arrives, it tells the agent which emails are new. It's also what carries out what you approve:
  archiving, marking read, labelling and saving drafts.
- **The agent** is your [Claude Code](https://docs.anthropic.com/en/docs/claude-code), run once for each batch of new
  mail. It reads each email in full with gog, researches with your files, notes and the web, and posts its proposals
  (cards) and the report through the `quietdeck` command. By default it has full access to your computer, like your
  own Claude Code; you can restrict it instead.
- **The Inbox** is a web page on your computer, at `http://127.0.0.1:7350`. It shows the agent's report, with what it
  proposes for each email. You approve one item, change it, or approve everything at once.

The agent never changes your mail itself, and solo never sends mail: a reply it proposes is saved as a Gmail draft
for you to read, change and send yourself. **The agent reads mail from strangers with full access to your computer,
so read [Quietdeck solo and your security](solo-security.md) before you start**: it explains the risk and the
restricted mode.

## What you need

- **A Mac or a Linux computer** that's on while you want mail triaged. Solo runs as a background service that starts
  when you log in.
- **Gmail.** Solo reads Gmail through gog. Other mail services aren't supported yet.
- **gog**, signed in to each Gmail account: see its [README](https://github.com/steipete/gogcli) for installing it and
  setting up its Google sign-in.
- **Claude Code**, installed and signed in (a Claude subscription or an API key). The agent runs as `claude -p`.
- **To build solo**: a Rust toolchain ([rustup](https://rustup.rs)) and Node.js 24 with pnpm. There are no prebuilt
  releases yet.

## Install

Build the web app first, so the Inbox is built into the program, then install `quietdeck-solo`:

```sh
git clone https://github.com/QuietdeckAI/quietdeck
cd quietdeck
pnpm -C web install && pnpm -C web build
cargo install --locked --path crates/quietdeck-solo
```

`cargo install` puts `quietdeck-solo` in `~/.cargo/bin`. If you skip the web build, solo still runs, but the Inbox
page only says "This build has no web app".

## Sign gog in to your mail

Sign gog in to each Gmail account you want triaged, with access to Gmail:

```sh
gog auth add you@gmail.com --services gmail
```

Check it can read your mail: `gog --account you@gmail.com gmail search 'in:inbox' --max 3`.

## Set it up

```sh
quietdeck-solo init
```

`init` finds the accounts gog is signed in to and writes two files:

- `~/.config/quietdeck/solo.yaml`: your settings (the accounts, the Inbox's port, the agent).
- `~/.config/quietdeck/solo-agent.md`: the agent's instructions. Read them: they're plain English, and you can change
  them.

To choose the accounts and what the Inbox calls them, name them yourself:

```sh
quietdeck-solo init --account personal=you@gmail.com --account work=you@example.com
```

`--model sonnet` picks the Claude model the agent uses, `--port` the Inbox's port. Running `init` again needs
`--force`, and replaces both files.

## Start it

```sh
quietdeck-solo install
```

This runs solo as a background service that starts when you log in: a launchd agent on a Mac, a systemd user service
on Linux. On Linux, run `loginctl enable-linger "$USER"` once if it should keep running while you're logged out.

To try it in a terminal first instead, run `quietdeck-solo run` and stop it with Ctrl-C.

## Open the Inbox

```sh
quietdeck-solo open
```

This opens the Inbox in your browser, signed in. The link it uses works once, for five minutes; your browser then stays
signed in. Run `quietdeck-solo open --print` to get the link without opening a browser.

On its first run, solo hands the agent the newest mail in your inbox (up to 25 messages), so the first report has
something in it within a minute or two. After that, new mail is triaged as it arrives.

## Using it

The report starts with what needs you, numbered, then the routine mail grouped under headings, ending with one
**Archive all** for everything that only needs archiving. Each item says what the agent proposes.

- **Approve** an item to carry it out, or **Approve All** for everything on the report.
- **Change this** on an item to say what you'd do instead, in your own words ("keep this one, I'll reply myself",
  "label it Receipts"). The agent updates its proposal; nothing runs until you approve the new version.
- **Drafts** the agent wrote are in Gmail's Drafts folder once you approve them, and on the Inbox's Drafts tab. Edit and
  send them from Gmail.
- **The chat box** under the report talks to the agent, for anything else.

Archiving in Gmail yourself is fine: solo notices within a few minutes and takes those emails off the report.

## Keep an eye on it

When something stops working (gog's sign-in expired, Claude Code can't start), the Inbox shows a warning in the top
right until it's fixed, saying what's wrong. From a terminal:

```sh
quietdeck-solo status
```

prints the same, and exits with 1 when something is wrong, so a health checker can run it. The logs are in
`~/Library/Logs/quietdeck-solo.log` on a Mac, and `journalctl --user -u quietdeck-solo` on Linux.

## Change the settings

Edit `~/.config/quietdeck/solo.yaml` or the agent's instructions, then restart solo with `quietdeck-solo install`.
[solo.yaml and the quietdeck-solo command](solo-reference.md) lists every setting.

## Stop it

`quietdeck-solo uninstall` stops the service and removes it. Your settings, the reports and solo's record of your
mailbox stay in `~/.config/quietdeck` and `~/.local/share/quietdeck/solo`; delete those folders to start from scratch.

## When something goes wrong

| What you see | What to do |
|---|---|
| "gog isn't signed in to … any more" | Sign in again: `gog auth add <address> --services gmail`. |
| "gog isn't installed or isn't on PATH" | Install gog, or set `gog: { bin: /full/path/to/gog }` in solo.yaml. Then `quietdeck-solo install` again, which also records your current PATH. |
| "The agent's last turn failed" | Check Claude Code works in a terminal (`claude -p "hello"`). The error after the colon says what failed. |
| The page says "sign in on this machine" | Run `quietdeck-solo open` again: each link works once. |
| "This build has no web app" | Build the web app (`pnpm -C web build`) and install solo again. |
| `quietdeck-solo status` says it isn't answering | Solo isn't running: `quietdeck-solo install`, then check the log. |

## What solo doesn't do yet

Solo is the smallest Quietdeck: one computer, Gmail, one agent, and the Inbox's mail actions. It doesn't send mail or
forward it to other agents, start other agent sessions, or create tickets or calendar events. Its agent is Claude
Code: Codex can't be limited the same way yet ([why](solo-security.md#why-the-agent-is-claude-code-not-codex)). The full
Quietdeck, with a deck on a server and agents on several machines, is in
[Getting started](getting-started.md).
