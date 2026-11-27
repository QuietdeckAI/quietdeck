# Set up an overseer

When ten agent sessions run at once, the hard part is knowing which ones are waiting on you. One is stopped on a
permission prompt, one finished an hour ago and wants wrapping up, and one asked you a question before lunch. The
**overseer** is an agent that watches them for you and keeps the [Work](../work.md) tab: what needs you at the top,
progress and completions underneath, a thread under every entry.

This guide assumes a full deck with a runner and a triage agent already working ([Getting started](../getting-started.md)),
and sessions that run through [Fleetlayer](with-fleetlayer.md) v0.3.11 or later. Fleetlayer tells the deck when a
session goes idle or stops on a prompt.

## 1. Where it runs

The overseer reads your sessions' summaries and transcripts, not strangers' mail, so it can run as your own user, next to
the sessions it watches. Give it its own folder for notes:

```sh
mkdir -p ~/agents/overseer
```

## 2. Its instructions

The overseer's instructions are a skill. Start from the
[example skill](https://github.com/QuietdeckAI/quietdeck/blob/main/skills/overseer/SKILL.md): copy
`skills/overseer` into your agent's skills folder (`~/.claude/skills/overseer` for Claude Code) and fill in the parts
marked **yours**: where your tasks live, what your agents' wrap-up is called, which sessions to leave alone. It
already covers what wakes the overseer, how little of a session to read, what goes in Needs you and what in FYI, and
that it never starts or closes a session without your approval.

To split Work into several documents ("Business" and "Personal", kept separate), say in the skill which sessions belong
in which.

## 3. Add the Work document

In the deck's **Settings**, under **Documents**, choose **Add a document**: name it Work, put it under the Work tab,
choose the kind **work**, and make `overseer@<machine>` its writer. Or, in the deck's container:

```sh
quietdeck-server documents-add --slug work --name Work --tab Work --kind work --writer overseer@<machine>
```

## 4. Share the runner

The runner that carries out the triage agent's approved actions does the overseer's too. On the runner's machine, add
the overseer to the runner's `agent` section, so it gets its own wake-ups and the cards it posted come back to it, and
add `session` to `eligible`:

```yaml
runner:
  enabled: true
  agent:
    tab: email triage@<machine>          # the triage agent, as before
    tabs: [overseer@<machine>]
  eligible: [archive, mark_read, label, draft, session]   # keep what you have, and add session
```

`session` covers the three session actions you approve under an entry: **Wrap up session**, **Message session** and
**Close session**.

## 5. Blocked prompts

When a session stops on one of its own menus ("Allow this command?"), Work shows the menu as a small form. Two of
Quietdeck's Fleetlayer plugins do the work:

- **On each session's machine,** the node's plugin reads the prompt. It needs the grants `message.send` and
  `sessions.query`.
- **On the runner's machine,** the runner's plugin types your answer into the session. It needs `session.answer`.

Only the runner may answer. An answer can approve something the session wanted to do, so it goes through the same
approval as every other action, and Fleetlayer's policy enforces that.

## 6. Sessions to ignore

The overseer leaves out the triage agent, the agents the runner works for, and sessions idle for more than six hours.
For any other session that never needs you (a worker that runs jobs all day, for example), choose **Ignore this
session** from its entry's session menu. The overseer still shows an ignored session that's stuck on a prompt.

## 7. Start it

Start the overseer in a tab of its own, in its notes folder:

```sh
quietdeck session start --on <machine> --tab overseer --cwd ~/agents/overseer \
  --prompt "Read the overseer skill, then build the first Work document."
```

Open Work. Within a few minutes of a session going idle, it has an entry. The deck's `/healthz/overseer` says whether
the overseer is being woken: point your uptime checks at it.

## What to try first

- Leave a session on a permission prompt, and answer it from Work's form.
- Let a session finish, approve **Wrap up session**, then approve the stories it proposes and **Close session**.
- **Park** a project you won't touch this week, and **Unpark** it from the Parked section.
- Ask the overseer in an entry's thread what a session did yesterday.
