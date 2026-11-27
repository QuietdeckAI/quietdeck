# Run the triage agent on its own Unix user

A triage agent reads every email that reaches you, and anyone in the world can send you one. In the full Quietdeck it
usually runs as a long-lived Claude Code session in a herdr tab on an always-on machine, often with permissions
skipped (`--dangerously-skip-permissions`) so it never stops on a prompt nobody is there to answer. So it can run any
command its Unix user can.

If that user is also the one your other agents run as, a persuasive email can reach everything they can: your SSH
keys, your secrets manager's token, your repositories, your cloud CLIs. Its own instructions say mail is data, and
nothing it proposes happens until you approve, but neither stops a shell command that runs during its own turn.

This guide is for anyone running a triage agent, or any agent that reads untrusted input with permissions skipped, on a
machine where other agents also run. It gives the agent a Unix user of its own, with its own Quietdeck node and its own
herdr, so the operating system decides what it can read. The examples use a machine called `agents`, your own user
`you`, and a new user `triage`.

For isolating agents that read untrusted input in general (containers, network rules, what to grant), see Fleetlayer's
[untrusted input guide](https://fleetlayer.ai/guides/untrusted-input/). This page covers the Quietdeck side.

If you only want the agent to run a few commands and nothing else, [Give an agent untrusted mail and one
command](headless-agent-one-cli.md) shows the other approach: a headless agent whose only tool is `quietdeck`. The two
combine well.

## What the triage user holds

Only what triage needs:

- its own Claude Code sign-in;
- its own Quietdeck node token;
- its mail access: gog's sign-in, or the runner's Executor token (see [below](#the-runner-runs-in-the-triage-users-node));
- its working folder, its instructions and the email-triage skill.

And nothing else: no SSH keys, no secrets manager token, no checkouts of your repositories, no cloud or Kubernetes
credentials, no shared notes it doesn't need, no sudo. If something only lives under your own user, triage can't read
it, so put nothing for it anywhere it could.

## Create the user

As an admin on `agents`:

```sh
sudo adduser --disabled-password --gecos "" triage
sudo loginctl enable-linger triage
```

- `--disabled-password`: nobody logs in with a password. You'll reach it over SSH with your own key, below.
- `enable-linger`: its user services (herdr and the node) run with nobody logged in.

Then check that it can't read other users' homes, and that they can't read its own:

```sh
ls -ld /home/*
sudo chmod 700 /home/you /home/triage
```

Recent Debian and Ubuntu make new homes `700`, but older systems and homes made by other tools are often `755`, which
lets every user read every file not marked otherwise. Don't add `triage` to any group (`sudo`, `docker`, `adm`), and
don't give it an entry in `sudoers`: membership of `docker` alone is as good as root.

Install the programs it runs (Claude Code, herdr, `quietdeck` and `quietdeck-node`, gog) somewhere it can run but not
change, such as `/usr/local/bin`, as the admin, or in its own home as `triage`. Never point it at binaries in your
home: it can't read them, and if it could, it could replace them.

To let your own laptop open its herdr tabs, add your laptop's public key, and only that, to
`/home/triage/.ssh/authorized_keys` (mode `600`, owned by `triage`).

## Log in as triage to set it up

The setup below runs as `triage`, in a login session: `systemctl --user` needs one, and `sudo -u triage` doesn't give
you one (it has no `XDG_RUNTIME_DIR`). Use SSH, or on systemd:

```sh
sudo machinectl shell triage@
```

## Its own herdr

Each Unix user runs its own herdr server, and the node of that user reads it. As `triage`, run herdr's server as a user
service:

```ini
# ~/.config/systemd/user/herdr.service
[Unit]
Description=herdr server

[Service]
ExecStart=/usr/local/bin/herdr server
Restart=on-failure

[Install]
WantedBy=default.target
```

```sh
systemctl --user enable --now herdr
```

Run the same herdr version as your laptop, which attaches to it. On the laptop, add it as a machine of its own:

```sh
herdr machine add triage@agents --label agents-triage
```

Its tabs then show in your laptop's herdr beside your own user's.

## Claude Code, signed in as triage

Still as `triage`, sign Claude Code in:

```sh
claude auth login
```

Never copy your own `~/.claude` or `~/.claude.json` across: two users refreshing the same sign-in sign each other out,
and your folder holds your settings, MCP servers and history, none of which triage should have. Give it only its own
global instructions and the skills it uses.

Mark its working folder as trusted before it starts there unattended, so it doesn't stop on Claude Code's trust prompt:
Fleetlayer's [unattended agents guide](https://fleetlayer.ai/guides/unattended-agents/) says how.

## Its own Quietdeck node

Each Unix user that runs agents needs its own node: the node finds sessions through its own user's herdr, and hands
out tokens only to that user's sessions. Enrol `triage` as a machine of its own, with a one-time code from the deck's
**Settings** (**Enrol a machine**):

```sh
quietdeck node enroll <code> --deck https://<deck address> --name agents-triage
quietdeck-node install
quietdeck node status
```

Give it a name with `--name`. The default is the host name, which your own user's node on the same machine already
has. The token goes in triage's `~/.config/quietdeck/node-token`, readable only by `triage`, and `quietdeck-node install`
makes a systemd user service, which lingering keeps running. `quietdeck node status` should print `agents-triage`, the
deck's address and `connected`.

On the deck, `agents-triage` is a machine like any other: its sessions show as `<tab>@agents-triage`.

## Start the triage tab

Open `agents-triage` in your laptop's herdr, make a tab named `email triage`, and start `claude` in its working folder
with the email-triage skill. If the node starts it for you, the arguments every Claude Code session gets on that node
(such as `--dangerously-skip-permissions`) come from `sessions.claude_args` in triage's own `node.yaml`, never from the
deck or a card.

Check, from the tab:

```sh
quietdeck whoami
```

It should say `You are email triage@agents-triage`.

## The runner runs in the triage user's node

Without Fleetlayer, the runner wakes the agent through the node the agent's tab runs under: it delivers only to a tab
on its own node. So
its `runner:` section goes in triage's `~/.config/quietdeck/node.yaml`, with `agent: { tab: email triage }`, and
nowhere else.

That means the runner's mail access belongs to `triage` too: gog signed in as `triage` ([Read and change Gmail with
gog](mail-through-gog.md)), or the runner's Executor token in a file under triage's home ([Executor](../executor.md)).
The agent can read it. That's the one credential worth giving it, since triage is about mail, but keep it to mail:
a gog sign-in with `--services gmail` only, or an Executor token for the tools the runner needs. Everything the runner
carries out still needs your approval first.

With [Fleetlayer](with-fleetlayer.md) running Quietdeck's plugin on both nodes, there's a better arrangement: keep the
runner in your own user's node, holding the mail credential, and point it at the tab on triage's node with
`agent: { tab: "email triage@agents-triage" }`. The runner then reaches the agent through Fleetlayer, and triage's user
holds no mail credential at all. Without Fleetlayer, the runner can only wake a tab on its own node.

## How other sessions reach it

`SendMessage`, Claude Code's own messaging, reaches only sessions of the same Unix user on the same machine. Your other
agents, and you, reach triage through Quietdeck:

```sh
quietdeck msg send --to "email triage@agents-triage" "The build alerts are mine now; skip them."
```

The message arrives in its tab at its next turn, saying who sent it and how to answer.
[Let agents message each other](agents-messaging.md) covers the rest, including why an agent must never type into
another session's terminal instead.

Messages going the other way are where to be careful. An agent that reads strangers' mail is the one most likely to be
asked, by an email, to message another agent or start a session. In the full deck, forwards and new sessions are card
actions that wait for your approval, so triage doesn't need to send them itself. If you run it headless, leave
`msg send` and `session start` out of its allowed commands, as solo does.

## Checking it

As `triage`, each of these should fail:

```sh
ls /home/you
cat /home/you/.ssh/id_ed25519
sudo -n true
```

And from one of your own sessions, `quietdeck sessions --node agents-triage` should list the `email triage` tab as
running.

If you copy triage's Claude Code transcripts into your own home (to back them up or search them), anything that sweeps
your own `~/.claude/projects` will see them as yours. Fleetlayer's [day-to-day guide](https://fleetlayer.ai/guides/day-to-day/)
has the fix.

## Where this lives in Quietdeck

- Enrolling a node, and `msg send`:
  [crates/quietdeck-cli/src/main.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-cli/src/main.rs)
- The CLI finding its node's socket in `/run/user/<uid>/` when there's no login session:
  [crates/quietdeck-cli/src/client.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-cli/src/client.rs)
- `sessions.claude_args`, the arguments every session on a node gets:
  [crates/quietdeck-node/src/config.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/config.rs)
- The runner delivering only to a tab on its own node:
  [crates/quietdeck-node/src/runner/deck.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/deck.rs)
- Enrolling and running a node: [Getting started](../getting-started.md#enrol-a-machine)
