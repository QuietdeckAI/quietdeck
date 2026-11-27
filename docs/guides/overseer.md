# Set up an overseer

When ten agent sessions run at once, the hard part is knowing which ones are waiting on you. One is stopped on a
permission prompt, one finished an hour ago and wants wrapping up, and one asked you a question before lunch. The
**overseer** is an agent that watches them for you and keeps the [Work](../work.md) tab: what needs you at the top,
progress and completions underneath, a thread under every entry.

<!-- DRAFT: commands, config keys and the skill path to confirm with the quietdeck work session before this ships. -->

This guide assumes a full deck with a runner and a triage agent already working ([Getting started](../getting-started.md)),
and sessions that run through [Fleetlayer](with-fleetlayer.md), which tells the overseer when a session goes idle or
stops on a prompt.

## 1. Where it runs

The overseer reads your sessions' summaries and transcripts, not strangers' mail, so it can run as your own user, next to
the sessions it watches. Give it its own folder for notes:

```sh
mkdir -p ~/agents/overseer
```

## 2. Its instructions

Write the overseer's instructions as a skill, so they're the same on every machine you start it from. They say:

- which sessions to watch, and which to leave alone;
- what belongs at the top of Work (blocked prompts, questions, finished sessions, reviews, your tasks for today) and
  what belongs in the FYI list;
- how to split Work into documents, if you have more than one ("Business" and "Personal", kept separate);
- that it never starts or closes a session without your approval.

<!-- TODO: link the example skill once it's published. -->

## 3. Share the runner

The runner that carries out the triage agent's approved actions does the overseer's too. List both agents in the
runner's config, so each gets its own wake-ups and the work it posted comes back to it:

```yaml
runner:
  enabled: true
  agent:
    tabs: [email triage, overseer]
```

`agent: { tab: … }`, with one agent, keeps working.

## 4. Sessions to ignore

The overseer leaves out the triage agent. For any other session that never needs you (a worker that runs jobs all day,
for example), choose **Ignore this session** from its entry's session menu. The overseer still shows an ignored session
that's stuck on a prompt.

## 5. Start it

Start the overseer in a tab of its own, in its notes folder, with a first prompt that loads its skill:

```sh
quietdeck session start --on <machine> --tab overseer --cwd ~/agents/overseer --prompt "Run the overseer skill."
```

Open Work. Within a few minutes of a session going idle, it has an entry.

## What to try first

- Leave a session on a permission prompt, and answer it from Work's form.
- Let a session finish, approve **Wrap up session**, then approve the stories it proposes and **Close session**.
- **Park** a project you won't touch this week, and **Unpark** it from the Parked section.
- Ask the overseer in an entry's thread what a session did yesterday.
